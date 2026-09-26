# Source attribution

This repository preserves the Git history of MannLabs TimsRust through commit `80e235a331057e0ef922eb3c784b7a631c9b9666` (0.6.6).

The TDF byte shuffling, scan offsets and TOF reconstruction in `src/compression.rs` derive from the upstream `crates/timsrust-tdf/src/frame_reader/compression1.rs` and `compression2.rs`. Bounds checks, resource limits and error paths were rewritten for the narrowed reader API.

Mobility equations and the reference fixture come from the local Sage Plus `crates/sage-cloudpath/src/tims_mobility.rs` and `tests/data/bruker/tims_calibration_sdk.json`. The latter records outputs of Bruker `tims_scannum_to_oneoverk0`, including fractional coordinates and boundary continuation. No Bruker SDK code or binaries are included.

Synthetic SQLite and compressed frame fixtures were generated locally for these regression tests. No private acquisition is included.

The original repository license notice and upstream package license declaration are both retained. Dependencies keep their own licenses.
