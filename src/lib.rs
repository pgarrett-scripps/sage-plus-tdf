//! Read-only TDF frames and source metadata without spectrum synthesis.
#![forbid(unsafe_code)]
mod compression;
mod database;
mod mobility;

pub use database::{MetadataDatabase, MetadataRow};
pub use mobility::MobilityModel;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Database(String),
    #[error(transparent)]
    Metadata(#[from] serde_json::Error),
    #[error("invalid TDF data: {0}")]
    Invalid(String),
    #[error("unsupported TDF feature: {0}")]
    Unsupported(String),
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// Resource ceilings checked before allocating compressed or decoded frame buffers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_compressed_bytes: usize,
    pub max_decompressed_bytes: usize,
    pub max_peaks: usize,
    pub max_scans: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: 256 * 1024 * 1024,
            max_decompressed_bytes: 512 * 1024 * 1024,
            max_peaks: 32_000_000,
            max_scans: 100_000,
        }
    }
}

/// Source frame identifiers and counts, without renumbering or inferred acquisition types.
#[derive(Clone, Debug, Deserialize)]
pub struct FrameInfo {
    #[serde(rename = "Id")]
    pub id: u64,
    #[serde(rename = "Time")]
    pub retention_time_seconds: f64,
    #[serde(rename = "MsMsType")]
    pub msms_type: u32,
    #[serde(rename = "NumScans")]
    pub scan_count: usize,
    #[serde(rename = "NumPeaks")]
    pub peak_count: usize,
    #[serde(rename = "TimsId")]
    pub binary_offset: u64,
    #[serde(rename = "TimsCalibration", default)]
    pub mobility_calibration_id: Option<u64>,
}

/// Original raw coordinates and counts. Intensities are not rescaled.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: u64,
    pub scan_offsets: Vec<usize>,
    pub tof_indices: Vec<u32>,
    pub intensities: Vec<u32>,
}

pub struct TdfReader {
    directory: PathBuf,
    metadata: MetadataDatabase,
    binary: File,
    frames: BTreeMap<u64, FrameInfo>,
    compression: u32,
    limits: Limits,
}
impl TdfReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_limits(path, Limits::default())
    }

    pub fn with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<Self> {
        let path = path.as_ref();
        let directory = path
            .ancestors()
            .find(|p| p.join("analysis.tdf").is_file() && p.join("analysis.tdf_bin").is_file())
            .ok_or_else(|| {
                Error::Unsupported(
                    "expected a local TDF directory with analysis.tdf and analysis.tdf_bin".into(),
                )
            })?
            .to_path_buf();
        let metadata = MetadataDatabase::open(directory.join("analysis.tdf"))?;
        let global = metadata.table("GlobalMetadata")?;
        let compression = global
            .iter()
            .find(|r| r.get("Key").and_then(|v| v.as_str()) == Some("TimsCompressionType"))
            .and_then(|r| r.get("Value"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid("missing TimsCompressionType"))?
            .parse::<u32>()
            .map_err(|_| invalid("invalid TimsCompressionType"))?;
        if !matches!(compression, 1 | 2) {
            return Err(Error::Unsupported(format!("compression {compression}")));
        }
        let binary = File::open(directory.join("analysis.tdf_bin"))?;
        let binary_length = binary.metadata()?.len();
        let mut frames = BTreeMap::new();
        for row in metadata.table("Frames")? {
            let frame: FrameInfo = serde_json::from_value(serde_json::Value::Object(row))?;
            if frame.id == 0
                || !frame.retention_time_seconds.is_finite()
                || frame.retention_time_seconds < 0.0
                || frame.scan_count > limits.max_scans
                || frame.peak_count > limits.max_peaks
                || frame
                    .binary_offset
                    .checked_add(8)
                    .is_none_or(|end| end > binary_length)
            {
                return Err(invalid("invalid frame metadata or resource limit exceeded"));
            }
            if frames.insert(frame.id, frame).is_some() {
                return Err(invalid("duplicate frame identifier"));
            }
        }
        Ok(Self {
            directory,
            metadata,
            binary,
            frames,
            compression,
            limits,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn metadata(&self) -> &MetadataDatabase {
        &self.metadata
    }
    /// Sorted by the original identifier. Sparse identifiers are retained.
    pub fn frames(&self) -> impl Iterator<Item = &FrameInfo> {
        self.frames.values()
    }

    pub fn read_frame(&mut self, id: u64) -> Result<Frame> {
        let info = self
            .frames
            .get(&id)
            .ok_or_else(|| invalid(format!("unknown frame {id}")))?;
        self.binary.seek(SeekFrom::Start(info.binary_offset))?;
        let mut header = [0u8; 8];
        self.binary.read_exact(&mut header)?;
        let bytes = compression::word(&header, 0)? as usize;
        let scans = compression::word(&header, 4)? as usize;
        let binary_length = self.binary.metadata()?.len();
        if bytes < 8
            || bytes > self.limits.max_compressed_bytes
            || scans != info.scan_count
            || info
                .binary_offset
                .checked_add(bytes as u64)
                .is_none_or(|end| end > binary_length)
        {
            return Err(invalid("invalid frame header, extent or scan count"));
        }
        let mut data = vec![0u8; bytes - 8];
        self.binary.read_exact(&mut data)?;
        let frame = compression::decode(
            self.compression,
            id,
            &data,
            scans,
            info.peak_count,
            self.limits,
        )?;
        Ok(frame)
    }

    /// Read the exact per-frame model. Missing references never fall back to another frame.
    pub fn mobility_model(&self, frame: u64) -> Result<MobilityModel> {
        let id = self
            .frames
            .get(&frame)
            .ok_or_else(|| invalid("unknown frame"))?
            .mobility_calibration_id
            .ok_or_else(|| invalid("frame has no TimsCalibration reference"))?;
        let rows = self.metadata.table("TimsCalibration")?;
        let matches: Vec<_> = rows
            .iter()
            .filter(|row| row.get("Id").and_then(|v| v.as_u64()) == Some(id))
            .collect();
        if matches.len() != 1 {
            return Err(invalid("missing or duplicate calibration identifier"));
        }
        let row = matches[0];
        if row.get("ModelType").and_then(|v| v.as_u64()) != Some(2) {
            return Err(Error::Unsupported("mobility ModelType must be 2".into()));
        }
        let mut coefficients = [0.0; 10];
        for (i, c) in coefficients.iter_mut().enumerate() {
            *c = row
                .get(&format!("C{i}"))
                .and_then(|v| v.as_f64())
                .ok_or_else(|| invalid("missing calibration coefficient"))?;
        }
        MobilityModel::new(coefficients)
    }
}
