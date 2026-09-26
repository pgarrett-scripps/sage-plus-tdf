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
}
