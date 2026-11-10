# CLAUDE.md

Notes for an agent here. `CONTRIBUTING.md` stays authoritative; this holds what the code does not show.

## What the crate is, and is not

A read-only reader for completed Bruker TDF and TSF `.d` acquisitions and ProteoScape
miniTDF spectra: frames, calibration, metadata and typed tables, pure Rust, `#![forbid(unsafe_code)]`.

Everything downstream of reading belongs to [Sage
Plus](https://github.com/pgarrett-scripps/sage-plus): summing, smoothing, centroiding, window
splitting, spectrum synthesis, writing acquisition files, the Bruker SDK, native C SQLite or
Zstandard. A task asking for one of those is in the wrong repository. Default features stay
pure Rust; only the optional `minitdf` feature pulls in C code, via `parquet`.

## The public API is load-bearing

Sage Plus depends on this crate from crates.io, not from this directory, so a change here
is invisible to it until a release is published. Adding an item is cheap. Renaming or
removing one, or changing a signature, breaks Sage Plus's build and needs Ty's approval
first. Prefer additive changes, and say in the commit message what a caller must do.

## Layout

`src/` is flat, one concern per file, and the layering is the repository's main invariant:

- `database.rs` opens `analysis.tdf` through `turso_core` read-only and enforces
  `MetadataLimits`, `schema.rs` interprets columns, `tables.rs` exposes typed rows. Keep
  that order: no SQL above `database.rs`, no column names above `schema.rs`.
- `reader.rs` (`TdfReader`), `tsf.rs` (`TsfReader`) and `minitdf.rs` (`MiniTdfReader`) are
  the three public readers, over `compression.rs` (Zstandard and LZF), `mass.rs` and
  `mobility.rs` (calibration), and `error.rs`.
- `Limits` bounds every allocation a file can ask for. A new parser reads through it and
  reports an exceeded bound as `ErrorKind::ResourceLimit`, malformed bytes as
  `ErrorKind::CorruptData`.

## Before you commit

The five checks in `CONTRIBUTING.md`: `fmt --all --check`, `clippy -- -D warnings`, `test`,
`doc --no-deps` with `RUSTDOCFLAGS="-D warnings"`, and `deny check`; the middle three take
`--locked --all-features`, which needs a C compiler. This machine is shared, so every cargo
build, test and clippy run goes through the `ajs` scheduler, never `exclusive=true` unless the
timing will be reported; only `cargo fmt` and a single-crate `cargo check` run directly.

```sh
ajs submit --cpu 4 --mem 8G --max-runtime 25m --title "sage-plus-tdf checks" \
  --description "what this gates" --meta est=8m -- bash -lc 'cd ~/Repos/sage-plus-tdf \
  && cargo clippy --locked --all-targets --all-features -- -D warnings \
  && cargo test --locked --all-features'
```

## Fixtures are evidence

`tests/fixtures/` and the two SDK reference JSONs have their origin, licence and
independently obtained expected values recorded in `NOTICE.md`. They are inputs: do not
regenerate, re-round or edit one to make a test pass. A failing fixture means the code is
wrong, or the expectation was never justified. New malformed-input tests use synthetic
fixtures (`scripts/generate_synthetic_fixtures.py`) or build the bytes in the test, and
private acquisition data is never committed.

## Calibration stays exact

Conversions use recorded source metadata and equations justified in `NOTICE.md`. An
unsupported `ModelType` returns `ErrorKind::Unsupported` (`src/mass.rs:157`), never an
approximation or a silently linear scale. Document model changes in the README.

## Releases and upstream

`RELEASING.md` is the process: pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`,
which checks the tag against `Cargo.toml`, tests on three platforms, publishes to crates.io
and creates the GitHub release. Nothing is published by hand. `UPSTREAM.md` records the
merge base against MannLabs timsrust and which upstream releases were reviewed, and why each
does or does not apply; add an entry there rather than re-judging a reviewed release.

## Ask Ty first

A version bump in `Cargo.toml`, a dated CHANGELOG heading, a tag, a crates.io publish, a
push to `main`, a force-push, a public API removal or rename, or anything sent upstream. An
`[Unreleased]` CHANGELOG entry and a pushed feature branch need no permission.
