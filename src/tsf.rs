//! TSF acquisitions (`analysis.tsf` + `analysis.tsf_bin`): one spectrum per
//! frame and no mobility dimension, as written by timsTOF fleX, MALDI and
//! TIMS-off methods. Line (centroid) spectra are read; profile spectra are not.
//!
//! The line spectrum layout follows MannLabs TimsRust `timsrust-tsf`; the
//! `FrameMsMsInfo` precursor table follows ProteoWizard's Bruker TSF reader.
use crate::{
    DecodeBuffers, Error, ErrorContext, GlobalMetadata, Limits, LineSpectrum, LinearMzScale,
    MetadataDatabase, MzCalibration, MzModel, Result, TsfFrameInfo, TsfFrameMsMsInfo, compression,
    invalid, limit, reader::read_exact_at, schema::TdfMetadata,
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
    /// GlobalMetadata `HasLineSpectra` is 1. A missing or malformed value reads
    /// as `false`; [`Self::read_line_spectrum`] tells the two apart.
    pub fn has_line_spectra(&self) -> bool {
        self.line_spectra_flag().unwrap_or(false)
    }

    /// `HasLineSpectra` as recorded: an absent key is `false`, a value that is
    /// not an integer is corrupt data rather than an absence of line spectra.
    fn line_spectra_flag(&self) -> Result<bool> {
        if self.global.get("HasLineSpectra").is_none() {
            return Ok(false);
        }
        Ok(self.global.integer("HasLineSpectra")? == 1)
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
        let mut buffers = DecodeBuffers::default();
        let mut spectrum = LineSpectrum::default();
        self.read_line_spectrum_into_buffers(id, &mut buffers, &mut spectrum)?;
        Ok(spectrum)
    }

    /// Read a line spectrum into buffers the caller keeps. `data` takes the
    /// compressed bytes and `spectrum` the decoded peaks; both are cleared
    /// first and keep their capacity, so reading many spectra in a loop
    /// allocates once instead of per spectrum. Decoding and every limit are
    /// those of [`read_line_spectrum`](Self::read_line_spectrum). On error both
    /// buffers hold partial data and must not be read.
    pub fn read_line_spectrum_into(
        &self,
        id: u64,
        data: &mut Vec<u8>,
        spectrum: &mut LineSpectrum,
    ) -> Result<()> {
        // The decoder's own output buffer is allocated per call here.
        // `read_line_spectrum_into_buffers` keeps that one too.
        let mut decoded = Vec::new();
        self.read_line_spectrum_into_parts(id, data, &mut decoded, spectrum)
    }

    /// Read a line spectrum through buffers the caller keeps, including the one
    /// the Zstandard decoder writes into, which
    /// [`read_line_spectrum_into`](Self::read_line_spectrum_into) still
    /// allocates per call. Decoding and every limit are those of
    /// [`read_line_spectrum`](Self::read_line_spectrum). On error the buffers
    /// and the spectrum hold partial data and must not be read.
    pub fn read_line_spectrum_into_buffers(
        &self,
        id: u64,
        buffers: &mut DecodeBuffers,
        spectrum: &mut LineSpectrum,
    ) -> Result<()> {
        self.read_line_spectrum_into_parts(
            id,
            &mut buffers.compressed,
            &mut buffers.decoded,
            spectrum,
        )
    }

    fn read_line_spectrum_into_parts(
        &self,
        id: u64,
        data: &mut Vec<u8>,
        decoded: &mut Vec<u8>,
        spectrum: &mut LineSpectrum,
    ) -> Result<()> {
        let offset = self.frames.get(&id).and_then(|frame| frame.binary_offset);
        self.read_line_inner(id, data, decoded, spectrum)
            .map_err(|error| {
                error.with_context(ErrorContext {
                    path: Some(self.directory.join("analysis.tsf_bin")),
                    frame_id: Some(id),
                    byte_offset: offset,
                    ..Default::default()
                })
            })
    }

    fn read_line_inner(
        &self,
        id: u64,
        data: &mut Vec<u8>,
        decoded: &mut Vec<u8>,
        spectrum: &mut LineSpectrum,
    ) -> Result<()> {
        data.clear();
        spectrum.frame_id = id;
        spectrum.indices.clear();
        spectrum.intensities.clear();
        if !self.line_spectra_flag()? {
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
        let Some(offset) = info.binary_offset else {
            return if peaks == 0 {
                Ok(())
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
                Ok(())
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
        data.resize(compressed, 0);
        read_exact_at(&self.binary, data, offset + 8)?;
        compression::zstd_into(data, expected, self.limits, decoded)?;
        if decoded.len() != expected {
            return Err(invalid(
                "decoded line spectrum length disagrees with NumPeaks",
            ));
        }
        crate::tsf_blob::line_spectrum_peaks(
            decoded,
            peaks,
            &mut spectrum.indices,
            &mut spectrum.intensities,
        )
    }
}

const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<TsfReader>();
};
