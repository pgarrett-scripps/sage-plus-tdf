//! File-backed acquisition access over validated TDF metadata and raw frame decoding.
use crate::{
    AcquisitionType, DiaFrameMsMsInfo, DiaFrameMsMsWindow, DiaFrameMsMsWindowGroup, Error,
    ErrorContext, Frame, FrameInfo, GlobalMetadata, Limits, LinearMobilityScale, LinearMzScale,
    MetadataDatabase, MobilityModel, MzCalibration, MzModel, PasefFrameMsMsInfo, Precursor, Result,
    TimsCalibration, compression, invalid, limit, schema::TdfMetadata,
};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

/// An open acquisition. All methods take `&self`, and frames can be read from
/// many threads at once: the binary file is read with positional reads, never
/// a shared cursor.
pub struct TdfReader {
    directory: PathBuf,
    metadata: TdfMetadata,
    global: GlobalMetadata,
    binary: File,
    binary_length: u64,
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
        let global = metadata.global_metadata()?;
        let compression = global.compression_type()?;
        if !matches!(compression, 1 | 2) {
            return Err(crate::field(
                Error::Unsupported(format!("compression {compression}")),
                "GlobalMetadata",
                "TimsCompressionType",
            ));
        }
        let binary = File::open(directory.join("analysis.tdf_bin"))?;
        let binary_length = binary.metadata()?.len();
        let frames = metadata.frames(limits, binary_length)?;
        Ok(Self {
            directory,
            metadata,
            global,
            binary,
            binary_length,
            frames,
            compression,
            limits,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Untyped access to every table and view, including vendor extensions.
    pub fn metadata(&self) -> &MetadataDatabase {
        &self.metadata.database
    }
    pub fn global_metadata(&self) -> &GlobalMetadata {
        &self.global
    }
    /// Sorted by the original identifier. Sparse identifiers are retained.
    pub fn frames(&self) -> impl Iterator<Item = &FrameInfo> {
        self.frames.values()
    }
    pub fn frame(&self, id: u64) -> Option<&FrameInfo> {
        self.frames.get(&id)
    }
    /// Largest `Frames.NumScans`, or 0 without frames.
    pub fn max_scan_count(&self) -> usize {
        self.frames
            .values()
            .map(|f| f.scan_count)
            .max()
            .unwrap_or(0)
    }
    /// `TimsCompressionType` (1 LZF, 2 Zstandard).
    pub fn compression_type(&self) -> u32 {
        self.compression
    }
    pub fn acquisition_type(&self) -> AcquisitionType {
        if self.frames.values().any(|f| f.msms_type == 8) {
            AcquisitionType::DdaPasef
        } else if self.frames.values().any(|f| f.msms_type == 9) {
            AcquisitionType::DiaPasef
        } else {
            AcquisitionType::Unknown
        }
    }

    // Typed tables. A table missing from the file has no rows.
    pub fn pasef_frame_msms_info(&self) -> Result<Vec<PasefFrameMsMsInfo>> {
        self.in_tdf(self.metadata.pasef_frame_msms_info())
    }
    pub fn precursors(&self) -> Result<Vec<Precursor>> {
        self.in_tdf(self.metadata.precursors())
    }
    pub fn dia_frame_msms_info(&self) -> Result<Vec<DiaFrameMsMsInfo>> {
        self.in_tdf(self.metadata.dia_frame_msms_info())
    }
    pub fn dia_frame_msms_windows(&self) -> Result<Vec<DiaFrameMsMsWindow>> {
        self.in_tdf(self.metadata.dia_frame_msms_windows())
    }
    pub fn dia_frame_msms_window_groups(&self) -> Result<Vec<DiaFrameMsMsWindowGroup>> {
        self.in_tdf(self.metadata.dia_frame_msms_window_groups())
    }
    /// `TimsCalibration` rows by `Id`. `FrameInfo::mobility_calibration_id` refers here.
    pub fn tims_calibrations(&self) -> Result<Vec<TimsCalibration>> {
        let rows = self
            .metadata
            .tims_calibrations()
            .map(|m| m.values().cloned().collect());
        self.in_tdf(rows)
    }
    /// `MzCalibration` rows by `Id`. `FrameInfo::mz_calibration_id` refers here.
    pub fn mz_calibrations(&self) -> Result<Vec<MzCalibration>> {
        let rows = self
            .metadata
            .mz_calibrations()
            .map(|m| m.values().copied().collect());
        self.in_tdf(rows)
    }

    /// The uncalibrated m/z scale timsrust 0.6 uses: GlobalMetadata
    /// `MzAcqRange*` (widened by 5 for `Bruker otofControl`) over `DigitizerNumSamples`.
    pub fn linear_mz_scale(&self) -> Result<LinearMzScale> {
        let (mut lower, mut upper) = self.global.mz_acq_range()?;
        if self.global.acquisition_software() == Some("Bruker otofControl") {
            lower -= 5.0;
            upper += 5.0;
        }
        LinearMzScale::new(lower, upper, self.global.digitizer_num_samples()?)
    }

    /// The uncalibrated mobility scale timsrust 0.6 uses: GlobalMetadata
    /// `OneOverK0AcqRange*` over scans 0 to `max_scan_count() - 1`.
    pub fn linear_mobility_scale(&self) -> Result<LinearMobilityScale> {
        let (lower, upper) = self.global.one_over_k0_acq_range()?;
        let last = self
            .max_scan_count()
            .checked_sub(1)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid("no scans to span the mobility range"))?;
        LinearMobilityScale::new(lower, upper, last)
    }

    fn in_tdf<T>(&self, result: Result<T>) -> Result<T> {
        result.map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tdf")),
                ..Default::default()
            })
        })
    }

    pub fn read_frame(&self, id: u64) -> Result<Frame> {
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

    fn read_frame_inner(&self, id: u64) -> Result<Frame> {
        let info = self
            .frames
            .get(&id)
            .ok_or_else(|| invalid(format!("unknown frame {id}")))?;
        let mut header = [0u8; 8];
        read_exact_at(&self.binary, &mut header, info.binary_offset)?;
        let bytes = compression::word(&header, 0)? as usize;
        let scans = compression::word(&header, 4)? as usize;
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
            .is_none_or(|end| end > self.binary_length)
        {
            return Err(header_field(
                invalid("frame extent is outside binary file"),
                "frame_size",
            ));
        }
        let mut data = vec![0u8; bytes - 8];
        read_exact_at(&self.binary, &mut data, info.binary_offset + 8)?;
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

#[cfg(unix)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buffer: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buffer.is_empty() {
        match file.seek_read(buffer, offset) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buffer = &mut buffer[n..];
                offset += n as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<TdfReader>();
};
