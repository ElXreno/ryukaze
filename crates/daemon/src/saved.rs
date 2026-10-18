//! The keyboard light as it was left over D-Bus: the profile picked last,
//! and the profiles saved by the window's editor, kept in the state
//! directory's keyboard.toml (`picked`, and `[[profiles]]` the way the
//! config has them) and read back as the daemon starts.

use std::{io, path::Path};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{config::KeyboardProfile, state};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Kept {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<String>,
    #[serde(default)]
    pub profiles: Vec<KeyboardProfile>,
}

pub struct Store {
    file: state::File,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self {
            file: state::File::new(dir, "keyboard.toml"),
        }
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// What was kept: nothing without the file, and of a file gone bad what
    /// still holds, with a warning for the rest.
    pub fn load(&self) -> Kept {
        let path = self.path().display();
        let read: Kept = self.file.load();
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
        Kept {
            picked: read.picked,
            profiles,
        }
    }

    pub fn store(&self, kept: &Kept) -> io::Result<()> {
        self.file.store(kept)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn keeps_profiles_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(&dir.path().join("state"));
        // Nothing kept yet.
        let read = store.load();
        assert!(read.picked.is_none() && read.profiles.is_empty());

        let profiles: Vec<KeyboardProfile> = [
            "name = \"orange\"\ncolor = \"#ff6000\"\nkeys = { wasd = \"#ff0000\" }\n",
            "name = \"split\"\neffect = \"wave\"\n[[zones]]\nkeys = [\"numpad\"]\neffect = \"breathe\"\ncolor = \"#0040ff\"\n",
        ]
        .iter()
        .map(|text| toml::from_str(text).unwrap())
        .collect();
        let kept = Kept {
            picked: Some("wave".into()),
            profiles: profiles.clone(),
        };
        store.store(&kept).unwrap();
        let read = store.load();
        assert_eq!(read.picked.as_deref(), Some("wave"));
        let names: Vec<&str> = read.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["orange", "split"]);
        assert_eq!(read.profiles[1].lighting(), profiles[1].lighting());
        assert!(!store.path().with_extension("toml.new").exists());

        // Gone bad: the good ones still, the others warned of.
        fs::write(
            store.path(),
            "[[profiles]]\nname = \"ok\"\ncolor = \"#ffffff\"\n\n[[profiles]]\nname = \"bad\"\neffect = \"breathe\"\n\n[[profiles]]\nname = \"ok\"\ncolor = \"#000000\"\n",
        )
        .unwrap();
        let read = store.load();
        assert_eq!(read.profiles.len(), 1);
        assert_eq!(read.profiles[0].name, "ok");
        fs::write(store.path(), "not toml [").unwrap();
        assert!(store.load().profiles.is_empty());
    }
}
