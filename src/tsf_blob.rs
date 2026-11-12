//! The TSF line spectrum blob: the only byte parser TSF needs, in its own
//! file so the fuzz harness can compile it without `analysis.tsf` or a `File`.
use crate::{Result, invalid};

/// Split a decoded line spectrum into its two arrays: `peaks` little-endian
/// f64 TOF indices followed by `peaks` little-endian f32 intensities. Every
/// value must be finite and nonnegative, so a corrupt blob cannot reach a
/// caller as a NaN index or intensity. `indices` and `intensities` are cleared
/// first and keep their capacity; on error they hold partial data.
pub(crate) fn line_spectrum_peaks(
    decoded: &[u8],
    peaks: usize,
    indices: &mut Vec<f64>,
    intensities: &mut Vec<f32>,
) -> Result<()> {
    indices.clear();
    intensities.clear();
    if peaks.checked_mul(12) != Some(decoded.len()) {
        return Err(invalid(
            "decoded line spectrum length disagrees with NumPeaks",
        ));
    }
    let (index_bytes, intensity_bytes) = decoded.split_at(peaks * 8);
    indices.reserve(peaks);
    indices.extend(
        index_bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| f64::from_le_bytes(*b)),
    );
    if indices.iter().any(|i| !i.is_finite() || *i < 0.0) {
        return Err(invalid("invalid line spectrum index"));
    }
    intensities.reserve(peaks);
    intensities.extend(
        intensity_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b)),
    );
    if intensities.iter().any(|i| !i.is_finite() || *i < 0.0) {
        return Err(invalid("invalid line spectrum intensity"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blob(indices: &[f64], intensities: &[f32]) -> Vec<u8> {
        let mut bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        bytes.extend(intensities.iter().flat_map(|i| i.to_le_bytes()));
        bytes
    }
    fn decode(decoded: &[u8], peaks: usize) -> Result<(Vec<f64>, Vec<f32>)> {
        let mut indices = Vec::new();
        let mut intensities = Vec::new();
        line_spectrum_peaks(decoded, peaks, &mut indices, &mut intensities)?;
        Ok((indices, intensities))
    }
    #[test]
    fn keeps_finite_nonnegative_peaks() {
        let (indices, intensities) = decode(&blob(&[0.0, 221.5], &[0.0, 250.5]), 2).unwrap();
        assert_eq!(indices, [0.0, 221.5]);
        assert_eq!(intensities, [0.0, 250.5]);
    }
    #[test]
    fn rejects_nonfinite_or_negative_intensities() {
        for intensities in [[1.0, f32::NAN], [1.0, f32::INFINITY], [1.0, -1.0]] {
            let error = decode(&blob(&[1.0, 2.0], &intensities), 2).unwrap_err();
            assert_eq!(error.kind(), crate::ErrorKind::CorruptData);
        }
    }
    #[test]
    fn rejects_nonfinite_or_negative_indices_and_wrong_lengths() {
        for indices in [[1.0, f64::NAN], [1.0, f64::NEG_INFINITY], [1.0, -1.0]] {
            assert!(decode(&blob(&indices, &[1.0, 2.0]), 2).is_err());
        }
        assert!(decode(&blob(&[1.0], &[1.0]), 2).is_err());
    }
}
