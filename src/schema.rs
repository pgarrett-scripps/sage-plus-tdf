//! TDF schema interpretation, separate from generic SQLite access and binary I/O.
use crate::{
    Error, ErrorContext, FrameInfo, Limits, MetadataDatabase, MetadataLimits, MetadataRow,
    MobilityModel, MzCalibration, MzModel, Result, field, invalid, limit,
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
            .ok_or_else(|| {
                field(
                    invalid("missing TimsCompressionType"),
                    "GlobalMetadata",
                    "TimsCompressionType",
                )
            })?
            .parse::<u32>()
            .map_err(|source| {
                field(
                    Error::ParseInteger(source),
                    "GlobalMetadata",
                    "TimsCompressionType",
                )
            })?;
        if !matches!(compression, 1 | 2) {
            return Err(field(
                Error::Unsupported(format!("compression {compression}")),
                "GlobalMetadata",
                "TimsCompressionType",
            ));
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
            let row = canonical_columns(row).map_err(|error| field(error, "Frames", "columns"))?;
            let id = row.get("Id").and_then(|value| value.as_u64());
            let frame = parse_frame(&row)
                .and_then(|frame| {
                    validate_frame(&frame, limits, binary_length)?;
                    Ok(frame)
                })
                .map_err(|error| {
                    error.with_context(ErrorContext {
                        frame_id: id,
                        table: Some("Frames".into()),
                        ..Default::default()
                    })
                })?;
            let id = frame.id;
            if frames.insert(id, frame).is_some() {
                return Err(field(invalid("duplicate frame identifier"), "Frames", "Id")
                    .with_context(ErrorContext {
                        frame_id: Some(id),
                        ..Default::default()
                    }));
            }
        }
        Ok(frames)
    }

    pub(crate) fn mobility_model(&self, frame: &FrameInfo) -> Result<MobilityModel> {
        let id = frame.mobility_calibration_id.ok_or_else(|| {
            field(
                invalid("missing calibration reference"),
                "Frames",
                "TimsCalibration",
            )
        })?;
        let row = self
            .mobility_rows
            .get(&self.database)?
            .get(&id)
            .ok_or_else(|| {
                field(
                    invalid(format!("missing calibration identifier {id}")),
                    "TimsCalibration",
                    "Id",
                )
            })?;
        let model_type = row
            .get("ModelType")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                field(
                    invalid("missing or noninteger model type"),
                    "TimsCalibration",
                    "ModelType",
                )
            })?;
        if model_type != 2 {
            return Err(field(
                Error::Unsupported("mobility ModelType must be 2".into()),
                "TimsCalibration",
                "ModelType",
            ));
        }
        let mut coefficients = [0.0; 10];
        for (i, c) in coefficients.iter_mut().enumerate() {
            *c = row
                .get(&format!("C{i}"))
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    field(
                        invalid("missing calibration coefficient"),
                        "TimsCalibration",
                        &format!("C{i}"),
                    )
                })?;
        }
        MobilityModel::new(coefficients)
    }

    pub(crate) fn mz_model(&self, frame: &FrameInfo) -> Result<MzModel> {
        let id = frame.mz_calibration_id.ok_or_else(|| {
            field(
                invalid("missing calibration reference"),
                "Frames",
                "MzCalibration",
            )
        })?;
        let t1 = frame
            .t1
            .ok_or_else(|| field(invalid("missing frame temperature"), "Frames", "T1"))?;
        let row = self.mz_rows.get(&self.database)?.get(&id).ok_or_else(|| {
            field(
                invalid(format!("missing calibration identifier {id}")),
                "MzCalibration",
                "Id",
            )
        })?;
        let calibration = mz_calibration(row)?;
        // Without a frame T2 the dC2 term is only known when it vanishes.
        let t2 = frame
            .t2
            .or((calibration.dc2 == 0.0).then_some(calibration.t2))
            .ok_or_else(|| field(invalid("missing frame temperature"), "Frames", "T2"))?;
        MzModel::new(&calibration, t1, t2)
    }
}

pub(super) fn mz_calibration(row: &MetadataRow) -> Result<MzCalibration> {
    let number = |key: &str| {
        row.get(key)
            .and_then(|v| v.as_f64())
            .ok_or_else(|| field(invalid("missing m/z coefficient"), "MzCalibration", key))
    };
    Ok(MzCalibration {
        id: row.get("Id").and_then(|v| v.as_u64()).ok_or_else(|| {
            field(
                invalid("invalid calibration identifier"),
                "MzCalibration",
                "Id",
            )
        })?,
        model_type: row
            .get("ModelType")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| field(invalid("invalid model type"), "MzCalibration", "ModelType"))?,
        digitizer_timebase: number("DigitizerTimebase")?,
        digitizer_delay: number("DigitizerDelay")?,
        t1: number("T1")?,
        t2: number("T2")?,
        dc1: number("dC1")?,
        dc2: number("dC2")?,
        c0: number("C0")?,
        c1: number("C1")?,
        c2: number("C2")?,
        c3: number("C3")?,
        c4: number("C4")?,
    })
}

pub(super) fn parse_frame(row: &MetadataRow) -> Result<FrameInfo> {
    fn required<T: serde::de::DeserializeOwned>(row: &MetadataRow, name: &str) -> Result<T> {
        serde_json::from_value(row.get(name).cloned().unwrap_or(serde_json::Value::Null))
            .map_err(|source| field(Error::Metadata(source), "Frames", name))
    }
    fn optional<T: serde::de::DeserializeOwned>(
        row: &MetadataRow,
        name: &str,
    ) -> Result<Option<T>> {
        if row.get(name).is_none_or(|value| value.is_null()) {
            return Ok(None);
        }
        required(row, name).map(Some)
    }
    Ok(FrameInfo {
        id: required(row, "Id")?,
        retention_time_seconds: required(row, "Time")?,
        msms_type: required(row, "MsMsType")?,
        scan_count: required(row, "NumScans")?,
        peak_count: required(row, "NumPeaks")?,
        binary_offset: required(row, "TimsId")?,
        mobility_calibration_id: optional(row, "TimsCalibration")?,
        mz_calibration_id: optional(row, "MzCalibration")?,
        t1: optional(row, "T1")?,
        t2: optional(row, "T2")?,
        polarity: optional(row, "Polarity")?,
        accumulation_time_ms: optional(row, "AccumulationTime")?,
    })
}

pub(super) fn validate_frame(frame: &FrameInfo, limits: Limits, binary_length: u64) -> Result<()> {
    if frame.id == 0 {
        return Err(field(invalid("zero frame identifier"), "Frames", "Id"));
    }
    if !frame.retention_time_seconds.is_finite() || frame.retention_time_seconds < 0.0 {
        return Err(field(invalid("invalid retention time"), "Frames", "Time"));
    }
    if frame.scan_count > limits.max_scans {
        return Err(field(
            limit("scans", limits.max_scans, frame.scan_count),
            "Frames",
            "NumScans",
        ));
    }
    if frame.peak_count > limits.max_peaks {
        return Err(field(
            limit("peaks", limits.max_peaks, frame.peak_count),
            "Frames",
            "NumPeaks",
        ));
    }
    if frame
        .binary_offset
        .checked_add(8)
        .is_none_or(|end| end > binary_length)
    {
        return Err(field(
            invalid("frame header is outside binary file"),
            "Frames",
            "TimsId",
        )
        .with_context(ErrorContext {
            byte_offset: Some(frame.binary_offset),
            ..Default::default()
        }));
    }
    Ok(())
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
            let id = row.get("Id").and_then(|v| v.as_u64()).ok_or_else(|| {
                field(invalid("invalid calibration identifier"), self.table, "Id")
            })?;
            if rows.insert(id, row).is_some() {
                return Err(field(
                    invalid("duplicate calibration identifier"),
                    self.table,
                    "Id",
                ));
            }
        }
        Ok(self.rows.get_or_init(|| rows))
    }
}

// SQLite identifiers are ASCII case insensitive. Public metadata retains source spelling.
pub(super) fn canonical_columns(row: MetadataRow) -> Result<MetadataRow> {
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
        let error = parse_frame(&normalized).unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::CorruptData);
        assert!(
            error
                .contexts()
                .any(|context| context.field.as_deref() == Some("Id"))
        );
        assert!(matches!(error.root_error(), Error::Metadata(_)));
    }
}
