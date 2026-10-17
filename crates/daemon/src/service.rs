use std::{
    fmt::Display,
    io,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use ryukaze_core::{
    COOLER_BOOST_MARKS, Status,
    fans::{self, Curve, Fan, Layout},
    gpu_mux::{self, GpuMode, GpuMux},
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
};

pub struct Service {
    ec: MsiEc,
    fans: Option<Fans>,
    boost: AutoBoost,
    /// USB Power Share, where the EC's map has it.
    usb_share: Option<(RawEc, UsbShare)>,
    /// The GPU MUX, where the EC's map has it.
    gpu_mux: Option<(RawEc, GpuMux)>,
    /// The configured scenarios, or the built-in ones.
    scenarios: Vec<Scenario>,
    /// Told by the setters that may change the modes, so a change is
    /// announced at once rather than at the next tick.
    poke: Arc<Notify>,
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
    [
        ("silent", "eco", "silent"),
        ("balanced", "comfort", "auto"),
        ("performance", "turbo", "auto"),
    ]
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
            scenarios,
            poke: Arc::new(Notify::new()),
        }
    }

    /// The modes `profile` sets: super battery left before the shift mode
    /// goes up, and entered after it comes down. All are tried; the first
    /// failure is returned and the others logged.
    pub fn apply_profile(&mut self, profile: &Profile) -> fdo::Result<()> {
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

    /// What `ModesChanged` carries: the four modes and the scenario they
    /// make.
    pub fn modes(&self) -> Status {
        let now = self.snapshot();
        let scenario = self.scenario(&now);
        Status {
            shift_mode: now.shift_mode,
            fan_mode: now.fan_mode,
            super_battery: now.super_battery,
            cooler_boost: now.cooler_boost,
            scenario,
            ..Status::default()
        }
    }

    fn scenario(&self, now: &Snapshot) -> Option<String> {
        self.scenarios
            .iter()
            .find(|scenario| now.has(&scenario.modes()))
            .map(|scenario| scenario.name.clone())
    }

    /// Every couple of seconds: the adaptive fans and the automatic boost.
    pub fn tick(&mut self) -> fdo::Result<()> {
        let adapted = self.adapt();
        self.auto_boost();
        adapted
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
            shift_mode: self.ec.shift_mode().ok(),
            fan_mode: self.ec.fan_mode().ok(),
            super_battery: self.ec.super_battery().ok(),
            cooler_boost: self.ec.cooler_boost().ok(),
        }
    }

    /// Puts back what was there before sleep; what's unchanged stays
    /// unwritten. Failures go to the journal.
    pub fn restore(&mut self, snapshot: &Snapshot) {
        let results = [
            snapshot
                .shift_mode
                .as_deref()
                .map(|mode| self.set_shift_mode(mode)),
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
        let now = Snapshot {
            shift_mode: field("shift_mode", ec.shift_mode()),
            fan_mode: field("fan_mode", ec.fan_mode()),
            super_battery: field("super_battery", ec.super_battery()),
            cooler_boost: field("cooler_boost", ec.cooler_boost()),
        };
        let scenario = self.scenario(&now);
        let scenarios = (!self.scenarios.is_empty())
            .then(|| self.scenarios.iter().map(|s| s.name.clone()).collect());
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
            shift_modes: field("available_shift_modes", ec.shift_modes()),
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
            usb_share: self
                .usb_share
                .as_ref()
                .and_then(|(raw, share)| share.get(raw).ok()),
            gpu_mode: mux(GpuMux::now),
            gpu_mode_next: mux(GpuMux::next),
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
            .position(|scenario| now.has(&scenario.modes()))
            .map_or(0, |i| (i + 1) % self.scenarios.len());
        let scenario = self.scenarios[next].clone();
        info!("scenario: {}", scenario.name);
        self.apply_profile(&scenario.modes())?;
        Ok(scenario.name)
    }

    /// One of the shift modes in Status.
    pub fn set_shift_mode(&mut self, mode: &str) -> fdo::Result<()> {
        self.poke.notify_one();
        let modes = self.ec.shift_modes().map_err(|e| error("shift_mode", e))?;
        check_mode(mode, &modes)?;
        apply(
            "shift_mode",
            mode.to_owned(),
            || self.ec.shift_mode(),
            |_| self.ec.set_shift_mode(mode),
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

    /// The curve advanced runs `fan` ("cpu" or "gpu") on, until the daemon
    /// restarts; see `fans::Curve`. In advanced it's on the fan at once.
    pub fn set_fan_curve(&mut self, fan: &str, temps: Vec<u8>, speeds: Vec<u8>) -> fdo::Result<()> {
        let fan = parse_fan(fan)?;
        let curve = Curve::new(&temps, &speeds).map_err(fdo::Error::InvalidArgs)?;
        let fans = self.fans.as_mut().ok_or_else(no_curves)?;
        fans.active[fan as usize] = curve;
        fans.adaptive.targets[fan as usize] = None;
        self.adapt()?;
        self.sync_curves()
    }

    /// Back to the configured curve, or the EC's own where none is.
    pub fn reset_fan_curve(&mut self, fan: &str) -> fdo::Result<()> {
        let fan = parse_fan(fan)?;
        self.fans.as_mut().ok_or_else(no_curves)?.reset(fan);
        self.adapt()?;
        self.sync_curves()
    }

    /// Whether advanced is adaptive, until the daemon restarts; off, the EC
    /// steps through the curves itself.
    pub fn set_adaptive_fans(&mut self, on: bool) -> fdo::Result<()> {
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

    /// While adaptive: whether the GPU fan runs at least as fast as the
    /// CPU's curve asks, until the daemon restarts.
    pub fn set_gpu_fan_follows_cpu(&mut self, on: bool) -> fdo::Result<()> {
        let fans = self.fans.as_mut().ok_or_else(no_curves)?;
        if fans.adaptive.gpu_follows_cpu == on {
            return Ok(());
        }
        fans.adaptive.gpu_follows_cpu = on;
        fans.adaptive.targets[Fan::Gpu as usize] = None;
        info!("gpu fan follows the cpu: {}", if on { "on" } else { "off" });
        self.adapt()
    }

    pub fn set_webcam(&mut self, on: bool) -> fdo::Result<()> {
        apply(
            "webcam",
            on,
            || self.ec.webcam(),
            |&on| self.ec.set_webcam(on),
        )
    }

    /// `left` or `right` of the Win key.
    pub fn set_fn_key(&mut self, side: &str) -> fdo::Result<()> {
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

    /// USB ports powered while the laptop is off.
    pub fn set_usb_share(&mut self, on: bool) -> fdo::Result<()> {
        let (raw, share) = self.usb_share.as_ref().ok_or_else(|| {
            fdo::Error::NotSupported("usb_share: this EC's map doesn't have it".into())
        })?;
        apply("usb_share", on, || share.get(raw), |&on| share.set(raw, on))
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

    /// Cooler boost by itself when it gets hot, until the daemon restarts.
    /// Switched off, a boost it started ends.
    pub fn set_auto_cooler_boost(&mut self, on: bool) -> fdo::Result<()> {
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

    /// The CPU and GPU temperatures, °C, cooler boost comes on at by
    /// itself, until the daemon restarts.
    pub fn set_cooler_boost_marks(&mut self, cpu_c: u8, gpu_c: u8) -> fdo::Result<()> {
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

    /// 10 to 100 percent; charging resumes 10 points below it.
    pub fn set_charge_limit(&mut self, percent: u8) -> fdo::Result<()> {
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
        Service::new(ec, fans, AutoBoost::default(), Vec::new())
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
