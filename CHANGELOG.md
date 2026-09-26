# Changelog

## 0.1.1-rc.1, unpublished public release candidate

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
