//! TSF acquisitions (`analysis.tsf` + `analysis.tsf_bin`): one spectrum per
//! frame and no mobility dimension, as written by timsTOF fleX, MALDI and
//! TIMS-off methods. Line (centroid) spectra are read; profile spectra are not.
//!
//! The line spectrum layout follows MannLabs TimsRust `timsrust-tsf`; the
//! `FrameMsMsInfo` precursor table follows ProteoWizard's Bruker TSF reader.
use crate::{
    Error, ErrorContext, GlobalMetadata, Limits, LineSpectrum, LinearMzScale, MetadataDatabase,
    MzCalibration, MzModel, Result, TsfFrameInfo, TsfFrameMsMsInfo, compression, invalid, limit,
    reader::read_exact_at, schema::TdfMetadata,
};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

/// An open TSF acquisition. All methods take `&self` and frames can be read
/// from many threads at once.
pub struct TsfReader {
    directory: PathBuf,
    metadata: TdfMetadata,
    global: GlobalMetadata,
    binary: File,
    binary_length: u64,
    frames: BTreeMap<u64, TsfFrameInfo>,
    limits: Limits,
}

impl TsfReader {
    /// Open the `.d` directory itself, which must hold `analysis.tsf` and
    /// `analysis.tsf_bin`. Parents are never searched.
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
        if !path.join("analysis.tsf").is_file() || !path.join("analysis.tsf_bin").is_file() {
            return Err(Error::Unsupported(
                "expected a local .d directory containing analysis.tsf and analysis.tsf_bin".into(),
            ));
        }
        let directory = path.to_path_buf();
        let metadata = TdfMetadata::open(directory.join("analysis.tsf"), limits.metadata)?;
        let global = metadata.global_metadata()?;
        let binary = File::open(directory.join("analysis.tsf_bin"))?;
        let binary_length = binary.metadata()?.len();
        let frames = metadata.tsf_frames(limits, binary_length)?;
        Ok(Self {
            directory,
            metadata,
            global,
            binary,
            binary_length,
            frames,
            limits,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Untyped access to every table and view.
    pub fn metadata(&self) -> &MetadataDatabase {
        &self.metadata.database
    }
    pub fn global_metadata(&self) -> &GlobalMetadata {
        &self.global
    }
    /// GlobalMetadata `HasLineSpectra` is 1.
    pub fn has_line_spectra(&self) -> bool {
        self.global.integer("HasLineSpectra").ok() == Some(1)
    }
    /// Sorted by the original identifier.
    pub fn frames(&self) -> impl Iterator<Item = &TsfFrameInfo> {
        self.frames.values()
    }
    pub fn frame(&self, id: u64) -> Option<&TsfFrameInfo> {
        self.frames.get(&id)
    }

    /// `FrameMsMsInfo` rows: the isolation (`TriggerMass`, `IsolationWidth`)
    /// and optional charge of each MS/MS frame. No rows without the table.
    pub fn frame_msms_info(&self) -> Result<Vec<TsfFrameMsMsInfo>> {
        self.in_tsf(self.metadata.tsf_frame_msms_info())
    }

    /// `MzCalibration` rows by `Id`.
    pub fn mz_calibrations(&self) -> Result<Vec<MzCalibration>> {
        let rows = self
            .metadata
            .mz_calibrations()
            .map(|m| m.values().copied().collect());
        self.in_tsf(rows)
    }

    /// The frame's calibrated m/z model (ModelType 1 or 2), from its
    /// `MzCalibration`, `T1` and `T2`.
    pub fn mz_model(&self, frame: u64) -> Result<MzModel> {
        let result = self
            .frames
            .get(&frame)
            .ok_or_else(|| invalid("unknown frame"))
            .and_then(|f| self.metadata.mz_model_for(f.mz_calibration_id, f.t1, f.t2));
        result.map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tsf")),
                frame_id: Some(frame),
                table: Some("MzCalibration".into()),
                ..Default::default()
            })
        })
    }

    /// The uncalibrated scale timsrust 0.6 applies to TSF, as for TDF.
    pub fn linear_mz_scale(&self) -> Result<LinearMzScale> {
        let (mut lower, mut upper) = self.global.mz_acq_range()?;
        if self.global.acquisition_software() == Some("Bruker otofControl") {
            lower -= 5.0;
            upper += 5.0;
        }
        LinearMzScale::new(lower, upper, self.global.digitizer_num_samples()?)
    }

    fn in_tsf<T>(&self, result: Result<T>) -> Result<T> {
        result.map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tsf")),
                ..Default::default()
            })
        })
    }

    /// The frame's line spectrum. Requires `HasLineSpectra` = 1.
    pub fn read_line_spectrum(&self, id: u64) -> Result<LineSpectrum> {
        let offset = self.frames.get(&id).and_then(|frame| frame.binary_offset);
        self.read_line_inner(id).map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.directory.join("analysis.tsf_bin")),
                frame_id: Some(id),
                byte_offset: offset,
                ..Default::default()
            })
        })
    }

    fn read_line_inner(&self, id: u64) -> Result<LineSpectrum> {
        if !self.has_line_spectra() {
            return Err(Error::Unsupported(
                "TSF without line spectra (HasLineSpectra is not 1)".into(),
            ));
        }
        let info = self
            .frames
            .get(&id)
            .ok_or_else(|| invalid(format!("unknown frame {id}")))?;
        let peaks = info
            .peak_count
            .ok_or_else(|| crate::field(invalid("missing peak count"), "Frames", "NumPeaks"))?;
        let empty = LineSpectrum {
            frame_id: id,
            ..Default::default()
        };
        let Some(offset) = info.binary_offset else {
            return if peaks == 0 {
                Ok(empty)
            } else {
                Err(crate::field(
                    invalid("frame with peaks has no binary offset"),
                    "Frames",
                    "TimsId",
                ))
            };
        };
        let mut header = [0u8; 8];
        read_exact_at(&self.binary, &mut header, offset)?;
        // Block size including this header and any profile data, then the
        // compressed length of the line spectrum that follows the header.
        let block = compression::word(&header, 0)? as usize;
        let compressed = compression::word(&header, 4)? as usize;
        if compressed > self.limits.max_compressed_bytes {
            return Err(limit(
                "compressed_bytes",
                self.limits.max_compressed_bytes,
                compressed,
            ));
        }
        if compressed.checked_add(8).is_none_or(|end| end > block)
            || offset
                .checked_add(8 + compressed as u64)
                .is_none_or(|end| end > self.binary_length)
        {
            return Err(invalid("line spectrum extent is outside its block or file"));
        }
        if compressed == 0 {
            return if peaks == 0 {
                Ok(empty)
            } else {
                Err(invalid("empty line spectrum for a frame with peaks"))
            };
        }
        // Each peak is an f64 index and an f32 intensity, stored as two arrays.
        let expected = peaks
            .checked_mul(12)
            .ok_or_else(|| invalid("decoded extent overflow"))?;
        if expected > self.limits.max_decompressed_bytes {
            return Err(limit(
                "decompressed_bytes",
                self.limits.max_decompressed_bytes,
                expected,
            ));
        }
        let mut data = vec![0u8; compressed];
        read_exact_at(&self.binary, &mut data, offset + 8)?;
        let decoded = compression::zstd(&data, expected, self.limits)?;
        if decoded.len() != expected {
            return Err(invalid(
                "decoded line spectrum length disagrees with NumPeaks",
            ));
        }
        let (indices, intensities) = decoded.split_at(peaks * 8);
        let indices: Vec<f64> = indices
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| f64::from_le_bytes(*b))
            .collect();
        if indices.iter().any(|i| !i.is_finite() || *i < 0.0) {
            return Err(invalid("invalid line spectrum index"));
        }
        let intensities = intensities
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        Ok(LineSpectrum {
            frame_id: id,
            indices,
            intensities,
        })
    }
}

const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<TsfReader>();
};
