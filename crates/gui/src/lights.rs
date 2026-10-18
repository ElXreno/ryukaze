//! The keyboard light's editor: a picture of the Alpha 17's keys to pick and
//! paint, an effect and what it takes, shown on the keyboard as it goes (as
//! a preview the daemon drops at the next profile or scenario) and copied
//! out as Nix, a profile for `keyboard.profiles`.

use std::collections::{BTreeMap, BTreeSet};

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
}

const ORIGINS: [(Origin, &str); 5] = [
    (Origin::Middle, "middle"),
    (Origin::Left, "left"),
    (Origin::Right, "right"),
    (Origin::Top, "top"),
    (Origin::Bottom, "bottom"),
];

/// Colours to pick at a click, as the LEDs show them rather than a screen.
const SWATCHES: [Color; 9] = [
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
const GROUPS: [(&str, &str); 7] = [
    ("letters", "Letters"),
    ("digits", "Digits"),
    ("function", "F keys"),
    ("arrows", "Arrows"),
    ("numpad", "Numpad"),
    ("modifiers", "Modifiers"),
    ("wasd", "WASD"),
];

pub struct Editor {
    effect: Effect,
    /// The keys without their own: still, breathing, between flashes.
    color: Color,
    fade_to: Color,
    flash: Color,
    /// A cycle's or a wave's; none, the rainbow.
    colors: Vec<Color>,
    period_ms: u32,
    flash_ms: u32,
    origin: Origin,
    inward: bool,
    /// Keys with colours of their own.
    keys: BTreeMap<u8, Color>,
    selected: BTreeSet<u8>,
    /// What painting, or setting a colour, takes.
    brush: Color,
    /// The brush as typed.
    hex: String,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Toggle(u8),
    SelectAll,
    SelectNone,
    SelectGroup(&'static str),
    Paint,
    Unpaint,
    Brush(Color),
    Hex(String),
    Channel(usize, u8),
    Effect(Effect),
    SetColor,
    SetFadeTo,
    SetFlash,
    AddColor,
    Rainbow,
    Period(u32),
    FlashMs(u32),
    /// A slider let go: show what it set.
    Released,
    Origin(Origin),
    Inward(bool),
    Copy,
    Done,
}

/// What a change asks of the window.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// Show this profile, TOML, on the keyboard.
    Preview(String),
    /// Put this on the clipboard.
    Copy(String),
    Done,
}

impl Editor {
    pub fn new() -> Self {
        let white = Color([255; 3]);
        Self {
            effect: Effect::Static,
            color: white,
            fade_to: Color::OFF,
            flash: white,
            colors: Vec::new(),
            period_ms: 4000,
            flash_ms: 500,
            origin: Origin::Middle,
            inward: false,
            keys: BTreeMap::new(),
            selected: BTreeSet::new(),
            brush: white,
            hex: white.to_string(),
        }
    }

    pub fn update(&mut self, msg: Msg) -> Outcome {
        match msg {
            Msg::Toggle(key) => {
                if !self.selected.remove(&key) {
                    self.selected.insert(key);
                }
                return Outcome::Nothing;
            }
            Msg::SelectAll => {
                self.selected = board().iter().map(|key| key.code).collect();
                return Outcome::Nothing;
            }
            Msg::SelectNone => {
                self.selected.clear();
                return Outcome::Nothing;
            }
            Msg::SelectGroup(group) => {
                self.selected
                    .extend(keyboard::named(group).unwrap_or_default());
                return Outcome::Nothing;
            }
            Msg::Brush(color) => {
                self.set_brush(color);
                return Outcome::Nothing;
            }
            Msg::Hex(hex) => {
                if let Ok(color) = hex.parse() {
                    self.brush = color;
                }
                self.hex = hex;
                return Outcome::Nothing;
            }
            Msg::Channel(channel, value) => {
                let mut color = self.brush;
                color.0[channel] = value;
                self.set_brush(color);
                return Outcome::Nothing;
            }
            Msg::Period(period_ms) => {
                self.period_ms = period_ms;
                return Outcome::Nothing;
            }
            Msg::FlashMs(flash_ms) => {
                self.flash_ms = flash_ms;
                return Outcome::Nothing;
            }
            Msg::Copy => return Outcome::Copy(self.nix()),
            Msg::Done => return Outcome::Done,
            Msg::Paint => {
                for &key in &self.selected {
                    self.keys.insert(key, self.brush);
                }
            }
            Msg::Unpaint => {
                for key in &self.selected {
                    self.keys.remove(key);
                }
            }
            Msg::Effect(effect) => self.effect = effect,
            Msg::SetColor => self.color = self.brush,
            Msg::SetFadeTo => self.fade_to = self.brush,
            Msg::SetFlash => self.flash = self.brush,
            Msg::AddColor => {
                if self.colors.len() < *keyboard::COLORS.end() {
                    self.colors.push(self.brush);
                }
            }
            Msg::Rainbow => self.colors.clear(),
            Msg::Released => {}
            Msg::Origin(origin) => self.origin = origin,
            Msg::Inward(inward) => self.inward = inward,
        }
        Outcome::Preview(self.toml())
    }

    /// What the keyboard should show as the editor opens.
    pub fn preview(&self) -> String {
        self.toml()
    }

    fn set_brush(&mut self, color: Color) {
        self.brush = color;
        self.hex = color.to_string();
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
        self.period_ms.max(shortest)
    }

    /// The profile as the config takes it, field by field.
    fn fields(&self) -> Vec<(&'static str, Field)> {
        let hex = |color: Color| Field::Str(color.to_string());
        let mut fields = vec![("name", Field::Str("custom".into()))];
        if self.effect != Effect::Static {
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
                }
            }
            Effect::Reactive => {
                fields.push(("color", hex(self.color)));
                fields.push(("flash", hex(self.flash)));
                fields.push(("flash_ms", Field::Int(self.flash_ms)));
            }
        }
        let names = keyboard::names();
        let keys: Vec<(String, String)> = self
            .keys
            .iter()
            .filter_map(|(code, color)| {
                let (name, _) = names.iter().find(|(_, known)| known == code)?;
                Some((name.clone(), color.to_string()))
            })
            .collect();
        if !keys.is_empty() {
            fields.push(("keys", Field::Keys(keys)));
        }
        fields
    }

    fn toml(&self) -> String {
        let mut table = toml::Table::new();
        for (key, field) in self.fields() {
            let value = match field {
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
            };
            table.insert(key.into(), value);
        }
        toml::to_string(&table).unwrap_or_default()
    }

    /// For `services.ryukaze.settings.keyboard.profiles`.
    fn nix(&self) -> String {
        let entries: Vec<(String, nix::Value)> = self
            .fields()
            .into_iter()
            .map(|(key, field)| {
                let value = match field {
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
                };
                (key.to_owned(), value)
            })
            .collect();
        nix::attrset(&entries)
    }

    /// The colour the picture gives a key: its own, or the effect's at
    /// about where it sits.
    fn shown(&self, key: &Key) -> Color {
        if let Some(&color) = self.keys.get(&key.code) {
            return color;
        }
        match self.effect {
            Effect::Cycle => self.round()[0],
            Effect::Wave => {
                let round = self.round();
                let along = (key.rect.center_x() / ACROSS).clamp(0.0, 0.999);
                round[(along * round.len() as f32) as usize]
            }
            _ => self.color,
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let header = row![
            text("Keyboard light").size(20),
            space::horizontal(),
            small("Copy as Nix", Msg::Copy),
            small("Done", Msg::Done),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let picture = canvas(Board { editor: self })
            .width(Length::Fill)
            .height(320);
        let hint = text(
            "Shown on the keyboard as you go, until another profile is picked; \
             Copy as Nix to keep it, under services.ryukaze.settings.keyboard.profiles",
        )
        .size(12)
        .style(theme::muted);
        column![
            header,
            picture,
            row![self.keys_panel(), self.brush_panel(), self.effect_panel()].spacing(12),
            hint,
        ]
        .spacing(12)
        .into()
    }

    fn keys_panel(&self) -> Element<'_, Msg> {
        let mut groups =
            row![small("All", Msg::SelectAll), small("None", Msg::SelectNone)].spacing(6);
        for (group, label) in GROUPS {
            groups = groups.push(small(label, Msg::SelectGroup(group)));
        }
        panel(
            "Keys",
            column![
                text(format!(
                    "{} picked: click keys on the picture, or",
                    self.selected.len()
                ))
                .size(13)
                .style(theme::muted),
                groups.wrap().vertical_spacing(6),
                row![
                    small("Paint with the colour", Msg::Paint),
                    small("Back to the effect", Msg::Unpaint),
                ]
                .spacing(6),
            ]
            .spacing(10),
        )
    }

    fn brush_panel(&self) -> Element<'_, Msg> {
        let channel = |index: usize, name: &'static str| {
            row![
                text(name).width(16),
                slider(0..=255, self.brush.0[index], move |value| Msg::Channel(
                    index, value
                ))
                .style(theme::level),
                text(self.brush.0[index].to_string())
                    .width(32)
                    .align_x(Alignment::End),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        };
        let swatches = row(SWATCHES.map(|color| {
            button(space().width(22).height(22))
                .padding(0)
                .on_press(Msg::Brush(color))
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
        .spacing(6);
        panel(
            "Colour",
            column![
                row![
                    swatch(self.brush, 32.0),
                    text_input("#rrggbb", &self.hex)
                        .on_input(Msg::Hex)
                        .width(110),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                channel(0, "R"),
                channel(1, "G"),
                channel(2, "B"),
                swatches,
            ]
            .spacing(10),
        )
    }

    fn effect_panel(&self) -> Element<'_, Msg> {
        let effects = row(Effect::ALL.map(|effect| {
            choice(
                theme_label(effect.name()),
                self.effect == effect,
                Msg::Effect(effect),
            )
        }))
        .spacing(6);
        let set = |label: &'static str, color: Color, msg: Msg| {
            row![
                swatch(color, 22.0),
                text(label).width(Length::Fill),
                small("Take the colour", msg),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        };
        let period = row![
            text("Period").width(56),
            slider(keyboard::PERIODS_MS.clone(), self.period(), Msg::Period)
                .step(500u32)
                .on_release(Msg::Released)
                .style(theme::level),
            text(format!("{:.1} s", self.period() as f32 / 1000.0))
                .width(48)
                .align_x(Alignment::End),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let mut controls = column![effects].spacing(10);
        match self.effect {
            Effect::Static => {
                controls = controls.push(set("The keys", self.color, Msg::SetColor));
            }
            Effect::Breathe => {
                controls = controls
                    .push(set("Breathing", self.color, Msg::SetColor))
                    .push(set("Fading to", self.fade_to, Msg::SetFadeTo))
                    .push(period);
            }
            Effect::Cycle | Effect::Wave => {
                let round =
                    row(self.round().into_iter().map(|color| swatch(color, 22.0))).spacing(4);
                controls = controls
                    .push(
                        row![
                            round,
                            space::horizontal(),
                            small("Add the colour", Msg::AddColor),
                            small("Rainbow", Msg::Rainbow),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .push(period);
                if self.effect == Effect::Wave {
                    let origins = row(ORIGINS.map(|(origin, name)| {
                        choice(
                            theme_label(name),
                            self.origin == origin,
                            Msg::Origin(origin),
                        )
                    }))
                    .spacing(6);
                    let direction = row![
                        choice("Out".into(), !self.inward, Msg::Inward(false)),
                        choice("In".into(), self.inward, Msg::Inward(true)),
                    ]
                    .spacing(6);
                    controls = controls.push(origins).push(direction);
                }
            }
            Effect::Reactive => {
                let flash_ms = row![
                    text("Flash").width(56),
                    slider(keyboard::FLASHES_MS.clone(), self.flash_ms, Msg::FlashMs)
                        .step(100u32)
                        .on_release(Msg::Released)
                        .style(theme::level),
                    text(format!("{:.1} s", self.flash_ms as f32 / 1000.0))
                        .width(48)
                        .align_x(Alignment::End),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                controls = controls
                    .push(set("The keys", self.color, Msg::SetColor))
                    .push(set("A pressed key", self.flash, Msg::SetFlash))
                    .push(flash_ms);
            }
        }
        panel("Effect", controls)
    }
}

/// A profile's field, for TOML and Nix alike.
enum Field {
    Str(String),
    Int(u32),
    Strs(Vec<String>),
    Keys(Vec<(String, String)>),
}

/// A colour as the LEDs show it, on screen: as it is.
fn led(color: Color) -> iced::Color {
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
        .padding([4, 10])
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

/// The picture: each key in the colour it gets, the picked ones outlined.
struct Board<'a> {
    editor: &'a Editor,
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
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Msg>> {
        let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event else {
            return None;
        };
        let key = Self::key_at(bounds, cursor.position_in(bounds)?)?;
        Some(canvas::Action::publish(Msg::Toggle(key)).and_capture())
    }

    fn draw(
        &self,
        _state: &(),
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
            let picked = self.editor.selected.contains(&key.code);
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(if picked { colors.strong } else { colors.border })
                    .with_width(if picked { 3.0 } else { 1.0 }),
            );
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
        _state: &(),
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
        let _ = editor.update(Msg::Brush(Color([255, 0, 0])));
        let _ = editor.update(Msg::SelectGroup("wasd"));
        let _ = editor.update(Msg::Toggle(0x64));
        let shown = editor.update(Msg::Paint);
        assert_eq!(
            shown,
            Outcome::Preview(
                "color = \"#ffffff\"\nname = \"custom\"\n\n[keys]\n102nd = \"#ff0000\"\na = \"#ff0000\"\nd = \"#ff0000\"\ns = \"#ff0000\"\nw = \"#ff0000\"\n"
                    .into()
            )
        );

        let _ = editor.update(Msg::Effect(Effect::Wave));
        let _ = editor.update(Msg::Brush(Color([0, 0, 255])));
        let _ = editor.update(Msg::AddColor);
        let _ = editor.update(Msg::Brush(Color([255, 0, 0])));
        let _ = editor.update(Msg::AddColor);
        let _ = editor.update(Msg::Origin(Origin::Left));
        let _ = editor.update(Msg::Inward(true));
        let Outcome::Copy(nix) = editor.update(Msg::Copy) else {
            panic!("no copy");
        };
        assert_eq!(
            nix,
            "{\n  name = \"custom\";\n  effect = \"wave\";\n  colors = [\n    \"#0000ff\"\n    \"#ff0000\"\n  ];\n  period_ms = 4000;\n  origin = \"left\";\n  direction = \"in\";\n  keys = {\n    a = \"#ff0000\";\n    d = \"#ff0000\";\n    s = \"#ff0000\";\n    w = \"#ff0000\";\n    \"102nd\" = \"#ff0000\";\n  };\n}"
        );
        // Unpainted, back to the effect; picking keys shows nothing new.
        let _ = editor.update(Msg::SelectNone);
        assert_eq!(editor.update(Msg::Toggle(0x04)), Outcome::Nothing);
        let _ = editor.update(Msg::Unpaint);
        assert!(!editor.keys.contains_key(&0x04));
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
