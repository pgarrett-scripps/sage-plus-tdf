# TimsRust Reader

A read-only Rust reader for completed Bruker TDF `.d` directories. Derived from MannLabs TimsRust 0.6.6 at `80e235a`, with upstream history retained.

The `0.1.1-rc.1` public release candidate is prepared in a private repository. The crate has not been published to crates.io, and no hosted API documentation is available yet. This project is an independent derivative, not an upstream MannLabs release or an official Bruker product.

The library exposes original frame IDs, scan offsets, TOF indices, detector intensities and source SQLite metadata. It does not generate pseudo-DDA spectra, merge DIA windows, centroid peaks or write acquisition files.

## Scope

- Reads `analysis.tdf` and `analysis.tdf_bin` using local filesystem paths.
- Uses the Rust Turso SQLite engine, pinned to `0.8.0-pre.13`, in read-only mode.
- Uses Rust Zstandard and LZF decoders. No C SQLite or C Zstandard library, Bruker SDK, Python binding, cloud storage, Parquet dependency or export CLI is included.
- Supports metadata tables and views, case-insensitive schema lookup, quoted identifiers and original sparse frame IDs.
- Preserves every metadata column through `MetadataDatabase::table`, including precursor, isolation-window and calibration tables.
- Provides validated source ModelType 2 mobility conversion with boundary continuation matching Sage Plus's Bruker SDK reference.
- Enforces configurable compressed size, decompressed size, scan and peak limits. Malformed offsets, count mismatches and integer overflows return errors.

This branch deliberately changes the API and package name. It is not a drop-in replacement for the upstream `SpectrumReader`. TDF is supported. TSF, BAF, miniTDF and live acquisitions with active SQLite journals are rejected. Raw TOF coordinates are preserved. `TdfReader::mz_model` supports source ModelType 1 with per-frame T1 temperature and zero dC2, C2, C3 and C4. Other models return `Unsupported`, without fitting or substituting approximate masses.

## Use

Rust 1.97.1 is pinned for reproducible package and CI checks. Install from the private GitHub tag or revision using authenticated Git access. Public crates.io publishing is disabled in the manifest. Calibration tables are loaded once per reader when first requested. `FrameInfo` also exposes recorded polarity, accumulation time, temperature and calibration references when present.

```rust,no_run
use timsrust_reader::TdfReader;

fn read(path: &str) -> timsrust_reader::Result<()> {
    let mut reader = TdfReader::open(path)?;
    let ids: Vec<_> = reader.frames().map(|frame| frame.id).collect();
    for id in ids {
        let frame = reader.read_frame(id)?;
        println!("frame {}: {} ions", id, frame.tof_indices.len());
    }
    Ok(())
}
```

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo run --locked --example inspect -- /path/to/acquisition.d
```

`Limits::default()` allows 256 MiB compressed data, 512 MiB decompressed data, 32 million peaks and 100,000 scans per frame. `TdfReader::with_limits` accepts alternative ceilings. `Limits::metadata` controls metadata materialization and execution. Defaults allow 2 million rows, 256 columns, 16 MiB per source value and approximately 512 MiB of materialized values per query. Queries stop after approximately 100 million VM operations or 30 seconds. These bound this library's output and VM execution, not every internal allocation made by the prerelease database engine. Schema preparation and a single SQLite value may allocate before output checks. Use process isolation when parsing hostile databases.

## Evidence and adoption

See [the code review](REVIEW.md) and [machine-readable qualification](qualification.json). Both DDA and DIA raw-frame digests match upstream TimsRust exactly on four local fixtures, including a 710-frame acquisition with 26,086,520 ions and an SCP DIA acquisition with 131 frames and 4,470,167 ions. The tests include independently generated SQLite and Zstandard fixtures, LZF decoding, metadata views, malformed frames and mobility reference values.

Sage Plus and MSConvert RS have not been switched to this branch. Adoption requires replacing their upstream spectrum synthesis calls with explicit application-level processing over these raw frames. Read-only SQLite and view support are part of this library, so there is no `filemanager` patch to carry. Keep application-level scientific validation during the migration.

## Attribution

Upstream is [MannLabs TimsRust](https://github.com/MannLabs/timsrust). The original MannLabs MIT notice is retained in `LICENSE`, and the Sage MIT notice is retained in `LICENSE-MIT-SAGE`. Upstream crate manifests declare Apache-2.0, whose text is retained in `LICENSE-APACHE`. See `NOTICE.md` for source details. Local acquisitions are not distributed.

## Development and release preparation

See [CONTRIBUTING.md](CONTRIBUTING.md) for development checks, [SECURITY.md](SECURITY.md) for the input trust boundary and vulnerability reporting, and [RELEASING.md](RELEASING.md) for the guarded publication dry run.
