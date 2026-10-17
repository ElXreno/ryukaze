//! The keyboard's backlight: a SteelSeries KLC on USB, which colours keys
//! from a HID feature report on its first interface. Only on controllers
//! checked by hand: 1038:113a (Alpha 17 C7VG). There one report of
//! `0c 00 66 00` and then (key, red, green, blue) entries, the keys by
//! their HID usage, colours the keys it lists and leaves the others.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    str::FromStr,
};

use rustix::ioctl::{Opcode, Updater, ioctl, opcode};

/// (vendor, product) of the controllers checked.
const KNOWN: [(u16, u16); 1] = [(0x1038, 0x113a)];

const REPORT_LEN: usize = 524;

/// The controller reads about the first hundred entries of a report.
const KEYS_PER_REPORT: usize = 96;

/// HIDIOCSFEATURE: the report ID (0, none) and then the report.
const SET_FEATURE: Opcode = opcode::read_write::<[u8; REPORT_LEN + 1]>(b'H', 0x06);

/// One colour for all the keys; black is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 3]);

impl Color {
    pub const OFF: Color = Color([0; 3]);
}

/// `#rrggbb`, or `off`.
impl FromStr for Color {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if text == "off" {
            return Ok(Self::OFF);
        }
        let bad = || format!("keyboard colour {text:?}: #rrggbb or off");
        let hex = text.strip_prefix('#').ok_or_else(bad)?;
        if hex.len() != 6 {
            return Err(bad());
        }
        let byte = |at: usize| {
            hex.get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(bad)
        };
        Ok(Self([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            [0, 0, 0] => f.write_str("off"),
            [r, g, b] => write!(f, "#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

/// A controller found: its hidraw node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Klc {
    node: PathBuf,
}

/// The first known controller's first interface, under `sysfs`, as a node
/// under `dev`.
pub fn find(sysfs: &Path, dev: &Path) -> Option<Klc> {
    let ids: Vec<String> = KNOWN
        .iter()
        .map(|(vendor, product)| format!("HID_ID=0003:{vendor:08X}:{product:08X}"))
        .collect();
    let mut nodes: Vec<PathBuf> = fs::read_dir(sysfs.join("class/hidraw"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    nodes.sort();
    nodes.into_iter().find_map(|node| {
        let device = node.join("device");
        let uevent = fs::read_to_string(device.join("uevent")).ok()?;
        // The HID device's parent is the USB interface.
        let interface = fs::read_to_string(device.join("../bInterfaceNumber")).ok()?;
        let known = uevent.lines().any(|line| ids.iter().any(|id| id == line));
        let name = node.file_name()?;
        (known && interface.trim() == "00").then(|| Klc {
            node: dev.join(name),
        })
    })
}

impl Klc {
    pub fn node(&self) -> &Path {
        &self.node
    }

    /// Each key in its colour; the keys not listed keep theirs.
    pub fn set(&self, keys: &[(u8, Color)]) -> io::Result<()> {
        let node = OpenOptions::new().read(true).write(true).open(&self.node)?;
        for report in reports(keys) {
            let mut data = [0; REPORT_LEN + 1];
            data[1..].copy_from_slice(&report);
            // SAFETY: HIDIOCSFEATURE reads the report from `data`, which is
            // as long as its opcode says, and writes nothing past it.
            unsafe { ioctl(&node, Updater::<SET_FEATURE, _>::new(&mut data)) }?;
        }
        Ok(())
    }
}

/// The HID usages of every key there may be: modifiers, Fn (0xf0), and
/// the rest from A to the numpad and Power.
fn keys() -> impl Iterator<Item = u8> {
    (0xe0..=0xe7).chain([0xf0]).chain(0x04..=0x66)
}

/// Each key's name, as the config writes it, and its HID usage. Checked
/// on the Alpha 17's keyboard, which has no Home, End or Pause of their
/// own (they share PgUp, PgDn and the numpad), a Copilot key where right
/// Ctrl would be, and `102nd`, the `\` between AltGr and Fn.
fn names() -> Vec<(String, u8)> {
    let mut names: Vec<(String, u8)> = Vec::new();
    names.extend((b'a'..=b'z').map(|c| (char::from(c).to_string(), c - b'a' + 0x04)));
    names.extend((1..=9).map(|n| (n.to_string(), 0x1d + n)));
    names.push(("0".into(), 0x27));
    names.extend((1..=12).map(|n| (format!("f{n}"), 0x39 + n)));
    names.extend((1..=9).map(|n| (format!("kp{n}"), 0x58 + n)));
    names.push(("kp0".into(), 0x62));
    names.extend(
        [
            ("enter", 0x28),
            ("esc", 0x29),
            ("backspace", 0x2a),
            ("tab", 0x2b),
            ("space", 0x2c),
            ("minus", 0x2d),
            ("equal", 0x2e),
            ("lbracket", 0x2f),
            ("rbracket", 0x30),
            ("backslash", 0x31),
            ("semicolon", 0x33),
            ("apostrophe", 0x34),
            ("grave", 0x35),
            ("comma", 0x36),
            ("dot", 0x37),
            ("slash", 0x38),
            ("capslock", 0x39),
            ("prtsc", 0x46),
            ("scrolllock", 0x47),
            ("insert", 0x49),
            ("pageup", 0x4b),
            ("delete", 0x4c),
            ("pagedown", 0x4e),
            ("right", 0x4f),
            ("left", 0x50),
            ("down", 0x51),
            ("up", 0x52),
            ("numlock", 0x53),
            ("kpslash", 0x54),
            ("kpasterisk", 0x55),
            ("kpminus", 0x56),
            ("kpplus", 0x57),
            ("kpenter", 0x58),
            ("kpdot", 0x63),
            ("102nd", 0x64),
            ("power", 0x66),
            ("lctrl", 0xe0),
            ("lshift", 0xe1),
            ("lalt", 0xe2),
            ("win", 0xe3),
            ("copilot", 0xe4),
            ("rshift", 0xe5),
            ("ralt", 0xe6),
            ("fn", 0xf0),
        ]
        .map(|(name, key)| (name.to_owned(), key)),
    );
    names
}

/// Named groups of keys.
fn groups() -> [(&'static str, Vec<u8>); 8] {
    [
        ("letters", (0x04..=0x1d).collect()),
        ("digits", (0x1e..=0x27).collect()),
        ("function", (0x3a..=0x45).collect()),
        ("navigation", vec![0x46, 0x47, 0x49, 0x4b, 0x4c, 0x4e]),
        ("arrows", (0x4f..=0x52).collect()),
        ("numpad", (0x53..=0x63).collect()),
        (
            "modifiers",
            vec![0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xf0],
        ),
        ("wasd", vec![0x1a, 0x04, 0x16, 0x07]),
    ]
}

/// The numpad's keys but NumLock, which stays lit to be found while the
/// others are dark.
pub fn numpad_but_numlock() -> std::ops::RangeInclusive<u8> {
    0x54..=0x63
}

/// The keys a key's or a group's name stands for.
pub fn named(name: &str) -> Option<Vec<u8>> {
    let key = names()
        .into_iter()
        .find(|(known, _)| known == name)
        .map(|(_, key)| vec![key]);
    key.or_else(|| {
        groups()
            .into_iter()
            .find(|(group, _)| *group == name)
            .map(|(_, keys)| keys)
    })
}

/// What to send: every key in `color`, if there is one, then the keys with
/// their own, the larger groups first so the smaller and single keys win.
pub fn paint(color: Option<Color>, own: &[(Vec<u8>, Color)]) -> Vec<(u8, Color)> {
    let mut painted: Vec<(u8, Color)> = color
        .map(|color| keys().map(|key| (key, color)).collect())
        .unwrap_or_default();
    let mut own: Vec<&(Vec<u8>, Color)> = own.iter().collect();
    own.sort_by_key(|(keys, _)| std::cmp::Reverse(keys.len()));
    for (keys, color) in own {
        for &key in keys {
            match painted.iter_mut().find(|(painted, _)| *painted == key) {
                Some(entry) => entry.1 = *color,
                None => painted.push((key, *color)),
            }
        }
    }
    painted
}

/// The reports for `keys`, as many to each as the controller reads.
fn reports(keys: &[(u8, Color)]) -> Vec<[u8; REPORT_LEN]> {
    keys.chunks(KEYS_PER_REPORT)
        .map(|chunk| {
            let mut report = [0; REPORT_LEN];
            report[..4].copy_from_slice(&[0x0c, 0x00, 0x66, 0x00]);
            let (entries, _) = report[4..].as_chunks_mut::<4>();
            // An entry with key 0xff is unused.
            for entry in entries.iter_mut() {
                entry[0] = 0xff;
            }
            for (entry, &(key, Color([r, g, b]))) in entries.iter_mut().zip(chunk) {
                *entry = [key, r, g, b];
            }
            report
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn colours_read_and_written() {
        assert_eq!("#fab387".parse(), Ok(Color([0xfa, 0xb3, 0x87])));
        assert_eq!("off".parse(), Ok(Color::OFF));
        assert_eq!(Color([0xfa, 0xb3, 0x87]).to_string(), "#fab387");
        assert_eq!("#000000".parse::<Color>().unwrap().to_string(), "off");
        for bad in ["fab387", "#fab38", "#fab3877", "#gab387", "red", "#ffé000"] {
            assert!(bad.parse::<Color>().is_err(), "{bad}");
        }
    }

    #[test]
    fn names_and_groups() {
        assert_eq!(named("a"), Some(vec![0x04]));
        assert_eq!(named("0"), Some(vec![0x27]));
        assert_eq!(named("f7"), Some(vec![0x40]));
        assert_eq!(named("kp9"), Some(vec![0x61]));
        assert_eq!(named("copilot"), Some(vec![0xe4]));
        assert_eq!(named("wasd"), Some(vec![0x1a, 0x04, 0x16, 0x07]));
        assert_eq!(named("numpad").unwrap().len(), 17);
        assert_eq!(named("home"), None);
        // Every name once, and every key it names one there is.
        let names = names();
        let all: Vec<u8> = keys().collect();
        for (i, (name, key)) in names.iter().enumerate() {
            assert!(names[..i].iter().all(|(other, _)| other != name), "{name}");
            assert!(all.contains(key), "{name}");
        }
        for (group, keys) in groups() {
            assert!(names.iter().all(|(name, _)| name != group), "{group}");
            assert!(keys.iter().all(|key| all.contains(key)), "{group}");
        }
    }

    #[test]
    fn single_keys_over_smaller_groups_over_larger() {
        let red = Color([255, 0, 0]);
        let white = Color([255; 3]);
        let own = [
            (named("w").unwrap(), white),
            (named("letters").unwrap(), Color::OFF),
            (named("wasd").unwrap(), red),
        ];
        let painted = paint(Some(Color([0, 0, 255])), &own);
        let at = |name: &str| {
            let key = named(name).unwrap()[0];
            painted
                .iter()
                .find(|(painted, _)| *painted == key)
                .unwrap()
                .1
        };
        assert_eq!(painted.len(), keys().count());
        assert_eq!(at("w"), white);
        assert_eq!(at("a"), red);
        assert_eq!(at("q"), Color::OFF);
        assert_eq!(at("1"), Color([0, 0, 255]));
        // Without a colour for all, only the keys named.
        assert_eq!(paint(None, &own).len(), 26);
        assert!(paint(None, &[]).is_empty());
    }

    #[test]
    fn every_key_once_in_reports_it_reads() {
        let reports = reports(&paint(Some(Color([1, 2, 3])), &[]));
        assert_eq!(reports.len(), 2);
        let mut seen = Vec::new();
        for report in &reports {
            assert_eq!(&report[..4], &[0x0c, 0x00, 0x66, 0x00]);
            let (entries, _) = report[4..].as_chunks::<4>();
            let used: Vec<&[u8; 4]> = entries.iter().filter(|entry| entry[0] != 0xff).collect();
            assert!(used.len() <= KEYS_PER_REPORT);
            assert!(used.iter().all(|entry| entry[1..] == [1, 2, 3]));
            seen.extend(used.iter().map(|entry| entry[0]));
        }
        assert_eq!(seen, keys().collect::<Vec<_>>());
        // Numpad 9, 0 and ., and the key by right Alt, which a single
        // report left alone on the Alpha.
        for key in [0x61, 0x62, 0x63, 0x64] {
            assert!(seen.contains(&key));
        }
    }

    /// `class/hidraw/<name>` → its HID device, inside a USB interface.
    fn hidraw(sysfs: &Path, name: &str, id: &str, interface: &str) {
        let usb = sysfs.join(format!("devices/usb/{name}-if"));
        let hid = usb.join("hid");
        fs::create_dir_all(&hid).unwrap();
        fs::write(usb.join("bInterfaceNumber"), format!("{interface}\n")).unwrap();
        fs::write(
            hid.join("uevent"),
            format!("DRIVER=hid-generic\nHID_ID={id}\n"),
        )
        .unwrap();
        let class = sysfs.join("class/hidraw").join(name);
        fs::create_dir_all(&class).unwrap();
        symlink(&hid, class.join("device")).unwrap();
    }

    #[test]
    fn finds_the_first_interface_of_one_it_knows() {
        let sysfs = tempfile::tempdir().unwrap();
        hidraw(sysfs.path(), "hidraw0", "0003:0000046D:0000C52B", "00");
        hidraw(sysfs.path(), "hidraw3", "0003:00001038:0000113A", "00");
        hidraw(sysfs.path(), "hidraw4", "0003:00001038:0000113A", "01");
        let klc = find(sysfs.path(), Path::new("/dev")).unwrap();
        assert_eq!(klc.node(), Path::new("/dev/hidraw3"));

        let other = tempfile::tempdir().unwrap();
        hidraw(other.path(), "hidraw0", "0003:00001038:00001122", "00");
        assert_eq!(find(other.path(), Path::new("/dev")), None);
        assert_eq!(find(Path::new("/nonexistent"), Path::new("/dev")), None);
    }
}
