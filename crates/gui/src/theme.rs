//! Plain light and dark, following the desktop's preference. Every widget
//! on screen is styled here, so none of iced's accent colours shows.

use iced::{
    Border, Color, Theme, color,
    theme::Style,
    widget::{button, container, progress_bar, slider, text, toggler},
};

/// What the styles draw with.
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

/// iced picks its built-in Light or Dark from the desktop's preference;
/// only which of the two it is matters here.
pub fn colors(theme: &Theme) -> &'static Colors {
    if theme.extended_palette().is_dark {
        &DARK
    } else {
        &LIGHT
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
