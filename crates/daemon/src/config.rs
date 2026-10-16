//! /etc/ryukaze/config.toml. Every key is optional: what's unset, the
//! daemon leaves alone.

use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};
use ryukaze_core::power_supply::Source;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Battery charge end threshold in percent, set at start and whenever
    /// msi-ec binds.
    pub charge_limit: Option<u8>,

    pub profiles: Profiles,
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
    }
}
