//! /etc/ryukaze/config.toml. Every key is optional: what's unset, the
//! daemon leaves alone.

use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result, bail};
use ryukaze_core::{
    COOLER_BOOST_MARKS,
    fans::{Curve, Fan},
    keyboard::{self, Color, Effect, Lighting, Origin, Zone},
    power_supply::Source,
};
use serde::{Deserialize, Serialize};

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

    /// Of the keys the window changes too, those this config sets, which
    /// it decides at every start.
    #[serde(skip)]
    pub set: Changes,
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

/// One way to light the keyboard; written back as it was read, but for
/// what was left unset, for an editor to start from.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardProfile {
    pub name: String,

    #[serde(default)]
    pub effect: EffectName,

    /// The keys without their own: still, breathing, or between flashes;
    /// with a cycle or a wave, the power button, which they pass by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<KeyboardColor>,

    /// How long a breath, or a round of the colours, takes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u32>,

    /// In `breathe`, what the colour fades to and back from: black unless
    /// told.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_to: Option<KeyboardColor>,

    /// In `cycle` and `wave`, the colours gone through: the rainbow unless
    /// told.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<KeyboardColor>,

    /// In `wave`: `out` of the origin (the default) or `in` to it, the
    /// origin `middle` (the default), `left`, `right`, `top` or `bottom`,
    /// and how wide a round of the colours lies, in percent of the board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wavelength: Option<u8>,

    /// In `reactive`, what a pressed key flashes, and for how long.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash: Option<KeyboardColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash_ms: Option<u32>,

    /// Keys and groups of keys in their own colours, by name, still
    /// whatever the effects: a single key over a group, a smaller group
    /// over a larger one.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, KeyboardColor>,

    /// Parts of the board with an effect of their own, a later one over an
    /// earlier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<KeyboardZone>,
}

/// Keys, by name, with an effect of their own: what a profile takes, but
/// for still colours, which go in its `keys`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardZone {
    pub keys: Vec<String>,
    pub effect: EffectName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<KeyboardColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_to: Option<KeyboardColor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<KeyboardColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wavelength: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash: Option<KeyboardColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectName {
    #[default]
    Static,
    Breathe,
    Cycle,
    Wave,
    Reactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OriginName {
    Middle,
    Left,
    Right,
    Top,
    Bottom,
}

/// What a profile and a zone have in common: an effect and what it takes.
struct EffectFields<'a> {
    effect: EffectName,
    color: Option<KeyboardColor>,
    period_ms: Option<u32>,
    fade_to: Option<KeyboardColor>,
    colors: &'a [KeyboardColor],
    direction: Option<Direction>,
    origin: Option<OriginName>,
    wavelength: Option<u8>,
    flash: Option<KeyboardColor>,
    flash_ms: Option<u32>,
}

impl EffectFields<'_> {
    /// The effect, once checked: what it needs is there and in range, and
    /// nothing it has no use for is. Errors are for `whose`.
    fn effect(&self, whose: &str) -> Result<Effect, String> {
        use EffectName::*;
        let effect = self.effect;
        let only = |set: bool, name: &str, effects: &[EffectName], said: &str| {
            if set && !effects.contains(&effect) {
                Err(format!("{whose}: {name}: only for {said}"))
            } else {
                Ok(())
            }
        };
        only(
            self.period_ms.is_some(),
            "period_ms",
            &[Breathe, Cycle, Wave],
            "breathe, cycle and wave",
        )?;
        only(self.fade_to.is_some(), "fade_to", &[Breathe], "breathe")?;
        only(
            !self.colors.is_empty(),
            "colors",
            &[Cycle, Wave],
            "cycle and wave",
        )?;
        let waving = self.direction.is_some() || self.origin.is_some() || self.wavelength.is_some();
        only(waving, "direction, origin and wavelength", &[Wave], "wave")?;
        let flashing = self.flash.is_some() || self.flash_ms.is_some();
        only(flashing, "flash", &[Reactive], "reactive")?;
        if self.color.is_none() && matches!(effect, Static | Breathe | Reactive) {
            return Err(format!("{whose}: {effect:?} needs a color"));
        }

        let colors: Vec<Color> = if self.colors.is_empty() {
            keyboard::RAINBOW.to_vec()
        } else if keyboard::COLORS.contains(&self.colors.len()) {
            self.colors
                .iter()
                .map(|&KeyboardColor(color)| color)
                .collect()
        } else {
            return Err(format!(
                "{whose}: {} colors; from {} to {}",
                self.colors.len(),
                keyboard::COLORS.start(),
                keyboard::COLORS.end()
            ));
        };
        // Unset, the default, or longer if the colours need it.
        let period = |default: u32, shortest: u32| match self.period_ms {
            None => Ok(default.max(shortest)),
            Some(period) if (shortest..=*keyboard::PERIODS_MS.end()).contains(&period) => {
                Ok(period)
            }
            Some(period) => Err(format!(
                "{whose}: period_ms {period} is outside {shortest}..{}",
                keyboard::PERIODS_MS.end()
            )),
        };
        let round_shortest = keyboard::shortest_period_ms(colors.len());
        Ok(match effect {
            Static => Effect::Static,
            Breathe => Effect::Breathe {
                period_ms: period(4000, *keyboard::PERIODS_MS.start())?,
                fade_to: self
                    .fade_to
                    .map_or(Color::OFF, |KeyboardColor(color)| color),
            },
            Cycle => Effect::Cycle {
                period_ms: period(6000, round_shortest)?,
                colors,
            },
            Wave => {
                let wavelength = self
                    .wavelength
                    .unwrap_or(keyboard::Wave::default().wavelength);
                if !keyboard::WAVELENGTHS.contains(&wavelength) {
                    return Err(format!(
                        "{whose}: wavelength {wavelength} is outside {}..{}",
                        keyboard::WAVELENGTHS.start(),
                        keyboard::WAVELENGTHS.end()
                    ));
                }
                Effect::Wave {
                    period_ms: period(3000, round_shortest)?,
                    colors,
                    wave: keyboard::Wave {
                        origin: match self.origin.unwrap_or(OriginName::Middle) {
                            OriginName::Middle => Origin::Middle,
                            OriginName::Left => Origin::Left,
                            OriginName::Right => Origin::Right,
                            OriginName::Top => Origin::Top,
                            OriginName::Bottom => Origin::Bottom,
                        },
                        inward: self.direction == Some(Direction::In),
                        wavelength,
                    },
                }
            }
            Reactive => {
                let KeyboardColor(flash) = self
                    .flash
                    .ok_or_else(|| format!("{whose}: reactive needs a flash"))?;
                let flash_ms = self.flash_ms.unwrap_or(500);
                if !keyboard::FLASHES_MS.contains(&flash_ms) {
                    return Err(format!(
                        "{whose}: flash_ms {flash_ms} is outside {}..{}",
                        keyboard::FLASHES_MS.start(),
                        keyboard::FLASHES_MS.end()
                    ));
                }
                Effect::Reactive { flash, flash_ms }
            }
        })
    }
}

/// The keys a list of names stands for.
fn named_keys(
    names: impl IntoIterator<Item = impl AsRef<str>>,
    whose: &str,
) -> Result<Vec<u8>, String> {
    let mut keys = Vec::new();
    for name in names {
        let name = name.as_ref();
        let named = keyboard::named(name)
            .ok_or_else(|| format!("{whose}: no key or group is called {name:?}"))?;
        keys.extend(named);
    }
    Ok(keys)
}

impl KeyboardProfile {
    fn fields(&self) -> EffectFields<'_> {
        EffectFields {
            effect: self.effect,
            color: self.color,
            period_ms: self.period_ms,
            fade_to: self.fade_to,
            colors: &self.colors,
            direction: self.direction,
            origin: self.origin,
            wavelength: self.wavelength,
            flash: self.flash,
            flash_ms: self.flash_ms,
        }
    }

    /// The light it makes, once checked.
    pub fn lighting(&self) -> Result<Lighting, String> {
        let whose = format!("keyboard profile {:?}", self.name);
        let effect = self.fields().effect(&whose)?;
        let keys = self
            .keys
            .iter()
            .map(|(key, &KeyboardColor(color))| Ok((named_keys([key], &whose)?, color)))
            .collect::<Result<_, String>>()?;
        if self.zones.len() > keyboard::ZONES {
            return Err(format!(
                "{whose}: {} zones; the controller holds {} besides the profile's",
                self.zones.len(),
                keyboard::ZONES
            ));
        }
        let zones = self
            .zones
            .iter()
            .enumerate()
            .map(|(i, zone)| zone.zone(&format!("{whose}, zone {}", i + 1)))
            .collect::<Result<_, String>>()?;
        Ok(Lighting {
            effect,
            color: self.color.map(|KeyboardColor(color)| color),
            keys,
            zones,
        })
    }
}

impl KeyboardZone {
    fn zone(&self, whose: &str) -> Result<Zone, String> {
        if self.effect == EffectName::Static {
            return Err(format!("{whose}: still colours go in the profile's keys"));
        }
        if self.keys.is_empty() {
            return Err(format!("{whose}: no keys"));
        }
        let fields = EffectFields {
            effect: self.effect,
            color: self.color,
            period_ms: self.period_ms,
            fade_to: self.fade_to,
            colors: &self.colors,
            direction: self.direction,
            origin: self.origin,
            wavelength: self.wavelength,
            flash: self.flash,
            flash_ms: self.flash_ms,
        };
        Ok(Zone {
            keys: named_keys(&self.keys, whose)?,
            effect: fields.effect(whose)?,
            color: self.color.map(|KeyboardColor(color)| color),
        })
    }
}

/// The standard profiles, one an effect, there without a config.
const STANDARD: &str = r##"
[[profiles]]
name = "static"
color = "#ffffff"

[[profiles]]
name = "breathe"
effect = "breathe"
color = "#ffffff"

[[profiles]]
name = "cycle"
effect = "cycle"

[[profiles]]
name = "wave"
effect = "wave"

[[profiles]]
name = "reactive"
effect = "reactive"
color = "#202020"
flash = "#ffffff"
"##;

#[derive(Deserialize)]
struct Standard {
    profiles: Vec<KeyboardProfile>,
}

impl Keyboard {
    /// The standard profiles, each in place of the config's of its name if
    /// there is one, then the config's others: the profiles there are.
    pub fn all_profiles(&self) -> Vec<KeyboardProfile> {
        let Standard { mut profiles } = toml::from_str(STANDARD).expect("the standard profiles");
        for own in &self.profiles {
            match profiles
                .iter_mut()
                .find(|standard| standard.name == own.name)
            {
                Some(standard) => standard.clone_from(own),
                None => profiles.push(own.clone()),
            }
        }
        profiles
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
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyboardColor(pub Color);

impl TryFrom<String> for KeyboardColor {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse().map(KeyboardColor)
    }
}

impl From<KeyboardColor> for String {
    fn from(KeyboardColor(color): KeyboardColor) -> String {
        color.to_string()
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
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "RawCurve", into = "RawCurve")]
pub struct FanCurve(pub Curve);

#[derive(Deserialize, Serialize)]
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

impl From<FanCurve> for RawCurve {
    fn from(FanCurve(curve): FanCurve) -> Self {
        Self {
            temps: curve.temps.to_vec(),
            speeds: curve.speeds.to_vec(),
        }
    }
}

/// What the window changed, kept in the state directory's settings.toml and
/// set at start over the config: the config's keys, in its shape, that the
/// setters change. Unset, the config's stands.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Changes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_limit: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webcam: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fn_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_share: Option<bool>,
    #[serde(skip_serializing_if = "FanChanges::is_empty")]
    pub fans: FanChanges,
    #[serde(skip_serializing_if = "BoostChanges::is_empty")]
    pub cooler_boost: BoostChanges,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FanChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<FanCurve>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<FanCurve>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_follows_cpu: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoostChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_c: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_c: Option<u8>,
}

impl FanChanges {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn curve(&mut self, fan: Fan) -> &mut Option<FanCurve> {
        match fan {
            Fan::Cpu => &mut self.cpu,
            Fan::Gpu => &mut self.gpu,
        }
    }
}

impl BoostChanges {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl Changes {
    /// Of these keys, the ones `table`, the config as written, sets, with
    /// the values they came to in `config`.
    fn set_in(config: &Config, table: &toml::Table) -> Self {
        let has = |path: &str| {
            let mut at = Some(table);
            let mut parts = path.split('.').peekable();
            while let Some(part) = parts.next() {
                let value = at.and_then(|table| table.get(part));
                if parts.peek().is_none() {
                    return value.is_some();
                }
                at = value.and_then(toml::Value::as_table);
            }
            false
        };
        let (fans, boost) = (&config.fans, &config.cooler_boost);
        Self {
            charge_limit: config.charge_limit,
            webcam: config.webcam,
            fn_key: config.fn_key.clone(),
            usb_share: config.usb_share,
            fans: FanChanges {
                cpu: fans.cpu,
                gpu: fans.gpu,
                adaptive: has("fans.adaptive").then_some(fans.adaptive),
                gpu_follows_cpu: has("fans.gpu_follows_cpu").then_some(fans.gpu_follows_cpu),
            },
            cooler_boost: BoostChanges {
                auto: has("cooler_boost.auto").then_some(boost.auto),
                cpu_c: has("cooler_boost.cpu_c").then_some(boost.cpu_c),
                gpu_c: has("cooler_boost.gpu_c").then_some(boost.gpu_c),
            },
        }
    }

    /// These but for the keys `set` has: the config's to decide.
    pub fn without(self, set: &Changes) -> Self {
        fn unless<T, U>(value: Option<T>, set: &Option<U>) -> Option<T> {
            if set.is_some() { None } else { value }
        }
        let (fans, boost) = (self.fans, self.cooler_boost);
        Self {
            charge_limit: unless(self.charge_limit, &set.charge_limit),
            webcam: unless(self.webcam, &set.webcam),
            fn_key: unless(self.fn_key, &set.fn_key),
            usb_share: unless(self.usb_share, &set.usb_share),
            fans: FanChanges {
                cpu: unless(fans.cpu, &set.fans.cpu),
                gpu: unless(fans.gpu, &set.fans.gpu),
                adaptive: unless(fans.adaptive, &set.fans.adaptive),
                gpu_follows_cpu: unless(fans.gpu_follows_cpu, &set.fans.gpu_follows_cpu),
            },
            cooler_boost: BoostChanges {
                auto: unless(boost.auto, &set.cooler_boost.auto),
                cpu_c: unless(boost.cpu_c, &set.cooler_boost.cpu_c),
                gpu_c: unless(boost.gpu_c, &set.cooler_boost.gpu_c),
            },
        }
    }

    /// The ones the config would take, the others dropped with what's wrong.
    pub fn checked(mut self) -> (Self, Vec<String>) {
        let mut wrong = Vec::new();
        if let Some(limit) = self.charge_limit
            && !(10..=100).contains(&limit)
        {
            wrong.push(format!("charge_limit {limit} is outside 10..100"));
            self.charge_limit = None;
        }
        if let Some(side) = &self.fn_key
            && !matches!(side.as_str(), "left" | "right")
        {
            wrong.push(format!("fn_key {side:?}: left or right"));
            self.fn_key = None;
        }
        let boost = &mut self.cooler_boost;
        for (key, mark) in [("cpu_c", &mut boost.cpu_c), ("gpu_c", &mut boost.gpu_c)] {
            if let Some(c) = *mark
                && !COOLER_BOOST_MARKS.contains(&c)
            {
                wrong.push(format!(
                    "cooler_boost.{key} {c} is outside {}..{}",
                    COOLER_BOOST_MARKS.start(),
                    COOLER_BOOST_MARKS.end()
                ));
                *mark = None;
            }
        }
        (self, wrong)
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
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let table: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.set = Changes::set_in(&config, &table);
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
        let all_profiles = keyboard.all_profiles();
        let profile_names: Vec<&str> = all_profiles.iter().map(|p| p.name.as_str()).collect();
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
    fn the_documented_example_parses() {
        let page = include_str!("../../../docs/config.md");
        let example = page
            .split("```toml\n")
            .filter_map(|block| block.split_once("```").map(|(toml, _)| toml))
            .find(|toml| toml.contains("[profiles."))
            .expect("a config example in docs/config.md");
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
        assert_eq!(config.keyboard.profiles.len(), 5);
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
        let lightings: Vec<(String, Lighting)> = config
            .keyboard
            .profiles
            .iter()
            .map(|profile| (profile.name.clone(), profile.lighting().unwrap()))
            .collect();
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
        assert_eq!(
            lightings[1].1.effect,
            Effect::Breathe {
                period_ms: 4000,
                fade_to: Color::OFF
            }
        );
        assert_eq!(
            lightings[2].1.effect,
            Effect::Wave {
                period_ms: 2500,
                colors: keyboard::RAINBOW.to_vec(),
                wave: keyboard::Wave::default()
            }
        );
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
                zones: Vec::new(),
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
        // The standard ones without a config of their own.
        assert!(load("[keyboard]\nprofile = \"wave\"\n").is_ok());
        assert!(load("[keyboard.scenarios]\nsilent = \"none\"\n").is_err());
        let twice = "[[keyboard.profiles]]\nname = \"p\"\neffect = \"cycle\"\n";
        assert!(load(&twice.repeat(2)).is_err());
        assert!(load(&format!("{twice}[keyboard.scenarios]\nloud = \"p\"\n")).is_err());
    }

    #[test]
    fn knows_what_it_sets_of_what_the_window_changes() {
        let config = load("charge_limit = 80\n[fans]\nadaptive = false\n").unwrap();
        let set = &config.set;
        assert_eq!(set.charge_limit, Some(80));
        assert_eq!(set.fans.adaptive, Some(false));
        assert_eq!(set.fans.gpu_follows_cpu, None);
        assert_eq!(set.cooler_boost.auto, None);
        let window = Changes {
            charge_limit: Some(60),
            webcam: Some(true),
            fans: FanChanges {
                adaptive: Some(true),
                gpu_follows_cpu: Some(true),
                ..FanChanges::default()
            },
            ..Changes::default()
        };
        let kept = window.without(set);
        assert_eq!(kept.charge_limit, None);
        assert_eq!(kept.webcam, Some(true));
        assert_eq!(kept.fans.adaptive, None);
        assert_eq!(kept.fans.gpu_follows_cpu, Some(true));
        // Written in the config's shape, only what's there.
        assert_eq!(
            toml::to_string(&kept).unwrap(),
            "webcam = true\n\n[fans]\ngpu_follows_cpu = true\n"
        );
    }

    #[test]
    fn has_a_standard_profile_an_effect() {
        let names = |config: &Config| -> Vec<String> {
            let profiles = config.keyboard.all_profiles();
            for profile in &profiles {
                profile.lighting().unwrap();
            }
            profiles.into_iter().map(|profile| profile.name).collect()
        };
        let standard = ["static", "breathe", "cycle", "wave", "reactive"];
        assert_eq!(names(&load("").unwrap()), standard);
        // The config's in place of the standard of its name, its others after.
        let config = load(
            "[[keyboard.profiles]]\nname = \"mine\"\ncolor = \"#00ff00\"\n\n[[keyboard.profiles]]\nname = \"static\"\ncolor = \"#ff6000\"\n",
        )
        .unwrap();
        assert_eq!(names(&config), [&standard[..], &["mine"]].concat());
        let all = config.keyboard.all_profiles();
        assert_eq!(
            all[0].color.map(|KeyboardColor(c)| c),
            Some(Color([255, 96, 0]))
        );
    }

    /// Written back for KeyboardProfile as it was read, unset fields left
    /// out, and read again the same.
    #[test]
    fn writes_a_profile_back() {
        let config = load(
            r##"
            [[keyboard.profiles]]
            name = "split"
            effect = "wave"
            colors = ["#ff0000", "#0000ff"]
            origin = "left"
            keys = { wasd = "#ffffff", "102nd" = "off" }

            [[keyboard.profiles.zones]]
            keys = ["numpad"]
            effect = "breathe"
            color = "#ff6000"
            "##,
        )
        .unwrap();
        let profile = &config.keyboard.profiles[0];
        let written = toml::to_string(profile).unwrap();
        assert_eq!(
            written,
            "name = \"split\"\neffect = \"wave\"\ncolors = [\"#ff0000\", \"#0000ff\"]\norigin = \"left\"\n\n[keys]\n102nd = \"off\"\nwasd = \"#ffffff\"\n\n[[zones]]\nkeys = [\"numpad\"]\neffect = \"breathe\"\ncolor = \"#ff6000\"\n"
        );
        let again: KeyboardProfile = toml::from_str(&written).unwrap();
        assert_eq!(again.lighting(), profile.lighting());
    }

    /// What the window's editor sends to PreviewKeyboardProfile, as its TOML
    /// writer lays it out: the keys sorted, tables last.
    #[test]
    fn reads_what_the_editor_writes() {
        let still = "color = \"#ffffff\"\nname = \"custom\"\n\n[keys]\n102nd = \"#ff0000\"\na = \"#ff0000\"\n";
        let wave = "colors = [\"#0000ff\", \"#ff0000\"]\ndirection = \"in\"\neffect = \"wave\"\nname = \"custom\"\norigin = \"left\"\nperiod_ms = 4000\n\n[keys]\nw = \"#ff0000\"\n";
        for profile in [still, wave] {
            let profile: KeyboardProfile = toml::from_str(profile).unwrap();
            let lighting = profile.lighting().unwrap();
            assert_eq!(
                lighting.keys.len(),
                if profile.colors.is_empty() { 2 } else { 1 }
            );
        }
    }

    #[test]
    fn reads_effect_options_and_zones() {
        let config = load(
            r##"
            [[keyboard.profiles]]
            name = "split"
            color = "#ff6000"

            [[keyboard.profiles.zones]]
            keys = ["numpad", "arrows"]
            effect = "breathe"
            color = "#ff6000"
            fade_to = "#0040ff"

            [[keyboard.profiles.zones]]
            keys = ["letters"]
            effect = "wave"
            colors = ["#ff0000", "#0000ff"]
            direction = "in"
            origin = "left"
            wavelength = 60
            "##,
        )
        .unwrap();
        let split = &config.keyboard.profiles[0].lighting().unwrap();
        assert_eq!(split.effect, Effect::Static);
        assert_eq!(split.zones.len(), 2);
        assert_eq!(split.zones[0].keys.len(), 17 + 4);
        assert_eq!(
            split.zones[0].effect,
            Effect::Breathe {
                period_ms: 4000,
                fade_to: Color([0, 0x40, 0xff])
            }
        );
        assert_eq!(
            split.zones[1].effect,
            Effect::Wave {
                period_ms: 3000,
                colors: vec![Color([255, 0, 0]), Color([0, 0, 255])],
                wave: keyboard::Wave {
                    origin: Origin::Left,
                    inward: true,
                    wavelength: 60
                }
            }
        );

        let zone = |zone: &str| {
            load(&format!(
                "[[keyboard.profiles]]\nname = \"p\"\ncolor = \"#ff6000\"\n[[keyboard.profiles.zones]]\n{zone}"
            ))
        };
        assert!(zone("keys = [\"wasd\"]\neffect = \"cycle\"\n").is_ok());
        for bad in [
            // Still keys go in the profile's keys; a zone needs keys.
            "keys = [\"wasd\"]\neffect = \"static\"\ncolor = \"#ffffff\"\n",
            "keys = []\neffect = \"cycle\"\n",
            "keys = [\"home\"]\neffect = \"cycle\"\n",
            // Options for other effects, or out of range.
            "keys = [\"wasd\"]\neffect = \"cycle\"\nfade_to = \"#ffffff\"\n",
            "keys = [\"wasd\"]\neffect = \"cycle\"\norigin = \"left\"\n",
            "keys = [\"wasd\"]\neffect = \"wave\"\nwavelength = 0\n",
            "keys = [\"wasd\"]\neffect = \"wave\"\ncolors = [\"#ffffff\"]\n",
            "keys = [\"wasd\"]\neffect = \"wave\"\norigin = \"centre\"\n",
        ] {
            assert!(zone(bad).is_err(), "{bad}");
        }
        // Eight colours need a longer round than the rainbow.
        let eight = "colors = [\"#ff0000\", \"#00ff00\", \"#0000ff\", \"#ffff00\", \"#00ffff\", \"#ff00ff\", \"#ffffff\", \"#ff6000\"]\n";
        assert!(
            zone(&format!(
                "keys = [\"wasd\"]\neffect = \"cycle\"\nperiod_ms = 2000\n{eight}"
            ))
            .is_err()
        );
        assert!(
            zone(&format!(
                "keys = [\"wasd\"]\neffect = \"cycle\"\nperiod_ms = 3000\n{eight}"
            ))
            .is_ok()
        );
        let nine = "[[keyboard.profiles.zones]]\nkeys = [\"wasd\"]\neffect = \"cycle\"\n".repeat(9);
        assert!(
            load(&format!(
                "[[keyboard.profiles]]\nname = \"p\"\ncolor = \"#ff6000\"\n{nine}"
            ))
            .is_err()
        );
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
