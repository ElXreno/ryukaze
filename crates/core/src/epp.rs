//! The CPU's energy-performance preference, in cpufreq: the hint
//! intel_pstate and amd-pstate take for how eagerly a core clocks up.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// As the config writes it: a name the driver lists, or with intel_pstate
/// a number, 0 for all performance to 255 for all saving.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Epp {
    Raw(u8),
    Named(String),
}

impl fmt::Display for Epp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw(raw) => write!(f, "{raw}"),
            Self::Named(name) => f.write_str(name),
        }
    }
}

impl Epp {
    /// As typed: a number where it is one, nothing where it's blank.
    pub fn typed(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        Some(
            text.parse()
                .map_or_else(|_| Self::Named(text.into()), Self::Raw),
        )
    }
}

/// Each policy's file; `NotFound` where the driver has no preference.
fn files(sysfs: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(sysfs.join("devices/system/cpu/cpufreq"))? {
        let file = entry?.path().join("energy_performance_preference");
        if file.exists() {
            files.push(file);
        }
    }
    if files.is_empty() {
        return Err(io::ErrorKind::NotFound.into());
    }
    files.sort();
    Ok(files)
}

fn read(file: &Path) -> io::Result<String> {
    Ok(fs::read_to_string(file)?.trim_end().to_owned())
}

/// The preference every policy is at; `None` while they differ.
pub fn get(sysfs: &Path) -> io::Result<Option<String>> {
    let mut common = None;
    for file in files(sysfs)? {
        let epp = read(&file)?;
        match &common {
            None => common = Some(epp),
            Some(first) if *first == epp => {}
            Some(_) => return Ok(None),
        }
    }
    Ok(common)
}

/// Puts every policy at `epp`; one already there stays unwritten.
pub fn set(sysfs: &Path, epp: &str) -> io::Result<()> {
    for file in files(sysfs)? {
        if read(&file)? != epp {
            fs::write(&file, epp)?;
        }
    }
    Ok(())
}

/// The names the driver takes.
pub fn names(sysfs: &Path) -> io::Result<Vec<String>> {
    let file = files(sysfs)?.swap_remove(0);
    let names =
        fs::read_to_string(file.with_file_name("energy_performance_available_preferences"))?;
    Ok(names.split_whitespace().map(str::to_owned).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpufreq(policies: &[&str]) -> tempfile::TempDir {
        let sysfs = tempfile::tempdir().unwrap();
        for (i, epp) in policies.iter().enumerate() {
            let policy = sysfs
                .path()
                .join(format!("devices/system/cpu/cpufreq/policy{i}"));
            fs::create_dir_all(&policy).unwrap();
            fs::write(
                policy.join("energy_performance_preference"),
                format!("{epp}\n"),
            )
            .unwrap();
            fs::write(
                policy.join("energy_performance_available_preferences"),
                "default performance balance_performance balance_power power \n",
            )
            .unwrap();
        }
        sysfs
    }

    #[test]
    fn reads_what_every_policy_is_at() {
        let sysfs = cpufreq(&["performance", "performance"]);
        assert_eq!(get(sysfs.path()).unwrap().as_deref(), Some("performance"));
        let mixed = cpufreq(&["performance", "132"]);
        assert_eq!(get(mixed.path()).unwrap(), None);
    }

    #[test]
    fn writes_every_policy() {
        let sysfs = cpufreq(&["performance", "balance_power"]);
        set(sysfs.path(), "132").unwrap();
        assert_eq!(get(sysfs.path()).unwrap().as_deref(), Some("132"));
    }

    #[test]
    fn has_none_without_the_file() {
        let sysfs = tempfile::tempdir().unwrap();
        fs::create_dir_all(sysfs.path().join("devices/system/cpu/cpufreq/policy0")).unwrap();
        assert_eq!(
            get(sysfs.path()).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let bare = tempfile::tempdir().unwrap();
        assert_eq!(
            get(bare.path()).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn lists_the_names() {
        let sysfs = cpufreq(&["performance"]);
        assert_eq!(names(sysfs.path()).unwrap().len(), 5);
    }

    #[test]
    fn takes_a_number_or_a_name() {
        #[derive(Deserialize, Serialize)]
        struct File {
            epp: Epp,
        }
        let raw: File = toml::from_str("epp = 132\n").unwrap();
        assert_eq!(raw.epp, Epp::Raw(132));
        assert_eq!(toml::to_string(&raw).unwrap(), "epp = 132\n");
        let named: File = toml::from_str("epp = \"balance_power\"\n").unwrap();
        assert_eq!(named.epp.to_string(), "balance_power");
        assert_eq!(Epp::typed(" 132 "), Some(Epp::Raw(132)));
        assert_eq!(Epp::typed("power"), Some(Epp::Named("power".into())));
        assert_eq!(Epp::typed(" "), None);
    }
}
