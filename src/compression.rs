//! TDF decoding derived from MannLabs TimsRust compression1 and compression2.
//! Counts, extents and arithmetic are validated before indexing or allocation.
use crate::{Error, Frame, Limits, Result, invalid, limit};
use std::io::Read;

pub(crate) fn word(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or_else(|| invalid("word offset overflow"))?,
        )
        .ok_or_else(|| invalid("truncated word"))?;
    Ok(u32::from_le_bytes(
        bytes.try_into().map_err(|_| invalid("truncated word"))?,
    ))
}

/// Decode into an existing frame, reusing its three buffers. On error the
/// frame holds whatever had been decoded so far and must not be read.
// The frame metadata, the limits and the two buffers are all needed here, and
// the compressed bytes are borrowed from a buffer the caller owns, so they
// cannot be grouped into one struct with the decoded buffer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_into(
    kind: u32,
    id: u64,
    data: &[u8],
    scans: usize,
    peaks: usize,
    limits: Limits,
    decoded: &mut Vec<u8>,
    frame: &mut Frame,
) -> Result<()> {
    if scans > limits.max_scans {
        return Err(limit("scans", limits.max_scans, scans));
    }
    if peaks > limits.max_peaks {
        return Err(limit("peaks", limits.max_peaks, peaks));
    }
    if scans == 0 && peaks != 0 {
        return Err(invalid("invalid scan or peak count"));
    }
    let minimum_bytes = peaks
        .checked_mul(8)
        .and_then(|n| scans.checked_mul(4).and_then(|s| n.checked_add(s)))
        .ok_or_else(|| invalid("decoded extent overflow"))?;
    if minimum_bytes > limits.max_decompressed_bytes {
        return Err(limit(
            "decompressed_bytes",
            limits.max_decompressed_bytes,
            minimum_bytes,
        ));
    }
    frame.id = id;
    // An LZF frame never reaches `zstd_into`, which is what otherwise clears
    // this, so clear it here: a caller reading a mixed acquisition must not see
    // the bytes of the previous Zstandard frame.
    decoded.clear();
    frame.scan_offsets.clear();
    frame.tof_indices.clear();
    frame.intensities.clear();
    frame.scan_offsets.push(0);
    frame.tof_indices.reserve(peaks);
    frame.intensities.reserve(peaks);
    match kind {
        2 => decode_v2(data, scans, peaks, limits, decoded, frame)?,
        1 => decode_v1(data, scans, peaks, limits, frame)?,
        _ => return Err(invalid("unsupported compression")),
    }
    if frame.tof_indices.len() != peaks
        || frame.intensities.len() != peaks
        || frame.scan_offsets.len() != scans + 1
    {
        return Err(invalid("decoded counts disagree with Frames metadata"));
    }
    Ok(())
}

fn decode_v2(
    data: &[u8],
    scans: usize,
    peaks: usize,
    limits: Limits,
    decoded: &mut Vec<u8>,
    frame: &mut Frame,
) -> Result<()> {
    let expected = peaks
        .checked_mul(2)
        .and_then(|n| n.checked_add(scans))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| invalid("decoded extent overflow"))?;
    if expected > limits.max_decompressed_bytes {
        return Err(limit(
            "decompressed_bytes",
            limits.max_decompressed_bytes,
            expected,
        ));
    }
    zstd_into(data, expected, limits, decoded)?;
    decode_v2_bytes(decoded, scans, peaks, frame)
}

/// Decode one complete Zstandard frame of at most `max_bytes` output bytes.
/// Longer output is corrupt, or a resource limit when `max_bytes` exceeds
/// `limits.max_decompressed_bytes`. Trailing compressed bytes, truncation and
/// checksum mismatches are errors. Shared by TDF, TSF and miniTDF blobs.
///
/// `out` is cleared first and keeps its capacity, so decoding frame after
/// frame through one buffer allocates once instead of per frame. On error it
/// holds partial output and must not be read.
pub(crate) fn zstd_into(
    data: &[u8],
    max_bytes: usize,
    limits: Limits,
    out: &mut Vec<u8>,
) -> Result<()> {
    out.clear();
    let capped = max_bytes > limits.max_decompressed_bytes;
    let max_bytes = max_bytes.min(limits.max_decompressed_bytes);
    let mut decoder = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(
        data,
        limits.max_decompressed_bytes as u64,
    )
    .map_err(|source| {
        if let ruzstd::decoding::errors::FrameDecoderError::WindowSizeTooBig { requested, max } =
            &source
        {
            return Error::ResourceLimit {
                resource: "zstandard_window_bytes",
                maximum: *max,
                observed: *requested,
                source: Some(Box::new(source)),
            };
        }
        Error::Decoder {
            format: "Zstandard",
            source: Box::new(source),
        }
    })?;
    decoder
        .by_ref()
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(out)
        .map_err(|source| Error::Decoder {
            format: "Zstandard",
            source: Box::new(source),
        })?;
    if out.len() > max_bytes {
        return Err(if capped {
            limit("decompressed_bytes", max_bytes, out.len())
        } else {
            invalid("decoded byte count exceeds the recorded extent")
        });
    }
    if !decoder.decoder.is_finished() || !decoder.get_ref().is_empty() {
        return Err(invalid(
            "incomplete Zstandard frame or trailing compressed bytes",
        ));
    }
    if let Some(expected_checksum) = decoder.decoder.get_checksum_from_data()
        && decoder.decoder.get_calculated_checksum() != Some(expected_checksum)
    {
        return Err(invalid("Zstandard checksum mismatch"));
    }
    Ok(())
}

fn decode_v2_bytes(bytes: &[u8], scans: usize, peaks: usize, frame: &mut Frame) -> Result<()> {
    let words = scans
        .checked_add(
            peaks
                .checked_mul(2)
                .ok_or_else(|| invalid("peak overflow"))?,
        )
        .ok_or_else(|| invalid("word overflow"))?;
    if bytes.len()
        != words
            .checked_mul(4)
            .ok_or_else(|| invalid("size overflow"))?
    {
        return Err(invalid("decoded byte count disagrees with frame metadata"));
    }
    if scans == 0 {
        return Ok(());
    }
    // Byte-transposed layout: the four bytes of word `i` are the `i`-th byte of
    // four equal planes. Splitting the planes once, and the scan table off the
    // peak pairs, gives the same words as indexing each byte separately. The
    // first `scans` words are the scan table and the rest are (TOF delta,
    // intensity) pairs read in order, so the peaks need no index arithmetic.
    let (plane0, rest) = bytes.split_at(words);
    let (plane1, rest) = rest.split_at(words);
    let (plane2, plane3) = rest.split_at(words);
    let (table0, peaks0) = plane0.split_at(scans);
    let (table1, peaks1) = plane1.split_at(scans);
    let (table2, peaks2) = plane2.split_at(scans);
    let (table3, peaks3) = plane3.split_at(scans);
    let scan_word = |i: usize| u32::from_le_bytes([table0[i], table1[i], table2[i], table3[i]]);
    let mut pairs = peaks0
        .as_chunks::<2>()
        .0
        .iter()
        .zip(peaks1.as_chunks::<2>().0)
        .zip(peaks2.as_chunks::<2>().0)
        .zip(peaks3.as_chunks::<2>().0)
        .map(|(((b0, b1), b2), b3)| {
            (
                u32::from_le_bytes([b0[0], b1[0], b2[0], b3[0]]),
                u32::from_le_bytes([b0[1], b1[1], b2[1], b3[1]]),
            )
        });
    if scan_word(0) as usize != scans {
        return Err(invalid("decoded scan count mismatch"));
    }
    let mut offset = 0usize;
    for scan in 0..scans {
        let end = if scan + 1 == scans {
            peaks
        } else {
            let doubled = scan_word(scan + 1);
            if doubled % 2 != 0 {
                return Err(invalid("odd scan peak count"));
            }
            offset
                .checked_add(doubled as usize / 2)
                .ok_or_else(|| invalid("scan offset overflow"))?
        };
        if end < offset || end > peaks {
            return Err(invalid("scan peak count exceeds frame"));
        }
        let mut tof = 0u32;
        for _ in offset..end {
            let (delta, intensity) = pairs
                .next()
                .ok_or_else(|| invalid("truncated peak words"))?;
            tof = tof
                .checked_add(delta)
                .ok_or_else(|| invalid("TOF accumulation overflow"))?;
            frame.tof_indices.push(
                tof.checked_sub(1)
                    .ok_or_else(|| invalid("zero TOF coordinate"))?,
            );
            frame.intensities.push(intensity);
        }
        frame.scan_offsets.push(end);
        offset = end;
    }
    Ok(())
}

fn decode_v1(
    data: &[u8],
    scans: usize,
    peaks: usize,
    limits: Limits,
    frame: &mut Frame,
) -> Result<()> {
    let table_bytes = scans
        .checked_add(1)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| invalid("scan table overflow"))?;
    if data.len() < table_bytes {
        return Err(invalid("truncated scan offset table"));
    }
    if word(data, 0)? as usize != table_bytes + 8
        || word(data, scans * 4)? as usize != data.len() + 8
    {
        return Err(invalid("unreferenced bytes in LZF frame"));
    }
    let per_scan_limit = peaks
        .checked_mul(8)
        .ok_or_else(|| invalid("scan limit overflow"))?
        .min(limits.max_decompressed_bytes);
    let mut decoded_bytes = 0usize;
    for scan in 0..scans {
        let start = (word(data, scan * 4)? as usize)
            .checked_sub(8)
            .ok_or_else(|| invalid("invalid scan start"))?;
        let end = (word(data, (scan + 1) * 4)? as usize)
            .checked_sub(8)
            .ok_or_else(|| invalid("invalid scan end"))?;
        if start < table_bytes || start > end || end > data.len() {
            return Err(invalid("invalid compressed scan range"));
        }
        if start != end {
            let remaining = limits
                .max_decompressed_bytes
                .checked_sub(decoded_bytes)
                .ok_or_else(|| {
                    limit(
                        "decompressed_bytes",
                        limits.max_decompressed_bytes,
                        decoded_bytes,
                    )
                })?;
            let bytes = lzf::decompress(&data[start..end], per_scan_limit.min(remaining))
                .map_err(Error::Lzf)?;
            decoded_bytes += bytes.len();
            if bytes.len() % 4 != 0 {
                return Err(invalid("partial LZF word"));
            }
            // As libtimsdata decodes it: a negative word skips that many TOF
            // bins, a positive word is the intensity at the current bin, which
            // then advances by one.
            let mut tof = 0u64;
            for bytes in bytes.as_chunks::<4>().0 {
                let value = i32::from_le_bytes(*bytes);
                if value > 0 {
                    frame
                        .tof_indices
                        .push(u32::try_from(tof).map_err(|_| invalid("TOF range overflow"))?);
                    frame.intensities.push(value as u32);
                    if frame.intensities.len() > peaks {
                        return Err(invalid("too many decoded peaks"));
                    }
                    tof += 1;
                } else if value < 0 {
                    tof += u64::from(value.unsigned_abs());
                } else {
                    return Err(invalid("zero word in LZF scan"));
                }
            }
        }
        frame.scan_offsets.push(frame.intensities.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(
        kind: u32,
        id: u64,
        data: &[u8],
        scans: usize,
        peaks: usize,
        limits: Limits,
    ) -> Result<Frame> {
        let mut frame = Frame::default();
        let mut decoded = Vec::new();
        decode_into(
            kind,
            id,
            data,
            scans,
            peaks,
            limits,
            &mut decoded,
            &mut frame,
        )?;
        Ok(frame)
    }
    fn shuffled(words: &[u32]) -> Vec<u8> {
        (0..4)
            .flat_map(|b| words.iter().map(move |w| w.to_le_bytes()[b]))
            .collect()
    }
    /// The indexed decoder this module used before the four byte planes were
    /// split once: every word came from four bounds-checked byte lookups
    /// behind a closure. Kept to pin that the planes decode identically.
    fn reference_v2_bytes(
        bytes: &[u8],
        scans: usize,
        peaks: usize,
        frame: &mut Frame,
    ) -> Result<()> {
        let words = scans
            .checked_add(
                peaks
                    .checked_mul(2)
                    .ok_or_else(|| invalid("peak overflow"))?,
            )
            .ok_or_else(|| invalid("word overflow"))?;
        if bytes.len()
            != words
                .checked_mul(4)
                .ok_or_else(|| invalid("size overflow"))?
        {
            return Err(invalid("decoded byte count disagrees with frame metadata"));
        }
        if scans == 0 {
            return Ok(());
        }
        let at = |i: usize| -> Result<u32> {
            if i >= words {
                return Err(invalid("invalid shuffled word offset"));
            }
            Ok(u32::from_le_bytes([
                bytes[i],
                bytes[i + words],
                bytes[i + 2 * words],
                bytes[i + 3 * words],
            ]))
        };
        if at(0)? as usize != scans {
            return Err(invalid("decoded scan count mismatch"));
        }
        let mut offset = 0usize;
        for scan in 0..scans {
            let end = if scan + 1 == scans {
                peaks
            } else {
                let doubled = at(scan + 1)?;
                if doubled % 2 != 0 {
                    return Err(invalid("odd scan peak count"));
                }
                offset
                    .checked_add(doubled as usize / 2)
                    .ok_or_else(|| invalid("scan offset overflow"))?
            };
            if end < offset || end > peaks {
                return Err(invalid("scan peak count exceeds frame"));
            }
            let mut tof = 0u32;
            for peak in offset..end {
                let delta = at(scans + 2 * peak)?;
                tof = tof
                    .checked_add(delta)
                    .ok_or_else(|| invalid("TOF accumulation overflow"))?;
                frame.tof_indices.push(
                    tof.checked_sub(1)
                        .ok_or_else(|| invalid("zero TOF coordinate"))?,
                );
                frame.intensities.push(at(scans + 2 * peak + 1)?);
            }
            frame.scan_offsets.push(end);
            offset = end;
        }
        Ok(())
    }

    /// One well-formed Zstandard-layer frame: the scan table, then the
    /// (TOF delta, intensity) pairs in scan order.
    fn v2_words(scan_counts: &[usize], deltas: &[u32], intensities: &[u32]) -> Vec<u32> {
        let scans = scan_counts.len();
        let mut words = vec![scans as u32];
        for count in &scan_counts[..scans - 1] {
            words.push(2 * *count as u32);
        }
        for (delta, intensity) in deltas.iter().zip(intensities) {
            words.push(*delta);
            words.push(*intensity);
        }
        words
    }

    #[test]
    fn split_planes_decode_exactly_as_the_indexed_words_did() {
        let mut state = 0x2026_1110_u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };
        let mut cases: Vec<(Vec<u8>, usize, usize)> = Vec::new();
        // Well-formed frames, including empty scans and a single-peak frame.
        for scan_counts in [
            vec![0usize],
            vec![1],
            vec![0, 2, 1],
            vec![3, 0, 0, 4],
            vec![7, 1, 2, 0, 5, 9],
        ] {
            let peaks: usize = scan_counts.iter().sum();
            let deltas: Vec<u32> = (0..peaks).map(|_| 1 + next() % 1000).collect();
            let intensities: Vec<u32> = (0..peaks).map(|_| next() % 10_000).collect();
            let words = v2_words(&scan_counts, &deltas, &intensities);
            cases.push((shuffled(&words), scan_counts.len(), peaks));
        }
        // Malformed frames: random words, truncations and wrong counts, so the
        // error paths are compared too.
        for case in 0..400 {
            let scans = 1 + case % 5;
            let peaks = case % 4;
            let words: Vec<u32> = (0..scans + 2 * peaks).map(|_| next()).collect();
            let bytes = shuffled(&words);
            cases.push((bytes.clone(), scans, peaks));
            cases.push((
                bytes[..bytes.len().saturating_sub(4)].to_vec(),
                scans,
                peaks,
            ));
            cases.push((bytes, scans + 1, peaks));
        }
        for (bytes, scans, peaks) in cases {
            let mut split = empty();
            let mut reference = empty();
            let split_result = decode_v2_bytes(&bytes, scans, peaks, &mut split);
            let reference_result = reference_v2_bytes(&bytes, scans, peaks, &mut reference);
            match (split_result, reference_result) {
                (Ok(()), Ok(())) => assert_eq!(split, reference),
                (Err(split), Err(reference)) => {
                    assert_eq!(split.to_string(), reference.to_string())
                }
                (split, reference) => {
                    panic!("{scans} scans, {peaks} peaks: {split:?} vs {reference:?}")
                }
            }
        }
    }

    fn empty() -> Frame {
        Frame {
            id: 1,
            scan_offsets: vec![0],
            tof_indices: vec![],
            intensities: vec![],
        }
    }
    #[test]
    fn retains_raw_coordinates_and_empty_scans() {
        let mut frame = empty();
        decode_v2_bytes(&shuffled(&[3, 0, 4, 1, 10, 5, 20, 9, 30]), 3, 3, &mut frame).unwrap();
        assert_eq!(frame.scan_offsets, [0, 0, 2, 3]);
        assert_eq!(frame.tof_indices, [0, 5, 8]);
        assert_eq!(frame.intensities, [10, 20, 30]);
    }
    #[test]
    fn malformed_counts_and_tof_values_return_errors() {
        for words in [
            vec![2, 8, 1, 10],
            vec![2, 1, 1, 10],
            vec![2, 2, 0, 10],
            vec![9, 2, 1, 10],
        ] {
            assert!(decode_v2_bytes(&shuffled(&words), 2, 1, &mut empty()).is_err());
        }
        assert!(decode_v2_bytes(&shuffled(&[1, u32::MAX, 10, 2, 20]), 1, 2, &mut empty()).is_err());
        assert!(decode_v2_bytes(&[], 1, 0, &mut empty()).is_err());
    }
    #[test]
    fn truncated_lzf_table_returns_error() {
        assert!(decode(1, 1, &[], 1, 1, Limits::default()).is_err());
    }
    #[test]
    fn an_unbounded_decompressed_ceiling_does_not_overflow() {
        let compressed = include_bytes!("../tests/fixtures/checksum.zstd");
        let limits = Limits {
            max_decompressed_bytes: usize::MAX,
            ..Limits::default()
        };
        let mut decoded = Vec::new();
        zstd_into(compressed, usize::MAX, limits, &mut decoded).unwrap();
        assert_eq!(decoded.len(), 36);
    }
    #[test]
    fn zstandard_checksum_trailing_bytes_and_truncations_are_rejected() {
        let compressed = include_bytes!("../tests/fixtures/checksum.zstd");
        let result = decode(2, 1, compressed, 3, 3, Limits::default()).unwrap();
        assert_eq!(result.tof_indices, [0, 5, 8]);
        let mut corrupt = compressed.to_vec();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode(2, 1, &corrupt, 3, 3, Limits::default()).is_err());
        let mut trailing = compressed.to_vec();
        trailing.push(0);
        assert!(decode(2, 1, &trailing, 3, 3, Limits::default()).is_err());
        for end in 0..compressed.len() {
            assert!(decode(2, 1, &compressed[..end], 3, 3, Limits::default()).is_err());
        }
    }

    fn lzf_frame(scans: &[&[i32]]) -> Vec<u8> {
        // Literal-only LZF blocks (at most 32 bytes each) behind the offset table.
        let blocks: Vec<Vec<u8>> = scans
            .iter()
            .map(|words| {
                let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                if raw.is_empty() {
                    return raw;
                }
                assert!(raw.len() <= 32);
                [vec![raw.len() as u8 - 1], raw].concat()
            })
            .collect();
        let mut offset = 8 + 4 * (scans.len() + 1);
        let mut table = vec![offset as u32];
        for block in &blocks {
            offset += block.len();
            table.push(offset as u32);
        }
        let mut data: Vec<u8> = table.iter().flat_map(|w| w.to_le_bytes()).collect();
        data.extend(blocks.concat());
        data
    }

    #[test]
    fn lzf_gaps_skip_bins_and_peaks_advance_one_bin_like_libtimsdata() {
        let data = lzf_frame(&[&[-10, 5, -3, 7, 9], &[], &[-2, 1]]);
        let frame = decode(1, 1, &data, 3, 4, Limits::default()).unwrap();
        assert_eq!(frame.scan_offsets, [0, 3, 3, 4]);
        assert_eq!(frame.tof_indices, [10, 14, 15, 2]);
        assert_eq!(frame.intensities, [5, 7, 9, 1]);
        let zero = lzf_frame(&[&[-10, 0, 5]]);
        assert!(decode(1, 1, &zero, 1, 1, Limits::default()).is_err());
    }

    #[test]
    fn lzf_rejects_leading_gaps_and_trailing_bytes() {
        let bytes = include_bytes!("../tests/fixtures/lzf.d/analysis.tdf_bin");
        let compressed = &bytes[8..];
        decode(1, 1, compressed, 2, 2, Limits::default()).unwrap();
        let mut trailing = compressed.to_vec();
        trailing.push(0);
        assert!(decode(1, 1, &trailing, 2, 2, Limits::default()).is_err());
        let mut gap = compressed.to_vec();
        gap[0..4].copy_from_slice(&24u32.to_le_bytes());
        assert!(decode(1, 1, &gap, 2, 2, Limits::default()).is_err());
    }
}
