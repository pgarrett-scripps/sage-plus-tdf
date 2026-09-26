use sha2::{Digest, Sha256};
use std::path::PathBuf;
use timsrust_reader::{Limits, MetadataDatabase, MobilityModel, TdfReader};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic.d")
}
fn digest(path: &std::path::Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

#[test]
fn reads_views_case_variants_sparse_ids_and_raw_ions_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["analysis.tdf", "analysis.tdf_bin"] {
        std::fs::copy(fixture().join(name), dir.path().join(name)).unwrap();
    }
    let before = digest(&dir.path().join("analysis.tdf"));
    let mut reader = TdfReader::open(dir.path().join("analysis.tdf_bin")).unwrap();
    assert_eq!(reader.frames().map(|f| f.id).collect::<Vec<_>>(), [10, 42]);
    for id in [10, 42] {
        let frame = reader.read_frame(id).unwrap();
        assert_eq!(frame.scan_offsets, [0, 0, 2, 3]);
        assert_eq!(frame.tof_indices, [0, 5, 8]);
        assert_eq!(frame.intensities, [10, 20, 30]);
        assert!(
            reader
                .mobility_model(id)
                .unwrap()
                .one_over_k0(50.0)
                .unwrap()
                .is_finite()
        );
    }
    assert!(reader.read_frame(1).is_err());
    assert!(reader.mobility_model(1).is_err());
    let blob = reader.metadata().table("quoted\"table").unwrap();
    assert_eq!(blob[0]["Data"], serde_json::json!([1, 2, 255]));
    assert!(reader.metadata().table("Precursors").unwrap().is_empty());
    drop(reader);
    assert_eq!(digest(&dir.path().join("analysis.tdf")), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn missing_database_is_not_created_and_limits_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    assert!(MetadataDatabase::open(dir.path().join("missing.db")).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert!(
        TdfReader::with_limits(
            fixture(),
            Limits {
                max_peaks: 2,
                ..Limits::default()
            }
        )
        .is_err()
    );
    let mut reader = TdfReader::with_limits(
        fixture(),
        Limits {
            max_decompressed_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(reader.read_frame(10).is_err());
}

#[test]
fn active_journal_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture().join("analysis.tdf"),
        dir.path().join("analysis.tdf"),
    )
    .unwrap();
    std::fs::write(dir.path().join("analysis.tdf-wal"), b"active").unwrap();
    assert!(MetadataDatabase::open(dir.path().join("analysis.tdf")).is_err());
}

#[test]
fn lzf_frames_retain_empty_scans_and_reject_corrupt_offsets() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lzf.d");
    let mut reader = TdfReader::open(&source).unwrap();
    let frame = reader.read_frame(10).unwrap();
    assert_eq!(frame.scan_offsets, [0, 2, 2]);
    assert_eq!(frame.tof_indices, [4, 5]);
    assert_eq!(frame.intensities, [10, 20]);
    let temp = tempfile::tempdir().unwrap();
    for name in ["analysis.tdf", "analysis.tdf_bin"] {
        std::fs::copy(source.join(name), temp.path().join(name)).unwrap();
    }
    let path = temp.path().join("analysis.tdf_bin");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    assert!(
        TdfReader::open(temp.path())
            .unwrap()
            .read_frame(10)
            .is_err()
    );
}

#[test]
fn mobility_matches_sage_plus_sdk_reference_including_boundaries() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tims_calibration_sdk.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let c: [f64; 10] = serde_json::from_value(case["coefficients"].clone()).unwrap();
        let model = MobilityModel::new(c).unwrap();
        let scans = case["scans"].as_array().unwrap();
        let values = case["one_over_k0"].as_array().unwrap();
        for (scan, expected) in scans.iter().zip(values) {
            let actual = model.one_over_k0(scan.as_f64().unwrap()).unwrap();
            assert!((actual - expected.as_f64().unwrap()).abs() < 1e-12);
        }
    }
}
