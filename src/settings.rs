//! Versioned, validated configuration. No enabled/capture state is persisted.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const FREQUENCIES: [f32; 5] = [80.0, 250.0, 1000.0, 4000.0, 12000.0];
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Processing {
    pub gain_db: f32,
    pub eq_db: [f32; 5],
    pub bass_db: f32,
    pub clarity_db: f32,
    pub width: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub low_cut_hz: f32,
    pub denoise: bool,
}
impl Default for Processing {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            eq_db: [0.0; 5],
            bass_db: 0.0,
            clarity_db: 0.0,
            width: 1.0,
            threshold_db: -18.0,
            ratio: 1.0,
            low_cut_hz: 0.0,
            denoise: false,
        }
    }
}
impl Processing {
    pub fn validate(&self) -> Result<()> {
        for (v, min, max, name) in [
            (self.gain_db, -36.0, 18.0, "gain"),
            (self.bass_db, 0.0, 9.0, "bass"),
            (self.clarity_db, 0.0, 6.0, "clarity"),
            (self.width, 0.0, 2.0, "width"),
            (self.threshold_db, -48.0, 0.0, "threshold"),
            (self.ratio, 1.0, 10.0, "ratio"),
            (self.low_cut_hz, 0.0, 300.0, "low cut"),
        ] {
            ensure!(
                v.is_finite() && (min..=max).contains(&v),
                "Invalid {name}: expected {min} to {max}"
            );
        }
        ensure!(
            self.eq_db
                .iter()
                .all(|x| x.is_finite() && (-12.0..=12.0).contains(x)),
            "EQ gains must be finite and between -12 and 12 dB"
        );
        Ok(())
    }
    pub fn preset(name: &str, input: bool) -> Self {
        let mut p = Self::default();
        if input {
            p.denoise = true;
            p.low_cut_hz = 70.0;
        }
        match name {
            "Music" => {
                p.eq_db = [2.0, 0.0, -1.0, 1.0, 1.0];
                p.bass_db = 2.0;
            }
            "Movies" => {
                p.eq_db = [2.0, 0.0, 1.0, 2.0, 0.0];
                p.ratio = 2.5;
                p.width = 1.15;
            }
            "Voice" | "Clear Speech" => {
                p.eq_db = [-3.0, -1.0, 1.0, 3.0, -1.0];
                p.ratio = 2.5;
            }
            "Warm Voice" => {
                p.eq_db = [1.0, 2.0, 0.0, 1.0, -2.0];
                p.ratio = 2.0;
            }
            "Noisy Room" => {
                p.low_cut_hz = 120.0;
                p.eq_db = [-4.0, -2.0, 1.0, 2.0, -2.0];
                p.ratio = 2.0;
            }
            _ => {}
        }
        p
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Channel {
    pub device: String,
    pub preset: String,
    pub processing: Processing,
}
impl Default for Channel {
    fn default() -> Self {
        Self {
            device: String::new(),
            preset: "Neutral".into(),
            processing: Processing::default(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedPreset {
    pub name: String,
    pub input: bool,
    pub processing: Processing,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub input: Channel,
    pub output: Channel,
    pub monitor: bool,
    pub headphones: String,
    pub background: bool,
    pub presets: Vec<SavedPreset>,
    /// Optional theme identifier. `None` means first-launch auto-detect.
    /// `"follow-omarchy"` follows the active Omarchy palette. Any other
    /// value names a bundled palette. Added in version 1; older files
    /// that omit this field load cleanly with `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            input: Channel {
                preset: "Natural".into(),
                processing: Processing::preset("Natural", true),
                ..Channel::default()
            },
            output: Channel::default(),
            monitor: false,
            headphones: String::new(),
            background: false,
            presets: Vec::new(),
            theme: None,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported settings version {}",
            self.version
        );
        ensure!(
            self.theme
                .as_ref()
                .is_none_or(|s| s.len() <= 128 && !s.contains('\0')),
            "Invalid theme identifier"
        );
        self.input.processing.validate()?;
        self.output.processing.validate()?;
        ensure!(
            self.presets.len() <= 32,
            "At most 32 custom presets supported"
        );
        for s in [
            &self.input.device,
            &self.output.device,
            &self.headphones,
            &self.input.preset,
            &self.output.preset,
        ] {
            ensure!(
                s.len() <= 512 && !s.contains('\0'),
                "Invalid configuration string"
            );
        }
        for p in &self.presets {
            ensure!(
                !p.name.trim().is_empty() && p.name.len() <= 64,
                "Preset name must have 1–64 bytes"
            );
            p.processing.validate()?;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        ensure!(
            fs::metadata(path)?.len() <= 65536,
            "Settings file is too large"
        );
        let bytes = fs::read(path).context("Cannot read settings")?;
        ensure!(bytes.len() <= 65536, "Settings file is too large");
        let c: Self = serde_json::from_slice(&bytes)
            .context("Settings are malformed; repair or move the settings file to reset")?;
        c.validate()?;
        Ok(c)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().context("Invalid settings path")?;
        fs::create_dir_all(parent)?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self)?)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}
pub fn config_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .context("HOME or absolute XDG_CONFIG_HOME is required")?;
    Ok(base.join("clearwave/settings.json"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_and_presets() {
        for name in [
            "Neutral",
            "Music",
            "Movies",
            "Voice",
            "Natural",
            "Clear Speech",
            "Warm Voice",
            "Noisy Room",
        ] {
            Processing::preset(name, true).validate().unwrap();
        }
        let mut p = Processing {
            gain_db: f32::NAN,
            ..Default::default()
        };
        assert!(p.validate().is_err());
        p.gain_db = -36.0;
        assert!(p.validate().is_ok());
    }
    #[test]
    fn persistence_and_malformed() {
        let path = std::env::temp_dir().join(format!(
            "clearwave-settings-test-{}.json",
            std::process::id()
        ));
        let c = Config::default();
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        fs::write(&path, b"{bad}").unwrap();
        assert!(Config::load(&path).is_err());
        fs::write(&path, b"{\"version\":99}").unwrap();
        assert!(Config::load(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;
    #[test]
    fn old_settings_and_theme_validation() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(old.theme, None);
        let mut c = old;
        c.theme = Some("catppuccin-latte".into());
        let copy: Config = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert_eq!(c, copy);
        c.theme = Some("x".repeat(129));
        assert!(c.validate().is_err());
    }
}
