use sha2::{Digest, Sha256};
use std::path::PathBuf;
use timsrust_reader::{Limits, MetadataDatabase, MobilityModel, MzCalibration, MzModel, TdfReader};

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
    let reader = TdfReader::open(dir.path().join("analysis.tdf_bin")).unwrap();
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
fn frames_read_concurrently_from_a_shared_reader() {
    let reader = TdfReader::open(fixture()).unwrap();
    let expected = reader.read_frame(10).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..50 {
                    for id in [10, 42] {
                        let frame = reader.read_frame(id).unwrap();
                        assert_eq!(frame.tof_indices, expected.tof_indices);
                        assert_eq!(frame.intensities, expected.intensities);
                    }
                    assert!(reader.mobility_model(42).is_ok());
                }
            });
        }
    });
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
    let reader = TdfReader::with_limits(
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
    let reader = TdfReader::open(&source).unwrap();
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
    assert!((model.mz(100_000.0).unwrap() - 317.80525993456973).abs() < 1e-9);
    assert!(reader.mz_model(999).is_err());
    let calibration = mz_calibration(&serde_json::json!({
        "ModelType": 1, "DigitizerTimebase": 0.2, "DigitizerDelay": 25870.0,
        "T1": 25.62513732863827, "T2": 25.019305770074833, "dC1": 20.0, "dC2": 0.0,
        "C0": 317.86417140238336, "C1": 153159.143753129, "C2": 0.0, "C3": 0.0, "C4": 0.0
    }));
    let same = MzModel::new(&calibration, 25.548975152547968, 0.0).unwrap();
    assert_eq!(same.mz(100_000.0).unwrap(), model.mz(100_000.0).unwrap());
    let warmer = MzModel::new(&calibration, 26.0, 0.0).unwrap();
    assert!(warmer.mz(100_000.0).unwrap() < model.mz(100_000.0).unwrap());
}

fn mz_calibration(value: &serde_json::Value) -> MzCalibration {
    let n = |key: &str| value[key].as_f64().unwrap();
    MzCalibration {
        id: 1,
        model_type: value["ModelType"].as_i64().unwrap(),
        digitizer_timebase: n("DigitizerTimebase"),
        digitizer_delay: n("DigitizerDelay"),
        t1: n("T1"),
        t2: n("T2"),
        dc1: n("dC1"),
        dc2: n("dC2"),
        c0: n("C0"),
        c1: n("C1"),
        c2: n("C2"),
        c3: n("C3"),
        c4: n("C4"),
    }
}

#[test]
fn full_mass_model_matches_bruker_sdk_within_a_tenth_of_a_ppm() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mz_calibration_sdk.json")).unwrap();
    let mut worst = 0.0f64;
    for case in fixture["mz_cases"].as_array().unwrap() {
        let model = MzModel::new(
            &mz_calibration(&case["calibration"]),
            case["frame_t1"].as_f64().unwrap(),
            case["frame_t2"].as_f64().unwrap(),
        )
        .unwrap();
        let tofs = case["tof"].as_array().unwrap();
        for (tof, expected) in tofs.iter().zip(case["mz"].as_array().unwrap()) {
            let expected = expected.as_f64().unwrap();
            let actual = model.mz(tof.as_f64().unwrap()).unwrap();
            let ppm = (actual - expected).abs() / expected * 1e6;
            assert!(ppm < 0.1, "{}: {ppm} ppm at TOF {tof}", case["name"]);
            worst = worst.max(ppm);
        }
        let mzs = case["inverse_mz"].as_array().unwrap();
        for (mz, expected) in mzs.iter().zip(case["inverse_tof"].as_array().unwrap()) {
            let actual = model.tof_index(mz.as_f64().unwrap()).unwrap();
            let expected = expected.as_f64().unwrap();
            assert!(
                (actual - expected).abs() < 1e-3,
                "{}: TOF {actual} vs {expected}",
                case["name"]
            );
        }
    }
    assert!(worst < 1e-3, "worst {worst} ppm");
}

#[test]
fn inverse_mobility_matches_bruker_sdk() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mz_calibration_sdk.json")).unwrap();
    for case in fixture["mobility_inverse_cases"].as_array().unwrap() {
        let c: [f64; 10] = serde_json::from_value(case["coefficients"].clone()).unwrap();
        let model = MobilityModel::new(c).unwrap();
        let values = case["one_over_k0"].as_array().unwrap();
        for (value, expected) in values.iter().zip(case["scans"].as_array().unwrap()) {
            let value = value.as_f64().unwrap();
            let scan = model.scan_number(value).unwrap();
            assert!(
                (scan - expected.as_f64().unwrap()).abs() < 1e-6,
                "{}",
                case["name"]
            );
            assert!((model.one_over_k0(scan).unwrap() - value).abs() < 1e-12);
        }
    }
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

#[test]
fn calibration_tables_with_equal_ids_keep_independent_cached_models() {
    let reader = TdfReader::open(fixture().parent().unwrap().join("metadata.d")).unwrap();
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tims_calibration_sdk.json")).unwrap();
    let scan = reference["cases"][0]["scans"][0].as_f64().unwrap();
    let expected = reference["cases"][0]["one_over_k0"][0].as_f64().unwrap();
    for frame in reader.frames() {
        assert_eq!(frame.mobility_calibration_id, frame.mz_calibration_id);
    }
    for id in [42, 10, 42] {
        let mz = reader.mz_model(id).unwrap().mz(100_000.0).unwrap();
        let mobility = reader
            .mobility_model(id)
            .unwrap()
            .one_over_k0(scan)
            .unwrap();
        assert!((mz - 317.80525993456973).abs() < 1e-9);
        assert!((mobility - expected).abs() < 1e-12);
    }
}

#[test]
fn malformed_calibration_stays_lazy_and_never_leaves_a_partial_cache() {
    let reader =
        TdfReader::open(fixture().parent().unwrap().join("duplicate-calibration.d")).unwrap();
    for _ in 0..3 {
        assert_eq!(reader.read_frame(10).unwrap().tof_indices, [0, 5, 8]);
        assert!(reader.mobility_model(10).is_ok());
        let error = reader.mz_model(10).unwrap_err();
        assert_eq!(
            error.root_error().to_string(),
            "invalid TDF data: duplicate calibration identifier"
        );
    }
}

#[test]
fn contextual_errors_distinguish_limits_from_corruption_and_keep_sources() {
    use timsrust_reader::{Error, ErrorKind};
    let error = match TdfReader::with_limits(
        fixture(),
        Limits {
            max_peaks: 2,
            ..Default::default()
        },
    ) {
        Ok(_) => panic!("peak ceiling was ignored"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    assert!(error.contexts().any(|context| context.frame_id == Some(10)));
    assert!(
        error
            .contexts()
            .any(|context| context.field.as_deref() == Some("NumPeaks"))
    );
    assert!(matches!(
        error.root_error(),
        Error::ResourceLimit {
            resource: "peaks",
            maximum: 2,
            observed: 3,
            ..
        }
    ));

    let dir = tempfile::tempdir().unwrap();
    for name in ["analysis.tdf", "analysis.tdf_bin"] {
        std::fs::copy(fixture().join(name), dir.path().join(name)).unwrap();
    }
    let reader = TdfReader::open(dir.path()).unwrap();
    std::fs::write(dir.path().join("analysis.tdf_bin"), []).unwrap();
    let error = reader.read_frame(10).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::CorruptData);
    assert!(
        error
            .contexts()
            .any(|context| context.frame_id == Some(10) && context.byte_offset == Some(0))
    );
    assert!(
        matches!(error.root_error(), Error::Io(source) if source.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    assert!(std::error::Error::source(&error).is_some());
    let unknown = reader.read_frame(999).unwrap_err();
    assert!(
        unknown
            .contexts()
            .any(|context| context.frame_id == Some(999))
    );
}

#[test]
fn metadata_limits_and_invalid_database_have_actionable_locations() {
    use timsrust_reader::{Error, ErrorKind, MetadataLimits};
    let path = fixture().parent().unwrap().join("metadata.d/analysis.tdf");
    let db = MetadataDatabase::with_limits(
        &path,
        MetadataLimits {
            max_value_bytes: 1024,
            max_vm_operations: 10_000,
            ..Default::default()
        },
    )
    .unwrap();
    let value_error = db.table("Oversized").unwrap_err();
    assert_eq!(value_error.kind(), ErrorKind::ResourceLimit);
    assert!(
        value_error
            .contexts()
            .any(|context| context.table.as_deref() == Some("Oversized"))
    );
    assert!(
        value_error
            .contexts()
            .any(|context| context.field.as_deref() == Some("Data"))
    );
    assert!(
        value_error
            .contexts()
            .any(|context| context.path.as_ref() == Some(&path))
    );
    let execution_error = db.table("Expensive").unwrap_err();
    assert_eq!(execution_error.kind(), ErrorKind::ResourceLimit);
    assert!(matches!(
        execution_error.root_error(),
        Error::ResourceLimit {
            resource: "vm_operations",
            ..
        }
    ));

    let dir = tempfile::tempdir().unwrap();
    let invalid = dir.path().join("invalid.tdf");
    std::fs::write(&invalid, [0x55; 4096]).unwrap();
    let error = match MetadataDatabase::open(&invalid) {
        Ok(_) => panic!("invalid SQLite file was accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::CorruptData);
    assert!(matches!(error.root_error(), Error::DatabaseSource(_)));
    assert!(
        error
            .contexts()
            .any(|context| context.path.as_ref() == Some(&invalid))
    );
}
