//! Whether the laptop runs on AC, and its battery, from
//! /sys/class/power_supply.

use std::{fmt, fs, io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Ac,
    Battery,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ac => "ac",
            Self::Battery => "battery",
        })
    }
}

/// AC when any Mains supply is online. `None` while no Mains supply is
/// registered yet (ac.ko not loaded), since that can't tell either way.
pub fn source(sysfs: &Path) -> io::Result<Option<Source>> {
    let mut seen = false;
    for entry in fs::read_dir(sysfs.join("class/power_supply"))? {
        let supply = entry?.path();
        let Ok(kind) = fs::read_to_string(supply.join("type")) else {
            continue;
        };
        if kind.trim_end() != "Mains" {
            continue;
        }
        let Ok(online) = fs::read_to_string(supply.join("online")) else {
            continue;
        };
        seen = true;
        if online.trim_end() == "1" {
            return Ok(Some(Source::Ac));
        }
    }
    Ok(seen.then_some(Source::Battery))
}

/// The battery as the kernel has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// Percent.
    pub level: u8,
    /// `Charging`, `Discharging`, `Not charging`, `Full` or `Unknown`.
    pub state: String,
    /// Watts going in or out, where the battery says.
    pub watts: Option<f64>,
    /// Its full charge now against the design's, percent.
    pub health: Option<u8>,
}

/// The first battery; `None` without one.
pub fn battery(sysfs: &Path) -> io::Result<Option<Battery>> {
    for entry in fs::read_dir(sysfs.join("class/power_supply"))? {
        let supply = entry?.path();
        let text = |attr: &str| {
            fs::read_to_string(supply.join(attr)).map(|text| text.trim_end().to_owned())
        };
        let number = |attr: &str| text(attr).ok()?.parse::<f64>().ok();
        if text("type").ok().as_deref() != Some("Battery") {
            continue;
        }
        let level = text("capacity")?;
        let level = level.parse().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, format!("capacity {level:?}"))
        })?;
        // µW, or µA times µV.
        let watts = number("power_now")
            .map(|power| power / 1e6)
            .or_else(|| Some(number("current_now")? * number("voltage_now")? / 1e12));
        let health = [
            ("charge_full", "charge_full_design"),
            ("energy_full", "energy_full_design"),
        ]
        .into_iter()
        .find_map(|(full, design)| {
            let design = number(design).filter(|&design| design > 0.0)?;
            Some((number(full)? / design * 100.0).round().min(255.0) as u8)
        });
        return Ok(Some(Battery {
            level,
            state: text("status")?,
            watts: watts.map(f64::abs),
            health,
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(sysfs: &Path, name: &str, kind: &str, online: Option<&str>) {
        let dir = sysfs.join("class/power_supply").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        if let Some(online) = online {
            fs::write(dir.join("online"), format!("{online}\n")).unwrap();
        }
    }

    #[test]
    fn follows_the_adapter() {
        let sysfs = tempfile::tempdir().unwrap();
        supply(sysfs.path(), "BAT1", "Battery", None);
        assert_eq!(source(sysfs.path()).unwrap(), None);

        supply(sysfs.path(), "ADP1", "Mains", Some("0"));
        assert_eq!(source(sysfs.path()).unwrap(), Some(Source::Battery));

        supply(sysfs.path(), "ADP1", "Mains", Some("1"));
        assert_eq!(source(sysfs.path()).unwrap(), Some(Source::Ac));
    }

    #[test]
    fn reads_the_battery() {
        let sysfs = tempfile::tempdir().unwrap();
        assert_eq!(battery(sysfs.path()).ok().flatten(), None);

        supply(sysfs.path(), "ADP1", "Mains", Some("1"));
        let bat = sysfs.path().join("class/power_supply/BAT1");
        supply(sysfs.path(), "BAT1", "Battery", None);
        // As the Alpha 17's reads: charge in µAh, no power_now.
        for (attr, text) in [
            ("capacity", "91"),
            ("status", "Discharging"),
            ("current_now", "1100000"),
            ("voltage_now", "16822000"),
            ("charge_full", "5998000"),
            ("charge_full_design", "6250000"),
        ] {
            fs::write(bat.join(attr), format!("{text}\n")).unwrap();
        }
        let battery = battery(sysfs.path()).unwrap().unwrap();
        assert_eq!(battery.level, 91);
        assert_eq!(battery.state, "Discharging");
        assert!((battery.watts.unwrap() - 18.5).abs() < 0.01);
        assert_eq!(battery.health, Some(96));
    }
}
