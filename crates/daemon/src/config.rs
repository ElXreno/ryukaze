//! /etc/ryukaze/config.toml. Every key is optional: what's unset, the
//! daemon leaves alone.

use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};
use ryukaze_core::{
    fans::{Curve, Fan},
    power_supply::Source,
};
use serde::Deserialize;

use crate::{adaptive::Smoothing, boost};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Battery charge end threshold in percent, set at start and whenever
    /// msi-ec binds.
    pub charge_limit: Option<u8>,

    /// The camera on or off at start. The EC turns it back on at power-up.
    pub webcam: Option<bool>,

    /// Where Fn sits next to Win, `left` or `right`, set at start: the BIOS
    /// sets it again from its Setup at every boot.
    pub fn_key: Option<String>,

    pub profiles: Profiles,

    pub fans: Fans,

    pub cooler_boost: boost::Config,
}

/// The advanced fan mode: the curves the fans follow (a fan without one
/// keeps the EC's own) and how.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fans {
    pub cpu: Option<FanCurve>,
    pub gpu: Option<FanCurve>,

    /// The daemon sets the speed, straight between the curve's dots at the
    /// smoothed temperature; off, the EC steps through the curve itself.
    pub adaptive: bool,

    /// While adaptive: the GPU fan runs at least as fast as the CPU's curve
    /// asks, as the EC's own modes have it. It would sit still otherwise
    /// while the dGPU sleeps.
    pub gpu_follows_cpu: bool,

    pub smoothing: Smoothing,
}

impl Default for Fans {
    fn default() -> Self {
        Self {
            cpu: None,
            gpu: None,
            adaptive: true,
            gpu_follows_cpu: true,
            smoothing: Smoothing::default(),
        }
    }
}

impl Fans {
    pub fn get(&self, fan: Fan) -> Option<Curve> {
        match fan {
            Fan::Cpu => self.cpu,
            Fan::Gpu => self.gpu,
        }
        .map(|FanCurve(curve)| curve)
    }

    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.gpu.is_none()
    }
}

/// A [`Curve`] checked as it's parsed, so a bad one fails at start.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "RawCurve")]
pub struct FanCurve(pub Curve);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCurve {
    temps: Vec<u8>,
    speeds: Vec<u8>,
}

impl TryFrom<RawCurve> for FanCurve {
    type Error = String;

    fn try_from(raw: RawCurve) -> Result<Self, String> {
        Curve::new(&raw.temps, &raw.speeds).map(FanCurve)
    }
}

/// What to switch the EC to when the laptop goes on AC or on battery.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profiles {
    pub ac: Option<Profile>,
    pub battery: Option<Profile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub shift_mode: Option<String>,
    pub fan_mode: Option<String>,
    pub super_battery: Option<bool>,
}

impl Config {
    /// A missing file is an empty config. Mode names can only be checked
    /// against the EC, so they're checked when applied.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        let config: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if let Some(percent) = config.charge_limit
            && !(10..=100).contains(&percent)
        {
            bail!(
                "{}: charge_limit {percent} is outside 10..100",
                path.display()
            );
        }
        if let Some(side) = &config.fn_key
            && !matches!(side.as_str(), "left" | "right")
        {
            bail!(
                "{}: fn_key {side:?} is neither left nor right",
                path.display()
            );
        }
        Ok(config)
    }
}

impl Profiles {
    pub fn get(&self, source: Source) -> Option<&Profile> {
        match source {
            Source::Ac => self.ac.as_ref(),
            Source::Battery => self.battery.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, text).unwrap();
        Config::load(&path)
    }

    #[test]
    fn reads_profiles() {
        let config = load(
            r#"
            charge_limit = 80

            [profiles.ac]
            shift_mode = "turbo"

            [profiles.battery]
            shift_mode = "eco"
            super_battery = true
            "#,
        )
        .unwrap();
        assert_eq!(config.charge_limit, Some(80));
        let ac = config.profiles.get(Source::Ac).unwrap();
        assert_eq!(ac.shift_mode.as_deref(), Some("turbo"));
        assert_eq!(ac.fan_mode, None);
        let battery = config.profiles.get(Source::Battery).unwrap();
        assert_eq!(battery.super_battery, Some(true));
    }

    #[test]
    fn missing_file_is_empty() {
        let config = Config::load(Path::new("/nonexistent/ryukaze.toml")).unwrap();
        assert_eq!(config.charge_limit, None);
        assert!(config.profiles.get(Source::Ac).is_none());
    }

    #[test]
    fn rejects_typos_and_bad_limits() {
        assert!(load("[profiles.ac]\nshift = \"turbo\"\n").is_err());
        assert!(load("charge_limit = 5\n").is_err());
        assert!(load("fn_key = \"middle\"\n").is_err());
        assert!(load("fn_key = \"left\"\nwebcam = false\n").is_ok());
    }

    #[test]
    fn reads_fan_curves() {
        let config = load(
            r#"
            [fans.cpu]
            temps = [55, 62, 69, 76, 83, 90]
            speeds = [0, 30, 35, 40, 50, 60, 100]
            "#,
        )
        .unwrap();
        let cpu = config.fans.get(Fan::Cpu).unwrap();
        assert_eq!(cpu.speeds[6], 100);
        assert!(config.fans.get(Fan::Gpu).is_none());
        // Adaptive, with the GPU fan helping, unless told otherwise.
        assert!(config.fans.adaptive && config.fans.gpu_follows_cpu);
    }

    #[test]
    fn the_readme_example_parses() {
        let readme = include_str!("../../../readme.md");
        let (_, rest) = readme
            .split_once("```toml\n")
            .expect("a toml block in the readme");
        let (example, _) = rest.split_once("```").unwrap();
        let config = load(example).unwrap();
        assert_eq!(config.charge_limit, Some(80));
        assert_eq!(config.fn_key.as_deref(), Some("left"));
        assert!(config.profiles.get(Source::Battery).is_some());
        assert!(config.fans.get(Fan::Cpu).is_some());
    }

    #[test]
    fn reads_fan_options() {
        let config = load(
            r#"
            [fans]
            adaptive = false
            gpu_follows_cpu = false

            [fans.smoothing]
            fall_secs = 30
            "#,
        )
        .unwrap();
        assert!(!config.fans.adaptive && !config.fans.gpu_follows_cpu);
        assert_eq!(config.fans.smoothing.fall_secs, 30.0);
        assert_eq!(config.fans.smoothing.rise_secs, 10.0);
    }

    #[test]
    fn rejects_bad_curves_at_load() {
        // Slower when hotter.
        let err = load(
            "[fans.gpu]\ntemps = [55, 60, 65, 70, 75, 80]\nspeeds = [0, 48, 55, 30, 68, 77, 85]\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("must not fall"), "{err:#}");
        assert!(load("[fans.cpu]\ntemps = [50]\nspeeds = [0]\n").is_err());
    }
}
