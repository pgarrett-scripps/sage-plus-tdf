# Contributing

This crate is developed for [Sage Plus](https://github.com/pgarrett-scripps/sage-plus) and published on crates.io for anyone who needs to read Bruker timsTOF data from Rust. Open an issue to discuss larger changes, and use pull requests against `main`. Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Keep the library focused on reading completed Bruker TDF acquisitions: frames, calibration, metadata and typed tables. Spectrum synthesis, summing, centroiding, writing acquisition files, the Bruker SDK and native C SQLite or Zstandard dependencies are outside its scope. Scientific conversions must use recorded source metadata with independently justified equations. Unsupported models must return explicit errors.

Development uses Rust 1.97.1 from `rust-toolchain.toml`, which pins formatting and lint results; the published crate only requires Rust 1.97 (`rust-version`). Before submitting a change, run:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
cargo deny check
```

The `minitdf` feature builds the C `zstd` library through `parquet`, so `--all-features` needs a C compiler. The default features must stay pure Rust; CI builds and tests them with the C compiler disabled. CI also runs these checks on macOS and Windows, builds with the MSRV, runs `cargo publish --dry-run` and smoke-tests the fuzz targets.

Include a focused regression test for parsing or scientific behavior changes. Synthetic fixtures are preferred for malformed inputs. Record the origin, license and independent expected result of every reference fixture. Do not commit private acquisition data, patient information, access tokens or unpublished results. Describe any changes to supported formats or calibration models in the README.

Retain existing copyright and license notices. Identify the source and license of any borrowed code or fixture in `NOTICE.md`. The aggregate package is declared `Apache-2.0 AND MIT`, which does not offer a choice between those licenses. Maintainers must review provenance before accepting external material.

Report potential security issues according to [SECURITY.md](SECURITY.md).

To regenerate equivalent synthetic SQLite, Zstandard and LZF fixtures, run `python3 scripts/generate_synthetic_fixtures.py /tmp/new-tdf-fixtures`. This generation tool needs Python 3.12 and a system libzstd. Neither is required by the reader. The output directory must not already exist. Encoder versions can change compressed bytes or SQLite page layout while decoded values remain unchanged. The SDK reference JSON is a separate source-derived artifact with provenance in `NOTICE.md`.
