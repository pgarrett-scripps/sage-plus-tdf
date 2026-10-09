//! Bruker `MzCalibration` ModelTypes 1 and 2 and the timsrust-compatible linear m/z scale.
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
    /// `C5` to `C14`, present in TSF schemas. ModelType 2 reads them; a missing
    /// column is `None`.
    pub c5_to_c14: [Option<f64>; 10],
}

/// The mass calibration of one frame, as computed by Bruker's
/// `tims_index_to_mz` / `tsf_index_to_mz` and their inverses.
///
/// ModelType 1 (TDF and TSF):
///
/// ```text
/// t      = DigitizerDelay + tof_index * DigitizerTimebase
/// C1'    = C1 * (1 + (dC1 * (T1 - frame T1) + dC2 * (T2 - frame T2)) / 1e6)
/// r      = C1 / C1'
/// m      = mz + C4
/// t - C0 = 1e6 * sqrt(m / C1') + r * (C2 * m + C3 * m^1.5)
/// ```
///
/// ModelType 2 (seen in TSF) has the same core without the `C3` and `C4` terms
/// (those columns repeat `C0` and `C2`), giving an uncorrected `m`. A
/// polynomial `P(x) = sum C(8+k) x^k`, `k < C7`, is then subtracted, fitted on
/// `[C5, C6]` and fading as a Gaussian of unit width (in Th) outside it:
///
/// ```text
/// x  = clamp(m, C5, C6)
/// mz = m - P(x) * exp(-(m - x)^2)
/// ```
///
/// ModelType 2 is reconstructed from SDK output, not a specification: it
/// matches Bruker's SDK to 0.0001 ppm on every centroid of a timsTOF fleX
/// TSF run (`timsTOF_autoMSMS_Urine_50s_neg.d`, 70,265 peaks). Rows where
/// `C3`/`C4` differ from `C0`/`C2` are rejected as unverified.
///
/// m/z to TOF is closed form for ModelType 1. TOF to m/z solves the cubic in
/// `sqrt(m)` by Newton iteration. ModelType 1 agrees with libtimsdata to better
/// than 0.001 ppm on real calibrations (see `tests/fixtures/mz_calibration_sdk.json`).
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
    correction: Option<Correction>,
}

/// The ModelType 2 polynomial correction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Correction {
    lower: f64,
    upper: f64,
    terms: usize,
    coefficients: [f64; 7],
}

impl Correction {
    fn new(c: &MzCalibration) -> Result<Self> {
        let column = |i: usize| {
            c.c5_to_c14[i - 5].ok_or_else(|| {
                field(
                    invalid("missing ModelType 2 coefficient"),
                    "MzCalibration",
                    &format!("C{i}"),
                )
            })
        };
        let (lower, upper, terms) = (column(5)?, column(6)?, column(7)?);
        if !(lower.is_finite() && upper.is_finite() && lower < upper) {
            return Err(field(
                invalid("invalid ModelType 2 correction range"),
                "MzCalibration",
                "C5",
            ));
        }
        if terms.fract() != 0.0 || !(0.0..=7.0).contains(&terms) {
            return Err(field(
                invalid("ModelType 2 term count must be an integer from 0 to 7"),
                "MzCalibration",
                "C7",
            ));
        }
        let terms = terms as usize;
        let mut coefficients = [0.0; 7];
        for (k, value) in coefficients.iter_mut().enumerate().take(terms) {
            *value = column(8 + k)?;
            if !value.is_finite() {
                return Err(field(
                    invalid("nonfinite ModelType 2 coefficient"),
                    "MzCalibration",
                    &format!("C{}", 8 + k),
                ));
            }
        }
        Ok(Self {
            lower,
            upper,
            terms,
            coefficients,
        })
    }

    /// Amount subtracted from the uncorrected m/z `m`.
    fn offset(&self, m: f64) -> f64 {
        let x = m.clamp(self.lower, self.upper);
        let p = self.coefficients[..self.terms]
            .iter()
            .rev()
            .fold(0.0, |sum, c| sum * x + c);
        p * (-(m - x) * (m - x)).exp()
    }
}

impl MzModel {
    /// Model for a frame recorded at temperatures `frame_t1` and `frame_t2`.
    pub fn new(calibration: &MzCalibration, frame_t1: f64, frame_t2: f64) -> Result<Self> {
        let c = calibration;
        let correction = match c.model_type {
            1 => None,
            2 => {
                if c.c3 != c.c0 || c.c4 != c.c2 {
                    return Err(field(
                        Error::Unsupported(
                            "m/z ModelType 2 with C3/C4 differing from C0/C2".into(),
                        ),
                        "MzCalibration",
                        "C3",
                    ));
                }
                Some(Correction::new(c)?)
            }
            other => {
                return Err(field(
                    Error::Unsupported(format!("m/z ModelType {other} (only 1 and 2 are known)")),
                    "MzCalibration",
                    "ModelType",
                ));
            }
        };
        let (c3, c4) = if correction.is_some() {
            (0.0, 0.0)
        } else {
            (c.c3, c.c4)
        };
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
            c3,
            c4,
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
            c4,
            a1: 1e6 / c1.sqrt(),
            a2: c.c2 * ratio,
            a3: c3 * ratio,
            correction,
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
        let mut mz = x * x - self.c4;
        if let Some(correction) = &self.correction {
            mz -= correction.offset(mz);
        }
        if !mz.is_finite() || x <= 0.0 || mz <= 0.0 {
            return Err(invalid("invalid calibrated m/z"));
        }
        Ok(mz)
    }

    /// Fractional TOF index of an m/z value, the inverse of [`MzModel::mz`].
    pub fn tof_index(&self, mz: f64) -> Result<f64> {
        let mut m = mz + self.c4;
        if let Some(correction) = &self.correction {
            // Fixed point of m = mz + offset(m); the offset varies by ppm per Th.
            for _ in 0..64 {
                let next = mz + correction.offset(m);
                if !next.is_finite() {
                    return Err(invalid("m/z is outside the calibration domain"));
                }
                let step = next - m;
                m = next;
                if step.abs() <= 1e-15 * m.abs() {
                    break;
                }
            }
        }
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
            c5_to_c14: [None; 10],
        }
    }

    const SDK_POINTS: [(f64, f64); 8] = [
        (221.5, 20.155193851563446),
        (77259.97916666667, 111.04787914506481),
        (78425.64772727272, 112.98545043063426),
        (227807.9090909091, 499.86573762004514),
        (331255.0294117647, 928.991931919409),
        (331486.875, 930.1019414903158),
        (331749.01666666666, 931.3590318575407),
        (400391.14, 1289.3342981641363),
    ];

    /// The urine TSF run's ModelType 2 row.
    fn model_type_2() -> MzCalibration {
        MzCalibration {
            id: 1,
            model_type: 2,
            digitizer_timebase: 0.2,
            digitizer_delay: 11713.6,
            t1: 26.033650228332323,
            t2: 31.759732732508898,
            dc1: 20.0,
            dc2: 0.0,
            c0: 321.6281669619091,
            c1: 154106.10635395756,
            c2: 0.0010977838105375988,
            c3: 321.6281669619091,
            c4: 0.0010977838105375988,
            c5_to_c14: [
                Some(111.985626),
                Some(929.834709),
                Some(7.0),
                Some(-0.008000452158366601),
                Some(0.00011161355141355055),
                Some(-5.29643113538779e-07),
                Some(1.2037715403038465e-09),
                Some(-1.4397727161577292e-12),
                Some(8.502506375101492e-16),
                Some(-1.8370479745732004e-19),
            ],
        }
    }

    #[test]
    fn model_type_2_matches_sdk_centroids() {
        // Frame 1 of timsTOF_autoMSMS_Urine_50s_neg.d: line index, SDK m/z
        // (ProteoWizard Reader_Bruker_Test.data mzML), below, inside and above [C5, C6].
        let model = MzModel::new(&model_type_2(), 26.048285014622895, 31.906942058709145).unwrap();
        for (index, sdk) in SDK_POINTS {
            let mz = model.mz(index).unwrap();
            assert!(((mz - sdk) / sdk).abs() < 1e-9, "{index}: {mz} vs {sdk}");
            assert!((model.tof_index(mz).unwrap() - index).abs() < 1e-6);
        }
    }

    #[test]
    fn model_type_2_rejects_unverified_rows() {
        let mut c = model_type_2();
        c.c3 = 0.0;
        assert_eq!(
            MzModel::new(&c, 26.0, 31.0).unwrap_err().kind(),
            crate::ErrorKind::Unsupported
        );
        let mut c = model_type_2();
        c.c5_to_c14[2] = Some(8.0);
        assert!(MzModel::new(&c, 26.0, 31.0).is_err());
        let mut c = model_type_2();
        c.c5_to_c14[9] = None;
        assert!(MzModel::new(&c, 26.0, 31.0).is_err());
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
        c.model_type = 3;
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
