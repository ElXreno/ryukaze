//! The keys in a layout file's units: across in the letters' pitch, down
//! in the rows', from the top left of Esc, each box taking the gap to the
//! next key right and below.

use std::{collections::BTreeMap, fmt::Write};

use ryukaze_core::layout::{File, Key};

use crate::boxes::Patch;

/// The rows of letters and digits, left to right, by HID usage.
const ROWS: [&[u8]; 4] = [
    &[
        0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2d, 0x2e,
    ],
    &[
        0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30,
    ],
    &[
        0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34,
    ],
    &[0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38],
];

const ESC: u8 = 0x29;

/// The keys' labels, as the built-in layouts have them.
const LABELS: &[(u8, &str)] = &[
    (0x04, "A"),
    (0x05, "B"),
    (0x06, "C"),
    (0x07, "D"),
    (0x08, "E"),
    (0x09, "F"),
    (0x0a, "G"),
    (0x0b, "H"),
    (0x0c, "I"),
    (0x0d, "J"),
    (0x0e, "K"),
    (0x0f, "L"),
    (0x10, "M"),
    (0x11, "N"),
    (0x12, "O"),
    (0x13, "P"),
    (0x14, "Q"),
    (0x15, "R"),
    (0x16, "S"),
    (0x17, "T"),
    (0x18, "U"),
    (0x19, "V"),
    (0x1a, "W"),
    (0x1b, "X"),
    (0x1c, "Y"),
    (0x1d, "Z"),
    (0x1e, "1"),
    (0x1f, "2"),
    (0x20, "3"),
    (0x21, "4"),
    (0x22, "5"),
    (0x23, "6"),
    (0x24, "7"),
    (0x25, "8"),
    (0x26, "9"),
    (0x27, "0"),
    (0x28, "Enter"),
    (0x29, "Esc"),
    (0x2a, "Backspace"),
    (0x2b, "Tab"),
    (0x2c, ""),
    (0x2d, "-"),
    (0x2e, "="),
    (0x2f, "["),
    (0x30, "]"),
    (0x31, "\\"),
    (0x32, "#"),
    (0x33, ";"),
    (0x34, "'"),
    (0x35, "`"),
    (0x36, ","),
    (0x37, "."),
    (0x38, "/"),
    (0x39, "Caps"),
    (0x3a, "F1"),
    (0x3b, "F2"),
    (0x3c, "F3"),
    (0x3d, "F4"),
    (0x3e, "F5"),
    (0x3f, "F6"),
    (0x40, "F7"),
    (0x41, "F8"),
    (0x42, "F9"),
    (0x43, "F10"),
    (0x44, "F11"),
    (0x45, "F12"),
    (0x46, "PrtSc"),
    (0x47, "ScrLk"),
    (0x48, "Pause"),
    (0x49, "Ins"),
    (0x4a, "Home"),
    (0x4b, "PgUp"),
    (0x4c, "Del"),
    (0x4d, "End"),
    (0x4e, "PgDn"),
    (0x4f, "→"),
    (0x50, "←"),
    (0x51, "↓"),
    (0x52, "↑"),
    (0x53, "Num"),
    (0x54, "/"),
    (0x55, "*"),
    (0x56, "-"),
    (0x57, "+"),
    (0x58, "Enter"),
    (0x59, "1"),
    (0x5a, "2"),
    (0x5b, "3"),
    (0x5c, "4"),
    (0x5d, "5"),
    (0x5e, "6"),
    (0x5f, "7"),
    (0x60, "8"),
    (0x61, "9"),
    (0x62, "0"),
    (0x63, "."),
    (0x64, "\\"),
    (0x66, "Power"),
    (0xe0, "Ctrl"),
    (0xe1, "Shift"),
    (0xe2, "Alt"),
    (0xe3, "Win"),
    (0xe4, "Ctrl"),
    (0xe5, "Shift"),
    (0xe6, "AltGr"),
];

/// The label of the Fn key, which HID has no usage for.
const FN: (u8, &str) = (0xf0, "Fn");

fn label(code: u8) -> &'static str {
    LABELS
        .iter()
        .chain([&FN])
        .find(|(known, _)| *known == code)
        .map_or("", |(_, label)| label)
}

/// What the picture's pixels are in the file's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Units {
    /// A letter's pitch across, a row's down.
    pub pitch: (f32, f32),
    /// The gap between letters, and between rows.
    pub gap: (f32, f32),
    /// The top left of Esc.
    pub origin: (f32, f32),
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.0),
    }
}

impl Units {
    /// From the rows of letters and digits, and Esc.
    pub fn measure(keys: &BTreeMap<u8, Patch>) -> Result<Self, String> {
        let esc = keys.get(&ESC).ok_or("no Esc to measure from")?;
        let mut across = Vec::new();
        let mut tops = Vec::new();
        let mut sizes = (Vec::new(), Vec::new());
        for (row, codes) in ROWS.iter().enumerate() {
            let present: Vec<(usize, &Patch)> = codes
                .iter()
                .enumerate()
                .filter_map(|(at, code)| Some((at, keys.get(code)?)))
                .collect();
            if let (Some(first), Some(last)) = (present.first(), present.last())
                && last.0 > first.0
            {
                across.push((last.1.x0 as f32 - first.1.x0 as f32) / (last.0 - first.0) as f32);
            }
            if let Some(top) = median(present.iter().map(|(_, key)| key.y0 as f32).collect()) {
                tops.push((row, top));
            }
            sizes
                .0
                .extend(present.iter().map(|(_, key)| key.width() as f32));
            sizes
                .1
                .extend(present.iter().map(|(_, key)| key.height() as f32));
        }
        let down: Vec<f32> = tops
            .windows(2)
            .map(|two| (two[1].1 - two[0].1) / (two[1].0 - two[0].0) as f32)
            .collect();
        let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
        if across.is_empty() || down.is_empty() {
            return Err("too few letters to measure by".into());
        }
        let pitch = (mean(&across), mean(&down));
        let size = (
            median(sizes.0).unwrap_or(pitch.0),
            median(sizes.1).unwrap_or(pitch.1),
        );
        if pitch.0 <= size.0 * 0.5 || pitch.1 <= size.1 * 0.5 {
            return Err("letters that don't step along their rows".into());
        }
        Ok(Self {
            pitch,
            gap: (pitch.0 - size.0, pitch.1 - size.1),
            origin: (esc.x0 as f32, esc.y0 as f32),
        })
    }

    fn key(&self, code: u8, patch: &Patch) -> Key {
        let round = |value: f32| (value * 1000.0).round() / 1000.0 + 0.0;
        Key {
            hid: code,
            label: label(code).to_owned(),
            x: round((patch.x0 as f32 - self.origin.0) / self.pitch.0),
            y: round((patch.y0 as f32 - self.origin.1) / self.pitch.1),
            w: round((patch.width() as f32 + self.gap.0) / self.pitch.0),
            h: round((patch.height() as f32 + self.gap.1) / self.pitch.1),
        }
    }
}

/// A layout file of `keys`, top row first, each key cut back where it
/// would run into the next.
/// With the gaps between keys, in key widths.
pub fn layout(
    name: &str,
    chids: Vec<String>,
    keys: &BTreeMap<u8, Patch>,
) -> Result<(File, (f32, f32)), String> {
    let units = Units::measure(keys)?;
    let mut keys: Vec<Key> = keys
        .iter()
        .map(|(&code, patch)| units.key(code, patch))
        .collect();
    keys.sort_by(|a, b| {
        a.y.total_cmp(&b.y)
            .then(a.x.total_cmp(&b.x))
            .then(a.hid.cmp(&b.hid))
    });
    for at in 0..keys.len() {
        for other in 0..keys.len() {
            if at == other || !keys[at].bounds().overlaps(&keys[other].bounds()) {
                continue;
            }
            let (a, b) = (&keys[at], &keys[other]);
            let across = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
            let down = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
            let (x, y) = (b.x, b.y);
            let a = &mut keys[at];
            if down >= across && a.x < x {
                a.w = x - a.x;
            } else if down < across && a.y < y {
                a.h = y - a.y;
            }
        }
    }
    let file = File {
        name: name.to_owned(),
        boards: Vec::new(),
        chids,
        keys,
        touchpad: None,
        deck: None,
        logo: None,
    };
    file.check()?;
    let gap = (units.gap.0 / units.pitch.0, units.gap.1 / units.pitch.1);
    Ok((file, gap))
}

/// A TOML string.
fn quoted(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => write!(out, "\\u{:04x}", u32::from(c)).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The file's text, as the built-in ones are written.
pub fn text(file: &File) -> String {
    let mut out = format!("name = {}\n", quoted(&file.name));
    let list = |out: &mut String, key: &str, values: &[String]| {
        if !values.is_empty() {
            writeln!(out, "{key} = [").unwrap();
            for value in values {
                writeln!(out, "  {},", quoted(value)).unwrap();
            }
            out.push_str("]\n");
        }
    };
    list(&mut out, "boards", &file.boards);
    list(&mut out, "chids", &file.chids);
    out.push_str("keys = [\n");
    for key in &file.keys {
        writeln!(
            out,
            "  {{ hid = {:#04x}, label = {}, x = {:?}, y = {:?}, w = {:?}, h = {:?} }},",
            key.hid,
            quoted(&key.label),
            key.x,
            key.y,
            key.w,
            key.h
        )
        .unwrap();
    }
    out.push_str("]\n");
    if let Some(pad) = &file.touchpad {
        writeln!(
            out,
            "\n[touchpad]\nx = {:?}\ny = {:?}\nw = {:?}\nh = {:?}\nrows = {}",
            pad.x, pad.y, pad.w, pad.h, pad.rows
        )
        .unwrap();
    }
    if let Some(deck) = &file.deck {
        writeln!(
            out,
            "\n[deck]\nx = {:?}\ny = {:?}\nw = {:?}\nh = {:?}",
            deck.x, deck.y, deck.w, deck.h
        )
        .unwrap();
    }
    if let Some(logo) = &file.logo {
        writeln!(
            out,
            "\n[logo]\nlid = {:?}\nx = {:?}\ny = {:?}\nw = {:?}\nh = {:?}",
            logo.lid, logo.x, logo.y, logo.w, logo.h
        )
        .unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use ryukaze_core::layout::{Logo, Touchpad};

    use super::*;

    fn patch(x0: u32, y0: u32, w: u32, h: u32) -> Patch {
        Patch {
            x0,
            y0,
            x1: x0 + w,
            y1: y0 + h,
        }
    }

    /// Esc above a board of letters 100 px apart across and 110 down, the
    /// keys 90 by 96, with a wide Backspace and a key two rows high.
    fn board() -> BTreeMap<u8, Patch> {
        let mut keys = BTreeMap::from([(ESC, patch(20, 10, 90, 60))]);
        for (row, codes) in ROWS.iter().enumerate() {
            for (column, &code) in codes.iter().enumerate() {
                let x = 20 + 100 * column as u32 + 25 * row as u32;
                keys.insert(code, patch(x, 90 + 110 * row as u32, 90, 96));
            }
        }
        keys.insert(0x2a, patch(1220, 90, 190, 96));
        keys.insert(0x57, patch(1500, 200, 90, 206));
        keys
    }

    #[test]
    fn measures_in_letters_and_rows() {
        let units = Units::measure(&board()).unwrap();
        assert_eq!(units.pitch, (100.0, 110.0));
        assert_eq!(units.gap, (10.0, 14.0));
        assert_eq!(units.origin, (20.0, 10.0));
        let (file, gap) = layout("kbd", vec![], &board()).unwrap();
        assert_eq!(gap, (0.1, 14.0 / 110.0));
        let key = |code: u8| file.keys.iter().find(|key| key.hid == code).unwrap();
        assert_eq!(
            (key(ESC).x, key(ESC).y, key(ESC).w, key(ESC).h),
            (0.0, 0.0, 1.0, 0.673)
        );
        let q = key(0x14);
        assert_eq!(
            (q.x, q.y, q.w, q.h, q.label.as_str()),
            (0.25, 1.727, 1.0, 1.0, "Q")
        );
        assert_eq!(key(0x2a).w, 2.0);
        assert_eq!(key(0x57).h, 2.0);
        assert_eq!(file.keys[0].hid, ESC);
    }

    #[test]
    fn cuts_back_keys_that_run_into_the_next() {
        let mut keys = board();
        // Closer to 1 than the gap.
        keys.insert(0x1f, patch(118, 90, 90, 96));
        let (file, _) = layout("kbd", vec![], &keys).unwrap();
        let key = |code: u8| file.keys.iter().find(|key| key.hid == code).unwrap();
        assert_eq!(key(0x1e).x + key(0x1e).w, key(0x1f).x);
    }

    #[test]
    fn needs_esc_and_letters() {
        let mut keys = board();
        keys.remove(&ESC);
        assert!(Units::measure(&keys).unwrap_err().contains("Esc"));
        let alone = BTreeMap::from([(ESC, patch(0, 0, 10, 10)), (0x04, patch(20, 20, 10, 10))]);
        assert!(Units::measure(&alone).unwrap_err().contains("too few"));
    }

    #[test]
    fn writes_what_reads_back() {
        let (mut file, _) = layout(
            "a \"kbd\"",
            vec!["532fd82f-3120-553c-acab-54599f390612".into()],
            &board(),
        )
        .unwrap();
        file.touchpad = Some(Touchpad {
            x: 4.0,
            y: 6.0,
            w: 6.5,
            h: 4.25,
            rows: 5,
        });
        file.logo = Some(Logo {
            lid: 1.501,
            x: 0.099,
            y: 0.354,
            w: 0.114,
            h: 0.206,
        });
        let text = text(&file);
        assert!(text.starts_with("name = \"a \\\"kbd\\\"\"\n"), "{text}");
        assert!(text.contains("\n[logo]\nlid = 1.501\n"), "{text}");
        assert_eq!(quoted("\\"), "\"\\\\\"");
        assert!(text.contains("chids = [\n  \"532fd82f-"), "{text}");
        assert_eq!(File::parse(&text).unwrap(), file);
    }
}
