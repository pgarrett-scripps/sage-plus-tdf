# Public release preparation

The repository remains private and `publish = false` remains in the real manifest. No crates.io package or hosted docs.rs page is claimed to exist. This document prepares a future launch and does not authorize publishing, changing repository visibility or creating a release tag.

## Naming and provenance

On 2026-09-26, the crates.io API returned HTTP 404 for both `timsrust-reader` and `timsrust_reader`. The proposed name appears available at that time and is not reserved. Recheck availability immediately before launch. This project is an independent derivative of MannLabs TimsRust, not an upstream release or an official Bruker product.

Review [NOTICE.md](NOTICE.md) and all packaged license files. Upstream's root license is MIT, while its TDF crate manifest declares Apache-2.0. The combined package retains both obligations as `Apache-2.0 AND MIT`. Do not change that to an `OR` expression without verifying permission and provenance. The Sage-derived mobility source also retains the original Sage MIT notice.

Before making history public, complete the repository history and fixture provenance audit. Confirm the redistribution status of the calibration reference values and every retained historical file. Changing the current package file list does not remove historical content.

## Safe preflight

Run the normal checks in [CONTRIBUTING.md](CONTRIBUTING.md), commit the candidate and ensure the working tree is clean. Python 3.12 or later is needed for the release helper only, not for the reader library.

```sh
python3 scripts/publication_preflight.py
```

The helper exports the committed tree to a temporary directory. It changes the publication guard only inside that disposable staging copy and invokes `cargo publish --dry-run --locked --registry crates-io`. It never invokes an upload command, does not create a tag and checks that the real manifest remains unchanged. `--offline` can be used when the dependencies and registry metadata are already cached.

Build documentation with the same target and feature selection declared for docs.rs:

```sh
DOCS_RS=1 RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps --target x86_64-unknown-linux-gnu
```

This verifies the documentation locally. An actual docs.rs build can only be confirmed after a future registry publication.

## Explicit launch gate

A future public launch requires a separate explicit instruction from the repository owner. Before that action:

1. Resolve naming, attribution, fixture redistribution and historical-content findings.
2. Confirm a private vulnerability reporting channel, repository ownership and the intended crates.io owner account.
3. Choose a public release version. Keep the existing private `v0.1.0` release immutable and use a new version for any changed release.
4. Recheck the registry name, pinned dependency advisories and exact candidate CI results on Linux, macOS and Windows.
5. Review the actual `.crate` file list, unpack it and rerun its tests. Record the archive SHA256 and source commit.
6. After explicit authorization, make only the agreed visibility and publication changes, then verify the resulting registry package and documentation.

The current CI only builds, tests and performs a disposable dry run. It has no deployment job, publishing token or tag-triggered upload path.
