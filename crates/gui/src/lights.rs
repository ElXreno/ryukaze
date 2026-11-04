//! The keyboard light's editor: a picture of the laptop's keys to pick and
//! paint, an effect and its colours, and zones of keys with effects of
//! their own over it, shown on the keyboard as it goes (as a
//! preview the daemon drops at the next profile or scenario), saved with the
//! daemon's profiles, and copied out as Nix, a profile for
//! `keyboard.profiles`.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use iced::{
    Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme, alignment, mouse,
    widget::{
        button, canvas, column, container, row, slider, space, text,
        text::Alignment as TextAlignment, text_input,
    },
};
use ryukaze_core::{
    keyboard, lightbar,
    lighting::{self, Color, Origin, Part},
    steelseries,
};

use crate::anim::Hover;
use crate::{
    bootlogo::Logo,
    laptop::Laptop,
    layouts::{self, Key, Layout},
    nix, picker, theme,
};

/// The lights beside the keys a laptop has, from the names Status gives.
pub fn parts(names: Option<&[String]>) -> Vec<Part> {
    names
        .unwrap_or_default()
        .iter()
        .filter_map(|name| Part::named(name))
        .collect()
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
            wavelength: lighting::Wave::default().wavelength,
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
        if self.colors.len() >= *lighting::COLORS.start() {
            self.colors.clone()
        } else {
            lighting::RAINBOW.to_vec()
        }
    }

    /// The period, at least what the colours need.
    fn period(&self) -> u32 {
        let shortest = match self.effect {
            Effect::Cycle | Effect::Wave => lighting::shortest_period_ms(self.round().len()),
            _ => *lighting::PERIODS_MS.start(),
        };
        self.period_ms.clamp(shortest, *lighting::PERIODS_MS.end())
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
                if self.colors.len() >= *lighting::COLORS.start() {
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
                    if self.wavelength != lighting::Wave::default().wavelength {
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

    /// The colour at a point of the KLC's plane: the effect's at about
    /// where it is.
    fn at(&self, at: [u16; 2]) -> Color {
        match self.effect {
            Effect::Cycle => self.round()[0],
            // From one colour to the next, as the controller fades them.
            Effect::Wave => {
                let round = self.round();
                let along = self.lag(at) * round.len() as f64;
                let first = along.floor();
                let from = (first as i64).rem_euclid(round.len() as i64) as usize;
                let to = round[(from + 1) % round.len()];
                let mix = (along - first) as f32;
                Color(std::array::from_fn(|c| {
                    let (from, to) = (f32::from(round[from].0[c]), f32::from(to.0[c]));
                    (from + (to - from) * mix).round() as u8
                }))
            }
            _ => self.color,
        }
    }

    /// How many rounds of a wave's colours lie between its origin and a
    /// point of the plane, as the daemon lights the keys and beyond.
    fn lag(&self, at: [u16; 2]) -> f64 {
        let wave = lighting::Wave {
            origin: self.origin,
            inward: self.inward,
            wavelength: self.wavelength,
        };
        steelseries::wave_lag(&wave, at)
    }
}

/// Keys with an effect of their own beside the profile's.
#[derive(Debug, Clone, PartialEq)]
struct Zone {
    keys: BTreeSet<u8>,
    /// The lights beside the keys it takes in, kept as read.
    parts: BTreeSet<Part>,
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
    /// The lights beside the keys with colours of their own, kept as read.
    parts: BTreeMap<Part, Color>,
    picked: BTreeSet<u8>,
    /// The lights beside the keys picked, as keys are.
    picked_parts: BTreeSet<Part>,
    slot: Slot,
    /// The colour as typed.
    hex: String,
    /// Changed since it was opened or saved.
    unsaved: bool,
    /// Where to go once it's said what becomes of the changes.
    leaving: Option<Leaving>,
    /// Where to go once the daemon has saved it.
    after_save: Option<Leaving>,
    /// The colour picker open, and the hue it shows a grey in.
    picking: bool,
    hue: f32,
    /// A zone asked to be removed, until it's said.
    removing: Option<usize>,
    /// The laptop, whose keys and logo the picture draws.
    laptop: Arc<Laptop>,
}

/// Another profile asked for with changes unsaved.
#[derive(Debug, Clone, PartialEq)]
enum Leaving {
    New,
    Open(String),
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
    /// Ask before removing zone n, then remove it or keep it.
    RemoveZone(usize),
    ConfirmRemoveZone,
    KeepZone,
    Slot(Slot),
    Swatch(Color),
    Hex(String),
    /// A colour from the picker, and the hue it was picked in.
    Picked([u8; 3], f32),
    /// The picked keys back to the effect.
    Unpaint,
    /// A light beside the keys: in the profile, its own colour, the one
    /// picked, or back with the effect; in a zone, in it or out.
    Part(Part),
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
    /// The changes dropped, for the profile asked for.
    Discard,
    /// The changes kept, and the profile asked for left.
    Stay,
    /// The changes saved, under this name if one, then the profile asked for.
    SaveAndLeave(Option<String>),
    Save,
    /// Saved under this name instead: a standard one's copy.
    SaveAs(String),
    /// The changes dropped, for the profile on the keyboard as it is.
    Revert,
    Delete,
    /// The colour picker shown or hidden.
    Picker,
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
    /// Back to the profile on the keyboard, as the daemon has it.
    Revert,
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
            parts: BTreeMap::new(),
            picked: BTreeSet::new(),
            picked_parts: BTreeSet::new(),
            slot: Slot::Color,
            hex,
            unsaved: false,
            leaving: None,
            after_save: None,
            picking: false,
            hue: 0.0,
            removing: None,
            laptop: Laptop::any(),
        }
    }

    /// For `laptop`, as read.
    pub fn set_laptop(&mut self, laptop: Arc<Laptop>) {
        self.laptop = laptop;
    }

    /// A new profile under `name`, all white and still, not saved yet.
    pub fn fresh(name: &str) -> Self {
        Self {
            unsaved: true,
            ..Self::named(name)
        }
    }

    /// A standard profile without the daemon to read it: its effect, for
    /// one named after an effect, or else all white and still.
    pub fn standard(name: &str) -> Self {
        let mut editor = Self::named(name);
        if let Some(effect) = Effect::ALL.into_iter().find(|effect| effect.name() == name) {
            let mut table = toml::Table::new();
            table.insert("effect".into(), effect.name().into());
            if let Ok(look) = Look::read(&table) {
                editor.hex = look.color.to_string();
                editor.look = look;
            }
        }
        editor
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
            let color = read_color(value)?;
            match Part::named(name) {
                Some(part) => {
                    editor.parts.insert(part, color);
                }
                None => keys.push((named(name)?, color)),
            }
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
            let mut parts = BTreeSet::new();
            for name in zone
                .get("keys")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
            {
                match Part::named(name) {
                    Some(part) => {
                        parts.insert(part);
                    }
                    None => keys.extend(named(name)?),
                }
            }
            editor.zones.push(Zone {
                keys,
                parts,
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
        // Anything else answers it: the changes stay.
        let leaving = self.leaving.take();
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
                    Msg::PickAll => self.laptop.layout.keys.iter().map(|key| key.code).collect(),
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
                        self.unpick();
                        self.picked_parts.clear();
                    }
                    self.picked.extend(keys);
                    self.picking_changed();
                    return Outcome::Nothing;
                }
            }
            Msg::Layer(layer) => {
                self.layer = layer.min(self.zones.len());
                self.unpick();
                self.slot = self.look().slots()[0];
                self.hex = self.slot_color().to_string();
                return Outcome::Nothing;
            }
            Msg::AddZone => {
                if self.zones.len() >= lighting::ZONES {
                    return Outcome::Nothing;
                }
                self.zones.push(Zone {
                    keys: BTreeSet::new(),
                    parts: BTreeSet::new(),
                    look: Look::new(Effect::Breathe),
                });
                self.unpick();
                self.layer = self.zones.len();
                self.slot = Slot::Color;
                self.hex = self.slot_color().to_string();
                return Outcome::Nothing;
            }
            Msg::RemoveZone(zone) => {
                self.removing = Some(zone);
                return Outcome::Nothing;
            }
            Msg::KeepZone => {
                self.removing = None;
                return Outcome::Nothing;
            }
            Msg::ConfirmRemoveZone => {
                let Some(zone) = self.removing.take() else {
                    return Outcome::Nothing;
                };
                if zone == 0 || zone > self.zones.len() {
                    return Outcome::Nothing;
                }
                self.zones.remove(zone - 1);
                // Another removed, the one shown stays as it was, what's
                // picked and the colour being set too; one after it moves
                // down a place.
                if self.layer == zone {
                    self.layer -= 1;
                    self.unpick();
                    self.slot = self.look().slots()[0];
                    self.hex = self.slot_color().to_string();
                } else if self.layer > zone {
                    self.layer -= 1;
                }
            }
            Msg::Slot(slot) => {
                self.unpick();
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
            Msg::Picked(picked, hue) => {
                // Shown on the picture at once, on the keyboard once let go.
                self.hue = hue;
                let color = Color(picked);
                self.set_slot_color(color);
                self.hex = color.to_string();
                self.unsaved = true;
                return Outcome::Nothing;
            }
            Msg::Unpaint => {
                for key in &self.picked {
                    self.keys.remove(key);
                }
                for part in &self.picked_parts {
                    self.parts.remove(part);
                }
                self.hex = self.slot_color().to_string();
            }
            Msg::Part(part) => match self.layer {
                0 => {
                    if !self.picked_parts.remove(&part) {
                        self.picked_parts.insert(part);
                    }
                    self.picking_changed();
                    return Outcome::Nothing;
                }
                zone => {
                    let parts = &mut self.zones[zone - 1].parts;
                    if !parts.remove(&part) {
                        parts.insert(part);
                    }
                }
            },
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
                if round.len() >= *lighting::COLORS.end() {
                    return Outcome::Nothing;
                }
                // A copy of the last to change.
                round.push(round[round.len() - 1]);
                self.slot = Slot::Round(round.len() - 1);
                self.look_mut().colors = round;
                self.unpick();
            }
            Msg::RemoveColor => {
                let mut round = self.look().round();
                let Slot::Round(i) = self.slot else {
                    return Outcome::Nothing;
                };
                if round.len() <= *lighting::COLORS.start() || i >= round.len() {
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
                    self.slot = Slot::Round(i.min(lighting::RAINBOW.len() - 1));
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
                self.after_save = None;
                return if self.name.trim().is_empty() {
                    Outcome::Nothing
                } else {
                    Outcome::Save(self.toml())
                };
            }
            Msg::SaveAndLeave(name) => {
                if let Some(name) = name {
                    self.name = name;
                }
                if self.name.trim().is_empty() {
                    self.leaving = leaving;
                    return Outcome::Nothing;
                }
                self.after_save = leaving;
                return Outcome::Save(self.toml());
            }
            Msg::Revert => {
                self.unsaved = false;
                return Outcome::Revert;
            }
            Msg::Picker => {
                self.picking = !self.picking;
                return Outcome::Nothing;
            }
            Msg::New | Msg::Open(_) if self.unsaved => {
                self.leaving = Some(match msg {
                    Msg::Open(name) => Leaving::Open(name),
                    _ => Leaving::New,
                });
                return Outcome::Nothing;
            }
            Msg::New => return Outcome::New,
            Msg::Open(name) => return Outcome::Open(name),
            Msg::Discard => {
                return match leaving {
                    Some(Leaving::New) => Outcome::New,
                    Some(Leaving::Open(name)) => Outcome::Open(name),
                    None => Outcome::Nothing,
                };
            }
            Msg::Stay => return Outcome::Nothing,
            Msg::SaveAs(name) => {
                self.after_save = None;
                self.name = name;
                return Outcome::Save(self.toml());
            }
            Msg::Delete => return Outcome::Delete(self.name.trim().to_owned()),
        }
        self.unsaved = true;
        Outcome::Preview(self.toml())
    }

    pub fn name(&self) -> &str {
        self.name.trim()
    }

    /// The profile as it stands, for the keyboard to show.
    pub fn preview(&self) -> String {
        self.toml()
    }

    /// Changed since it was opened or saved.
    pub fn unsaved(&self) -> bool {
        self.unsaved
    }

    /// The daemon took it: where to go next, if it was saved on the way to
    /// another profile.
    pub fn saved(&mut self) -> Outcome {
        self.unsaved = false;
        match self.after_save.take() {
            Some(Leaving::New) => Outcome::New,
            Some(Leaving::Open(name)) => Outcome::Open(name),
            None => Outcome::Nothing,
        }
    }

    fn unpick(&mut self) {
        self.picked.clear();
        self.picked_parts.clear();
    }

    /// How many keys and lights beside them are picked.
    fn picked_count(&self) -> usize {
        self.picked.len() + self.picked_parts.len()
    }

    /// Picked keys and lights are what the colour paints; none, the
    /// effect's colour.
    fn picking_changed(&mut self) {
        if self.picked_count() > 0 {
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
                .or_else(|| {
                    self.picked_parts
                        .first()
                        .and_then(|part| self.parts.get(part))
                })
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
                for &part in &self.picked_parts {
                    self.parts.insert(part, color);
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
        // A zone without keys or lights is none.
        let zones: Vec<Vec<(&'static str, Field)>> = self
            .zones
            .iter()
            .filter(|zone| !zone.keys.is_empty() || !zone.parts.is_empty())
            .map(|zone| {
                let mut names = names_of(&zone.keys);
                names.extend(zone.parts.iter().map(|part| part.name().to_owned()));
                let mut fields = vec![("keys", Field::Strs(names))];
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
        named.extend(
            self.parts
                .iter()
                .map(|(part, color)| (part.name().to_owned(), color.to_string())),
        );
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
    pub fn nix_value(&self) -> nix::Value {
        nix::Value::Attrs(
            self.fields()
                .into_iter()
                .map(|(key, field)| (key.to_owned(), field.nix()))
                .collect(),
        )
    }

    #[cfg(test)]
    fn nix(&self) -> String {
        let nix::Value::Attrs(entries) = self.nix_value() else {
            unreachable!("a profile is an attribute set")
        };
        nix::attrset(&entries)
    }

    /// The colour the picture gives a key: its own, or its zone's or the
    /// effect's at about where it sits.
    fn shown(&self, key: &Key) -> Color {
        if let Some(&color) = self.keys.get(&key.code) {
            return color;
        }
        let at = self.laptop.layout.plane(key.rect.center());
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

    /// The colour the picture gives a light beside the keys at `at`, in the
    /// plane: its own, or its zone's or the effect's there.
    fn shown_part(&self, part: Part, at: [u16; 2]) -> Color {
        if let Some(&color) = self.parts.get(&part) {
            return color;
        }
        match self
            .zones
            .iter()
            .rev()
            .find(|zone| zone.parts.contains(&part))
        {
            Some(zone) => zone.look.at(at),
            None => self.look.at(at),
        }
    }

    /// A light beside the keys outlined: picked, or in the zone shown.
    fn marked_part(&self, part: Part) -> bool {
        match self.layer {
            0 => self.picked_parts.contains(&part),
            zone => self.zones[zone - 1].parts.contains(&part),
        }
    }

    /// Those of the laptop's lights beside the keys the picture has.
    fn drawn(&self, parts: &[Part]) -> Vec<Part> {
        parts
            .iter()
            .copied()
            .filter(|&part| self.laptop.layout.part(part).is_some())
            .collect()
    }

    /// In `width`: the profiles to pick, what becomes of the changes, the
    /// picture as wide as there is, and the effect under it.
    pub fn view<'a>(&'a self, width: f32, known: &Known) -> Element<'a, Msg> {
        const GAP: f32 = 12.0;
        let across = width - 32.0;
        let parts = self.drawn(&known.parts);
        let extent = Beside::new(&self.laptop.layout, &parts, None).extent(&self.laptop.layout);
        let down = self.laptop.layout.keys_span().height;
        let picture = canvas(Board {
            editor: self,
            parts: parts.clone(),
        })
        .width(Length::Fill)
        .height((across * extent.height / extent.width).min(600.0 * extent.height / down));
        let hero = container(column![picture, self.picking(&parts)].spacing(GAP))
            .padding(16)
            .width(Length::Fill)
            .style(theme::card);
        column![
            self.strip(known),
            self.bar(known),
            hero,
            self.effect_card(across)
        ]
        .spacing(GAP)
        .into()
    }

    /// The daemon's profiles, the one on the keyboard filled, to show one
    /// there and edit it; and a new one.
    fn strip<'a>(&self, known: &Known) -> Element<'a, Msg> {
        let name = self.name();
        let chips = known
            .standard
            .iter()
            .chain(&known.saved)
            .map(|profile| -> Element<'a, Msg> {
                let shown = known.shown.as_deref() == Some(profile.as_str());
                let open = profile == name;
                let mut label = theme_label(profile);
                if open && self.unsaved {
                    label.push_str(" •");
                }
                let here = shown && open && !self.unsaved;
                button(text(label).size(14))
                    .on_press_maybe((!here).then(|| Msg::Open(profile.clone())))
                    .padding([6, 12])
                    .hover(theme::segment(shown))
            });
        row(chips)
            .push(small("+ New", Msg::New))
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
            .into()
    }

    /// What becomes of the changes: asked before another profile, the ways
    /// to keep or drop them while there are any, else Delete.
    fn bar<'a>(&'a self, known: &Known) -> Element<'a, Msg> {
        let name = self.name();
        let standard = known.standard.iter().any(|known| known == name);
        let saved = known.saved.iter().any(|known| known == name);
        let taken = |copy: &String| known.standard.contains(copy) || known.saved.contains(copy);
        // A standard one stays as it is: saved, it's a copy.
        let copy = (!name.is_empty())
            .then(|| {
                (2..)
                    .map(|n| format!("{name} {n}"))
                    .find(|copy| !taken(copy))
            })
            .flatten();
        if self.leaving.is_some() {
            let save = if standard {
                copy.map(|copy| Msg::SaveAndLeave(Some(copy)))
            } else {
                (!name.is_empty()).then_some(Msg::SaveAndLeave(None))
            };
            return row![
                text(format!("The changes to {name} aren't saved")).size(14),
                small_maybe("Save and switch", save),
                small("Discard them", Msg::Discard),
                small("Keep editing", Msg::Stay),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .wrap()
            .vertical_spacing(6)
            .into();
        }
        let delete = saved.then(|| small("Delete", Msg::Delete));
        if !self.unsaved {
            let note = if known.shown.as_deref() == Some(name) {
                "On the keyboard"
            } else {
                ""
            };
            return row![text(note).size(14).style(theme::muted).width(Length::Fill)]
                .push(delete)
                .spacing(8)
                .align_y(Alignment::Center)
                .into();
        }
        let save = (!standard && !name.is_empty()).then_some(Msg::Save);
        let save_as = copy.map(Msg::SaveAs);
        let field = text_input("a name to save it by", &self.name)
            .style(theme::field)
            .on_input(Msg::Name)
            .on_submit_maybe(save.clone().or_else(|| save_as.clone()))
            .size(14)
            .width(200);
        let mut actions = row![field];
        if let Some(save) = save {
            actions = actions.push(
                button(text("Save").size(14))
                    .on_press(save)
                    .padding([6, 12])
                    .hover(theme::segment(true)),
            );
        }
        let note = if known.previewing {
            "On the keyboard until saved or reverted"
        } else {
            "Not saved"
        };
        actions
            .push(small_maybe("Save as copy", save_as))
            .push(small("Revert", Msg::Revert))
            .push(delete)
            .push(text(note).size(14).style(theme::muted))
            .spacing(8)
            .align_y(Alignment::Center)
            .wrap()
            .vertical_spacing(6)
            .into()
    }

    /// What's picked: "3 keys, the touchpad and the logo", "the logo".
    fn picked_what(&self) -> String {
        let mut what = Vec::new();
        match self.picked.len() {
            0 => {}
            1 => what.push("1 key".to_owned()),
            n => what.push(format!("{n} keys")),
        }
        what.extend(
            self.picked_parts
                .iter()
                .map(|&part| part_label(part).to_owned()),
        );
        match what.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        }
    }

    /// Under the picture: picking by group, and what a click does.
    fn picking(&self, parts: &[Part]) -> Element<'_, Msg> {
        let mut groups = row![small("All", Msg::PickAll), small("None", Msg::PickNone)].spacing(6);
        for (group, label) in GROUPS {
            groups = groups.push(small(label, Msg::PickGroup(group)));
        }
        let lit = parts
            .iter()
            .map(|&part| part_label(part))
            .collect::<Vec<_>>()
            .join(" or ");
        let note = match (self.layer, self.picked_count()) {
            (0, 0) if lit.is_empty() => {
                "Click keys, or drag over them, to pick them and paint them in a colour of their own"
                    .to_owned()
            }
            (0, 0) => format!(
                "Click keys or {lit}, or drag over keys, to pick them and paint them in a colour \
                 of their own"
            ),
            (0, 1) => format!("{} picked: the colour paints it", crate::app::capitalize(&self.picked_what())),
            (0, _) => format!(
                "{} picked: the colour paints them",
                crate::app::capitalize(&self.picked_what())
            ),
            (zone, _) => {
                let also = if lit.is_empty() {
                    String::new()
                } else {
                    format!(" or {lit}")
                };
                format!(
                    "Click keys{also}, or drag over keys, to put them in zone {zone} or take them \
                     out; a later zone is over an earlier, and a key's own colour over both"
                )
            }
        };
        column![
            groups.wrap().vertical_spacing(6),
            text(note).size(14).style(theme::muted),
        ]
        .spacing(8)
        .into()
    }

    /// The layers, the profile and its zones, the effect of the one shown,
    /// and the colour being set.
    fn effect_card(&self, across: f32) -> Element<'_, Msg> {
        let look = self.look();
        let layer = |label: String, layer: usize| -> Element<'_, Msg> {
            let chosen = self.layer == layer;
            button(text(label).size(14))
                .on_press_maybe((!chosen).then_some(Msg::Layer(layer)))
                .padding([5, 10])
                .hover(theme::segment(chosen))
        };
        let mut layers = row![layer("Keyboard".into(), 0)]
            .spacing(4)
            .align_y(Alignment::Center);
        // A zone's tab, and its cross right against it.
        for zone in 1..=self.zones.len() {
            let chosen = self.layer == zone;
            layers = layers.push(
                row![
                    layer(format!("Zone {zone}"), zone),
                    button(text("×").size(14))
                        .on_press(Msg::RemoveZone(zone))
                        .padding([5, 8])
                        .hover(theme::segment(chosen)),
                ]
                .spacing(1),
            );
        }
        if self.zones.len() < lighting::ZONES {
            layers = layers.push(small("+ Zone", Msg::AddZone));
        }
        let effects: Vec<Effect> = Effect::ALL
            .into_iter()
            .filter(|&effect| self.layer == 0 || effect != Effect::Static)
            .collect();
        // In rows of three where they'd be too narrow for their names.
        let per_row = if across / effects.len() as f32 >= 84.0 {
            effects.len()
        } else {
            3
        };
        let effects = column(effects.chunks(per_row).map(|chunk| {
            let mut line = row(chunk.iter().map(|&effect| {
                choice(
                    theme_label(effect.name()),
                    look.effect == effect,
                    Msg::Effect(effect),
                )
            }))
            .spacing(4);
            for _ in chunk.len()..per_row {
                line = line.push(space().width(Length::Fill));
            }
            line.into()
        }))
        .spacing(4);
        let period = self.level(
            "Period",
            slider(lighting::PERIODS_MS.clone(), look.period(), Msg::Period)
                .step(500u32)
                .on_release(Msg::Released)
                .style(theme::level),
            format!("{:.1} s", look.period() as f32 / 1000.0),
        );

        let mut controls = column![layers.wrap().vertical_spacing(4)].spacing(10);
        if let Some(zone) = self.removing {
            controls = controls.push(
                row![
                    text(format!("Remove zone {zone} and its effect?")).size(14),
                    space::horizontal(),
                    small("Remove", Msg::ConfirmRemoveZone),
                    small("Keep", Msg::KeepZone),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if self.layer > 0 {
            let zone = &self.zones[self.layer - 1];
            let count = match zone.keys.len() {
                0 => "no keys yet: click them on the picture".to_owned(),
                1 => "1 key".to_owned(),
                n => format!("{n} keys"),
            };
            controls = controls.push(
                text(format!("Zone {}: {count}", self.layer))
                    .size(14)
                    .style(theme::muted),
            );
        }
        controls = controls.push(effects);
        // A round's colours with what sets the one picked right under
        // them, rather than below the wave's other settings.
        let mut docked = false;
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
                if round.len() < *lighting::COLORS.end() {
                    colors = colors.push(small("+", Msg::AddColor));
                }
                let mut tools = row![].spacing(6);
                if matches!(self.slot, Slot::Round(_)) && round.len() > *lighting::COLORS.start() {
                    tools = tools.push(small("Remove", Msg::RemoveColor));
                }
                if !look.colors.is_empty() {
                    tools = tools.push(small("Rainbow", Msg::Rainbow));
                }
                controls = controls
                    .push(
                        text("Colours: click one to set it")
                            .size(14)
                            .style(theme::muted),
                    )
                    .push(colors.wrap().vertical_spacing(6))
                    .push(tools)
                    .push(self.dock())
                    .push(period);
                docked = true;
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
                            lighting::WAVELENGTHS.clone(),
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
                    slider(lighting::FLASHES_MS.clone(), look.flash_ms, Msg::FlashMs)
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
                            .size(14)
                            .style(theme::muted),
                    );
            }
        }
        if !docked {
            controls = controls.push(self.dock());
        }
        panel("Effect", controls)
    }

    /// One of the effect's colours, to pick for the colour to set.
    fn slot<'a>(&self, label: &'a str, slot: Slot) -> Element<'a, Msg> {
        let color = self.look().get(slot);
        button(
            row![swatch(color, 20.0), text(label).size(14)]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .on_press(Msg::Slot(slot))
        .padding([6, 10])
        .width(Length::Fill)
        .hover(theme::segment(self.slot == slot))
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
            text(label).size(14).width(60),
            slider.into(),
            text(value).size(14).width(52).align_x(Alignment::End),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// The colour being set: what for, a few to pick, its hex code, and the
    /// picker, opened from the colour itself.
    fn dock(&self) -> Element<'_, Msg> {
        let color = self.slot_color();
        let what = match self.slot {
            Slot::Keys => format!("For {}", self.picked_what()),
            Slot::Color => match self.look().effect {
                Effect::Breathe => "Breathing".to_owned(),
                _ => "The keys'".to_owned(),
            },
            Slot::FadeTo => "Fading to".to_owned(),
            Slot::Flash => "A pressed key's".to_owned(),
            Slot::Round(i) => format!("Colour {}", i + 1),
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
            button(swatch(color, 28.0))
                .padding(0)
                .on_press(Msg::Picker)
                .style(|_: &Theme, _| button::Style::default()),
            text_input("#rrggbb", &self.hex)
                .style(theme::field)
                .on_input(Msg::Hex)
                .size(14)
                .width(100),
            button(
                text(if self.picking {
                    "Hide the picker"
                } else {
                    "Pick"
                })
                .size(14)
            )
            .on_press(Msg::Picker)
            .padding([5, 10])
            .hover(theme::segment(self.picking)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if self.slot == Slot::Keys {
            top = top.push(small("Back to the effect", Msg::Unpaint));
        }
        let mut dock = column![
            text(what).size(14).style(theme::muted),
            swatches,
            top.wrap().vertical_spacing(6),
        ]
        .spacing(10);
        if self.picking {
            // The hue it was picked in while that still makes it: a grey
            // has none of its own, and a dark or pale colour's, rounded to
            // 8 bits, drifts from the one dragged in.
            let hue = picked_hue(color.0, self.hue);
            dock = dock.push(
                canvas(picker::Picker {
                    color: color.0,
                    hue,
                    on_pick: Box::new(Msg::Picked),
                    on_release: Msg::Released,
                })
                .width(Length::Fill)
                .height(160),
            );
        }
        dock.into()
    }
}

/// The daemon's keyboard light profiles: the standard ones and the
/// config's, which stay as they are, and those saved here.
pub struct Known {
    pub standard: Vec<String>,
    pub saved: Vec<String>,
    /// The profile on the keyboard.
    pub shown: Option<String>,
    /// The lights beside the keys the laptop has.
    pub parts: Vec<Part>,
    /// The editor's changes on the keyboard now.
    pub previewing: bool,
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
    small_maybe(label, Some(press))
}

/// A small button, greyed out without anything to do.
fn small_maybe<'a>(label: &'a str, press: Option<Msg>) -> Element<'a, Msg> {
    button(text(label).size(14))
        .on_press_maybe(press)
        .padding([5, 10])
        .hover(theme::segment(false))
}

fn choice<'a>(label: String, chosen: bool, pick: Msg) -> Element<'a, Msg> {
    button(text(label).size(14).width(Length::Fill).center())
        .on_press_maybe((!chosen).then_some(pick))
        .padding([5, 4])
        .width(Length::Fill)
        .hover(theme::segment(chosen))
}

fn panel<'a>(title: &'a str, content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    let title = text(title).size(16).font(theme::heading());
    container(column![title, content.into()].spacing(12))
        .padding(16)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// A light beside the keys, as a sentence names it.
/// The hue to show `color` in: `last`, where it still makes `color` to
/// within rounding, or else the colour's own.
fn picked_hue(color: [u8; 3], last: f32) -> f32 {
    let (own, saturation, value) = picker::hsv(color);
    let again = picker::rgb(last, saturation, value);
    let close = again.iter().zip(color).all(|(&a, b)| a.abs_diff(b) <= 1);
    if saturation == 0.0 || close {
        last
    } else {
        own
    }
}

fn part_label(part: Part) -> &'static str {
    match part {
        Part::Bar => "the touchpad",
        Part::Logo => "the logo",
    }
}

/// The gap between the keys, in key widths.
const KEY_GAP: f32 = 0.08;

/// How far from the keys the touchpad and the lid are drawn, and how tall.
const BESIDE_GAP: f32 = 0.5;
const BESIDE_HEIGHT: f32 = 2.6;

/// The lights beside the keys. With the deck known, where they are: the
/// keys and the touchpad on the deck, and the lid above it, as wide, as
/// it's shaped. Without, drawn close to the keys, which would otherwise
/// sit in a mostly empty picture: the touchpad under them as it's shaped
/// and where it is across, the lid above them, as tall. The logo is where
/// it sits as seen from the keys; the colours come from where they are.
/// On the deck, a touchpad without light is there too, dark.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Beside {
    deck: Option<Rectangle>,
    pad: Option<Rectangle>,
    dark_pad: Option<Rectangle>,
    lid: Option<Rectangle>,
    logo: Option<Rectangle>,
    /// The firmware's logo's width to its height, where there is one.
    logo_aspect: Option<f32>,
}

impl Beside {
    fn new(layout: &Layout, parts: &[Part], logo: Option<&Logo>) -> Self {
        let logo_aspect = logo.map(|logo| logo.width as f32 / logo.height as f32);
        let real = |part: Part| parts.contains(&part).then(|| layout.part(part)).flatten();
        let lit_logo = parts.contains(&Part::Logo).then(|| layout.logo()).flatten();
        if let Some(deck) = layout.deck() {
            let lid = lit_logo.map(|logo| {
                let height = deck.width / logo.lid;
                Rectangle::new(
                    Point::new(deck.x, deck.y - BESIDE_GAP - height),
                    Size::new(deck.width, height),
                )
            });
            return Self {
                deck: Some(deck),
                logo_aspect,
                pad: real(Part::Bar),
                dark_pad: (!parts.contains(&Part::Bar))
                    .then(|| layout.part(Part::Bar))
                    .flatten(),
                lid,
                logo: lit_logo.zip(lid).map(|(logo, lid)| on_lid(logo, lid)),
            };
        }
        let span = layout.keys_span();
        let top = span.y + span.height + BESIDE_GAP;
        let pad = real(Part::Bar).map(|pad| {
            let width = BESIDE_HEIGHT * pad.width / pad.height;
            Rectangle::new(
                Point::new(pad.center_x() - width / 2.0, top),
                Size::new(width, BESIDE_HEIGHT),
            )
        });
        // The lid over the middle of the keys, as tall, as it's shaped.
        let lid = lit_logo.map(|logo| {
            let height = span.height;
            let width = height * logo.lid;
            Rectangle::new(
                Point::new(span.center_x() - width / 2.0, span.y - BESIDE_GAP - height),
                Size::new(width, height),
            )
        });
        Self {
            deck: None,
            logo_aspect,
            pad,
            dark_pad: None,
            lid,
            logo: lit_logo.zip(lid).map(|(logo, lid)| on_lid(logo, lid)),
        }
    }

    /// What the picture spans.
    fn extent(&self, layout: &Layout) -> Rectangle {
        [self.deck, self.pad, self.lid]
            .into_iter()
            .flatten()
            .fold(layout.keys_span(), |extent, rect| extent.union(&rect))
    }

    /// Where a click picks `part`.
    fn area(&self, part: Part) -> Option<Rectangle> {
        match part {
            Part::Bar => self.pad,
            Part::Logo => self.logo.map(|spot| logo_drawn(spot, self.logo_aspect)),
        }
    }

    /// The touchpad's `rows` rows of light, from the keys to the front.
    fn bands(&self, rows: usize) -> Vec<Rectangle> {
        let Some(pad) = self.pad else {
            return Vec::new();
        };
        let height = pad.height / rows.max(1) as f32;
        (0..rows)
            .map(|row| {
                Rectangle::new(
                    Point::new(pad.x, pad.y + height * row as f32),
                    Size::new(pad.width, height),
                )
            })
            .collect()
    }
}

/// Where the logo is on `lid`, as parts of it.
fn on_lid(logo: layouts::Logo, lid: Rectangle) -> Rectangle {
    let on = logo.on_lid;
    Rectangle::new(
        Point::new(lid.x + on.x * lid.width, lid.y + on.y * lid.height),
        Size::new(on.width * lid.width, on.height * lid.height),
    )
}

/// The logo as drawn in its spot: the firmware's own, as it's shaped
/// (`aspect`, its width to its height), as big as fits and in the middle;
/// without one, the spot.
fn logo_drawn(spot: Rectangle, aspect: Option<f32>) -> Rectangle {
    let Some(aspect) = aspect else {
        return spot;
    };
    let size = if spot.width / spot.height > aspect {
        Size::new(spot.height * aspect, spot.height)
    } else {
        Size::new(spot.width, spot.width / aspect)
    };
    Rectangle::new(
        Point::new(
            spot.center_x() - size.width / 2.0,
            spot.center_y() - size.height / 2.0,
        ),
        size,
    )
}

/// What a click on the picture lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Key(u8),
    Part(Part),
}

/// The picture: each key and light in the colour it gets, the picked ones
/// outlined, picked by a click or by dragging over the keys.
struct Board<'a> {
    editor: &'a Editor,
    /// The lights beside the keys the laptop has, and the picture too.
    parts: Vec<Part>,
}

/// The pointer over the picture.
#[derive(Default)]
struct Pointer {
    over: Option<Target>,
    /// Picking or letting go of the keys dragged over, as the first was.
    dragging: Option<bool>,
}

impl Board<'_> {
    fn layout(&self) -> &Layout {
        &self.editor.laptop.layout
    }

    fn beside(&self) -> Beside {
        Beside::new(self.layout(), &self.parts, self.editor.laptop.logo.as_ref())
    }

    /// Where `part` is picked on the picture, if the laptop has it.
    fn part(&self, part: Part) -> Option<Rectangle> {
        self.beside().area(part)
    }

    /// Key widths to pixels: the scale, and where the keys' top left corner
    /// goes, the picture centred in `bounds`.
    fn fit(&self, bounds: Rectangle) -> (f32, Point) {
        let extent = self.beside().extent(self.layout());
        let scale = (bounds.width / extent.width).min(bounds.height / extent.height);
        let corner = Point::new(
            (bounds.width - extent.width * scale) / 2.0 - extent.x * scale,
            (bounds.height - extent.height * scale) / 2.0 - extent.y * scale,
        );
        (scale, corner)
    }

    /// `rect` in pixels, `gap` narrower and shorter.
    fn on_screen(rect: Rectangle, gap: f32, scale: f32, corner: Point) -> Rectangle {
        Rectangle::new(
            Point::new(
                corner.x + (rect.x + gap / 2.0) * scale,
                corner.y + (rect.y + gap / 2.0) * scale,
            ),
            Size::new((rect.width - gap) * scale, (rect.height - gap) * scale),
        )
    }

    fn target_at(&self, bounds: Rectangle, at: Point) -> Option<Target> {
        let (scale, corner) = self.fit(bounds);
        if let Some(key) = self
            .layout()
            .keys
            .iter()
            .find(|key| Self::on_screen(key.rect, KEY_GAP, scale, corner).contains(at))
        {
            return Some(Target::Key(key.code));
        }
        self.parts
            .iter()
            .copied()
            .find(|&part| {
                self.part(part)
                    .is_some_and(|rect| Self::on_screen(rect, 0.0, scale, corner).contains(at))
            })
            .map(Target::Part)
    }

    /// An outline around `rect`, rounded by `radius`, off its edge into
    /// the gap around it: for what's marked, `width` in the accent, or in
    /// the text's colour on a light (`lit`) close to the accent; thin under
    /// the pointer; a hairline on the edge otherwise.
    #[allow(clippy::too_many_arguments)]
    fn ring(
        frame: &mut canvas::Frame,
        rect: Rectangle,
        radius: f32,
        lit: Option<Color>,
        colors: &theme::Colors,
        marked: bool,
        over: bool,
        width: f32,
    ) {
        let (outline, width) = if marked {
            let near = lit.is_some_and(|lit| {
                let accent = colors.strong.into_rgba8();
                let distance: f32 = (0..3)
                    .map(|at| (f32::from(lit.0[at]) - f32::from(accent[at])).powi(2))
                    .sum();
                distance.sqrt() < 80.0
            });
            (if near { colors.text } else { colors.strong }, width)
        } else if over {
            (colors.text, 1.5)
        } else {
            (colors.border, 1.0)
        };
        let off = if marked || over { width / 2.0 } else { 0.0 };
        let path = canvas::Path::rounded_rectangle(
            Point::new(rect.x - off, rect.y - off),
            Size::new(rect.width + 2.0 * off, rect.height + 2.0 * off),
            (radius + off).into(),
        );
        frame.stroke(
            &path,
            canvas::Stroke::default()
                .with_color(outline)
                .with_width(width),
        );
    }

    /// What reads on a light of `color`: black on light ones, white on
    /// dark.
    fn ink(color: Color) -> iced::Color {
        let [r, g, b] = color.0;
        let light = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 140.0;
        if light {
            iced::Color::BLACK
        } else {
            iced::Color::WHITE
        }
    }

    /// A label on a light of `color`: dark ink on light ones, light on dark.
    /// None at all on a picture too small to read them, and none on a key
    /// too small for its own.
    fn label(frame: &mut canvas::Frame, content: &str, rect: Rectangle, color: Color, scale: f32) {
        const LEAST: f32 = 7.0;
        let size = scale * 0.26;
        // Smaller for a long label on a narrow key: Copilot.
        let fits = size
            .min(rect.width / (content.chars().count() as f32 * 0.6))
            .min(rect.height * 0.8);
        if size < LEAST || fits < LEAST {
            return;
        }
        frame.fill_text(canvas::Text {
            content: content.to_owned(),
            position: rect.center(),
            color: Self::ink(color),
            size: fits.into(),
            font: theme::font(),
            align_x: TextAlignment::Center,
            align_y: alignment::Vertical::Center,
            ..canvas::Text::default()
        });
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
            .and_then(|at| self.target_at(bounds, at));
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => match under? {
                Target::Key(key) => {
                    let pick = !self.editor.marked(key);
                    pointer.dragging = Some(pick);
                    Some(canvas::Action::publish(Msg::Pick(key, pick)).and_capture())
                }
                Target::Part(part) => Some(canvas::Action::publish(Msg::Part(part)).and_capture()),
            },
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                pointer.dragging = None;
                None
            }
            mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft => {
                let moved = pointer.over != under;
                pointer.over = under;
                if let (Some(pick), Some(Target::Key(key))) = (pointer.dragging, under)
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
        let (scale, corner) = self.fit(bounds);
        let rounded = |rect: Rectangle, radius: f32| {
            canvas::Path::rounded_rectangle(rect.position(), rect.size(), radius.into())
        };
        let beside = self.beside();
        if let Some(deck) = beside.deck {
            let path = rounded(Self::on_screen(deck, 0.0, scale, corner), 0.0);
            frame.fill(&path, colors.idle);
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(colors.border)
                    .with_width(1.0),
            );
        }
        // Rings go into the gaps between the keys, as wide as fits.
        let ring = (KEY_GAP * scale * 0.8).clamp(1.5, 3.0);
        let keys: Vec<_> = self
            .layout()
            .keys
            .iter()
            .map(|key| {
                let rect = Self::on_screen(key.rect, KEY_GAP, scale, corner);
                (key, rect, self.editor.shown(key))
            })
            .collect();
        for &(key, rect, color) in &keys {
            frame.fill(&rounded(rect, scale * 0.12), led(color));
            Self::label(&mut frame, &key.label, rect, color, scale);
        }
        for &(key, rect, color) in &keys {
            Self::ring(
                &mut frame,
                rect,
                scale * 0.12,
                Some(color),
                &colors,
                self.editor.marked(key.code),
                pointer.over == Some(Target::Key(key.code)),
                ring,
            );
        }
        if let (Some(lid), Some(spot)) = (beside.lid, beside.logo) {
            let rect = Self::on_screen(lid, 0.0, scale, corner);
            let radius = if beside.deck.is_some() { 0.0 } else { 0.2 };
            let path = rounded(rect, scale * radius);
            frame.fill(&path, colors.idle);
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(colors.border)
                    .with_width(1.0),
            );
            let logo = self.editor.laptop.logo.as_ref();
            let drawn = Self::on_screen(logo_drawn(spot, beside.logo_aspect), 0.0, scale, corner);
            let color = self.editor.shown_part(Part::Logo, lightbar::LOGO_AT);
            match logo {
                // The firmware's own logo, lit.
                Some(logo) => {
                    frame.draw_image(drawn, canvas::Image::new(logo.tinted(color.0)));
                }
                None => frame.fill(&rounded(drawn, drawn.height / 4.0), led(color)),
            }
            // Ringed a little off it, on the lid.
            let margin = scale * 0.15;
            let around = Rectangle::new(
                Point::new(drawn.x - margin, drawn.y - margin),
                Size::new(drawn.width + 2.0 * margin, drawn.height + 2.0 * margin),
            );
            let over = pointer.over == Some(Target::Part(Part::Logo));
            let marked = self.editor.marked_part(Part::Logo);
            if marked || over {
                Self::ring(
                    &mut frame,
                    around,
                    margin * 2.0,
                    None,
                    &colors,
                    marked,
                    over,
                    3.0,
                );
            }
        }
        if let Some(pad) = beside.dark_pad {
            let path = rounded(Self::on_screen(pad, 0.0, scale, corner), scale * 0.25);
            frame.fill(&path, colors.track);
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(colors.border)
                    .with_width(1.0),
            );
        }
        if let Some(pad) = beside.pad {
            let rect = Self::on_screen(pad, 0.0, scale, corner);
            let radius = scale * 0.25;
            let bands = beside.bands(self.layout().stripes().len());
            let last = bands.len().saturating_sub(1);
            for (row, band) in bands.into_iter().enumerate() {
                let band = Self::on_screen(band, 0.0, scale, corner);
                let top = if row == 0 { radius } else { 0.0 };
                let bottom = if row == last { radius } else { 0.0 };
                let corners = iced::border::Radius {
                    top_left: top,
                    top_right: top,
                    bottom_right: bottom,
                    bottom_left: bottom,
                };
                let color = self
                    .editor
                    .shown_part(Part::Bar, lightbar::bar_row_at(row as u8));
                frame.fill(
                    &canvas::Path::rounded_rectangle(band.position(), band.size(), corners),
                    led(color),
                );
                if row > 0 {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(band.x, band.y),
                            Point::new(band.x + band.width, band.y),
                        ),
                        canvas::Stroke::default()
                            .with_color(colors.card)
                            .with_width(2.0),
                    );
                }
            }
            Self::ring(
                &mut frame,
                rect,
                radius,
                Some(self.editor.shown_part(Part::Bar, lightbar::bar_row_at(0))),
                &colors,
                self.editor.marked_part(Part::Bar),
                pointer.over == Some(Target::Part(Part::Bar)),
                3.0,
            );
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
            .and_then(|at| self.target_at(bounds, at))
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
        let nix = editor.nix();
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
        editor.saved();
        assert!(!editor.unsaved());
        assert_eq!(editor.update(Msg::New), Outcome::New);
        let _ = editor.update(Msg::Name(" ".into()));
        assert_eq!(editor.update(Msg::Save), Outcome::Nothing);
    }

    #[test]
    fn asks_before_dropping_changes() {
        let open = || Msg::Open("wave".into());
        let mut editor = Editor::new();
        // Nothing to lose: straight there.
        assert_eq!(editor.update(open()), Outcome::Open("wave".into()));

        let _ = editor.update(Msg::Name("mine".into()));
        assert_eq!(editor.update(open()), Outcome::Nothing);
        assert_eq!(editor.leaving, Some(Leaving::Open("wave".into())));
        assert_eq!(editor.update(Msg::Discard), Outcome::Open("wave".into()));

        // Kept: nothing left of the question, and a discard goes nowhere.
        assert_eq!(editor.update(Msg::New), Outcome::Nothing);
        assert_eq!(editor.update(Msg::Stay), Outcome::Nothing);
        assert_eq!(editor.leaving, None);
        assert_eq!(editor.update(Msg::Discard), Outcome::Nothing);
        assert!(editor.unsaved());

        // Going on editing answers it too.
        assert_eq!(editor.update(Msg::New), Outcome::Nothing);
        let _ = editor.update(Msg::PickAll);
        assert_eq!(editor.leaving, None);
    }

    /// Saved on the way to another profile: there once the daemon has it.
    #[test]
    fn saves_then_switches() {
        let mut editor = Editor::new();
        let _ = editor.update(Msg::Swatch(Color([255, 0, 0])));
        assert_eq!(editor.update(Msg::Open("wave".into())), Outcome::Nothing);
        let save = editor.update(Msg::SaveAndLeave(Some("custom 2".into())));
        assert!(matches!(save, Outcome::Save(ref profile) if profile.contains("custom 2")));
        assert_eq!(editor.saved(), Outcome::Open("wave".into()));
        assert!(!editor.unsaved());
        // A save of its own goes nowhere after.
        let _ = editor.update(Msg::Open("cycle".into()));
        let _ = editor.update(Msg::Swatch(Color::OFF));
        let _ = editor.update(Msg::Save);
        assert_eq!(editor.saved(), Outcome::Nothing);
        // Reverted: nothing unsaved, the daemon's again.
        let _ = editor.update(Msg::Swatch(Color([0, 255, 0])));
        assert_eq!(editor.update(Msg::Revert), Outcome::Revert);
        assert!(!editor.unsaved());
    }

    fn known(standard: &[&str], parts: Vec<Part>) -> Known {
        Known {
            standard: standard.iter().map(|&name| name.to_owned()).collect(),
            saved: Vec::new(),
            shown: standard.first().map(|&name| name.to_owned()),
            parts,
            previewing: false,
        }
    }

    #[test]
    fn shows_the_question() {
        let known = known(&["wave"], Vec::new());
        let mut editor = Editor::new();
        let _ = editor.update(Msg::Name("mine".into()));
        let mut ui = iced_test::simulator(editor.view(1000.0, &known));
        assert!(ui.find("Keep editing").is_err());
        assert!(ui.find("Revert").is_ok());
        drop(ui);

        let _ = editor.update(Msg::New);
        let mut ui = iced_test::simulator(editor.view(1000.0, &known));
        assert!(ui.find("The changes to mine aren't saved").is_ok());
        ui.click("Discard them").unwrap();
        let messages: Vec<Msg> = ui.into_messages().collect();
        assert!(matches!(messages[..], [Msg::Discard]));
    }

    /// The profiles as chips: another one opened, the one on the keyboard
    /// and in the editor not again; Save and Revert only once changed.
    #[test]
    fn picks_from_the_strip() {
        let known = known(&["orange", "typing"], Vec::new());
        let editor = Editor::standard("orange");
        let mut ui = iced_test::simulator(editor.view(800.0, &known));
        assert!(ui.find("Revert").is_err() && ui.find("Save").is_err());
        assert!(ui.find("On the keyboard").is_ok());
        ui.click("Orange").unwrap();
        ui.click("Typing").unwrap();
        let messages: Vec<Msg> = ui.into_messages().collect();
        assert!(matches!(&messages[..], [Msg::Open(name)] if name == "typing"));

        // A standard one is saved as a copy only.
        let mut editor = Editor::standard("orange");
        let _ = editor.update(Msg::Swatch(Color::OFF));
        let mut ui = iced_test::simulator(editor.view(800.0, &known));
        assert!(ui.find("Save").is_err());
        ui.click("Save as copy").unwrap();
        let messages: Vec<Msg> = ui.into_messages().collect();
        assert!(matches!(&messages[..], [Msg::SaveAs(name)] if name == "orange 2"));
    }

    /// Without the daemon, a standard profile by its effect's name.
    #[test]
    fn a_standard_profile_by_its_name() {
        assert_eq!(Editor::standard("wave").look.effect, Effect::Wave);
        assert_eq!(Editor::standard("wave").look.color, Color::OFF);
        assert_eq!(Editor::standard("breathe").look.effect, Effect::Breathe);
        let other = Editor::standard("Static 2");
        assert_eq!(
            (other.name(), other.look.effect),
            ("Static 2", Effect::Static)
        );
        assert!(!other.unsaved() && Editor::fresh("profile 1").unsaved());
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

    /// The light bar and the logo, named among the keys, kept as read.
    #[test]
    fn keeps_the_light_bar_and_the_logo() {
        let read = "name = \"lit\"\neffect = \"cycle\"\n\n[keys]\nlogo = \"#ffffff\"\nw = \"off\"\n\n[[zones]]\nkeys = [\"bar\"]\neffect = \"breathe\"\ncolor = \"#ff6000\"\n";
        let editor = Editor::from_toml(read).unwrap();
        assert_eq!(editor.parts[&Part::Logo], Color([255; 3]));
        assert_eq!(editor.keys[&0x1a], Color::OFF);
        assert!(editor.zones[0].keys.is_empty());
        assert!(editor.zones[0].parts.contains(&Part::Bar));
        let written = editor.toml();
        assert!(written.contains("logo = \"#ffffff\""), "{written}");
        assert!(written.contains("keys = [\"bar\"]"), "{written}");
        assert_eq!(Editor::from_toml(&written).unwrap().toml(), written);
    }

    /// The bar and the logo picked in the profile as keys are: the colour
    /// paints them, and back to the effect takes it off; in a zone, they
    /// join it.
    #[test]
    fn picks_the_light_bar_and_the_logo() {
        let mut editor = Editor::new();
        let _ = editor.update(Msg::Part(Part::Logo));
        let _ = editor.update(Msg::Pick(0x04, true));
        assert!(editor.marked_part(Part::Logo) && editor.parts.is_empty());
        assert_eq!(editor.picked_what(), "1 key and the logo");
        let _ = editor.update(Msg::Part(Part::Bar));
        let _ = editor.update(Msg::Pick(0x05, true));
        assert_eq!(editor.picked_what(), "2 keys, the touchpad and the logo");
        let _ = editor.update(Msg::Part(Part::Bar));
        let _ = editor.update(Msg::Pick(0x05, false));
        let _ = editor.update(Msg::Swatch(Color([255, 0, 0])));
        assert_eq!(editor.parts[&Part::Logo], Color([255, 0, 0]));
        assert_eq!(editor.keys[&0x04], Color([255, 0, 0]));
        assert!(editor.toml().contains("logo = \"#ff0000\""));
        // Let go, it keeps its colour.
        let _ = editor.update(Msg::Part(Part::Logo));
        assert!(!editor.marked_part(Part::Logo));
        assert_eq!(editor.parts[&Part::Logo], Color([255, 0, 0]));
        let _ = editor.update(Msg::Part(Part::Logo));
        let _ = editor.update(Msg::Unpaint);
        assert!(editor.parts.is_empty());
        let _ = editor.update(Msg::PickNone);
        assert!(!editor.marked_part(Part::Logo));
        let _ = editor.update(Msg::AddZone);
        let _ = editor.update(Msg::Part(Part::Bar));
        assert!(editor.zones[0].parts.contains(&Part::Bar));
        assert!(
            editor.toml().contains("keys = [\"bar\"]"),
            "{}",
            editor.toml()
        );
    }

    /// A drag into the dark keeps its hue; a colour typed in brings its own.
    #[test]
    fn keeps_the_hue_dragged_in() {
        assert_eq!(
            picked_hue(picker::rgb(200.0, 1.0, 1.0 / 255.0), 200.0),
            200.0
        );
        assert_eq!(picked_hue([128, 128, 128], 200.0), 200.0);
        let (red, _, _) = picker::hsv([255, 0, 0]);
        assert_eq!(picked_hue([255, 0, 0], 200.0), red);
    }

    /// Removing a zone not shown leaves the keys picked on the profile, and
    /// the colour still paints them.
    #[test]
    fn removing_another_zone_keeps_the_picks() {
        let mut editor = Editor::new();
        let _ = editor.update(Msg::AddZone);
        let _ = editor.update(Msg::Layer(0));
        let _ = editor.update(Msg::Pick(0x04, true));
        let _ = editor.update(Msg::RemoveZone(1));
        let _ = editor.update(Msg::ConfirmRemoveZone);
        assert!(editor.zones.is_empty());
        assert_eq!(editor.slot, Slot::Keys);
        let _ = editor.update(Msg::Swatch(Color([0, 255, 0])));
        assert_eq!(editor.keys[&0x04], Color([0, 255, 0]));
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
        let _ = editor.update(Msg::RemoveZone(2));
        assert_eq!(editor.zones.len(), 2, "asked first");
        let _ = editor.update(Msg::ConfirmRemoveZone);
        assert_eq!((editor.zones.len(), editor.layer), (1, 1));
        let _ = editor.update(Msg::RemoveZone(1));
        let _ = editor.update(Msg::KeepZone);
        let _ = editor.update(Msg::ConfirmRemoveZone);
        assert_eq!(editor.zones.len(), 1, "kept");
    }

    #[test]
    fn a_click_picks_the_key_under_it() {
        let editor = Editor::new();
        let board = Board {
            editor: &editor,
            parts: Vec::new(),
        };
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1891.4, 568.9));
        let at = |x, y| board.target_at(bounds, Point::new(x, y));
        // Scale 100: Esc in the top left, the numpad's 5 at (16.75, 3.19).
        assert_eq!(at(40.0, 40.0), Some(Target::Key(0x29)));
        assert_eq!(at(1675.0, 319.0), Some(Target::Key(0x5d)));
        // The tall Plus and Enter, low and high.
        assert_eq!(at(1850.0, 340.0), Some(Target::Key(0x57)));
        assert_eq!(at(1850.0, 550.0), Some(Target::Key(0x58)));
        // Fn before Copilot, Right under NumLock.
        assert_eq!(at(1175.0, 519.0), Some(Target::Key(0xf0)));
        assert_eq!(at(1275.0, 519.0), Some(Target::Key(0xe4)));
        assert_eq!(at(1581.0, 519.0), Some(Target::Key(0x4f)));
        // Between keys, nothing.
        assert_eq!(at(98.0, 30.0), None);
    }

    /// A laptop's touchpad and logo where its layout has none: not drawn,
    /// nothing to click, the picture the keys alone.
    #[test]
    fn no_touchpad_or_logo_where_the_layout_has_none() {
        let editor = Editor::new();
        assert!(editor.drawn(&[Part::Bar, Part::Logo]).is_empty());
        let board = Board {
            editor: &editor,
            parts: vec![Part::Bar, Part::Logo],
        };
        assert_eq!(
            (board.part(Part::Bar), board.part(Part::Logo)),
            (None, None)
        );
        assert_eq!(
            board.beside().extent(board.layout()),
            board.layout().keys_span()
        );
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1891.4, 1080.0));
        assert_eq!(board.target_at(bounds, Point::new(945.0, 1000.0)), None);
        let names = ["bar".to_owned(), "logo".to_owned(), "lid".to_owned()];
        assert_eq!(parts(Some(&names)), [Part::Bar, Part::Logo]);
        assert_eq!(parts(None), []);
    }

    fn titan() -> Editor {
        Editor {
            laptop: Arc::new(Laptop {
                machine: Default::default(),
                layout: Arc::new(layouts::built_in("ms-1824.toml")),
                logo: None,
            }),
            ..Editor::new()
        }
    }

    /// On the Titan's own picture: the arrows, the numpad and the bottom
    /// row as measured.
    #[test]
    fn a_click_on_the_titan_picks_its_key() {
        let editor = titan();
        let board = Board {
            editor: &editor,
            parts: Vec::new(),
        };
        // Scale 100, the picture as big as the deck, at the keys' corner.
        let deck = board.beside().extent(board.layout());
        assert_eq!(Some(deck), board.layout().deck());
        let bounds = Rectangle::new(
            Point::ORIGIN,
            Size::new(deck.width * 100.0, deck.height * 100.0),
        );
        let at = |x: f32, y: f32| {
            board.target_at(bounds, Point::new(x - deck.x * 100.0, y - deck.y * 100.0))
        };
        assert_eq!(at(1398.0, 411.0), Some(Target::Key(0x52)));
        assert_eq!(at(1398.0, 511.0), Some(Target::Key(0x51)));
        assert_eq!(at(1498.0, 511.0), Some(Target::Key(0x4f)));
        assert_eq!(at(1598.0, 511.0), Some(Target::Key(0x62)));
        assert_eq!(at(1698.0, 511.0), Some(Target::Key(0x63)));
        assert_eq!(at(1117.0, 511.0), Some(Target::Key(0xe4)));
        assert_eq!(at(1498.0, 30.0), Some(Target::Key(0x46)));
        assert_eq!(at(1498.0, 110.0), Some(Target::Key(0x53)));
        assert_eq!(at(1798.0, 460.0), Some(Target::Key(0x58)));
        assert_eq!(at(1798.0, 260.0), Some(Target::Key(0x57)));
        assert_eq!(at(1798.0, 30.0), Some(Target::Key(0x66)));
    }

    /// The Titan's touchpad and lid where they are, there only when the
    /// laptop has them: the keys and touchpad on the deck, the lid above it
    /// as wide, the logo on it left of its middle as it sits.
    #[test]
    fn the_titans_touchpad_and_logo() {
        let editor = titan();
        let lit = Board {
            editor: &editor,
            parts: vec![Part::Bar, Part::Logo],
        };
        let beside = lit.beside();
        let (pad, lid, spot) = (
            beside.pad.unwrap(),
            beside.lid.unwrap(),
            beside.logo.unwrap(),
        );
        let keys = editor.laptop.layout.keys_span();
        let deck = beside.deck.unwrap();
        assert_eq!(Some(pad), editor.laptop.layout.part(Part::Bar));
        assert!(pad.y > keys.y + keys.height);
        let on_deck = |rect: Rectangle| {
            deck.intersection(&rect)
                .is_some_and(|common| (common.area() - rect.area()).abs() < 1e-3)
        };
        assert!(on_deck(keys) && on_deck(pad));
        // The lid above the deck, as wide, as it's shaped.
        assert!(lid.y + lid.height < deck.y);
        assert_eq!((lid.x, lid.width), (deck.x, deck.width));
        assert!((lid.width / lid.height - 1.504).abs() < 1e-4);
        assert!(spot.center_x() < lid.center_x());
        // Inside the lid, near its left edge.
        assert!(spot.x >= lid.x && spot.x + spot.width <= lid.x + lid.width);
        assert!((spot.x - lid.x - 0.099 * lid.width).abs() < 1e-4);
        let extent = beside.extent(&editor.laptop.layout);
        // Scale 100, the picture as wide as the keys.
        let bounds = Rectangle::new(
            Point::ORIGIN,
            Size::new(extent.width * 100.0, extent.height * 100.0),
        );
        let on = |rect: Rectangle| {
            Point::new(
                (rect.center_x() - extent.x) * 100.0,
                (rect.center_y() - extent.y) * 100.0,
            )
        };
        let (logo, pad) = (on(spot), on(pad));
        assert_eq!(lit.target_at(bounds, logo), Some(Target::Part(Part::Logo)));
        // The logo picks it, not the rest of the lid.
        assert_eq!(lit.target_at(bounds, on(lid)), None);
        assert_eq!(lit.target_at(bounds, pad), Some(Target::Part(Part::Bar)));
        let esc = editor
            .laptop
            .layout
            .keys
            .iter()
            .find(|key| key.code == 0x29)
            .unwrap()
            .rect;
        assert_eq!(lit.target_at(bounds, on(esc)), Some(Target::Key(0x29)));
        let dark = Board {
            editor: &editor,
            parts: Vec::new(),
        };
        assert_eq!(dark.target_at(bounds, logo), None);
        assert_eq!(dark.target_at(bounds, pad), None);
        // Its touchpad still on the deck, dark; no lid.
        let unlit = dark.beside();
        assert_eq!((unlit.pad, unlit.lid), (None, None));
        assert_eq!(unlit.dark_pad, editor.laptop.layout.part(Part::Bar));
    }

    /// The touchpad's rows from a wave as the daemon lights them: in order
    /// from the top, and from the bottom too, below the keys' plane.
    #[test]
    fn a_wave_runs_down_the_touchpad() {
        let wave = |origin: &str| {
            Editor::from_toml(&format!(
                "name = \"w\"\neffect = \"wave\"\ncolors = [\"#ff0000\", \"#0000ff\"]\norigin = \"{origin}\"\n"
            ))
            .unwrap()
        };
        for origin in ["top", "bottom"] {
            let editor = wave(origin);
            let rows: Vec<[u16; 2]> = (0..lightbar::BAR_ROWS).map(lightbar::bar_row_at).collect();
            let lags: Vec<f64> = rows.iter().map(|&at| editor.look.lag(at)).collect();
            assert!(lags.iter().all(|&lag| lag > 0.0), "{origin}: {lags:?}");
            assert!(lags.is_sorted_by(|a, b| a < b), "{origin}: {lags:?}");
            let blue: Vec<u8> = rows
                .iter()
                .map(|&at| editor.shown_part(Part::Bar, at).0[2])
                .collect();
            assert!(blue.is_sorted_by(|a, b| a < b), "{origin}: {blue:?}");
            assert!(blue[0] > 0 && blue[4] < 255, "{origin}: {blue:?}");
        }
        // The same maths as the daemon's, the keys included.
        let editor = wave("top");
        let esc = editor
            .laptop
            .layout
            .plane(editor.laptop.layout.keys[0].rect.center());
        let lag = steelseries::wave_lag(
            &lighting::Wave {
                origin: Origin::Top,
                ..lighting::Wave::default()
            },
            esc,
        );
        assert_eq!(editor.look.lag(esc), lag);
        // A light of its own keeps it.
        let mut editor = wave("top");
        let _ = editor.update(Msg::Part(Part::Bar));
        let _ = editor.update(Msg::Swatch(Color([0, 255, 0])));
        assert_eq!(
            editor.shown_part(Part::Bar, lightbar::bar_row_at(2)),
            Color([0, 255, 0])
        );
    }
}
