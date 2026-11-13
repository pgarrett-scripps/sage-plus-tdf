#!/usr/bin/env bash
# Run every CI check that can run on one Linux machine, in one command.
#
#   scripts/local-gate.sh
#
# Each step is labelled with the CI job it stands for, and the summary at the end
# says which of the ten jobs this covered and which it could not. The exit status
# is 0 only when every step that ran passed.
#
# Two jobs cannot run here at all: `test (macos-latest)` and `test
# (windows-latest)` need those operating systems. A third, `fuzz-smoke`, runs its
# libFuzzer targets under a pinned nightly with cargo-fuzz and AddressSanitizer;
# this script compiles the fuzz workspace instead, which is what catches a source
# change that breaks the harness. See CONTRIBUTING.md.
#
# Builds go to ~/.cache by default, on the root drive, not into the repository.
set -uo pipefail

cd "$(dirname "$0")/.."
repo="$PWD"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/sage-plus-tdf-target}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export CARGO_INCREMENTAL=0
mkdir -p "$CARGO_TARGET_DIR"

# Keep these in sync with .github/workflows/ci.yml and rust-toolchain.toml.
rust_msrv=1.97.0
fuzz_nightly=nightly-2026-09-25

labels=()
jobs=()
results=()

run() {
    local label="$1" job="$2"
    shift 2
    printf '\n=== %s (CI: %s) ===\n' "$label" "$job"
    printf '    %s\n' "$*"
    if "$@"; then
        labels+=("$label")
        jobs+=("$job")
        results+=(pass)
    else
        printf '!!! %s FAILED\n' "$label"
        labels+=("$label")
        jobs+=("$job")
        results+=(FAIL)
    fi
}

skip() {
    local label="$1" job="$2" why="$3"
    printf '\n=== %s (CI: %s) === skipped: %s\n' "$label" "$job" "$why"
    labels+=("$label")
    jobs+=("$job")
    results+=("skip: $why")
}

# fmt: both workspaces, as CI does.
run "fmt, crate" fmt cargo fmt --all --check
run "fmt, fuzz" fmt cargo fmt --all --check --manifest-path fuzz/Cargo.toml

# clippy: all features and default features, both denying warnings.
run "clippy, all features" clippy \
    cargo clippy --locked --all-targets --all-features -- -D warnings
run "clippy, default features" clippy \
    cargo clippy --locked --all-targets -- -D warnings

# test: the default features must build with no C compiler available, as the
# published crate promises; `minitdf` then builds C zstd through parquet.
run "test, default features (no C compiler)" "test (ubuntu-latest)" \
    env CC=prohibited-native-c-compiler CXX=prohibited-native-cxx-compiler \
    cargo test --locked
run "test, all features" "test (ubuntu-latest)" cargo test --locked --all-features

# msrv: the published crate must build on the oldest supported compiler.
if rustup toolchain list 2>/dev/null | grep -q "^$rust_msrv"; then
    run "msrv build" msrv \
        cargo "+$rust_msrv" build --locked --all-features --lib
else
    skip "msrv build" msrv "install it with: rustup toolchain install $rust_msrv"
fi

# docs: a broken intra-doc link or an undocumented public item fails CI.
run "docs" docs env RUSTDOCFLAGS="-D warnings" \
    cargo doc --locked --no-deps --all-features

# deny: licences, bans, advisories and sources.
if command -v cargo-deny >/dev/null 2>&1; then
    # cargo-deny takes --locked before the subcommand, unlike cargo itself.
    run "deny" deny cargo deny --locked check
else
    skip "deny" deny "install it with: cargo install cargo-deny --locked"
fi

# package: the crates.io artifact carries the licences and no development files.
package_files="$CARGO_TARGET_DIR/package-files.txt"
check_package() {
    cargo package --locked --list >"$package_files" || return 1
    local required
    for required in LICENSE LICENSE-APACHE LICENSE-MIT-SAGE LICENSE-MIT-TDFPY \
        NOTICE.md README.md CHANGELOG.md; do
        grep -qx "$required" "$package_files" || {
            echo "missing $required from package"
            return 1
        }
    done
    if grep -E '^(fuzz|scripts|\.github)/' "$package_files"; then
        echo "development-only files in package"
        return 1
    fi
    cargo package --locked --no-verify "${dirty_flag[@]}" || return 1
    local crate size
    crate="$(find "$CARGO_TARGET_DIR/package" -maxdepth 1 -name 'sage-plus-tdf-*.crate' | head -1)"
    size="$(stat -c %s "$crate")"
    echo "$crate: $size bytes"
    # crates.io rejects crates over 10 MB; CI keeps this far below it.
    test "$size" -lt 2000000
}

# `cargo publish --dry-run` refuses an unclean tree, which a local run often has.
dirty_flag=()
dirty_note=""
if [ -n "$(git -C "$repo" status --porcelain)" ]; then
    dirty_flag=(--allow-dirty)
    dirty_note=" (uncommitted changes: packaged with --allow-dirty)"
fi

run "package contents and size" package check_package
run "publish dry run" package \
    cargo publish --locked --dry-run "${dirty_flag[@]}"

# fuzz-smoke, as far as it goes here: compile the harness and every target. This
# is what caught nothing in commit a430347, whose schema change broke the fuzz
# harness while all nine other jobs passed.
run "fuzz workspace compiles" fuzz-smoke \
    env CARGO_TARGET_DIR="$CARGO_TARGET_DIR/fuzz" \
    cargo check --locked --all-targets --manifest-path fuzz/Cargo.toml
run "fuzz lock unchanged" fuzz-smoke git diff --exit-code -- fuzz/Cargo.lock

if command -v cargo-fuzz >/dev/null 2>&1; then
    skip "fuzz target runs" fuzz-smoke \
        "cargo-fuzz is installed; CI runs the three targets under $fuzz_nightly with AddressSanitizer"
else
    skip "fuzz target runs" fuzz-smoke \
        "needs cargo-fuzz 0.13.2 and $fuzz_nightly; CI runs the three targets"
fi

skip "test, macOS" "test (macos-latest)" "no macOS host"
skip "test, Windows" "test (windows-latest)" "no Windows host"

printf '\n===== summary =====\n'
failed=0
for i in "${!labels[@]}"; do
    case "${results[$i]}" in
    pass) printf '  pass  %-42s %s\n' "${labels[$i]}" "[${jobs[$i]}]" ;;
    FAIL)
        printf '  FAIL  %-42s %s\n' "${labels[$i]}" "[${jobs[$i]}]"
        failed=1
        ;;
    *) printf '  skip  %-42s %s  %s\n' "${labels[$i]}" "[${jobs[$i]}]" "${results[$i]#skip: }" ;;
    esac
done

printf '\nCI jobs fully covered here: fmt, clippy, test (ubuntu-latest), msrv, docs, deny, package\n'
printf 'CI jobs partly covered:     fuzz-smoke (compiles, does not fuzz)\n'
printf 'CI jobs not covered:        test (macos-latest), test (windows-latest)\n'
if [ -n "${SAGE_PLUS_MINITDF_ZENODO:-}" ] || [ -n "${SAGE_PLUS_TSF_URINE:-}" ]; then
    printf 'Real-data tests: ran for the variables that are set.\n'
else
    printf 'Real-data tests: skipped, SAGE_PLUS_MINITDF_ZENODO and SAGE_PLUS_TSF_URINE unset (CI skips them too).\n'
fi
printf 'Target directory: %s%s\n' "$CARGO_TARGET_DIR" "$dirty_note"

if [ "$failed" -ne 0 ]; then
    printf '\nLOCAL_GATE_FAILED\n'
    exit 1
fi
printf '\nLOCAL_GATE_PASSED\n'
