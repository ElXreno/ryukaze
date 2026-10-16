//! Whether the laptop runs on AC, from /sys/class/power_supply.

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
}
