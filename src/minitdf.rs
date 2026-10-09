//! ProteoScape miniTDF MS2 spectra: a `<name>.ms2spectrum.parquet` precursor
//! table and a `<name>.ms2spectrum.bin` of Zstandard blobs with centroided
//! m/z and intensities. Requires the `minitdf` feature.
//!
//! The blob layout and the isolation width heuristic follow MannLabs TimsRust
//! `timsrust-minitdf` 0.6.6.
use crate::{Error, ErrorContext, Limits, Result, compression, invalid, limit};
use parquet::{
    file::reader::{FileReader, SerializedFileReader},
    record::{Field, Row},
};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

const SUFFIX: &str = "ms2spectrum";

/// One row of the `ms2spectrum.parquet` precursor table.
#[derive(Clone, Debug, PartialEq)]
pub struct MiniTdfPrecursor {
    /// `Id`.
    pub id: u64,
    pub scan_mode: Option<i64>,
    /// `RetentionTime`, in seconds.
    pub retention_time_seconds: f64,
    pub largest_peak_mz: Option<f64>,
    pub average_mz: Option<f64>,
    pub monoisotopic_mz: Option<f64>,
    pub charge: Option<i64>,
    pub intensity: Option<f64>,
    /// `ooK0`, inverse reduced mobility.
    pub one_over_k0: Option<f64>,
    pub peak_count: Option<usize>,
    /// `MS1ParentFrameId`.
    pub parent_frame: Option<u64>,
    /// `BinaryOffset` of the spectrum blob in `ms2spectrum.bin`.
    pub binary_offset: u64,
    pub collision_energy: Option<f64>,
}

impl MiniTdfPrecursor {
    /// The isolation width timsrust 0.6 assumes, since miniTDF does not store
    /// one: 2 below m/z 700, 3 above m/z 800, linear in between. Centered on
    /// the monoisotopic m/z.
    pub fn assumed_isolation_width(mz: f64) -> f64 {
        if mz <= 700.0 {
            2.0
        } else if mz >= 800.0 {
            3.0
        } else {
            2.0 + (mz - 700.0) / 100.0
        }
    }
}

/// A decoded miniTDF spectrum. m/z values are stored as `f64`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MiniTdfSpectrum {
    pub index: usize,
    pub mz: Vec<f64>,
    pub intensities: Vec<f32>,
}

/// An open miniTDF spectrum file pair. All methods take `&self`.
pub struct MiniTdfReader {
    binary_path: PathBuf,
    parquet_path: PathBuf,
    binary: File,
    binary_length: u64,
    precursors: Vec<MiniTdfPrecursor>,
    limits: Limits,
}

impl MiniTdfReader {
    /// Open a directory holding exactly one `ms2spectrum.bin` or
    /// `<name>.ms2spectrum.bin` and its matching `.parquet`. Parents are
    /// never searched.
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

    /// The `(bin, parquet)` pair in a directory, if it holds miniTDF spectra.
    pub fn find(directory: impl AsRef<Path>) -> Result<Option<(PathBuf, PathBuf)>> {
        let directory = directory.as_ref();
        if !directory.is_dir() {
            return Ok(None);
        }
        let mut found = Vec::new();
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".bin") else {
                continue;
            };
            if (stem == SUFFIX || stem.ends_with(&format!(".{SUFFIX}"))) && path.is_file() {
                let parquet = directory.join(format!("{stem}.parquet"));
                if parquet.is_file() {
                    found.push((path, parquet));
                }
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(found.pop()),
            _ => Err(Error::Unsupported(
                "several miniTDF ms2spectrum files in one directory".into(),
            )),
        }
    }

    fn open_inner(path: &Path, limits: Limits) -> Result<Self> {
        let (binary_path, parquet_path) = Self::find(path)?.ok_or_else(|| {
            Error::Unsupported(
                "expected a directory containing ms2spectrum.bin and ms2spectrum.parquet".into(),
            )
        })?;
        let precursors = read_precursors(&parquet_path, limits).map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(parquet_path.clone()),
                ..Default::default()
            })
        })?;
        let binary = File::open(&binary_path)?;
        let binary_length = binary.metadata()?.len();
        Ok(Self {
            binary_path,
            parquet_path,
            binary,
            binary_length,
            precursors,
            limits,
        })
    }

    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }
    pub fn parquet_path(&self) -> &Path {
        &self.parquet_path
    }
    /// Precursor rows in file order; spectrum `i` belongs to row `i`.
    pub fn precursors(&self) -> &[MiniTdfPrecursor] {
        &self.precursors
    }
    pub fn len(&self) -> usize {
        self.precursors.len()
    }
    pub fn is_empty(&self) -> bool {
        self.precursors.is_empty()
    }

    /// The spectrum of precursor row `index`.
    pub fn read_spectrum(&self, index: usize) -> Result<MiniTdfSpectrum> {
        let offset = self.precursors.get(index).map(|p| p.binary_offset);
        self.read_inner(index).map_err(|error| {
            error.with_context(ErrorContext {
                path: Some(self.binary_path.clone()),
                byte_offset: offset,
                ..Default::default()
            })
        })
    }

    fn read_inner(&self, index: usize) -> Result<MiniTdfSpectrum> {
        let precursor = self
            .precursors
            .get(index)
            .ok_or_else(|| invalid(format!("unknown spectrum {index}")))?;
        let offset = precursor.binary_offset;
        if offset
            .checked_add(8)
            .is_none_or(|end| end > self.binary_length)
        {
            return Err(invalid("spectrum header is outside the binary file"));
        }
        let mut header = [0u8; 8];
        crate::reader::read_exact_at(&self.binary, &mut header, offset)?;
        // Blob size including this header, then the peak count.
        let size = compression::word(&header, 0)? as usize;
        let peaks = compression::word(&header, 4)? as usize;
        if peaks > self.limits.max_peaks {
            return Err(limit("peaks", self.limits.max_peaks, peaks));
        }
        if precursor.peak_count.is_some_and(|n| n != peaks) {
            return Err(invalid("blob peak count disagrees with NumPeaks"));
        }
        let compressed = size
            .checked_sub(8)
            .ok_or_else(|| invalid("spectrum blob is smaller than its header"))?;
        if compressed > self.limits.max_compressed_bytes {
            return Err(limit(
                "compressed_bytes",
                self.limits.max_compressed_bytes,
                compressed,
            ));
        }
        if offset
            .checked_add(size as u64)
            .is_none_or(|end| end > self.binary_length)
        {
            return Err(invalid("spectrum blob is outside the binary file"));
        }
        if compressed == 0 {
            return Err(invalid("empty spectrum blob"));
        }
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
        crate::reader::read_exact_at(&self.binary, &mut data, offset + 8)?;
        let decoded = compression::zstd(&data, expected, self.limits)?;
        if decoded.len() != expected {
            return Err(invalid("decoded spectrum length disagrees with peak count"));
        }
        // Byte-transposed u32 words: byte k of word i is at k * words + i.
        let words = peaks * 3;
        let word = |i: usize| {
            u32::from_le_bytes([
                decoded[i],
                decoded[words + i],
                decoded[2 * words + i],
                decoded[3 * words + i],
            ])
        };
        let mz: Vec<f64> = (0..peaks)
            .map(|i| f64::from_bits(u64::from(word(2 * i)) | (u64::from(word(2 * i + 1)) << 32)))
            .collect();
        if mz.iter().any(|mz| !mz.is_finite() || *mz <= 0.0) {
            return Err(invalid("invalid spectrum m/z"));
        }
        let intensities = (0..peaks)
            .map(|i| f32::from_bits(word(2 * peaks + i)))
            .collect();
        Ok(MiniTdfSpectrum {
            index,
            mz,
            intensities,
        })
    }
}

fn parquet_error(error: parquet::errors::ParquetError) -> Error {
    Error::Decoder {
        format: "Parquet",
        source: Box::new(error),
    }
}

fn read_precursors(path: &Path, limits: Limits) -> Result<Vec<MiniTdfPrecursor>> {
    let reader = SerializedFileReader::new(File::open(path)?).map_err(parquet_error)?;
    let rows = reader.metadata().file_metadata().num_rows();
    let rows = usize::try_from(rows).map_err(|_| invalid("negative row count"))?;
    if rows > limits.metadata.max_rows {
        return Err(limit("rows", limits.metadata.max_rows, rows));
    }
    let mut precursors = Vec::with_capacity(rows);
    for (index, row) in reader
        .get_row_iter(None)
        .map_err(parquet_error)?
        .enumerate()
    {
        let row = row.map_err(parquet_error)?;
        let precursor = parse_precursor(&row).map_err(|error| {
            error.with_context(ErrorContext {
                table: Some(format!("row {index}")),
                ..Default::default()
            })
        })?;
        precursors.push(precursor);
    }
    if precursors.len() != rows {
        return Err(invalid("row count disagrees with file metadata"));
    }
    Ok(precursors)
}

fn parse_precursor(row: &Row) -> Result<MiniTdfPrecursor> {
    let c = Columns(row);
    let retention_time_seconds = c.f64("RetentionTime")?;
    if retention_time_seconds < 0.0 {
        return Err(c.error("RetentionTime", "negative retention time"));
    }
    Ok(MiniTdfPrecursor {
        id: c.u64("Id")?,
        scan_mode: c.opt_i64("ScanMode")?,
        retention_time_seconds,
        largest_peak_mz: c.opt_f64("LargestPeakMz")?,
        average_mz: c.opt_f64("AverageMz")?,
        monoisotopic_mz: c.opt_f64("MonoisotopicMz")?,
        charge: c.opt_i64("Charge")?,
        intensity: c.opt_f64("Intensity")?,
        one_over_k0: c.opt_f64("ooK0")?,
        peak_count: c
            .opt_i64("NumPeaks")?
            .map(|n| usize::try_from(n).map_err(|_| c.error("NumPeaks", "negative count")))
            .transpose()?,
        parent_frame: c
            .opt_i64("MS1ParentFrameId")?
            .map(|n| u64::try_from(n).map_err(|_| c.error("MS1ParentFrameId", "negative id")))
            .transpose()?,
        binary_offset: c.u64("BinaryOffset")?,
        collision_energy: c.opt_f64("CollisionEnergy")?,
    })
}

/// Typed access to one row: integer columns accept any integer type, real
/// columns accept integers and must be finite. NULL and a missing column are
/// both absent.
struct Columns<'a>(&'a Row);

impl Columns<'_> {
    fn error(&self, name: &str, message: &str) -> Error {
        crate::field(invalid(message), "ms2spectrum", name)
    }
    fn get(&self, name: &str) -> Option<&Field> {
        self.0
            .get_column_iter()
            .find(|(column, _)| column.as_str() == name)
            .map(|(_, value)| value)
            .filter(|value| !matches!(value, Field::Null))
    }
    fn opt_i64(&self, name: &str) -> Result<Option<i64>> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let value = match *value {
            Field::Byte(v) => i64::from(v),
            Field::Short(v) => i64::from(v),
            Field::Int(v) => i64::from(v),
            Field::Long(v) => v,
            Field::UByte(v) => i64::from(v),
            Field::UShort(v) => i64::from(v),
            Field::UInt(v) => i64::from(v),
            Field::ULong(v) => {
                i64::try_from(v).map_err(|_| self.error(name, "integer overflow"))?
            }
            _ => return Err(self.error(name, "expected an integer")),
        };
        Ok(Some(value))
    }
    fn u64(&self, name: &str) -> Result<u64> {
        let value = self
            .opt_i64(name)?
            .ok_or_else(|| self.error(name, "missing value"))?;
        u64::try_from(value).map_err(|_| self.error(name, "negative value"))
    }
    fn opt_f64(&self, name: &str) -> Result<Option<f64>> {
        let value = match self.get(name) {
            None => return Ok(None),
            Some(Field::Float(v)) => f64::from(*v),
            Some(Field::Double(v)) => *v,
            Some(_) => match self.opt_i64(name)? {
                Some(v) => v as f64,
                None => return Ok(None),
            },
        };
        if !value.is_finite() {
            return Err(self.error(name, "non-finite value"));
        }
        Ok(Some(value))
    }
    fn f64(&self, name: &str) -> Result<f64> {
        self.opt_f64(name)?
            .ok_or_else(|| self.error(name, "missing value"))
    }
}

const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<MiniTdfReader>();
};
