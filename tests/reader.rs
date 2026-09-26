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

#[test]
fn metadata_limits_stop_large_values_rows_and_recursive_queries() {
    use timsrust_reader::MetadataLimits;
    let path = fixture().parent().unwrap().join("metadata.d/analysis.tdf");
    let db = MetadataDatabase::with_limits(
        &path,
        MetadataLimits {
            max_value_bytes: 1024,
            max_vm_operations: 10_000,
            ..MetadataLimits::default()
        },
    )
    .unwrap();
    assert!(
        db.table("Oversized")
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    assert!(db.table("Expensive").is_err());
    // An interrupted statement must not poison later metadata queries.
    assert_eq!(db.table("Frames").unwrap().len(), 2);
    let db = MetadataDatabase::with_limits(
        &path,
        MetadataLimits {
            max_rows: 20,
            ..MetadataLimits::default()
        },
    )
    .unwrap();
    assert_eq!(db.table("Frames").unwrap().len(), 2);
    assert!(db.table("ManyRows").is_err());
    let db = MetadataDatabase::with_limits(
        &path,
        MetadataLimits {
            max_materialized_bytes: 1024,
            ..MetadataLimits::default()
        },
    )
    .unwrap();
    assert!(db.table("Oversized").is_err());
}

#[test]
fn source_mass_model_uses_temperature_and_canonical_column_names() {
    let reader = TdfReader::open(fixture().parent().unwrap().join("metadata.d")).unwrap();
    assert_eq!(reader.frames().next().unwrap().scan_count, 3);
    assert!(reader.metadata().table("Frames").unwrap()[0].contains_key("numscans"));
    let model = reader.mz_model(10).unwrap();
    assert!((model.mz(100_000).unwrap() - 317.80525993456973).abs() < 1e-9);
    assert!(reader.mz_model(999).is_err());
    let mut row = reader.metadata().table("MzCalibration").unwrap().remove(0);
    let warmer = timsrust_reader::MzModel::from_metadata(&row, 26.0).unwrap();
    assert!(warmer.mz(100_000).unwrap() < model.mz(100_000).unwrap());
    row.insert("dC2".into(), 1.into());
    assert!(matches!(
        timsrust_reader::MzModel::from_metadata(&row, 26.0),
        Err(timsrust_reader::Error::Unsupported(_))
    ));
    row.insert("dC2".into(), 0.into());
    row.insert("C1".into(), (-1).into());
    assert!(timsrust_reader::MzModel::from_metadata(&row, 26.0).is_err());
}

#[test]
fn concurrent_metadata_queries_keep_independent_budgets() {
    let db = MetadataDatabase::with_limits(
        fixture().parent().unwrap().join("metadata.d/analysis.tdf"),
        timsrust_reader::MetadataLimits {
            max_vm_operations: 10_000,
            ..Default::default()
        },
    )
    .unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                assert!(db.table("Expensive").is_err());
                assert_eq!(db.table("Frames").unwrap().len(), 2);
            });
        }
    });
}
