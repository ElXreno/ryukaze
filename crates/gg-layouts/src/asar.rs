//! Electron's asar archives: a JSON index of the files after a small
//! header, the files' bytes after the index, and those too big or native
//! enough to be left out in `<archive>.unpacked` beside it.

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// An archive, its index read.
pub struct Archive {
    path: PathBuf,
    /// Where the files' bytes start.
    base: u64,
    files: BTreeMap<String, Entry>,
}

struct Entry {
    offset: u64,
    size: u64,
    unpacked: bool,
}

#[derive(Deserialize)]
struct Node {
    files: Option<BTreeMap<String, Node>>,
    offset: Option<String>,
    size: Option<u64>,
    #[serde(default)]
    unpacked: bool,
}

/// The index can't be larger than this.
const MAX_INDEX: u32 = 256 << 20;

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = fs::File::open(path).with_context(|| format!("{}", path.display()))?;
        let mut head = [0; 16];
        file.read_exact(&mut head)
            .with_context(|| format!("{}: too short for an asar archive", path.display()))?;
        let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        let (index_size, json_len) = (word(4), word(12));
        if word(0) != 4 || json_len > index_size || index_size > MAX_INDEX {
            bail!("{}: not an asar archive", path.display());
        }
        let mut json = vec![0; json_len as usize];
        file.read_exact(&mut json)
            .with_context(|| format!("{}: its index is cut short", path.display()))?;
        let root: Node = serde_json::from_slice(&json)
            .with_context(|| format!("{}: its index doesn't read", path.display()))?;
        let mut files = BTreeMap::new();
        walk(&root, "", &mut files)?;
        Ok(Self {
            path: path.to_owned(),
            base: 8 + u64::from(index_size),
            files,
        })
    }

    /// The files' paths, `/` between the parts.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .files
            .get(name)
            .with_context(|| format!("{}: no {name} in it", self.path.display()))?;
        if entry.unpacked {
            let mut outside = self.path.clone().into_os_string();
            outside.push(".unpacked");
            let outside = PathBuf::from(outside).join(name);
            return fs::read(&outside).with_context(|| format!("{}", outside.display()));
        }
        let mut file = fs::File::open(&self.path)?;
        // Its size checked against the archive's before it's made room for.
        let start = self.base.checked_add(entry.offset);
        let end = start.and_then(|start| start.checked_add(entry.size));
        let (Some(start), Some(end)) = (start, end) else {
            bail!("{}: {name} is out of it", self.path.display());
        };
        if end > file.metadata()?.len() {
            bail!("{}: {name} is cut short", self.path.display());
        }
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![0; usize::try_from(entry.size)?];
        file.read_exact(&mut bytes)
            .with_context(|| format!("{}: {name} is cut short", self.path.display()))?;
        Ok(bytes)
    }
}

fn walk(node: &Node, path: &str, files: &mut BTreeMap<String, Entry>) -> Result<()> {
    let Some(children) = &node.files else {
        return Ok(());
    };
    for (name, child) in children {
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        if child.files.is_some() {
            walk(child, &path, files)?;
            continue;
        }
        // Links and the like: nothing of ours.
        let Some(size) = child.size else {
            continue;
        };
        let offset = match &child.offset {
            Some(offset) => offset
                .parse()
                .with_context(|| format!("{path}: an offset that isn't a number"))?,
            None if child.unpacked => 0,
            None => bail!("{path}: no offset"),
        };
        files.insert(
            path,
            Entry {
                offset,
                size,
                unpacked: child.unpacked,
            },
        );
    }
    Ok(())
}

/// An archive of `files`, as Electron's tools write one.
#[cfg(test)]
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    use serde_json::{Map, Value, json};

    let mut root = Map::new();
    let mut body = Vec::new();
    for (path, bytes) in files {
        let mut parts: Vec<&str> = path.split('/').collect();
        let leaf = parts.pop().unwrap();
        let mut dir = &mut root;
        for part in parts {
            let node = dir
                .entry(part)
                .or_insert_with(|| json!({ "files": {} }))
                .as_object_mut()
                .unwrap();
            dir = node["files"].as_object_mut().unwrap();
        }
        dir.insert(
            leaf.to_owned(),
            json!({ "size": bytes.len(), "offset": body.len().to_string() }),
        );
        body.extend_from_slice(bytes);
    }
    let mut json = serde_json::to_vec(&json!({ "files": Value::Object(root) })).unwrap();
    let json_len = json.len() as u32;
    json.resize(json.len().next_multiple_of(4), 0);
    let index_size = json.len() as u32 + 8;
    let mut archive = Vec::new();
    for word in [4, index_size, index_size - 4, json_len] {
        archive.extend_from_slice(&u32::to_le_bytes(word));
    }
    archive.extend_from_slice(&json);
    archive.extend_from_slice(&body);
    archive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_files_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.asar");
        let files: [(&str, &[u8]); 3] = [
            ("render/index.css", b"a { b: c }"),
            ("render/img/key.png", &[0, 1, 2, 3, 4]),
            ("main.js", b""),
        ];
        fs::write(&path, build(&files)).unwrap();
        let archive = Archive::open(&path).unwrap();
        let mut names: Vec<&str> = archive.names().collect();
        names.sort();
        assert_eq!(names, ["main.js", "render/img/key.png", "render/index.css"]);
        for (name, bytes) in files {
            assert_eq!(archive.read(name).unwrap(), bytes, "{name}");
        }
        assert!(archive.read("nothing").is_err());
    }

    #[test]
    fn reads_unpacked_files_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.asar");
        let mut archive = build(&[("a.txt", b"inside")]);
        let index =
            br#"{"files":{"a.txt":{"size":6,"offset":"0"},"b.node":{"size":3,"unpacked":true}}}"#;
        let index_size = index.len().next_multiple_of(4) as u32 + 8;
        let mut head = Vec::new();
        for word in [4, index_size, index_size - 4, index.len() as u32] {
            head.extend_from_slice(&u32::to_le_bytes(word));
        }
        head.extend_from_slice(index);
        head.resize(8 + index_size as usize, 0);
        let old_base = 8 + u32::from_le_bytes(archive[4..8].try_into().unwrap()) as usize;
        head.extend(archive.drain(old_base..));
        fs::write(&path, head).unwrap();
        fs::create_dir_all(dir.path().join("app.asar.unpacked")).unwrap();
        fs::write(dir.path().join("app.asar.unpacked/b.node"), "out").unwrap();
        let archive = Archive::open(&path).unwrap();
        assert_eq!(archive.read("a.txt").unwrap(), b"inside");
        assert_eq!(archive.read("b.node").unwrap(), b"out");
    }

    /// A file bigger than the archive, as a broken index has it: an error,
    /// not room made for it.
    #[test]
    fn refuses_a_file_past_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.asar");
        let index = br#"{"files":{"a.txt":{"size":4000000000000,"offset":"0"},"b.txt":{"size":2,"offset":"18446744073709551615"}}}"#;
        let index_size = index.len().next_multiple_of(4) as u32 + 8;
        let mut head = Vec::new();
        for word in [4, index_size, index_size - 4, index.len() as u32] {
            head.extend_from_slice(&u32::to_le_bytes(word));
        }
        head.extend_from_slice(index);
        head.resize(8 + index_size as usize, 0);
        head.extend_from_slice(b"tiny");
        fs::write(&path, head).unwrap();
        let archive = Archive::open(&path).unwrap();
        let err = archive.read("a.txt").unwrap_err().to_string();
        assert!(err.contains("cut short"), "{err}");
        let err = archive.read("b.txt").unwrap_err().to_string();
        assert!(err.contains("out of it"), "{err}");
    }

    #[test]
    fn refuses_what_isnt_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.asar");
        fs::write(&path, b"PK\x03\x04 a zip, rather").unwrap();
        let err = Archive::open(&path).err().unwrap().to_string();
        assert!(err.contains("not an asar archive"), "{err}");
    }
}
