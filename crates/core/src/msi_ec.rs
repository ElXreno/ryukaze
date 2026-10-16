//! The sysfs attributes of the upstream msi-ec driver
//! (github.com/BeardOverflow/msi-ec). Which ones exist depends on the EC
//! firmware; a missing one reads and writes as `ErrorKind::NotFound`.
//! Writing takes root; the driver rejects values it doesn't know.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub struct MsiEc {
    device: PathBuf,
    power_supply: PathBuf,
}

impl MsiEc {
    /// `sysfs` is `/sys`, or a fake tree for tests and `ryukazed --sysfs`.
    pub fn new(sysfs: &Path) -> Self {
        Self {
            device: sysfs.join("devices/platform/msi-ec"),
            power_supply: sysfs.join("class/power_supply"),
        }
    }

    /// Whether the driver is bound: it isn't when it rejected this firmware.
    pub fn is_bound(&self) -> bool {
        self.device.join("fw_version").exists()
    }

    pub fn firmware(&self) -> io::Result<String> {
        self.read("fw_version")
    }

    pub fn firmware_date(&self) -> io::Result<String> {
        self.read("fw_release_date")
    }

    pub fn shift_mode(&self) -> io::Result<String> {
        self.read("shift_mode")
    }

    /// One of `shift_modes`.
    pub fn set_shift_mode(&self, mode: &str) -> io::Result<()> {
        write(&self.device.join("shift_mode"), mode)
    }

    pub fn shift_modes(&self) -> io::Result<Vec<String>> {
        self.read_list("available_shift_modes")
    }

    pub fn fan_mode(&self) -> io::Result<String> {
        self.read("fan_mode")
    }

    /// One of `fan_modes`.
    pub fn set_fan_mode(&self, mode: &str) -> io::Result<()> {
        write(&self.device.join("fan_mode"), mode)
    }

    pub fn fan_modes(&self) -> io::Result<Vec<String>> {
        self.read_list("available_fan_modes")
    }

    pub fn super_battery(&self) -> io::Result<bool> {
        self.read_on_off("super_battery")
    }

    pub fn set_super_battery(&self, on: bool) -> io::Result<()> {
        write(&self.device.join("super_battery"), on_off(on))
    }

    pub fn cooler_boost(&self) -> io::Result<bool> {
        self.read_on_off("cooler_boost")
    }

    pub fn set_cooler_boost(&self, on: bool) -> io::Result<()> {
        write(&self.device.join("cooler_boost"), on_off(on))
    }

    /// The charge end threshold msi-ec adds to the battery; charging
    /// resumes 10 points below it.
    pub fn charge_limit(&self) -> io::Result<u8> {
        let path = self.charge_limit_path()?;
        let text = fs::read_to_string(&path)?;
        text.trim().parse().map_err(|_| invalid(&path, &text))
    }

    /// 10 to 100 percent. The EC keeps it across reboots.
    pub fn set_charge_limit(&self, percent: u8) -> io::Result<()> {
        write(&self.charge_limit_path()?, &percent.to_string())
    }

    fn charge_limit_path(&self) -> io::Result<PathBuf> {
        for entry in fs::read_dir(&self.power_supply)? {
            let path = entry?.path().join("charge_control_end_threshold");
            if path.exists() {
                return Ok(path);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no battery has charge_control_end_threshold",
        ))
    }

    fn read(&self, attr: &str) -> io::Result<String> {
        let text = fs::read_to_string(self.device.join(attr))?;
        Ok(text.trim_end().to_owned())
    }

    fn read_list(&self, attr: &str) -> io::Result<Vec<String>> {
        Ok(self.read(attr)?.lines().map(str::to_owned).collect())
    }

    fn read_on_off(&self, attr: &str) -> io::Result<bool> {
        match self.read(attr)?.as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            text => Err(invalid(&self.device.join(attr), text)),
        }
    }
}

/// One write call, and no O_CREAT: a missing attribute stays NotFound,
/// including in a fake tree. Sysfs ignores O_TRUNC; a fake tree needs it.
fn write(path: &Path, value: &str) -> io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?
        .write_all(value.as_bytes())
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn invalid(path: &Path, text: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}: unexpected {:?}", path.display(), text.trim_end()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// msi-ec's attributes as read on an Alpha 17 C7VG.
    fn alpha17() -> tempfile::TempDir {
        let sysfs = tempfile::tempdir().unwrap();
        let device = sysfs.path().join("devices/platform/msi-ec");
        let battery = sysfs.path().join("class/power_supply/BAT1");
        fs::create_dir_all(&device).unwrap();
        fs::create_dir_all(&battery).unwrap();
        for (attr, text) in [
            ("fw_version", "17KKIMS1.115\n"),
            ("fw_release_date", "2026-01-15T17:29:02\n"),
            ("shift_mode", "turbo\n"),
            ("available_shift_modes", "turbo\neco\ncomfort\n"),
            ("fan_mode", "auto\n"),
            ("available_fan_modes", "auto\nsilent\nadvanced\n"),
            ("super_battery", "off\n"),
            ("cooler_boost", "on\n"),
        ] {
            fs::write(device.join(attr), text).unwrap();
        }
        fs::write(battery.join("charge_control_end_threshold"), "80\n").unwrap();
        sysfs
    }

    #[test]
    fn reads_attributes() {
        let sysfs = alpha17();
        let ec = MsiEc::new(sysfs.path());
        assert!(ec.is_bound());
        assert_eq!(ec.firmware().unwrap(), "17KKIMS1.115");
        assert_eq!(ec.shift_mode().unwrap(), "turbo");
        assert_eq!(ec.shift_modes().unwrap(), ["turbo", "eco", "comfort"]);
        assert_eq!(ec.fan_modes().unwrap(), ["auto", "silent", "advanced"]);
        assert!(!ec.super_battery().unwrap());
        assert!(ec.cooler_boost().unwrap());
        assert_eq!(ec.charge_limit().unwrap(), 80);
    }

    #[test]
    fn missing_is_not_found() {
        let sysfs = tempfile::tempdir().unwrap();
        let ec = MsiEc::new(sysfs.path());
        assert!(!ec.is_bound());
        assert_eq!(ec.shift_mode().unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(
            ec.charge_limit().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn rejects_garbage() {
        let sysfs = alpha17();
        let device = sysfs.path().join("devices/platform/msi-ec");
        fs::write(device.join("cooler_boost"), "maybe\n").unwrap();
        let ec = MsiEc::new(sysfs.path());
        assert_eq!(
            ec.cooler_boost().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn writes_attributes() {
        let sysfs = alpha17();
        let ec = MsiEc::new(sysfs.path());
        ec.set_shift_mode("eco").unwrap();
        ec.set_cooler_boost(false).unwrap();
        ec.set_charge_limit(60).unwrap();
        assert_eq!(ec.shift_mode().unwrap(), "eco");
        assert!(!ec.cooler_boost().unwrap());
        assert_eq!(ec.charge_limit().unwrap(), 60);
    }

    #[test]
    fn writing_missing_is_not_found() {
        let sysfs = alpha17();
        let device = sysfs.path().join("devices/platform/msi-ec");
        fs::remove_file(device.join("super_battery")).unwrap();
        let ec = MsiEc::new(sysfs.path());
        let err = ec.set_super_battery(true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!device.join("super_battery").exists());
    }
}
