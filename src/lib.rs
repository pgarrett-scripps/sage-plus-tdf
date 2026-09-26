//! Read-only TDF frames and source metadata without spectrum synthesis.
//!
//! Frame identifiers, scan offsets, TOF indices and detector intensities are retained.
//! Calibration is explicit and unsupported source models return an error.
//!
//! ```no_run
//! use timsrust_reader::{Result, TdfReader};
//!
//! fn inspect(path: &str) -> Result<()> {
//!     let mut reader = TdfReader::open(path)?;
//!     let ids: Vec<_> = reader.frames().map(|frame| frame.id).collect();
//!     for id in ids {
//!         let frame = reader.read_frame(id)?;
//!         for (scan, offsets) in frame.scan_offsets.windows(2).enumerate() {
//!             let ions = offsets[0]..offsets[1];
//!             println!("frame {id}, scan {scan}: {} ions", ions.len());
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! Mass and mobility models are loaded from the frame's recorded references.
//! Missing calibration and unsupported equations are reported explicitly.
//!
//! ```no_run
//! # use timsrust_reader::{Result, TdfReader};
//! # fn calibrated(reader: &TdfReader, id: u64) -> Result<()> {
//! let mz = reader.mz_model(id)?.mz(100_000)?;
//! let mobility = reader.mobility_model(id)?.one_over_k0(100.0)?;
//! println!("m/z {mz}, inverse reduced mobility {mobility}");
//! # Ok(())
//! # }
//! ```
#![forbid(unsafe_code)]
mod compression;
mod database;
mod mass;
mod mobility;

pub use database::{MetadataDatabase, MetadataLimits, MetadataRow};
pub use mass::MzModel;
pub use mobility::MobilityModel;
use serde::Deserialize;
use std::{
    cell::OnceCell,
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
    pub metadata: MetadataLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: 256 * 1024 * 1024,
            max_decompressed_bytes: 512 * 1024 * 1024,
            max_peaks: 32_000_000,
            max_scans: 100_000,
            metadata: MetadataLimits::default(),
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
    #[serde(rename = "MzCalibration", default)]
    pub mz_calibration_id: Option<u64>,
    #[serde(rename = "T1", default)]
    pub temperature: Option<f64>,
    #[serde(rename = "Polarity", default)]
    pub polarity: Option<String>,
    #[serde(rename = "AccumulationTime", default)]
    pub accumulation_time_ms: Option<f64>,
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
    mobility_rows: OnceCell<BTreeMap<u64, MetadataRow>>,
    mz_rows: OnceCell<BTreeMap<u64, MetadataRow>>,
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
        let metadata =
            MetadataDatabase::with_limits(directory.join("analysis.tdf"), limits.metadata)?;
        let global = metadata
            .table("GlobalMetadata")?
            .into_iter()
            .map(canonical_columns)
            .collect::<Result<Vec<_>>>()?;
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
            let frame: FrameInfo =
                serde_json::from_value(serde_json::Value::Object(canonical_columns(row)?))?;
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
            mobility_rows: OnceCell::new(),
            mz_rows: OnceCell::new(),
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
        let row = self
            .calibration_rows("TimsCalibration", &self.mobility_rows)?
            .get(&id)
            .ok_or_else(|| invalid("missing referenced mobility calibration"))?;
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

impl TdfReader {
    fn calibration_rows<'a>(
        &self,
        table: &str,
        cache: &'a OnceCell<BTreeMap<u64, MetadataRow>>,
    ) -> Result<&'a BTreeMap<u64, MetadataRow>> {
        if let Some(rows) = cache.get() {
            return Ok(rows);
        }
        let mut rows = BTreeMap::new();
        for row in self.metadata.table(table)? {
            let row = canonical_columns(row)?;
            let id = row
                .get("Id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| invalid("invalid calibration identifier"))?;
            if rows.insert(id, row).is_some() {
                return Err(invalid("duplicate calibration identifier"));
            }
        }
        Ok(cache.get_or_init(|| rows))
    }

    /// Exact supported source model for this frame, including its recorded temperature.
    /// Unknown models and higher-order terms return Unsupported instead of approximate masses.
    pub fn mz_model(&self, frame: u64) -> Result<MzModel> {
        let frame = self
            .frames
            .get(&frame)
            .ok_or_else(|| invalid("unknown frame"))?;
        let id = frame
            .mz_calibration_id
            .ok_or_else(|| invalid("frame has no MzCalibration reference"))?;
        let temperature = frame
            .temperature
            .ok_or_else(|| invalid("frame has no T1 temperature"))?;
        let row = self
            .calibration_rows("MzCalibration", &self.mz_rows)?
            .get(&id)
            .ok_or_else(|| invalid("missing referenced m/z calibration"))?;
        MzModel::from_metadata(row, temperature)
    }
}

// SQLite identifiers are ASCII case insensitive. Public metadata retains source spelling.
fn canonical_columns(row: MetadataRow) -> Result<MetadataRow> {
    const NAMES: &[&str] = &[
        "Id",
        "Time",
        "MsMsType",
        "NumScans",
        "NumPeaks",
        "TimsId",
        "TimsCalibration",
        "MzCalibration",
        "T1",
        "T2",
        "Polarity",
        "AccumulationTime",
        "Key",
        "Value",
        "ModelType",
        "DigitizerTimebase",
        "DigitizerDelay",
        "dC1",
        "dC2",
        "C0",
        "C1",
        "C2",
        "C3",
        "C4",
        "C5",
        "C6",
        "C7",
        "C8",
        "C9",
    ];
    let mut result = MetadataRow::new();
    for (name, value) in row {
        let canonical = NAMES.iter().find(|key| key.eq_ignore_ascii_case(&name));
        let name = canonical.map(|s| (*s).to_owned()).unwrap_or(name);
        if result.insert(name, value).is_some() {
            return Err(invalid("ambiguous metadata column spelling"));
        }
    }
    Ok(result)
}
