use std::{
    collections::BTreeMap,
    fmt::Display,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use ryukaze_core::{
    COOLER_BOOST_MARKS, Status,
    apex::{self, APEX, Apex},
    dgpu::{self, Dgpu},
    fans::{self, Curve, Fan, Layout},
    gpu_mux::{self, GpuMode, GpuMux},
    keyboard::{self, Color, Klc, Lighting},
    msi_ec::MsiEc,
    power_supply::{self, Source},
    raw_ec::RawEc,
    usb_share::{self, UsbShare},
};
use tokio::sync::Notify;
use tracing::{debug, info, warn};
use zbus::{fdo, object_server::SignalEmitter};

use crate::{
    adaptive::{self, Smoothed, Smoothing},
    boost::AutoBoost,
    config::{self, Profile, Scenario},
    power_profiles, saved, state,
};

pub struct Service {
    ec: MsiEc,
    fans: Option<Fans>,
    boost: AutoBoost,
    /// USB Power Share, where the EC's map has it.
    usb_share: Option<(RawEc, UsbShare)>,
    /// The GPU MUX, where the EC's map has it.
    gpu_mux: Option<(RawEc, GpuMux)>,
    /// Apex, where the EC's map has it.
    apex: Option<(RawEc, Apex)>,
    /// Turbo is Apex on AC wherever a mode is picked for the user.
    apex_on_ac: bool,
    /// The configured scenarios, or the built-in ones.
    scenarios: Vec<Scenario>,
    /// Told by the setters that may change the modes, so a change is
    /// announced at once rather than at the next tick.
    poke: Arc<Notify>,
    /// The discrete GPU, on a hybrid laptop.
    dgpu: Option<Dgpu>,
    /// Whether it was awake at the last tick, and since when.
    dgpu_seen: Option<(bool, Instant)>,
    /// Where its users are looked for.
    proc: PathBuf,
    /// The keyboard's standard light profiles and the config's.
    keyboard_config: Vec<config::KeyboardProfile>,
    /// Those saved over D-Bus, under names of their own, and the one picked
    /// last, kept on the disk.
    keyboard_kept: saved::Kept,
    /// Where they're kept; none in the tests.
    keyboard_store: Option<saved::Store>,
    /// The two together, in order: the config's, then the saved.
    keyboard_profiles: Vec<config::KeyboardProfile>,
    /// The one the config names to show outside the scenarios with one of
    /// their own; or else the one picked last, or else the config's first.
    keyboard_named: Option<String>,
    keyboard_first: Option<String>,
    /// (scenario, profile) pairs: the config's, and with what the window
    /// changed.
    keyboard_config_scenarios: Vec<(String, String)>,
    keyboard_scenarios: Vec<(String, String)>,
    /// The one shown, to show again when the controller loses it.
    keyboard_profile: Option<String>,
    /// A profile an editor shows for now, over the one picked.
    keyboard_preview: Option<Lighting>,
    /// The settings changed in the window, kept for the next start but for
    /// those the config sets, `config_set`, which it decides; and where.
    changes: config::Changes,
    config_set: config::Changes,
    changes_file: Option<state::File>,
    /// What to apply when the laptop goes on AC or on battery.
    source_profiles: config::Profiles,
    /// The power button's light: what shows on it, what it follows, and
    /// the colour it was given last.
    power_light: config::PowerLight,
    power_follows: config::PowerFollows,
    power_dgpu_waking: bool,
    /// Its colours: a shift mode's, and `dgpu` for the dGPU's awake.
    power_colors: BTreeMap<String, Color>,
    power_shown: Mutex<Option<Color>>,
    /// NumLock, and whether the numpad follows it.
    numlock: Option<bool>,
    numpad_follows: bool,
    /// power-profiles-daemon: whether it runs, whether its profile goes
    /// with the shift mode, and whether to tell it the shift mode at once.
    ppd_found: bool,
    ppd_sync: bool,
    ppd_resync: bool,
    /// Where the controller's hidraw node is.
    dev: PathBuf,
}

/// Fan curves and tachometers, where the EC's layout is known and its RAM
/// can be read.
pub struct Fans {
    raw: RawEc,
    layout: &'static Layout,
    configured: config::Fans,
    /// The curves advanced runs on: the configured ones, the EC's own where
    /// none is, until a client sets others.
    active: [Curve; 2],
    adaptive: Adaptive,
}

/// The adaptive side of advanced, see adaptive.rs.
#[derive(Debug)]
struct Adaptive {
    on: bool,
    gpu_follows_cpu: bool,
    smoothing: Smoothing,
    temps: [Smoothed; 2],
    /// The speeds the EC is held at, once worked out.
    targets: [Option<u8>; 2],
    last_tick: Option<Instant>,
    ticks: u32,
}

/// Targets this close to the held speed aren't worth an EC write.
const HYSTERESIS: u8 = 2;

/// Every this many ticks the tables are checked even if nothing changed:
/// something else may have written them.
const VERIFY_EVERY: u32 = 15;

impl Adaptive {
    /// Starts over: smoothing from the next reading, targets worked out anew.
    fn rest(&mut self) {
        self.temps = Default::default();
        self.targets = [None; 2];
        self.last_tick = None;
    }
}

impl Fans {
    /// `None`, with the reason in the journal, when this EC can't have them.
    pub fn open(sysfs: &Path, firmware: Option<&str>, configured: config::Fans) -> Option<Self> {
        let Some(layout) = firmware.and_then(fans::layout) else {
            if !configured.is_empty() {
                warn!("fan curves: this EC firmware's layout isn't known, leaving the fans alone");
            }
            return None;
        };
        let raw = RawEc::new(sysfs);
        if let Err(err) = layout.rpm(&raw, Fan::Cpu) {
            warn!("fan curves and rpm: EC RAM unreadable (ec_sys loaded? root?): {err}");
            return None;
        }
        let adaptive = Adaptive {
            on: configured.adaptive,
            gpu_follows_cpu: configured.gpu_follows_cpu,
            smoothing: configured.smoothing,
            temps: Default::default(),
            targets: [None; 2],
            last_tick: None,
            ticks: 0,
        };
        let mut fans = Self {
            raw,
            layout,
            configured,
            active: [layout.factory(Fan::Cpu), layout.factory(Fan::Gpu)],
            adaptive,
        };
        for fan in Fan::ALL {
            fans.reset(fan);
        }
        Some(fans)
    }

    fn reset(&mut self, fan: Fan) {
        self.active[fan as usize] = self
            .configured
            .get(fan)
            .unwrap_or_else(|| self.layout.factory(fan));
        self.adaptive.targets[fan as usize] = None;
    }

    /// What `fan`'s table should hold.
    fn want(&self, fan: Fan, advanced: bool) -> Curve {
        if !advanced {
            return self.layout.factory(fan);
        }
        match (self.adaptive.on, self.adaptive.targets[fan as usize]) {
            (true, Some(speed)) => adaptive::hold(fan, speed),
            // Until the first tick works the speed out, the EC steps.
            _ => self.active[fan as usize],
        }
    }
}

/// The modes sleep can reset: on S3 the EC drops advanced, cooler boost
/// and super battery, and loads its own curves.
#[derive(Debug, Default)]
pub struct Snapshot {
    shift_mode: Option<String>,
    fan_mode: Option<String>,
    super_battery: Option<bool>,
    cooler_boost: Option<bool>,
}

impl Snapshot {
    /// In `profile`'s modes, as far as it sets them.
    fn has(&self, profile: &Profile) -> bool {
        fn same<T: PartialEq>(want: &Option<T>, now: &Option<T>) -> bool {
            want.is_none() || want == now
        }
        same(&profile.shift_mode, &self.shift_mode)
            && same(&profile.fan_mode, &self.fan_mode)
            && same(&profile.super_battery, &self.super_battery)
    }
}

/// MSI Center's, as far as this EC has their modes: one with a mode it
/// lacks is left out, and super battery is turned off only where there is
/// one.
fn built_in_scenarios(ec: &MsiEc) -> Vec<Scenario> {
    let shift_modes = ec.shift_modes().unwrap_or_default();
    let fan_modes = ec.fan_modes().unwrap_or_default();
    let super_battery = ec.super_battery().ok().map(|_| false);
    config::BUILT_IN_SCENARIOS
        .into_iter()
        .filter(|(_, shift, fan)| {
            shift_modes.iter().any(|mode| mode == shift) && fan_modes.iter().any(|mode| mode == fan)
        })
        .map(|(name, shift, fan)| Scenario {
            name: name.into(),
            shift_mode: Some(shift.into()),
            fan_mode: Some(fan.into()),
            super_battery,
        })
        .collect()
}

impl Service {
    pub fn new(ec: MsiEc, fans: Option<Fans>, boost: AutoBoost, scenarios: Vec<Scenario>) -> Self {
        let firmware = ec.firmware().unwrap_or_default();
        let usb_share =
            usb_share::usb_share(&firmware).map(|share| (RawEc::new(ec.sysfs()), share));
        let gpu_mux = gpu_mux::gpu_mux(&firmware).map(|mux| (RawEc::new(ec.sysfs()), mux));
        let apex = apex::apex(&firmware).map(|apex| (RawEc::new(ec.sysfs()), apex));
        let dgpu = dgpu::find(ec.sysfs());
        let scenarios = if scenarios.is_empty() {
            built_in_scenarios(&ec)
        } else {
            scenarios
        };
        Self {
            ec,
            fans,
            boost,
            usb_share,
            gpu_mux,
            apex,
            apex_on_ac: false,
            scenarios,
            poke: Arc::new(Notify::new()),
            dgpu,
            dgpu_seen: None,
            proc: PathBuf::from("/proc"),
            keyboard_config: Vec::new(),
            keyboard_kept: saved::Kept::default(),
            keyboard_store: None,
            keyboard_profiles: Vec::new(),
            keyboard_named: None,
            keyboard_first: None,
            keyboard_config_scenarios: Vec::new(),
            keyboard_scenarios: Vec::new(),
            keyboard_profile: None,
            keyboard_preview: None,
            changes: config::Changes::default(),
            config_set: config::Changes::default(),
            changes_file: None,
            source_profiles: config::Profiles::default(),
            power_light: config::PowerLight::default(),
            power_follows: config::PowerFollows::Profile,
            power_dgpu_waking: false,
            power_colors: BTreeMap::new(),
            power_shown: Mutex::new(None),
            numlock: None,
            numpad_follows: false,
            ppd_found: false,
            ppd_sync: true,
            ppd_resync: false,
            dev: PathBuf::from("/dev"),
        }
    }

    /// The modes `profile` sets: super battery left before the shift mode
    /// goes up, and entered after it comes down. All are tried; the first
    /// failure is returned and the others logged.
    pub fn apply_profile(&mut self, profile: &Profile) -> fdo::Result<()> {
        let profile = &self.fit(self.resolve(profile)?);
        let super_battery = |service: &mut Self| {
            profile
                .super_battery
                .map(|on| service.set_super_battery(on))
        };
        let fan_mode = |service: &mut Self| {
            let mode = profile.fan_mode.as_deref()?;
            Some(service.set_fan_mode(mode))
        };
        let shift_mode = |service: &mut Self| {
            let mode = profile.shift_mode.as_deref()?;
            Some(service.set_shift_mode(mode))
        };
        let results = if profile.super_battery == Some(true) {
            [shift_mode(self), fan_mode(self), super_battery(self)]
        } else {
            [super_battery(self), fan_mode(self), shift_mode(self)]
        };
        let mut errors = results.into_iter().flatten().filter_map(Result::err);
        let first = errors.next();
        for err in errors {
            warn!("{err}");
        }
        first.map_or(Ok(()), Err)
    }

    /// Notified whenever a setter may have changed the modes.
    pub fn poke(&self) -> Arc<Notify> {
        self.poke.clone()
    }

    /// What `ModesChanged` carries; see `Status::modes`.
    pub fn modes(&self) -> Status {
        let now = self.snapshot();
        let keyboard = self.keyboard().map(|_| ());
        Status {
            scenario: self.scenario(&now),
            scenarios: self.scenario_names(),
            keyboard_profile: keyboard.and(self.keyboard_profile.clone()),
            keyboard_profiles: keyboard.and(self.keyboard_names()),
            shift_mode: now.shift_mode,
            fan_mode: now.fan_mode,
            super_battery: now.super_battery,
            cooler_boost: now.cooler_boost,
            ..Status::default()
        }
    }

    fn scenario_names(&self) -> Option<Vec<String>> {
        (!self.scenarios.is_empty()).then(|| {
            self.scenarios
                .iter()
                .map(|scenario| scenario.name.clone())
                .collect()
        })
    }

    fn scenario_named(&self, name: &str) -> fdo::Result<Scenario> {
        self.scenarios
            .iter()
            .find(|scenario| scenario.name == name)
            .cloned()
            .ok_or_else(|| {
                let names = self.scenario_names().unwrap_or_default();
                fdo::Error::InvalidArgs(format!("{name} is not one of: {}", names.join(", ")))
            })
    }

    /// `profile` with its scenario's modes under its own.
    fn resolve(&self, profile: &Profile) -> fdo::Result<Profile> {
        let Some(name) = &profile.scenario else {
            return Ok(profile.clone());
        };
        let modes = self.scenario_named(name)?.modes();
        Ok(Profile {
            scenario: None,
            shift_mode: profile.shift_mode.clone().or(modes.shift_mode),
            fan_mode: profile.fan_mode.clone().or(modes.fan_mode),
            super_battery: profile.super_battery.or(modes.super_battery),
        })
    }

    /// Follows power-profiles-daemon to `profile`, unless the shift mode
    /// is one for it already: the first of its shift modes this EC has, by
    /// a scenario with it or alone. Names what it applied.
    pub fn follow_power_profile(&mut self, profile: &str) -> fdo::Result<Option<String>> {
        let now = self.shift_mode_now().map_err(|e| error("shift_mode", e))?;
        let modes = self
            .shift_modes_here()
            .map_err(|e| error("shift_mode", e))?;
        let wanted: Vec<&str> = power_profiles::shift_modes_for(profile)
            .iter()
            .map(|mode| self.on_source(mode))
            .filter(|mode| modes.iter().any(|known| known == mode))
            .collect();
        // Turbo on AC with Apex wanted there is not where it should be.
        if power_profiles::profile_for(&now) == Some(profile) && wanted.contains(&now.as_str()) {
            return Ok(None);
        }
        for mode in &wanted {
            if let Some(scenario) = self
                .scenarios
                .iter()
                .find(|scenario| {
                    scenario.shift_mode.as_deref().map(|m| self.on_source(m)) == Some(*mode)
                })
                .cloned()
            {
                self.apply_profile(&scenario.modes())?;
                return Ok(Some(format!("scenario {}", scenario.name)));
            }
        }
        let Some(mode) = wanted.first() else {
            return Ok(None);
        };
        self.set_shift_mode(mode)?;
        Ok(Some(format!("shift mode {mode}")))
    }

    /// From the config: whether turbo is Apex on AC.
    pub fn put_apex_on_ac(&mut self, on: bool) {
        self.apex_on_ac = on;
    }

    /// The shift mode the EC is in: msi-ec's, or Apex, which it can't name.
    fn shift_mode_now(&self) -> io::Result<String> {
        let mode = self.ec.shift_mode()?;
        if mode.starts_with("unknown")
            && let Some((raw, apex)) = &self.apex
            && apex.on(raw)?
        {
            return Ok(APEX.into());
        }
        Ok(mode)
    }

    /// msi-ec's shift modes, and Apex where the EC's map has it.
    fn shift_modes_here(&self) -> io::Result<Vec<String>> {
        let mut modes = self.ec.shift_modes()?;
        if self.apex.is_some() {
            modes.push(APEX.into());
        }
        Ok(modes)
    }

    fn on_ac(&self) -> bool {
        power_supply::source(self.ec.sysfs()).ok().flatten() == Some(Source::Ac)
    }

    /// `mode` as the power source takes it: Apex is turbo off AC, and with
    /// `apex_on_ac` turbo is Apex on it.
    fn on_source<'a>(&self, mode: &'a str) -> &'a str {
        match mode {
            APEX if !self.on_ac() => "turbo",
            "turbo" if self.apex_on_ac && self.apex.is_some() && self.on_ac() => APEX,
            _ => mode,
        }
    }

    /// `profile` with its shift mode as the power source takes it.
    fn fit(&self, mut profile: Profile) -> Profile {
        profile.shift_mode = profile
            .shift_mode
            .map(|mode| self.on_source(&mode).to_owned());
        profile
    }

    /// As the laptop goes on AC or off it: out of Apex without AC, and
    /// with `apex_on_ac` from turbo into it with AC.
    pub fn follow_source(&mut self) -> fdo::Result<()> {
        if self.apex.is_none() {
            return Ok(());
        }
        let now = self.shift_mode_now().map_err(|e| error("shift_mode", e))?;
        let fit = self.on_source(&now).to_owned();
        if fit == now {
            return Ok(());
        }
        self.set_shift_mode(&fit)
    }

    /// The settings changed in the window before, read as the daemon
    /// starts but for those the config sets, `set`, which it decides: the
    /// fans' and the automatic boost's set again here, the EC's with the
    /// config's (see `changes`); and where to keep them from now on.
    pub fn keep_changes(&mut self, file: state::File, set: config::Changes) {
        let (changes, wrong) = file.load::<config::Changes>().checked();
        for wrong in wrong {
            warn!("{}: {wrong}", file.path().display());
        }
        self.changes = changes.without(&set);
        self.config_set = set;
        self.changes_file = Some(file);
        let report = |result: fdo::Result<()>| {
            if let Err(err) = result {
                warn!("{err}");
            }
        };
        let config::Changes {
            fans,
            cooler_boost: boost,
            ..
        } = self.changes.clone();
        for (fan, curve) in [(Fan::Cpu, fans.cpu), (Fan::Gpu, fans.gpu)] {
            if let Some(config::FanCurve(curve)) = curve {
                report(self.put_fan_curve(fan, curve));
            }
        }
        if let Some(on) = fans.adaptive {
            report(self.put_adaptive_fans(on));
        }
        if let Some(on) = fans.gpu_follows_cpu {
            report(self.put_gpu_fan_follows_cpu(on));
        }
        if let Some(on) = boost.auto {
            report(self.put_auto_cooler_boost(on));
        }
        if let Some(scenarios) = self.changes.scenarios.clone() {
            match self.check_scenarios(&scenarios) {
                Ok(()) => self.scenarios = scenarios,
                Err(err) => warn!("{}: scenarios: {err}", file_path(&self.changes_file)),
            }
        }
        for source in [Source::Ac, Source::Battery] {
            if let Some(profile) = self.changes.profiles.get(source) {
                *self.source_profiles.get_mut(source) = Some(profile.clone());
            }
        }
        if let Some(follows) = self.changes.keyboard.power {
            self.power_follows = follows;
        }
        if let Some(on) = self.changes.keyboard.power_dgpu_waking {
            self.power_dgpu_waking = on;
        }
        for (name, &config::KeyboardColor(color)) in &self.changes.keyboard.power_colors {
            self.power_colors.insert(name.clone(), color);
        }
        if let Some(on) = self.changes.keyboard.numpad_follows_numlock {
            report(self.put_numpad_follows_numlock(on));
        }
        if let Some(on) = self.changes.power_profiles_daemon.sync {
            self.put_power_profiles_sync(on);
        }
        if boost.cpu_c.is_some() || boost.gpu_c.is_some() {
            let marks = &self.boost.config;
            let (cpu_c, gpu_c) = (
                boost.cpu_c.unwrap_or(marks.cpu_c),
                boost.gpu_c.unwrap_or(marks.gpu_c),
            );
            report(self.put_cooler_boost_marks(cpu_c, gpu_c));
        }
    }

    /// The settings changed in the window and kept, none the config sets.
    pub fn changes(&self) -> &config::Changes {
        &self.changes
    }

    /// A change from the window kept for the next start, unless the config
    /// sets it: then the config's comes back at the next start.
    fn remember(&mut self, change: impl FnOnce(&mut config::Changes)) -> fdo::Result<()> {
        let mut changes = self.changes.clone();
        change(&mut changes);
        let changes = changes.without(&self.config_set);
        if changes == self.changes {
            return Ok(());
        }
        self.changes = changes;
        let Some(file) = &self.changes_file else {
            return Ok(());
        };
        file.store(&self.changes).map_err(|err| {
            fdo::Error::IOError(format!(
                "settings: set until ryukazed restarts, as {} can't be written: {err}",
                file.path().display()
            ))
        })
    }

    /// The numpad dark while NumLock is off, or not.
    pub fn put_numpad_follows_numlock(&mut self, on: bool) -> fdo::Result<()> {
        if self.numpad_follows == on {
            return Ok(());
        }
        self.numpad_follows = on;
        info!("numpad follows numlock: {}", if on { "on" } else { "off" });
        if self.numlock != Some(false) {
            return Ok(());
        }
        let Some(klc) = self.keyboard() else {
            return Ok(());
        };
        if on {
            self.overlay(&klc)
        } else {
            self.light(&klc)
        }
    }

    /// Whether power-profiles-daemon's profile goes with the shift mode;
    /// switched on, it's told the shift mode at the next look.
    pub fn put_power_profiles_sync(&mut self, on: bool) {
        if self.ppd_sync == on {
            return;
        }
        self.ppd_sync = on;
        self.ppd_resync = on;
        info!(
            "in step with power-profiles-daemon: {}",
            if on { "on" } else { "off" }
        );
        self.poke.notify_one();
    }

    /// Scenarios as the EC takes them: named, each name once, in modes it
    /// has, and with every one the config names for AC, battery or a
    /// keyboard light, which stay as they are.
    fn check_scenarios(&self, scenarios: &[Scenario]) -> Result<(), String> {
        let shift_modes = self.shift_modes_here().ok();
        let fan_modes = self.ec.fan_modes().ok();
        let super_battery = self.ec.super_battery().is_ok();
        for (i, scenario) in scenarios.iter().enumerate() {
            let name = &scenario.name;
            if name.trim().is_empty() {
                return Err(format!("scenario {} has no name", i + 1));
            }
            if scenarios[..i].iter().any(|other| other.name == *name) {
                return Err(format!("two scenarios are called {name:?}"));
            }
            let known = |mode: &Option<String>, modes: &Option<Vec<String>>| {
                mode.as_ref()
                    .zip(modes.as_ref())
                    .is_none_or(|(mode, modes)| modes.contains(mode))
            };
            if !known(&scenario.shift_mode, &shift_modes) || !known(&scenario.fan_mode, &fan_modes)
            {
                return Err(format!("{name}: a mode this EC doesn't have"));
            }
            if scenario.super_battery.is_some() && !super_battery {
                return Err(format!("{name}: this EC has no super battery"));
            }
        }
        let set = &self.config_set;
        let named = [&set.profiles.ac, &set.profiles.battery]
            .into_iter()
            .flatten()
            .filter_map(|profile| profile.scenario.as_ref())
            .chain(set.keyboard.scenarios.keys());
        for name in named {
            if scenarios.iter().all(|scenario| scenario.name != *name) {
                return Err(format!("the config names {name:?}, which has to stay"));
            }
        }
        Ok(())
    }

    /// From the config: what the power button's light shows.
    pub fn put_power_light(&mut self, power: config::PowerLight) {
        self.power_follows = power.follows;
        self.power_dgpu_waking = power.dgpu_waking;
        self.power_colors = power.colors();
        self.power_light = power;
    }

    /// From the config: what to apply on AC and on battery.
    pub fn put_source_profiles(&mut self, profiles: config::Profiles) {
        self.source_profiles = profiles;
    }

    /// What to apply on `source`, if anything.
    pub fn source_profile(&self, source: Source) -> Option<config::Profile> {
        self.source_profiles.get(source).cloned()
    }

    /// For Status: the scenario `source` applies, `""` for nothing, `*` for
    /// modes of the config's own.
    fn source_scenario(&self, source: Source) -> Option<String> {
        self.scenario_names()?;
        Some(match self.source_profiles.get(source) {
            Some(config::Profile {
                scenario: Some(scenario),
                ..
            }) => scenario.clone(),
            Some(profile) if *profile != config::Profile::default() => "*".into(),
            _ => String::new(),
        })
    }

    /// NumLock can't be watched: a numpad set to follow it can't.
    pub fn numlock_unwatched(&self) -> fdo::Result<()> {
        if self.numpad_follows {
            return Err(fdo::Error::NotSupported(
                "the numpad can't follow NumLock: it isn't watched".into(),
            ));
        }
        Ok(())
    }

    /// Whether power-profiles-daemon runs, for Status.
    pub fn power_profiles_found(&mut self, found: bool) {
        self.ppd_found = found;
    }

    /// Whether power-profiles-daemon goes with the shift mode.
    pub fn power_profiles_sync(&self) -> bool {
        self.ppd_sync
    }

    /// Whether it's to be told the shift mode now, once.
    pub fn take_power_profiles_resync(&mut self) -> bool {
        std::mem::take(&mut self.ppd_resync)
    }

    pub fn put_fan_curve(&mut self, fan: Fan, curve: Curve) -> fdo::Result<()> {
        let fans = self.fans.as_mut().ok_or_else(no_curves)?;
        fans.active[fan as usize] = curve;
        fans.adaptive.targets[fan as usize] = None;
        self.adapt()?;
        self.sync_curves()
    }

    fn put_adaptive_fans(&mut self, on: bool) -> fdo::Result<()> {
        let fans = self.fans.as_mut().ok_or_else(no_curves)?;
        if fans.adaptive.on == on {
            return Ok(());
        }
        fans.adaptive.on = on;
        fans.adaptive.rest();
        info!("adaptive fans: {}", if on { "on" } else { "off" });
        self.adapt()?;
        self.sync_curves()
    }

    fn put_gpu_fan_follows_cpu(&mut self, on: bool) -> fdo::Result<()> {
        let fans = self.fans.as_mut().ok_or_else(no_curves)?;
        if fans.adaptive.gpu_follows_cpu == on {
            return Ok(());
        }
        fans.adaptive.gpu_follows_cpu = on;
        fans.adaptive.targets[Fan::Gpu as usize] = None;
        info!("gpu fan follows the cpu: {}", if on { "on" } else { "off" });
        self.adapt()
    }

    pub fn put_webcam(&mut self, on: bool) -> fdo::Result<()> {
        apply(
            "webcam",
            on,
            || self.ec.webcam(),
            |&on| self.ec.set_webcam(on),
        )
    }

    pub fn put_fn_key(&mut self, side: &str) -> fdo::Result<()> {
        if !matches!(side, "left" | "right") {
            return Err(fdo::Error::InvalidArgs(format!(
                "fn key {side:?}: left or right"
            )));
        }
        apply(
            "fn_key",
            side.to_owned(),
            || self.ec.fn_key(),
            |_| self.ec.set_fn_key(side),
        )
    }

    pub fn put_usb_share(&mut self, on: bool) -> fdo::Result<()> {
        let (raw, share) = self.usb_share.as_ref().ok_or_else(|| {
            fdo::Error::NotSupported("usb_share: this EC's map doesn't have it".into())
        })?;
        apply("usb_share", on, || share.get(raw), |&on| share.set(raw, on))
    }

    fn put_auto_cooler_boost(&mut self, on: bool) -> fdo::Result<()> {
        if self.boost.config.auto == on {
            return Ok(());
        }
        self.boost.config.auto = on;
        info!("cooler boost when hot: {}", if on { "on" } else { "off" });
        let boost_on = self
            .ec
            .cooler_boost()
            .map_err(|e| error("cooler_boost", e))?;
        match self.boost.stop(boost_on) {
            Some(want) if !on => self.set_cooler_boost(want),
            _ => Ok(()),
        }
    }

    fn put_cooler_boost_marks(&mut self, cpu_c: u8, gpu_c: u8) -> fdo::Result<()> {
        for (name, mark) in [("CPU", cpu_c), ("GPU", gpu_c)] {
            if !COOLER_BOOST_MARKS.contains(&mark) {
                return Err(fdo::Error::InvalidArgs(format!(
                    "{name} mark {mark} °C is outside {}..{}",
                    COOLER_BOOST_MARKS.start(),
                    COOLER_BOOST_MARKS.end()
                )));
            }
        }
        let config = &mut self.boost.config;
        if (config.cpu_c, config.gpu_c) != (cpu_c, gpu_c) {
            info!(
                "cooler boost marks: CPU {} -> {cpu_c} °C, GPU {} -> {gpu_c} °C",
                config.cpu_c, config.gpu_c
            );
            (config.cpu_c, config.gpu_c) = (cpu_c, gpu_c);
        }
        Ok(())
    }

    pub fn put_charge_limit(&mut self, percent: u8) -> fdo::Result<()> {
        if !(10..=100).contains(&percent) {
            return Err(fdo::Error::InvalidArgs(format!(
                "charge limit {percent}% is outside 10..100"
            )));
        }
        apply(
            "charge_limit",
            percent,
            || self.ec.charge_limit(),
            |&percent| self.ec.set_charge_limit(percent),
        )
    }

    /// The profiles saved before and the one picked last, read as the
    /// daemon starts, and where to keep them from now on.
    pub fn keep_keyboard_profiles(&mut self, store: saved::Store) {
        self.keyboard_kept = store.load();
        self.keyboard_store = Some(store);
        self.merge_keyboard_profiles();
    }

    /// From the config: the profiles, the standard ones among them,
    /// checked; the one it names to show outside the scenarios with their
    /// own, and its first, shown if it names none and none was picked; and
    /// the scenarios'. Shown at once for `scenario`, the one the EC is in.
    /// The saved profiles stay.
    pub fn configure_keyboard(
        &mut self,
        profiles: Vec<config::KeyboardProfile>,
        named: Option<String>,
        first: Option<String>,
        scenarios: Vec<(String, String)>,
        scenario: Option<&str>,
    ) -> fdo::Result<()> {
        self.keyboard_named = named;
        self.keyboard_first = first;
        self.keyboard_config = profiles;
        self.merge_keyboard_profiles();
        self.keyboard_config_scenarios = scenarios;
        self.merge_keyboard_scenarios();
        self.keyboard_profile = self.keyboard_profile_for(scenario);
        self.keyboard_preview = None;
        if let Some(profile) = &self.keyboard_profile {
            if self.keyboard().is_none() {
                return Err(no_keyboard(", yet"));
            }
            info!(
                "keyboard: {profile}, for {}",
                scenario.unwrap_or("no scenario")
            );
        }
        self.keyboard_back()
    }

    /// The keyboard's light again: after sleep, and when a hidraw device
    /// appears, which may be the controller coming back.
    pub fn keyboard_back(&self) -> fdo::Result<()> {
        match self.keyboard() {
            Some(klc) => self.light(&klc),
            None => Ok(()),
        }
    }

    /// The profile of `scenario`, the one the EC's modes make, if it has
    /// one, or else the usual one; shown unless it is already.
    pub fn keyboard_for(&mut self, scenario: Option<&str>) -> fdo::Result<()> {
        // A new scenario ends a preview too.
        let previewing = self.keyboard_preview.take().is_some();
        let profile = self.keyboard_profile_for(scenario);
        if profile.is_none() || profile == self.keyboard_profile {
            return if previewing {
                self.keyboard_back()
            } else {
                Ok(())
            };
        }
        self.keyboard_profile = profile;
        if let Some(profile) = &self.keyboard_profile {
            info!(
                "keyboard: {profile}, for {}",
                scenario.unwrap_or("no scenario")
            );
        }
        self.keyboard_back()
    }

    fn keyboard_profile_for(&self, scenario: Option<&str>) -> Option<String> {
        let own = scenario.and_then(|name| {
            self.keyboard_scenarios
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, profile)| profile.clone())
        });
        let picked = self.keyboard_kept.picked.clone().filter(|name| {
            self.keyboard_profiles
                .iter()
                .any(|profile| &profile.name == name)
        });
        own.or_else(|| self.keyboard_named.clone())
            .or(picked)
            .or_else(|| self.keyboard_first.clone())
    }

    /// The config's scenario profiles, and the window's for scenarios it
    /// leaves.
    fn merge_keyboard_scenarios(&mut self) {
        let mut scenarios = self.keyboard_config_scenarios.clone();
        for (scenario, profile) in &self.changes.keyboard.scenarios {
            scenarios.retain(|(known, _)| known != scenario);
            if !profile.is_empty() {
                scenarios.push((scenario.clone(), profile.clone()));
            }
        }
        self.keyboard_scenarios = scenarios;
    }

    /// The config's profiles, then the saved ones but for any of a name the
    /// config has taken since, which wait in the file.
    fn merge_keyboard_profiles(&mut self) {
        let mut profiles = self.keyboard_config.clone();
        for saved in &self.keyboard_kept.profiles {
            if profiles.iter().any(|known| known.name == saved.name) {
                warn!(
                    "keyboard: the saved profile {:?} is unshown, the config having one of its name",
                    saved.name
                );
            } else {
                profiles.push(saved.clone());
            }
        }
        self.keyboard_profiles = profiles;
    }

    /// The names of the saved profiles shown.
    fn keyboard_saved_names(&self) -> Vec<String> {
        self.keyboard_kept
            .profiles
            .iter()
            .filter(|saved| {
                self.keyboard_config
                    .iter()
                    .all(|known| known.name != saved.name)
            })
            .map(|saved| saved.name.clone())
            .collect()
    }

    /// The saved profiles and the one picked to the disk, if they have a
    /// place there.
    fn store_keyboard_profiles(&self) -> fdo::Result<()> {
        let Some(store) = &self.keyboard_store else {
            return Ok(());
        };
        store.store(&self.keyboard_kept).map_err(|err| {
            fdo::Error::IOError(format!(
                "keyboard profiles: kept until ryukazed restarts, as {} can't be written: {err}",
                store.path().display()
            ))
        })
    }

    fn find_keyboard_profile(&self, name: &str) -> fdo::Result<&config::KeyboardProfile> {
        self.keyboard_profiles
            .iter()
            .find(|known| known.name == name)
            .ok_or_else(|| {
                let names: Vec<&str> = self
                    .keyboard_profiles
                    .iter()
                    .map(|profile| profile.name.as_str())
                    .collect();
                fdo::Error::InvalidArgs(format!(
                    "{name} is not one of the keyboard profiles: {}",
                    names.join(", ")
                ))
            })
    }

    /// Shows the profile `name`, and picks it: shown again but in a scenario
    /// with its own, and at the next start.
    fn show_keyboard_profile(&mut self, name: &str) -> fdo::Result<()> {
        let klc = self.keyboard().ok_or_else(|| no_keyboard(""))?;
        self.find_keyboard_profile(name)?;
        self.poke.notify_one();
        self.keyboard_profile = Some(name.to_owned());
        self.keyboard_preview = None;
        info!("keyboard: {name}");
        // Picked, for the next start too.
        if self.keyboard_kept.picked.as_deref() != Some(name) {
            self.keyboard_kept.picked = Some(name.to_owned());
            if let Err(err) = self.store_keyboard_profiles() {
                warn!("{err}");
            }
        }
        self.light(&klc)
    }

    /// What the keyboard shows, kept in the controller's flash for it to
    /// show from power-on, before ryukazed starts: as the laptop goes off
    /// or to sleep, where the blink that saving brings goes unseen, and
    /// not so often as to wear the flash.
    pub fn save_keyboard(&self) -> fdo::Result<()> {
        let Some(klc) = self.keyboard() else {
            return Ok(());
        };
        klc.save()
            .map_err(|err| fdo::Error::IOError(format!("keyboard: {err}")))?;
        info!("keyboard: saved in the controller");
        Ok(())
    }

    /// NumLock went on or off: for a numpad that follows it, dark while
    /// it's off, but for NumLock itself.
    pub fn numlock(&mut self, on: bool) -> fdo::Result<()> {
        if self.numlock == Some(on) {
            return Ok(());
        }
        self.numlock = Some(on);
        if !self.numpad_follows {
            return Ok(());
        }
        let Some(klc) = self.keyboard() else {
            return Ok(());
        };
        // Back on, the numpad rejoins the effect: only the refresh that
        // takes it does, and that starts every key's effect over.
        if on {
            self.light(&klc)
        } else {
            self.overlay(&klc)
        }
    }

    /// What the keyboard shows: the preview, or the profile picked.
    fn lighting(&self) -> fdo::Result<Option<Lighting>> {
        if let Some(preview) = &self.keyboard_preview {
            return Ok(Some(preview.clone()));
        }
        match &self.keyboard_profile {
            // Checked as the config was read or the profile saved.
            Some(name) => self
                .find_keyboard_profile(name)?
                .lighting()
                .map(Some)
                .map_err(fdo::Error::Failed),
            None => Ok(None),
        }
    }

    /// What goes still over the profile: the numpad dark while NumLock is
    /// off, the power button in a colour of its own.
    fn over(&self) -> keyboard::Over {
        keyboard::Over {
            numpad_dark: self.numpad_follows && self.numlock == Some(false),
            power: self.power_color(),
        }
    }

    fn light(&self, klc: &Klc) -> fdo::Result<()> {
        let lighting = self.lighting()?;
        let over = self.over();
        if lighting.is_some() || over != keyboard::Over::default() {
            klc.show(lighting.as_ref(), over)
                .map_err(|err| fdo::Error::IOError(format!("keyboard: {err}")))?;
        }
        self.power_shown(over.power);
        Ok(())
    }

    /// Only what goes over changed: drawn at once, and kept in the
    /// controller's tables, with what runs going on.
    fn overlay(&self, klc: &Klc) -> fdo::Result<()> {
        let lighting = self.lighting()?;
        let over = self.over();
        self.power_shown(over.power);
        klc.overlay(lighting.as_ref(), over)
            .map_err(|err| fdo::Error::IOError(format!("keyboard: {err}")))
    }

    /// The dGPU woke in the last five seconds, for a power button that
    /// shows it.
    fn dgpu_woke(&self) -> bool {
        self.power_dgpu_waking
            && self
                .dgpu_seen
                .is_some_and(|(awake, since)| awake && since.elapsed() < Duration::from_secs(5))
    }

    /// What the power button was last given, `shown` from now on.
    fn power_shown(&self, shown: Option<Color>) -> Option<Color> {
        let mut last = self
            .power_shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *last, shown)
    }

    /// The power button's light as it follows the shift mode or the dGPU,
    /// where it's changed since: at every look.
    pub fn refresh_power(&self) -> fdo::Result<()> {
        if self.power_follows == config::PowerFollows::Profile {
            return Ok(());
        }
        let color = self.power_color();
        // Once, even if it fails: tried again at the next change.
        if color.is_none() || self.power_shown(color) == color {
            return Ok(());
        }
        match self.keyboard() {
            Some(klc) => self.overlay(&klc),
            None => Ok(()),
        }
    }

    /// The power button's colour of its own, following the shift mode or
    /// the dGPU, the profile's where they have none; none following the
    /// profile.
    fn power_color(&self) -> Option<Color> {
        let profile = || self.lighting().ok().flatten().as_ref().and_then(power_in);
        match self.power_follows {
            config::PowerFollows::Profile => None,
            // The dGPU's for a moment as it wakes, if asked.
            config::PowerFollows::ShiftMode if self.dgpu_woke() => {
                self.power_colors.get("dgpu").copied()
            }
            config::PowerFollows::ShiftMode => self
                .shift_mode_now()
                .ok()
                .and_then(|mode| {
                    let colors = &self.power_colors;
                    let turbo = || colors.get("turbo").filter(|_| mode == APEX);
                    colors.get(&mode).or_else(turbo).copied()
                })
                .or_else(profile),
            config::PowerFollows::Dgpu => {
                let awake = self.dgpu_seen.is_some_and(|(awake, _)| awake);
                let color = if awake {
                    self.power_colors.get("dgpu").copied()
                } else {
                    self.power_light.dgpu(false)
                };
                color.or_else(profile)
            }
        }
    }

    fn keyboard_names(&self) -> Option<Vec<String>> {
        (!self.keyboard_profiles.is_empty()).then(|| {
            self.keyboard_profiles
                .iter()
                .map(|profile| profile.name.clone())
                .collect()
        })
    }

    /// Looked for each time: its hidraw node changes when it comes back.
    fn keyboard(&self) -> Option<Klc> {
        keyboard::find(self.ec.sysfs(), &self.dev)
    }

    fn scenario(&self, now: &Snapshot) -> Option<String> {
        self.scenarios
            .iter()
            .find(|scenario| now.has(&self.fit(scenario.modes())))
            .map(|scenario| scenario.name.clone())
    }

    /// Every couple of seconds: the adaptive fans, the automatic boost, and
    /// when the dGPU went to sleep or woke.
    pub fn tick(&mut self) -> fdo::Result<()> {
        let adapted = self.adapt();
        self.auto_boost();
        self.watch_dgpu();
        adapted
    }

    fn watch_dgpu(&mut self) {
        let Some(Ok(awake)) = self.dgpu.as_ref().map(Dgpu::awake) else {
            return;
        };
        if self.dgpu_seen.is_none_or(|(was, _)| was != awake) {
            self.dgpu_seen = Some((awake, Instant::now()));
        }
    }

    fn auto_boost(&mut self) {
        let Ok(on) = self.ec.cooler_boost() else {
            return;
        };
        let temps = [self.ec.cpu_temp().ok(), self.ec.gpu_temp().ok()];
        if let Some(want) = self.boost.tick(temps, on) {
            let [cpu, gpu] = temps.map(|temp| temp.map_or("?".into(), |temp| format!("{temp} °C")));
            info!("cooler boost by itself (cpu {cpu}, gpu {gpu})");
            if let Err(err) = self.set_cooler_boost(want) {
                warn!("{err}");
            }
        }
    }

    /// The curves that go with the fan mode the EC is in now: wherever the
    /// EC may have reloaded its own (start, msi-ec binding, resume).
    pub fn sync_curves(&self) -> fdo::Result<()> {
        if self.fans.is_none() {
            return Ok(());
        }
        let mode = self.ec.fan_mode().map_err(|e| error("fan_mode", e))?;
        self.put_curves(mode == "advanced")
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            shift_mode: self.shift_mode_now().ok(),
            fan_mode: self.ec.fan_mode().ok(),
            super_battery: self.ec.super_battery().ok(),
            cooler_boost: self.ec.cooler_boost().ok(),
        }
    }

    /// Puts back what was there before sleep, Apex only on AC; what's
    /// unchanged stays unwritten. Failures go to the journal.
    pub fn restore(&mut self, snapshot: &Snapshot) {
        let results = [
            snapshot.shift_mode.as_deref().map(|mode| {
                let mode = match mode {
                    APEX if !self.on_ac() => "turbo",
                    _ => mode,
                };
                self.set_shift_mode(mode)
            }),
            snapshot
                .fan_mode
                .as_deref()
                .map(|mode| self.set_fan_mode(mode)),
            snapshot.super_battery.map(|on| self.set_super_battery(on)),
            snapshot.cooler_boost.map(|on| self.set_cooler_boost(on)),
            Some(self.sync_curves()),
        ];
        for err in results.into_iter().flatten().filter_map(Result::err) {
            warn!("{err}");
        }
    }

    /// One step of the adaptive mode, every couple of seconds: in advanced,
    /// the speeds the curves ask for at the smoothed temperatures.
    pub fn adapt(&mut self) -> fdo::Result<()> {
        let Some(fans) = &mut self.fans else {
            return Ok(());
        };
        if !fans.adaptive.on {
            return Ok(());
        }
        if self.ec.fan_mode().map_err(|e| error("fan_mode", e))? != "advanced" {
            fans.adaptive.rest();
            return Ok(());
        }
        let adaptive = &mut fans.adaptive;
        let now = Instant::now();
        let dt = adaptive
            .last_tick
            .replace(now)
            .map_or(0.0, |last| (now - last).as_secs_f32());

        let readings = [self.ec.cpu_temp(), self.ec.gpu_temp()];
        let mut wanted: [Option<f32>; 2] = [None; 2];
        for fan in Fan::ALL {
            let i = fan as usize;
            if let Ok(temp) = &readings[i] {
                let seen = adaptive.temps[i].update(f32::from(*temp), dt, &adaptive.smoothing);
                wanted[i] = Some(adaptive::speed_at(&fans.active[i], seen));
            }
        }
        if adaptive.gpu_follows_cpu
            && let [Some(cpu), gpu] = &mut wanted
        {
            *gpu = Some(gpu.map_or(*cpu, |gpu| gpu.max(*cpu)));
        }

        let mut changed = false;
        for (target, wanted) in adaptive.targets.iter_mut().zip(wanted) {
            let Some(wanted) = wanted else {
                continue;
            };
            let speed = wanted.round().clamp(0.0, 100.0) as u8;
            let close = target.is_some_and(|held| {
                held.abs_diff(speed) < HYSTERESIS && (held == 0) == (speed == 0)
            });
            if !close {
                *target = Some(speed);
                changed = true;
            }
        }
        adaptive.ticks = adaptive.ticks.wrapping_add(1);
        if changed || adaptive.ticks.is_multiple_of(VERIFY_EVERY) {
            self.put_curves(true)?;
        }
        Ok(())
    }

    /// As the daemon stops: from the held speeds back to the curves as the
    /// EC steps them, so the fans go on following the temperature, and a
    /// boost it started ended.
    pub fn stop(&mut self) {
        if let Some(fans) = &mut self.fans {
            fans.adaptive.on = false;
        }
        if let Err(err) = self.sync_curves() {
            warn!("{err}");
        }
        if let Ok(on) = self.ec.cooler_boost()
            && let Some(want) = self.boost.stop(on)
            && let Err(err) = self.set_cooler_boost(want)
        {
            warn!("{err}");
        }
    }

    /// What goes with the mode: in advanced the active curves, held flat
    /// by the adaptive mode or as they are; the EC's own otherwise.
    fn put_curves(&self, advanced: bool) -> fdo::Result<()> {
        let Some(fans) = &self.fans else {
            return Ok(());
        };
        for fan in Fan::ALL {
            let factory = fans.layout.factory(fan);
            let want = fans.want(fan, advanced);
            let held = advanced
                .then(|| fans.adaptive.targets[fan as usize])
                .flatten();
            let which = match held {
                Some(speed) if fans.adaptive.on => format!("held at {speed}%"),
                _ if want == factory => "factory".into(),
                _ => "custom".into(),
            };
            let name = format!("{fan} fan curve");
            let read = || {
                fans.layout
                    .read_curve(&fans.raw, fan)
                    .map_err(|e| raw_error(&name, e))
            };
            if read()? == want {
                continue;
            }
            fans.layout
                .write_curve(&fans.raw, fan, &want)
                .map_err(|e| raw_error(&name, e))?;
            if read()? != want {
                return Err(fdo::Error::Failed(format!("{name}: the EC didn't take it")));
            }
            // Held speeds change all the time; only the rest is worth the journal.
            if held.is_some() && fans.adaptive.on {
                debug!("{name}: {which}");
            } else {
                info!("{name}: {which}");
            }
        }
        Ok(())
    }
}

/// The name has to be a literal here; clients take it from this file.
/// Setters take `&mut self`, so zbus runs them one at a time.
#[zbus::interface(name = "io.github.kontonkara.ryukaze1")]
impl Service {
    /// Reads the EC's state afresh on every call.
    fn status(&self) -> Status {
        let ec = &self.ec;
        let keyboard = self.keyboard().map(|_| ());
        // Quiet: whether it can be read was settled at start.
        let rpm = |fan| {
            let fans = self.fans.as_ref()?;
            fans.layout.rpm(&fans.raw, fan).ok()
        };
        let curve = |fan: Fan| {
            let Curve { temps, speeds } = self.fans.as_ref()?.active[fan as usize];
            Some((temps.to_vec(), speeds.to_vec()))
        };
        let battery = power_supply::battery(ec.sysfs()).unwrap_or_else(|err| {
            warn!("battery: {err}");
            None
        });
        // Where there's a cooler boost to switch.
        let boost = ec.cooler_boost().ok().map(|_| self.boost.config);
        let mux = |read: fn(&GpuMux, &RawEc) -> io::Result<GpuMode>| {
            let (raw, mux) = self.gpu_mux.as_ref()?;
            read(mux, raw).ok().map(|mode| mode.to_string())
        };
        let on_ac = power_supply::source(ec.sysfs())
            .ok()
            .flatten()
            .map(|source| source == Source::Ac);
        let dgpu_awake = self.dgpu.as_ref().and_then(|dgpu| dgpu.awake().ok());
        // Changed since the last tick: just now.
        let dgpu_for_secs = dgpu_awake.zip(self.dgpu_seen).map(|(awake, (was, since))| {
            if awake == was {
                since.elapsed().as_secs()
            } else {
                0
            }
        });
        // Only while it's awake: a scan of /proc isn't free.
        let dgpu_users = self
            .dgpu
            .as_ref()
            .filter(|_| dgpu_awake == Some(true))
            .map(|dgpu| dgpu.users(&self.proc));
        let now = Snapshot {
            shift_mode: field("shift_mode", self.shift_mode_now()),
            fan_mode: field("fan_mode", ec.fan_mode()),
            super_battery: field("super_battery", ec.super_battery()),
            cooler_boost: field("cooler_boost", ec.cooler_boost()),
        };
        let scenario = self.scenario(&now);
        let scenarios = self.scenario_names();
        // Outside advanced the EC runs the two fans together (the same rpm),
        // yet each register holds only what its own curve asks: a cool GPU
        // reads 0 % while its fan turns with the CPU's. Where the layout is
        // known, both get the higher of the two. Seen with the GPU's lower;
        // with it higher, the same is assumed.
        let together = self.fans.is_some() && now.fan_mode.as_deref() != Some("advanced");
        let (cpu_fan, gpu_fan) = match (
            field("cpu/realtime_fan_speed", ec.cpu_fan()),
            field("gpu/realtime_fan_speed", ec.gpu_fan()),
        ) {
            (Some(cpu), Some(gpu)) if together => (Some(cpu.max(gpu)), Some(cpu.max(gpu))),
            fans => fans,
        };
        Status {
            firmware: field("fw_version", ec.firmware()),
            firmware_date: field("fw_release_date", ec.firmware_date()),
            shift_mode: now.shift_mode,
            shift_modes: field("available_shift_modes", self.shift_modes_here()),
            fan_mode: now.fan_mode,
            fan_modes: field("available_fan_modes", ec.fan_modes()),
            super_battery: now.super_battery,
            cooler_boost: now.cooler_boost,
            scenario,
            scenarios,
            cpu_temp: field("cpu/realtime_temperature", ec.cpu_temp()),
            gpu_temp: field("gpu/realtime_temperature", ec.gpu_temp()),
            cpu_fan,
            gpu_fan,
            cpu_rpm: rpm(Fan::Cpu),
            gpu_rpm: rpm(Fan::Gpu),
            cpu_curve: curve(Fan::Cpu),
            gpu_curve: curve(Fan::Gpu),
            adaptive_fans: self.fans.as_ref().map(|fans| fans.adaptive.on),
            gpu_fan_follows_cpu: self.fans.as_ref().map(|fans| fans.adaptive.gpu_follows_cpu),
            auto_cooler_boost: boost.map(|boost| boost.auto),
            cooler_boost_cpu_c: boost.map(|boost| boost.cpu_c),
            cooler_boost_gpu_c: boost.map(|boost| boost.gpu_c),
            webcam: field("webcam", ec.webcam()),
            fn_key: field("fn_key", ec.fn_key()),
            keyboard_profile: keyboard.and(self.keyboard_profile.clone()),
            keyboard_profiles: keyboard.and(self.keyboard_names()),
            keyboard_saved_profiles: keyboard.map(|()| self.keyboard_saved_names()),
            numpad_follows_numlock: keyboard.map(|()| self.numpad_follows),
            power_light: keyboard.map(|()| self.power_follows.name().to_owned()),
            power_light_dgpu: keyboard.map(|()| self.power_dgpu_waking),
            power_colors: keyboard.map(|()| {
                self.power_colors
                    .iter()
                    .map(|(name, color)| (name.clone(), color.to_string()))
                    .collect()
            }),
            ac_scenario: self.source_scenario(Source::Ac),
            battery_scenario: self.source_scenario(Source::Battery),
            config_keys: Some(self.config_set.keys()),
            keyboard_scenarios: keyboard
                .map(|()| self.keyboard_scenarios.iter().cloned().collect()),
            power_profiles_sync: self.ppd_found.then_some(self.ppd_sync),
            usb_share: self
                .usb_share
                .as_ref()
                .and_then(|(raw, share)| share.get(raw).ok()),
            gpu_mode: mux(GpuMux::now),
            gpu_mode_next: mux(GpuMux::next),
            dgpu_awake,
            dgpu_for_secs,
            dgpu_users,
            on_ac,
            battery_level: battery.as_ref().map(|battery| battery.level),
            battery_state: battery.as_ref().map(|battery| battery.state.clone()),
            battery_watts: battery.as_ref().and_then(|battery| battery.watts),
            battery_health: battery.as_ref().and_then(|battery| battery.health),
            charge_limit: field("charge_limit", ec.charge_limit()),
        }
    }

    /// The modes changed, whoever changed them: a client, a power profile,
    /// the automatic boost, or the EC itself (Fn+F8). Seen within a tick.
    #[zbus(signal)]
    pub async fn modes_changed(emitter: &SignalEmitter<'_>, modes: Status) -> zbus::Result<()>;

    /// Applies the scenario after the one the EC is in (the first, if it's
    /// in none) and names it.
    pub fn next_scenario(&mut self) -> fdo::Result<String> {
        if self.scenarios.is_empty() {
            return Err(fdo::Error::NotSupported(
                "scenarios: this EC lacks the built-in ones' modes; set some in the config".into(),
            ));
        }
        let now = self.snapshot();
        let next = self
            .scenarios
            .iter()
            .position(|scenario| now.has(&self.fit(scenario.modes())))
            .map_or(0, |i| (i + 1) % self.scenarios.len());
        let scenario = self.scenarios[next].clone();
        info!("scenario: {}", scenario.name);
        self.apply_profile(&scenario.modes())?;
        Ok(scenario.name)
    }

    /// One of the scenarios in Status.
    pub fn set_scenario(&mut self, name: &str) -> fdo::Result<()> {
        let scenario = self.scenario_named(name)?;
        info!("scenario: {name}");
        self.apply_profile(&scenario.modes())
    }

    /// One of the shift modes in Status; Apex only on AC.
    pub fn set_shift_mode(&mut self, mode: &str) -> fdo::Result<()> {
        self.poke.notify_one();
        let modes = self
            .shift_modes_here()
            .map_err(|e| error("shift_mode", e))?;
        check_mode(mode, &modes)?;
        if mode == APEX && !self.on_ac() {
            return Err(fdo::Error::Failed("shift_mode: apex only on AC".into()));
        }
        apply(
            "shift_mode",
            mode.to_owned(),
            || self.shift_mode_now(),
            |_| match &self.apex {
                Some((raw, apex)) if mode == APEX => apex.enter(raw),
                _ => self.ec.set_shift_mode(mode),
            },
        )
    }

    /// One of the fan modes in Status. Advanced runs on the configured
    /// curves, the others on the EC's own.
    pub fn set_fan_mode(&mut self, mode: &str) -> fdo::Result<()> {
        self.poke.notify_one();
        let modes = self.ec.fan_modes().map_err(|e| error("fan_mode", e))?;
        check_mode(mode, &modes)?;
        let advanced = mode == "advanced";
        // The curves go in before the switch, so advanced starts on them,
        // and the EC's own come back after it.
        if advanced {
            self.put_curves(true)?;
        }
        apply(
            "fan_mode",
            mode.to_owned(),
            || self.ec.fan_mode(),
            |_| self.ec.set_fan_mode(mode),
        )?;
        if !advanced {
            self.put_curves(false)?;
        }
        Ok(())
    }

    /// The curve advanced runs `fan` ("cpu" or "gpu") on; see
    /// `fans::Curve`. In advanced it's on the fan at once.
    pub fn set_fan_curve(&mut self, fan: &str, temps: Vec<u8>, speeds: Vec<u8>) -> fdo::Result<()> {
        let fan = parse_fan(fan)?;
        let curve = Curve::new(&temps, &speeds).map_err(fdo::Error::InvalidArgs)?;
        self.put_fan_curve(fan, curve)?;
        self.remember(|changes| *changes.fans.curve(fan) = Some(config::FanCurve(curve)))
    }

    /// Back to the configured curve, or the EC's own where none is.
    pub fn reset_fan_curve(&mut self, fan: &str) -> fdo::Result<()> {
        let fan = parse_fan(fan)?;
        self.fans.as_mut().ok_or_else(no_curves)?.reset(fan);
        self.adapt()?;
        self.sync_curves()?;
        self.remember(|changes| *changes.fans.curve(fan) = None)
    }

    /// Whether advanced is adaptive; off, the EC steps through the curves
    /// itself.
    pub fn set_adaptive_fans(&mut self, on: bool) -> fdo::Result<()> {
        self.put_adaptive_fans(on)?;
        self.remember(|changes| changes.fans.adaptive = Some(on))
    }

    /// While adaptive: whether the GPU fan runs at least as fast as the
    /// CPU's curve asks.
    pub fn set_gpu_fan_follows_cpu(&mut self, on: bool) -> fdo::Result<()> {
        self.put_gpu_fan_follows_cpu(on)?;
        self.remember(|changes| changes.fans.gpu_follows_cpu = Some(on))
    }

    pub fn set_webcam(&mut self, on: bool) -> fdo::Result<()> {
        self.put_webcam(on)?;
        self.remember(|changes| changes.webcam = Some(on))
    }

    /// `left` or `right` of the Win key.
    pub fn set_fn_key(&mut self, side: &str) -> fdo::Result<()> {
        self.put_fn_key(side)?;
        self.remember(|changes| changes.fn_key = Some(side.to_owned()))
    }

    /// One of the keyboard profiles in Status, picked.
    pub fn set_keyboard_profile(&mut self, name: &str) -> fdo::Result<()> {
        self.show_keyboard_profile(name)
    }

    /// A profile as the config writes one, `[[keyboard.profiles]]`'s keys in
    /// TOML, shown until another is picked, the scenario changes or the
    /// config is read again: for an editor to show what it has. Empty, the
    /// one picked again.
    pub fn preview_keyboard_profile(&mut self, profile: &str) -> fdo::Result<()> {
        let klc = self.keyboard().ok_or_else(|| no_keyboard(""))?;
        self.keyboard_preview = if profile.trim().is_empty() {
            None
        } else {
            Some(
                keyboard_profile(profile, "keyboard preview")?
                    .lighting()
                    .map_err(fdo::Error::InvalidArgs)?,
            )
        };
        self.light(&klc)
    }

    /// The keyboard profile `name` as the config writes one, in TOML: for
    /// an editor to start from.
    pub fn read_keyboard_profile(&self, name: &str) -> fdo::Result<String> {
        toml::to_string(self.find_keyboard_profile(name)?)
            .map_err(|err| fdo::Error::Failed(format!("keyboard profile {name}: {err}")))
    }

    /// A profile as the config writes one, in TOML, kept with the others
    /// and on the disk, in place of a saved one of its name, but under a
    /// name the standard ones and the config's don't have; shown at once,
    /// and picked.
    pub fn save_keyboard_profile(&mut self, profile: &str) -> fdo::Result<()> {
        if self.keyboard().is_none() {
            return Err(no_keyboard(""));
        }
        let profile = keyboard_profile(profile, "keyboard profile")?;
        let name = profile.name.clone();
        if name.trim().is_empty() {
            return Err(fdo::Error::InvalidArgs("keyboard profile: no name".into()));
        }
        if self.keyboard_config.iter().any(|known| known.name == name) {
            return Err(fdo::Error::InvalidArgs(format!(
                "keyboard profile {name}: a standard one or the config's; save it under another name"
            )));
        }
        profile.lighting().map_err(fdo::Error::InvalidArgs)?;
        let saved = &mut self.keyboard_kept.profiles;
        match saved.iter_mut().find(|saved| saved.name == name) {
            Some(saved) => *saved = profile,
            None => saved.push(profile),
        }
        self.merge_keyboard_profiles();
        info!("keyboard: {name} saved");
        let stored = self.store_keyboard_profiles();
        self.show_keyboard_profile(&name)?;
        stored
    }

    /// A saved profile gone, from the disk too; the one shown, if it was
    /// that, the usual one again.
    pub fn delete_keyboard_profile(&mut self, name: &str) -> fdo::Result<()> {
        let saved = &mut self.keyboard_kept.profiles;
        let Some(at) = saved.iter().position(|saved| saved.name == name) else {
            return Err(fdo::Error::InvalidArgs(format!(
                "{name} is not a saved keyboard profile; the standard ones and the config's stay"
            )));
        };
        saved.remove(at);
        if self.keyboard_kept.picked.as_deref() == Some(name) {
            self.keyboard_kept.picked = None;
        }
        self.merge_keyboard_profiles();
        info!("keyboard: {name} deleted");
        let stored = self.store_keyboard_profiles();
        if self.keyboard_profile.as_deref() == Some(name)
            && self
                .keyboard_profiles
                .iter()
                .all(|known| known.name != name)
        {
            self.keyboard_profile = self.keyboard_profile_for(None);
        }
        self.poke.notify_one();
        self.keyboard_back()?;
        stored
    }

    /// Shows the keyboard profile after the one shown (the first, if none)
    /// and names it.
    pub fn next_keyboard_profile(&mut self) -> fdo::Result<String> {
        if self.keyboard().is_none() {
            return Err(no_keyboard(""));
        }
        if self.keyboard_profiles.is_empty() {
            return Err(fdo::Error::NotSupported(
                "keyboard: no profiles; set some in the config".into(),
            ));
        }
        let next = self
            .keyboard_profiles
            .iter()
            .position(|profile| Some(&profile.name) == self.keyboard_profile.as_ref())
            .map_or(0, |i| (i + 1) % self.keyboard_profiles.len());
        let name = self.keyboard_profiles[next].name.clone();
        self.show_keyboard_profile(&name)?;
        Ok(name)
    }

    /// USB ports powered while the laptop is off.
    pub fn set_usb_share(&mut self, on: bool) -> fdo::Result<()> {
        self.put_usb_share(on)?;
        self.remember(|changes| changes.usb_share = Some(on))
    }

    /// `hybrid` or `discrete` from the next restart on. Waits for the
    /// BIOS's answer, which comes through ACPI in a moment.
    pub async fn set_gpu_mode(&mut self, mode: &str) -> fdo::Result<()> {
        let want = GpuMode::from_name(mode).ok_or_else(|| {
            fdo::Error::InvalidArgs(format!("gpu mode {mode:?}: hybrid or discrete"))
        })?;
        let (raw, mux) = self.gpu_mux.as_ref().ok_or_else(|| {
            fdo::Error::NotSupported("gpu_mode: this EC's map doesn't have the MUX".into())
        })?;
        let failed = |e| raw_error("gpu_mode", e);
        // The BIOS flips, so ask only to change what's coming.
        let before = mux.next(raw).map_err(failed)?;
        if before == want {
            return Ok(());
        }
        mux.ask(raw).map_err(failed)?;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            match mux.answer(raw).map_err(failed)? {
                Some(mode) if mode == want => {
                    info!("gpu_mode: {before} -> {want} from the next restart");
                    return Ok(());
                }
                Some(mode) => {
                    return Err(fdo::Error::Failed(format!(
                        "gpu_mode: asked for {want}, but the BIOS took {mode}"
                    )));
                }
                None => {}
            }
        }
        Err(fdo::Error::Failed(
            "gpu_mode: the EC didn't pass the switch on to the BIOS".into(),
        ))
    }

    /// Cooler boost by itself when it gets hot. Switched off, a boost it
    /// started ends.
    pub fn set_auto_cooler_boost(&mut self, on: bool) -> fdo::Result<()> {
        self.put_auto_cooler_boost(on)?;
        self.remember(|changes| changes.cooler_boost.auto = Some(on))
    }

    /// The CPU and GPU temperatures, °C, cooler boost comes on at by
    /// itself.
    pub fn set_cooler_boost_marks(&mut self, cpu_c: u8, gpu_c: u8) -> fdo::Result<()> {
        self.put_cooler_boost_marks(cpu_c, gpu_c)?;
        self.remember(|changes| {
            changes.cooler_boost.cpu_c = Some(cpu_c);
            changes.cooler_boost.gpu_c = Some(gpu_c);
        })
    }

    pub fn set_super_battery(&mut self, on: bool) -> fdo::Result<()> {
        self.poke.notify_one();
        apply(
            "super_battery",
            on,
            || self.ec.super_battery(),
            |&on| self.ec.set_super_battery(on),
        )
    }

    pub fn set_cooler_boost(&mut self, on: bool) -> fdo::Result<()> {
        self.poke.notify_one();
        apply(
            "cooler_boost",
            on,
            || self.ec.cooler_boost(),
            |&on| self.ec.set_cooler_boost(on),
        )
    }

    /// The scenarios, `[[scenarios]]` in TOML as the config writes them:
    /// for an editor to start from.
    pub fn read_scenarios(&self) -> fdo::Result<String> {
        #[derive(serde::Serialize)]
        struct Written<'a> {
            scenarios: &'a [Scenario],
        }
        toml::to_string(&Written {
            scenarios: &self.scenarios,
        })
        .map_err(|err| fdo::Error::Failed(format!("scenarios: {err}")))
    }

    /// All the scenarios, in order, `[[scenarios]]` in TOML as the config
    /// writes them. What AC, battery or a keyboard light took from the
    /// window and loses its scenario goes back to nothing.
    pub fn set_scenarios(&mut self, scenarios: &str) -> fdo::Result<()> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Read {
            #[serde(default)]
            scenarios: Vec<Scenario>,
        }
        let Read { scenarios } = toml::from_str(scenarios)
            .map_err(|err| fdo::Error::InvalidArgs(format!("scenarios: {err}")))?;
        self.check_scenarios(&scenarios)
            .map_err(|err| fdo::Error::InvalidArgs(format!("scenarios: {err}")))?;
        let gone = |name: &str| scenarios.iter().all(|scenario| scenario.name != name);
        let mut changes = self.changes.clone();
        for source in [Source::Ac, Source::Battery] {
            let lost = self
                .source_profiles
                .get(source)
                .and_then(|profile| profile.scenario.as_deref())
                .is_some_and(gone);
            if lost {
                *self.source_profiles.get_mut(source) = Some(config::Profile::default());
                *changes.profiles.get_mut(source) = Some(config::Profile::default());
            }
        }
        changes
            .keyboard
            .scenarios
            .retain(|scenario, _| !gone(scenario));
        let names: Vec<&str> = scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();
        info!("scenarios: {}", names.join(", "));
        changes.scenarios = Some(scenarios.clone());
        self.scenarios = scenarios;
        let kept = self.remember(|kept| *kept = changes);
        self.merge_keyboard_scenarios();
        self.poke.notify_one();
        kept
    }

    /// The scenario to apply when the laptop goes on `source`, `ac` or
    /// `battery`, or nothing for `""`; applied at once on that source.
    pub fn set_source_scenario(&mut self, source: &str, scenario: &str) -> fdo::Result<()> {
        let source = match source {
            "ac" => Source::Ac,
            "battery" => Source::Battery,
            _ => {
                return Err(fdo::Error::InvalidArgs(format!(
                    "{source:?}: ac or battery"
                )));
            }
        };
        let profile = if scenario.is_empty() {
            config::Profile::default()
        } else if self.scenarios.iter().any(|known| known.name == scenario) {
            config::Profile {
                scenario: Some(scenario.to_owned()),
                ..config::Profile::default()
            }
        } else {
            return Err(fdo::Error::InvalidArgs(format!(
                "{scenario} is not one of the scenarios"
            )));
        };
        info!(
            "on {source}: {}",
            if scenario.is_empty() {
                "nothing"
            } else {
                scenario
            }
        );
        *self.source_profiles.get_mut(source) = Some(profile.clone());
        if power_supply::source(self.ec.sysfs()).ok().flatten() == Some(source) {
            self.apply_profile(&profile)?;
        }
        self.remember(|changes| *changes.profiles.get_mut(source) = Some(profile))
    }

    /// The keyboard profile shown while the EC is in `scenario`'s modes, or
    /// none of its own for `""`; shown at once in that scenario.
    pub fn set_scenario_keyboard_profile(
        &mut self,
        scenario: &str,
        profile: &str,
    ) -> fdo::Result<()> {
        if self.scenarios.iter().all(|known| known.name != scenario) {
            return Err(fdo::Error::InvalidArgs(format!(
                "{scenario} is not one of the scenarios"
            )));
        }
        if !profile.is_empty() {
            self.find_keyboard_profile(profile)?;
        }
        let (scenario, profile) = (scenario.to_owned(), profile.to_owned());
        info!(
            "keyboard: {} in {scenario}",
            if profile.is_empty() {
                "the one picked"
            } else {
                &profile
            }
        );
        let mut changes = self.changes.clone();
        changes
            .keyboard
            .scenarios
            .insert(scenario.clone(), profile.clone());
        let kept = self.remember(|kept| *kept = changes);
        // Until the next start, where the config sets it.
        if self.config_set.keyboard.scenarios.contains_key(&scenario) {
            self.keyboard_config_scenarios
                .retain(|(known, _)| *known != scenario);
            if !profile.is_empty() {
                self.keyboard_config_scenarios
                    .push((scenario.clone(), profile));
            }
        }
        self.merge_keyboard_scenarios();
        let now = self.modes().scenario;
        self.keyboard_for(now.as_deref())?;
        kept
    }

    /// What the power button's light follows: `profile`, `shift_mode` or
    /// `dgpu`.
    pub fn set_power_light(&mut self, follows: &str) -> fdo::Result<()> {
        let follows = config::PowerFollows::ALL
            .into_iter()
            .find(|known| known.name() == follows)
            .ok_or_else(|| {
                fdo::Error::InvalidArgs(format!("{follows:?}: profile, shift_mode or dgpu"))
            })?;
        info!("power button: {}", follows.name());
        self.power_follows = follows;
        self.power_shown(None);
        if let Some(klc) = self.keyboard() {
            // Back on the profile's effect: only the refresh puts it there.
            if follows == config::PowerFollows::Profile {
                self.light(&klc)?;
            } else {
                self.overlay(&klc)?;
            }
        }
        self.remember(|changes| changes.keyboard.power = Some(follows))
    }

    /// The power button's colour for `name`, a shift mode or `dgpu`, the
    /// dGPU's awake: `#rrggbb` or `off`.
    pub fn set_power_color(&mut self, name: &str, color: &str) -> fdo::Result<()> {
        let modes = self.shift_modes_here().unwrap_or_default();
        if name != "dgpu" && !modes.iter().any(|mode| mode == name) {
            return Err(fdo::Error::InvalidArgs(format!(
                "{name}: a shift mode, or dgpu"
            )));
        }
        let color: Color = color.parse().map_err(fdo::Error::InvalidArgs)?;
        info!("power button: {name} {color}");
        self.power_colors.insert(name.to_owned(), color);
        self.refresh_power()?;
        self.remember(|changes| {
            changes
                .keyboard
                .power_colors
                .insert(name.to_owned(), config::KeyboardColor(color));
        })
    }

    /// Following the shift mode, whether the power button shows the dGPU's
    /// colour for five seconds as it wakes.
    pub fn set_power_light_dgpu(&mut self, on: bool) -> fdo::Result<()> {
        info!(
            "power button: the dgpu waking {}",
            if on { "shown" } else { "not shown" }
        );
        self.power_dgpu_waking = on;
        self.refresh_power()?;
        self.remember(|changes| changes.keyboard.power_dgpu_waking = Some(on))
    }

    /// The numpad dark while NumLock is off, but for NumLock itself.
    pub fn set_numpad_follows_numlock(&mut self, on: bool) -> fdo::Result<()> {
        self.put_numpad_follows_numlock(on)?;
        self.remember(|changes| changes.keyboard.numpad_follows_numlock = Some(on))
    }

    /// power-profiles-daemon's profile with the shift mode, both ways.
    pub fn set_power_profiles_sync(&mut self, on: bool) -> fdo::Result<()> {
        self.put_power_profiles_sync(on);
        self.remember(|changes| changes.power_profiles_daemon.sync = Some(on))
    }

    /// 10 to 100 percent; charging resumes 10 points below it.
    pub fn set_charge_limit(&mut self, percent: u8) -> fdo::Result<()> {
        self.put_charge_limit(percent)?;
        self.remember(|changes| changes.charge_limit = Some(percent))
    }
}

/// A missing attribute is normal (this firmware lacks the feature); any
/// other failure is worth a line in the journal.
fn field<T>(name: &str, result: io::Result<T>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => {
            warn!("{name}: {err}");
            None
        }
    }
}

fn check_mode(mode: &str, modes: &[String]) -> fdo::Result<()> {
    if modes.iter().any(|known| known == mode) {
        Ok(())
    } else {
        Err(fdo::Error::InvalidArgs(format!(
            "{mode} is not one of: {}",
            modes.join(", ")
        )))
    }
}

/// Writes unless the EC already reports `want`, then reads it back: the
/// driver accepts some writes the EC ignores.
fn apply<T: PartialEq + Display>(
    name: &str,
    want: T,
    read: impl Fn() -> io::Result<T>,
    write: impl FnOnce(&T) -> io::Result<()>,
) -> fdo::Result<()> {
    let before = read().map_err(|e| error(name, e))?;
    if before == want {
        return Ok(());
    }
    write(&want).map_err(|e| error(name, e))?;
    let after = read().map_err(|e| error(name, e))?;
    if after != want {
        return Err(fdo::Error::Failed(format!(
            "{name}: wrote {want}, but the EC reports {after}"
        )));
    }
    info!("{name}: {before} -> {want}");
    Ok(())
}

fn error(name: &str, err: io::Error) -> fdo::Error {
    match err.kind() {
        io::ErrorKind::NotFound => {
            fdo::Error::NotSupported(format!("{name}: msi-ec doesn't expose it on this EC"))
        }
        _ => fdo::Error::IOError(format!("{name}: {err}")),
    }
}

/// The colour `lighting` gives the power button, still or under its
/// effect: what it shows where what it follows has none.
fn power_in(lighting: &Lighting) -> Option<Color> {
    keyboard::paint(lighting.color, &lighting.keys)
        .into_iter()
        .rev()
        .find(|&(key, _)| key == keyboard::POWER)
        .map(|(_, color)| color)
}

/// Where a state file is, for a warning.
fn file_path(file: &Option<state::File>) -> String {
    file.as_ref()
        .map_or_else(String::new, |file| file.path().display().to_string())
}

/// A keyboard profile sent over D-Bus, in TOML; errors for `whose`.
fn keyboard_profile(text: &str, whose: &str) -> fdo::Result<config::KeyboardProfile> {
    toml::from_str(text).map_err(|err| fdo::Error::InvalidArgs(format!("{whose}: {err}")))
}

fn no_keyboard(yet: &str) -> fdo::Error {
    fdo::Error::NotSupported(format!(
        "keyboard: no backlight controller ryukaze knows{yet}"
    ))
}

fn raw_error(name: &str, err: io::Error) -> fdo::Error {
    fdo::Error::IOError(format!("{name}: EC RAM: {err}"))
}

fn parse_fan(name: &str) -> fdo::Result<Fan> {
    Fan::from_name(name)
        .ok_or_else(|| fdo::Error::InvalidArgs(format!("no fan {name:?}: cpu or gpu")))
}

fn no_curves() -> fdo::Error {
    fdo::Error::NotSupported("fan curves: this EC's layout isn't known".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    use crate::config::FanCurve;

    const QUIET: [u8; 7] = [0, 30, 35, 40, 50, 60, 100];

    /// msi-ec's attributes and the EC's RAM as on the Alpha 17, in auto.
    fn alpha17(firmware: &str) -> tempfile::TempDir {
        let sysfs = tempfile::tempdir().unwrap();
        let device = sysfs.path().join("devices/platform/msi-ec");
        fs::create_dir_all(device.join("cpu")).unwrap();
        fs::create_dir_all(device.join("gpu")).unwrap();
        for (attr, text) in [
            ("fw_version", firmware),
            ("fan_mode", "auto\n"),
            ("available_fan_modes", "auto\nsilent\nadvanced\n"),
            ("shift_mode", "comfort\n"),
            ("available_shift_modes", "eco\ncomfort\nturbo\n"),
            ("super_battery", "off\n"),
            ("cpu/realtime_temperature", "62\n"),
            ("gpu/realtime_temperature", "0\n"),
            ("cpu/realtime_fan_speed", "45\n"),
            ("gpu/realtime_fan_speed", "0\n"),
            ("cooler_boost", "off\n"),
            ("webcam", "on\n"),
            ("fn_key", "right\n"),
        ] {
            fs::write(device.join(attr), text).unwrap();
        }
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        ram[0x6a..0x70].copy_from_slice(&[50, 57, 64, 71, 78, 85]);
        ram[0x72..0x79].copy_from_slice(&[0, 40, 45, 50, 57, 65, 80]);
        ram[0x82..0x88].copy_from_slice(&[55, 60, 65, 70, 75, 80]);
        ram[0x8a..0x91].copy_from_slice(&[0, 48, 55, 62, 68, 77, 85]);
        // BIOS bit 3 set, USB Power Share off.
        ram[0xbf] = 0x08;
        // The panel on the iGPU, the rest of the byte as the EC starts it.
        ram[0x2e] = 0x0b;
        fs::write(io, ram).unwrap();
        sysfs
    }

    /// The EC steps through the curves: what most of these tests are about.
    fn service(sysfs: &Path) -> Service {
        service_with(sysfs, false)
    }

    fn service_with(sysfs: &Path, adaptive: bool) -> Service {
        let curves = config::Fans {
            cpu: Some(FanCurve(
                Curve::new(&[55, 62, 69, 76, 83, 90], &QUIET).unwrap(),
            )),
            adaptive,
            ..config::Fans::default()
        };
        let ec = MsiEc::new(sysfs);
        let firmware = ec.firmware().ok();
        let fans = Fans::open(sysfs, firmware.as_deref(), curves);
        let mut service = Service::new(ec, fans, AutoBoost::default(), Vec::new());
        // Never the machine's own /dev: a keyboard found in a made-up sysfs
        // would get the test's colours on the real one.
        service.dev = sysfs.join("dev");
        service
    }

    fn ram(sysfs: &Path) -> Vec<u8> {
        fs::read(sysfs.join("kernel/debug/ec/ec0/io")).unwrap()
    }

    #[test]
    fn advanced_runs_on_the_configured_curve() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());

        service.set_fan_mode("advanced").unwrap();
        let ram = ram(sysfs.path());
        assert_eq!(ram[0x72..0x79], QUIET);
        // The GPU fan has no curve configured: the EC's stays.
        assert_eq!(ram[0x8a..0x91], [0, 48, 55, 62, 68, 77, 85]);
        assert_eq!(service.ec.fan_mode().unwrap(), "advanced");
    }

    #[test]
    fn leaving_advanced_brings_the_factory_curve_back() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        service.set_fan_mode("advanced").unwrap();
        service.set_fan_mode("silent").unwrap();
        assert_eq!(ram(sysfs.path())[0x72..0x79], [0, 40, 45, 50, 57, 65, 80]);
    }

    #[test]
    fn sync_follows_the_mode_the_ec_is_in() {
        let sysfs = alpha17("17KKIMS1.115");
        let service = service(sysfs.path());
        // As after a daemon restart in advanced: the EC still has its own.
        fs::write(
            sysfs.path().join("devices/platform/msi-ec/fan_mode"),
            "advanced\n",
        )
        .unwrap();
        service.sync_curves().unwrap();
        assert_eq!(ram(sysfs.path())[0x72..0x79], QUIET);
    }

    #[test]
    fn unknown_firmware_leaves_the_fans_alone() {
        let sysfs = alpha17("17KKIMS1.114");
        let mut service = service(sysfs.path());
        let before = ram(sysfs.path());
        service.set_fan_mode("advanced").unwrap();
        assert_eq!(ram(sysfs.path()), before);
        assert_eq!(service.status().cpu_rpm, None);
        assert!(service.status().cpu_curve.is_none());
        assert!(
            service
                .set_fan_curve("cpu", vec![0; 6], vec![0; 7])
                .is_err()
        );
    }

    const LOUD: [u8; 7] = [20, 40, 50, 60, 70, 80, 100];

    #[test]
    fn a_set_curve_is_on_at_once_in_advanced() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        service.set_fan_mode("advanced").unwrap();
        service
            .set_fan_curve("gpu", vec![50, 55, 60, 65, 70, 75], LOUD.to_vec())
            .unwrap();
        assert_eq!(ram(sysfs.path())[0x8a..0x91], LOUD);
        assert_eq!(service.status().gpu_curve.unwrap().1, LOUD);
    }

    #[test]
    fn a_set_curve_waits_for_advanced() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        service
            .set_fan_curve("cpu", vec![50, 55, 60, 65, 70, 75], LOUD.to_vec())
            .unwrap();
        // In auto the EC keeps its own.
        assert_eq!(ram(sysfs.path())[0x72..0x79], [0, 40, 45, 50, 57, 65, 80]);
        service.set_fan_mode("advanced").unwrap();
        assert_eq!(ram(sysfs.path())[0x72..0x79], LOUD);
    }

    #[test]
    fn reset_goes_back_to_the_configured_curve() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        service.set_fan_mode("advanced").unwrap();
        service
            .set_fan_curve("cpu", vec![50, 55, 60, 65, 70, 75], LOUD.to_vec())
            .unwrap();
        service.reset_fan_curve("cpu").unwrap();
        assert_eq!(ram(sysfs.path())[0x72..0x79], QUIET);
        // No curve configured for the GPU: its reset is the EC's own.
        service.reset_fan_curve("gpu").unwrap();
        assert_eq!(ram(sysfs.path())[0x8a..0x91], [0, 48, 55, 62, 68, 77, 85]);
    }

    #[test]
    fn rejects_bad_fans_and_curves() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        let before = ram(sysfs.path());
        assert!(
            service
                .set_fan_curve("pump", vec![50, 55, 60, 65, 70, 75], LOUD.to_vec())
                .is_err()
        );
        let falling = vec![50, 40, 30, 20, 10, 0, 0];
        assert!(
            service
                .set_fan_curve("cpu", vec![50, 55, 60, 65, 70, 75], falling)
                .is_err()
        );
        assert_eq!(ram(sysfs.path()), before);
    }

    fn table(sysfs: &Path, fan: Fan) -> Curve {
        let ram = ram(sysfs);
        let (temps, speeds) = match fan {
            Fan::Cpu => (0x6a, 0x72),
            Fan::Gpu => (0x82, 0x8a),
        };
        Curve::new(&ram[temps..temps + 6], &ram[speeds..speeds + 7]).unwrap()
    }

    fn set_temp(sysfs: &Path, fan: Fan, temp: u8) {
        let attr = format!("devices/platform/msi-ec/{fan}/realtime_temperature");
        fs::write(sysfs.join(attr), format!("{temp}\n")).unwrap();
    }

    #[test]
    fn adaptive_holds_what_the_curve_asks() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service_with(sysfs.path(), true);
        service.set_fan_mode("advanced").unwrap();
        service.adapt().unwrap();
        // 62 °C is a dot of the configured curve: 35%.
        assert_eq!(table(sysfs.path(), Fan::Cpu), adaptive::hold(Fan::Cpu, 35));
        // The dGPU sleeps (0 °C, 0%), but its fan helps the CPU's.
        assert_eq!(table(sysfs.path(), Fan::Gpu), adaptive::hold(Fan::Gpu, 35));
        assert_eq!(service.status().adaptive_fans, Some(true));
    }

    #[test]
    fn the_gpu_fan_can_rest() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service_with(sysfs.path(), true);
        service.set_fan_mode("advanced").unwrap();
        service.set_gpu_fan_follows_cpu(false).unwrap();
        assert_eq!(table(sysfs.path(), Fan::Gpu), adaptive::hold(Fan::Gpu, 0));
    }

    #[test]
    fn small_changes_wait_and_big_ones_dont() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service_with(sysfs.path(), true);
        service.set_fan_mode("advanced").unwrap();
        service.adapt().unwrap();
        // 63 °C asks for 36%: within the hysteresis of 35.
        set_temp(sysfs.path(), Fan::Cpu, 63);
        service.adapt().unwrap();
        assert_eq!(table(sysfs.path(), Fan::Cpu), adaptive::hold(Fan::Cpu, 35));
        // A jump straight to 95 °C is past panic, so no smoothing: 100%.
        set_temp(sysfs.path(), Fan::Cpu, 95);
        service.adapt().unwrap();
        assert_eq!(table(sysfs.path(), Fan::Cpu), adaptive::hold(Fan::Cpu, 100));
    }

    #[test]
    fn off_or_stopped_the_ec_steps_again() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service_with(sysfs.path(), true);
        service.set_fan_mode("advanced").unwrap();
        service.adapt().unwrap();
        service.set_adaptive_fans(false).unwrap();
        assert_eq!(ram(sysfs.path())[0x72..0x79], QUIET);

        service.set_adaptive_fans(true).unwrap();
        assert_eq!(table(sysfs.path(), Fan::Cpu), adaptive::hold(Fan::Cpu, 35));
        service.stop();
        assert_eq!(ram(sysfs.path())[0x72..0x79], QUIET);
    }

    #[test]
    fn adaptive_waits_for_advanced() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service_with(sysfs.path(), true);
        let before = ram(sysfs.path());
        service.adapt().unwrap();
        assert_eq!(ram(sysfs.path()), before);
    }

    #[test]
    fn boosts_when_hot_and_stops_with_the_daemon() {
        let sysfs = alpha17("17KKIMS1.115");
        let ec = MsiEc::new(sysfs.path());
        let boost = AutoBoost::new(crate::boost::Config {
            auto: true,
            ..Default::default()
        });
        let mut service = Service::new(ec, None, boost, Vec::new());
        assert_eq!(service.status().auto_cooler_boost, Some(true));

        set_temp(sysfs.path(), Fan::Cpu, 97);
        for _ in 0..3 {
            service.tick().unwrap();
        }
        assert!(service.ec.cooler_boost().unwrap());

        service.stop();
        assert!(!service.ec.cooler_boost().unwrap());
    }

    #[test]
    fn boosts_at_the_marks_set() {
        let sysfs = alpha17("17KKIMS1.115");
        let ec = MsiEc::new(sysfs.path());
        let boost = AutoBoost::new(crate::boost::Config {
            auto: true,
            ..Default::default()
        });
        let mut service = Service::new(ec, None, boost, Vec::new());
        let marks = |service: &Service| {
            let status = service.status();
            (status.cooler_boost_cpu_c, status.cooler_boost_gpu_c)
        };
        assert_eq!(marks(&service), (Some(95), Some(85)));
        assert!(service.set_cooler_boost_marks(50, 85).is_err());
        assert!(service.set_cooler_boost_marks(80, 101).is_err());
        service.set_cooler_boost_marks(80, 75).unwrap();
        assert_eq!(marks(&service), (Some(80), Some(75)));

        set_temp(sysfs.path(), Fan::Cpu, 82);
        for _ in 0..3 {
            service.tick().unwrap();
        }
        assert!(service.ec.cooler_boost().unwrap());
    }

    #[test]
    fn steps_through_the_scenarios() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        // The Alpha has all three built-in ones; comfort and auto is balanced.
        let status = service.status();
        assert_eq!(
            status.scenarios.unwrap(),
            ["silent", "balanced", "performance"]
        );
        assert_eq!(status.scenario.as_deref(), Some("balanced"));

        assert_eq!(service.next_scenario().unwrap(), "performance");
        let status = service.status();
        assert_eq!(status.shift_mode.as_deref(), Some("turbo"));
        assert_eq!(status.fan_mode.as_deref(), Some("auto"));
        assert_eq!(service.next_scenario().unwrap(), "silent");
        assert_eq!(service.status().fan_mode.as_deref(), Some("silent"));

        // In none of them: the first, super battery off again.
        service.set_super_battery(true).unwrap();
        assert_eq!(service.status().scenario, None);
        assert_eq!(service.next_scenario().unwrap(), "silent");
        assert_eq!(service.status().super_battery, Some(false));
    }

    #[test]
    fn the_fans_run_together_outside_advanced() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        // Auto: the GPU's register says 0 %, its fan turns with the CPU's.
        let status = service.status();
        assert_eq!((status.cpu_fan, status.gpu_fan), (Some(45), Some(45)));

        service.set_fan_mode("advanced").unwrap();
        let status = service.status();
        assert_eq!((status.cpu_fan, status.gpu_fan), (Some(45), Some(0)));

        // Elsewhere the registers as they are.
        let other = alpha17("17KKIMS1.114");
        let status = service_with(other.path(), false).status();
        assert_eq!((status.cpu_fan, status.gpu_fan), (Some(45), Some(0)));
    }

    #[tokio::test]
    async fn mode_setters_poke_and_modes_say_what_changed() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        let poke = service.poke();
        let before = service.modes();
        assert_eq!(before.scenario.as_deref(), Some("balanced"));
        // Only the modes: nothing the window polls for.
        assert_eq!(before.cpu_temp, None);

        service.set_cooler_boost(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), poke.notified())
            .await
            .expect("a poke");
        let after = service.modes();
        assert_eq!(after.cooler_boost, Some(true));
        assert_ne!(after, before);

        // By the EC itself: no poke, but the next look sees it.
        fs::write(
            sysfs.path().join("devices/platform/msi-ec/shift_mode"),
            "turbo\n",
        )
        .unwrap();
        assert_eq!(service.modes().scenario.as_deref(), Some("performance"));
    }

    #[test]
    fn configured_scenarios_replace_the_built_in_ones() {
        let sysfs = alpha17("17KKIMS1.115");
        let games = Scenario {
            name: "games".into(),
            shift_mode: Some("turbo".into()),
            fan_mode: None,
            super_battery: None,
        };
        let ec = MsiEc::new(sysfs.path());
        let mut service = Service::new(ec, None, AutoBoost::default(), vec![games]);
        assert_eq!(service.status().scenarios.unwrap(), ["games"]);
        assert_eq!(service.next_scenario().unwrap(), "games");
        assert_eq!(service.status().scenario.as_deref(), Some("games"));
        // A lone one stays put.
        assert_eq!(service.next_scenario().unwrap(), "games");
    }

    #[test]
    fn tells_how_long_the_dgpu_slept_and_who_woke_it() {
        let sysfs = alpha17("17KKIMS1.115");
        let runtime = sysfs
            .path()
            .join("bus/pci/devices/0000:01:00.0/power/runtime_status");
        for (address, boot, node) in [
            ("0000:06:00.0", "1", "card0"),
            ("0000:01:00.0", "0", "card1"),
        ] {
            let device = sysfs.path().join("bus/pci/devices").join(address);
            fs::create_dir_all(device.join("power")).unwrap();
            fs::create_dir_all(device.join("drm").join(node)).unwrap();
            fs::write(device.join("class"), "0x030000\n").unwrap();
            fs::write(device.join("boot_vga"), boot).unwrap();
            fs::write(device.join("power/runtime_status"), "suspended\n").unwrap();
        }
        let proc = tempfile::tempdir().unwrap();
        let game = proc.path().join("42");
        fs::create_dir_all(game.join("fd")).unwrap();
        fs::write(game.join("comm"), "game\n").unwrap();
        std::os::unix::fs::symlink("/dev/dri/card1", game.join("fd/7")).unwrap();

        let mut service = service(sysfs.path());
        service.proc = proc.path().to_owned();
        let status = service.status();
        assert_eq!(status.dgpu_awake, Some(false));
        // Not seen by a tick yet: no telling how long.
        assert_eq!(status.dgpu_for_secs, None);
        assert_eq!(status.dgpu_users, None);

        service.tick().unwrap();
        assert_eq!(service.status().dgpu_for_secs, Some(0));

        fs::write(&runtime, "active\n").unwrap();
        let status = service.status();
        assert_eq!(status.dgpu_awake, Some(true));
        assert_eq!(status.dgpu_for_secs, Some(0));
        assert_eq!(status.dgpu_users.unwrap(), ["game"]);
    }

    #[test]
    fn picks_a_scenario_by_name() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        service.set_scenario("silent").unwrap();
        let status = service.status();
        assert_eq!(status.scenario.as_deref(), Some("silent"));
        assert_eq!(status.fan_mode.as_deref(), Some("silent"));
        let err = service.set_scenario("loud").unwrap_err();
        assert!(
            err.to_string().contains("silent, balanced, performance"),
            "{err}"
        );
    }

    #[test]
    fn follows_power_profiles_daemon() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        // Comfort is balanced already.
        assert_eq!(service.follow_power_profile("balanced").unwrap(), None);
        assert_eq!(
            service
                .follow_power_profile("power-saver")
                .unwrap()
                .as_deref(),
            Some("scenario silent")
        );
        assert_eq!(service.status().fan_mode.as_deref(), Some("silent"));
        assert_eq!(
            service
                .follow_power_profile("performance")
                .unwrap()
                .as_deref(),
            Some("scenario performance")
        );
        assert_eq!(service.status().shift_mode.as_deref(), Some("turbo"));
        // A profile this daemon doesn't know.
        assert_eq!(service.follow_power_profile("quiet").unwrap(), None);

        // Without a scenario for it, the shift mode alone.
        let games = Scenario {
            name: "games".into(),
            shift_mode: Some("turbo".into()),
            fan_mode: None,
            super_battery: None,
        };
        let ec = MsiEc::new(sysfs.path());
        let mut service = Service::new(ec, None, AutoBoost::default(), vec![games]);
        assert_eq!(
            service
                .follow_power_profile("power-saver")
                .unwrap()
                .as_deref(),
            Some("shift mode eco")
        );
        let status = service.status();
        assert_eq!(status.shift_mode.as_deref(), Some("eco"));
        assert_eq!(status.fan_mode.as_deref(), Some("auto"));
    }

    #[test]
    fn keyboard_profiles_where_the_controller_is() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        let profile = |text: &str| keyboard_profile(text, "test").unwrap();
        let profiles = vec![
            profile("name = \"orange\"\ncolor = \"#ff6000\"\n"),
            profile("name = \"rainbow\"\neffect = \"wave\"\n"),
            profile("name = \"white\"\ncolor = \"#ffffff\"\n"),
        ];
        let silent = vec![("silent".to_owned(), "rainbow".to_owned())];
        // No controller: said so, and nothing in Status.
        assert!(matches!(
            service.configure_keyboard(
                profiles.clone(),
                None,
                Some("orange".into()),
                silent.clone(),
                Some("balanced")
            ),
            Err(fdo::Error::NotSupported(_))
        ));
        assert!(matches!(
            service.next_keyboard_profile(),
            Err(fdo::Error::NotSupported(_))
        ));
        assert_eq!(service.status().keyboard_profile, None);

        // The controller's first interface, its node a plain file here.
        let usb = sysfs.path().join("devices/usb/5-1.2:1.0");
        let hid = usb.join("0003:1038:113A.0004");
        fs::create_dir_all(&hid).unwrap();
        fs::write(usb.join("bInterfaceNumber"), "00\n").unwrap();
        fs::write(hid.join("uevent"), "HID_ID=0003:00001038:0000113A\n").unwrap();
        let class = sysfs.path().join("class/hidraw/hidraw3");
        fs::create_dir_all(&class).unwrap();
        std::os::unix::fs::symlink(&hid, class.join("device")).unwrap();
        let dev = tempfile::tempdir().unwrap();
        fs::write(dev.path().join("hidraw3"), "").unwrap();
        service.dev = dev.path().to_owned();

        // The usual profile, the first; a scenario's own while the EC is in
        // it. A file takes no HID reports, hence the errors.
        let shown = |service: &Service| service.status().keyboard_profile.unwrap();
        let _ = service.configure_keyboard(
            profiles.clone(),
            None,
            Some("orange".into()),
            silent,
            Some("balanced"),
        );
        assert_eq!(shown(&service), "orange");
        assert_eq!(
            service.status().keyboard_profiles.unwrap(),
            ["orange", "rainbow", "white"]
        );
        let _ = service.keyboard_for(Some("silent"));
        assert_eq!(shown(&service), "rainbow");
        let _ = service.keyboard_for(Some("balanced"));
        assert_eq!(shown(&service), "orange");

        // Picked, stepped through, and announced; the next scenario's wins.
        assert!(matches!(
            service.set_keyboard_profile("blue"),
            Err(fdo::Error::InvalidArgs(_))
        ));
        assert!(matches!(
            service.set_keyboard_profile("white"),
            Err(fdo::Error::IOError(_))
        ));
        assert_eq!(shown(&service), "white");
        let _ = service.next_keyboard_profile();
        assert_eq!(shown(&service), "orange");
        assert_eq!(service.modes().keyboard_profile.as_deref(), Some("orange"));
        let _ = service.keyboard_for(Some("silent"));
        assert_eq!(shown(&service), "rainbow");
        assert!(service.keyboard_back().is_err());

        // A preview, checked as the config is, until a profile is picked,
        // the scenario changes or it's emptied.
        let green = "name = \"draft\"\ncolor = \"#00ff00\"\n";
        for bad in [
            "name = \"draft\"\neffect = \"sparkle\"\n",
            "name = \"draft\"\n",
        ] {
            assert!(matches!(
                service.preview_keyboard_profile(bad),
                Err(fdo::Error::InvalidArgs(_))
            ));
        }
        let _ = service.preview_keyboard_profile(green);
        assert!(service.keyboard_preview.is_some());
        let _ = service.keyboard_for(Some("balanced"));
        assert!(service.keyboard_preview.is_none());
        let _ = service.preview_keyboard_profile(green);
        let _ = service.set_keyboard_profile("white");
        assert!(service.keyboard_preview.is_none());
        let _ = service.preview_keyboard_profile(green);
        let _ = service.preview_keyboard_profile("");
        assert!(service.keyboard_preview.is_none());

        // Read as the config has it; saved over one of its name or after
        // the others, shown, and kept when the config is applied again.
        assert_eq!(
            service.read_keyboard_profile("rainbow").unwrap(),
            "name = \"rainbow\"\neffect = \"wave\"\n"
        );
        assert!(matches!(
            service.read_keyboard_profile("blue"),
            Err(fdo::Error::InvalidArgs(_))
        ));
        for bad in ["name = \"\"\ncolor = \"#00ff00\"\n", "name = \"draft\"\n"] {
            assert!(matches!(
                service.save_keyboard_profile(bad),
                Err(fdo::Error::InvalidArgs(_))
            ));
        }
        let state = tempfile::tempdir().unwrap();
        service.keep_keyboard_profiles(saved::Store::new(state.path()));
        // Under a name of its own only: the config's stay as they are.
        assert!(matches!(
            service.save_keyboard_profile("name = \"white\"\ncolor = \"#eeeeee\"\n"),
            Err(fdo::Error::InvalidArgs(_))
        ));
        let _ = service.save_keyboard_profile(green);
        assert_eq!(shown(&service), "draft");
        let names = ["orange", "rainbow", "white", "draft"];
        assert_eq!(service.status().keyboard_profiles.unwrap(), names);
        assert_eq!(service.status().keyboard_saved_profiles.unwrap(), ["draft"]);
        let orange = Some("orange".to_owned());
        let _ =
            service.configure_keyboard(profiles.clone(), None, orange.clone(), Vec::new(), None);
        assert_eq!(service.status().keyboard_profiles.unwrap(), names);

        // On the disk for the next start, picked as it was left.
        let mut next = self::service(sysfs.path());
        next.dev = service.dev.clone();
        next.keep_keyboard_profiles(saved::Store::new(state.path()));
        let _ = next.configure_keyboard(profiles.clone(), None, orange.clone(), Vec::new(), None);
        assert_eq!(next.status().keyboard_profiles.unwrap(), names);
        assert_eq!(shown(&next), "draft");
        let _ = next.set_keyboard_profile("white");
        let mut again = self::service(sysfs.path());
        again.keep_keyboard_profiles(saved::Store::new(state.path()));
        let _ = again.configure_keyboard(profiles.clone(), None, orange.clone(), Vec::new(), None);
        assert_eq!(again.keyboard_profile.as_deref(), Some("white"));
        // The one the config names, over the one picked.
        let _ = again.configure_keyboard(profiles.clone(), orange.clone(), None, Vec::new(), None);
        assert_eq!(again.keyboard_profile.as_deref(), Some("orange"));

        // Deleted: only a saved one, and the usual one shown for it.
        assert!(matches!(
            next.delete_keyboard_profile("orange"),
            Err(fdo::Error::InvalidArgs(_))
        ));
        let _ = next.set_keyboard_profile("draft");
        let _ = next.delete_keyboard_profile("draft");
        assert_eq!(shown(&next), "orange");
        assert_eq!(next.status().keyboard_saved_profiles.unwrap(), [""; 0]);
        let mut gone = self::service(sysfs.path());
        gone.keep_keyboard_profiles(saved::Store::new(state.path()));
        assert!(gone.keyboard_kept.profiles.is_empty());
        assert_eq!(gone.keyboard_kept.picked, None);

        // A saved one the config took the name of since waits, unshown.
        gone.keyboard_kept
            .profiles
            .push(keyboard_profile("name = \"white\"\ncolor = \"#010101\"\n", "test").unwrap());
        let _ = gone.configure_keyboard(profiles, None, orange, Vec::new(), None);
        assert_eq!(
            gone.status().keyboard_profiles.unwrap(),
            ["orange", "rainbow", "white"]
        );
        assert!(
            gone.read_keyboard_profile("white")
                .unwrap()
                .contains("#ffffff")
        );

        // A scenario's own from the window: kept, and back to the picked
        // one with none.
        gone.dev = service.dev.clone();
        gone.keep_changes(
            state::File::new(state.path(), "settings.toml"),
            config::Changes::default(),
        );
        assert!(gone.set_scenario_keyboard_profile("loud", "white").is_err());
        assert!(
            gone.set_scenario_keyboard_profile("silent", "blue")
                .is_err()
        );
        let _ = gone.set_scenario_keyboard_profile("silent", "white");
        let own = |service: &Service| service.status().keyboard_scenarios.unwrap();
        assert_eq!(own(&gone).get("silent").map(String::as_str), Some("white"));
        let kept: config::Changes = state::File::new(state.path(), "settings.toml").load();
        assert_eq!(
            kept.keyboard.scenarios.get("silent").map(String::as_str),
            Some("white")
        );
        let _ = gone.set_scenario_keyboard_profile("silent", "");
        assert_eq!(own(&gone).get("silent"), None);
    }

    #[test]
    fn a_profile_takes_a_scenario_under_its_own_modes() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        let battery = Profile {
            scenario: Some("silent".into()),
            super_battery: Some(true),
            ..Profile::default()
        };
        service.apply_profile(&battery).unwrap();
        let status = service.status();
        assert_eq!(status.shift_mode.as_deref(), Some("eco"));
        assert_eq!(status.fan_mode.as_deref(), Some("silent"));
        assert_eq!(status.super_battery, Some(true));
        // Super battery over silent's: no longer silent.
        assert_eq!(status.scenario, None);
    }

    #[test]
    fn keeps_the_windows_changes_but_not_the_configs() {
        let sysfs = alpha17("17KKIMS1.115");
        let state = tempfile::tempdir().unwrap();
        let file = || state::File::new(state.path(), "settings.toml");
        // The config sets the Fn key and what AC applies, and nothing of
        // the boost.
        let set = config::Changes {
            fn_key: Some("right".into()),
            profiles: config::Profiles {
                ac: Some(config::Profile::default()),
                battery: None,
            },
            ..config::Changes::default()
        };
        let mut service = service(sysfs.path());
        service.keep_changes(file(), set.clone());
        service.set_fn_key("left").unwrap();
        service.set_auto_cooler_boost(true).unwrap();
        service.set_cooler_boost_marks(90, 80).unwrap();
        service.set_numpad_follows_numlock(true).unwrap();
        assert!(service.set_power_light("rainbow").is_err());
        service.set_power_light("shift_mode").unwrap();
        service.set_power_light_dgpu(true).unwrap();
        service.set_power_color("turbo", "#ff00ff").unwrap();
        service.set_power_color("dgpu", "#0000ff").unwrap();
        assert!(service.set_power_color("warp", "#ff00ff").is_err());
        assert!(service.set_power_color("eco", "pink").is_err());
        assert_eq!(service.status().power_light_dgpu, None);
        service.set_power_profiles_sync(false).unwrap();
        assert!(!service.power_profiles_sync());
        service.set_source_scenario("ac", "silent").unwrap();
        service.set_source_scenario("battery", "silent").unwrap();
        assert!(service.set_source_scenario("battery", "loud").is_err());
        assert!(service.set_source_scenario("mains", "").is_err());
        let status = service.status();
        assert_eq!(status.battery_scenario.as_deref(), Some("silent"));
        assert!(
            status
                .config_keys
                .unwrap()
                .contains(&"profiles.ac".to_owned())
        );
        let kept: config::Changes = file().load();
        assert_eq!(kept.profiles.ac, None);
        assert_eq!(
            kept.profiles
                .battery
                .and_then(|profile| profile.scenario)
                .as_deref(),
            Some("silent")
        );
        assert_eq!(kept.keyboard.numpad_follows_numlock, Some(true));
        assert_eq!(kept.keyboard.power, Some(config::PowerFollows::ShiftMode));
        assert_eq!(kept.keyboard.power_dgpu_waking, Some(true));
        let kept_color = |name: &str| kept.keyboard.power_colors.get(name).map(|c| c.0);
        assert_eq!(kept_color("turbo"), Some(Color([255, 0, 255])));
        assert_eq!(kept_color("dgpu"), Some(Color([0, 0, 255])));
        assert_eq!(kept.power_profiles_daemon.sync, Some(false));
        assert_eq!(kept.fn_key, None);
        assert_eq!(kept.cooler_boost.auto, Some(true));
        assert_eq!(
            (kept.cooler_boost.cpu_c, kept.cooler_boost.gpu_c),
            (Some(90), Some(80))
        );

        // Set again at the next start; a bad one left out.
        std::fs::write(
            file().path(),
            "fn_key = \"left\"\ncharge_limit = 5\n[cooler_boost]\nauto = true\ncpu_c = 90\ngpu_c = 80\n",
        )
        .unwrap();
        let mut next = self::service(sysfs.path());
        next.keep_changes(file(), set);
        assert_eq!(next.changes().fn_key, None);
        assert_eq!(next.changes().charge_limit, None);
        assert!(next.boost.config.auto);
        assert_eq!((next.boost.config.cpu_c, next.boost.config.gpu_c), (90, 80));
    }

    #[test]
    fn edits_the_scenarios() {
        let sysfs = alpha17("17KKIMS1.115");
        let state = tempfile::tempdir().unwrap();
        let file = || state::File::new(state.path(), "settings.toml");
        // The config's AC takes performance, which has to stay.
        let set = config::Changes {
            profiles: config::Profiles {
                ac: Some(config::Profile {
                    scenario: Some("performance".into()),
                    ..config::Profile::default()
                }),
                battery: None,
            },
            ..config::Changes::default()
        };
        let mut service = service(sysfs.path());
        service.keep_changes(file(), set.clone());
        assert!(service.read_scenarios().unwrap().contains(
            "[[scenarios]]\nname = \"silent\"\nshift_mode = \"eco\"\nfan_mode = \"silent\"\n"
        ));
        let performance = "[[scenarios]]\nname = \"performance\"\nshift_mode = \"turbo\"\n";
        for bad in [
            "[[scenarios]]\nname = \"silent\"\nshift_mode = \"eco\"\n".to_owned(),
            format!("{performance}[[scenarios]]\nname = \"warp\"\nshift_mode = \"warp\"\n"),
            format!("{performance}{performance}"),
            format!("{performance}[[scenarios]]\nname = \" \"\n"),
        ] {
            assert!(service.set_scenarios(&bad).is_err(), "{bad}");
        }

        // The window's battery loses its scenario: nothing again.
        service.set_source_scenario("battery", "silent").unwrap();
        let quiet =
            "[[scenarios]]\nname = \"quiet\"\nshift_mode = \"eco\"\nfan_mode = \"silent\"\n";
        service
            .set_scenarios(&format!("{performance}{quiet}"))
            .unwrap();
        let status = service.status();
        assert_eq!(status.scenarios.unwrap(), ["performance", "quiet"]);
        assert_eq!(status.battery_scenario.as_deref(), Some(""));

        // Kept for the next start.
        let mut next = self::service(sysfs.path());
        next.keep_changes(file(), set);
        assert_eq!(next.status().scenarios.unwrap(), ["performance", "quiet"]);
    }

    #[test]
    fn camera_and_fn_key() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        // The camera bit does nothing on this firmware: not offered.
        assert!(service.set_webcam(false).is_err());
        service.set_fn_key("left").unwrap();
        let status = service.status();
        assert_eq!(status.webcam, None);
        assert_eq!(status.fn_key.as_deref(), Some("left"));
        assert!(service.set_fn_key("middle").is_err());
        assert_eq!(service.ec.fn_key().unwrap(), "left");
    }

    #[test]
    fn usb_share_where_the_map_has_it() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut alpha = service(sysfs.path());
        assert_eq!(alpha.status().usb_share, Some(false));
        alpha.set_usb_share(true).unwrap();
        assert_eq!(ram(sysfs.path())[0xbf], 0x28);
        assert_eq!(alpha.status().usb_share, Some(true));

        let elsewhere = alpha17("17KKIMS1.114");
        let mut other = service(elsewhere.path());
        assert_eq!(other.status().usb_share, None);
        assert!(other.set_usb_share(true).is_err());
        assert_eq!(ram(elsewhere.path())[0xbf], 0x08);
    }

    /// The Titan 18 HX with msi-ec reading `shift_mode` as `msi_ec`, its
    /// byte at `d2`, on AC or not. msi-ec can't be made to follow the byte
    /// here, so each test starts where its own direction can be read back.
    fn titan18hx(msi_ec: &str, d2: u8, ac: bool) -> tempfile::TempDir {
        let sysfs = alpha17("1824EMS1.509");
        let device = sysfs.path().join("devices/platform/msi-ec");
        fs::write(device.join("shift_mode"), format!("{msi_ec}\n")).unwrap();
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        let mut bytes = fs::read(&io).unwrap();
        bytes[0xd2] = d2;
        fs::write(&io, bytes).unwrap();
        plug(sysfs.path(), ac);
        sysfs
    }

    fn plug(sysfs: &Path, ac: bool) {
        let adapter = sysfs.join("class/power_supply/ADP1");
        fs::create_dir_all(&adapter).unwrap();
        fs::write(adapter.join("type"), "Mains\n").unwrap();
        fs::write(adapter.join("online"), if ac { "1\n" } else { "0\n" }).unwrap();
    }

    #[test]
    fn apex_where_the_map_has_it_and_on_ac() {
        let sysfs = titan18hx("unknown (196)", 0xc4, true);
        let mut titan = service(sysfs.path());
        let modes = titan.status().shift_modes.unwrap();
        assert_eq!(modes, ["eco", "comfort", "turbo", "apex"]);
        titan.set_shift_mode("apex").unwrap();
        assert_eq!(ram(sysfs.path())[0xd2], 0xc5);
        assert_eq!(titan.status().shift_mode.as_deref(), Some("apex"));

        let unplugged = titan18hx("unknown (196)", 0xc4, false);
        let mut titan = service(unplugged.path());
        assert!(titan.set_shift_mode("apex").is_err());
        assert_eq!(ram(unplugged.path())[0xd2], 0xc4);

        let elsewhere = alpha17("17KKIMS1.115");
        plug(elsewhere.path(), true);
        let mut alpha = service(elsewhere.path());
        assert!(!alpha.status().shift_modes.unwrap().contains(&"apex".into()));
        assert!(alpha.set_shift_mode("apex").is_err());
    }

    #[test]
    fn turbo_is_apex_on_ac_when_asked() {
        let sysfs = titan18hx("turbo", 0xc4, true);
        let mut titan = service(sysfs.path());
        assert_eq!(titan.on_source("turbo"), "turbo");
        assert_eq!(titan.on_source("apex"), "apex");
        titan.put_apex_on_ac(true);
        assert_eq!(titan.on_source("turbo"), "apex");
        assert_eq!(titan.on_source("comfort"), "comfort");
        let performance = Scenario {
            name: "performance".into(),
            shift_mode: Some("turbo".into()),
            fan_mode: None,
            super_battery: None,
        };
        assert_eq!(
            titan.fit(performance.modes()).shift_mode.as_deref(),
            Some("apex")
        );

        plug(sysfs.path(), false);
        assert_eq!(titan.on_source("turbo"), "turbo");
        assert_eq!(titan.on_source("apex"), "turbo");

        let elsewhere = alpha17("17KKIMS1.115");
        plug(elsewhere.path(), true);
        let mut alpha = service(elsewhere.path());
        alpha.put_apex_on_ac(true);
        assert_eq!(alpha.on_source("turbo"), "turbo");
    }

    #[test]
    fn apex_goes_with_ac() {
        let sysfs = titan18hx("unknown (197)", 0xc5, true);
        let mut titan = service(sysfs.path());
        titan.put_apex_on_ac(true);
        titan.follow_source().unwrap();
        assert_eq!(titan.status().shift_mode.as_deref(), Some("apex"));
        assert_eq!(titan.follow_power_profile("performance").unwrap(), None);

        plug(sysfs.path(), false);
        titan.follow_source().unwrap();
        assert_eq!(titan.status().shift_mode.as_deref(), Some("turbo"));
    }

    #[test]
    fn performance_off_ac_is_turbo() {
        let sysfs = titan18hx("unknown (197)", 0xc5, false);
        let mut titan = service(sysfs.path());
        titan.put_apex_on_ac(true);
        assert_eq!(
            titan
                .follow_power_profile("performance")
                .unwrap()
                .as_deref(),
            Some("scenario performance")
        );
        assert_eq!(titan.status().shift_mode.as_deref(), Some("turbo"));
    }

    /// The BIOS behind the EC, as far as the MUX goes: answers each ask in
    /// 0xD1 by flipping the mode it keeps, as its SMM does, until `done`.
    fn bios(sysfs: &Path) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
        let ec = RawEc::new(sysfs);
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let bios = thread::spawn(move || {
            let mut discrete = false;
            while !stop.load(Ordering::Relaxed) {
                let mut switch = [0];
                ec.read(0xd1, &mut switch).unwrap();
                if switch[0] & 1 != 0 {
                    discrete = !discrete;
                    let to = if discrete { 0x04 } else { 0 };
                    ec.write(0xd1, &[switch[0] & !0x07 | 0x02 | to]).unwrap();
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        (done, bios)
    }

    fn gpu_modes(service: &Service) -> (Option<String>, Option<String>) {
        let status = service.status();
        (status.gpu_mode, status.gpu_mode_next)
    }

    #[tokio::test]
    async fn gpu_mode_from_the_next_restart() {
        let sysfs = alpha17("17KKIMS1.115");
        let mut service = service(sysfs.path());
        let both = |now: &str, next: &str| (Some(now.to_owned()), Some(next.to_owned()));
        assert_eq!(gpu_modes(&service), both("hybrid", "hybrid"));

        let (done, bios) = bios(sysfs.path());
        service.set_gpu_mode("discrete").await.unwrap();
        assert_eq!(gpu_modes(&service), both("hybrid", "discrete"));
        // Already coming: asking again would flip it back.
        service.set_gpu_mode("discrete").await.unwrap();
        assert_eq!(gpu_modes(&service), both("hybrid", "discrete"));
        service.set_gpu_mode("hybrid").await.unwrap();
        assert_eq!(gpu_modes(&service), both("hybrid", "hybrid"));
        done.store(true, Ordering::Relaxed);
        bios.join().unwrap();

        assert!(service.set_gpu_mode("both").await.is_err());
        // Where the panel is, only the EC writes.
        assert_eq!(ram(sysfs.path())[0x2e], 0x0b);
    }

    #[tokio::test(start_paused = true)]
    async fn gpu_mode_needs_an_answer() {
        let sysfs = alpha17("17KKIMS1.115");
        let err = service(sysfs.path())
            .set_gpu_mode("discrete")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("didn't pass"), "{err}");

        let elsewhere = alpha17("17KKIMS1.114");
        let mut other = service(elsewhere.path());
        assert_eq!(gpu_modes(&other), (None, None));
        assert!(other.set_gpu_mode("discrete").await.is_err());
        assert_eq!(ram(elsewhere.path())[0xd1], 0);
    }
}
