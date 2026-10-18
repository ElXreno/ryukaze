//! Plain light and dark, following the desktop's preference, with whatever
//! `ryukaze/theme.toml` changes. Every widget on screen is styled here, so
//! none of iced's accent colours shows.

use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use iced::{
    Border, Color, Font, Shadow, Theme, color,
    theme::Style,
    widget::{button, container, overlay::menu, pick_list, progress_bar, slider, text, toggler},
};
use serde::Deserialize;

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
    background: color!(0xffffff),
    card: color!(0xffffff),
    border: color!(0xe4e4e7),
    text: color!(0x09090b),
    muted: color!(0x71717a),
    idle: color!(0xf4f4f5),
    hover: color!(0xe4e4e7),
    strong: color!(0x09090b),
    on_strong: color!(0xffffff),
    track: color!(0xd4d4d8),
    track_hover: color!(0xa1a1aa),
    knob: color!(0xffffff),
    danger: color!(0xdc2626),
};

const DARK: Colors = Colors {
    background: color!(0x000000),
    card: color!(0x0a0a0a),
    border: color!(0x27272a),
    text: color!(0xfafafa),
    muted: color!(0xa1a1aa),
    idle: color!(0x18181b),
    hover: color!(0x27272a),
    strong: color!(0xfafafa),
    on_strong: color!(0x09090b),
    track: color!(0x3f3f46),
    track_hover: color!(0x52525b),
    knob: color!(0xa1a1aa),
    danger: color!(0xf87171),
};

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
}

#[derive(Debug)]
struct Palettes {
    /// Light or dark whatever the desktop prefers.
    mode: Option<Mode>,
    /// A family the system has; iced's default otherwise.
    font: Option<&'static str>,
    light: Colors,
    dark: Colors,
}

const BUILT_IN: Palettes = Palettes {
    mode: None,
    font: None,
    light: LIGHT,
    dark: DARK,
};

/// The theme file's, read once before the window opens.
static PALETTES: OnceLock<Palettes> = OnceLock::new();

fn palettes() -> &'static Palettes {
    PALETTES.get().unwrap_or(&BUILT_IN)
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Light,
    Dark,
}

/// `ryukaze/theme.toml`: `mode`, `font`, and colours over the built-in ones
/// by name.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    mode: Option<Mode>,
    font: Option<String>,
    light: HashMap<String, String>,
    dark: HashMap<String, String>,
}

/// Reads the first `ryukaze/theme.toml` in the XDG config dirs, the user's
/// before the system's. A broken one is reported, and the built-in
/// palettes stay.
pub fn load() {
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
        Err(err) => eprintln!("{}: {err}\nkeeping the built-in colours", path.display()),
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
        ..BUILT_IN
    };
    for (name, table, colors) in [
        ("light", &file.light, &mut palettes.light),
        ("dark", &file.dark, &mut palettes.dark),
    ] {
        for (key, value) in table {
            let slot = colors
                .get_mut(key)
                .ok_or_else(|| format!("{name}.{key}: no such colour"))?;
            *slot = value
                .parse()
                .map_err(|_| format!("{name}.{key}: {value:?} isn't #rrggbb or #rrggbbaa"))?;
        }
    }
    Ok(palettes)
}

/// What all text is drawn in.
pub fn font() -> Font {
    palettes().font.map_or(Font::DEFAULT, Font::with_name)
}

/// Whether the window is dark: as the theme file fixes it, or as the
/// desktop prefers.
pub fn is_dark(desktop: bool) -> bool {
    palettes().mode.map_or(desktop, |mode| mode == Mode::Dark)
}

/// The app hands iced its built-in Light or Dark; only which of the two it
/// is matters here.
pub fn colors(theme: &Theme) -> &'static Colors {
    let palettes = palettes();
    if theme.extended_palette().is_dark {
        &palettes.dark
    } else {
        &palettes.light
    }
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
        shadow: Shadow::default(),
    }
}

pub fn switch(theme: &Theme, status: toggler::Status) -> toggler::Style {
    let colors = colors(theme);
    let (track, knob) = match status {
        toggler::Status::Active { is_toggled: true }
        | toggler::Status::Hovered { is_toggled: true } => (colors.strong, colors.on_strong),
        toggler::Status::Hovered { is_toggled: false } => (colors.track_hover, colors.knob),
        toggler::Status::Active { is_toggled: false } => (colors.track, colors.knob),
        toggler::Status::Disabled { .. } => (colors.idle, colors.track),
    };
    toggler::Style {
        background: track.into(),
        foreground: knob.into(),
        ..toggler::default(theme, status)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_go_over_the_built_in_ones() {
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
        assert_eq!(palettes.dark.strong, color!(0xfab387));
        assert_eq!(palettes.dark.muted, color!(0xcdd6f4, 128.0 / 255.0));
        assert_eq!(palettes.dark.background, DARK.background);
        assert_eq!(palettes.light, LIGHT);
    }

    #[test]
    fn an_empty_file_changes_nothing() {
        let palettes = parse("").unwrap();
        assert_eq!(palettes.mode, None);
        assert_eq!(palettes.font, None);
        assert_eq!((palettes.light, palettes.dark), (LIGHT, DARK));
    }

    #[test]
    fn rejects_what_it_doesnt_know() {
        let err = |text: &str| parse(text).unwrap_err();
        assert!(err("[dark]\naccent = \"#fab387\"\n").contains("dark.accent: no such colour"));
        assert!(err("[light]\ntext = \"black\"\n").contains("isn't #rrggbb"));
        assert!(parse("mode = \"auto\"\n").is_err());
        assert!(parse("[colors]\ntext = \"#000000\"\n").is_err());
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
        assert_ne!(palettes.dark, DARK);
        assert!(palettes.font.is_some());
    }
}
