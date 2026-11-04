//! The keyboard pictures' files, as the window reads them and
//! ryukaze-gg-layouts writes them (see docs/keyboard.md#the-picture), and
//! the laptops each is for.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{keyboard, lightbar};

/// Where the touchpad's stripes stand off its edges, and apart.
pub const PAD_INSET: f32 = 0.2;
pub const STRIPE_GAP: f32 = 0.15;

/// A layout file: the laptops it's for, by DMI board name or by CHID, its
/// keys, touchpad and deck in key widths from the top left of Esc, and its
/// logo.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub name: String,
    #[serde(default)]
    pub boards: Vec<String>,
    #[serde(default)]
    pub chids: Vec<String>,
    pub keys: Vec<Key>,
    pub touchpad: Option<Touchpad>,
    /// The laptop's lower half, keys and touchpad on it.
    pub deck: Option<Bounds>,
    pub logo: Option<Logo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub hid: u8,
    #[serde(default)]
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Key {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Touchpad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default = "bar_rows")]
    pub rows: u8,
}

impl Touchpad {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

/// The logo on the lid: where, as parts of the lid seen from the keys, and
/// the lid's width to its height.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logo {
    pub lid: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Logo {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

/// A box of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bounds {
    /// Its size positive.
    pub fn check(&self, what: &str) -> Result<(), String> {
        if ![self.x, self.y, self.w, self.h]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(format!("{what}: a number that isn't"));
        }
        if self.w <= 0.0 || self.h <= 0.0 {
            return Err(format!("{what}: a size that isn't positive"));
        }
        Ok(())
    }

    /// `other` inside, give or take a hundredth.
    pub fn holds(&self, other: &Bounds) -> bool {
        const SLACK: f32 = 0.01;
        other.x >= self.x - SLACK
            && other.y >= self.y - SLACK
            && other.x + other.w <= self.x + self.w + SLACK
            && other.y + other.h <= self.y + self.h + SLACK
    }

    /// More than touching.
    pub fn overlaps(&self, other: &Bounds) -> bool {
        let across = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let down = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        across >= 0.01 && down >= 0.01
    }
}

fn bar_rows() -> u8 {
    lightbar::BAR_ROWS
}

impl File {
    /// A layout file's text, checked (see [`File::check`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self =
            toml::from_str(text).map_err(|err| err.to_string().trim_end().to_owned())?;
        file.check()?;
        Ok(file)
    }

    /// Keys the config can name, each once, none over another or over the
    /// touchpad, sizes positive, keys and touchpad on the deck, the logo on
    /// its lid, CHIDs as Windows writes them.
    pub fn check(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("no name".into());
        }
        if self.keys.is_empty() {
            return Err("no keys".into());
        }
        if let Some(chid) = self.chids.iter().find(|chid| !is_chid(chid)) {
            return Err(format!("chid {chid:?}: not a CHID"));
        }
        let known: BTreeSet<u8> = keyboard::names()
            .into_iter()
            .map(|(_, code)| code)
            .collect();
        for (at, key) in self.keys.iter().enumerate() {
            let what = format!("key {:#04x}", key.hid);
            if !known.contains(&key.hid) {
                return Err(format!("{what}: no such key"));
            }
            let earlier = &self.keys[..at];
            if earlier.iter().any(|other| other.hid == key.hid) {
                return Err(format!("{what}: twice"));
            }
            key.bounds().check(&what)?;
            if let Some(other) = earlier
                .iter()
                .find(|other| other.bounds().overlaps(&key.bounds()))
            {
                return Err(format!("{what}: over key {:#04x}", other.hid));
            }
        }
        if let Some(pad) = &self.touchpad {
            pad.bounds().check("the touchpad")?;
            if !(1..=lightbar::BAR_ROWS).contains(&pad.rows) {
                return Err(format!("the touchpad: rows, 1 to {}", lightbar::BAR_ROWS));
            }
            let least = 2.0 * PAD_INSET - STRIPE_GAP + STRIPE_GAP * f32::from(pad.rows);
            if pad.w <= 2.0 * PAD_INSET || pad.h <= least {
                return Err("the touchpad: too small for its rows".into());
            }
        }
        if let Some(logo) = &self.logo {
            logo.bounds().check("the logo")?;
            let inside = |from: f32, size: f32| from >= 0.0 && from + size <= 1.0 + 1e-4;
            if !(inside(logo.x, logo.w) && inside(logo.y, logo.h)) {
                return Err("the logo: off the lid, x, y, w and h are parts of it".into());
            }
            if !(logo.lid.is_finite() && logo.lid > 0.0) {
                return Err("the logo: lid, its width to its height, positive".into());
            }
        }
        if let Some(pad) = &self.touchpad
            && let Some(key) = self
                .keys
                .iter()
                .find(|key| key.bounds().overlaps(&pad.bounds()))
        {
            return Err(format!("the touchpad: over key {:#04x}", key.hid));
        }
        if let Some(deck) = &self.deck {
            deck.check("the deck")?;
            if let Some(key) = self.keys.iter().find(|key| !deck.holds(&key.bounds())) {
                return Err(format!("the deck: key {:#04x} off it", key.hid));
            }
            if let Some(pad) = &self.touchpad
                && !deck.holds(&pad.bounds())
            {
                return Err("the deck: the touchpad off it".into());
            }
        }
        Ok(())
    }

    /// For any laptop: no boards and no CHIDs.
    pub fn for_any(&self) -> bool {
        self.boards.is_empty() && self.chids.is_empty()
    }

    /// For the laptop of this DMI board name, or of this CHID.
    pub fn fits(&self, board: Option<&str>, chid: Option<&str>) -> bool {
        let board = board.map(str::trim);
        let chid = chid.map(str::trim);
        self.boards
            .iter()
            .any(|known| board.is_some_and(|board| known.trim().eq_ignore_ascii_case(board)))
            || self
                .chids
                .iter()
                .any(|known| chid.is_some_and(|chid| known.trim().eq_ignore_ascii_case(chid)))
    }
}

/// The namespace of Windows' CHIDs.
const CHID_NAMESPACE: [u8; 16] = [
    0x70, 0xff, 0xd8, 0x12, 0x4c, 0x7f, 0x4c, 0x7d, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The CHID Windows makes of a laptop's DMI manufacturer (`sys_vendor`)
/// and product name (`product_name`), HardwareID-9 of ComputerHardwareIds:
/// a version 5 UUID of the two joined by `&`, in UTF-16.
pub fn chid(manufacturer: &str, product: &str) -> String {
    let text = format!("{}&{}", manufacturer.trim(), product.trim());
    let mut hash = sha1_smol::Sha1::new();
    hash.update(&CHID_NAMESPACE);
    let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    hash.update(&utf16);
    let mut bytes: [u8; 16] = hash.digest().bytes()[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// `8-4-4-4-12` hex digits.
fn is_chid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(at, c)| match at {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSI: &str = "Micro-Star International Co., Ltd.";

    #[test]
    fn makes_chids_as_windows_does() {
        assert_eq!(
            chid(MSI, "Alpha 17 C7VG"),
            "532fd82f-3120-553c-acab-54599f390612"
        );
        assert_eq!(
            chid(MSI, " Titan 18 HX Dragon Edition Draco Epic A2WJ\n"),
            "a8a8a9eb-cb9b-5f46-a7ca-e77ca36a8e13"
        );
    }

    #[test]
    fn fits_by_board_or_chid() {
        let text = "name = \"x\"\nboards = [\"MS-1824\"]\nchids = [\"532FD82F-3120-553C-ACAB-54599F390612\"]\nkeys = [{ hid = 0x29, x = 0, y = 0, w = 1, h = 1 }]\n";
        let file = File::parse(text).unwrap();
        assert!(!file.for_any());
        assert!(file.fits(Some("ms-1824\n"), None));
        assert!(file.fits(None, Some(&chid(MSI, "Alpha 17 C7VG"))));
        assert!(!file.fits(Some("MS-17KK"), Some(&chid(MSI, "Alpha 17 C7VF"))));
        assert!(!file.fits(None, None));
        let bad = text.replace("532FD82F-", "532FD82F");
        assert!(File::parse(&bad).unwrap_err().contains("not a CHID"));
    }

    #[test]
    fn keeps_keys_and_touchpad_on_the_deck() {
        let text = "name = \"x\"\nkeys = [{ hid = 0x29, x = 0, y = 0, w = 1, h = 1 }]\n[touchpad]\nx = 0\ny = 2\nw = 3\nh = 2\n";
        let on = format!("{text}[deck]\nx = -1\ny = -1\nw = 5\nh = 6\n");
        assert!(File::parse(&on).unwrap().deck.is_some());
        let short = format!("{text}[deck]\nx = -1\ny = -1\nw = 5\nh = 4\n");
        assert_eq!(
            File::parse(&short).unwrap_err(),
            "the deck: the touchpad off it"
        );
        let narrow = format!("{text}[deck]\nx = 0.5\ny = -1\nw = 5\nh = 6\n");
        assert_eq!(
            File::parse(&narrow).unwrap_err(),
            "the deck: key 0x29 off it"
        );
    }
}
