Fork of MannLabs timsrust by Sander Willems and contributors; not affiliated with MannLabs or Bruker

# sage-plus-tdf

A read-only, pure Rust reader for Bruker TDF and TSF `.d` directories and ProteoScape miniTDF spectra, used by Sage Plus. It reads the file and nothing else: frames, calibration, metadata and typed tables. Summing, smoothing, centroiding and window splitting belong to the application.

Derived from MannLabs timsrust 0.6.6 at `80e235a`, with upstream history retained. Not published to crates.io.

## What it reads

- `analysis.tdf` through the Rust Turso SQLite engine (`turso_core`, pinned to `0.8.0-pre.13`), opened read-only. No C SQLite.
- `analysis.tdf_bin` frames, Zstandard (compression 2) and LZF (compression 1), with positional reads so one `&TdfReader` can be shared across threads.
- Raw frame coordinates: original frame IDs, scan offsets, TOF indices and detector intensities. Nothing is rescaled.
- Typed tables: `Frames`, `GlobalMetadata`, `PasefFrameMsMsInfo`, `Precursors`, `DiaFrameMsMsInfo`, `DiaFrameMsMsWindows`, `DiaFrameMsMsWindowGroups`, `TimsCalibration` and `MzCalibration`. Any other table is available as raw `SqlValue` rows through `TdfReader::metadata`.
- Calibration: the full ModelType 1 m/z model (per-frame `T1` and `T2`, `dC1`, `dC2`, `C0` to `C4`), agreeing with libtimsdata to better than 0.001 ppm, and the ModelType 2 mobility model in both directions. Other models return `Unsupported`.
- timsrust-compatible linear m/z and mobility scales, for callers that need timsrust's uncalibrated output.
- TSF (`TsfReader`): `analysis.tsf` frames, `FrameMsMsInfo` precursors (trigger mass, isolation width, optional charge) and `analysis.tsf_bin` line (centroid) spectra as fractional TOF indices and intensities. Calibrated m/z through ModelType 1 or the ModelType 2 model with its polynomial correction, which agrees with the Bruker SDK to better than 0.001 ppm on public test data. Profile spectra are not read.
- miniTDF (`MiniTdfReader`, `minitdf` feature): the `ms2spectrum.parquet` precursor table and `ms2spectrum.bin` centroided MS2 spectra, under the plain or the `<name>.ms2spectrum.*` names. Adds a pure Rust Parquet dependency with Snappy and Zstandard support. miniTDF MS1 frames (`msframe.*`) are not read.

`TdfReader::open` takes the `.d` directory itself, as does `TsfReader::open`; `MiniTdfReader::open` takes the directory holding the spectrum files. BAF and live acquisitions with active SQLite journals are rejected. LZF frames are decoded exactly as libtimsdata does or rejected; they are never silently wrong.

## Use

```rust,no_run
use sage_plus_tdf::{Result, TdfReader};

fn read(path: &str) -> Result<()> {
    let reader = TdfReader::open(path)?;
    for info in reader.frames() {
        let frame = reader.read_frame(info.id)?;
        let mz = reader.mz_model(info.id)?;
        let first = frame.tof_indices.first().map(|&tof| mz.mz(f64::from(tof)));
        println!("frame {}: {} ions, first m/z {first:?}", info.id, frame.tof_indices.len());
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

`Limits::default()` allows 256 MiB compressed data, 512 MiB decompressed data, 32 million peaks and 100,000 scans per frame. `TdfReader::with_limits` accepts other ceilings, and `Limits::metadata` bounds metadata rows, columns, value sizes and query execution. These bound this library's output, not every allocation inside the prerelease database engine, so isolate processing of hostile databases.

## Errors

`Error::kind()` separates unsupported features, corrupt input, resource ceilings, I/O failures and database failures. `Error::contexts()` names the acquisition path, frame ID, table, field and frame-header byte offset when known, and `Error::root_error()` keeps the original cause. Resource errors report the configured maximum and the observed value. The reader never retries with a larger limit.

## Attribution and license

Upstream is [MannLabs timsrust](https://github.com/MannLabs/timsrust). The MannLabs MIT notice is in `LICENSE`, the Sage MIT notice in `LICENSE-MIT-SAGE`, the tdfpy MIT notice in `LICENSE-MIT-TDFPY` and the Apache-2.0 text in `LICENSE-APACHE`. See `NOTICE.md` for sources and fixture provenance. No Bruker SDK code or binaries are included.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development checks, [SECURITY.md](SECURITY.md) for the input trust boundary and `fuzz/README.md` for bounded parser fuzzing.
