# Bruker reader review

Reviewed TimsRust 0.6.6 at `80e235a`, the Sage Plus vendored filemanager and mobility adapter, and MSConvert RS's case-insensitive SQLite compatibility patch.

## Findings and resolutions

| Priority | Finding | Resolution |
| --- | --- | --- |
| P1 | Compression2 trusts decoded scan counts, subtracts unchecked counts and unwraps empty blobs. Corrupt files can panic or allocate excessively. | Check sizes against metadata and explicit limits. Reject malformed scan counts, odd count words, TOF overflow and zero coordinates. |
| P1 | Compression1 slices the scan-offset table and scan payload without checking bounds. | Validate every extent and limit each decompression before interpreting words. |
| P1 | DIA frame lookup assumes dense, ordered identifiers and indexes vectors with `frame - 1`. | Preserve source IDs in a sorted map. Processing no longer invents cycles or spectrum identities. |
| P1 | Boundary interpolation is not equivalent to the source mobility calibration used by Sage Plus. | Provide explicit ModelType 2 conversion with validated coefficients and the SDK continuation rule. Missing frame calibration references return errors. |
| P2 | Metadata lookup can reject valid case variants and TDF schemas exposing tables as views. | Resolve tables and views using case-insensitive names, then quote the resolved identifier. |
| P2 | Blob metadata is converted to null. | Preserve blob bytes as JSON integer arrays. |
| P2 | The full package brings SDK, Python, cloud, Parquet and spectrum processing into a frame-reading task. | Replace the broad workspace with a focused native reader crate. All spectrum synthesis and export code is removed from the current tree. Upstream remains in Git history. |

## Database and dependency decisions

Turso is an exact pinned prerelease. `fs`, `uuid`, `json` and `pure-rust-crypto` features are enabled with defaults disabled. The prerelease fails compilation when UUID or JSON is disabled. SIMD, native allocator and C crypto features are excluded. A dependency-tree check found no `rusqlite`, `libsqlite3-sys`, `zstd-sys`, vendor SDK, Python or cloud dependency in the reader.

The database is opened using `OpenFlags::ReadOnly`. Missing paths are rejected before opening. Nonempty SQLite WAL and journal sidecars are rejected because this reader targets completed acquisitions. This is not a filesystem snapshot or a guarantee against a concurrent acquisition process.

## Validation

Formatting, Clippy with warnings denied, 3 decoder unit tests and 5 integration tests pass on Linux. Tests cover read-only behavior without sidecars, source hashes, SQLite views, casing, quoted identifiers, blob values, sparse IDs, empty tables, active journals, limits, malformed LZF offsets and source mobility reference values.

Independent fixture generation uses SQLite from Python's standard library and libzstd to create static test assets. Neither is a runtime dependency of this reader. Both compression modes have synthetic regression coverage. The local DDA and DIA acquisitions exercise Zstandard decoding.

The complete raw frame digests, including source frame ID, scan offsets, TOF indices and intensity counts, exactly match TimsRust 0.6.6 with MSConvert RS's table-casing patch. DDA has 4 frames and 136 ions. DIA has 6 frames and 9,050,385 ions. See `qualification.json`.

## Remaining work

- Turso is a prerelease. Broader real TDF schema and SQLite feature coverage is required before production adoption.
- Metadata tables are read into memory. Metadata row and query-time limits remain to be implemented.
- More real acquisitions are needed for LZF, calibration variants, damaged databases and different instrument generations.
- No m/z calibration API is exposed yet. Raw coordinates and source calibration tables are retained without approximating masses.
- TSF and BAF are outside this TDF-only branch. No pseudo-DDA spectrum compatibility layer is provided.
- Windows and macOS are included in CI configuration but were not executed locally.
- Integrating Sage Plus and MSConvert RS requires an explicit consumer migration with scientific regression tests. This change does not modify either consumer.
