# Changelog

## [Unreleased]

- Breaking (Rust): `FrameInfo::polarity` and `TsfFrameInfo::polarity` are a `Polarity` enum (`Positive`, `Negative`, `Unknown`) instead of `Option<String>`, so reading the `Frames` table no longer allocates a one-byte `String` per frame. `+` and `-` map as Bruker writes them; a NULL, a schema without the column and any other text all read as `Polarity::Unknown`, and a non-text value is still corrupt data on the `Polarity` field, so no acquisition that reads today starts failing. A caller matching on the text calls `Polarity::from_bruker_text`, and one printing it calls `Polarity::as_bruker_text`, which gives `Some("+")`, `Some("-")` or `None`. Opening the 22,508-frame acquisition, which parses every `Frames` row, takes 16.4 ms against 18.0 ms: medians of nine opens per variant in three alternating rounds on an idle machine. Dropping each process's first open, which is a few milliseconds slower in both, leaves 16.2 ms against 17.7 ms with no overlap between the two sets. It is 1.5 ms once per acquisition, next to the seconds its frames then take to read.

## 0.2.4, 2026-11-12

- Add `DecodeBuffers` and `TdfReader::read_frame_into_buffers`, `TsfReader::read_line_spectrum_into_buffers` and `MiniTdfReader::read_spectrum_into_buffers`, which also reuse the buffer the Zstandard decoder writes into. `read_frame_into`, `read_line_spectrum_into` and `read_spectrum_into` allocated that one per call, which is why they saved no time over the allocating methods; these keep it. Every existing method is unchanged and now calls the new path with a buffer set of its own, and `examples/inspect.rs` reads an acquisition through one set. Reading every frame of a 2.0 GB acquisition (22,508 frames, 366.7 million peaks) takes 9.03 s against 9.39 s on 0.2.3's path, and 600,000 TSF line spectrum reads take 313 ms against 349 ms: medians of nine passes per variant, in three alternating rounds on an idle machine, with no overlap between the two sets of passes. miniTDF spectra, whose blobs are a few kilobytes, measure the same either way.
- `examples/inspect.rs` reads TSF and miniTDF as well as TDF, choosing by what the directory holds, and prints the same shape of JSON digest for each. All three modes read through the buffer-reusing methods, so the example also shows how to use them.
- Document the public items a caller meets first: `Result`, `Limits`'s five ceilings, `Frame`'s four fields, every `ErrorKind` variant, every `ErrorContext` field, and `Error` with its variants, named fields and four methods. `missing_docs` now warns, with a module-level allow on the modules whose items are still bare, so the rest can be documented module by module instead of in one pass.
- The miniTDF real-data test pins the peak total it decodes from Zenodo record 10908040, 1,897,058 over 2,467 spectra, instead of printing it. Its TSF counterpart already pinned 70,265, so a decoder change that halved every miniTDF spectrum would have passed.
- Test the ModelType 2 mobility model the way the m/z model is tested: `MobilityModel::scan_number` is asserted to invert `one_over_k0` inside the C8 to C9 window and across both linear continuations, `one_over_k0` to decrease strictly over 40,000 scans, and the non-finite, degenerate and unusable-coefficient paths to be reported as corrupt data. `src/mobility.rs` had no test module. No source line changed.
- Fuzz the TSF line spectrum and miniTDF spectrum blob parsers. Both moved into their own files (`src/tsf_blob.rs`, `src/minitdf_blob.rs`), so the harness compiles them without `analysis.tsf`, `parquet` or a `File`, and a `spectrum_blobs` target runs them against the same input with reused buffers, asserting that an accepted blob yields exactly `peaks` finite values. Nothing a caller sees changed.
- Add `TsfReader::read_line_spectrum_into` and `MiniTdfReader::read_spectrum_into`, which decode into a `Vec<u8>` and a spectrum the caller keeps, instead of allocating a compressed-byte buffer and two output vectors per call. `read_line_spectrum` and `read_spectrum` are unchanged and now call them, as `read_frame` calls `read_frame_into`. `LineSpectrum` and `MiniTdfSpectrum` already implement `Default`, so a caller can make the empty spectrum to fill. What they save is three allocations per call, not time: reading Zenodo record 10908040 and 600,000 TSF fixture spectra through them measured the same as through the wrappers (125 ms and 378 ms against 121 ms and 377 ms), because the Zstandard decoder still allocates its own output buffer per call.
- Decode the miniTDF spectrum blob by splitting its four byte planes once, instead of rebuilding every word through a closure that indexed the blob four times, as the Zstandard frame decoder already does. The length check at the top of the function already proved every one of those indices in range. What is decoded is unchanged, and a test pins it against the previous decoder, blob by blob and error message by error message. Reading every spectrum of Zenodo record 10908040 (2,467 spectra, 1,897,058 peaks) takes 121 ms instead of 133 ms, the median of three passes in each of two interleaved runs on an idle machine.

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
