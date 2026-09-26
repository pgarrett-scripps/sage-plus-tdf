# Contributing

This repository is currently private. Public distribution is being prepared, and publication remains disabled. Use pull requests to propose changes against `main`.

Keep the library focused on reading completed Bruker TDF acquisitions. Spectrum synthesis, writing acquisition files, the Bruker SDK and native C SQLite or Zstandard dependencies are outside its scope. Scientific conversions must use recorded source metadata with independently justified equations. Unsupported models must return explicit errors.

Use Rust 1.97.1 from `rust-toolchain.toml`. Before submitting a change, run:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo doc --locked --no-deps
cargo package --locked
```

Include a focused regression test for parsing or scientific behavior changes. Synthetic fixtures are preferred for malformed inputs. Record the origin, license and independent expected result of every reference fixture. Do not commit private acquisition data, patient information, access tokens or unpublished results. Describe any changes to supported formats or calibration models in the README.

Retain existing copyright and license notices. Identify the source and license of any borrowed code or fixture in `NOTICE.md`. The aggregate package is declared `Apache-2.0 AND MIT`, which does not offer a choice between those licenses. Maintainers must review provenance before accepting external material.

Report potential security issues according to [SECURITY.md](SECURITY.md). Public release preparation is described in [RELEASING.md](RELEASING.md).

To regenerate equivalent synthetic SQLite, Zstandard and LZF fixtures, run `python3 scripts/generate_synthetic_fixtures.py /tmp/new-tdf-fixtures`. This generation tool needs Python 3.12 and a system libzstd. Neither is required by the reader. The output directory must not already exist. Encoder versions can change compressed bytes or SQLite page layout while decoded values remain unchanged. The SDK reference JSON is a separate source-derived artifact with provenance in `NOTICE.md`.
