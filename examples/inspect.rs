//! Print a digest of every raw frame plus typed-table counts as JSON.
use sage_plus_tdf::TdfReader;
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect acquisition.d")?;
    let reader = TdfReader::open(path)?;
    let ids: Vec<_> = reader.frames().map(|f| f.id).collect();
    let mut peaks = 0u64;
    let mut hash = Sha256::new();
    for id in &ids {
        let frame = reader.read_frame(*id)?;
        reader.mz_model(*id)?;
        reader.mobility_model(*id)?;
        hash.update(id.to_le_bytes());
        for offset in frame.scan_offsets {
            hash.update((offset as u64).to_le_bytes());
        }
        for (tof, intensity) in frame.tof_indices.iter().zip(&frame.intensities) {
            hash.update(tof.to_le_bytes());
            hash.update(intensity.to_le_bytes());
        }
        peaks += frame.tof_indices.len() as u64;
    }
    println!(
        "{}",
        serde_json::json!({
            "frames": ids.len(),
            "peaks": peaks,
            "raw_digest": format!("{:x}", hash.finalize()),
            "acquisition": format!("{:?}", reader.acquisition_type()),
            "compression": reader.compression_type(),
            "max_scans": reader.max_scan_count(),
            "pasef_frame_msms_info": reader.pasef_frame_msms_info()?.len(),
            "precursors": reader.precursors()?.len(),
            "dia_frame_msms_info": reader.dia_frame_msms_info()?.len(),
            "dia_frame_msms_windows": reader.dia_frame_msms_windows()?.len(),
            "tims_calibrations": reader.tims_calibrations()?.len(),
            "linear_mz_scale": reader.linear_mz_scale().is_ok(),
            "linear_mobility_scale": reader.linear_mobility_scale().is_ok(),
        })
    );
    Ok(())
}
