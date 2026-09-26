# TDF reader responsibility refactor

The crate root previously combined its public data types with acquisition file access, vendor schema interpretation, calibration loading and column normalization. That made a schema change touch the same implementation as frame I/O and gave calibration lookup callers separate table-name and cache arguments that could be mismatched.

The public API remains at the crate root. `reader.rs` now owns acquisition paths, file handles, frame lookup and compressed-frame reads. `schema.rs` owns TDF column normalization, typed frame validation and conversion of source metadata into existing calibration models. Each calibration table name is bound to its own lazy cache. The generic SQLite database, decoder and numerical model implementations are unchanged.

This is an internal responsibility change. Package name, version, dependencies, root exports, method signatures and publication guard remain unchanged. Frame validation order, source-ID ordering, strict integer parsing, source spelling in public metadata, lazy calibration errors and cache population after complete validation are preserved. Per-query mutex scope and resource accounting remain in the unchanged database implementation. Every frame read still checks the current binary file length.

Regression coverage exercises equal calibration IDs in independent mass and mobility tables, repeated lookup after duplicate-ID errors, ambiguous column spelling and rejection of floating-point integer fields. Existing malformed-data, resource-limit, read-only, concurrency and scientific-reference tests remain applicable. CI verifies the source and packaged tests, documentation and the staging-only publication dry run. No tag, registry upload or visibility change is part of this refactor.
