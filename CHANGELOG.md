# Changelog

## 0.2.0, 2026-10-10

First crates.io release.

- Rename the package to `sage-plus-tdf` (library `sage_plus_tdf`) and drop the publication-preparation documents, qualification JSON, benchmarks and corpus scripts.
- Implement the full ModelType 1 m/z model (`dC2`/`T2` drift and `C2`, `C3`, `C4`) with a closed-form inverse, checked against libtimsdata to under 0.001 ppm. Add the inverse mobility model.
- Read frames through `&self` with positional reads, so one reader can be shared across threads.
- Read `Frames`, `GlobalMetadata`, `PasefFrameMsMsInfo`, `Precursors`, the DIA window tables and both calibration tables as typed rows. Raw rows are `SqlValue`; blobs stay bytes and infinite REALs no longer fail a table.
- Decode LZF frames exactly as libtimsdata does, including the scan count from the frame header. The old decoder drifted TOF by one bin per gap.
- Require the `.d` directory in `TdfReader::open`.
- Add timsrust-compatible linear m/z and mobility scales and the MsMsType-based acquisition type.
- Add `TsfReader` for TSF line spectra, `Frames` and `FrameMsMsInfo` precursors. timsrust 0.6 returned TSF MS/MS spectra without precursors and truncated TOF indices; both are now kept.
- Add the TSF ModelType 2 m/z model and its polynomial correction, checked against Bruker SDK centroids to under 0.001 ppm.
- Add `MiniTdfReader` behind the `minitdf` feature. Unlike timsrust 0.6, it finds the `<name>.ms2spectrum.*` files ProteoScape writes and keeps m/z as `f64`.
- Move `turso_core` from `0.8.0-pre.13` to the stable `0.8.2` and `parquet` from 59 to 60. Tests pass and decoded output on Bruker TDF, TSF and miniTDF acquisitions is byte-identical.
- Prepare the crate for crates.io: publishable manifest with an explicit file list, CI on Linux, macOS and Windows with MSRV, docs, `cargo deny` and packaging checks, and a tag-driven release workflow using crates.io Trusted Publishing.

## 0.1.1-rc.2, unpublished public release candidate

- Add structured failure categories, actionable acquisition/frame/table/field locations and retained source errors.
- Distinguish configured resource ceilings from corrupt data and unsupported source models.
- Add bounded decoder, schema and calibration fuzzing and a dependency-light JSON benchmark.

## 0.1.1-rc.1, unpublished public release candidate

- Separate acquisition I/O from TDF schema and calibration-cache ownership while preserving the public API and parsing behavior.

- Prepare package discovery metadata, compiled API examples and a docs.rs-compatible documentation build.
- Retain the Sage and tdfpy MIT notices alongside the existing MannLabs MIT and Apache-2.0 texts.
- Document contribution, security reporting and a guarded public release process.
- Add a disposable crates.io publication dry run and tests of the packaged source to CI. Publication and repository visibility remain unchanged.

## 0.1.0, private release, 2026-09-26

- Introduce the focused read-only TDF reader with Rust Turso SQLite, Rust Zstandard and LZF decoding.
- Preserve source frame IDs, raw ion coordinates and metadata.
- Add bounded frame decoding, metadata execution and concurrent query isolation.
- Support explicit restricted source mass and mobility calibration models.
- Validate Linux, macOS and Windows package builds and local acquisition digests.
