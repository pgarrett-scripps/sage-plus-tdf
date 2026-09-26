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

pub(crate) fn decode(
    kind: u32,
    id: u64,
    data: &[u8],
    scans: usize,
    peaks: usize,
    limits: Limits,
) -> Result<Frame> {
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
    let mut frame = Frame {
        id,
        scan_offsets: vec![0],
        tof_indices: Vec::with_capacity(peaks),
        intensities: Vec::with_capacity(peaks),
    };
    match kind {
        2 => decode_v2(data, scans, peaks, limits, &mut frame)?,
        1 => decode_v1(data, scans, peaks, limits, &mut frame)?,
        _ => return Err(invalid("unsupported compression")),
    }
    if frame.tof_indices.len() != peaks
        || frame.intensities.len() != peaks
        || frame.scan_offsets.len() != scans + 1
    {
        return Err(invalid("decoded counts disagree with Frames metadata"));
    }
    Ok(frame)
}

fn decode_v2(
    data: &[u8],
    scans: usize,
    peaks: usize,
    limits: Limits,
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
    let mut decoded = Vec::with_capacity(expected);
    decoder
        .by_ref()
        .take(expected as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|source| Error::Decoder {
            format: "Zstandard",
            source: Box::new(source),
        })?;
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
    decode_v2_bytes(&decoded, scans, peaks, frame)
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
            let mut tof = 0i64;
            for bytes in bytes.as_chunks::<4>().0 {
                let value = i32::from_le_bytes(*bytes);
                if value > 0 {
                    tof -= 1;
                    frame
                        .tof_indices
                        .push(u32::try_from(-tof).map_err(|_| invalid("TOF range overflow"))?);
                    frame.intensities.push(value as u32);
                    if frame.intensities.len() > peaks {
                        return Err(invalid("too many decoded peaks"));
                    }
                } else {
                    tof = tof
                        .checked_add(i64::from(value) + 1)
                        .ok_or_else(|| invalid("TOF accumulation overflow"))?;
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
    fn shuffled(words: &[u32]) -> Vec<u8> {
        (0..4)
            .flat_map(|b| words.iter().map(move |w| w.to_le_bytes()[b]))
            .collect()
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
