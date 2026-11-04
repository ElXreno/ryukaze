//! Which laptops GG takes each keyboard for: its engine looks a laptop's
//! CHID up in a Go map built into it, for a controller's product IDs,
//! which the migrations of its database name.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

/// SteelSeries' USB vendor ID, which GG's product IDs carry above the
/// product's own.
const STEELSERIES: u32 = 0x1038;

/// A run of CHIDs shorter than this isn't the table.
const LEAST_ENTRIES: usize = 32;

/// How far apart, in bytes of code, the map's loop loads its two arrays.
const NEAR: usize = 256;

/// The sections' flag for code.
const EXECUTE: u32 = 0x2000_0000;

/// A 64-bit Windows executable's sections, where it asks to be loaded.
pub struct Image {
    data: Vec<u8>,
    sections: Vec<Section>,
}

struct Section {
    va: u64,
    raw: usize,
    len: usize,
    code: bool,
}

impl Image {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        let u16_at = |at: usize| -> Result<u16> {
            Ok(u16::from_le_bytes(
                data.get(at..at + 2).context("cut short")?.try_into()?,
            ))
        };
        let u32_at = |at: usize| -> Result<u32> {
            Ok(u32::from_le_bytes(
                data.get(at..at + 4).context("cut short")?.try_into()?,
            ))
        };
        ensure!(data.starts_with(b"MZ"), "not a Windows executable");
        let pe = u32_at(0x3c)? as usize;
        ensure!(
            data.get(pe..pe + 4) == Some(b"PE\0\0"),
            "not a Windows executable"
        );
        ensure!(
            u16_at(pe + 4)? == 0x8664 && u16_at(pe + 24)? == 0x20b,
            "not a 64-bit x86 executable"
        );
        let count = u16_at(pe + 6)? as usize;
        let optional = u16_at(pe + 20)? as usize;
        let base = u64::from_le_bytes(
            data.get(pe + 48..pe + 56)
                .context("cut short")?
                .try_into()?,
        );
        let table = pe + 24 + optional;
        let mut sections = Vec::with_capacity(count);
        for index in 0..count {
            let at = table + 40 * index;
            let (virtual_size, va) = (u32_at(at + 8)?, u32_at(at + 12)?);
            let (raw_size, raw) = (u32_at(at + 16)?, u32_at(at + 20)? as usize);
            let raw = raw.min(data.len());
            let loaded = if virtual_size == 0 {
                raw_size
            } else {
                raw_size.min(virtual_size)
            };
            let len = (loaded as usize).min(data.len().saturating_sub(raw));
            sections.push(Section {
                va: base
                    .checked_add(u64::from(va))
                    .context("a section past the end")?,
                raw,
                len,
                code: u32_at(at + 36)? & EXECUTE != 0,
            });
        }
        Ok(Self { data, sections })
    }

    fn read(&self, va: u64, len: usize) -> Option<&[u8]> {
        let section = self.sections.iter().find(|section| {
            section.va <= va && va < section.va.saturating_add(section.len as u64)
        })?;
        let from = (va - section.va) as usize;
        (from + len <= section.len)
            .then(|| &self.data[section.raw + from..section.raw + from + len])
    }

    fn u64_at(&self, va: u64) -> Option<u64> {
        Some(u64::from_le_bytes(self.read(va, 8)?.try_into().ok()?))
    }

    /// The Go string whose header is at `va`, if it's a CHID.
    fn chid_at(&self, va: u64) -> Option<&str> {
        let (pointer, len) = (self.u64_at(va)?, self.u64_at(va + 8)?);
        if !(36..=48).contains(&len) {
            return None;
        }
        let text = std::str::from_utf8(self.read(pointer, len as usize)?).ok()?;
        is_chid_key(text).then_some(text)
    }

    /// The map's keys, as Go compiles a map literal: an array of Go string
    /// headers, here the longest run of them that are CHIDs.
    fn chid_keys(&self) -> Result<(u64, Vec<String>)> {
        let mut runs: Vec<(u64, Vec<String>)> = Vec::new();
        for section in &self.sections {
            let mut run: Option<(u64, Vec<String>)> = None;
            let mut at = section.va;
            while at + 16 <= section.va + section.len as u64 {
                match (self.chid_at(at), &mut run) {
                    (Some(key), Some((_, keys))) => keys.push(key.to_owned()),
                    (Some(key), None) => run = Some((at, vec![key.to_owned()])),
                    (None, _) => {
                        runs.extend(run.take());
                        at += 8;
                        continue;
                    }
                }
                at += 16;
            }
            runs.extend(run);
        }
        runs.sort_by_key(|(_, keys)| std::cmp::Reverse(keys.len()));
        let Some((at, keys)) = runs.first() else {
            bail!("no table of CHIDs in it");
        };
        ensure!(
            keys.len() >= LEAST_ENTRIES,
            "no table of CHIDs in it, only {} together",
            keys.len()
        );
        if runs
            .get(1)
            .is_some_and(|(_, other)| other.len() == keys.len())
        {
            bail!("two tables of CHIDs alike in it");
        }
        Ok((*at, keys.clone()))
    }

    /// What the code loads beside `target`: the targets of
    /// `lea reg, [rip + disp]` within [`NEAR`] bytes of one that loads it.
    fn loaded_beside(&self, target: u64) -> BTreeSet<u64> {
        let mut found = BTreeSet::new();
        for section in self.sections.iter().filter(|section| section.code) {
            let code = &self.data[section.raw..section.raw + section.len];
            let leas: Vec<(usize, u64)> = (0..code.len().saturating_sub(7))
                .filter(|&at| {
                    matches!(code[at], 0x48 | 0x4c)
                        && code[at + 1] == 0x8d
                        && code[at + 2] & 0xc7 == 0x05
                })
                .map(|at| {
                    let disp = i32::from_le_bytes(code[at + 3..at + 7].try_into().unwrap());
                    let next = section.va + at as u64 + 7;
                    (at, next.wrapping_add_signed(i64::from(disp)))
                })
                .collect();
            for &(at, _) in leas.iter().filter(|(_, to)| *to == target) {
                found.extend(
                    leas.iter()
                        .filter(|(other, to)| other.abs_diff(at) <= NEAR && *to != target)
                        .map(|(_, to)| *to),
                );
            }
        }
        found
    }

    fn u32s(&self, va: u64, count: usize) -> Option<Vec<u32>> {
        let bytes = self.read(va, count * 4)?;
        Some(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| u32::from_le_bytes(*word))
                .collect(),
        )
    }
}

/// `8-4-4-4-12` hex digits, and a suffix such as `-KLC` for the
/// controller it's for.
fn is_chid_key(text: &str) -> bool {
    let (uuid, suffix) = text.split_at_checked(36).unwrap_or((text, ""));
    let shape = uuid.len() == 36
        && uuid.char_indices().all(|(at, c)| match at {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    shape
        && (suffix.is_empty()
            || suffix.strip_prefix('-').is_some_and(|tag| {
                !tag.is_empty()
                    && tag
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            }))
}

/// GG's devices, from its database's migrations.
#[derive(Debug, Default)]
pub struct Devices {
    /// The devices' names by product ID, the vendor's above the product's.
    names: BTreeMap<u32, BTreeSet<String>>,
    /// The controllers' names, with their bootloaders', by product ID.
    physical: BTreeMap<u32, BTreeSet<(u32, String)>>,
    /// The devices each controller shows as.
    shown_as: BTreeMap<u32, BTreeSet<u32>>,
}

impl Devices {
    /// From the `.sql` files under `dir`.
    pub fn read(dir: &Path) -> Result<Self> {
        let mut files = Vec::new();
        collect_sql(dir, &mut files)?;
        ensure!(!files.is_empty(), "{}: no .sql files", dir.display());
        files.sort();
        let mut devices = Self::default();
        for file in files {
            let text = fs::read_to_string(&file).with_context(|| format!("{}", file.display()))?;
            devices.add(&text);
        }
        Ok(devices)
    }

    pub fn add(&mut self, sql: &str) {
        for insert in inserts(sql) {
            for row in &insert.rows {
                let value = |column: &str| {
                    let at = insert.columns.iter().position(|name| name == column)?;
                    row.get(at)
                };
                let int = |column: &str| match value(column)? {
                    Value::Int(value) => u32::try_from(*value).ok(),
                    _ => None,
                };
                let text = |column: &str| match value(column)? {
                    Value::Text(value) => Some(value.clone()),
                    _ => None,
                };
                match insert.table.as_str() {
                    "devices" => {
                        if let (Some(product), Some(name)) = (int("product_id"), text("name")) {
                            self.names.entry(product).or_default().insert(name);
                        }
                    }
                    "physical_devices" => {
                        if let (Some(product), Some(boot), Some(name)) = (
                            int("product_id"),
                            int("bootloader_product_id"),
                            text("name"),
                        ) {
                            self.physical
                                .entry(product)
                                .or_default()
                                .insert((boot, name));
                        }
                    }
                    "devices_to_physical_devices" => {
                        if let (Some(physical), Some(main)) = (
                            int("physical_device_product_id"),
                            int("main_device_product_id"),
                        ) {
                            self.shown_as.entry(physical).or_default().insert(main);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn knows(&self, product: u16) -> bool {
        let id = STEELSERIES << 16 | u32::from(product);
        self.names.contains_key(&id) || self.physical.contains_key(&id)
    }

    /// The names a value of the table goes by, the bootloader's product
    /// above the controller's: the controller's own and its device's.
    fn names_of(&self, value: u32) -> BTreeSet<String> {
        let id = STEELSERIES << 16 | (value & 0xffff);
        let boot = STEELSERIES << 16 | value >> 16;
        let mut names = BTreeSet::new();
        for (bootloader, name) in self.physical.get(&id).into_iter().flatten() {
            if *bootloader == boot || !self.knows((value >> 16) as u16) {
                names.insert(name.clone());
            }
        }
        let shown = self.shown_as.get(&id).into_iter().flatten();
        for id in std::iter::once(&id).chain(shown) {
            names.extend(self.names.get(id).into_iter().flatten().cloned());
        }
        names
    }
}

fn collect_sql(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("{}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_sql(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "sql") {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Int(i64),
    Text(String),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Int(i64),
    Text(String),
    Punct(char),
}

/// SQL's tokens, its comments left out.
fn tokens(sql: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let starts = |what: &str| sql[at..].starts_with(what);
        match c {
            c if c.is_whitespace() => {}
            '-' if starts("--") => while chars.next_if(|&(_, c)| c != '\n').is_some() {},
            '/' if starts("/*") => {
                let end = sql[at + 2..]
                    .find("*/")
                    .map_or(sql.len(), |end| at + 2 + end + 2);
                while chars.next_if(|&(next, _)| next < end).is_some() {}
            }
            '\'' => {
                let mut text = String::new();
                while let Some((_, c)) = chars.next() {
                    if c == '\'' {
                        if chars.next_if(|&(_, c)| c == '\'').is_some() {
                            text.push('\'');
                            continue;
                        }
                        break;
                    }
                    text.push(c);
                }
                tokens.push(Token::Text(text));
            }
            c if c.is_ascii_digit()
                || (c == '-' && chars.peek().is_some_and(|(_, d)| d.is_ascii_digit())) =>
            {
                let mut end = at + 1;
                while let Some((next, _)) =
                    chars.next_if(|(_, d)| d.is_ascii_alphanumeric() || *d == '.')
                {
                    end = next + 1;
                }
                let text = &sql[at..end];
                tokens.push(
                    text.parse()
                        .map_or_else(|_| Token::Word(text.into()), Token::Int),
                );
            }
            c if c.is_alphanumeric() || matches!(c, '_' | '"' | '`') => {
                let mut end = at + c.len_utf8();
                while let Some((next, d)) = chars
                    .next_if(|(_, d)| d.is_alphanumeric() || matches!(d, '_' | '.' | '"' | '`'))
                {
                    end = next + d.len_utf8();
                }
                let word = sql[at..end].trim_matches(|c| c == '"' || c == '`');
                tokens.push(Token::Word(word.to_ascii_lowercase()));
            }
            c => tokens.push(Token::Punct(c)),
        }
    }
    tokens
}

/// An `INSERT ... VALUES` statement's rows.
#[derive(Debug, PartialEq)]
struct Insert {
    table: String,
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn inserts(sql: &str) -> Vec<Insert> {
    let tokens = tokens(sql);
    let word = |at: usize, what: &str| matches!(tokens.get(at), Some(Token::Word(w)) if w == what);
    let punct = |at: usize, what: char| tokens.get(at) == Some(&Token::Punct(what));
    let mut inserts = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        if !word(at, "insert") {
            at += 1;
            continue;
        }
        while at < tokens.len() && !word(at, "into") && !punct(at, ';') {
            at += 1;
        }
        let Some(Token::Word(table)) = tokens.get(at + 1) else {
            continue;
        };
        at += 2;
        if !punct(at, '(') {
            continue;
        }
        at += 1;
        let mut columns = Vec::new();
        while let Some(Token::Word(column)) = tokens.get(at) {
            columns.push(column.clone());
            at += 1;
            if punct(at, ',') {
                at += 1;
            }
        }
        if !punct(at, ')') || !word(at + 1, "values") {
            continue;
        }
        at += 2;
        let mut rows = Vec::new();
        while punct(at, '(') {
            at += 1;
            let mut row = Vec::new();
            loop {
                row.push(match tokens.get(at) {
                    Some(Token::Int(value)) => Value::Int(*value),
                    Some(Token::Text(value)) => Value::Text(value.clone()),
                    _ => Value::Other,
                });
                // Past the value, whatever it is, to its comma or the row's end.
                let mut depth = 0usize;
                while let Some(token) = tokens.get(at) {
                    match token {
                        Token::Punct('(') => depth += 1,
                        Token::Punct(')' | ',') if depth == 0 => break,
                        Token::Punct(')') => depth -= 1,
                        _ => {}
                    }
                    at += 1;
                }
                at += 1;
                if !punct(at - 1, ',') {
                    break;
                }
            }
            rows.push(row);
            if punct(at, ',') {
                at += 1;
            }
        }
        inserts.push(Insert {
            table: table.clone(),
            columns,
            rows,
        });
    }
    inserts
}

/// GG's table, read: each CHID, its suffix the controller it's for, and
/// the names of the devices it gives.
pub struct Table {
    pub entries: Vec<(String, BTreeSet<String>)>,
    /// How many give no device the database names.
    pub unnamed: usize,
}

impl Table {
    /// The keys, and as values the array of as many product IDs that the
    /// code loads beside them, all but a tenth of them in the database.
    pub fn read(image: &Image, devices: &Devices) -> Result<Self> {
        let (at, keys) = image.chid_keys()?;
        let known = |values: &[u32]| {
            values
                .iter()
                .filter(|&&value| devices.knows(value as u16))
                .count()
        };
        let mut candidates: Vec<(Vec<u32>, usize)> = image
            .loaded_beside(at)
            .into_iter()
            .filter_map(|va| {
                let values = image.u32s(va, keys.len())?;
                let known = known(&values);
                (known * 10 >= keys.len() * 9).then_some((values, known))
            })
            .collect();
        candidates.sort_by_key(|(_, known)| std::cmp::Reverse(*known));
        candidates.dedup_by(|a, b| a.0 == b.0);
        let Some((values, best)) = candidates.first() else {
            bail!(
                "a table of {} CHIDs, but no array of as many product IDs beside it",
                keys.len()
            );
        };
        if candidates.get(1).is_some_and(|(_, other)| other == best) {
            bail!("two arrays of product IDs alike beside the table of CHIDs");
        }
        let mut unnamed = 0;
        let entries = keys
            .into_iter()
            .zip(values)
            .map(|(key, &value)| {
                let names = devices.names_of(value);
                unnamed += usize::from(names.is_empty());
                (key, names)
            })
            .collect();
        Ok(Self { entries, unnamed })
    }

    /// The CHIDs for each device's keyboard controller, `-KLC` theirs.
    pub fn keyboards(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut by_device: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (key, names) in &self.entries {
            let Some(chid) = key.strip_suffix("-KLC") else {
                continue;
            };
            for name in names {
                by_device
                    .entry(name.clone())
                    .or_default()
                    .insert(chid.to_ascii_lowercase());
            }
        }
        by_device
    }
}

/// An executable such as the engine, for tests: the table's two arrays,
/// a decoy, and code that loads the two.
#[cfg(test)]
pub fn build(keys: &[&str], values: &[u32]) -> Vec<u8> {
    const BASE: u64 = 0x1_4000_0000;
    const CODE: u32 = 0x1000;
    const DATA: u32 = 0x2000;
    let mut data = vec![0xaa; 24];
    let keys_at = data.len();
    let strings_at = keys_at + 16 * keys.len();
    let mut strings = Vec::new();
    for key in keys {
        let pointer = BASE + u64::from(DATA) + (strings_at + strings.len()) as u64;
        data.extend_from_slice(&pointer.to_le_bytes());
        data.extend_from_slice(&(key.len() as u64).to_le_bytes());
        strings.extend_from_slice(key.as_bytes());
    }
    data.extend_from_slice(&strings);
    data.resize(data.len().next_multiple_of(4), 0);
    let values_at = data.len();
    data.extend(values.iter().flat_map(|value| value.to_le_bytes()));
    data.extend(values.iter().flat_map(|_| 0xdead_beef_u32.to_le_bytes()));

    let mut code = vec![0x90; 16];
    for (modrm, target) in [(0x05u8, keys_at), (0x0d, values_at)] {
        let next = CODE + code.len() as u32 + 7;
        let disp = (DATA + target as u32).wrapping_sub(next);
        code.extend_from_slice(&[0x48, 0x8d, modrm]);
        code.extend_from_slice(&disp.to_le_bytes());
        code.extend_from_slice(&[0x90; 5]);
    }

    let pe = 0x80;
    let table = pe + 24 + 240;
    let mut image = vec![0; 0x400];
    let mut put = |at: usize, bytes: &[u8]| image[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, b"MZ");
    put(0x3c, &(pe as u32).to_le_bytes());
    put(pe, b"PE\0\0");
    put(pe + 4, &0x8664u16.to_le_bytes());
    put(pe + 6, &2u16.to_le_bytes());
    put(pe + 20, &240u16.to_le_bytes());
    put(pe + 24, &0x20bu16.to_le_bytes());
    put(pe + 48, &BASE.to_le_bytes());
    let mut raw = 0x400u32;
    let sections = [(CODE, &code, EXECUTE), (DATA, &data, 0x4000_0000)];
    for (index, (va, body, flags)) in sections.iter().enumerate() {
        let at = table + 40 * index;
        let size = body.len() as u32;
        put(at + 8, &size.to_le_bytes());
        put(at + 12, &va.to_le_bytes());
        put(at + 16, &size.to_le_bytes());
        put(at + 20, &raw.to_le_bytes());
        put(at + 36, &flags.to_le_bytes());
        raw += size;
    }
    image.extend_from_slice(&code);
    image.extend_from_slice(&data);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQL: &str = "
-- +goose Up
INSERT INTO devices(id, product_id, name, full_name, type, settings) VALUES (
  312,
  272113752,
  'msi_titan18_klc',
  'MSI Per Key RGB Keyboard', 0, '{\"a\": [1, 2]}');
INSERT OR REPLACE INTO devices(id, product_id, name) VALUES (206, 272113722, 'msi_gp76_klc');
INSERT INTO physical_devices(product_id, bootloader_product_id, name, full_name) values
  (272113752, 272113753, 'msi_titan18_klc', 'it''s a keyboard'),
  (272113723, 272113721, 'msi_ge76_klc', 'MSI Per Key RGB Keyboard');
INSERT INTO devices_to_physical_devices(physical_device_product_id, main_device_product_id)
  values (272113723, 272113722);
/* INSERT INTO devices(id, product_id, name) VALUES (1, 272113752, 'commented_out'); */
UPDATE physical_devices SET full_name='x' where product_id=272110445;
";

    fn chid(n: u32, suffix: &str) -> String {
        format!("{n:08x}-1111-5222-8333-444455556666{suffix}")
    }

    fn devices() -> Devices {
        let mut devices = Devices::default();
        devices.add(SQL);
        devices
    }

    #[test]
    fn reads_the_migrations() {
        let devices = devices();
        assert_eq!(
            devices.names_of(0x2059_2058),
            BTreeSet::from(["msi_titan18_klc".to_owned()])
        );
        // A controller shown as another device: both names.
        assert_eq!(
            devices.names_of(0x2039_203b),
            BTreeSet::from(["msi_ge76_klc".to_owned(), "msi_gp76_klc".to_owned()])
        );
        assert!(devices.names_of(0x1234_5678).is_empty());
        let inserts = inserts(SQL);
        assert_eq!(inserts.len(), 4);
        assert_eq!(inserts[0].rows[0][5], Value::Text("{\"a\": [1, 2]}".into()));
        assert_eq!(inserts[2].rows.len(), 2);
        assert_eq!(inserts[2].rows[0][3], Value::Text("it's a keyboard".into()));
    }

    #[test]
    fn finds_the_table_by_its_shape() {
        let keys: Vec<String> = (0..40)
            .map(|n| chid(n, if n % 3 == 0 { "" } else { "-KLC" }))
            .collect();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let values: Vec<u32> = (0..40)
            .map(|n| if n % 2 == 0 { 0x2059_2058 } else { 0x2039_203b })
            .collect();
        let image = Image::parse(build(&keys, &values)).unwrap();
        let table = Table::read(&image, &devices()).unwrap();
        assert_eq!(table.entries.len(), 40);
        assert_eq!(table.unnamed, 0);
        let keyboards = table.keyboards();
        assert_eq!(
            keyboards.keys().collect::<Vec<_>>(),
            ["msi_ge76_klc", "msi_gp76_klc", "msi_titan18_klc"]
        );
        // The even ones but those without a suffix, every third.
        let titan = &keyboards["msi_titan18_klc"];
        assert!(titan.contains(&chid(2, "")) && !titan.contains(&chid(0, "")));
        assert_eq!(titan.len(), 13);
    }

    #[test]
    fn refuses_a_table_too_short_or_unread() {
        let keys: Vec<String> = (0..8).map(|n| chid(n, "-KLC")).collect();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let image = Image::parse(build(&keys, &[0x2059_2058; 8])).unwrap();
        let err = Table::read(&image, &devices()).err().unwrap().to_string();
        assert!(err.contains("only 8"), "{err}");

        let keys: Vec<String> = (0..40).map(|n| chid(n, "-KLC")).collect();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let image = Image::parse(build(&keys, &[0x1111_2222; 40])).unwrap();
        let err = Table::read(&image, &devices()).err().unwrap().to_string();
        assert!(err.contains("no array"), "{err}");

        assert!(Image::parse(b"MZ not really".to_vec()).is_err());
    }

    #[test]
    fn knows_a_chid() {
        assert!(is_chid_key("a8a8a9eb-cb9b-5f46-a7ca-e77ca36a8e13"));
        assert!(is_chid_key("a8a8a9eb-cb9b-5f46-a7ca-e77ca36a8e13-KLC"));
        assert!(!is_chid_key("a8a8a9eb-cb9b-5f46-a7ca-e77ca36a8e13-klc"));
        assert!(!is_chid_key("a8a8a9eb-cb9b-5f46-a7ca-e77ca36a8e1z"));
        assert!(!is_chid_key("a8a8a9eb_cb9b-5f46-a7ca-e77ca36a8e13"));
    }
}
