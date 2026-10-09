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


def tables(directory, binary):
    """Bruker-schema metadata for the typed tables, over the synthetic frames."""
    directory.mkdir()
    shutil.copy(binary, directory / "analysis.tdf_bin")
    connection = sqlite3.connect(directory / "analysis.tdf")
    statements = [
        "CREATE TABLE GlobalMetadata (Key TEXT PRIMARY KEY, Value TEXT)",
        "CREATE TABLE Frames (Id INTEGER PRIMARY KEY, Time REAL NOT NULL, Polarity CHAR(1) NOT NULL, ScanMode INTEGER NOT NULL, MsMsType INTEGER NOT NULL, TimsId INTEGER, MaxIntensity INTEGER NOT NULL, SummedIntensities INTEGER NOT NULL, NumScans INTEGER NOT NULL, NumPeaks INTEGER NOT NULL, MzCalibration INTEGER NOT NULL, T1 REAL NOT NULL, T2 REAL NOT NULL, TimsCalibration INTEGER NOT NULL, PropertyGroup INTEGER, AccumulationTime REAL NOT NULL, RampTime REAL NOT NULL, Pressure REAL)",
        "INSERT INTO Frames VALUES (10, 0.668585, '+', 8, 0, 0, 30, 60, 3, 3, 1, 25.4744657311153, 23.220480964598842, 1, 1, 100.045, 100.045, 2.336)",
        "INSERT INTO Frames VALUES (42, 0.787, '+', 8, 8, 33, 30, 60, 3, 3, 1, 25.4744657311153, 23.220480964598842, 1, 1, 50.0, 100.045, 9e999)",
        "CREATE TABLE MzCalibration (Id INTEGER PRIMARY KEY, ModelType INTEGER NOT NULL, DigitizerTimebase REAL NOT NULL, DigitizerDelay REAL NOT NULL, T1 REAL NOT NULL, T2 REAL NOT NULL, dC1 REAL NOT NULL, dC2 REAL NOT NULL, C0, C1, C2, C3, C4)",
        "INSERT INTO MzCalibration VALUES (1, 1, 0.125, 25835.25, 25.45627710651325, 23.13094591846571, 20.0, 0.0, 280.06963870190276, 153122.31936770084, 0, 0, 0)",
        "CREATE TABLE TimsCalibration (Id INTEGER PRIMARY KEY, ModelType INTEGER NOT NULL, C0, C1, C2, C3, C4, C5, C6, C7, C8, C9)",
        "INSERT INTO TimsCalibration VALUES (1, 2, 1, 935, 239.34640606518187, 102.96946384662338, 33.64485981308411, 1, -0.026580764926972034, 171.42849749723894, 16.838457909616054, 1732.6649859338625)",
        "CREATE TABLE Precursors (Id INTEGER PRIMARY KEY, LargestPeakMz REAL NOT NULL, AverageMz REAL NOT NULL, MonoisotopicMz REAL, Charge INTEGER, ScanNumber REAL NOT NULL, Intensity REAL NOT NULL, Parent INTEGER)",
        "INSERT INTO Precursors VALUES (1, 1488.420559903908, 1488.7961427589423, 1487.919127176326, 2, 275.5991596638655, 1341.0, 10)",
        "INSERT INTO Precursors VALUES (2, 600.5, 600.75, NULL, NULL, 120.25, 88.0, 10)",
        "CREATE TABLE PasefFrameMsMsInfo (Frame INTEGER NOT NULL, ScanNumBegin INTEGER NOT NULL, ScanNumEnd INTEGER NOT NULL, IsolationMz REAL NOT NULL, IsolationWidth REAL NOT NULL, CollisionEnergy REAL NOT NULL, Precursor INTEGER, PRIMARY KEY(Frame, ScanNumBegin))",
        "INSERT INTO PasefFrameMsMsInfo VALUES (42, 263, 288, 1488.7961427589423, 3.0, 44.79260595130748, 1)",
        "CREATE TABLE DiaFrameMsMsWindowGroups (Id INTEGER PRIMARY KEY)",
        "INSERT INTO DiaFrameMsMsWindowGroups VALUES (1)",
        "CREATE TABLE DiaFrameMsMsWindows (WindowGroup INTEGER NOT NULL, ScanNumBegin INTEGER NOT NULL, ScanNumEnd INTEGER NOT NULL, IsolationMz REAL NOT NULL, IsolationWidth REAL NOT NULL, CollisionEnergy REAL NOT NULL)",
        "INSERT INTO DiaFrameMsMsWindows VALUES (1, 125, 530, 812.5, 25.0, 45.08333333333333)",
        "CREATE TABLE DiaFrameMsMsInfo (Frame INTEGER PRIMARY KEY, WindowGroup INTEGER NOT NULL)",
        "INSERT INTO DiaFrameMsMsInfo VALUES (42, 1)",
        "CREATE TABLE Blobs (Id INTEGER, Data BLOB)",
        "INSERT INTO Blobs VALUES (1, randomblob(8192))",
    ]
    for statement in statements:
        connection.execute(statement)
    metadata = {
        "TimsCompressionType": "2",
        "AcquisitionSoftware": "Bruker otofControl",
        "MzAcqRangeLower": "100.000000",
        "MzAcqRangeUpper": "1700.000000",
        "OneOverK0AcqRangeLower": "0.6",
        "OneOverK0AcqRangeUpper": "1.6",
        "DigitizerNumSamples": "397888",
    }
    connection.executemany("INSERT INTO GlobalMetadata VALUES (?, ?)", metadata.items())
    connection.commit()
    connection.close()


def tsf(directory):
    """A TSF DDA run: two calibration models, FrameMsMsInfo, a frame without data."""
    directory.mkdir()
    blocks = []
    offsets = []
    position = 0
    spectra = {
        # Frame 1 indices are SDK reference points of the urine TSF run.
        1: ([221.5, 77259.97916666667, 227807.9090909091], [10.0, 250.5, 3.25]),
        2: ([1000.25, 2000.5], [7.0, 8.0]),
    }
    for frame in [1, 2]:
        indices, intensities = spectra[frame]
        payload = struct.pack(f"<{len(indices)}d", *indices) + struct.pack(f"<{len(intensities)}f", *intensities)
        compressed = zstandard(payload)
        profile = bytes(16)  # Profile data shares the block and is skipped.
        block = struct.pack("<II", 8 + len(compressed) + len(profile), len(compressed)) + compressed + profile
        offsets.append(position)
        position += len(block)
        blocks.append(block)
    (directory / "analysis.tsf_bin").write_bytes(b"".join(blocks))
    connection = sqlite3.connect(directory / "analysis.tsf")
    t1, t2 = 26.048285014622895, 31.906942058709145
    statements = [
        "CREATE TABLE GlobalMetadata (Key TEXT PRIMARY KEY, Value TEXT)",
        "CREATE TABLE Frames (Id INTEGER PRIMARY KEY, Time REAL NOT NULL, Polarity CHAR(1) NOT NULL, ScanMode INTEGER NOT NULL, MsMsType INTEGER NOT NULL, TimsId INTEGER, MaxIntensity INTEGER, SummedIntensities INTEGER, NumPeaks INTEGER, MzCalibration INTEGER NOT NULL, T1 REAL NOT NULL, T2 REAL NOT NULL, PropertyGroup INTEGER)",
        "CREATE TABLE MzCalibration (Id INTEGER PRIMARY KEY, ModelType INTEGER NOT NULL, DigitizerTimebase REAL NOT NULL, DigitizerDelay REAL NOT NULL, T1 REAL NOT NULL, T2 REAL NOT NULL, dC1 REAL NOT NULL, dC2 REAL NOT NULL, C0, C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14)",
        # The urine TSF run's ModelType 2 row (ProteoWizard test data).
        "INSERT INTO MzCalibration VALUES (1, 2, 0.2, 11713.6, 26.033650228332323, 31.759732732508898, 20.0, 0.0, 321.6281669619091, 154106.10635395756, 0.0010977838105375988, 321.6281669619091, 0.0010977838105375988, 111.985626, 929.834709, 7, -0.008000452158366601, 0.00011161355141355055, -5.29643113538779e-07, 1.2037715403038465e-09, -1.4397727161577292e-12, 8.502506375101492e-16, -1.8370479745732004e-19)",
        "INSERT INTO MzCalibration (Id, ModelType, DigitizerTimebase, DigitizerDelay, T1, T2, dC1, dC2, C0, C1, C2, C3, C4) VALUES (2, 1, 0.2, 25870, 25.62513732863827, 25.019305770074833, 20, 0, 317.86417140238336, 153159.143753129, 0, 0, 0)",
        "CREATE TABLE FrameMsMsInfo (Frame INTEGER PRIMARY KEY, Parent INTEGER, TriggerMass REAL NOT NULL, IsolationWidth REAL NOT NULL, PrecursorCharge INTEGER, CollisionEnergy REAL)",
        "INSERT INTO FrameMsMsInfo VALUES (2, 1, 44.99850455681751, 2.0, NULL, -20.0)",
        "INSERT INTO FrameMsMsInfo VALUES (3, 1, 500.25, 3.0, 2, 25.0)",
    ]
    for statement in statements:
        connection.execute(statement)
    frames = [
        (1, 17.360117, "-", 0, 0, offsets[0], 251, 264, 3, 1, t1, t2, 2),
        (2, 17.444204, "-", 0, 2, offsets[1], 8, 15, 2, 1, t1, t2, 2),
        (3, 17.528166, "-", 0, 2, None, 0, 0, 0, 2, 25.548975152547968, 25.0, 2),
    ]
    connection.executemany("INSERT INTO Frames VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)", frames)
    metadata = {
        "SchemaType": "TSF",
        "TimsCompressionType": "3",
        "HasLineSpectra": "1",
        "HasProfileSpectra": "1",
        "AcquisitionSoftware": "timsTOF",
        "MzAcqRangeLower": "20.000000",
        "MzAcqRangeUpper": "1300.000000",
        "DigitizerNumSamples": "402280",
    }
    connection.executemany("INSERT INTO GlobalMetadata VALUES (?, ?)", metadata.items())
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
    tables(root / "tables.d", root / "synthetic.d/analysis.tdf_bin")
    tsf(root / "tsf.d")
    (root / "checksum.zstd").write_bytes(zstandard(shuffled, checksum=True))
    print(f"Generated synthetic fixtures in {root}")


if __name__ == "__main__":
    main()
