#!/usr/bin/env python3
"""Generate equivalent synthetic fixtures with independent SQLite and Zstandard encoders.

Requires Python 3.12 and a system libzstd for fixture generation only.
The output directory must not exist. SQLite layout and Zstandard bytes may vary
across encoder versions while decoded values remain the same.
"""
import argparse
import ctypes
import ctypes.util
from pathlib import Path
import shutil
import sqlite3
import struct


def zstandard(data, checksum=False):
    lib = ctypes.CDLL(ctypes.util.find_library("zstd"))
    lib.ZSTD_createCCtx.restype = ctypes.c_void_p
    lib.ZSTD_CCtx_setParameter.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
    lib.ZSTD_compress2.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t]
    lib.ZSTD_compress2.restype = ctypes.c_size_t
    lib.ZSTD_isError.argtypes = [ctypes.c_size_t]
    lib.ZSTD_isError.restype = ctypes.c_uint
    lib.ZSTD_freeCCtx.argtypes = [ctypes.c_void_p]
    context = lib.ZSTD_createCCtx()
    if not context:
        raise RuntimeError("Zstandard context allocation failed")
    try:
        result = lib.ZSTD_CCtx_setParameter(context, 201, int(checksum))
        if lib.ZSTD_isError(result):
            raise RuntimeError("Zstandard checksum configuration failed")
        output = ctypes.create_string_buffer(len(data) + 1024)
        size = lib.ZSTD_compress2(context, output, len(output), data, len(data))
        if lib.ZSTD_isError(size):
            raise RuntimeError("Zstandard fixture encoding failed")
        return output.raw[:size]
    finally:
        lib.ZSTD_freeCCtx(context)


def database(path, compression, rows):
    connection = sqlite3.connect(path)
    connection.execute("CREATE TABLE FrameSource (Id INTEGER PRIMARY KEY, Time REAL, MsMsType INTEGER, NumScans INTEGER, NumPeaks INTEGER, TimsId INTEGER, TimsCalibration INTEGER)")
    connection.executemany("INSERT INTO FrameSource VALUES (?,?,?,?,?,?,?)", rows)
    connection.execute("CREATE TABLE GlobalMetaData (Key TEXT, Value TEXT)")
    connection.execute("INSERT INTO GlobalMetaData VALUES ('TimsCompressionType',?)", (str(compression),))
    connection.execute("CREATE TABLE Precursors (Id INTEGER, Parent INTEGER, ScanNumber REAL)")
    connection.execute("CREATE TABLE TimsCalibration (Id INTEGER, ModelType INTEGER,C0 REAL,C1 REAL,C2 REAL,C3 REAL,C4 REAL,C5 REAL,C6 REAL,C7 REAL,C8 REAL,C9 REAL)")
    coefficients = [1, 670, 207.7756769319643, 59.252667636882194, 33, 1, 0.020988900147314915, 131.44011309779785, 12.97123172958875, 2558.7169250593142]
    connection.execute("INSERT INTO TimsCalibration VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", [1, 2, *coefficients])
    connection.execute('CREATE TABLE "quoted""table" (Data BLOB)')
    connection.execute('INSERT INTO "quoted""table" VALUES (?)', (bytes([1, 2, 255]),))
    connection.execute("CREATE VIEW Frames AS SELECT * FROM FrameSource")
    connection.commit()
    connection.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    root = parser.parse_args().output
    root.mkdir(parents=True, exist_ok=False)
    for name in ["synthetic.d", "lzf.d"]:
        (root / name).mkdir()
    words = [3, 0, 4, 1, 10, 5, 20, 9, 30]
    shuffled = bytes(word.to_bytes(4, "little")[byte] for byte in range(4) for word in words)
    compressed = zstandard(shuffled)
    frame = struct.pack("<II", 8 + len(compressed), 3) + compressed
    (root / "synthetic.d/analysis.tdf_bin").write_bytes(frame * 2)
    database(root / "synthetic.d/analysis.tdf", 2, [(10, 1.2, 0, 3, 3, 0, 1), (42, 1.3, 9, 3, 3, len(frame), 1)])
    # A literal-only LZF block independently encodes three signed words.
    literal = bytes([11]) + struct.pack("<iii", -4, 10, 20)
    end = 20 + len(literal)
    frame = struct.pack("<IIIII", end, 2, 20, end, end) + literal
    (root / "lzf.d/analysis.tdf_bin").write_bytes(frame)
    database(root / "lzf.d/analysis.tdf", 1, [(10, 1.2, 0, 2, 2, 0, 1)])
    shutil.copytree(root / "synthetic.d", root / "metadata.d")
    connection = sqlite3.connect(root / "metadata.d/analysis.tdf")
    statements = [
        "ALTER TABLE FrameSource RENAME COLUMN NumScans TO numscans",
        "ALTER TABLE FrameSource ADD COLUMN MzCalibration INTEGER DEFAULT 1",
        "ALTER TABLE FrameSource ADD COLUMN T1 REAL DEFAULT 25.548975152547968",
        "CREATE TABLE MzCalibration(Id,ModelType,DigitizerTimebase,DigitizerDelay,T1,T2,dC1,dC2,C0,C1,C2,C3,C4)",
        "INSERT INTO MzCalibration VALUES(1,1,0.2,25870,25.62513732863827,25.019305770074833,20,0,317.86417140238336,153159.143753129,0,0,0)",
        "CREATE VIEW Oversized AS SELECT zeroblob(2048) AS Data",
        "CREATE VIEW Expensive AS WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000000) SELECT SUM(x) AS total FROM n",
        "CREATE VIEW ManyRows AS WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100) SELECT x FROM n",
    ]
    for statement in statements:
        connection.execute(statement)
    connection.commit()
    connection.close()
    shutil.copytree(root / "metadata.d", root / "duplicate-calibration.d")
    connection = sqlite3.connect(root / "duplicate-calibration.d/analysis.tdf")
    connection.execute("INSERT INTO MzCalibration SELECT * FROM MzCalibration")
    connection.commit()
    connection.close()
    (root / "checksum.zstd").write_bytes(zstandard(shuffled, checksum=True))
    print(f"Generated synthetic fixtures in {root}")


if __name__ == "__main__":
    main()
