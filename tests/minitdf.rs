#![cfg(feature = "minitdf")]
use sage_plus_tdf::{ErrorKind, Limits, MiniTdfPrecursor, MiniTdfReader, MiniTdfSpectrum};
use std::path::{Path, PathBuf};

/// MannLabs TimsRust `src/tests/test2.ms2` (two spectra).
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minitdf.d")
}

fn copy(dir: &Path) {
    for name in ["converter.ms2spectrum.bin", "converter.ms2spectrum.parquet"] {
        std::fs::copy(fixture().join(name), dir.join(name)).unwrap();
    }
}

#[test]
fn reads_spectra_and_precursors() {
    let reader = MiniTdfReader::open(fixture()).unwrap();
    assert_eq!(reader.len(), 2);
    let first = reader.read_spectrum(0).unwrap();
    assert_eq!(first.mz, [100.0, 200.002, 300.03, 400.4]);
    assert_eq!(first.intensities, [1.0, 2.0, 3.0, 4.0]);
    let second = reader.read_spectrum(1).unwrap();
    assert_eq!(second.mz, [1100.0, 1200.002, 1300.03, 1400.4]);
    assert_eq!(second.intensities, [10.0, 20.0, 30.0, 40.0]);

    let p = &reader.precursors()[0];
    assert_eq!(p.monoisotopic_mz, Some(123.4567));
    assert_eq!(p.retention_time_seconds, 12.345);
    assert_eq!(p.one_over_k0, Some(1.234));
    assert_eq!(p.charge, Some(1));
    assert_eq!(p.binary_offset, 0);
    let p = &reader.precursors()[1];
    assert_eq!(p.monoisotopic_mz, Some(987.6543));
    assert_eq!(p.charge, Some(2));
    assert_eq!(p.binary_offset, 65);
    assert_eq!(MiniTdfPrecursor::assumed_isolation_width(123.4567), 2.0);
    assert_eq!(MiniTdfPrecursor::assumed_isolation_width(987.6543), 3.0);
    assert_eq!(MiniTdfPrecursor::assumed_isolation_width(750.0), 2.5);
    assert!(reader.read_spectrum(2).is_err());
}

#[test]
fn finds_files_only_in_the_given_directory() {
    let parent = fixture().parent().unwrap().to_path_buf();
    assert_eq!(MiniTdfReader::find(&parent).unwrap(), None);
    assert_eq!(
        MiniTdfReader::open(parent).err().unwrap().kind(),
        ErrorKind::Unsupported
    );
    let dir = tempfile::tempdir().unwrap();
    copy(dir.path());
    // A bare bin without its parquet is ignored; a second pair is ambiguous.
    std::fs::write(dir.path().join("other.ms2spectrum.bin"), b"").unwrap();
    assert!(MiniTdfReader::open(dir.path()).is_ok());
    std::fs::copy(
        fixture().join("converter.ms2spectrum.parquet"),
        dir.path().join("other.ms2spectrum.parquet"),
    )
    .unwrap();
    assert_eq!(
        MiniTdfReader::open(dir.path()).err().unwrap().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn rejects_corrupt_blobs_and_enforces_limits() {
    let dir = tempfile::tempdir().unwrap();
    copy(dir.path());
    let bin = dir.path().join("converter.ms2spectrum.bin");
    let original = std::fs::read(&bin).unwrap();

    let mut bytes = original.clone();
    bytes[4..8].copy_from_slice(&5u32.to_le_bytes());
    std::fs::write(&bin, &bytes).unwrap();
    let reader = MiniTdfReader::open(dir.path()).unwrap();
    assert_eq!(
        reader.read_spectrum(0).unwrap_err().kind(),
        ErrorKind::CorruptData
    );

    std::fs::write(&bin, &original[..original.len() - 1]).unwrap();
    let reader = MiniTdfReader::open(dir.path()).unwrap();
    assert!(reader.read_spectrum(0).is_ok());
    assert_eq!(
        reader.read_spectrum(1).unwrap_err().kind(),
        ErrorKind::CorruptData
    );

    let limits = Limits {
        max_peaks: 3,
        ..Limits::default()
    };
    let reader = MiniTdfReader::with_limits(fixture(), limits).unwrap();
    assert_eq!(
        reader.read_spectrum(0).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
}

/// Set `SAGE_PLUS_MINITDF_ZENODO` to the extracted Zenodo record 10908040,
/// "Example dataset for the Bruker miniTDF format" (CC-BY-4.0), whose
/// `1min_snippit_miniTDFcontents.tar` is
/// `d49d12f03faca09386eccd2675ed992a978a42e4d6150585f230444c48e86213`. The peak
/// total below is what this reader decodes from that record.
#[test]
fn reads_real_minitdf_when_available() {
    let Some(path) = std::env::var_os("SAGE_PLUS_MINITDF_ZENODO") else {
        println!("skipped: SAGE_PLUS_MINITDF_ZENODO is not set");
        return;
    };
    let reader = MiniTdfReader::open(path).unwrap();
    assert_eq!(reader.len(), 2467);
    let mut peaks = 0;
    for (index, precursor) in reader.precursors().iter().enumerate() {
        let spectrum = reader.read_spectrum(index).unwrap();
        assert_eq!(Some(spectrum.mz.len()), precursor.peak_count);
        assert!(spectrum.mz.is_sorted());
        peaks += spectrum.mz.len();
    }
    assert_eq!(peaks, 1_897_058);
}

#[test]
fn read_spectrum_into_reuses_buffers_and_matches_read_spectrum() {
    let reader = MiniTdfReader::open(fixture()).unwrap();
    let mut data = Vec::new();
    let mut spectrum = MiniTdfSpectrum::default();
    reader
        .read_spectrum_into(0, &mut data, &mut spectrum)
        .unwrap();
    let capacities = (data.capacity(), spectrum.mz.capacity());
    assert!(capacities.0 > 0 && capacities.1 > 0);
    for _ in 0..3 {
        reader
            .read_spectrum_into(0, &mut data, &mut spectrum)
            .unwrap();
        assert_eq!(spectrum, reader.read_spectrum(0).unwrap());
        // Re-reading a spectrum reuses the buffers instead of allocating again.
        assert_eq!((data.capacity(), spectrum.mz.capacity()), capacities);
    }
    for index in [1, 0] {
        reader
            .read_spectrum_into(index, &mut data, &mut spectrum)
            .unwrap();
        assert_eq!(spectrum, reader.read_spectrum(index).unwrap());
    }
    // A failed read leaves both buffers usable for the next spectrum.
    assert!(
        reader
            .read_spectrum_into(2, &mut data, &mut spectrum)
            .is_err()
    );
    reader
        .read_spectrum_into(1, &mut data, &mut spectrum)
        .unwrap();
    assert_eq!(spectrum, reader.read_spectrum(1).unwrap());
}
