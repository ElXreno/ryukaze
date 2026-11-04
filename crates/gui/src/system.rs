//! The desktop's own look, as GTK 4 applications get it: libadwaita's named
//! colours from `gtk-4.0/gtk.css` (`@define-color`), and the font from
//! `gtk-4.0/settings.ini`. Both are a convention rather than an API, so
//! only the names wanted are read, each on its own: one that doesn't parse
//! is left out, never the whole file.

use std::{collections::HashMap, env, ffi::OsString, fs, path::PathBuf};

use iced::Color;

/// The user's GTK 4 config directory.
fn gtk4_dir(config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    set(config_home)
        .map(PathBuf::from)
        .or_else(|| set(home).map(|home| PathBuf::from(home).join(".config")))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("gtk-4.0"))
}

fn read(file: &str) -> Option<String> {
    let dir = gtk4_dir(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))?;
    fs::read_to_string(dir.join(file)).ok()
}

/// The named colours in `gtk.css`, now.
pub fn named_colors() -> HashMap<String, Color> {
    read("gtk.css").map(|css| parse(&css)).unwrap_or_default()
}

/// The UI font's family in `settings.ini` (`gtk-font-name=DejaVu Sans 12`),
/// the size left off.
pub fn font_family() -> Option<String> {
    family(&read("settings.ini")?)
}

fn family(ini: &str) -> Option<String> {
    let value = ini
        .lines()
        .filter_map(|line| line.trim().strip_prefix("gtk-font-name"))
        .find_map(|rest| rest.trim_start().strip_prefix('='))?
        .trim()
        .trim_matches('"');
    // The size is the last word, if it's a number.
    let family = match value.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f32>().is_ok() => family,
        _ => value,
    };
    let family = family.trim().trim_end_matches(',');
    (!family.is_empty()).then(|| family.to_owned())
}

/// `@define-color name value;`, for the values read here.
fn parse(css: &str) -> HashMap<String, Color> {
    css.split(';')
        .filter_map(|statement| {
            let rest = statement.trim().strip_prefix("@define-color")?;
            let (name, value) = rest.trim().split_once(char::is_whitespace)?;
            Some((name.to_owned(), color(value.trim())?))
        })
        .collect()
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)` and `rgba(r, g, b, a)`
/// with channels 0 to 255, or 0 to 1 where every channel is a fraction.
fn color(value: &str) -> Option<Color> {
    if let Some(hex) = value.strip_prefix('#') {
        let hex = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => hex.to_owned(),
            _ => return None,
        };
        return format!("#{hex}").parse().ok();
    }
    let inner = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<f32> = inner
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    let (rgb, alpha) = match parts.as_slice() {
        [r, g, b] => ([*r, *g, *b], 1.0),
        [r, g, b, a] => ([*r, *g, *b], *a),
        _ => return None,
    };
    let fractions = inner.split(',').take(3).all(|part| part.contains('.'))
        && rgb.iter().all(|channel| *channel <= 1.0);
    let scale = if fractions { 1.0 } else { 255.0 };
    if rgb.iter().any(|channel| !(0.0..=scale).contains(channel)) || !(0.0..=1.0).contains(&alpha) {
        return None;
    }
    Some(Color::from_rgba(
        rgb[0] / scale,
        rgb[1] / scale,
        rgb[2] / scale,
        alpha,
    ))
}

/// The portal's accent, 0 to 1 a channel; out of range, none.
pub fn accent((r, g, b): (f64, f64, f64)) -> Option<Color> {
    [r, g, b]
        .iter()
        .all(|channel| (0.0..=1.0).contains(channel))
        .then(|| Color::from_rgb(r as f32, g as f32, b as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::color;

    /// Lines as the owner's home-manager writes them, the odd one included.
    const CSS: &str = "@define-color accent_color #4f9dfc;
@define-color accent_bg_color #4f9dfc;
@define-color accent_fg_color #1a1a19;
@define-color window_bg_color #1a1a19;
@define-color window_fg_color #cccac3;
@define-color headerbar_border_color rgba(0.164062, 0.160156, 0.152344, 0.7);
@define-color card_shade_color rgba(0, 0, 0, 0.07);
@define-color card_bg_color #2a2927;
@define-color broken nonsense;
.background { color: red; }
";

    #[test]
    fn reads_the_named_colours() {
        let named = parse(CSS);
        assert_eq!(named["accent_bg_color"], color!(0x4f9dfc));
        assert_eq!(named["window_bg_color"], color!(0x1a1a19));
        assert_eq!(
            named["card_shade_color"],
            Color::from_rgba(0.0, 0.0, 0.0, 0.07)
        );
        let border = named["headerbar_border_color"];
        assert!((border.r - 0.164_062).abs() < 1e-5 && (border.a - 0.7).abs() < 1e-5);
        assert!(!named.contains_key("broken"));
        assert_eq!(color("#fff"), Some(color!(0xffffff)));
        assert_eq!(color("rgb(255, 0, 0)"), Some(color!(0xff0000)));
        assert_eq!(color("rgba(300, 0, 0, 1)"), None);
        assert_eq!(color("mix(@a, @b, 0.5)"), None);
    }

    #[test]
    fn reads_the_font_family() {
        assert_eq!(
            family("[Settings]\ngtk-font-name=DejaVu Sans 12\n").as_deref(),
            Some("DejaVu Sans")
        );
        assert_eq!(
            family("gtk-font-name = \"Inter Display 10.5\"").as_deref(),
            Some("Inter Display")
        );
        assert_eq!(
            family("gtk-font-name=Cantarell").as_deref(),
            Some("Cantarell")
        );
        assert_eq!(family("gtk-theme-name=adw-gtk3"), None);
    }

    #[test]
    fn finds_the_gtk4_dir() {
        assert_eq!(
            gtk4_dir(None, Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/gtk-4.0"))
        );
        assert_eq!(
            gtk4_dir(Some("/x".into()), Some("/home/u".into())),
            Some(PathBuf::from("/x/gtk-4.0"))
        );
        assert_eq!(gtk4_dir(Some("rel".into()), None), None);
    }

    #[test]
    fn takes_the_portals_accent() {
        let accent = accent((0.207_843, 0.517_647, 0.894_118)).unwrap();
        assert!((accent.b - 0.894_118).abs() < 1e-5);
        assert_eq!(super::accent((1.5, 0.0, 0.0)), None);
    }
}
