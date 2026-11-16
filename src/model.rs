use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Input,
    Output,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub is_default: bool,
    /// Linear volume multiplier (1.0 = 100%), when the backend supports it.
    pub volume: Option<f32>,
    pub muted: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub backend: String,
    pub devices: Vec<Device>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub input: String,
    pub output: Option<String>,
    pub denoise: bool,
    pub boost_db: f32,
    pub monitor: bool,
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.input.is_empty(), "Choose a microphone first.");
        ensure!(
            self.boost_db.is_finite() && (-36.0..=18.0).contains(&self.boost_db),
            "Microphone boost must be between -36 and 18 dB."
        );
        ensure!(
            !self.monitor || self.output.as_ref().is_some_and(|s| !s.is_empty()),
            "Choose headphones before enabling monitoring."
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Telemetry {
    pub input_peak: f32,
    pub output_peak: f32,
    pub clipped_samples: u64,
    pub error: Option<String>,
    pub running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_or_incomplete_settings() {
        let mut settings = Settings {
            input: "mic".into(),
            output: None,
            denoise: true,
            boost_db: 0.0,
            monitor: false,
        };
        assert!(settings.validate().is_ok());
        settings.boost_db = f32::NAN;
        assert!(settings.validate().is_err());
        settings.boost_db = 19.0;
        assert!(settings.validate().is_err());
        settings.boost_db = 0.0;
        settings.monitor = true;
        assert!(settings.validate().is_err());
    }
}
