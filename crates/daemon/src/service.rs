use std::{fmt::Display, io};

use ryukaze_core::{Status, msi_ec::MsiEc};
use tracing::{info, warn};
use zbus::fdo;

pub struct Service {
    ec: MsiEc,
}

impl Service {
    pub fn new(ec: MsiEc) -> Self {
        Self { ec }
    }
}

/// The name has to be a literal here; clients take it from this file.
/// Setters take `&mut self`, so zbus runs them one at a time.
#[zbus::interface(name = "io.github.kontonkara.ryukaze1")]
impl Service {
    /// Reads the EC's state afresh on every call.
    fn status(&self) -> Status {
        let ec = &self.ec;
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

    /// One of the fan modes in Status.
    pub fn set_fan_mode(&mut self, mode: &str) -> fdo::Result<()> {
        let modes = self.ec.fan_modes().map_err(|e| error("fan_mode", e))?;
        check_mode(mode, &modes)?;
        apply(
            "fan_mode",
            mode.to_owned(),
            || self.ec.fan_mode(),
            |_| self.ec.set_fan_mode(mode),
        )
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
