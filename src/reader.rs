//! File-backed acquisition access over validated TDF metadata and raw frame decoding.
use crate::{
    Error, ErrorContext, Frame, FrameInfo, Limits, MetadataDatabase, MobilityModel, MzModel,
    Result, compression, invalid, limit, schema::TdfMetadata,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub struct TdfReader {
    directory: PathBuf,
    metadata: TdfMetadata,
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
        Self::open_inner(path, limits).map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(path.to_path_buf()),
                ..Default::default()
            })
        })
    }

    fn open_inner(path: &Path, limits: Limits) -> Result<Self> {
        let directory = path
            .ancestors()
            .find(|p| p.join("analysis.tdf").is_file() && p.join("analysis.tdf_bin").is_file())
            .ok_or_else(|| {
                Error::Unsupported(
                    "expected a local TDF directory with analysis.tdf and analysis.tdf_bin".into(),
                )
            })?
            .to_path_buf();
        let metadata = TdfMetadata::open(directory.join("analysis.tdf"), limits.metadata)?;
        let compression = metadata.compression()?;
        let binary = File::open(directory.join("analysis.tdf_bin"))?;
        let binary_length = binary.metadata()?.len();
        let frames = metadata.frames(limits, binary_length)?;
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
        &self.metadata.database
    }
    /// Sorted by the original identifier. Sparse identifiers are retained.
    pub fn frames(&self) -> impl Iterator<Item = &FrameInfo> {
        self.frames.values()
    }

    pub fn read_frame(&mut self, id: u64) -> Result<Frame> {
        let offset = self.frames.get(&id).map(|frame| frame.binary_offset);
        self.read_frame_inner(id).map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tdf_bin")),
                frame_id: Some(id),
                byte_offset: offset,
                ..Default::default()
            })
        })
    }

    fn read_frame_inner(&mut self, id: u64) -> Result<Frame> {
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
        if bytes > self.limits.max_compressed_bytes {
            return Err(limit(
                "compressed_bytes",
                self.limits.max_compressed_bytes,
                bytes,
            ));
        }
        let header_field = |error: Error, field: &str| {
            error.with_context(ErrorContext {
                field: Some(field.into()),
                ..Default::default()
            })
        };
        if bytes < 8 {
            return Err(header_field(
                invalid("frame size is smaller than its header"),
                "frame_size",
            ));
        }
        if scans != info.scan_count {
            return Err(header_field(
                invalid(format!(
                    "binary scan count {scans} differs from metadata count {}",
                    info.scan_count
                )),
                "NumScans",
            ));
        }
        if info
            .binary_offset
            .checked_add(bytes as u64)
            .is_none_or(|end| end > binary_length)
        {
            return Err(header_field(
                invalid("frame extent is outside binary file"),
                "frame_size",
            ));
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
        let result = self
            .frames
            .get(&frame)
            .ok_or_else(|| invalid("unknown frame"))
            .and_then(|frame| self.metadata.mobility_model(frame));
        result.map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tdf")),
                frame_id: Some(frame),
                table: Some("TimsCalibration".into()),
                ..Default::default()
            })
        })
    }

    /// Exact supported source model for this frame, including its recorded temperature.
    /// Unknown models and higher-order terms return Unsupported instead of approximate masses.
    pub fn mz_model(&self, frame: u64) -> Result<MzModel> {
        let result = self
            .frames
            .get(&frame)
            .ok_or_else(|| invalid("unknown frame"))
            .and_then(|frame| self.metadata.mz_model(frame));
        result.map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tdf")),
                frame_id: Some(frame),
                table: Some("MzCalibration".into()),
                ..Default::default()
            })
        })
    }
}
