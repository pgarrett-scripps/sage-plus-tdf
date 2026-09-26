# Reader qualification

`qualification.json` records exact raw-array comparisons against the pinned upstream reader. The encoding hashes source frame IDs, every scan offset, and paired TOF indices and intensities. This tests exact parsing and does not claim that an approximate mass calibration agrees with vendor software.

The expanded public DIA and PRM acquisitions come from tdfpy v2.2.0. `public-corpus.json` records download URLs and SHA256 hashes. Fetch and verify the expanded corpus with Python 3 and a compiled inspection example:

```sh
cargo build --locked --example inspect
python3 scripts/qualify_public_corpus.py /tmp/reader-public-corpus --inspect target/debug/examples/inspect
```

Use a fresh destination for each qualification run. The tool downloads pinned bytes, preserves original acquisitions, creates explicitly named derived copies and writes machine-readable results. It does not install or call the Bruker SDK.

## LZF coverage and limits

The pinned ProteoWizard archive supplies real LZF-compressed frames, but its test databases intentionally or historically disagree with the binary scan counts. ThyroglobMRM000003.d declares 274 scans while both binary frames contain 2742. Hela_QC_PASEF_Slot1-first-6-frames.d declares 350 scans for its first five frames while the binary contains 985. The sixth Hela frame also extends beyond the available binary file. Both original datasets must fail with actionable corruption errors. These are negative qualification cases, not supported complete acquisitions.

For decoder qualification only, the script copies the sources, changes `Frames.NumScans` to the validated binary count and removes Hela frame 6 from its derived copy. It leaves the binary bytes unchanged. An independent Python LZF decoder computes each scan offset and raw TOF/intensity pair, validates metadata peak counts and compares the entire digest against this reader. This provides real compressed-payload evidence without relaxing production checks or claiming the derived metadata is scientifically complete. Other metadata tables in the derived files are retained and are not qualified for downstream processing.

`public-lzf-qualification.json` records these exact transformations and independent comparisons. Original archives are never embedded in the crate. Public source provenance is retained in `public-corpus.json`.

Source-derived mobility calibration cases already compare against vendor reference values in `tests/fixtures/tims_calibration_sdk.json`. Mass conversion remains restricted to the explicitly supported source model. Unsupported coefficients return an error. Expanded raw acquisition coverage does not extend that calibration contract.
