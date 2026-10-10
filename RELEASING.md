# Releasing

Releases are driven by tags. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`, which:

1. checks that the tag equals `v` plus the `version` in `Cargo.toml`, and that `CHANGELOG.md` has a dated (not "unreleased") `## X.Y.Z` section;
2. runs the tests on Linux, macOS and Windows, with default and all features;
3. runs `cargo publish --dry-run`, then `cargo publish` to crates.io;
4. creates the GitHub release `vX.Y.Z`, using that version's CHANGELOG section as the notes. Versions with a `-` suffix are marked as prereleases.

crates.io versions cannot be replaced or deleted, only yanked, so publishing happens only after every check passes. The GitHub release is created after the publish succeeds.

## One-time setup

crates.io Trusted Publishing only works for a crate that already exists, so the first release uses an API token.

1. Create a crates.io API token with the `publish-new` and `publish-update` scopes, restricted to the crate `sage-plus-tdf`, with a short expiry.
2. Save it as the repository secret `CARGO_REGISTRY_TOKEN` (Settings, Secrets and variables, Actions).
3. Release as described below. The workflow tries Trusted Publishing first, sees it is not configured yet and falls back to the secret.
4. On crates.io, open the crate's Settings, Trusted Publishing, and add a GitHub publisher: owner `pgarrett-scripps`, repository `sage-plus-tdf`, workflow `release.yml`, no environment.
5. Delete the `CARGO_REGISTRY_TOKEN` secret and revoke the token. Later releases use short-lived OIDC tokens from `rust-lang/crates-io-auth-action`.

## Cutting a release

1. On a branch, set `version` in `Cargo.toml`, run `cargo update --workspace` so `Cargo.lock` matches, and change the CHANGELOG heading from `## X.Y.Z, unreleased` to `## X.Y.Z, YYYY-MM-DD`.
2. Run the checks in [CONTRIBUTING.md](CONTRIBUTING.md) and `cargo publish --dry-run --locked`. Merge to `main` once CI is green.
3. Tag the merged commit on `main` and push the tag:

   ```sh
   git switch main && git pull --ff-only
   git tag -a vX.Y.Z -m "sage-plus-tdf X.Y.Z"
   git push origin vX.Y.Z
   ```

4. Watch the Release workflow under Actions. Check the new version on crates.io and that docs.rs built it.

If the workflow fails before `cargo publish`, fix the problem on `main`, delete the tag (`git push --delete origin vX.Y.Z` and `git tag -d vX.Y.Z`) and tag again. If it fails after publishing, do not reuse the version: create the GitHub release by hand (`gh release create vX.Y.Z --notes-file <notes>`) or fix forward with a new patch version.
