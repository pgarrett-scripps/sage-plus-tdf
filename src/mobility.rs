use crate::{Result, invalid};

/// Source ModelType 2 mobility calibration, with the SDK boundary continuation.
#[derive(Clone, Copy, Debug)]
pub struct MobilityModel {
    coefficients: [f64; 10],
}
impl MobilityModel {
    pub fn new(c: [f64; 10]) -> Result<Self> {
        if c.iter().any(|x| !x.is_finite())
            || c[1] <= 0.0
            || c[5] != 1.0
            || c[8] > c[9]
            || c[6] * c[8] + c[7] <= 0.0
            || c[6] * c[9] + c[7] <= 0.0
        {
            return Err(invalid("invalid or unsupported mobility coefficients"));
        }
        Ok(Self { coefficients: c })
    }
    pub fn one_over_k0(&self, scan: f64) -> Result<f64> {
        if !scan.is_finite() {
            return Err(invalid("nonfinite scan coordinate"));
        }
        let [c0, c1, c2, c3, c4, _, c6, c7, c8, c9] = self.coefficients;
        let v = c2 + (c2 - c3) / c1 * (c4 + c0 - scan);
        let bounded = v.clamp(c8, c9);
        let denominator = c6 * bounded + c7;
        let result = bounded / denominator + c7 / denominator.powi(2) * (v - bounded);
        if !result.is_finite() {
            return Err(invalid("nonfinite calibrated mobility"));
        }
        Ok(result)
    }

    /// Fractional scan number of an inverse reduced mobility, the inverse of
    /// [`MobilityModel::one_over_k0`] including its linear continuation past C8 and C9.
    pub fn scan_number(&self, one_over_k0: f64) -> Result<f64> {
        let [c0, c1, c2, c3, c4, _, c6, c7, c8, c9] = self.coefficients;
        if !one_over_k0.is_finite() || c2 == c3 || c7 == 0.0 {
            return Err(invalid(
                "inverse mobility is outside the calibration domain",
            ));
        }
        let curve = |v: f64| v / (c6 * v + c7);
        let slope = |v: f64| c7 / (c6 * v + c7).powi(2);
        let v = if one_over_k0 < curve(c8) {
            c8 + (one_over_k0 - curve(c8)) / slope(c8)
        } else if one_over_k0 > curve(c9) {
            c9 + (one_over_k0 - curve(c9)) / slope(c9)
        } else {
            c7 * one_over_k0 / (1.0 - c6 * one_over_k0)
        };
        let scan = c4 + c0 - (v - c2) * c1 / (c2 - c3);
        if !scan.is_finite() {
            return Err(invalid("nonfinite scan coordinate"));
        }
        Ok(scan)
    }
}

/// The uncalibrated mobility scale of timsrust 0.6 (`Scan2ImConverter::from_boundaries`):
/// linear from `upper` at scan 0 to `lower` at `scan_max_index`.
/// [`crate::TdfReader::linear_mobility_scale`] builds it from GlobalMetadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearMobilityScale {
    intercept: f64,
    slope: f64,
}

impl LinearMobilityScale {
    pub fn new(lower: f64, upper: f64, scan_max_index: u32) -> Result<Self> {
        let slope = (lower - upper) / f64::from(scan_max_index);
        if !upper.is_finite() || !slope.is_finite() || scan_max_index == 0 {
            return Err(invalid("invalid mobility acquisition range"));
        }
        Ok(Self {
            intercept: upper,
            slope,
        })
    }

    pub fn one_over_k0(&self, scan: f64) -> f64 {
        self.intercept + self.slope * scan
    }

    pub fn scan_number(&self, one_over_k0: f64) -> f64 {
        (one_over_k0 - self.intercept) / self.slope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ModelType 2 row of `example_dda.d`, as recorded in
    /// `tests/fixtures/tims_calibration_sdk.json` with the SDK's own values.
    fn coefficients() -> [f64; 10] {
        [
            1.0,
            670.0,
            207.775_676_931_964_3,
            59.252_667_636_882_194,
            33.0,
            1.0,
            0.020_988_900_147_314_915,
            131.440_113_097_797_85,
            12.971_231_729_588_75,
            2558.716_925_059_314_2,
        ]
    }

    #[test]
    fn inverse_round_trips_inside_the_window_and_both_continuations() {
        let model = MobilityModel::new(coefficients()).unwrap();
        // v leaves [c8, c9] below about scan 913 and above about scan -10571,
        // so these sweeps cover the curve and both linear continuations.
        for scan in [
            -20_000.0, -10_571.0, -1_000.0, 0.0, 1.0, 455.5, 912.0, 913.0, 2_000.0, 1e6,
        ] {
            let one_over_k0 = model.one_over_k0(scan).unwrap();
            let round_trip = model.scan_number(one_over_k0).unwrap();
            assert!(
                (round_trip - scan).abs() < 1e-9 * scan.abs().max(1.0),
                "scan {scan} round tripped to {round_trip} through {one_over_k0}"
            );
        }
    }

    #[test]
    fn one_over_k0_decreases_strictly_across_both_continuations() {
        let model = MobilityModel::new(coefficients()).unwrap();
        let mut previous = f64::INFINITY;
        for scan in (-20_000..20_000).step_by(100) {
            let value = model.one_over_k0(f64::from(scan)).unwrap();
            assert!(value < previous, "mobility rose at scan {scan}");
            previous = value;
        }
    }

    #[test]
    fn rejects_nonfinite_inputs_and_a_degenerate_scan_scale() {
        let model = MobilityModel::new(coefficients()).unwrap();
        for scan in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                model.one_over_k0(scan).unwrap_err().kind(),
                crate::ErrorKind::CorruptData
            );
        }
        for value in [f64::NAN, f64::INFINITY] {
            assert_eq!(
                model.scan_number(value).unwrap_err().kind(),
                crate::ErrorKind::CorruptData
            );
        }
        // C2 == C3 leaves the scan scale with no slope: one_over_k0 is a
        // constant, so no inverse exists.
        let mut c = coefficients();
        c[3] = c[2];
        let flat = MobilityModel::new(c).unwrap();
        assert!(flat.one_over_k0(100.0).is_ok());
        assert_eq!(
            flat.scan_number(0.5).unwrap_err().kind(),
            crate::ErrorKind::CorruptData
        );
    }

    #[test]
    fn rejects_unusable_coefficients() {
        for (index, value) in [(1, 0.0), (1, -670.0), (5, 2.0), (0, f64::NAN)] {
            let mut c = coefficients();
            c[index] = value;
            assert!(MobilityModel::new(c).is_err());
        }
        // C8 above C9, and a window the denominator vanishes on.
        let mut c = coefficients();
        c.swap(8, 9);
        assert!(MobilityModel::new(c).is_err());
        let mut c = coefficients();
        c[7] = -1.0;
        c[6] = 0.0;
        assert!(MobilityModel::new(c).is_err());
    }

    #[test]
    fn linear_scale_round_trips() {
        let scale = LinearMobilityScale::new(0.6, 1.6, 927).unwrap();
        assert!((scale.one_over_k0(0.0) - 1.6).abs() < 1e-12);
        assert!((scale.one_over_k0(927.0) - 0.6).abs() < 1e-12);
        assert!((scale.scan_number(scale.one_over_k0(455.0)) - 455.0).abs() < 1e-9);
        assert!(LinearMobilityScale::new(0.6, 1.6, 0).is_err());
    }
}
