use crate::{Error, MetadataRow, Result, field, invalid};

/// Restricted source ModelType 1 mass calibration with per-frame temperature.
/// Only zero dC2, C2, C3 and C4 are supported. No boundary fitting is performed.
#[derive(Clone, Copy, Debug)]
pub struct MzModel {
    scale: f64,
    timebase: f64,
    offset: f64,
}
impl MzModel {
    pub fn from_metadata(row: &MetadataRow, frame_temperature: f64) -> Result<Self> {
        let number = |key: &str| {
            row.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value)
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    field(
                        invalid("missing or nonfinite m/z coefficient"),
                        "MzCalibration",
                        key,
                    )
                })
        };
        if number("ModelType")? != 1.0 {
            return Err(field(
                Error::Unsupported("m/z ModelType must be 1".into()),
                "MzCalibration",
                "ModelType",
            ));
        }
        for key in ["dC2", "C2", "C3", "C4"] {
            if number(key)? != 0.0 {
                return Err(field(
                    Error::Unsupported(format!("nonzero m/z {key}")),
                    "MzCalibration",
                    key,
                ));
            }
        }
        let timebase = number("DigitizerTimebase")?;
        let offset = number("DigitizerDelay")? - number("C0")?;
        let scale =
            number("C1")? * (1.0 + number("dC1")? * (number("T1")? - frame_temperature) / 1e6);
        number("T2")?;
        if !frame_temperature.is_finite()
            || timebase <= 0.0
            || offset <= 0.0
            || !offset.is_finite()
            || !scale.is_finite()
            || scale <= 0.0
        {
            return Err(invalid("invalid source m/z calibration"));
        }
        Ok(Self {
            scale,
            timebase,
            offset,
        })
    }

    pub fn mz(&self, tof_index: u32) -> Result<f64> {
        let time = f64::from(tof_index) * self.timebase + self.offset;
        let mz = self.scale * time * time / 1e12;
        if !mz.is_finite() || mz <= 0.0 {
            return Err(invalid("invalid calibrated m/z"));
        }
        Ok(mz)
    }
}
