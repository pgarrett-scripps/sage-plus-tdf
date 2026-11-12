//! Print a digest of a Bruker acquisition as JSON. The format is taken from the
//! directory's contents: `analysis.tdf` digests every raw frame and the typed
//! tables, `analysis.tsf` every line spectrum, and a `<name>.ms2spectrum.*`
//! pair every miniTDF spectrum. Two runs of the same acquisition can be diffed,
//! and the `raw_digest` of two readers compared.
use sage_plus_tdf::{Frame, LineSpectrum, TdfReader, TsfReader};
use sha2::{Digest, Sha256};
use std::path::Path;

type Report = Result<serde_json::Value, Box<dyn std::error::Error>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect acquisition.d")?;
    let path = Path::new(&path);
    let report = if path.join("analysis.tdf").is_file() {
        tdf(path)?
    } else if path.join("analysis.tsf").is_file() {
        tsf(path)?
    } else {
        minitdf(path)?
    };
    println!("{report}");
    Ok(())
}

/// A TDF acquisition: frames, calibration and the typed tables.
fn tdf(path: &Path) -> Report {
    let reader = TdfReader::open(path)?;
    let ids: Vec<_> = reader.frames().map(|f| f.id).collect();
    let mut peaks = 0u64;
    let mut hash = Sha256::new();
    // One pair of buffers for the whole acquisition, as `read_frame_into` allows.
    let mut data = Vec::new();
    let mut frame = Frame::default();
    for id in &ids {
        reader.read_frame_into(*id, &mut data, &mut frame)?;
        reader.mz_model(*id)?;
        reader.mobility_model(*id)?;
        hash.update(id.to_le_bytes());
        for offset in &frame.scan_offsets {
            hash.update((*offset as u64).to_le_bytes());
        }
        for (tof, intensity) in frame.tof_indices.iter().zip(&frame.intensities) {
            hash.update(tof.to_le_bytes());
            hash.update(intensity.to_le_bytes());
        }
        peaks += frame.tof_indices.len() as u64;
    }
    Ok(serde_json::json!({
        "format": "tdf",
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
    }))
}

/// A TSF acquisition: one line spectrum per frame and no mobility dimension.
fn tsf(path: &Path) -> Report {
    let reader = TsfReader::open(path)?;
    let ids: Vec<_> = reader.frames().map(|f| f.id).collect();
    let levels = reader.frames().filter(|f| f.ms_level() == 2).count();
    let mut peaks = 0u64;
    let mut hash = Sha256::new();
    let mut data = Vec::new();
    let mut spectrum = LineSpectrum::default();
    if reader.has_line_spectra() {
        for id in &ids {
            reader.read_line_spectrum_into(*id, &mut data, &mut spectrum)?;
            reader.mz_model(*id)?;
            hash.update(id.to_le_bytes());
            for (index, intensity) in spectrum.indices.iter().zip(&spectrum.intensities) {
                hash.update(index.to_bits().to_le_bytes());
                hash.update(intensity.to_bits().to_le_bytes());
            }
            peaks += spectrum.indices.len() as u64;
        }
    }
    Ok(serde_json::json!({
        "format": "tsf",
        "frames": ids.len(),
        "ms2_frames": levels,
        "has_line_spectra": reader.has_line_spectra(),
        "peaks": peaks,
        "raw_digest": format!("{:x}", hash.finalize()),
        "frame_msms_info": reader.frame_msms_info()?.len(),
        "mz_calibrations": reader.mz_calibrations()?.len(),
        "linear_mz_scale": reader.linear_mz_scale().is_ok(),
    }))
}

/// A ProteoScape miniTDF spectrum pair. Requires the `minitdf` feature.
#[cfg(feature = "minitdf")]
fn minitdf(path: &Path) -> Report {
    let reader = sage_plus_tdf::MiniTdfReader::open(path)?;
    let mut peaks = 0u64;
    let mut hash = Sha256::new();
    let mut data = Vec::new();
    let mut spectrum = sage_plus_tdf::MiniTdfSpectrum::default();
    for index in 0..reader.len() {
        reader.read_spectrum_into(index, &mut data, &mut spectrum)?;
        hash.update((index as u64).to_le_bytes());
        for (mz, intensity) in spectrum.mz.iter().zip(&spectrum.intensities) {
            hash.update(mz.to_bits().to_le_bytes());
            hash.update(intensity.to_bits().to_le_bytes());
        }
        peaks += spectrum.mz.len() as u64;
    }
    let precursors = reader.precursors();
    Ok(serde_json::json!({
        "format": "minitdf",
        "spectra": reader.len(),
        "peaks": peaks,
        "raw_digest": format!("{:x}", hash.finalize()),
        "precursors": precursors.len(),
        "precursors_with_monoisotopic_mz": precursors
            .iter()
            .filter(|p| p.monoisotopic_mz.is_some())
            .count(),
        "precursors_with_mobility": precursors.iter().filter(|p| p.one_over_k0.is_some()).count(),
    }))
}

#[cfg(not(feature = "minitdf"))]
fn minitdf(path: &Path) -> Report {
    Err(format!(
        "{} holds no analysis.tdf or analysis.tsf; rebuild with --features minitdf to inspect \
         miniTDF spectra",
        path.display()
    )
    .into())
}
