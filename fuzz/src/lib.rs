#![forbid(unsafe_code)]
#![allow(dead_code)]
// Compile the private source modules in a standalone, unpublished harness.
// The shipped crate exposes no decoder entry point for unvalidated buffers.
pub use timsrust_reader::{
    Error, ErrorContext, Frame, FrameInfo, Limits, MetadataDatabase, MetadataLimits, MetadataRow,
    MobilityModel, MzCalibration, MzModel, Result,
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
    let mut context = ErrorContext::default();
    context.table = Some(table.into());
    context.field = Some(name.into());
    error.with_context(context)
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
    if let Ok(row) = serde_json::from_slice::<MetadataRow>(data)
        && let Ok(row) = schema::canonical_columns(row)
    {
        if let Ok(frame) = schema::parse_frame(&row) {
            let _ = schema::validate_frame(&frame, Limits::default(), 1_000_000);
        }
        if let Ok(calibration) = schema::mz_calibration(&row)
            && let Ok(model) = MzModel::new(&calibration, 25.0, 26.0)
        {
            for tof in [0.0, 100_000.0, f64::from(u32::MAX)] {
                if let Ok(mz) = model.mz(tof) {
                    let _ = model.tof_index(mz);
                }
            }
        }
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
