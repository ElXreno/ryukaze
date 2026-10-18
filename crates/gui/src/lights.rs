//! The keyboard light's editor: a picture of the Alpha 17's keys to pick and
//! paint, an effect and its colours, and zones of keys with effects of
//! their own over it, shown on the keyboard as it goes (as a
//! preview the daemon drops at the next profile or scenario), saved with the
//! daemon's profiles, and copied out as Nix, a profile for
//! `keyboard.profiles`.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use iced::{
    Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme, alignment, mouse,
    widget::{
        button, canvas, column, container, row, slider, space, text,
        text::Alignment as TextAlignment, text_input,
    },
};
use ryukaze_core::keyboard::{self, Color, Origin};

use crate::{nix, theme};

/// A key on the picture: its HID usage, a label, and where it sits, in key
/// widths.
struct Key {
    code: u8,
    label: &'static str,
    rect: Rectangle,
}

/// The board's size in key widths.
const ACROSS: f32 = 19.25;
const DOWN: f32 = 6.0;
/// Where the numpad starts.
const NUMPAD: f32 = 15.25;

fn place(keys: &mut Vec<Key>, y: f32, height: f32, x: f32, row: &[(u8, &'static str, f32)]) {
    let mut at = x;
    for &(code, label, width) in row {
        keys.push(Key {
            code,
            label,
            rect: Rectangle::new(Point::new(at, y), Size::new(width, height)),
        });
        at += width;
    }
}

/// The Alpha 17 C7VG's keyboard, as the photos have it: a short row of
/// Esc, the F keys, the navigation keys and Power, then five rows and a
/// numpad of three columns and Plus and Enter.
fn board() -> Vec<Key> {
    let mut keys = Vec::new();
    let top: [(u8, &str); 20] = [
        (0x29, "Esc"),
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
        (0x47, "ScrLk"),
        (0x49, "Ins"),
        (0x4c, "Del"),
        (0x46, "PrtSc"),
        (0x4b, "PgUp"),
        (0x4e, "PgDn"),
        (0x66, "Power"),
    ];
    let width = ACROSS / top.len() as f32;
    place(
        &mut keys,
        0.0,
        0.8,
        0.0,
        &top.map(|(code, label)| (code, label, width)),
    );
    #[rustfmt::skip]
    place(&mut keys, 1.0, 1.0, 0.0, &[
        (0x35, "`", 1.0), (0x1e, "1", 1.0), (0x1f, "2", 1.0), (0x20, "3", 1.0),
        (0x21, "4", 1.0), (0x22, "5", 1.0), (0x23, "6", 1.0), (0x24, "7", 1.0),
        (0x25, "8", 1.0), (0x26, "9", 1.0), (0x27, "0", 1.0), (0x2d, "-", 1.0),
        (0x2e, "=", 1.0), (0x2a, "Backspace", 2.0),
    ]);
    #[rustfmt::skip]
    place(&mut keys, 2.0, 1.0, 0.0, &[
        (0x2b, "Tab", 1.5), (0x14, "Q", 1.0), (0x1a, "W", 1.0), (0x08, "E", 1.0),
        (0x15, "R", 1.0), (0x17, "T", 1.0), (0x1c, "Y", 1.0), (0x18, "U", 1.0),
        (0x0c, "I", 1.0), (0x12, "O", 1.0), (0x13, "P", 1.0), (0x2f, "[", 1.0),
        (0x30, "]", 1.0), (0x31, "\\", 1.5),
    ]);
    #[rustfmt::skip]
    place(&mut keys, 3.0, 1.0, 0.0, &[
        (0x39, "Caps", 1.75), (0x04, "A", 1.0), (0x16, "S", 1.0), (0x07, "D", 1.0),
        (0x09, "F", 1.0), (0x0a, "G", 1.0), (0x0b, "H", 1.0), (0x0d, "J", 1.0),
        (0x0e, "K", 1.0), (0x0f, "L", 1.0), (0x33, ";", 1.0), (0x34, "'", 1.0),
        (0x28, "Enter", 2.25),
    ]);
    #[rustfmt::skip]
    place(&mut keys, 4.0, 1.0, 0.0, &[
        (0xe1, "Shift", 2.25), (0x1d, "Z", 1.0), (0x1b, "X", 1.0), (0x06, "C", 1.0),
        (0x19, "V", 1.0), (0x05, "B", 1.0), (0x11, "N", 1.0), (0x10, "M", 1.0),
        (0x36, ",", 1.0), (0x37, ".", 1.0), (0x38, "/", 1.0), (0xe5, "Shift", 1.75),
        (0x52, "↑", 1.0),
    ]);
    #[rustfmt::skip]
    place(&mut keys, 5.0, 1.0, 0.0, &[
        (0xe0, "Ctrl", 1.25), (0xe3, "Win", 1.0), (0xe2, "Alt", 1.0), (0x2c, "", 4.75),
        (0xe6, "AltGr", 1.0), (0x64, "\\", 1.0), (0xf0, "Fn", 1.0), (0xe4, "Copilot", 1.0),
        (0x50, "←", 1.0), (0x51, "↓", 1.0), (0x4f, "→", 1.0),
    ]);
    #[rustfmt::skip]
    place(&mut keys, 1.0, 1.0, NUMPAD, &[
        (0x53, "Num", 1.0), (0x54, "/", 1.0), (0x55, "*", 1.0), (0x56, "-", 1.0),
    ]);
    place(
        &mut keys,
        2.0,
        1.0,
        NUMPAD,
        &[(0x5f, "7", 1.0), (0x60, "8", 1.0), (0x61, "9", 1.0)],
    );
    place(&mut keys, 2.0, 2.0, NUMPAD + 3.0, &[(0x57, "+", 1.0)]);
    place(
        &mut keys,
        3.0,
        1.0,
        NUMPAD,
        &[(0x5c, "4", 1.0), (0x5d, "5", 1.0), (0x5e, "6", 1.0)],
    );
    place(
        &mut keys,
        4.0,
        1.0,
        NUMPAD,
        &[(0x59, "1", 1.0), (0x5a, "2", 1.0), (0x5b, "3", 1.0)],
    );
    place(&mut keys, 4.0, 2.0, NUMPAD + 3.0, &[(0x58, "Enter", 1.0)]);
    place(
        &mut keys,
        5.0,
        1.0,
        NUMPAD,
        &[(0x62, "0", 2.0), (0x63, ".", 1.0)],
    );
    keys
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Static,
    Breathe,
    Cycle,
    Wave,
    Reactive,
}

impl Effect {
    const ALL: [Effect; 5] = [
        Effect::Static,
        Effect::Breathe,
        Effect::Cycle,
        Effect::Wave,
        Effect::Reactive,
    ];

    /// As the config writes it.
    fn name(self) -> &'static str {
        match self {
            Effect::Static => "static",
            Effect::Breathe => "breathe",
            Effect::Cycle => "cycle",
            Effect::Wave => "wave",
            Effect::Reactive => "reactive",
        }
    }

    /// What the daemon takes for an unset `period_ms`.
    fn period_ms(self) -> u32 {
        match self {
            Effect::Cycle => 6000,
            Effect::Wave => 3000,
            _ => 4000,
        }
    }
}

const ORIGINS: [(Origin, &str); 5] = [
    (Origin::Middle, "middle"),
    (Origin::Left, "left"),
    (Origin::Right, "right"),
    (Origin::Top, "top"),
    (Origin::Bottom, "bottom"),
];

/// Colours to pick at a click, as the LEDs show them rather than a screen.
pub const SWATCHES: [Color; 9] = [
    Color([255, 0, 0]),
    Color([255, 96, 0]),
    Color([255, 200, 0]),
    Color([0, 255, 0]),
    Color([0, 255, 255]),
    Color([0, 64, 255]),
    Color([128, 0, 255]),
    Color([255, 255, 255]),
    Color::OFF,
];

/// Groups to pick keys by, as the config names them.
const GROUPS: [(&str, &str); 8] = [
    ("letters", "Letters"),
    ("digits", "Digits"),
    ("function", "F keys"),
    ("navigation", "Navigation"),
    ("arrows", "Arrows"),
    ("numpad", "Numpad"),
    ("modifiers", "Modifiers"),
    ("wasd", "WASD"),
];

/// What the colour sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The picked keys' own.
    Keys,
    /// The effect's: the keys without their own, still, breathing or
    /// between flashes.
    Color,
    FadeTo,
    Flash,
    /// One of a cycle's or a wave's colours.
    Round(usize),
}

/// An effect and what it takes: the profile's, or a zone's.
#[derive(Debug, Clone, PartialEq)]
struct Look {
    effect: Effect,
    color: Color,
    fade_to: Color,
    flash: Color,
    /// A cycle's or a wave's; none, the rainbow.
    colors: Vec<Color>,
    period_ms: u32,
    flash_ms: u32,
    origin: Origin,
    inward: bool,
    wavelength: u8,
}

impl Look {
    fn new(effect: Effect) -> Self {
        let white = Color([255; 3]);
        Self {
            effect,
            color: white,
            fade_to: Color::OFF,
            flash: white,
            colors: Vec::new(),
            period_ms: effect.period_ms(),
            flash_ms: 500,
            origin: Origin::Middle,
            inward: false,
            wavelength: keyboard::Wave::default().wavelength,
        }
    }

    /// As a profile or a zone has it in TOML; a cycle or a wave has no
    /// colour of the keys' unless told.
    fn read(table: &toml::Table) -> Result<Self, String> {
        let word = |key: &str| table.get(key).and_then(toml::Value::as_str);
        let number = |key: &str| table.get(key).and_then(toml::Value::as_integer);
        let field = |key: &str| table.get(key).map(read_color).transpose();
        let effect = match word("effect") {
            Some(name) => Effect::ALL
                .into_iter()
                .find(|effect| effect.name() == name)
                .ok_or_else(|| format!("no effect is called {name:?}"))?,
            None => Effect::Static,
        };
        let mut look = Self::new(effect);
        let unset = match effect {
            Effect::Cycle | Effect::Wave => Color::OFF,
            _ => look.color,
        };
        look.color = field("color")?.unwrap_or(unset);
        look.fade_to = field("fade_to")?.unwrap_or(Color::OFF);
        look.flash = field("flash")?.unwrap_or(look.flash);
        if let Some(colors) = table.get("colors").and_then(toml::Value::as_array) {
            look.colors = colors.iter().map(read_color).collect::<Result<_, _>>()?;
        }
        if let Some(ms) = number("period_ms") {
            look.period_ms = ms as u32;
        }
        if let Some(ms) = number("flash_ms") {
            look.flash_ms = ms as u32;
        }
        if let Some(name) = word("origin") {
            look.origin = ORIGINS
                .iter()
                .find(|(_, known)| *known == name)
                .map_or(Origin::Middle, |(origin, _)| *origin);
        }
        look.inward = word("direction") == Some("in");
        if let Some(wavelength) = number("wavelength") {
            look.wavelength = wavelength as u8;
        }
        Ok(look)
    }

    /// A cycle's or a wave's colours: its own, or the rainbow.
    fn round(&self) -> Vec<Color> {
        if self.colors.len() >= *keyboard::COLORS.start() {
            self.colors.clone()
        } else {
            keyboard::RAINBOW.to_vec()
        }
    }

    /// The period, at least what the colours need.
    fn period(&self) -> u32 {
        let shortest = match self.effect {
            Effect::Cycle | Effect::Wave => keyboard::shortest_period_ms(self.round().len()),
            _ => *keyboard::PERIODS_MS.start(),
        };
        self.period_ms.clamp(shortest, *keyboard::PERIODS_MS.end())
    }

    /// Its colours, as the picker sets them.
    fn slots(&self) -> Vec<Slot> {
        match self.effect {
            Effect::Static => vec![Slot::Color],
            Effect::Breathe => vec![Slot::Color, Slot::FadeTo],
            Effect::Cycle | Effect::Wave => (0..self.round().len()).map(Slot::Round).collect(),
            Effect::Reactive => vec![Slot::Color, Slot::Flash],
        }
    }

    fn get(&self, slot: Slot) -> Color {
        match slot {
            Slot::Keys | Slot::Color => self.color,
            Slot::FadeTo => self.fade_to,
            Slot::Flash => self.flash,
            Slot::Round(i) => self.round().get(i).copied().unwrap_or(self.color),
        }
    }

    fn set(&mut self, slot: Slot, color: Color) {
        match slot {
            Slot::Keys => {}
            Slot::Color => self.color = color,
            Slot::FadeTo => self.fade_to = color,
            Slot::Flash => self.flash = color,
            Slot::Round(i) => {
                let mut round = self.round();
                if let Some(at) = round.get_mut(i) {
                    *at = color;
                    self.colors = round;
                }
            }
        }
    }

    /// As the config takes it, field by field: a profile's, or a zone's,
    /// which is never still.
    fn fields(&self, profile: bool) -> Vec<(&'static str, Field)> {
        let hex = |color: Color| Field::Str(color.to_string());
        let mut fields = Vec::new();
        if self.effect != Effect::Static || !profile {
            fields.push(("effect", Field::Str(self.effect.name().into())));
        }
        match self.effect {
            Effect::Static => fields.push(("color", hex(self.color))),
            Effect::Breathe => {
                fields.push(("color", hex(self.color)));
                if self.fade_to != Color::OFF {
                    fields.push(("fade_to", hex(self.fade_to)));
                }
                fields.push(("period_ms", Field::Int(self.period())));
            }
            Effect::Cycle | Effect::Wave => {
                // As it was read, though a cycle or a wave goes without.
                if profile && self.color != Color::OFF {
                    fields.push(("color", hex(self.color)));
                }
                if self.colors.len() >= *keyboard::COLORS.start() {
                    let colors = self.colors.iter().map(|color| color.to_string());
                    fields.push(("colors", Field::Strs(colors.collect())));
                }
                fields.push(("period_ms", Field::Int(self.period())));
                if self.effect == Effect::Wave {
                    if self.origin != Origin::Middle {
                        let name = ORIGINS.iter().find(|(origin, _)| *origin == self.origin);
                        fields.push(("origin", Field::Str(name.map_or("middle", |o| o.1).into())));
                    }
                    if self.inward {
                        fields.push(("direction", Field::Str("in".into())));
                    }
                    if self.wavelength != keyboard::Wave::default().wavelength {
                        fields.push(("wavelength", Field::Int(self.wavelength.into())));
                    }
                }
            }
            Effect::Reactive => {
                fields.push(("color", hex(self.color)));
                fields.push(("flash", hex(self.flash)));
                fields.push(("flash_ms", Field::Int(self.flash_ms)));
            }
        }
        fields
    }

    /// The colour at a point of the board, in key widths: the effect's at
    /// about where it is.
    fn at(&self, at: Point) -> Color {
        match self.effect {
            Effect::Cycle => self.round()[0],
            // From one colour to the next, as the controller fades them.
            Effect::Wave => {
                let round = self.round();
                let along = self.along(at) * round.len() as f32;
                let from = along as usize % round.len();
                let to = round[(from + 1) % round.len()];
                let mix = along.fract();
                Color(std::array::from_fn(|c| {
                    let (from, to) = (f32::from(round[from].0[c]), f32::from(to.0[c]));
                    (from + (to - from) * mix).round() as u8
                }))
            }
            _ => self.color,
        }
    }

    /// How many rounds of a wave's colours lie between its origin and a
    /// point, in key widths.
    fn along(&self, at: Point) -> f32 {
        let from = match self.origin {
            Origin::Middle => (at.x - ACROSS / 2.0).hypot(at.y - DOWN / 2.0),
            Origin::Left => at.x,
            Origin::Right => ACROSS - at.x,
            Origin::Top => at.y,
            Origin::Bottom => DOWN - at.y,
        };
        from / (ACROSS * f32::from(self.wavelength) / 100.0)
    }
}

/// Keys with an effect of their own beside the profile's.
#[derive(Debug, Clone, PartialEq)]
struct Zone {
    keys: BTreeSet<u8>,
    look: Look,
}

pub struct Editor {
    name: String,
    /// The profile's effect, and its zones', a later over an earlier.
    look: Look,
    zones: Vec<Zone>,
    /// What the panels and the picture change: the profile, 0, or a zone.
    layer: usize,
    /// Keys with colours of their own.
    keys: BTreeMap<u8, Color>,
    picked: BTreeSet<u8>,
    slot: Slot,
    /// The colour as typed.
    hex: String,
    /// Changed since it was opened or saved.
    unsaved: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// A key picked, or let go, on the picture; put in the zone shown, or
    /// taken out of it.
    Pick(u8, bool),
    PickAll,
    PickNone,
    PickGroup(&'static str),
    /// The profile, 0, or a zone.
    Layer(usize),
    AddZone,
    RemoveZone,
    Slot(Slot),
    Swatch(Color),
    Hex(String),
    Channel(usize, u8),
    /// The picked keys back to the effect.
    Unpaint,
    Effect(Effect),
    AddColor,
    RemoveColor,
    Rainbow,
    Period(u32),
    FlashMs(u32),
    Wavelength(u8),
    /// A slider let go: show what it set.
    Released,
    Origin(Origin),
    Inward(bool),
    Name(String),
    /// A profile of its own, from scratch.
    New,
    /// Another of the daemon's, as it is.
    Open(String),
    Save,
    /// Saved under this name instead: a standard one's copy.
    SaveAs(String),
    Delete,
    Copy,
    Done,
}

/// What a change asks of the window.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// Show this profile, TOML, on the keyboard.
    Preview(String),
    /// Start a new profile instead.
    New,
    /// Open this one of the daemon's instead.
    Open(String),
    /// Keep this profile, TOML, with the daemon's.
    Save(String),
    /// Forget the saved profile of this name.
    Delete(String),
    /// Put this on the clipboard.
    Copy(String),
    Done,
}

impl Editor {
    pub fn new() -> Self {
        Self::named("custom")
    }

    /// All white and still, under `name`.
    pub fn named(name: &str) -> Self {
        let look = Look::new(Effect::Static);
        let hex = look.color.to_string();
        Self {
            name: name.into(),
            look,
            zones: Vec::new(),
            layer: 0,
            keys: BTreeMap::new(),
            picked: BTreeSet::new(),
            slot: Slot::Color,
            hex,
            unsaved: false,
        }
    }

    /// A profile as the daemon reads one out, in TOML.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let table: toml::Table = text
            .parse()
            .map_err(|err: toml::de::Error| err.to_string())?;
        let mut editor = Self::new();
        if let Some(name) = table.get("name").and_then(toml::Value::as_str) {
            name.clone_into(&mut editor.name);
        }
        editor.look = Look::read(&table)?;

        // The larger groups first, so the smaller and single keys win.
        let mut keys = Vec::new();
        for (name, value) in table
            .get("keys")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flatten()
        {
            keys.push((named(name)?, read_color(value)?));
        }
        keys.sort_by_key(|(codes, _)| Reverse(codes.len()));
        for (codes, color) in keys {
            editor
                .keys
                .extend(codes.into_iter().map(|code| (code, color)));
        }

        for zone in table
            .get("zones")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let zone = zone.as_table().ok_or("a zone that isn't a table")?;
            let mut keys = BTreeSet::new();
            for name in zone
                .get("keys")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
            {
                keys.extend(named(name)?);
            }
            editor.zones.push(Zone {
                keys,
                look: Look::read(zone)?,
            });
        }
        editor.hex = editor.look.color.to_string();
        Ok(editor)
    }

    /// The effect the panels change: the profile's or the zone shown's.
    fn look(&self) -> &Look {
        match self.layer {
            0 => &self.look,
            zone => &self.zones[zone - 1].look,
        }
    }

    fn look_mut(&mut self) -> &mut Look {
        match self.layer {
            0 => &mut self.look,
            zone => &mut self.zones[zone - 1].look,
        }
    }

    /// The keys outlined on the picture: those picked, or the zone's.
    fn marked(&self, key: u8) -> bool {
        match self.layer {
            0 => self.picked.contains(&key),
            zone => self.zones[zone - 1].keys.contains(&key),
        }
    }

    pub fn update(&mut self, msg: Msg) -> Outcome {
        match msg {
            Msg::Pick(key, on) => {
                if self.layer > 0 {
                    let keys = &mut self.zones[self.layer - 1].keys;
                    if on {
                        keys.insert(key);
                    } else {
                        keys.remove(&key);
                    }
                } else {
                    if on {
                        self.picked.insert(key);
                    } else {
                        self.picked.remove(&key);
                    }
                    self.picking_changed();
                    return Outcome::Nothing;
                }
            }
            Msg::PickAll | Msg::PickNone | Msg::PickGroup(_) => {
                let keys: Vec<u8> = match msg {
                    Msg::PickAll => board().iter().map(|key| key.code).collect(),
                    Msg::PickGroup(group) => keyboard::named(group).unwrap_or_default(),
                    _ => Vec::new(),
                };
                let clear = matches!(msg, Msg::PickNone);
                if self.layer > 0 {
                    let zone = &mut self.zones[self.layer - 1].keys;
                    if clear {
                        zone.clear();
                    }
                    zone.extend(keys);
                } else {
                    if clear {
                        self.picked.clear();
                    }
                    self.picked.extend(keys);
                    self.picking_changed();
                    return Outcome::Nothing;
                }
            }
            Msg::Layer(layer) => {
                self.layer = layer.min(self.zones.len());
                self.picked.clear();
                self.slot = self.look().slots()[0];
                self.hex = self.slot_color().to_string();
                return Outcome::Nothing;
            }
            Msg::AddZone => {
                if self.zones.len() >= keyboard::ZONES {
                    return Outcome::Nothing;
                }
                self.zones.push(Zone {
                    keys: BTreeSet::new(),
                    look: Look::new(Effect::Breathe),
                });
                self.picked.clear();
                self.layer = self.zones.len();
                self.slot = Slot::Color;
                self.hex = self.slot_color().to_string();
                return Outcome::Nothing;
            }
            Msg::RemoveZone => {
                if self.layer == 0 {
                    return Outcome::Nothing;
                }
                self.zones.remove(self.layer - 1);
                self.layer -= 1;
                self.slot = self.look().slots()[0];
                self.hex = self.slot_color().to_string();
            }
            Msg::Slot(slot) => {
                self.picked.clear();
                self.slot = slot;
                self.hex = self.slot_color().to_string();
                return Outcome::Nothing;
            }
            Msg::Swatch(color) => {
                self.set_slot_color(color);
                self.hex = color.to_string();
            }
            Msg::Hex(hex) => {
                let parsed = hex.parse();
                self.hex = hex;
                let Ok(color) = parsed else {
                    return Outcome::Nothing;
                };
                self.set_slot_color(color);
            }
            Msg::Channel(channel, value) => {
                // Shown on the picture at once, on the keyboard once let go.
                let mut color = self.slot_color();
                color.0[channel] = value;
                self.set_slot_color(color);
                self.hex = color.to_string();
                self.unsaved = true;
                return Outcome::Nothing;
            }
            Msg::Unpaint => {
                for key in &self.picked {
                    self.keys.remove(key);
                }
                self.hex = self.slot_color().to_string();
            }
            Msg::Effect(effect) => {
                // A zone is never still: still colours are the keys' own.
                if self.layer > 0 && effect == Effect::Static {
                    return Outcome::Nothing;
                }
                self.look_mut().effect = effect;
                if self.slot != Slot::Keys && !self.look().slots().contains(&self.slot) {
                    self.slot = self.look().slots()[0];
                }
                self.hex = self.slot_color().to_string();
            }
            Msg::AddColor => {
                let mut round = self.look().round();
                if round.len() >= *keyboard::COLORS.end() {
                    return Outcome::Nothing;
                }
                // A copy of the last to change.
                round.push(round[round.len() - 1]);
                self.slot = Slot::Round(round.len() - 1);
                self.look_mut().colors = round;
                self.picked.clear();
            }
            Msg::RemoveColor => {
                let mut round = self.look().round();
                let Slot::Round(i) = self.slot else {
                    return Outcome::Nothing;
                };
                if round.len() <= *keyboard::COLORS.start() || i >= round.len() {
                    return Outcome::Nothing;
                }
                round.remove(i);
                self.slot = Slot::Round(i.min(round.len() - 1));
                self.look_mut().colors = round;
                self.hex = self.slot_color().to_string();
            }
            Msg::Rainbow => {
                self.look_mut().colors.clear();
                if let Slot::Round(i) = self.slot {
                    self.slot = Slot::Round(i.min(keyboard::RAINBOW.len() - 1));
                }
                self.hex = self.slot_color().to_string();
            }
            Msg::Period(period_ms) => {
                self.look_mut().period_ms = period_ms;
                self.unsaved = true;
                return Outcome::Nothing;
            }
            Msg::FlashMs(flash_ms) => {
                self.look_mut().flash_ms = flash_ms;
                self.unsaved = true;
                return Outcome::Nothing;
            }
            Msg::Wavelength(wavelength) => {
                self.look_mut().wavelength = wavelength;
                self.unsaved = true;
                return Outcome::Nothing;
            }
            Msg::Released => {}
            Msg::Origin(origin) => self.look_mut().origin = origin,
            Msg::Inward(inward) => self.look_mut().inward = inward,
            Msg::Name(name) => {
                self.name = name;
                self.unsaved = true;
                return Outcome::Nothing;
            }
            Msg::Save => {
                return if self.name.trim().is_empty() {
                    Outcome::Nothing
                } else {
                    Outcome::Save(self.toml())
                };
            }
            Msg::New => return Outcome::New,
            Msg::Open(name) => return Outcome::Open(name),
            Msg::SaveAs(name) => {
                self.name = name;
                return Outcome::Save(self.toml());
            }
            Msg::Delete => return Outcome::Delete(self.name.trim().to_owned()),
            Msg::Copy => return Outcome::Copy(self.nix()),
            Msg::Done => return Outcome::Done,
        }
        self.unsaved = true;
        Outcome::Preview(self.toml())
    }

    pub fn name(&self) -> &str {
        self.name.trim()
    }

    /// What the keyboard should show as the editor opens.
    pub fn preview(&self) -> String {
        self.toml()
    }

    /// Changed since it was opened or saved.
    pub fn unsaved(&self) -> bool {
        self.unsaved
    }

    /// The daemon took it.
    pub fn saved(&mut self) {
        self.unsaved = false;
    }

    /// The daemon forgot the saved one: what's here is all there is.
    pub fn deleted(&mut self) {
        self.unsaved = true;
    }

    /// Picked keys are what the colour paints; none, the effect's colour.
    fn picking_changed(&mut self) {
        if !self.picked.is_empty() {
            self.slot = Slot::Keys;
        } else if self.slot == Slot::Keys {
            self.slot = Slot::Color;
        }
        self.hex = self.slot_color().to_string();
    }

    fn slot_color(&self) -> Color {
        match self.slot {
            Slot::Keys => self
                .picked
                .first()
                .and_then(|key| self.keys.get(key))
                .copied()
                .unwrap_or(self.look.color),
            slot => self.look().get(slot),
        }
    }

    fn set_slot_color(&mut self, color: Color) {
        match self.slot {
            Slot::Keys => {
                for &key in &self.picked {
                    self.keys.insert(key, color);
                }
            }
            slot => self.look_mut().set(slot, color),
        }
    }

    /// The profile as the config takes it, field by field.
    fn fields(&self) -> Vec<(&'static str, Field)> {
        let mut fields = vec![("name", Field::Str(self.name.trim().to_owned()))];
        fields.extend(self.look.fields(true));
        let keys = self.key_names();
        if !keys.is_empty() {
            fields.push(("keys", Field::Keys(keys)));
        }
        // A zone without keys is none.
        let zones: Vec<Vec<(&'static str, Field)>> = self
            .zones
            .iter()
            .filter(|zone| !zone.keys.is_empty())
            .map(|zone| {
                let mut fields = vec![("keys", Field::Strs(names_of(&zone.keys)))];
                fields.extend(zone.look.fields(false));
                fields
            })
            .collect();
        if !zones.is_empty() {
            fields.push(("zones", Field::Zones(zones)));
        }
        fields
    }

    /// The keys with their own colours by name: a group by its name where
    /// all of it has one colour, the larger groups tried first.
    fn key_names(&self) -> Vec<(String, String)> {
        let mut left = self.keys.clone();
        let mut named = Vec::new();
        for (group, keys) in groups() {
            let Some(&color) = keys.first().and_then(|key| left.get(key)) else {
                continue;
            };
            if keys.iter().all(|key| left.get(key) == Some(&color)) {
                for key in &keys {
                    left.remove(key);
                }
                named.push((group.to_owned(), color.to_string()));
            }
        }
        let names = keyboard::names();
        named.extend(left.iter().filter_map(|(code, color)| {
            let (name, _) = names.iter().find(|(_, known)| known == code)?;
            Some((name.clone(), color.to_string()))
        }));
        named
    }

    fn toml(&self) -> String {
        let table: toml::Table = self
            .fields()
            .into_iter()
            .map(|(key, field)| (key.to_owned(), field.toml()))
            .collect();
        toml::to_string(&table).unwrap_or_default()
    }

    /// For `services.ryukaze.settings.keyboard.profiles`.
    fn nix(&self) -> String {
        let entries: Vec<(String, nix::Value)> = self
            .fields()
            .into_iter()
            .map(|(key, field)| (key.to_owned(), field.nix()))
            .collect();
        nix::attrset(&entries)
    }

    /// The colour the picture gives a key: its own, or its zone's or the
    /// effect's at about where it sits.
    fn shown(&self, key: &Key) -> Color {
        if let Some(&color) = self.keys.get(&key.code) {
            return color;
        }
        let at = key.rect.center();
        match self
            .zones
            .iter()
            .rev()
            .find(|zone| zone.keys.contains(&key.code))
        {
            Some(zone) => zone.look.at(at),
            None => self.look.at(at),
        }
    }

    /// In `width`: the picture with the panels at its side, or under it in
    /// a narrow window, and the daemon's profiles to open.
    pub fn view<'a>(&'a self, width: f32, known: &Known) -> Element<'a, Msg> {
        const SIDE: f32 = 400.0;
        const GAP: f32 = 12.0;
        let name = self.name();
        let standard = known.standard.iter().any(|known| known == name);
        let saved = known.saved.iter().any(|known| known == name);
        // A standard one stays as it is: saved, it's a copy.
        let saving = if name.is_empty() {
            None
        } else if standard {
            let taken = |copy: &String| known.standard.contains(copy) || known.saved.contains(copy);
            (2..)
                .map(|n| format!("{name} {n}"))
                .find(|copy| !taken(copy))
                .map(Msg::SaveAs)
        } else {
            Some(Msg::Save)
        };
        let note = if standard {
            "a standard profile: Save keeps a copy"
        } else if self.unsaved {
            "changed, not saved"
        } else {
            ""
        };
        let title = row![
            text("Keyboard light").size(20),
            space().width(8),
            text("Profile").size(13).style(theme::muted),
            text_input("a name to save it by", &self.name)
                .on_input(Msg::Name)
                .on_submit_maybe(saving.clone())
                .size(13)
                .width(180),
            text(note).size(12).style(theme::muted),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .wrap()
        .vertical_spacing(6);
        let actions = row![
            button(text("Save").size(13))
                .on_press_maybe(saving)
                .padding([4, 10])
                .style(theme::segment(self.unsaved)),
            button(text("Delete").size(13))
                .on_press_maybe(saved.then_some(Msg::Delete))
                .padding([4, 10])
                .style(theme::segment(false)),
            small("Copy as Nix", Msg::Copy),
            small("Done", Msg::Done),
        ]
        .spacing(8);
        // The buttons on a line of their own in a narrow window.
        let header: Element<'a, Msg> = if width >= 1000.0 {
            row![title, space::horizontal(), actions]
                .align_y(Alignment::Center)
                .into()
        } else {
            column![title, actions].spacing(8).into()
        };

        // The profiles there are, to open one instead.
        let chip = |profile: &String| -> Element<'a, Msg> {
            let open = profile == name;
            button(text(theme_label(profile)).size(13))
                .on_press_maybe((!open).then(|| Msg::Open(profile.clone())))
                .padding([4, 10])
                .style(theme::segment(open))
                .into()
        };
        let mut profiles = row![text("Standard").size(12).style(theme::muted)]
            .spacing(6)
            .align_y(Alignment::Center);
        for profile in &known.standard {
            profiles = profiles.push(chip(profile));
        }
        profiles = profiles.push(space().width(12));
        if !known.saved.is_empty() {
            profiles = profiles.push(text("Saved here").size(12).style(theme::muted));
        }
        for profile in &known.saved {
            profiles = profiles.push(chip(profile));
        }
        let profiles = profiles
            .push(small("New", Msg::New))
            .wrap()
            .vertical_spacing(6);

        let wide = width >= 1000.0;
        let across = if wide { width - SIDE - GAP } else { width };
        let picture = canvas(Board { editor: self })
            .width(Length::Fill)
            .height((across * DOWN / ACROSS).min(440.0));
        let board = column![picture, self.picking()].spacing(GAP);
        let body: Element<'_, Msg> = if wide {
            let panels = column![self.effect_panel(), self.colour_panel()].spacing(GAP);
            row![board.width(Length::Fill), panels.width(SIDE)]
                .spacing(GAP)
                .into()
        } else if width >= 2.0 * 340.0 + GAP {
            let panels = row![self.effect_panel(), self.colour_panel()].spacing(GAP);
            column![board, panels].spacing(GAP).into()
        } else {
            let panels = column![self.effect_panel(), self.colour_panel()].spacing(GAP);
            column![board, panels].spacing(GAP).into()
        };

        let hint = String::from(
            "Shown on the keyboard as you go. Save keeps it with the profiles, here on this \
             computer, and Delete forgets it again; Copy as Nix puts it on the clipboard for \
             services.ryukaze.settings.keyboard.profiles.",
        );
        column![
            header,
            profiles,
            body,
            text(hint).size(12).style(theme::muted)
        ]
        .spacing(GAP)
        .into()
    }

    /// Under the picture: picking by group.
    fn picking(&self) -> Element<'_, Msg> {
        let mut groups = row![small("All", Msg::PickAll), small("None", Msg::PickNone)].spacing(6);
        for (group, label) in GROUPS {
            groups = groups.push(small(label, Msg::PickGroup(group)));
        }
        let note = match (self.layer, self.picked.len()) {
            (0, 0) => {
                "Click keys, or drag over them, to pick them and paint them in a colour of their own"
                    .to_owned()
            }
            (0, 1) => "1 key picked: the colour paints it".to_owned(),
            (0, n) => format!("{n} keys picked: the colour paints them"),
            (zone, _) => format!(
                "Click keys, or drag over them, to put them in zone {zone} or take them out; \
                 a later zone is over an earlier, and a key's own colour over both"
            ),
        };
        column![
            groups.wrap().vertical_spacing(6),
            text(note).size(13).style(theme::muted),
        ]
        .spacing(8)
        .into()
    }

    /// The layers, the profile and its zones, and the effect of the one
    /// shown.
    fn effect_panel(&self) -> Element<'_, Msg> {
        let look = self.look();
        let layer = |label: String, layer: usize| -> Element<'_, Msg> {
            let chosen = self.layer == layer;
            button(text(label).size(13))
                .on_press_maybe((!chosen).then_some(Msg::Layer(layer)))
                .padding([4, 10])
                .style(theme::segment(chosen))
                .into()
        };
        let mut layers = row![layer("Keyboard".into(), 0)]
            .spacing(4)
            .align_y(Alignment::Center);
        for zone in 1..=self.zones.len() {
            layers = layers.push(layer(format!("Zone {zone}"), zone));
        }
        if self.zones.len() < keyboard::ZONES {
            layers = layers.push(small("+ Zone", Msg::AddZone));
        }
        let effects = row(Effect::ALL
            .into_iter()
            .filter(|&effect| self.layer == 0 || effect != Effect::Static)
            .map(|effect| {
                choice(
                    theme_label(effect.name()),
                    look.effect == effect,
                    Msg::Effect(effect),
                )
            }))
        .spacing(4);
        let period = self.level(
            "Period",
            slider(keyboard::PERIODS_MS.clone(), look.period(), Msg::Period)
                .step(500u32)
                .on_release(Msg::Released)
                .style(theme::level),
            format!("{:.1} s", look.period() as f32 / 1000.0),
        );

        let mut controls = column![layers.wrap().vertical_spacing(4)].spacing(10);
        if self.layer > 0 {
            let zone = &self.zones[self.layer - 1];
            let count = match zone.keys.len() {
                0 => "no keys yet: click them on the picture".to_owned(),
                1 => "1 key".to_owned(),
                n => format!("{n} keys"),
            };
            controls = controls.push(
                row![
                    text(format!("Zone {}: {count}", self.layer))
                        .size(13)
                        .style(theme::muted),
                    space::horizontal(),
                    small("Remove the zone", Msg::RemoveZone),
                ]
                .align_y(Alignment::Center),
            );
        }
        controls = controls.push(effects);
        match look.effect {
            Effect::Static => {
                controls = controls.push(self.slot("The keys", Slot::Color));
            }
            Effect::Breathe => {
                controls = controls
                    .push(self.slot("Breathing", Slot::Color))
                    .push(self.slot("Fading to", Slot::FadeTo))
                    .push(period);
            }
            Effect::Cycle | Effect::Wave => {
                let round = look.round();
                let mut colors = row(round
                    .iter()
                    .enumerate()
                    .map(|(i, &color)| self.dot(color, Slot::Round(i))))
                .spacing(6)
                .align_y(Alignment::Center);
                if round.len() < *keyboard::COLORS.end() {
                    colors = colors.push(small("+", Msg::AddColor));
                }
                let mut tools = row![].spacing(6);
                if matches!(self.slot, Slot::Round(_)) && round.len() > *keyboard::COLORS.start() {
                    tools = tools.push(small("Remove", Msg::RemoveColor));
                }
                if !look.colors.is_empty() {
                    tools = tools.push(small("Rainbow", Msg::Rainbow));
                }
                controls = controls
                    .push(colors.wrap().vertical_spacing(6))
                    .push(tools)
                    .push(period);
                if look.effect == Effect::Wave {
                    let origins = row(ORIGINS.map(|(origin, name)| {
                        choice(
                            theme_label(name),
                            look.origin == origin,
                            Msg::Origin(origin),
                        )
                    }))
                    .spacing(4);
                    let direction = row![
                        choice("Outward".into(), !look.inward, Msg::Inward(false)),
                        choice("Inward".into(), look.inward, Msg::Inward(true)),
                    ]
                    .spacing(4);
                    let wavelength = self.level(
                        "Width",
                        slider(
                            keyboard::WAVELENGTHS.clone(),
                            look.wavelength,
                            Msg::Wavelength,
                        )
                        .on_release(Msg::Released)
                        .style(theme::level),
                        format!("{} %", look.wavelength),
                    );
                    controls = controls.push(origins).push(direction).push(wavelength);
                }
            }
            Effect::Reactive => {
                let flash_ms = self.level(
                    "Flash",
                    slider(keyboard::FLASHES_MS.clone(), look.flash_ms, Msg::FlashMs)
                        .step(100u32)
                        .on_release(Msg::Released)
                        .style(theme::level),
                    format!("{:.1} s", look.flash_ms as f32 / 1000.0),
                );
                controls = controls
                    .push(self.slot("The keys", Slot::Color))
                    .push(self.slot("A pressed key", Slot::Flash))
                    .push(flash_ms)
                    .push(
                        text("A key flashes as it's pressed and again as it's let go.")
                            .size(12)
                            .style(theme::muted),
                    );
            }
        }
        panel("Effect", controls)
    }

    /// One of the effect's colours, to pick for the colour to set.
    fn slot<'a>(&self, label: &'a str, slot: Slot) -> Element<'a, Msg> {
        let color = self.look().get(slot);
        button(
            row![swatch(color, 20.0), text(label).size(13)]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .on_press(Msg::Slot(slot))
        .padding([6, 10])
        .width(Length::Fill)
        .style(theme::segment(self.slot == slot))
        .into()
    }

    /// A round's colour, to pick for the colour to set.
    fn dot<'a>(&self, color: Color, slot: Slot) -> Element<'a, Msg> {
        let chosen = self.slot == slot;
        button(space().width(26).height(26))
            .padding(0)
            .on_press(Msg::Slot(slot))
            .style(move |theme: &Theme, _| {
                let colors = theme::colors(theme);
                button::Style {
                    background: Some(led(color).into()),
                    border: iced::Border::default()
                        .rounded(7)
                        .width(if chosen { 3 } else { 1 })
                        .color(if chosen { colors.strong } else { colors.border }),
                    ..button::Style::default()
                }
            })
            .into()
    }

    /// A slider with its name and value; its change shown on the keyboard
    /// once let go.
    fn level<'a>(
        &self,
        label: &'a str,
        slider: impl Into<Element<'a, Msg>>,
        value: String,
    ) -> Element<'a, Msg> {
        row![
            text(label).size(13).width(56),
            slider.into(),
            text(value).size(13).width(48).align_x(Alignment::End),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    fn colour_panel(&self) -> Element<'_, Msg> {
        let color = self.slot_color();
        let what = match self.slot {
            Slot::Keys if self.picked.len() == 1 => "The picked key's own".to_owned(),
            Slot::Keys => format!("The {} picked keys' own", self.picked.len()),
            Slot::Color => match self.look().effect {
                Effect::Breathe => "Breathing".to_owned(),
                _ => "The keys'".to_owned(),
            },
            Slot::FadeTo => "Fading to".to_owned(),
            Slot::Flash => "A pressed key's".to_owned(),
            Slot::Round(i) => format!("Colour {} of the round", i + 1),
        };
        let channel = |index: usize, name: &'static str| {
            row![
                text(name).size(13).width(16),
                slider(0..=255, color.0[index], move |value| Msg::Channel(
                    index, value
                ))
                .on_release(Msg::Released)
                .style(theme::level),
                text(color.0[index].to_string())
                    .size(13)
                    .width(32)
                    .align_x(Alignment::End),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        };
        let swatches = row(SWATCHES.map(|color| {
            button(space().width(24).height(24))
                .padding(0)
                .on_press(Msg::Swatch(color))
                .style(move |theme: &Theme, _| button::Style {
                    background: Some(led(color).into()),
                    border: iced::Border::default()
                        .rounded(6)
                        .width(1)
                        .color(theme::colors(theme).border),
                    ..button::Style::default()
                })
                .into()
        }))
        .spacing(6)
        .wrap()
        .vertical_spacing(6);
        let mut top = row![
            swatch(color, 32.0),
            text_input("#rrggbb", &self.hex)
                .on_input(Msg::Hex)
                .size(13)
                .width(100),
            space::horizontal(),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if self.slot == Slot::Keys {
            top = top.push(small("Back to the effect", Msg::Unpaint));
        }
        panel(
            "Colour",
            column![
                text(what).size(13),
                top,
                channel(0, "R"),
                channel(1, "G"),
                channel(2, "B"),
                swatches,
            ]
            .spacing(10),
        )
    }
}

/// The daemon's keyboard light profiles: the standard ones and the
/// config's, which stay as they are, and those saved here.
pub struct Known {
    pub standard: Vec<String>,
    pub saved: Vec<String>,
}

/// A profile's field, for TOML and Nix alike.
enum Field {
    Str(String),
    Int(u32),
    Strs(Vec<String>),
    Keys(Vec<(String, String)>),
    Zones(Vec<Vec<(&'static str, Field)>>),
}

impl Field {
    fn toml(self) -> toml::Value {
        match self {
            Field::Str(text) => toml::Value::String(text),
            Field::Int(n) => toml::Value::Integer(n.into()),
            Field::Strs(texts) => {
                toml::Value::Array(texts.into_iter().map(toml::Value::String).collect())
            }
            Field::Keys(keys) => toml::Value::Table(
                keys.into_iter()
                    .map(|(name, color)| (name, toml::Value::String(color)))
                    .collect(),
            ),
            Field::Zones(zones) => toml::Value::Array(
                zones
                    .into_iter()
                    .map(|fields| {
                        toml::Value::Table(
                            fields
                                .into_iter()
                                .map(|(key, field)| (key.to_owned(), field.toml()))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
        }
    }

    fn nix(self) -> nix::Value {
        match self {
            Field::Str(text) => nix::Value::Str(text),
            Field::Int(n) => nix::Value::Int(n),
            Field::Strs(texts) => {
                nix::Value::List(texts.into_iter().map(nix::Value::Str).collect())
            }
            Field::Keys(keys) => nix::Value::Attrs(
                keys.into_iter()
                    .map(|(name, color)| (name, nix::Value::Str(color)))
                    .collect(),
            ),
            Field::Zones(zones) => nix::Value::List(
                zones
                    .into_iter()
                    .map(|fields| {
                        nix::Value::Attrs(
                            fields
                                .into_iter()
                                .map(|(key, field)| (key.to_owned(), field.nix()))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
        }
    }
}

fn read_color(value: &toml::Value) -> Result<Color, String> {
    value.as_str().ok_or("a colour that isn't text")?.parse()
}

/// The keys a key's or a group's name stands for.
fn named(name: &str) -> Result<Vec<u8>, String> {
    keyboard::named(name).ok_or_else(|| format!("no key is called {name:?}"))
}

/// The groups by name, the larger first.
fn groups() -> Vec<(&'static str, Vec<u8>)> {
    let mut groups: Vec<(&str, Vec<u8>)> = GROUPS
        .iter()
        .filter_map(|&(group, _)| Some((group, keyboard::named(group)?)))
        .collect();
    groups.sort_by_key(|(_, keys)| Reverse(keys.len()));
    groups
}

/// Keys by name: a whole group by its name, the larger groups tried first.
fn names_of(keys: &BTreeSet<u8>) -> Vec<String> {
    let mut left = keys.clone();
    let mut names = Vec::new();
    for (group, keys) in groups() {
        if keys.iter().all(|key| left.contains(key)) {
            for key in &keys {
                left.remove(key);
            }
            names.push(group.to_owned());
        }
    }
    let known = keyboard::names();
    names.extend(left.iter().filter_map(|code| {
        let (name, _) = known.iter().find(|(_, known)| known == code)?;
        Some(name.clone())
    }));
    names
}

/// A colour as the LEDs show it, on screen: as it is.
pub fn led(color: Color) -> iced::Color {
    let [r, g, b] = color.0;
    iced::Color::from_rgb8(r, g, b)
}

fn theme_label(name: &str) -> String {
    crate::app::capitalize(name)
}

fn swatch<'a>(color: Color, size: f32) -> Element<'a, Msg> {
    container(space().width(size).height(size))
        .style(move |theme: &Theme| container::Style {
            background: Some(led(color).into()),
            border: iced::Border::default()
                .rounded(6)
                .width(1)
                .color(theme::colors(theme).border),
            ..container::Style::default()
        })
        .into()
}

fn small<'a>(label: &'a str, press: Msg) -> Element<'a, Msg> {
    button(text(label).size(13))
        .on_press(press)
        .padding([4, 10])
        .style(theme::segment(false))
        .into()
}

fn choice<'a>(label: String, chosen: bool, pick: Msg) -> Element<'a, Msg> {
    button(text(label).size(13).width(Length::Fill).center())
        .on_press_maybe((!chosen).then_some(pick))
        .padding([4, 4])
        .width(Length::Fill)
        .style(theme::segment(chosen))
        .into()
}

fn panel<'a>(title: &'a str, content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    let title = text(title.to_uppercase()).size(12).style(theme::muted);
    container(column![title, content.into()].spacing(10))
        .padding(16)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// The picture: each key in the colour it gets, the picked ones outlined,
/// picked by a click or by dragging over them.
struct Board<'a> {
    editor: &'a Editor,
}

/// The pointer over the picture.
#[derive(Default)]
struct Pointer {
    over: Option<u8>,
    /// Picking or letting go of the keys dragged over, as the first was.
    dragging: Option<bool>,
}

impl Board<'_> {
    /// Key widths to pixels: the scale and the top left corner, the board
    /// centred in `bounds`.
    fn fit(bounds: Rectangle) -> (f32, Point) {
        let scale = (bounds.width / ACROSS).min(bounds.height / DOWN);
        let corner = Point::new(
            (bounds.width - ACROSS * scale) / 2.0,
            (bounds.height - DOWN * scale) / 2.0,
        );
        (scale, corner)
    }

    fn on_screen(rect: Rectangle, scale: f32, corner: Point) -> Rectangle {
        // A gap between the keys.
        let gap = 0.08;
        Rectangle::new(
            Point::new(
                corner.x + (rect.x + gap / 2.0) * scale,
                corner.y + (rect.y + gap / 2.0) * scale,
            ),
            Size::new((rect.width - gap) * scale, (rect.height - gap) * scale),
        )
    }

    fn key_at(bounds: Rectangle, at: Point) -> Option<u8> {
        let (scale, corner) = Self::fit(bounds);
        board()
            .into_iter()
            .find(|key| Self::on_screen(key.rect, scale, corner).contains(at))
            .map(|key| key.code)
    }
}

impl canvas::Program<Msg> for Board<'_> {
    type State = Pointer;

    fn update(
        &self,
        pointer: &mut Pointer,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Msg>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        let under = cursor
            .position_in(bounds)
            .and_then(|at| Self::key_at(bounds, at));
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let key = under?;
                let pick = !self.editor.marked(key);
                pointer.dragging = Some(pick);
                Some(canvas::Action::publish(Msg::Pick(key, pick)).and_capture())
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                pointer.dragging = None;
                None
            }
            mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft => {
                let moved = pointer.over != under;
                pointer.over = under;
                if let (Some(pick), Some(key)) = (pointer.dragging, under)
                    && self.editor.marked(key) != pick
                {
                    return Some(canvas::Action::publish(Msg::Pick(key, pick)));
                }
                moved.then(canvas::Action::request_redraw)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        pointer: &Pointer,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let colors = theme::colors(theme);
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let (scale, corner) = Self::fit(bounds);
        for key in board() {
            let rect = Self::on_screen(key.rect, scale, corner);
            let path = canvas::Path::rounded_rectangle(
                rect.position(),
                rect.size(),
                (scale * 0.12).into(),
            );
            let color = self.editor.shown(&key);
            frame.fill(&path, led(color));
            let picked = self.editor.marked(key.code);
            let (outline, width) = if picked {
                (colors.text, 5.0)
            } else if pointer.over == Some(key.code) {
                (colors.text, 2.0)
            } else {
                (colors.border, 1.0)
            };
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(outline)
                    .with_width(width),
            );
            // A picked key ringed in the text's colour and the page's, to
            // stand out on a key of any colour.
            if picked {
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_color(colors.background)
                        .with_width(1.5),
                );
            }
            // Dark ink on light keys, light on dark ones.
            let [r, g, b] = color.0;
            let light = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 140.0;
            frame.fill_text(canvas::Text {
                content: key.label.to_owned(),
                position: rect.center(),
                color: if light {
                    iced::Color::BLACK
                } else {
                    iced::Color::WHITE
                },
                // Smaller for a long label on a narrow key: Copilot.
                size: (scale * 0.26)
                    .min(rect.width / (key.label.chars().count() as f32 * 0.6))
                    .max(8.0)
                    .into(),
                font: theme::font(),
                align_x: TextAlignment::Center,
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _pointer: &Pointer,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match cursor
            .position_in(bounds)
            .and_then(|at| Self::key_at(bounds, at))
        {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_has_every_key_once_and_nothing_overlaps() {
        let keys = board();
        for (i, key) in keys.iter().enumerate() {
            assert!(
                keys[..i].iter().all(|other| other.code != key.code),
                "{:#x} twice",
                key.code
            );
            assert!(
                keys[..i].iter().all(|other| other
                    .rect
                    .intersection(&key.rect)
                    .is_none_or(|both| { both.width < 0.01 || both.height < 0.01 })),
                "{:#x} overlaps",
                key.code
            );
            assert!(
                key.rect.x + key.rect.width <= ACROSS + 0.01,
                "{}",
                key.label
            );
            assert!(key.rect.y + key.rect.height <= DOWN + 0.01, "{}", key.label);
        }
        // Every key the config can name.
        for (name, code) in keyboard::names() {
            assert!(keys.iter().any(|key| key.code == code), "{name}");
        }
    }

    #[test]
    fn writes_the_profile_as_the_config_takes_it() {
        let mut editor = Editor::new();
        // Picked keys take the colour at once; whole groups go by name.
        let _ = editor.update(Msg::PickGroup("wasd"));
        assert_eq!(editor.slot, Slot::Keys);
        let _ = editor.update(Msg::Pick(0x64, true));
        let shown = editor.update(Msg::Swatch(Color([255, 0, 0])));
        assert_eq!(
            shown,
            Outcome::Preview(
                "color = \"#ffffff\"\nname = \"custom\"\n\n[keys]\n102nd = \"#ff0000\"\nwasd = \"#ff0000\"\n"
                    .into()
            )
        );
        assert!(editor.unsaved());

        // Let go, the colour is the effect's again.
        let _ = editor.update(Msg::PickNone);
        assert_eq!(editor.slot, Slot::Color);
        let _ = editor.update(Msg::Effect(Effect::Wave));
        let _ = editor.update(Msg::Slot(Slot::Round(0)));
        let _ = editor.update(Msg::Swatch(Color([0, 0, 255])));
        let _ = editor.update(Msg::Slot(Slot::Round(1)));
        let _ = editor.update(Msg::Swatch(Color([255, 0, 0])));
        for _ in 2..6 {
            let _ = editor.update(Msg::Slot(Slot::Round(2)));
            let _ = editor.update(Msg::RemoveColor);
        }
        let _ = editor.update(Msg::Slot(Slot::Color));
        let _ = editor.update(Msg::Swatch(Color::OFF));
        let _ = editor.update(Msg::Origin(Origin::Left));
        let _ = editor.update(Msg::Inward(true));
        let _ = editor.update(Msg::Name("two tone".into()));
        let Outcome::Copy(nix) = editor.update(Msg::Copy) else {
            panic!("no copy");
        };
        assert_eq!(
            nix,
            "{\n  name = \"two tone\";\n  effect = \"wave\";\n  colors = [\n    \"#0000ff\"\n    \"#ff0000\"\n  ];\n  period_ms = 4000;\n  origin = \"left\";\n  direction = \"in\";\n  keys = {\n    wasd = \"#ff0000\";\n    \"102nd\" = \"#ff0000\";\n  };\n}"
        );

        // A key let go of the group: by itself.
        let _ = editor.update(Msg::Pick(0x04, true));
        let _ = editor.update(Msg::Unpaint);
        assert_eq!(
            editor.key_names(),
            [
                ("d", "#ff0000"),
                ("s", "#ff0000"),
                ("w", "#ff0000"),
                ("102nd", "#ff0000")
            ]
            .map(|(name, color)| (name.to_owned(), color.to_owned()))
        );
        // Saved, until it changes again; not without a name.
        assert!(matches!(editor.update(Msg::Save), Outcome::Save(_)));
        assert_eq!(
            editor.update(Msg::Delete),
            Outcome::Delete("two tone".into())
        );
        assert_eq!(editor.update(Msg::New), Outcome::New);
        editor.saved();
        assert!(!editor.unsaved());
        let _ = editor.update(Msg::Name(" ".into()));
        assert_eq!(editor.update(Msg::Save), Outcome::Nothing);
    }

    /// What ReadKeyboardProfile gives, as the daemon writes it.
    #[test]
    fn starts_from_a_profile() {
        let read = "name = \"split\"\neffect = \"wave\"\ncolors = [\"#ff0000\", \"#0000ff\"]\norigin = \"left\"\n\n[keys]\nletters = \"#ffffff\"\nw = \"off\"\n\n[[zones]]\nkeys = [\"numpad\"]\neffect = \"breathe\"\ncolor = \"#ff6000\"\n";
        let editor = Editor::from_toml(read).unwrap();
        assert_eq!(editor.look.effect, Effect::Wave);
        assert_eq!(editor.look.origin, Origin::Left);
        assert_eq!(editor.look.period_ms, 3000);
        assert_eq!(editor.look.color, Color::OFF);
        // The single key over its group.
        assert_eq!(editor.keys[&0x1a], Color::OFF);
        assert_eq!(editor.keys[&0x04], Color([255; 3]));
        assert!(editor.zones[0].keys.contains(&0x5d));
        assert_eq!(editor.zones[0].look.color, Color([255, 96, 0]));
        assert!(!editor.unsaved());
        // Written back the same.
        let again = Editor::from_toml(&editor.toml()).unwrap();
        assert_eq!(again.toml(), editor.toml());
        assert!(editor.toml().contains(
            "[[zones]]\ncolor = \"#ff6000\"\neffect = \"breathe\"\nkeys = [\"numpad\"]\nperiod_ms = 4000\n"
        ));
        assert!(Editor::from_toml("name = \"x\"\neffect = \"sparkle\"\n").is_err());
    }

    #[test]
    fn edits_zones() {
        let mut editor = Editor::new();
        let _ = editor.update(Msg::AddZone);
        assert_eq!(editor.layer, 1);
        // Keys picked on the picture go in the zone shown.
        let _ = editor.update(Msg::PickGroup("arrows"));
        let _ = editor.update(Msg::Pick(0x2c, true));
        assert!(editor.marked(0x52) && editor.picked.is_empty());
        assert_eq!(editor.update(Msg::Effect(Effect::Static)), Outcome::Nothing);
        let _ = editor.update(Msg::Effect(Effect::Wave));
        let _ = editor.update(Msg::Slot(Slot::Round(0)));
        let _ = editor.update(Msg::Swatch(Color([0, 0, 255])));
        // The profile's stays still and white.
        let _ = editor.update(Msg::Layer(0));
        assert_eq!(editor.look.effect, Effect::Static);
        assert!(!editor.marked(0x52));
        let written = editor.toml();
        assert!(written.starts_with(
            "color = \"#ffffff\"\nname = \"custom\"\n\n[[zones]]\ncolors = [\"#0000ff\""
        ));
        assert!(written.contains("effect = \"wave\"\nkeys = [\"arrows\", \"space\"]\n"));
        // An empty zone isn't written; one removed is gone.
        let _ = editor.update(Msg::AddZone);
        assert_eq!(editor.toml().matches("[[zones]]").count(), 1);
        let _ = editor.update(Msg::RemoveZone);
        assert_eq!((editor.zones.len(), editor.layer), (1, 1));
    }

    #[test]
    fn a_click_picks_the_key_under_it() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1925.0, 600.0));
        // Scale 100: Esc in the top left, the numpad's 5 at (16.75, 3.5).
        assert_eq!(Board::key_at(bounds, Point::new(40.0, 40.0)), Some(0x29));
        assert_eq!(Board::key_at(bounds, Point::new(1675.0, 350.0)), Some(0x5d));
        // The tall Plus and Enter, low and high.
        assert_eq!(Board::key_at(bounds, Point::new(1875.0, 380.0)), Some(0x57));
        assert_eq!(Board::key_at(bounds, Point::new(1875.0, 580.0)), Some(0x58));
        // Between keys, nothing.
        assert_eq!(Board::key_at(bounds, Point::new(100.0, 100.0)), None);
    }
}
