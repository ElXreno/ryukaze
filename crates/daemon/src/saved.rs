//! Keyboard light profiles saved over D-Bus, by the window's editor: kept
//! in the state directory's keyboard-profiles.toml as `[[profiles]]`, the
//! way the config has them, and read back as the daemon starts.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::KeyboardProfile;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    #[serde(default)]
    profiles: Vec<KeyboardProfile>,
}

#[derive(Serialize)]
struct Written<'a> {
    profiles: &'a [KeyboardProfile],
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("keyboard-profiles.toml"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The profiles saved before: none without the file, and of a file gone
    /// bad what still holds, with a warning for the rest.
    pub fn load(&self) -> Vec<KeyboardProfile> {
        let path = self.path.display();
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(err) => {
                warn!("{path}: {err}");
                return Vec::new();
            }
        };
        let read: Read = match toml::from_str(&text) {
            Ok(read) => read,
            Err(err) => {
                warn!("{path}: {err}");
                return Vec::new();
            }
        };
        let mut profiles: Vec<KeyboardProfile> = Vec::new();
        for profile in read.profiles {
            if let Err(err) = profile.lighting() {
                warn!("{path}: {err}");
            } else if profile.name.trim().is_empty() {
                warn!("{path}: a keyboard profile with no name");
            } else if profiles.iter().any(|known| known.name == profile.name) {
                warn!(
                    "{path}: two keyboard profiles are called {:?}",
                    profile.name
                );
            } else {
                profiles.push(profile);
            }
        }
        profiles
    }

    /// All of them, written whole to a file beside it first, so that a
    /// crash midway leaves the old one.
    pub fn store(&self, profiles: &[KeyboardProfile]) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(&Written { profiles }).map_err(io::Error::other)?;
        let new = self.path.with_extension("toml.new");
        fs::write(&new, text)?;
        fs::rename(&new, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_profiles_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(&dir.path().join("state"));
        // Nothing saved yet.
        assert!(store.load().is_empty());

        let profiles: Vec<KeyboardProfile> = [
            "name = \"orange\"\ncolor = \"#ff6000\"\nkeys = { wasd = \"#ff0000\" }\n",
            "name = \"split\"\neffect = \"wave\"\n[[zones]]\nkeys = [\"numpad\"]\neffect = \"breathe\"\ncolor = \"#0040ff\"\n",
        ]
        .iter()
        .map(|text| toml::from_str(text).unwrap())
        .collect();
        store.store(&profiles).unwrap();
        let read = store.load();
        let names: Vec<&str> = read.iter().map(|profile| profile.name.as_str()).collect();
        assert_eq!(names, ["orange", "split"]);
        assert_eq!(read[1].lighting(), profiles[1].lighting());
        assert!(!store.path().with_extension("toml.new").exists());

        // Gone bad: the good ones still, the others warned of.
        fs::write(
            store.path(),
            "[[profiles]]\nname = \"ok\"\ncolor = \"#ffffff\"\n\n[[profiles]]\nname = \"bad\"\neffect = \"breathe\"\n\n[[profiles]]\nname = \"ok\"\ncolor = \"#000000\"\n",
        )
        .unwrap();
        let read = store.load();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "ok");
        fs::write(store.path(), "not toml [").unwrap();
        assert!(store.load().is_empty());
    }
}
