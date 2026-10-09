//! TDF schema interpretation, separate from generic SQLite access and binary I/O.
use crate::{
    DiaFrameMsMsInfo, DiaFrameMsMsWindow, DiaFrameMsMsWindowGroup, Error, ErrorContext, FrameInfo,
    GlobalMetadata, Limits, MetadataDatabase, MetadataLimits, MetadataRow, MobilityModel,
    MzCalibration, MzModel, PasefFrameMsMsInfo, Precursor, Result, SqlValue, TimsCalibration,
    field, invalid, limit,
};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

pub(crate) struct TdfMetadata {
    pub(crate) database: MetadataDatabase,
    tims_calibrations: OnceLock<BTreeMap<u64, TimsCalibration>>,
    mz_calibrations: OnceLock<BTreeMap<u64, MzCalibration>>,
}

impl TdfMetadata {
    pub(crate) fn open(path: impl AsRef<Path>, limits: MetadataLimits) -> Result<Self> {
        Ok(Self {
            database: MetadataDatabase::with_limits(path, limits)?,
            tims_calibrations: OnceLock::new(),
            mz_calibrations: OnceLock::new(),
        })
    }

    pub(crate) fn global_metadata(&self) -> Result<GlobalMetadata> {
        let table = "GlobalMetadata";
        let entries = self
            .database
            .table(table)?
            .iter()
            .map(|row| {
                let columns = Columns { row, table };
                let key = columns
                    .text("Key")?
                    .ok_or_else(|| columns.error("Key", "missing key"))?;
                let value = row.get("Value").cloned().unwrap_or(SqlValue::Null);
                Ok((key, value))
            })
            .collect::<Result<Vec<_>>>()?;
        GlobalMetadata::new(entries)
    }

    pub(crate) fn frames(
        &self,
        limits: Limits,
        binary_length: u64,
    ) -> Result<BTreeMap<u64, FrameInfo>> {
        let mut frames = BTreeMap::new();
        for row in self.database.table("Frames")? {
            let id = row.get("Id").and_then(SqlValue::as_i64);
            let frame = parse_frame(&row)
                .and_then(|frame| {
                    validate_frame(&frame, limits, binary_length)?;
                    Ok(frame)
                })
                .map_err(|error| {
                    error.with_context(ErrorContext {
                        frame_id: id.and_then(|id| u64::try_from(id).ok()),
                        table: Some("Frames".into()),
                        ..Default::default()
                    })
                })?;
            let id = frame.id;
            if frames.insert(id, frame).is_some() {
                return Err(field(invalid("duplicate frame identifier"), "Frames", "Id")
                    .with_context(ErrorContext {
                        frame_id: Some(id),
                        ..Default::default()
                    }));
            }
        }
        Ok(frames)
    }

    /// Typed rows of an optional table. A missing table has no rows.
    fn rows<T>(
        &self,
        table: &'static str,
        parse: impl Fn(&Columns) -> Result<T>,
    ) -> Result<Vec<T>> {
        if !self.database.has_table(table)? {
            return Ok(Vec::new());
        }
        self.database
            .table(table)?
            .iter()
            .map(|row| parse(&Columns { row, table }))
            .collect()
    }

    pub(crate) fn pasef_frame_msms_info(&self) -> Result<Vec<PasefFrameMsMsInfo>> {
        self.rows("PasefFrameMsMsInfo", |c| {
            Ok(PasefFrameMsMsInfo {
                frame: c.u64("Frame")?,
                scan_num_begin: c.u32("ScanNumBegin")?,
                scan_num_end: c.u32("ScanNumEnd")?,
                isolation_mz: c.f64("IsolationMz")?,
                isolation_width: c.f64("IsolationWidth")?,
                collision_energy: c.f64("CollisionEnergy")?,
                precursor: c.opt_u64("Precursor")?,
            })
        })
    }

    pub(crate) fn precursors(&self) -> Result<Vec<Precursor>> {
        self.rows("Precursors", |c| {
            Ok(Precursor {
                id: c.u64("Id")?,
                largest_peak_mz: c.f64("LargestPeakMz")?,
                average_mz: c.f64("AverageMz")?,
                monoisotopic_mz: c.opt_f64("MonoisotopicMz")?,
                charge: c.opt_i64("Charge")?,
                scan_number: c.opt_f64("ScanNumber")?,
                intensity: c.f64("Intensity")?,
                parent: c.opt_u64("Parent")?,
            })
        })
    }

    pub(crate) fn dia_frame_msms_info(&self) -> Result<Vec<DiaFrameMsMsInfo>> {
        self.rows("DiaFrameMsMsInfo", |c| {
            Ok(DiaFrameMsMsInfo {
                frame: c.u64("Frame")?,
                window_group: c.u32("WindowGroup")?,
            })
        })
    }

    pub(crate) fn dia_frame_msms_windows(&self) -> Result<Vec<DiaFrameMsMsWindow>> {
        self.rows("DiaFrameMsMsWindows", |c| {
            Ok(DiaFrameMsMsWindow {
                window_group: c.u32("WindowGroup")?,
                scan_num_begin: c.u32("ScanNumBegin")?,
                scan_num_end: c.u32("ScanNumEnd")?,
                isolation_mz: c.f64("IsolationMz")?,
                isolation_width: c.f64("IsolationWidth")?,
                collision_energy: c.f64("CollisionEnergy")?,
            })
        })
    }

    pub(crate) fn dia_frame_msms_window_groups(&self) -> Result<Vec<DiaFrameMsMsWindowGroup>> {
        self.rows("DiaFrameMsMsWindowGroups", |c| {
            Ok(DiaFrameMsMsWindowGroup { id: c.u32("Id")? })
        })
    }

    pub(crate) fn tims_calibrations(&self) -> Result<&BTreeMap<u64, TimsCalibration>> {
        cached(&self.tims_calibrations, || {
            let rows = self.rows("TimsCalibration", tims_calibration)?;
            by_id(rows, "TimsCalibration", |row| row.id)
        })
    }

    pub(crate) fn mz_calibrations(&self) -> Result<&BTreeMap<u64, MzCalibration>> {
        cached(&self.mz_calibrations, || {
            let rows = self.rows("MzCalibration", mz_calibration)?;
            by_id(rows, "MzCalibration", |row| row.id)
        })
    }

    pub(crate) fn mobility_model(&self, frame: &FrameInfo) -> Result<MobilityModel> {
        let id = frame.mobility_calibration_id.ok_or_else(|| {
            field(
                invalid("missing calibration reference"),
                "Frames",
                "TimsCalibration",
            )
        })?;
        let row = self.tims_calibrations()?.get(&id).ok_or_else(|| {
            field(
                invalid(format!("missing calibration identifier {id}")),
                "TimsCalibration",
                "Id",
            )
        })?;
        if row.model_type != 2 {
            return Err(field(
                Error::Unsupported(format!(
                    "mobility ModelType {} (only 2 is known)",
                    row.model_type
                )),
                "TimsCalibration",
                "ModelType",
            ));
        }
        MobilityModel::new(row.coefficients)
    }

    pub(crate) fn mz_model(&self, frame: &FrameInfo) -> Result<MzModel> {
        let id = frame.mz_calibration_id.ok_or_else(|| {
            field(
                invalid("missing calibration reference"),
                "Frames",
                "MzCalibration",
            )
        })?;
        let t1 = frame
            .t1
            .ok_or_else(|| field(invalid("missing frame temperature"), "Frames", "T1"))?;
        let calibration = self.mz_calibrations()?.get(&id).ok_or_else(|| {
            field(
                invalid(format!("missing calibration identifier {id}")),
                "MzCalibration",
                "Id",
            )
        })?;
        // Without a frame T2 the dC2 term is only known when it vanishes.
        let t2 = frame
            .t2
            .or((calibration.dc2 == 0.0).then_some(calibration.t2))
            .ok_or_else(|| field(invalid("missing frame temperature"), "Frames", "T2"))?;
        MzModel::new(calibration, t1, t2)
    }
}

// Errors are returned again on every call; only a complete table is cached.
fn cached<T>(cell: &OnceLock<T>, load: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = load()?;
    Ok(cell.get_or_init(|| value))
}

fn by_id<T>(rows: Vec<T>, table: &str, id: impl Fn(&T) -> u64) -> Result<BTreeMap<u64, T>> {
    let mut map = BTreeMap::new();
    for row in rows {
        if map.insert(id(&row), row).is_some() {
            return Err(field(
                invalid("duplicate calibration identifier"),
                table,
                "Id",
            ));
        }
    }
    Ok(map)
}

pub(crate) fn tims_calibration(c: &Columns) -> Result<TimsCalibration> {
    const NAMES: [&str; 10] = ["C0", "C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9"];
    let mut coefficients = [0.0; 10];
    for (value, name) in coefficients.iter_mut().zip(NAMES) {
        *value = c.f64(name)?;
    }
    Ok(TimsCalibration {
        id: c.u64("Id")?,
        model_type: c.i64("ModelType")?,
        coefficients,
    })
}

pub(crate) fn mz_calibration(c: &Columns) -> Result<MzCalibration> {
    Ok(MzCalibration {
        id: c.u64("Id")?,
        model_type: c.i64("ModelType")?,
        digitizer_timebase: c.f64("DigitizerTimebase")?,
        digitizer_delay: c.f64("DigitizerDelay")?,
        t1: c.f64("T1")?,
        t2: c.f64("T2")?,
        dc1: c.f64("dC1")?,
        dc2: c.f64("dC2")?,
        c0: c.f64("C0")?,
        c1: c.f64("C1")?,
        c2: c.f64("C2")?,
        c3: c.f64("C3")?,
        c4: c.f64("C4")?,
    })
}

pub(crate) fn parse_frame(row: &MetadataRow) -> Result<FrameInfo> {
    let c = Columns {
        row,
        table: "Frames",
    };
    Ok(FrameInfo {
        id: c.u64("Id")?,
        retention_time_seconds: c.f64("Time")?,
        polarity: c.text("Polarity")?,
        scan_mode: c.opt_u32("ScanMode")?,
        msms_type: c.u32("MsMsType")?,
        binary_offset: c.u64("TimsId")?,
        max_intensity: c.opt_u64("MaxIntensity")?,
        summed_intensities: c.opt_u64("SummedIntensities")?,
        scan_count: c.usize("NumScans")?,
        peak_count: c.usize("NumPeaks")?,
        mz_calibration_id: c.opt_u64("MzCalibration")?,
        t1: c.opt_f64("T1")?,
        t2: c.opt_f64("T2")?,
        mobility_calibration_id: c.opt_u64("TimsCalibration")?,
        property_group: c.opt_u64("PropertyGroup")?,
        accumulation_time_ms: c.opt_f64("AccumulationTime")?,
        ramp_time_ms: c.opt_f64("RampTime")?,
    })
}

pub(crate) fn validate_frame(frame: &FrameInfo, limits: Limits, binary_length: u64) -> Result<()> {
    if frame.id == 0 {
        return Err(field(invalid("zero frame identifier"), "Frames", "Id"));
    }
    if frame.retention_time_seconds < 0.0 {
        return Err(field(invalid("invalid retention time"), "Frames", "Time"));
    }
    if frame.scan_count > limits.max_scans {
        return Err(field(
            limit("scans", limits.max_scans, frame.scan_count),
            "Frames",
            "NumScans",
        ));
    }
    if frame.peak_count > limits.max_peaks {
        return Err(field(
            limit("peaks", limits.max_peaks, frame.peak_count),
            "Frames",
            "NumPeaks",
        ));
    }
    if frame
        .binary_offset
        .checked_add(8)
        .is_none_or(|end| end > binary_length)
    {
        return Err(field(
            invalid("frame header is outside binary file"),
            "Frames",
            "TimsId",
        )
        .with_context(ErrorContext {
            byte_offset: Some(frame.binary_offset),
            ..Default::default()
        }));
    }
    Ok(())
}

/// Strict typed access to one row. Integer columns must hold SQLite integers;
/// REAL columns accept integers and must be finite. NULL and a missing column
/// are both absent.
pub(crate) struct Columns<'a> {
    pub(crate) row: &'a MetadataRow,
    pub(crate) table: &'static str,
}

impl Columns<'_> {
    fn error(&self, name: &str, message: &str) -> Error {
        field(invalid(message), self.table, name)
    }

    fn value(&self, name: &str) -> Option<&SqlValue> {
        self.row.get(name).filter(|value| !value.is_null())
    }

    fn opt_i64(&self, name: &str) -> Result<Option<i64>> {
        self.value(name)
            .map(|value| {
                value
                    .as_i64()
                    .ok_or_else(|| self.error(name, "expected an integer"))
            })
            .transpose()
    }

    fn i64(&self, name: &str) -> Result<i64> {
        self.opt_i64(name)?
            .ok_or_else(|| self.error(name, "missing value"))
    }

    fn opt_unsigned<T: TryFrom<i64>>(&self, name: &str) -> Result<Option<T>> {
        self.opt_i64(name)?
            .map(|n| T::try_from(n).map_err(|_| self.error(name, "integer out of range")))
            .transpose()
    }

    fn opt_u64(&self, name: &str) -> Result<Option<u64>> {
        self.opt_unsigned(name)
    }

    fn opt_u32(&self, name: &str) -> Result<Option<u32>> {
        self.opt_unsigned(name)
    }

    fn u64(&self, name: &str) -> Result<u64> {
        self.opt_u64(name)?
            .ok_or_else(|| self.error(name, "missing value"))
    }

    fn u32(&self, name: &str) -> Result<u32> {
        self.opt_u32(name)?
            .ok_or_else(|| self.error(name, "missing value"))
    }

    fn usize(&self, name: &str) -> Result<usize> {
        self.opt_unsigned::<usize>(name)?
            .ok_or_else(|| self.error(name, "missing value"))
    }

    fn opt_f64(&self, name: &str) -> Result<Option<f64>> {
        self.value(name)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| self.error(name, "expected a finite number"))
            })
            .transpose()
    }

    fn f64(&self, name: &str) -> Result<f64> {
        self.opt_f64(name)?
            .ok_or_else(|| self.error(name, "missing value"))
    }

    fn text(&self, name: &str) -> Result<Option<String>> {
        self.value(name)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| self.error(name, "expected text"))
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(columns: &[&str], values: Vec<SqlValue>) -> Result<MetadataRow> {
        MetadataRow::new(columns.iter().map(|c| (*c).to_owned()).collect(), values)
    }

    #[test]
    fn rows_reject_ambiguous_source_names() {
        let values = vec![SqlValue::Integer(1), SqlValue::Integer(2)];
        assert!(row(&["Id", "ID"], values).is_err());
    }

    #[test]
    fn frames_match_columns_case_insensitively_with_strict_integers() {
        use SqlValue::{Integer, Real, Text};
        let names = [
            "id", "time", "msmstype", "numscans", "numpeaks", "timsid", "Vendor",
        ];
        let mut values = vec![
            Integer(10),
            Real(1.2),
            Integer(0),
            Integer(1),
            Integer(0),
            Integer(0),
            Text("source value".into()),
        ];
        let frame = parse_frame(&row(&names, values.clone()).unwrap()).unwrap();
        assert_eq!((frame.id, frame.scan_count, frame.t2), (10, 1, None));
        values[0] = Real(10.0);
        let error = parse_frame(&row(&names, values.clone()).unwrap()).unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::CorruptData);
        assert!(error.contexts().any(|c| c.field.as_deref() == Some("Id")));
        values[0] = Integer(10);
        values[1] = Real(f64::INFINITY);
        assert!(parse_frame(&row(&names, values).unwrap()).is_err());
    }

    #[test]
    fn calibration_reals_accept_sqlite_integers() {
        use SqlValue::{Integer, Real};
        let names = [
            "Id",
            "ModelType",
            "C0",
            "C1",
            "C2",
            "C3",
            "C4",
            "C5",
            "C6",
            "C7",
            "C8",
            "C9",
        ];
        let mut values = vec![Integer(1), Integer(2), Integer(1), Integer(935)];
        values.extend((0..8).map(|i| Real(f64::from(i) + 0.5)));
        let row = row(&names, values).unwrap();
        let calibration = tims_calibration(&Columns {
            row: &row,
            table: "TimsCalibration",
        })
        .unwrap();
        assert_eq!(calibration.coefficients[1], 935.0);
    }
}
