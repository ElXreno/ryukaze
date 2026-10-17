//! /etc/ryukaze/config.toml. Every key is optional: what's unset, the
//! daemon leaves alone.

use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result, bail};
use ryukaze_core::{
    COOLER_BOOST_MARKS,
    fans::{Curve, Fan},
    keyboard::{self, Color, Effect, Lighting},
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
    /// The profile shown outside the scenarios with one of their own; the
    /// first if none is named.
    pub profile: Option<String>,

    /// What NextKeyboardProfile steps through, in this order.
    pub profiles: Vec<KeyboardProfile>,

    /// A profile by scenario, shown while the EC is in its modes.
    pub scenarios: BTreeMap<String, String>,

    /// The numpad dark while NumLock is off, but for NumLock itself.
    pub numpad_follows_numlock: bool,
}

/// One way to light the keyboard.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardProfile {
    pub name: String,

    #[serde(default)]
    pub effect: EffectName,

    /// The keys without their own: still, breathing, or between flashes;
    /// with a cycle or a wave, the power button, which they pass by.
    pub color: Option<KeyboardColor>,

    /// How long a breath, or a round of the rainbow, takes.
    pub period_ms: Option<u32>,

    /// In `reactive`, what a pressed key flashes, and for how long.
    pub flash: Option<KeyboardColor>,
    pub flash_ms: Option<u32>,

    /// Keys and groups of keys in their own colours, by name, still
    /// whatever the effect: a single key over a group, a smaller group over
    /// a larger one.
    #[serde(default)]
    pub keys: BTreeMap<String, KeyboardColor>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectName {
    #[default]
    Static,
    Breathe,
    Cycle,
    Wave,
    Reactive,
}

impl KeyboardProfile {
    /// The light it makes, once checked: what its effect needs is there,
    /// in range, nothing it doesn't use is, and its keys exist.
    pub fn lighting(&self) -> Result<Lighting, String> {
        let name = &self.name;
        let color = self.color.map(|KeyboardColor(color)| color);
        let needs_color = || {
            color.map(|_| ()).ok_or_else(|| {
                format!("keyboard profile {name:?}: {:?} needs a color", self.effect)
            })
        };
        let period = |default: u32| {
            let period = self.period_ms.unwrap_or(default);
            if keyboard::PERIODS_MS.contains(&period) {
                Ok(period)
            } else {
                Err(format!(
                    "keyboard profile {name:?}: period_ms {period} is outside {}..{}",
                    keyboard::PERIODS_MS.start(),
                    keyboard::PERIODS_MS.end()
                ))
            }
        };
        let timed = matches!(
            self.effect,
            EffectName::Breathe | EffectName::Cycle | EffectName::Wave
        );
        if self.period_ms.is_some() && !timed {
            return Err(format!(
                "keyboard profile {name:?}: period_ms is for breathe, cycle and wave"
            ));
        }
        if (self.flash.is_some() || self.flash_ms.is_some()) && self.effect != EffectName::Reactive
        {
            return Err(format!("keyboard profile {name:?}: flash is for reactive"));
        }
        let effect = match self.effect {
            EffectName::Static => needs_color().map(|()| Effect::Static)?,
            EffectName::Breathe => {
                needs_color()?;
                Effect::Breathe {
                    period_ms: period(4000)?,
                }
            }
            EffectName::Cycle => Effect::Cycle {
                period_ms: period(6000)?,
            },
            EffectName::Wave => Effect::Wave {
                period_ms: period(3000)?,
            },
            EffectName::Reactive => {
                needs_color()?;
                let KeyboardColor(flash) = self
                    .flash
                    .ok_or_else(|| format!("keyboard profile {name:?}: reactive needs a flash"))?;
                let flash_ms = self.flash_ms.unwrap_or(500);
                if !keyboard::FLASHES_MS.contains(&flash_ms) {
                    return Err(format!(
                        "keyboard profile {name:?}: flash_ms {flash_ms} is outside {}..{}",
                        keyboard::FLASHES_MS.start(),
                        keyboard::FLASHES_MS.end()
                    ));
                }
                Effect::Reactive { flash, flash_ms }
            }
        };
        let keys = self
            .keys
            .iter()
            .map(|(key, &KeyboardColor(color))| {
                keyboard::named(key)
                    .map(|keys| (keys, color))
                    .ok_or_else(|| {
                        format!("keyboard profile {name:?}: no key or group is called {key:?}")
                    })
            })
            .collect::<Result<_, _>>()?;
        Ok(Lighting {
            effect,
            color,
            keys,
        })
    }
}

impl Keyboard {
    /// The profiles, by name; checked at load.
    pub fn lightings(&self) -> Vec<(String, Lighting)> {
        self.profiles
            .iter()
            .filter_map(|profile| Some((profile.name.clone(), profile.lighting().ok()?)))
            .collect()
    }

    /// (scenario, profile) pairs.
    pub fn scenario_profiles(&self) -> Vec<(String, String)> {
        self.scenarios
            .iter()
            .map(|(scenario, profile)| (scenario.clone(), profile.clone()))
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
        let keyboard = &config.keyboard;
        for (i, profile) in keyboard.profiles.iter().enumerate() {
            let name = &profile.name;
            if name.trim().is_empty() {
                bail!("{}: keyboard profile {} has no name", path.display(), i + 1);
            }
            if keyboard.profiles[..i]
                .iter()
                .any(|other| &other.name == name)
            {
                bail!(
                    "{}: two keyboard profiles are called {name:?}",
                    path.display()
                );
            }
            if let Err(err) = profile.lighting() {
                bail!("{}: {err}", path.display());
            }
        }
        let profile_names: Vec<&str> = keyboard.profiles.iter().map(|p| p.name.as_str()).collect();
        let named_profiles = keyboard
            .profile
            .iter()
            .map(|profile| ("keyboard.profile".to_owned(), profile));
        let scenario_profiles = keyboard
            .scenarios
            .iter()
            .map(|(scenario, profile)| (format!("keyboard.scenarios.{scenario}"), profile));
        for (key, profile) in named_profiles.chain(scenario_profiles) {
            if !profile_names.contains(&profile.as_str()) {
                bail!(
                    "{}: {key} {profile:?} is not a keyboard profile: {}",
                    path.display(),
                    profile_names.join(", ")
                );
            }
        }
        for scenario in keyboard.scenarios.keys() {
            if !names.contains(&scenario.as_str()) {
                bail!(
                    "{}: keyboard.scenarios.{scenario} is not one of: {}",
                    path.display(),
                    names.join(", ")
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
        assert!(config.keyboard.numpad_follows_numlock);
        assert_eq!(config.keyboard.lightings().len(), 4);
    }

    #[test]
    fn reads_keyboard_profiles() {
        let config = load(
            r##"
            [keyboard]
            profile = "breathe"

            [keyboard.scenarios]
            performance = "rainbow"

            [[keyboard.profiles]]
            name = "orange"
            color = "#ff6000"
            keys = { wasd = "#ff0000", f7 = "off" }

            [[keyboard.profiles]]
            name = "breathe"
            effect = "breathe"
            color = "#ff6000"

            [[keyboard.profiles]]
            name = "rainbow"
            effect = "wave"
            period_ms = 2500

            [[keyboard.profiles]]
            name = "typing"
            effect = "reactive"
            color = "#301000"
            flash = "#ffffff"
            "##,
        )
        .unwrap();
        let lightings = config.keyboard.lightings();
        let names: Vec<&str> = lightings.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["orange", "breathe", "rainbow", "typing"]);
        assert_eq!(lightings[0].1.effect, Effect::Static);
        assert_eq!(
            lightings[0].1.keys,
            [
                (vec![0x40], Color::OFF),
                (vec![0x1a, 0x04, 0x16, 0x07], Color([255, 0, 0]))
            ]
        );
        assert_eq!(lightings[1].1.effect, Effect::Breathe { period_ms: 4000 });
        assert_eq!(lightings[2].1.effect, Effect::Wave { period_ms: 2500 });
        let flash = Color([255; 3]);
        assert_eq!(
            lightings[3].1,
            Lighting {
                effect: Effect::Reactive {
                    flash,
                    flash_ms: 500
                },
                color: Some(Color([0x30, 0x10, 0])),
                keys: Vec::new(),
            }
        );
        assert_eq!(
            config.keyboard.scenario_profiles(),
            [("performance".to_owned(), "rainbow".to_owned())]
        );

        let one = |profile: &str| load(&format!("[[keyboard.profiles]]\nname = \"p\"\n{profile}"));
        assert!(one("color = \"#ff6000\"\n").is_ok());
        for bad in [
            // Still and breathing need a colour, flashing a flash.
            "",
            "effect = \"breathe\"\n",
            "effect = \"reactive\"\ncolor = \"#301000\"\n",
            // Too quick, and what an effect has no use for.
            "effect = \"wave\"\nperiod_ms = 500\n",
            "color = \"#ff6000\"\nperiod_ms = 4000\n",
            "effect = \"cycle\"\nflash = \"#ffffff\"\n",
            "effect = \"sparkle\"\n",
            "color = \"#ff6000\"\nkeys = { home = \"off\" }\n",
        ] {
            assert!(one(bad).is_err(), "{bad}");
        }
        assert!(load("[keyboard]\nprofile = \"none\"\n").is_err());
        assert!(load("[keyboard.scenarios]\nsilent = \"none\"\n").is_err());
        let twice = "[[keyboard.profiles]]\nname = \"p\"\neffect = \"cycle\"\n";
        assert!(load(&twice.repeat(2)).is_err());
        assert!(load(&format!("{twice}[keyboard.scenarios]\nloud = \"p\"\n")).is_err());
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
