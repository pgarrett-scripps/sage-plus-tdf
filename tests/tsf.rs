use sage_plus_tdf::{ErrorKind, Limits, TsfReader};
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tsf.d")
}

fn copy(dir: &Path) {
    for name in ["analysis.tsf", "analysis.tsf_bin"] {
        std::fs::copy(fixture().join(name), dir.join(name)).unwrap();
    }
}

#[test]
fn reads_frames_line_spectra_and_precursors() {
    let reader = TsfReader::open(fixture()).unwrap();
    assert!(reader.has_line_spectra());
    let levels: Vec<_> = reader.frames().map(|f| (f.id, f.ms_level())).collect();
    assert_eq!(levels, [(1, 1), (2, 2), (3, 2)]);
    assert_eq!(reader.frame(3).unwrap().binary_offset, None);

    let spectrum = reader.read_line_spectrum(1).unwrap();
    assert_eq!(
        spectrum.indices,
        [221.5, 77259.97916666667, 227807.9090909091]
    );
    assert_eq!(spectrum.intensities, [10.0, 250.5, 3.25]);
    let second = reader.read_line_spectrum(2).unwrap();
    assert_eq!(second.indices, [1000.25, 2000.5]);
    assert_eq!(second.intensities, [7.0, 8.0]);
    let empty = reader.read_line_spectrum(3).unwrap();
    assert!(empty.indices.is_empty() && empty.intensities.is_empty());

    let msms = reader.frame_msms_info().unwrap();
    assert_eq!(msms.len(), 2);
    assert_eq!(msms[0].frame, 2);
    assert_eq!(msms[0].parent, Some(1));
    assert_eq!(msms[0].trigger_mass, 44.99850455681751);
    assert_eq!(msms[0].isolation_width, 2.0);
    assert_eq!(msms[0].precursor_charge, None);
    assert_eq!(msms[0].collision_energy, Some(-20.0));
    assert_eq!(msms[1].precursor_charge, Some(2));
}

#[test]
fn model_type_2_matches_bruker_sdk_centroids() {
    // m/z the Bruker SDK reported for these indices (ProteoWizard reference mzML).
    let sdk = [20.155193851563446, 111.04787914506481, 499.86573762004514];
    let reader = TsfReader::open(fixture()).unwrap();
    let model = reader.mz_model(1).unwrap();
    let spectrum = reader.read_line_spectrum(1).unwrap();
    for (index, expected) in spectrum.indices.iter().zip(sdk) {
        let mz = model.mz(*index).unwrap();
        assert!(
            ((mz - expected) / expected).abs() < 1e-9,
            "{mz} vs {expected}"
        );
    }
    // The uncalibrated linear scale timsrust applies is off by ppm.
    let linear = reader.linear_mz_scale().unwrap().mz(spectrum.indices[2]);
    assert!(((linear - sdk[2]) / sdk[2]).abs() > 1e-6);
    // Frame 3 uses a ModelType 1 calibration.
    let model = reader.mz_model(3).unwrap();
    let mz = model.mz(100_000.0).unwrap();
    assert!((model.tof_index(mz).unwrap() - 100_000.0).abs() < 1e-6);
    assert_eq!(
        reader.mz_model(99).unwrap_err().kind(),
        ErrorKind::CorruptData
    );
}

#[test]
fn rejects_tdf_directories_and_parents() {
    let tdf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic.d");
    for path in [tdf, fixture().parent().unwrap().to_path_buf()] {
        let error = TsfReader::open(path).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
    }
}

#[test]
fn rejects_line_spectra_inconsistent_with_metadata() {
    let dir = tempfile::tempdir().unwrap();
    copy(dir.path());
    let bin = dir.path().join("analysis.tsf_bin");
    let original = std::fs::read(&bin).unwrap();

    // A compressed length past the block.
    let mut bytes = original.clone();
    let block = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    bytes[4..8].copy_from_slice(&(block - 7).to_le_bytes());
    std::fs::write(&bin, &bytes).unwrap();
    let reader = TsfReader::open(dir.path()).unwrap();
    let error = reader.read_line_spectrum(1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::CorruptData);
    assert!(error.contexts().any(|context| context.frame_id == Some(1)));

    // A binary file truncated inside the last line spectrum (16 profile bytes follow it).
    std::fs::write(&bin, &original[..original.len() - 17]).unwrap();
    let reader = TsfReader::open(dir.path()).unwrap();
    assert!(reader.read_line_spectrum(1).is_ok());
    assert_eq!(
        reader.read_line_spectrum(2).unwrap_err().kind(),
        ErrorKind::CorruptData
    );
    std::fs::write(&bin, &original[..4]).unwrap();
    assert_eq!(
        TsfReader::open(dir.path()).err().unwrap().kind(),
        ErrorKind::CorruptData
    );
}

#[test]
fn enforces_limits_and_line_spectra_flag() {
    let limits = Limits {
        max_decompressed_bytes: 24,
        ..Limits::default()
    };
    let reader = TsfReader::with_limits(fixture(), limits).unwrap();
    assert_eq!(
        reader.read_line_spectrum(1).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert!(reader.read_line_spectrum(2).is_ok());

    let dir = tempfile::tempdir().unwrap();
    copy(dir.path());
    let error = {
        // Rename the HasLineSpectra key in place, keeping its length.
        let path = dir.path().join("analysis.tsf");
        let mut bytes = std::fs::read(&path).unwrap();
        let key = b"HasLineSpectra";
        let at = bytes
            .windows(key.len())
            .position(|w| w == key)
            .expect("flag key");
        bytes[at..at + key.len()].copy_from_slice(b"HasLineSpectrX");
        std::fs::write(&path, bytes).unwrap();
        TsfReader::open(dir.path())
            .unwrap()
            .read_line_spectrum(1)
            .unwrap_err()
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[test]
fn reader_is_shareable_across_threads() {
    let reader = TsfReader::open(fixture()).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for id in [1, 2, 3] {
                    reader.read_line_spectrum(id).unwrap();
                }
            });
        }
    });
}

/// Set `SAGE_PLUS_TSF_URINE` to the ProteoWizard `timsTOF_autoMSMS_Urine_50s_neg.d`.
#[test]
fn reads_real_tsf_run_when_available() {
    let Some(path) = std::env::var_os("SAGE_PLUS_TSF_URINE") else {
        return;
    };
    let reader = TsfReader::open(path).unwrap();
    assert_eq!(reader.frames().count(), 50);
    let mut peaks = 0;
    for frame in reader.frames() {
        let spectrum = reader.read_line_spectrum(frame.id).unwrap();
        assert_eq!(Some(spectrum.indices.len()), frame.peak_count);
        assert!(spectrum.indices.is_sorted());
        reader.mz_model(frame.id).unwrap();
        peaks += spectrum.indices.len();
    }
    assert_eq!(peaks, 70_265);
    assert_eq!(reader.frame_msms_info().unwrap().len(), 30);
}
