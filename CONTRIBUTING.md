# Contributing

This crate is developed for [Sage Plus](https://github.com/pgarrett-scripps/sage-plus) and published on crates.io for anyone who needs to read Bruker timsTOF data from Rust. Open an issue to discuss larger changes, and use pull requests against `main`. Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Keep the library focused on reading completed Bruker TDF acquisitions: frames, calibration, metadata and typed tables. Spectrum synthesis, summing, centroiding, writing acquisition files, the Bruker SDK and native C SQLite or Zstandard dependencies are outside its scope. Scientific conversions must use recorded source metadata with independently justified equations. Unsupported models must return explicit errors.

Development uses Rust 1.97.1 from `rust-toolchain.toml`, which pins formatting and lint results; the published crate only requires Rust 1.97 (`rust-version`). Before submitting a change, run:

```sh
scripts/local-gate.sh
```

That one command runs every CI check that can run on a single Linux machine, in
the same form CI runs it, and ends with a summary of which of the ten CI jobs it
covered. It is the check to run; a hand-picked subset is how a change can reach
CI with, for example, a fuzz harness that no longer compiles. The steps are:

- `cargo fmt --all --check`, for the crate and for the `fuzz/` workspace (CI `fmt`);
- `cargo clippy --locked --all-targets -- -D warnings`, with all features and with
  the default ones (CI `clippy`);
- `cargo test --locked`, with the C compiler disabled so the pure-Rust default
  features stay pure, then `cargo test --locked --all-features` (CI `test`);
- `cargo +1.97.0 build --locked --all-features --lib`, the oldest supported
  compiler (CI `msrv`);
- `cargo doc --locked --no-deps --all-features` with `RUSTDOCFLAGS="-D warnings"`
  (CI `docs`);
- `cargo deny --locked check` (CI `deny`);
- the packaged file list, the licence and notice files it must carry, the crate
  size, and `cargo publish --locked --dry-run` (CI `package`);
- `cargo check --locked --all-targets --manifest-path fuzz/Cargo.toml`, plus
  `fuzz/Cargo.lock` being unchanged (CI `fuzz-smoke`, in part).

Steps whose tool is missing are reported as skipped, with the command that
installs it, rather than failing the run: `cargo deny` needs
`cargo install cargo-deny --locked`, and the MSRV step needs
`rustup toolchain install 1.97.0`. Builds go to `$CARGO_TARGET_DIR`, or
`~/.cache/sage-plus-tdf-target` when that is unset, never into the checkout.

Three CI jobs cannot be finished locally. `test (macos-latest)` and `test
(windows-latest)` need those operating systems. `fuzz-smoke` builds the three
libFuzzer targets with cargo-fuzz 0.13.2 under the pinned
`nightly-2026-09-25` and AddressSanitizer, then fuzzes each for 30 seconds; the
sanitizer and the pinned nightly are the reason it cannot be reproduced by a
plain `cargo` run, so the gate compiles the workspace with the development
toolchain instead. That compile is what catches a change the harness cannot
follow, which is the failure mode that has actually happened; only CI can find a
crash. Push the branch and wait for CI green even when the gate passes.

The `minitdf` feature builds the C `zstd` library through `parquet`, so `--all-features` needs a C compiler. The default features must stay pure Rust, which is why the gate and CI test them with the C compiler disabled.

Include a focused regression test for parsing or scientific behavior changes. Synthetic fixtures are preferred for malformed inputs. Record the origin, license and independent expected result of every reference fixture. Do not commit private acquisition data, patient information, access tokens or unpublished results. Describe any changes to supported formats or calibration models in the README.

Retain existing copyright and license notices. Identify the source and license of any borrowed code or fixture in `NOTICE.md`. The aggregate package is declared `Apache-2.0 AND MIT`, which does not offer a choice between those licenses. Maintainers must review provenance before accepting external material.

Report potential security issues according to [SECURITY.md](SECURITY.md).

To regenerate equivalent synthetic SQLite, Zstandard and LZF fixtures, run `python3 scripts/generate_synthetic_fixtures.py /tmp/new-tdf-fixtures`. This generation tool needs Python 3.12 and a system libzstd. Neither is required by the reader. The output directory must not already exist. Encoder versions can change compressed bytes or SQLite page layout while decoded values remain unchanged. The SDK reference JSON is a separate source-derived artifact with provenance in `NOTICE.md`.
