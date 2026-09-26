#!/usr/bin/env python3
"""Fetch hash-pinned public data and qualify only explicit derived LZF copies.

This opt-in network tool uses Python's SQLite and an independent LZF decoder.
It never changes the downloaded source metadata. Pass a built inspect executable.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import sqlite3
import struct
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def fetch(entry, destination):
    if destination.exists():
        data = destination.read_bytes()
    else:
        with urllib.request.urlopen(entry["url"], timeout=120) as response:
            data = response.read()
    if hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ValueError(f"source hash mismatch: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists():
        destination.write_bytes(data)
    return data


def lzf(data):
    output = bytearray()
    cursor = 0
    while cursor < len(data):
        control = data[cursor]
        cursor += 1
        if control < 32:
            count = control + 1
            if cursor + count > len(data):
                raise ValueError("truncated LZF literal")
            output.extend(data[cursor:cursor + count])
            cursor += count
        else:
            count = control >> 5
            reference = len(output) - ((control & 31) << 8) - 1
            if count == 7:
                count += data[cursor]
                cursor += 1
            reference -= data[cursor]
            cursor += 1
            if reference < 0:
                raise ValueError("invalid LZF reference")
            for index in range(count + 2):
                output.append(output[reference + index])
        if len(output) > 128 * 1024 * 1024:
            raise ValueError("independent decoder output bound")
    return output


def independent_frame(binary, offset):
    size, scans = struct.unpack_from("<II", binary, offset)
    if offset + size > len(binary) or scans > 100_000:
        raise ValueError("truncated or excessive LZF frame")
    table = struct.unpack_from(f"<{scans + 1}I", binary, offset + 8)
    if table[0] != 8 + 4 * (scans + 1) or table[-1] != size:
        raise ValueError("invalid LZF extent table")
    scan_offsets = [0]
    peaks = []
    for start, end in zip(table, table[1:]):
        if start > end or end > size:
            raise ValueError("invalid LZF scan extent")
        decoded = lzf(binary[offset + start:offset + end])
        tof = 0
        for (value,) in struct.iter_unpack("<i", decoded):
            if value > 0:
                tof += 1
                peaks.append((tof, value))
            else:
                tof -= value + 1
        scan_offsets.append(len(peaks))
    return scans, scan_offsets, peaks


def derive_and_compare(source, destination, keep, inspect):
    if destination.exists():
        raise ValueError(f"derived destination must be new: {destination}")
    shutil.copytree(source, destination)
    binary = (source / "analysis.tdf_bin").read_bytes()
    digest = hashlib.sha256()
    total = 0
    edits = []
    with sqlite3.connect(destination / "analysis.tdf") as connection:
        rows = connection.execute("SELECT Id, NumScans, NumPeaks, TimsId FROM Frames ORDER BY Id").fetchall()
        connection.execute("DELETE FROM Frames WHERE Id > ?", (keep,))
        for frame_id, metadata_scans, expected_peaks, offset in rows:
            if frame_id > keep:
                continue
            scans, offsets, peaks = independent_frame(binary, offset)
            if len(peaks) != expected_peaks:
                raise ValueError("decoded peak count differs from source metadata")
            connection.execute("UPDATE Frames SET NumScans = ? WHERE Id = ?", (scans, frame_id))
            edits.append({"frame_id": frame_id, "source_scans": metadata_scans, "derived_scans": scans})
            digest.update(struct.pack("<Q", frame_id))
            for scan_offset in offsets:
                digest.update(struct.pack("<Q", scan_offset))
            for tof, intensity in peaks:
                digest.update(struct.pack("<II", tof, intensity))
            total += len(peaks)
    expected = {"frames": keep, "peaks": total, "raw_digest": digest.hexdigest()}
    actual = json.loads(subprocess.check_output([str(inspect), str(destination)], text=True))
    if actual != expected:
        raise ValueError(f"independent raw-array mismatch: {actual} != {expected}")
    return {"fixture": destination.name, "independent_reference": expected, "matches_reader": True,
            "metadata_changes": edits, "omitted_frames_after": keep,
            "scope": "derived decoder qualification only, source metadata is inconsistent"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--inspect", type=Path, required=True)
    args = parser.parse_args()
    args.destination.mkdir(parents=True, exist_ok=True)
    pins = json.loads((ROOT / "public-corpus.json").read_text())
    for entry in pins["tdfpy_files"]:
        fetch(entry, args.destination / entry["fixture"] / entry["file"])
    archive = fetch(pins["pwiz_archive"], args.destination / "pwiz.tar.bz2")
    names = {"ThyroglobMRM000003.d": 2, "Hela_QC_PASEF_Slot1-first-6-frames.d": 5}
    # Extract only two explicitly named regular acquisition files per dataset.
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:bz2") as tar:
        for name in names:
            for filename in ("analysis.tdf", "analysis.tdf_bin"):
                member = tar.getmember(f"Reader_Bruker_Test.data/{name}/{filename}")
                if not member.isfile() or member.size > 64 * 1024 * 1024:
                    raise ValueError("unexpected archive member")
                destination = args.destination / name / filename
                destination.parent.mkdir(parents=True, exist_ok=True)
                data = tar.extractfile(member).read()
                if destination.exists() and destination.read_bytes() != data:
                    raise ValueError("existing source differs from pinned archive")
                destination.write_bytes(data)
    results = []
    for name, keep in names.items():
        source = args.destination / name
        result = subprocess.run([str(args.inspect), str(source)], text=True, capture_output=True)
        if result.returncode == 0 or "differs from metadata count" not in result.stderr:
            raise ValueError("original inconsistent fixture did not return expected rejection")
        results.append({"fixture": name, "expected_rejection": "binary scan count differs from metadata"})
        results.append(derive_and_compare(source, args.destination / f"derived-{name}", keep, args.inspect))
    expected_raw = {entry["fixture"]: entry for entry in
                    json.loads((ROOT / "qualification.json").read_text())["fixtures"]}
    for name in ("example_prm.d", "example_dia.d"):
        actual = json.loads(subprocess.check_output(
            [str(args.inspect), str(args.destination / name)], text=True))
        expected = {key: expected_raw[name][key] for key in ("frames", "peaks", "raw_digest")}
        if actual != expected:
            raise ValueError(f"raw acquisition regression: {name}")
        results.append({"fixture": name, **actual})
    (args.destination / "qualification-results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
