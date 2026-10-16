use std::io;

use ryukaze_core::{Status, msi_ec::MsiEc};
use tracing::warn;

pub struct Service {
    ec: MsiEc,
}

impl Service {
    pub fn new(ec: MsiEc) -> Self {
        Self { ec }
    }
}

/// The name has to be a literal here; clients take it from this file.
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
            charge_limit: field("charge_limit", ec.charge_limit()),
        }
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
