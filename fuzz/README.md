# Bounded parser fuzzing

This is a standalone unpublished cargo-fuzz workspace. It compiles the private decoder and schema modules directly while reusing the public types and calibration models. The normal library exposes no raw-buffer decoder API and has no libFuzzer dependency.

```sh
cargo +1.97.1 install cargo-fuzz --version 0.13.2 --locked
cargo +nightly-2026-09-25 fuzz build --dev
cargo +nightly-2026-09-25 fuzz run decoder --dev -- -max_total_time=30 -timeout=5 -rss_limit_mb=1024 -max_len=65536 -seed=20260926
cargo +nightly-2026-09-25 fuzz run schema_calibration --dev -- -max_total_time=30 -timeout=5 -rss_limit_mb=1024 -max_len=32768 -seed=20260926
```

The decoder target limits scans, peaks and decoded output before calling the production implementation. Its seeds include independent LZF and checked Zstandard fixtures. The schema target turns a JSON object into a typed SQLite row and exercises strict typed frame extraction, bounds checking and the source mass and mobility models. Its seeds are derived from the attributed fixtures documented in `NOTICE.md`.

Fuzzing uses AddressSanitizer and development-only libFuzzer C++ tooling. The shipped reader remains a Rust-only runtime. CI bounds each target's elapsed execution time, per-input timeout, input length and process RSS. A short fuzz run is a regression check, not proof that every malformed input is safe. Retain minimized crash inputs as ordinary regression fixtures after review.

CI uses the development profile to keep instrumentation builds bounded. Both development and optimized runs retain AddressSanitizer. The committed fuzz lockfile is checked for changes after execution because cargo-fuzz 0.13.2 has no `--locked` build option.
