//! /etc/ryukaze/config.toml. Every key is optional: what's unset, the
//! daemon leaves alone.

use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result, bail};
use ryukaze_core::{
    COOLER_BOOST_MARKS,
    fans::{Curve, Fan},
    keyboard::{self, Color},
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

    /// USB ports powered while the laptop is off, set at start. The EC
    /// keeps it.
    pub usb_share: Option<bool>,

    pub profiles: Profiles,

    /// Sets of modes to step through in this order (`NextScenario`, which
    /// MSI's Fn+F7 is for). None: silent, balanced and performance.
    pub scenarios: Vec<Scenario>,

    pub fans: Fans,

    pub cooler_boost: boost::Config,

    pub power_profiles_daemon: PowerProfilesDaemon,

    pub keyboard: Keyboard,
}

/// The keyboard's backlight, set at start, after sleep and whenever the
/// controller comes back.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    /// Every key alike: `#rrggbb`, or `off`.
    pub color: Option<KeyboardColor>,

    /// Keys and groups of keys in their own, by name: a single key over a
    /// group, a smaller group over a larger one.
    pub keys: BTreeMap<String, KeyboardColor>,

    /// The colour for all while the EC is in a scenario's modes, by the
    /// scenario's name, over `color`.
    pub scenarios: BTreeMap<String, KeyboardColor>,
}

impl Keyboard {
    pub fn scenario_colors(&self) -> Vec<(String, Color)> {
        self.scenarios
            .iter()
            .map(|(name, &KeyboardColor(color))| (name.clone(), color))
            .collect()
    }

    /// The keys with their own colours; their names were checked at load.
    pub fn own(&self) -> Vec<(Vec<u8>, Color)> {
        self.keys
            .iter()
            .filter_map(|(name, &KeyboardColor(color))| Some((keyboard::named(name)?, color)))
            .collect()
    }
}

/// A [`Color`] checked as it's parsed.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "String")]
pub struct KeyboardColor(pub Color);

impl TryFrom<String> for KeyboardColor {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse().map(KeyboardColor)
    }
}

/// power-profiles-daemon's profile and the shift mode kept in step; see
/// `power_profiles`.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerProfilesDaemon {
    pub sync: bool,
}

impl Default for PowerProfilesDaemon {
    fn default() -> Self {
        Self { sync: true }
    }
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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    /// A scenario by name: its modes, under any set here.
    pub scenario: Option<String>,
    pub shift_mode: Option<String>,
    pub fan_mode: Option<String>,
    pub super_battery: Option<bool>,
}

/// A [`Profile`] with a name, written out in full in the config: serde
/// can't flatten one into a struct that denies unknown fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub shift_mode: Option<String>,
    pub fan_mode: Option<String>,
    pub super_battery: Option<bool>,
}

/// MSI Center's scenarios, for an EC without configured ones: (name, shift
/// mode, fan mode), super battery off. Those whose modes the EC lacks are
/// left out when the daemon starts.
pub const BUILT_IN_SCENARIOS: [(&str, &str, &str); 3] = [
    ("silent", "eco", "silent"),
    ("balanced", "comfort", "auto"),
    ("performance", "turbo", "auto"),
];

impl Scenario {
    pub fn modes(&self) -> Profile {
        Profile {
            scenario: None,
            shift_mode: self.shift_mode.clone(),
            fan_mode: self.fan_mode.clone(),
            super_battery: self.super_battery,
        }
    }
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
        for (i, scenario) in config.scenarios.iter().enumerate() {
            let name = &scenario.name;
            if name.trim().is_empty() {
                bail!("{}: scenario {} has no name", path.display(), i + 1);
            }
            if config.scenarios[..i]
                .iter()
                .any(|other| &other.name == name)
            {
                bail!("{}: two scenarios are called {name:?}", path.display());
            }
            if scenario.modes() == Profile::default() {
                bail!("{}: scenario {name:?} sets nothing", path.display());
            }
        }
        let names: Vec<&str> = if config.scenarios.is_empty() {
            BUILT_IN_SCENARIOS.iter().map(|(name, ..)| *name).collect()
        } else {
            config.scenarios.iter().map(|s| s.name.as_str()).collect()
        };
        for (source, profile) in [
            ("ac", &config.profiles.ac),
            ("battery", &config.profiles.battery),
        ] {
            if let Some(name) = profile.as_ref().and_then(|p| p.scenario.as_deref())
                && !names.contains(&name)
            {
                bail!(
                    "{}: profiles.{source}.scenario {name:?} is not one of: {}",
                    path.display(),
                    names.join(", ")
                );
            }
        }
        for name in config.keyboard.scenarios.keys() {
            if !names.contains(&name.as_str()) {
                bail!(
                    "{}: keyboard.scenarios.{name} is not one of: {}",
                    path.display(),
                    names.join(", ")
                );
            }
        }
        for name in config.keyboard.keys.keys() {
            if keyboard::named(name).is_none() {
                bail!(
                    "{}: keyboard.keys: no key or group is called {name:?}",
                    path.display()
                );
            }
        }
        let boost = &config.cooler_boost;
        for (key, mark) in [("cpu_c", boost.cpu_c), ("gpu_c", boost.gpu_c)] {
            if !COOLER_BOOST_MARKS.contains(&mark) {
                bail!(
                    "{}: cooler_boost.{key} {mark} is outside {}..{}",
                    path.display(),
                    COOLER_BOOST_MARKS.start(),
                    COOLER_BOOST_MARKS.end()
                );
            }
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
        assert!(load("[cooler_boost]\ncpu_c = 40\n").is_err());
        assert!(load("[cooler_boost]\ngpu_c = 80\n").is_ok());
        assert!(load("").unwrap().power_profiles_daemon.sync);
        assert!(load("[keyboard]\ncolor = \"peach\"\n").is_err());
        let off = load("[keyboard]\ncolor = \"off\"\n").unwrap();
        assert_eq!(
            off.keyboard.color.map(|KeyboardColor(color)| color),
            Some(Color::OFF)
        );
        assert!(load("[keyboard.keys]\nhome = \"off\"\n").is_err());
        // Scenarios there are: the built-in ones, without any configured.
        let silent = load("[keyboard.scenarios]\nsilent = \"#0040ff\"\n").unwrap();
        assert_eq!(
            silent.keyboard.scenario_colors(),
            [("silent".to_owned(), Color([0, 0x40, 0xff]))]
        );
        assert!(load("[keyboard.scenarios]\nloud = \"#ff0000\"\n").is_err());
        let wasd = load("[keyboard.keys]\nwasd = \"#ff0000\"\nf7 = \"off\"\n").unwrap();
        assert_eq!(
            wasd.keyboard.own(),
            [
                (vec![0x40], Color::OFF),
                (vec![0x1a, 0x04, 0x16, 0x07], Color([255, 0, 0]))
            ]
        );
        assert!(
            !load("[power_profiles_daemon]\nsync = false\n")
                .unwrap()
                .power_profiles_daemon
                .sync
        );
    }

    #[test]
    fn reads_scenarios_in_order() {
        let config = load(
            r#"
            [[scenarios]]
            name = "quiet"
            shift_mode = "eco"
            fan_mode = "silent"

            [[scenarios]]
            name = "games"
            shift_mode = "turbo"
            "#,
        )
        .unwrap();
        let names: Vec<_> = config.scenarios.iter().map(|s| &s.name[..]).collect();
        assert_eq!(names, ["quiet", "games"]);
        assert_eq!(config.scenarios[1].modes().fan_mode, None);
        assert!(load("").unwrap().scenarios.is_empty());

        assert!(load("[[scenarios]]\nname = \"a\"\n").is_err());
        assert!(load("[[scenarios]]\nname = \" \"\nshift_mode = \"eco\"\n").is_err());
        assert!(
            load("[[scenarios]]\nname = \"a\"\nshift_mode = \"eco\"\n[[scenarios]]\nname = \"a\"\nshift_mode = \"turbo\"\n")
                .is_err()
        );
    }

    #[test]
    fn profiles_name_scenarios_there_are() {
        // The built-in ones, without any configured.
        let config =
            load("[profiles.battery]\nscenario = \"silent\"\nsuper_battery = true\n").unwrap();
        let battery = config.profiles.get(Source::Battery).unwrap();
        assert_eq!(battery.scenario.as_deref(), Some("silent"));
        assert!(load("[profiles.ac]\nscenario = \"games\"\n").is_err());

        // The configured ones then, not the built-in.
        let games = "[[scenarios]]\nname = \"games\"\nshift_mode = \"turbo\"\n";
        assert!(load(&format!("{games}[profiles.ac]\nscenario = \"games\"\n")).is_ok());
        let err = load(&format!("{games}[profiles.ac]\nscenario = \"silent\"\n")).unwrap_err();
        assert!(format!("{err:#}").contains("not one of: games"), "{err:#}");
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
        let example = readme
            .split("```toml\n")
            .filter_map(|block| block.split_once("```").map(|(toml, _)| toml))
            .find(|toml| toml.contains("[profiles."))
            .expect("a config example in the readme");
        let config = load(example).unwrap();
        assert_eq!(config.charge_limit, Some(80));
        assert_eq!(config.fn_key.as_deref(), Some("left"));
        assert!(config.profiles.get(Source::Battery).is_some());
        assert!(config.fans.get(Fan::Cpu).is_some());
        assert_eq!(config.scenarios.len(), 2);
        let ac = config.profiles.get(Source::Ac).unwrap();
        assert_eq!(ac.scenario.as_deref(), Some("games"));
        assert!(config.power_profiles_daemon.sync);
        let KeyboardColor(color) = config.keyboard.color.unwrap();
        assert_eq!(color.to_string(), "#ff6000");
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
