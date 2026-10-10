#![forbid(unsafe_code)]
#![allow(dead_code)]
// Compile the private source modules in a standalone, unpublished harness.
// The shipped crate exposes no decoder entry point for unvalidated buffers.
pub use sage_plus_tdf::{
    DiaFrameMsMsInfo, DiaFrameMsMsWindow, DiaFrameMsMsWindowGroup, Error, ErrorContext, ErrorKind,
    Frame, FrameInfo, GlobalMetadata, Limits, MetadataDatabase, MetadataLimits, MetadataRow,
    MobilityModel, MzCalibration, MzModel, PasefFrameMsMsInfo, Precursor, Result, SqlValue,
    TimsCalibration, TsfFrameInfo, TsfFrameMsMsInfo,
};

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn limit(resource: &'static str, maximum: usize, observed: usize) -> Error {
    Error::ResourceLimit {
        resource,
        maximum: maximum as u64,
        observed: observed as u64,
        source: None,
    }
}
fn field(error: Error, table: &str, name: &str) -> Error {
    error.with_context(ErrorContext {
        table: Some(table.into()),
        field: Some(name.into()),
        ..Default::default()
    })
}
#[path = "../../src/compression.rs"]
mod compression;
#[path = "../../src/schema.rs"]
mod schema;

pub fn decoder(data: &[u8]) {
    if data.len() < 5 || data.len() > 65_536 {
        return;
    }
    let scans = u16::from_le_bytes([data[1], data[2]]) as usize % 65;
    let peaks = u16::from_le_bytes([data[3], data[4]]) as usize % 2049;
    let limits = Limits {
        max_scans: 64,
        max_peaks: 2048,
        max_compressed_bytes: 65_536,
        max_decompressed_bytes: 32_768,
        ..Default::default()
    };
    let _ = compression::decode(
        1 + u32::from(data[0] & 1),
        1,
        &data[5..],
        scans,
        peaks,
        limits,
    );
}

pub fn schema_calibration(data: &[u8]) {
    if data.len() > 32_768 {
        return;
    }
    if let Some(row) = json_row(data) {
        if let Ok(frame) = schema::parse_frame(&row) {
            let _ = schema::validate_frame(&frame, Limits::default(), 1_000_000);
        }
        let columns = schema::Columns {
            row: &row,
            table: "MzCalibration",
        };
        if let Ok(calibration) = schema::mz_calibration(&columns)
            && let Ok(model) = MzModel::new(&calibration, 25.0, 26.0)
        {
            for tof in [0.0, 100_000.0, f64::from(u32::MAX)] {
                if let Ok(mz) = model.mz(tof) {
                    let _ = model.tof_index(mz);
                }
            }
        }
        let columns = schema::Columns {
            row: &row,
            table: "TimsCalibration",
        };
        let _ = schema::tims_calibration(&columns);
    }
    if data.len() >= 80 {
        let mut coefficients = [0.0; 10];
        for (coefficient, bytes) in coefficients.iter_mut().zip(data[..80].as_chunks::<8>().0) {
            *coefficient = f64::from_le_bytes(*bytes);
        }
        if let Ok(model) = MobilityModel::new(coefficients) {
            for scan in [0.0, 500.0, -1e10, 1e10] {
                let _ = model.one_over_k0(scan);
            }
        }
    }
}

/// A flat JSON object as one SQLite row, typed as SQLite would store it.
fn json_row(data: &[u8]) -> Option<MetadataRow> {
    let serde_json::Value::Object(object) = serde_json::from_slice(data).ok()? else {
        return None;
    };
    let mut columns = Vec::with_capacity(object.len());
    let mut values = Vec::with_capacity(object.len());
    for (name, value) in object {
        let value = match value {
            serde_json::Value::Null => SqlValue::Null,
            serde_json::Value::Bool(b) => SqlValue::Integer(i64::from(b)),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => SqlValue::Integer(i),
                None => SqlValue::Real(n.as_f64()?),
            },
            serde_json::Value::String(s) => SqlValue::Text(s),
            serde_json::Value::Array(bytes) => SqlValue::Blob(
                bytes
                    .iter()
                    .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                    .collect::<Option<_>>()?,
            ),
            serde_json::Value::Object(_) => return None,
        };
        columns.push(name);
        values.push(value);
    }
    MetadataRow::new(columns, values).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn corpus_seeds_run_and_json_rows_parse() {
        for target in ["decoder", "schema_calibration"] {
            for entry in std::fs::read_dir(format!("corpus/{target}")).unwrap() {
                let data = std::fs::read(entry.unwrap().path()).unwrap();
                match target {
                    "decoder" => super::decoder(&data),
                    _ => super::schema_calibration(&data),
                }
            }
        }
        let row = super::json_row(
            br#"{"Id": 1, "Time": 1.0, "MsMsType": 0, "NumScans": 3, "NumPeaks": 3, "TimsId": 0}"#,
        )
        .unwrap();
        assert!(super::schema::parse_frame(&row).is_ok());
    }
}
