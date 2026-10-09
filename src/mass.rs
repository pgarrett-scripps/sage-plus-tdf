//! Bruker `MzCalibration` ModelType 1 and the timsrust-compatible linear m/z scale.
use crate::{Error, Result, field, invalid};

/// One `MzCalibration` row, typed. Column names follow the TDF schema.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MzCalibration {
    pub id: u64,
    pub model_type: i64,
    pub digitizer_timebase: f64,
    pub digitizer_delay: f64,
    /// Calibration temperatures matched against `Frames.T1` and `Frames.T2`.
    pub t1: f64,
    pub t2: f64,
    /// Drift of `C1`, in ppm per degree of `T1` and `T2`.
    pub dc1: f64,
    pub dc2: f64,
    pub c0: f64,
    pub c1: f64,
    pub c2: f64,
    pub c3: f64,
    pub c4: f64,
}

/// The ModelType 1 mass calibration of one frame, as computed by Bruker's
/// `tims_index_to_mz` and `tims_mz_to_index`.
///
/// ```text
/// t      = DigitizerDelay + tof_index * DigitizerTimebase
/// C1'    = C1 * (1 + (dC1 * (T1 - frame T1) + dC2 * (T2 - frame T2)) / 1e6)
/// r      = C1 / C1'
/// m      = mz + C4
/// t - C0 = 1e6 * sqrt(m / C1') + r * (C2 * m + C3 * m^1.5)
/// ```
///
/// m/z to TOF is closed form. TOF to m/z solves the cubic in `sqrt(m)` by Newton
/// iteration. Both agree with libtimsdata to better than 0.001 ppm on real
/// calibrations (see `tests/fixtures/mz_calibration_sdk.json`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MzModel {
    timebase: f64,
    delay: f64,
    c0: f64,
    c4: f64,
    /// Coefficients of `sqrt(m)`, `m` and `m^1.5` in `t - C0`.
    a1: f64,
    a2: f64,
    a3: f64,
}

impl MzModel {
    /// Model for a frame recorded at temperatures `frame_t1` and `frame_t2`.
    pub fn new(calibration: &MzCalibration, frame_t1: f64, frame_t2: f64) -> Result<Self> {
        let c = calibration;
        if c.model_type != 1 {
            return Err(field(
                Error::Unsupported(format!("m/z ModelType {} (only 1 is known)", c.model_type)),
                "MzCalibration",
                "ModelType",
            ));
        }
        let values = [
            c.digitizer_timebase,
            c.digitizer_delay,
            c.t1,
            c.t2,
            c.dc1,
            c.dc2,
            c.c0,
            c.c1,
            c.c2,
            c.c3,
            c.c4,
            frame_t1,
            frame_t2,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            return Err(field(
                invalid("nonfinite m/z calibration or frame temperature"),
                "MzCalibration",
                "C0",
            ));
        }
        let c1 = c.c1 * (1.0 + (c.dc1 * (c.t1 - frame_t1) + c.dc2 * (c.t2 - frame_t2)) / 1e6);
        if c.digitizer_timebase <= 0.0 || c.c1 <= 0.0 || c1 <= 0.0 || !c1.is_finite() {
            return Err(field(
                invalid("invalid m/z calibration scale"),
                "MzCalibration",
                "C1",
            ));
        }
        let ratio = c.c1 / c1;
        Ok(Self {
            timebase: c.digitizer_timebase,
            delay: c.digitizer_delay,
            c0: c.c0,
            c4: c.c4,
            a1: 1e6 / c1.sqrt(),
            a2: c.c2 * ratio,
            a3: c.c3 * ratio,
        })
    }

    /// m/z of a (possibly fractional) TOF index.
    pub fn mz(&self, tof_index: f64) -> Result<f64> {
        let flight = tof_index * self.timebase + self.delay - self.c0;
        if !flight.is_finite() || flight <= 0.0 {
            return Err(invalid("TOF index is outside the m/z calibration domain"));
        }
        let (a1, a2, a3) = (self.a1, self.a2, self.a3);
        // Stable root of the quadratic part, then Newton steps for the m^1.5 term.
        let discriminant = a1 * a1 + 4.0 * a2 * flight;
        let mut x = if discriminant > 0.0 {
            2.0 * flight / (a1 + discriminant.sqrt())
        } else {
            flight / a1
        };
        for _ in 0..64 {
            let value = ((a3 * x + a2) * x + a1) * x - flight;
            let slope = (3.0 * a3 * x + 2.0 * a2) * x + a1;
            if slope.is_nan() || slope <= 0.0 {
                return Err(invalid(
                    "m/z calibration is not monotonic at this TOF index",
                ));
            }
            let step = value / slope;
            x -= step;
            if step.abs() <= 1e-15 * x.abs() {
                break;
            }
        }
        let mz = x * x - self.c4;
        if !mz.is_finite() || x <= 0.0 || mz <= 0.0 {
            return Err(invalid("invalid calibrated m/z"));
        }
        Ok(mz)
    }

    /// Fractional TOF index of an m/z value, the inverse of [`MzModel::mz`].
    pub fn tof_index(&self, mz: f64) -> Result<f64> {
        let m = mz + self.c4;
        if !m.is_finite() || m <= 0.0 {
            return Err(invalid("m/z is outside the calibration domain"));
        }
        let x = m.sqrt();
        let flight = ((self.a3 * x + self.a2) * x + self.a1) * x;
        let tof = (flight + self.c0 - self.delay) / self.timebase;
        if !tof.is_finite() {
            return Err(invalid("invalid calibrated TOF index"));
        }
        Ok(tof)
    }
}

/// The uncalibrated m/z scale of timsrust 0.6 (`Tof2MzConverter::from_boundaries`).
///
/// `sqrt(m/z)` is linear in the TOF index from `mz_min` at index 0 to `mz_max` at
/// `tof_max_index`. [`crate::TdfReader::linear_mz_scale`] builds it from
/// GlobalMetadata exactly as timsrust does. Kept for output compatibility only;
/// [`MzModel`] gives calibrated masses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearMzScale {
    intercept: f64,
    slope: f64,
}

impl LinearMzScale {
    pub fn new(mz_min: f64, mz_max: f64, tof_max_index: u32) -> Result<Self> {
        let intercept = mz_min.sqrt();
        let slope = (mz_max.sqrt() - intercept) / f64::from(tof_max_index);
        if !intercept.is_finite() || !slope.is_finite() || slope <= 0.0 {
            return Err(invalid("invalid m/z acquisition range"));
        }
        Ok(Self { intercept, slope })
    }

    pub fn mz(&self, tof_index: f64) -> f64 {
        let root = self.intercept + self.slope * tof_index;
        root * root
    }

    /// Fractional TOF index. timsrust truncates this to an integer.
    pub fn tof_index(&self, mz: f64) -> f64 {
        (mz.sqrt() - self.intercept) / self.slope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibration() -> MzCalibration {
        MzCalibration {
            id: 1,
            model_type: 1,
            digitizer_timebase: 0.2,
            digitizer_delay: 24864.0,
            t1: 25.5,
            t2: 26.2,
            dc1: 77.0,
            dc2: -3.7,
            c0: 313.6,
            c1: 157424.0,
            c2: 0.01,
            c3: 0.02,
            c4: 0.3,
        }
    }

    #[test]
    fn inverse_round_trips_with_all_terms() {
        let model = MzModel::new(&calibration(), 25.6, 27.6).unwrap();
        for tof in [0.0, 1.0, 1234.5, 100_000.0, 400_000.0] {
            let mz = model.mz(tof).unwrap();
            assert!((model.tof_index(mz).unwrap() - tof).abs() < 1e-6);
        }
    }

    #[test]
    fn rejects_unknown_models_and_domains() {
        let mut c = calibration();
        c.model_type = 2;
        assert_eq!(
            MzModel::new(&c, 25.0, 25.0).unwrap_err().kind(),
            crate::ErrorKind::Unsupported
        );
        let model = MzModel::new(&calibration(), 25.6, 27.6).unwrap();
        assert!(model.mz(-1e9).is_err());
        assert!(model.mz(f64::NAN).is_err());
        assert!(model.tof_index(-1.0).is_err());
        let mut c = calibration();
        c.c1 = -1.0;
        assert!(MzModel::new(&c, 25.0, 25.0).is_err());
        assert!(MzModel::new(&calibration(), f64::NAN, 25.0).is_err());
    }

    #[test]
    fn linear_scale_round_trips() {
        let scale = LinearMzScale::new(95.0, 1705.0, 397_888).unwrap();
        assert!((scale.mz(0.0) - 95.0).abs() < 1e-9);
        assert!((scale.mz(397_888.0) - 1705.0).abs() < 1e-9);
        assert!((scale.tof_index(scale.mz(200_000.0)) - 200_000.0).abs() < 1e-6);
    }
}
