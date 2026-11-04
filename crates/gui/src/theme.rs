//! The desktop's colours where it gives them (see [`crate::system`]): its
//! libadwaita palette, or else its accent over plain light and dark, as it
//! prefers; and over either whatever `ryukaze/theme.toml` changes. Every
//! widget on screen is styled here.

use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

use iced::{
    Border, Color, Font, Shadow, Theme, color,
    theme::{Palette, Style},
    widget::{
        button, container, overlay::menu, pick_list, progress_bar, scrollable, slider, text,
        text_input,
    },
};
use serde::Deserialize;

use crate::system;

/// What the styles draw with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub background: Color,
    pub card: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    /// A choice not taken, and under the pointer.
    pub idle: Color,
    pub hover: Color,
    /// The choice taken, a switch that's on, a filled bar.
    pub strong: Color,
    pub on_strong: Color,
    /// An empty track, and a switch that's off: its track, the track under
    /// the pointer, its knob.
    pub track: Color,
    pub track_hover: Color,
    pub knob: Color,
    pub danger: Color,
}

const LIGHT: Colors = Colors {
    background: color!(0xfafafa),
    card: color!(0xffffff),
    border: color!(0xe4e4e7),
    text: color!(0x09090b),
    muted: color!(0x71717a),
    idle: color!(0xf4f4f5),
    hover: color!(0xe4e4e7),
    strong: color!(0x3584e4),
    on_strong: color!(0xffffff),
    track: color!(0xd4d4d8),
    track_hover: color!(0xa1a1aa),
    knob: color!(0xffffff),
    danger: color!(0xdc2626),
};

const DARK: Colors = Colors {
    background: color!(0x1e1e1e),
    card: color!(0x2a2a2a),
    border: color!(0x3a3a3a),
    text: color!(0xfafafa),
    muted: color!(0xa1a1aa),
    idle: color!(0x333333),
    hover: color!(0x3d3d3d),
    strong: color!(0x78aeed),
    on_strong: color!(0x0a0a0a),
    track: color!(0x48484a),
    track_hover: color!(0x5a5a5c),
    knob: color!(0xe4e4e7),
    danger: color!(0xf87171),
};

/// A theme file's names for the slots.
const KEYS: [&str; 13] = [
    "background",
    "card",
    "border",
    "text",
    "muted",
    "idle",
    "hover",
    "strong",
    "on_strong",
    "track",
    "track_hover",
    "knob",
    "danger",
];

impl Colors {
    /// The colour a theme file names `key`.
    fn get_mut(&mut self, key: &str) -> Option<&mut Color> {
        Some(match key {
            "background" => &mut self.background,
            "card" => &mut self.card,
            "border" => &mut self.border,
            "text" => &mut self.text,
            "muted" => &mut self.muted,
            "idle" => &mut self.idle,
            "hover" => &mut self.hover,
            "strong" => &mut self.strong,
            "on_strong" => &mut self.on_strong,
            "track" => &mut self.track,
            "track_hover" => &mut self.track_hover,
            "knob" => &mut self.knob,
            "danger" => &mut self.danger,
            _ => return None,
        })
    }

    /// The slots from libadwaita's named colours: the window's and the
    /// cards' as they are, the rest the text over the card as libadwaita
    /// lays it (a button 10 % of it, a border 15 %, dim text 55 %), the
    /// accent for what's chosen. Without a window and its text, none.
    fn from_named(named: &HashMap<String, Color>) -> Option<Colors> {
        let get = |name: &str| named.get(name).copied();
        let background = opaque(get("window_bg_color")?, Color::BLACK);
        let text = opaque(get("window_fg_color")?, background);
        let card = get("card_bg_color")
            .or(get("view_bg_color"))
            .map_or(background, |card| opaque(card, background));
        let text = get("card_fg_color").map_or(text, |fg| opaque(fg, card));
        let over = |amount| mix(card, text, amount);
        let dark = luma(background) < 0.5;
        let fallback = if dark { DARK } else { LIGHT };
        let strong = get("accent_bg_color")
            .or(get("accent_color"))
            .map_or(fallback.strong, |accent| opaque(accent, card));
        let on_strong = get("accent_fg_color").map_or_else(|| on(strong), |fg| opaque(fg, strong));
        let danger = get("error_color")
            .or(get("destructive_color"))
            .map_or(fallback.danger, |danger| opaque(danger, card));
        Some(Colors {
            background,
            card,
            border: over(0.15),
            text,
            muted: over(0.55),
            idle: over(0.10),
            hover: over(0.16),
            strong,
            on_strong,
            track: over(0.20),
            track_hover: over(0.30),
            knob: over(0.85),
            danger,
        })
    }

    /// Built-in, the desktop's accent for what's chosen.
    fn with_accent(mut self, accent: Color) -> Colors {
        self.strong = accent;
        self.on_strong = on(accent);
        self
    }
}

/// `top` over `under`, as it shows.
fn opaque(top: Color, under: Color) -> Color {
    if top.a >= 1.0 {
        return top;
    }
    Color {
        a: 1.0,
        ..mix(under, Color { a: 1.0, ..top }, top.a)
    }
}

/// `amount` of the way from `from` to `to`.
fn mix(from: Color, to: Color, amount: f32) -> Color {
    let channel = |a: f32, b: f32| a + (b - a) * amount;
    Color::from_rgb(
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
    )
}

fn luma(color: Color) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}

/// Black or white, whichever reads on `background`.
fn on(background: Color) -> Color {
    if luma(background) > 0.5 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

#[derive(Debug, Default)]
struct Palettes {
    /// Light or dark whatever the desktop prefers.
    mode: Option<Mode>,
    /// A family the system has; the desktop's otherwise.
    font: Option<&'static str>,
    light: Vec<(&'static str, Color)>,
    dark: Vec<(&'static str, Color)>,
}

/// The theme file's, read once before the window opens.
static PALETTES: OnceLock<Palettes> = OnceLock::new();

fn palettes() -> &'static Palettes {
    static NONE: Palettes = Palettes {
        mode: None,
        font: None,
        light: Vec::new(),
        dark: Vec::new(),
    };
    PALETTES.get().unwrap_or(&NONE)
}

/// What the desktop gives, read again as it changes.
#[derive(Debug, Clone, Default)]
struct Desktop {
    /// Its libadwaita palette, and whether it's dark.
    palette: Option<(bool, Colors)>,
    /// Its accent, from the portal.
    accent: Option<Color>,
}

static DESKTOP: RwLock<Desktop> = RwLock::new(Desktop {
    palette: None,
    accent: None,
});

fn desktop() -> Desktop {
    DESKTOP
        .read()
        .map(|desktop| desktop.clone())
        .unwrap_or_default()
}

/// The desktop's palette read again, as at start or when it changes.
pub fn read_desktop() {
    let palette = Colors::from_named(&system::named_colors())
        .map(|colors| (luma(colors.background) < 0.5, colors));
    if let Ok(mut desktop) = DESKTOP.write() {
        desktop.palette = palette;
    }
}

/// The portal's accent, used where the desktop has no palette of its own.
pub fn set_accent(accent: Option<Color>) {
    if let Ok(mut desktop) = DESKTOP.write() {
        desktop.accent = accent;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Light,
    Dark,
}

/// `ryukaze/theme.toml`: `mode`, `font`, and colours over the desktop's by
/// name.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    mode: Option<Mode>,
    font: Option<String>,
    light: HashMap<String, String>,
    dark: HashMap<String, String>,
}

/// Reads the first `ryukaze/theme.toml` in the XDG config dirs, the user's
/// before the system's, and the desktop's palette. A broken theme file is
/// reported, and goes unused.
pub fn load() {
    read_desktop();
    let Some(path) = find(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        env::var_os("XDG_CONFIG_DIRS"),
    ) else {
        return;
    };
    let palettes = fs::read_to_string(&path)
        .map_err(|err| err.to_string())
        .and_then(|text| parse(&text));
    match palettes {
        Ok(palettes) => {
            let _ = PALETTES.set(palettes);
        }
        Err(err) => eprintln!("{}: {err}\nkeeping the desktop's colours", path.display()),
    }
}

/// As the XDG Base Directory spec has it: `$XDG_CONFIG_HOME` or
/// `~/.config`, then `$XDG_CONFIG_DIRS` or `/etc/xdg`.
fn find(
    config_home: Option<OsString>,
    home: Option<OsString>,
    config_dirs: Option<OsString>,
) -> Option<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    let user = set(config_home)
        .map(PathBuf::from)
        .or_else(|| set(home).map(|home| Path::new(&home).join(".config")));
    let system = set(config_dirs).unwrap_or_else(|| "/etc/xdg".into());
    user.into_iter()
        .chain(env::split_paths(&system))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("ryukaze/theme.toml"))
        .find(|path| path.is_file())
}

fn parse(text: &str) -> Result<Palettes, String> {
    let file: File = toml::from_str(text).map_err(|err| err.to_string())?;
    let mut palettes = Palettes {
        mode: file.mode,
        // Read once at start, and iced keeps font names for good.
        font: file.font.map(|name| &*name.leak()),
        ..Palettes::default()
    };
    for (name, table, colors) in [
        ("light", &file.light, &mut palettes.light),
        ("dark", &file.dark, &mut palettes.dark),
    ] {
        for (key, value) in table {
            let key = KEYS
                .into_iter()
                .find(|known| known == key)
                .ok_or_else(|| format!("{name}.{key}: no such colour"))?;
            let color = value
                .parse()
                .map_err(|_| format!("{name}.{key}: {value:?} isn't #rrggbb or #rrggbbaa"))?;
            colors.push((key, color));
        }
    }
    Ok(palettes)
}

/// What all text is drawn in: the theme file's, or the desktop's.
pub fn font() -> Font {
    static DESKTOP_FONT: OnceLock<Option<&'static str>> = OnceLock::new();
    let desktop = *DESKTOP_FONT.get_or_init(|| system::font_family().map(|name| &*name.leak()));
    palettes()
        .font
        .or(desktop)
        .map_or(Font::DEFAULT, Font::with_name)
}

/// Whether the window is dark: as the theme file fixes it, or as the
/// desktop's palette is, or as the desktop prefers.
pub fn is_dark(prefers: bool) -> bool {
    let palette = desktop().palette.map(|(dark, _)| dark);
    palettes()
        .mode
        .map(|mode| mode == Mode::Dark)
        .or(palette)
        .unwrap_or(prefers)
}

/// The colours for light or dark: the desktop's palette if it's that one,
/// or else the built-in with its accent; the theme file's over either.
fn resolve(dark: bool, desktop: &Desktop, file: &Palettes) -> Colors {
    let mut colors = match desktop.palette {
        Some((is_dark, colors)) if is_dark == dark => colors,
        _ => {
            let built_in = if dark { DARK } else { LIGHT };
            desktop
                .accent
                .map_or(built_in, |accent| built_in.with_accent(accent))
        }
    };
    let overrides = if dark { &file.dark } else { &file.light };
    for (key, color) in overrides {
        if let Some(slot) = colors.get_mut(key) {
            *slot = *color;
        }
    }
    colors
}

/// The colours a theme stands for: only whether it's dark matters.
pub fn colors(theme: &Theme) -> Colors {
    resolve(theme.extended_palette().is_dark, &desktop(), palettes())
}

/// The theme for the window: iced's own widgets, the scrollbars and text
/// selection among them, in the same colours as the rest.
pub fn theme(dark: bool) -> Theme {
    let colors = resolve(dark, &desktop(), palettes());
    let base = if dark { Theme::Dark } else { Theme::Light }.palette();
    Theme::custom(
        "ryukaze",
        Palette {
            background: colors.background,
            text: colors.text,
            primary: colors.strong,
            danger: colors.danger,
            ..base
        },
    )
}

/// The window behind everything, and the default text colour.
pub fn window(theme: &Theme) -> Style {
    let colors = colors(theme);
    Style {
        background_color: colors.background,
        text_color: colors.text,
    }
}

/// Secondary text: section titles, hints, the firmware version.
pub fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(colors(theme).muted),
    }
}

/// The panel behind each section.
pub fn card(theme: &Theme) -> container::Style {
    let colors = colors(theme);
    container::Style {
        background: Some(colors.card.into()),
        border: Border::default().rounded(12).width(1).color(colors.border),
        ..container::Style::default()
    }
}

/// A line about something that went wrong.
pub fn problem(theme: &Theme) -> container::Style {
    let danger = colors(theme).danger;
    container::Style {
        background: Some(Color { a: 0.1, ..danger }.into()),
        text_color: Some(danger),
        border: Border::default().rounded(8).width(1).color(danger),
        ..container::Style::default()
    }
}

/// One choice of a mode row; the chosen one is filled.
pub fn segment(chosen: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let colors = colors(theme);
        let (background, text) = match (chosen, status) {
            (true, _) => (colors.strong, colors.on_strong),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                (colors.hover, colors.text)
            }
            (false, button::Status::Disabled) => (colors.idle, colors.muted),
            (false, button::Status::Active) => (colors.idle, colors.text),
        };
        button::Style {
            background: Some(background.into()),
            text_color: text,
            border: Border::default().rounded(8),
            ..button::Style::default()
        }
    }
}

/// One choice of a row whose fills are drawn under it (see anim.rs): a
/// hovered one's own fill over them, the chosen one's text on the strong.
pub fn choice(chosen: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let colors = colors(theme);
        let (background, text) = match (chosen, status) {
            (true, _) => (None, colors.on_strong),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                (Some(colors.hover.into()), colors.text)
            }
            (false, button::Status::Disabled) => (None, colors.muted),
            (false, button::Status::Active) => (None, colors.text),
        };
        button::Style {
            background,
            text_color: text,
            border: Border::default().rounded(8),
            ..button::Style::default()
        }
    }
}

/// A pick list, closed: a choice not taken, as a segment is.
pub fn pick(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let colors = colors(theme);
    let background = match status {
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => colors.hover,
        pick_list::Status::Active => colors.idle,
    };
    pick_list::Style {
        text_color: colors.text,
        placeholder_color: colors.muted,
        handle_color: colors.muted,
        background: background.into(),
        border: Border::default().rounded(8),
    }
}

/// A pick list, open.
pub fn menu(theme: &Theme) -> menu::Style {
    let colors = colors(theme);
    menu::Style {
        background: colors.card.into(),
        border: Border::default().rounded(8).width(1).color(colors.border),
        text_color: colors.text,
        selected_text_color: colors.on_strong,
        selected_background: colors.strong.into(),
        shadow: Shadow {
            color: Color {
                a: 0.3,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
    }
}

/// The charge limit's slider: filled up to the handle.
pub fn level(theme: &Theme, status: slider::Status) -> slider::Style {
    let colors = colors(theme);
    let mut style = slider::default(theme, status);
    style.rail.backgrounds = (colors.strong.into(), colors.track.into());
    style.handle.background = colors.strong.into();
    style.handle.border_width = 2.0;
    style.handle.border_color = colors.card;
    style
}

/// A fan's duty: a thin filled bar on a track.
pub fn duty(theme: &Theme) -> progress_bar::Style {
    let colors = colors(theme);
    progress_bar::Style {
        background: colors.track.into(),
        bar: colors.strong.into(),
        border: Border::default().rounded(2),
    }
}

/// The line between a group's rows.
pub fn divider(theme: &Theme) -> iced::widget::rule::Style {
    iced::widget::rule::Style {
        color: colors(theme).border,
        radius: 0.into(),
        fill_mode: iced::widget::rule::FillMode::Full,
        snap: true,
    }
}

/// What headings are drawn in: the text's font, heavier.
pub fn heading() -> Font {
    Font {
        weight: iced::font::Weight::Semibold,
        ..font()
    }
}

/// A page in the sidebar: the one shown tinted with the accent.
pub fn nav(chosen: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let colors = colors(theme);
        let background = match (chosen, status) {
            (true, _) => Some(
                Color {
                    a: 0.18,
                    ..colors.strong
                }
                .into(),
            ),
            (false, button::Status::Hovered | button::Status::Pressed) => Some(colors.idle.into()),
            (false, _) => None,
        };
        button::Style {
            background,
            text_color: colors.text,
            border: Border::default().rounded(8),
            ..button::Style::default()
        }
    }
}

/// A tile on the overview: a card, its border lit under the pointer.
pub fn tile(theme: &Theme, status: button::Status) -> button::Style {
    let colors = colors(theme);
    let border = match status {
        button::Status::Hovered | button::Status::Pressed => colors.track_hover,
        _ => colors.border,
    };
    button::Style {
        background: Some(colors.card.into()),
        text_color: colors.text,
        border: Border::default().rounded(12).width(1).color(border),
        ..button::Style::default()
    }
}

/// A row that opens something: flat, lit under the pointer.
pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let colors = colors(theme);
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then(|| colors.idle.into()),
        text_color: colors.text,
        border: Border::default().rounded(8),
        ..button::Style::default()
    }
}

/// A tooltip: a card over the window.
pub fn tip(theme: &Theme) -> container::Style {
    let colors = colors(theme);
    container::Style {
        background: Some(colors.card.into()),
        text_color: Some(colors.text),
        border: Border::default().rounded(8).width(1).color(colors.border),
        shadow: Shadow {
            color: Color {
                a: 0.3,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..container::Style::default()
    }
}

/// The sidebar: the cards' colour, as libadwaita's sidebars are.
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(colors(theme).card.into()),
        ..container::Style::default()
    }
}

/// The accent, for a bar beside the page shown.
pub fn accent_bar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(colors(theme).strong.into()),
        border: Border::default().rounded(2),
        ..container::Style::default()
    }
}

/// A text field: the card's, its border the accent while typed in.
pub fn field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let colors = colors(theme);
    let border = match status {
        text_input::Status::Focused { .. } => colors.strong,
        text_input::Status::Hovered => colors.track_hover,
        _ => colors.border,
    };
    text_input::Style {
        background: colors.idle.into(),
        border: Border::default().rounded(8).width(1).color(border),
        icon: colors.muted,
        placeholder: colors.muted,
        value: colors.text,
        selection: Color {
            a: 0.35,
            ..colors.strong
        },
    }
}

/// A scrollbar: thin, the text's colour faintly, more under the pointer.
pub fn scroller(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let colors = colors(theme);
    let mut style = scrollable::default(theme, status);
    let scroller = match status {
        scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. } => {
            colors.track_hover
        }
        scrollable::Status::Active { .. } => colors.track,
    };
    for rail in [&mut style.vertical_rail, &mut style.horizontal_rail] {
        rail.background = None;
        rail.border = Border::default();
        rail.scroller.background = scroller.into();
        rail.scroller.border = Border::default().rounded(4);
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_go_over_the_desktops() {
        let palettes = parse(
            r##"
            mode = "dark"
            font = "Inter"

            [dark]
            strong = "#fab387"
            muted = "#cdd6f480"
            "##,
        )
        .unwrap();
        assert_eq!(palettes.mode, Some(Mode::Dark));
        assert_eq!(palettes.font, Some("Inter"));
        let dark = resolve(true, &Desktop::default(), &palettes);
        assert_eq!(dark.strong, color!(0xfab387));
        assert_eq!(dark.muted, color!(0xcdd6f4, 128.0 / 255.0));
        assert_eq!(dark.background, DARK.background);
        assert_eq!(resolve(false, &Desktop::default(), &palettes), LIGHT);
    }

    #[test]
    fn an_empty_file_changes_nothing() {
        let palettes = parse("").unwrap();
        assert_eq!(palettes.mode, None);
        assert_eq!(palettes.font, None);
        let none = Desktop::default();
        assert_eq!(
            (
                resolve(false, &none, &palettes),
                resolve(true, &none, &palettes)
            ),
            (LIGHT, DARK)
        );
    }

    #[test]
    fn rejects_what_it_doesnt_know() {
        let err = |text: &str| parse(text).unwrap_err();
        assert!(err("[dark]\naccent = \"#fab387\"\n").contains("dark.accent: no such colour"));
        assert!(err("[light]\ntext = \"black\"\n").contains("isn't #rrggbb"));
        assert!(parse("mode = \"auto\"\n").is_err());
        assert!(parse("[colors]\ntext = \"#000000\"\n").is_err());
    }

    /// The owner's palette: the window and cards as given, the accent for
    /// what's chosen, the rest between the card and the text.
    #[test]
    fn takes_the_desktops_palette() {
        let named: HashMap<String, Color> = [
            ("window_bg_color", color!(0x1a1a19)),
            ("window_fg_color", color!(0xcccac3)),
            ("card_bg_color", color!(0x2a2927)),
            ("accent_bg_color", color!(0x4f9dfc)),
            ("accent_fg_color", color!(0x1a1a19)),
            ("error_color", color!(0xff6366)),
        ]
        .into_iter()
        .map(|(name, color)| (name.to_owned(), color))
        .collect();
        let colors = Colors::from_named(&named).unwrap();
        assert_eq!(colors.background, color!(0x1a1a19));
        assert_eq!(colors.card, color!(0x2a2927));
        assert_eq!(colors.strong, color!(0x4f9dfc));
        assert_eq!(colors.on_strong, color!(0x1a1a19));
        assert_eq!(colors.danger, color!(0xff6366));
        assert!(luma(colors.card) < luma(colors.idle) && luma(colors.idle) < luma(colors.muted));
        assert!(luma(colors.muted) < luma(colors.text));

        // Dark, so it stands for dark only; light falls back to built-in.
        let desktop = Desktop {
            palette: Some((true, colors)),
            accent: Some(color!(0x3584e4)),
        };
        let none = Palettes::default();
        assert_eq!(resolve(true, &desktop, &none), colors);
        assert_eq!(resolve(false, &desktop, &none).strong, color!(0x3584e4));
        // A card colour with alpha shows over the window.
        let mut seethrough = named.clone();
        seethrough.insert("card_bg_color".into(), Color::from_rgba(1.0, 1.0, 1.0, 0.5));
        let card = Colors::from_named(&seethrough).unwrap().card;
        assert!((card.r - (0x1a as f32 / 255.0 + 1.0) / 2.0).abs() < 1e-3 && card.a == 1.0);
        assert_eq!(Colors::from_named(&HashMap::new()), None);
    }

    #[test]
    fn the_users_file_goes_before_the_systems() {
        let root = tempfile::tempdir().unwrap();
        let dir = |path: &str| {
            let dir = root.path().join(path);
            fs::create_dir_all(dir.join("ryukaze")).unwrap();
            dir
        };
        let (user, system, other) = (dir("home/.config"), dir("etc/xdg"), dir("other/xdg"));
        let dirs = env::join_paths([&other, &system]).ok();
        let home = Some(root.path().join("home").into_os_string());
        let found = || find(None, home.clone(), dirs.clone());

        assert_eq!(found(), None);
        fs::write(system.join("ryukaze/theme.toml"), "").unwrap();
        assert_eq!(found(), Some(system.join("ryukaze/theme.toml")));
        fs::write(user.join("ryukaze/theme.toml"), "").unwrap();
        assert_eq!(found(), Some(user.join("ryukaze/theme.toml")));
        // An explicit XDG_CONFIG_HOME stands in for ~/.config.
        assert_eq!(
            find(
                Some(other.clone().into_os_string()),
                home.clone(),
                dirs.clone()
            ),
            Some(system.join("ryukaze/theme.toml"))
        );
    }

    #[test]
    fn the_documented_example_parses() {
        let page = include_str!("../../../docs/usage.md");
        let example = page
            .split("```toml\n")
            .filter_map(|block| block.split_once("```").map(|(toml, _)| toml))
            .find(|toml| toml.starts_with("mode ="))
            .expect("a theme example in docs/usage.md");
        let palettes = parse(example).unwrap();
        assert_eq!(palettes.mode, Some(Mode::Dark));
        assert_ne!(resolve(true, &Desktop::default(), &palettes), DARK);
        assert!(palettes.font.is_some());
    }
}
