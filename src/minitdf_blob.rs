//! The miniTDF spectrum blob: the only byte parser miniTDF needs, in its own
//! file so the fuzz harness can compile it without `parquet` or a `File`.
use crate::{Result, invalid};

/// Decode a spectrum blob: byte-transposed u32 words holding `peaks` f64 m/z
/// values followed by `peaks` f32 intensities, where byte `k` of word `i` is at
/// `k * words + i`. Every value must be finite, the m/z positive and the
/// intensities nonnegative, so a corrupt blob cannot reach a caller as a NaN
/// m/z or intensity. `mz` and `intensities` are cleared first and keep their
/// capacity; on error they hold partial data.
pub(crate) fn spectrum_peaks(
    decoded: &[u8],
    peaks: usize,
    mz: &mut Vec<f64>,
    intensities: &mut Vec<f32>,
) -> Result<()> {
    mz.clear();
    intensities.clear();
    if peaks.checked_mul(12) != Some(decoded.len()) {
        return Err(invalid("decoded spectrum length disagrees with peak count"));
    }
    let words = peaks * 3;
    // Byte-transposed layout: the four bytes of word `i` are the `i`-th byte of
    // four equal planes. Splitting the planes once, and the m/z words off the
    // intensity words, gives the same words as indexing each byte separately,
    // because the length check above already proves every index is in range.
    // The first `2 * peaks` words are the f64 m/z values, low word first, and
    // the remaining `peaks` words are the f32 intensities, so neither needs
    // index arithmetic.
    let (plane0, rest) = decoded.split_at(words);
    let (plane1, rest) = rest.split_at(words);
    let (plane2, plane3) = rest.split_at(words);
    let (mz0, intensity0) = plane0.split_at(2 * peaks);
    let (mz1, intensity1) = plane1.split_at(2 * peaks);
    let (mz2, intensity2) = plane2.split_at(2 * peaks);
    let (mz3, intensity3) = plane3.split_at(2 * peaks);
    mz.reserve(peaks);
    mz.extend(
        mz0.as_chunks::<2>()
            .0
            .iter()
            .zip(mz1.as_chunks::<2>().0)
            .zip(mz2.as_chunks::<2>().0)
            .zip(mz3.as_chunks::<2>().0)
            .map(|(((b0, b1), b2), b3)| {
                let low = u32::from_le_bytes([b0[0], b1[0], b2[0], b3[0]]);
                let high = u32::from_le_bytes([b0[1], b1[1], b2[1], b3[1]]);
                f64::from_bits(u64::from(low) | (u64::from(high) << 32))
            }),
    );
    if mz.iter().any(|mz| !mz.is_finite() || *mz <= 0.0) {
        return Err(invalid("invalid spectrum m/z"));
    }
    intensities.reserve(peaks);
    intensities.extend(
        intensity0
            .iter()
            .zip(intensity1)
            .zip(intensity2)
            .zip(intensity3)
            .map(|(((b0, b1), b2), b3)| f32::from_bits(u32::from_le_bytes([*b0, *b1, *b2, *b3]))),
    );
    if intensities.iter().any(|i| !i.is_finite() || *i < 0.0) {
        return Err(invalid("invalid spectrum intensity"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blob(mz: &[f64], intensities: &[f32]) -> Vec<u8> {
        let mut words: Vec<u32> = Vec::new();
        for value in mz {
            let bits = value.to_bits();
            words.push(bits as u32);
            words.push((bits >> 32) as u32);
        }
        words.extend(intensities.iter().map(|i| i.to_bits()));
        (0..4)
            .flat_map(|b| words.iter().map(move |w| w.to_le_bytes()[b]))
            .collect()
    }
    fn decode(decoded: &[u8], peaks: usize) -> Result<(Vec<f64>, Vec<f32>)> {
        let mut mz = Vec::new();
        let mut intensities = Vec::new();
        spectrum_peaks(decoded, peaks, &mut mz, &mut intensities)?;
        Ok((mz, intensities))
    }
    #[test]
    fn keeps_finite_positive_mz_and_nonnegative_intensities() {
        let (mz, intensities) = decode(&blob(&[100.0, 200.002], &[0.0, 2.0]), 2).unwrap();
        assert_eq!(mz, [100.0, 200.002]);
        assert_eq!(intensities, [0.0, 2.0]);
    }
    #[test]
    fn rejects_nonfinite_or_negative_intensities() {
        for intensities in [[1.0, f32::NAN], [1.0, f32::INFINITY], [1.0, -1.0]] {
            let error = decode(&blob(&[100.0, 200.0], &intensities), 2).unwrap_err();
            assert_eq!(error.kind(), crate::ErrorKind::CorruptData);
        }
    }
    #[test]
    fn rejects_nonpositive_mz_and_wrong_lengths() {
        for mz in [[100.0, f64::NAN], [100.0, 0.0], [100.0, -1.0]] {
            assert!(decode(&blob(&mz, &[1.0, 2.0]), 2).is_err());
        }
        assert!(decode(&blob(&[100.0], &[1.0]), 2).is_err());
    }
    /// The decoder as it read the blob before the planes were split: every
    /// word rebuilt through a closure that indexed `decoded` four times.
    fn reference_spectrum_peaks(decoded: &[u8], peaks: usize) -> Result<(Vec<f64>, Vec<f32>)> {
        if peaks.checked_mul(12) != Some(decoded.len()) {
            return Err(invalid("decoded spectrum length disagrees with peak count"));
        }
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
        let intensities: Vec<f32> = (0..peaks)
            .map(|i| f32::from_bits(word(2 * peaks + i)))
            .collect();
        if intensities.iter().any(|i| !i.is_finite() || *i < 0.0) {
            return Err(invalid("invalid spectrum intensity"));
        }
        Ok((mz, intensities))
    }
    #[test]
    fn split_planes_decode_exactly_as_the_indexed_words_did() {
        let mut mz = Vec::new();
        let mut intensities = Vec::new();
        for i in 0..64u32 {
            mz.push(100.0 + f64::from(i) * 1.000_173);
            intensities.push(f32::from(i as u16) * 0.5);
        }
        // Valid blobs of every length from empty to 64 peaks.
        for peaks in 0..=mz.len() {
            let bytes = blob(&mz[..peaks], &intensities[..peaks]);
            let (left_mz, left_intensities) = decode(&bytes, peaks).unwrap();
            let (right_mz, right_intensities) = reference_spectrum_peaks(&bytes, peaks).unwrap();
            assert_eq!(left_mz, right_mz, "m/z disagree at {peaks} peaks");
            assert_eq!(
                left_intensities, right_intensities,
                "intensities disagree at {peaks} peaks"
            );
        }
        // Malformed and truncated blobs: the same error, message by message.
        let two = blob(&[100.0, 200.0], &[1.0, 2.0]);
        let malformed: [(&[u8], usize); 6] = [
            (&two, 1),
            (&two, 3),
            (&two[..two.len() - 1], 2),
            (&[], 1),
            (&blob(&[100.0, f64::NAN], &[1.0, 2.0]), 2),
            (&blob(&[100.0, 200.0], &[1.0, -1.0]), 2),
        ];
        for (bytes, peaks) in malformed {
            let left = decode(bytes, peaks).unwrap_err();
            let right = reference_spectrum_peaks(bytes, peaks).unwrap_err();
            assert_eq!(left.kind(), right.kind());
            assert_eq!(left.to_string(), right.to_string());
        }
    }
}
