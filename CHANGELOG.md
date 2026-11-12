# Changelog

## [Unreleased]

- Fuzz the TSF line spectrum and miniTDF spectrum blob parsers. Both moved into their own files (`src/tsf_blob.rs`, `src/minitdf_blob.rs`), so the harness compiles them without `analysis.tsf`, `parquet` or a `File`, and a `spectrum_blobs` target runs them against the same input with reused buffers, asserting that an accepted blob yields exactly `peaks` finite values. Nothing a caller sees changed.
- Add `TsfReader::read_line_spectrum_into` and `MiniTdfReader::read_spectrum_into`, which decode into a `Vec<u8>` and a spectrum the caller keeps, instead of allocating a compressed-byte buffer and two output vectors per call. `read_line_spectrum` and `read_spectrum` are unchanged and now call them, as `read_frame` calls `read_frame_into`. `LineSpectrum` and `MiniTdfSpectrum` already implement `Default`, so a caller can make the empty spectrum to fill.
- Decode the miniTDF spectrum blob by splitting its four byte planes once, instead of rebuilding every word through a closure that indexed the blob four times, as the Zstandard frame decoder already does. The length check at the top of the function already proved every one of those indices in range. What is decoded is unchanged, and a test pins it against the previous decoder, blob by blob and error message by error message.

## 0.2.3, 2026-11-11

- Add `TdfReader::read_frame_into`, which decodes a frame into a `Vec<u8>` and a `Frame` the caller keeps, instead of allocating a compressed-byte buffer and three frame vectors per frame. `read_frame` is unchanged and now calls it. `Frame` implements `Default`, so a caller can make the empty frame to fill.
- Decode the Zstandard frame layout by splitting its four byte planes once, instead of reading every word through a closure that bounds-checked an index already covered by the length check, and decide `TdfReader::acquisition_type` in one pass over the frames instead of two. Reading every frame of a 1.0 GB acquisition (22,508 frames, 366.7 million peaks) takes 9.95 s instead of 10.24 s, 2.8% less on the median of three passes. What is read is unchanged.
- Cache the schema object list in `MetadataDatabase`, so opening an acquisition and reading its typed tables issues one `sqlite_master` query instead of two or three per typed table. The list cannot change while the database is open: the file is opened read-only and an active SQLite journal is already rejected.

## 0.2.2, 2026-11-10

- Raise `ruzstd` from the exact `=0.9.0` to `0.9.1`, which carries the fix for RUSTSEC-2026-0344, a Zstandard decompression bomb. This crate was already not exposed: it builds the decoder with `new_with_max_window_size(Limits::max_decompressed_bytes)` and reads at most that ceiling, so both the decoder's window and its output were bounded. The requirement still excludes 0.10.
- Update the C Zstandard bindings that the `minitdf` feature pulls in through `parquet`: `zstd` 0.14.3, `zstd-safe` 8.2.0 and `zstd-sys` 2.1.2, for RUSTSEC-2026-0391, RUSTSEC-2026-0398 and RUSTSEC-2026-0403.

## 0.2.1, 2026-11-09

- Reject non-finite and negative intensities in TSF line spectra and miniTDF spectra as corrupt data, as the m/z arrays already were. A corrupt blob could previously deliver NaN intensities to the caller without an error.
- Saturate the Zstandard decompression ceiling, so a `Limits` with `max_decompressed_bytes: usize::MAX` no longer panics in a debug build or rejects valid data in a release build.
- Report a malformed `HasLineSpectra` value as corrupt data instead of as an absence of line spectra, so `TsfReader::read_line_spectrum` points at the real fault. A missing key is still `Unsupported`.
- Document that `PasefFrameMsMsInfo::isolation_width` and `DiaFrameMsMsWindow::isolation_width` are full widths in m/z, as the TSF field already said, and pin the pass-through in a test.
- Print which environment variable is missing when a real-data test skips, so a green run no longer hides that `SAGE_PLUS_TSF_URINE` or `SAGE_PLUS_MINITDF_ZENODO` was unset.
- Add `UPSTREAM.md`, which records the merge base and which upstream releases have been reviewed. timsrust 0.6.7 was reviewed on 2026-11-09 and does not apply: its fixes are in the centroid spectrum reader, a layer this fork does not have.

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
