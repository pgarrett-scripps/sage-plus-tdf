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
