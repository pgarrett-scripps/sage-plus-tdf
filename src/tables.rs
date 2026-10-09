//! Typed rows of the TDF metadata tables. Values are as stored; nothing is
//! summed, filtered or renumbered.
use crate::{Error, Result, SqlValue, field, invalid};

/// One `Frames` row.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    pub id: u64,
    /// `Time`, in seconds.
    pub retention_time_seconds: f64,
    /// `Polarity`, usually `+` or `-`.
    pub polarity: Option<String>,
    pub scan_mode: Option<u32>,
    pub msms_type: u32,
    /// `TimsId`: byte offset of the frame in `analysis.tdf_bin`.
    pub binary_offset: u64,
    pub max_intensity: Option<u64>,
    pub summed_intensities: Option<u64>,
    /// `NumScans`. LZF (compression 1) frames may hold more scans than this; see
    /// [`crate::Frame::scan_count`].
    pub scan_count: usize,
    pub peak_count: usize,
    pub mz_calibration_id: Option<u64>,
    pub t1: Option<f64>,
    pub t2: Option<f64>,
    pub mobility_calibration_id: Option<u64>,
    pub property_group: Option<u64>,
    /// `AccumulationTime`, in milliseconds. Missing in old schemas.
    pub accumulation_time_ms: Option<f64>,
    /// `RampTime`, in milliseconds.
    pub ramp_time_ms: Option<f64>,
}

impl FrameInfo {
    /// MS level from `MsMsType`, as timsrust maps it: 0 is MS1, 8 (ddaPASEF) and
    /// 9 (diaPASEF) are MS2, anything else is unknown.
    pub fn ms_level(&self) -> Option<u8> {
        match self.msms_type {
            0 => Some(1),
            8 | 9 => Some(2),
            _ => None,
        }
    }

    /// `1000 / AccumulationTime`: the factor timsrust applies to scale detector
    /// counts to a 1 s accumulation. `None` without a positive accumulation time.
    pub fn intensity_correction_factor(&self) -> Option<f64> {
        self.accumulation_time_ms
            .filter(|t| t.is_finite() && *t > 0.0)
            .map(|t| 1000.0 / t)
    }
}

/// Acquisition mode inferred from `Frames.MsMsType`, as timsrust does: ddaPASEF
/// if any frame is type 8, else diaPASEF if any frame is type 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquisitionType {
    DdaPasef,
    DiaPasef,
    Unknown,
}

/// One `PasefFrameMsMsInfo` row: a ddaPASEF isolation in one MS2 frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PasefFrameMsMsInfo {
    pub frame: u64,
    pub scan_num_begin: u32,
    pub scan_num_end: u32,
    pub isolation_mz: f64,
    pub isolation_width: f64,
    pub collision_energy: f64,
    pub precursor: Option<u64>,
}

/// One `Precursors` row.
#[derive(Clone, Debug, PartialEq)]
pub struct Precursor {
    pub id: u64,
    pub largest_peak_mz: f64,
    pub average_mz: f64,
    pub monoisotopic_mz: Option<f64>,
    pub charge: Option<i64>,
    /// Fractional scan number of the precursor's mobility apex.
    pub scan_number: Option<f64>,
    pub intensity: f64,
    /// MS1 frame the precursor was picked from.
    pub parent: Option<u64>,
}

/// One `DiaFrameMsMsInfo` row: the window group a diaPASEF frame used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiaFrameMsMsInfo {
    pub frame: u64,
    pub window_group: u32,
}

/// One `DiaFrameMsMsWindows` row: one isolation window of a window group.
#[derive(Clone, Debug, PartialEq)]
pub struct DiaFrameMsMsWindow {
    pub window_group: u32,
    pub scan_num_begin: u32,
    pub scan_num_end: u32,
    pub isolation_mz: f64,
    pub isolation_width: f64,
    pub collision_energy: f64,
}

/// One `DiaFrameMsMsWindowGroups` row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiaFrameMsMsWindowGroup {
    pub id: u32,
}

/// One `TimsCalibration` row. ModelType 2 rows feed [`crate::MobilityModel`].
#[derive(Clone, Debug, PartialEq)]
pub struct TimsCalibration {
    pub id: u64,
    pub model_type: i64,
    pub coefficients: [f64; 10],
}

/// The `GlobalMetadata` key/value table, in source order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlobalMetadata {
    entries: Vec<(String, SqlValue)>,
}

impl GlobalMetadata {
    pub(crate) fn new(entries: Vec<(String, SqlValue)>) -> Result<Self> {
        for (i, (key, _)) in entries.iter().enumerate() {
            if entries[..i].iter().any(|(k, _)| k == key) {
                return Err(field(
                    invalid(format!("duplicate key {key}")),
                    "GlobalMetadata",
                    "Key",
                ));
            }
        }
        Ok(Self { entries })
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &SqlValue)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn get(&self, key: &str) -> Option<&SqlValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// A text value. Numbers are not converted.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(SqlValue::as_str)
    }

    /// A number stored as text (as Bruker does) or as a SQLite number.
    pub fn number(&self, key: &str) -> Result<f64> {
        let value = match self.get(key) {
            Some(SqlValue::Text(text)) => text.trim().parse::<f64>().ok(),
            Some(value) => value.as_f64(),
            None => return Err(missing(key)),
        };
        value.filter(|v| v.is_finite()).ok_or_else(|| {
            field(
                invalid("value is not a finite number"),
                "GlobalMetadata",
                key,
            )
        })
    }

    /// A nonnegative integer stored as text or as a SQLite integer.
    pub fn integer(&self, key: &str) -> Result<u64> {
        let value = match self.get(key) {
            Some(SqlValue::Text(text)) => text
                .trim()
                .parse::<u64>()
                .map_err(|source| field(Error::ParseInteger(source), "GlobalMetadata", key))?,
            Some(SqlValue::Integer(n)) => u64::try_from(*n)
                .map_err(|_| field(invalid("negative integer"), "GlobalMetadata", key))?,
            Some(_) => {
                return Err(field(
                    invalid("value is not an integer"),
                    "GlobalMetadata",
                    key,
                ));
            }
            None => return Err(missing(key)),
        };
        Ok(value)
    }

    /// `AcquisitionSoftware`, for example `Bruker otofControl` or `timsTOF`.
    pub fn acquisition_software(&self) -> Option<&str> {
        self.text("AcquisitionSoftware")
    }

    /// `TimsCompressionType`: 1 is LZF, 2 is Zstandard.
    pub fn compression_type(&self) -> Result<u32> {
        let value = self.integer("TimsCompressionType")?;
        u32::try_from(value).map_err(|_| {
            field(
                invalid("out of range"),
                "GlobalMetadata",
                "TimsCompressionType",
            )
        })
    }

    /// `(MzAcqRangeLower, MzAcqRangeUpper)` as recorded.
    pub fn mz_acq_range(&self) -> Result<(f64, f64)> {
        Ok((
            self.number("MzAcqRangeLower")?,
            self.number("MzAcqRangeUpper")?,
        ))
    }

    /// `(OneOverK0AcqRangeLower, OneOverK0AcqRangeUpper)` as recorded.
    pub fn one_over_k0_acq_range(&self) -> Result<(f64, f64)> {
        Ok((
            self.number("OneOverK0AcqRangeLower")?,
            self.number("OneOverK0AcqRangeUpper")?,
        ))
    }

    /// `DigitizerNumSamples`: the number of TOF bins.
    pub fn digitizer_num_samples(&self) -> Result<u32> {
        let value = self.integer("DigitizerNumSamples")?;
        u32::try_from(value).map_err(|_| {
            field(
                invalid("out of range"),
                "GlobalMetadata",
                "DigitizerNumSamples",
            )
        })
    }
}

fn missing(key: &str) -> Error {
    field(invalid(format!("missing key {key}")), "GlobalMetadata", key)
}
