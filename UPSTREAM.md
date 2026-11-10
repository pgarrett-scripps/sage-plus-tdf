# Upstream

This crate is a fork of [MannLabs timsrust](https://github.com/MannLabs/timsrust).
Every line of code here derives from timsrust 0.6.6 at `80e235a`, the merge
base; upstream history is retained in this repository.

- Remote: `upstream`, https://github.com/MannLabs/timsrust
- Merge base: `80e235a` (timsrust 0.6.6)
- What this fork adds: `git log --oneline upstream/main..main`
- What upstream has that this fork does not: `git log --oneline main..upstream/main`

## Reviews

Upstream releases are read rather than merged by default. This fork is a single
crate with no centroid, SDK, MGF, Parquet-spectra, Python or CLI layer, so most
upstream changes touch files that do not exist here. Each release that has been
looked at is recorded below, so the same commits are not re-examined.

### timsrust 0.6.7 (`430afcb`), reviewed 2026-11-09: does not apply

The release carries two fixes, `a376dbe` and `e9c20c0`, both in
`crates/timsrust-centroid/src/spectrum_reader/narrow_spectrum_reader.rs`, where
the centroid spectrum reader emitted a half-sized DIA isolation window. That
file was removed from this fork in `e2b2f5a`, when the workspace was flattened
to this one crate, and the readers here pass `IsolationWidth` through from the
table without halving it, so the bug never existed here.

A simulated merge, `git merge-tree --write-tree main upstream/main`, conflicts
in `Cargo.lock`, `Cargo.toml` and `README.md` and reports a modify/delete on
the centroid file; resolving all four in this fork's favour leaves the tree
unchanged, which is why the merge was not kept. The convention the upstream fix
defends is documented on `PasefFrameMsMsInfo::isolation_width` and
`DiaFrameMsMsWindow::isolation_width`: a full width in m/z, never halved.
