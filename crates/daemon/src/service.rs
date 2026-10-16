use std::{fmt::Display, io, path::Path};

use ryukaze_core::{
    Status,
    fans::{self, Fan, Layout},
    msi_ec::MsiEc,
    raw_ec::RawEc,
};
use tracing::{info, warn};
use zbus::fdo;

use crate::config;

pub struct Service {
    ec: MsiEc,
    fans: Option<Fans>,
}

/// Fan curves and tachometers, where the EC's layout is known and its RAM
/// can be read.
pub struct Fans {
    raw: RawEc,
    layout: &'static Layout,
    /// The configured curves, for the advanced fan mode.
    curves: config::Fans,
}

impl Fans {
    /// `None`, with the reason in the journal, when this EC can't have them.
    pub fn open(sysfs: &Path, firmware: Option<&str>, curves: config::Fans) -> Option<Self> {
        let Some(layout) = firmware.and_then(fans::layout) else {
            if !curves.is_empty() {
                warn!("fan curves: this EC firmware's layout isn't known, leaving the fans alone");
            }
            return None;
        };
        let raw = RawEc::new(sysfs);
        if let Err(err) = layout.rpm(&raw, Fan::Cpu) {
            warn!("fan curves and rpm: EC RAM unreadable (ec_sys loaded? root?): {err}");
            return None;
        }
        Some(Self {
            raw,
            layout,
            curves,
        })
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

impl Service {
    pub fn new(ec: MsiEc, fans: Option<Fans>) -> Self {
        Self { ec, fans }
    }

    /// The curves that go with the fan mode the EC is in now: wherever the
    /// EC may have reloaded its own (start, msi-ec binding, resume).
    pub fn sync_curves(&self) -> fdo::Result<()> {
        if self.fans.as_ref().is_none_or(|fans| fans.curves.is_empty()) {
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

    /// For each fan with a configured curve: that curve in advanced, the
    /// EC's own otherwise.
    fn put_curves(&self, advanced: bool) -> fdo::Result<()> {
        let Some(fans) = &self.fans else {
            return Ok(());
        };
        for fan in Fan::ALL {
            let Some(configured) = fans.curves.get(fan) else {
                continue;
            };
            let (want, which) = if advanced {
                (configured, "configured")
            } else {
                (fans.layout.factory(fan), "factory")
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
            info!("{name}: {which}");
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
        Status {
            firmware: field("fw_version", ec.firmware()),
            firmware_date: field("fw_release_date", ec.firmware_date()),
            shift_mode: field("shift_mode", ec.shift_mode()),
            shift_modes: field("available_shift_modes", ec.shift_modes()),
            fan_mode: field("fan_mode", ec.fan_mode()),
            fan_modes: field("available_fan_modes", ec.fan_modes()),
            super_battery: field("super_battery", ec.super_battery()),
            cooler_boost: field("cooler_boost", ec.cooler_boost()),
            cpu_temp: field("cpu/realtime_temperature", ec.cpu_temp()),
            gpu_temp: field("gpu/realtime_temperature", ec.gpu_temp()),
            cpu_fan: field("cpu/realtime_fan_speed", ec.cpu_fan()),
            gpu_fan: field("gpu/realtime_fan_speed", ec.gpu_fan()),
            cpu_rpm: rpm(Fan::Cpu),
            gpu_rpm: rpm(Fan::Gpu),
            charge_limit: field("charge_limit", ec.charge_limit()),
        }
    }

    /// One of the shift modes in Status.
    pub fn set_shift_mode(&mut self, mode: &str) -> fdo::Result<()> {
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

    pub fn set_super_battery(&mut self, on: bool) -> fdo::Result<()> {
        apply(
            "super_battery",
            on,
            || self.ec.super_battery(),
            |&on| self.ec.set_super_battery(on),
        )
    }

    pub fn set_cooler_boost(&mut self, on: bool) -> fdo::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use ryukaze_core::fans::Curve;

    use crate::config::FanCurve;

    const QUIET: [u8; 7] = [0, 30, 35, 40, 50, 60, 100];

    /// msi-ec's attributes and the EC's RAM as on the Alpha 17, in auto.
    fn alpha17(firmware: &str) -> tempfile::TempDir {
        let sysfs = tempfile::tempdir().unwrap();
        let device = sysfs.path().join("devices/platform/msi-ec");
        fs::create_dir_all(&device).unwrap();
        for (attr, text) in [
            ("fw_version", firmware),
            ("fan_mode", "auto\n"),
            ("available_fan_modes", "auto\nsilent\nadvanced\n"),
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
        fs::write(io, ram).unwrap();
        sysfs
    }

    fn service(sysfs: &Path) -> Service {
        let curves = config::Fans {
            cpu: Some(FanCurve(
                Curve::new(&[55, 62, 69, 76, 83, 90], &QUIET).unwrap(),
            )),
            gpu: None,
        };
        let ec = MsiEc::new(sysfs);
        let firmware = ec.firmware().ok();
        let fans = Fans::open(sysfs, firmware.as_deref(), curves);
        Service::new(ec, fans)
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
    }
}
