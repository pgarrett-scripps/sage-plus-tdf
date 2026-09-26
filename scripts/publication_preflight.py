#!/usr/bin/env python3
"""Verify a publishable staging copy without uploading or changing the checkout."""
import argparse
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    root = Path(__file__).resolve().parent.parent
    manifest = root / "Cargo.toml"
    original = manifest.read_bytes()
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=root):
        raise SystemExit("Commit or remove pending changes before running the publication preflight")
    if original.count(b"publish = false") != 1:
        raise SystemExit("The real package must retain exactly one publish = false guard")
    archive = subprocess.check_output(["git", "archive", "--format=tar", "HEAD"], cwd=root)
    environment = os.environ.copy()
    environment.pop("CARGO_REGISTRY_TOKEN", None)
    environment.pop("CARGO_REGISTRIES_CRATES_IO_TOKEN", None)
    # The command below has no upload mode and accepts no additional Cargo arguments.
    with tempfile.TemporaryDirectory(prefix="timsrust-publication-preflight-") as temporary:
        stage = Path(temporary)
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(stage, filter="data")
        staged_manifest = stage / "Cargo.toml"
        staged_manifest.write_bytes(original.replace(b"publish = false", b'publish = ["crates-io"]', 1))
        environment["CARGO_TARGET_DIR"] = str(root / "target" / "publication-preflight")
        command = ["cargo", "publish", "--dry-run", "--locked", "--registry", "crates-io"]
        try:
            subprocess.run(command, cwd=stage, env=environment, check=True)
        finally:
            if manifest.read_bytes() != original:
                raise RuntimeError("The source manifest changed during the preflight")
    print("Publication preflight passed. No upload occurred and source publication remains disabled.")


if __name__ == "__main__":
    main()
