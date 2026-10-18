//! What ryukazed keeps between starts in its state directory, a TOML file
//! each: the keyboard light's, and the settings changed in the window.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use tracing::warn;

pub struct File {
    path: PathBuf,
}

impl File {
    pub fn new(dir: &Path, name: &str) -> Self {
        Self {
            path: dir.join(name),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What's kept: nothing without the file, and nothing of a file gone
    /// bad, with a warning.
    pub fn load<T: DeserializeOwned + Default>(&self) -> T {
        let path = self.path.display();
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return T::default(),
            Err(err) => {
                warn!("{path}: {err}");
                return T::default();
            }
        };
        toml::from_str(&text).unwrap_or_else(|err| {
            warn!("{path}: {err}");
            T::default()
        })
    }

    /// Written whole to a file beside it first, so that a crash midway
    /// leaves the old one.
    pub fn store<T: Serialize>(&self, value: &T) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(value).map_err(io::Error::other)?;
        let new = self.path.with_extension("toml.new");
        fs::write(&new, text)?;
        fs::rename(&new, &self.path)
    }
}
