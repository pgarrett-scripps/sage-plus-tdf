//! TDF schema interpretation, separate from generic SQLite access and binary I/O.
use crate::{
    Error, FrameInfo, Limits, MetadataDatabase, MetadataLimits, MetadataRow, MobilityModel,
    MzModel, Result, invalid,
};
use std::{cell::OnceCell, collections::BTreeMap, path::Path};

pub(crate) struct TdfMetadata {
    pub(crate) database: MetadataDatabase,
    mobility_rows: CalibrationRows,
    mz_rows: CalibrationRows,
}

impl TdfMetadata {
    pub(crate) fn open(path: impl AsRef<Path>, limits: MetadataLimits) -> Result<Self> {
        Ok(Self {
            database: MetadataDatabase::with_limits(path, limits)?,
            mobility_rows: CalibrationRows::new("TimsCalibration"),
            mz_rows: CalibrationRows::new("MzCalibration"),
        })
    }

    pub(crate) fn compression(&self) -> Result<u32> {
        let global = self
            .database
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
        Ok(compression)
    }

    pub(crate) fn frames(
        &self,
        limits: Limits,
        binary_length: u64,
    ) -> Result<BTreeMap<u64, FrameInfo>> {
        let mut frames = BTreeMap::new();
        for row in self.database.table("Frames")? {
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
        Ok(frames)
    }

    pub(crate) fn mobility_model(&self, frame: &FrameInfo) -> Result<MobilityModel> {
        let id = frame
            .mobility_calibration_id
            .ok_or_else(|| invalid("frame has no TimsCalibration reference"))?;
        let row = self
            .mobility_rows
            .get(&self.database)?
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

    pub(crate) fn mz_model(&self, frame: &FrameInfo) -> Result<MzModel> {
        let id = frame
            .mz_calibration_id
            .ok_or_else(|| invalid("frame has no MzCalibration reference"))?;
        let temperature = frame
            .temperature
            .ok_or_else(|| invalid("frame has no T1 temperature"))?;
        let row = self
            .mz_rows
            .get(&self.database)?
            .get(&id)
            .ok_or_else(|| invalid("missing referenced m/z calibration"))?;
        MzModel::from_metadata(row, temperature)
    }
}

// A table and its lazy rows share one owner so callers cannot mix calibration caches.
struct CalibrationRows {
    table: &'static str,
    rows: OnceCell<BTreeMap<u64, MetadataRow>>,
}
impl CalibrationRows {
    fn new(table: &'static str) -> Self {
        Self {
            table,
            rows: OnceCell::new(),
        }
    }

    fn get(&self, database: &MetadataDatabase) -> Result<&BTreeMap<u64, MetadataRow>> {
        if let Some(rows) = self.rows.get() {
            return Ok(rows);
        }
        let mut rows = BTreeMap::new();
        for row in database.table(self.table)? {
            let row = canonical_columns(row)?;
            let id = row
                .get("Id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| invalid("invalid calibration identifier"))?;
            if rows.insert(id, row).is_some() {
                return Err(invalid("duplicate calibration identifier"));
            }
        }
        Ok(self.rows.get_or_init(|| rows))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_columns_reject_ambiguous_source_names() {
        let row = serde_json::from_value(serde_json::json!({"Id": 1, "ID": 2})).unwrap();
        assert!(canonical_columns(row).is_err());
    }

    #[test]
    fn canonical_columns_preserve_unknown_fields_and_strict_integer_types() {
        let row = serde_json::from_value(serde_json::json!({
            "id": 10.0, "time": 1.2, "msmstype": 0, "numscans": 1,
            "numpeaks": 0, "timsid": 0, "VendorExtension": "source value"
        }))
        .unwrap();
        let normalized = canonical_columns(row).unwrap();
        assert_eq!(normalized["VendorExtension"], "source value");
        assert!(
            serde_json::from_value::<FrameInfo>(serde_json::Value::Object(normalized)).is_err()
        );
    }
}
