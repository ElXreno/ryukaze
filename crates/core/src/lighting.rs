//! What a keyboard profile asks of the lights, whichever controllers show
//! it: the keys' (see [`crate::keyboard`]) and those beside them (see
//! [`crate::lightbar`]).

use std::{fmt, str::FromStr};

use crate::steelseries::TICK_MS;

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

/// What the controller does with the keys that have no colour of their own.
/// All but `Static` are its own effects, which it runs by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Still, in the colour.
    Static,
    /// The colour fading to `fade_to`, black to breathe out, and back.
    Breathe { period_ms: u32, fade_to: Color },
    /// All of them through `colors` together, and round to the first.
    Cycle { period_ms: u32, colors: Vec<Color> },
    /// `colors` as a wave across the board.
    Wave {
        period_ms: u32,
        colors: Vec<Color>,
        wave: Wave,
    },
    /// Still in the colour, a key flashing `flash` when pressed: the
    /// controller sees the presses, the computer has no part in it.
    Reactive { flash: Color, flash_ms: u32 },
}

/// Where a wave comes from, and which way it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wave {
    pub origin: Origin,
    /// Into the origin rather than out of it.
    pub inward: bool,
    /// How wide a round of the colours lies across the keys, in percent.
    pub wavelength: u8,
}

impl Default for Wave {
    fn default() -> Self {
        Self {
            origin: Origin::Middle,
            inward: false,
            wavelength: 25,
        }
    }
}

/// A wave's origin: out of the middle it spreads both ways, from an edge
/// it runs straight across or down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Middle,
    Left,
    Right,
    Top,
    Bottom,
}

/// The colours a cycle or a wave goes through unless told others.
pub const RAINBOW: [Color; 6] = [
    Color([255, 0, 0]),
    Color([255, 255, 0]),
    Color([0, 255, 0]),
    Color([0, 255, 255]),
    Color([0, 0, 255]),
    Color([255, 0, 255]),
];

/// How long a breath or a round of the colours may take. The shortest
/// also depends on how many colours there are: see `shortest_period_ms`.
pub const PERIODS_MS: std::ops::RangeInclusive<u32> = 2000..=60000;

/// How many colours a cycle or a wave goes through.
pub const COLORS: std::ops::RangeInclusive<usize> = 2..=8;

/// How wide a wave may be, in percent of the board.
pub const WAVELENGTHS: std::ops::RangeInclusive<u8> = 1..=100;

/// How long a key's flash lasts.
pub const FLASHES_MS: std::ops::RangeInclusive<u32> = 100..=5000;

/// How many zones a profile may have, each an effect of its own beside
/// the power button's and the profile's: the controller runs effects
/// numbered up to 15 at least.
pub const ZONES: usize = 8;

/// The shortest round through `colors` colours: going from one to the
/// next, a channel moves at most 127/16 a tick, so a full swing takes 33
/// ticks of 10 ms.
pub fn shortest_period_ms(colors: usize) -> u32 {
    (colors as u32 * 33 * TICK_MS).max(*PERIODS_MS.start())
}

/// A light beside the keys, on a controller of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// The light bar: on the Titan 18 HX, the touchpad's, lit from below.
    Bar,
    /// The logo on the lid.
    Logo,
}

impl Part {
    pub const ALL: [Part; 2] = [Part::Bar, Part::Logo];

    /// As the config writes it, beside the keys' names.
    pub fn name(self) -> &'static str {
        match self {
            Part::Bar => "bar",
            Part::Logo => "logo",
        }
    }

    pub fn named(name: &str) -> Option<Part> {
        Self::ALL.into_iter().find(|part| part.name() == name)
    }
}

/// Part of the board with an effect of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub keys: Vec<u8>,
    /// The lights beside the keys it takes in too.
    pub parts: Vec<Part>,
    /// Any but `Static`: still keys go in the profile's `keys`.
    pub effect: Effect,
    /// As a profile's: breathing, or between flashes.
    pub color: Option<Color>,
}

/// A keyboard's worth of light, as a profile has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lighting {
    pub effect: Effect,
    /// The keys without one of their own: still, breathing, or between
    /// flashes; a cycle or a wave goes through colours of its own.
    pub color: Option<Color>,
    /// Keys with their own colours, still whatever the effects.
    pub keys: Vec<(Vec<u8>, Color)>,
    /// The lights beside the keys with their own colours, still whatever
    /// the effects, a later one over an earlier.
    pub parts: Vec<(Part, Color)>,
    /// Parts of the board with effects of their own, a later one over an
    /// earlier.
    pub zones: Vec<Zone>,
}

impl Lighting {
    /// The same light with no effect running: the keys without a colour of
    /// their own still in the profile's, or the first its effect goes
    /// through; a zone's keys and parts in the zone's colour, if it has one.
    pub fn still(&self) -> Lighting {
        let color = self.color.or(match &self.effect {
            Effect::Cycle { colors, .. } | Effect::Wave { colors, .. } => colors.first().copied(),
            _ => None,
        });
        let colored = || {
            self.zones
                .iter()
                .filter_map(|zone| Some((zone, zone.color?)))
        };
        let keys = colored().map(|(zone, color)| (zone.keys.clone(), color));
        let parts =
            colored().flat_map(|(zone, color)| zone.parts.iter().map(move |&part| (part, color)));
        Lighting {
            effect: Effect::Static,
            color,
            keys: keys.chain(self.keys.iter().cloned()).collect(),
            parts: parts.chain(self.parts.iter().copied()).collect(),
            zones: Vec::new(),
        }
    }

    /// What `part` shows: its own colour still, or its zone's effect, or
    /// the profile's, with the zone's number (0 for the profile's).
    pub fn part(&self, part: Part) -> PartLight<'_> {
        if let Some(&(_, color)) = self.parts.iter().rev().find(|(own, _)| *own == part) {
            return PartLight::Still(color);
        }
        let zone = self
            .zones
            .iter()
            .enumerate()
            .rev()
            .find(|(_, zone)| zone.parts.contains(&part));
        match zone {
            Some((i, zone)) => PartLight::Effect {
                zone: Some(i),
                effect: &zone.effect,
                color: zone.color,
            },
            None => PartLight::Effect {
                zone: None,
                effect: &self.effect,
                color: self.color,
            },
        }
    }
}

/// What a light beside the keys shows, as [`Lighting::part`] finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartLight<'a> {
    Still(Color),
    /// The profile's effect (no zone) or a zone's.
    Effect {
        zone: Option<usize>,
        effect: &'a Effect,
        color: Option<Color>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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

    const ORANGE: Color = Color([0xff, 0x60, 0x00]);
    const BLUE: Color = Color([0x00, 0x40, 0xff]);
    const POWER: u8 = 0x66;

    fn breathe(period_ms: u32) -> Effect {
        Effect::Breathe {
            period_ms,
            fade_to: Color::OFF,
        }
    }

    fn lighting(effect: Effect, color: Option<Color>) -> Lighting {
        Lighting {
            effect,
            color,
            keys: Vec::new(),
            parts: Vec::new(),
            zones: Vec::new(),
        }
    }

    #[test]
    fn still_keeps_the_colours_without_the_effects() {
        let wave = lighting(
            Effect::Wave {
                period_ms: 3000,
                colors: RAINBOW.to_vec(),
                wave: Wave::default(),
            },
            None,
        );
        assert_eq!(wave.still().color, Some(RAINBOW[0]));
        let zoned = Lighting {
            parts: vec![(Part::Logo, ORANGE)],
            zones: vec![Zone {
                keys: vec![POWER],
                parts: vec![Part::Bar],
                effect: breathe(3000),
                color: Some(BLUE),
            }],
            ..lighting(breathe(4000), Some(ORANGE))
        };
        let still = zoned.still();
        assert_eq!(still.effect, Effect::Static);
        assert_eq!(still.color, Some(ORANGE));
        assert!(still.zones.is_empty());
        assert_eq!(still.keys, vec![(vec![POWER], BLUE)]);
        assert_eq!(still.parts, vec![(Part::Bar, BLUE), (Part::Logo, ORANGE)]);
    }

    /// A part's own colour over its zone's effect over the profile's.
    #[test]
    fn parts_show_their_own_or_their_zones_or_the_profiles() {
        let breathing = breathe(3000);
        let zoned = Lighting {
            parts: vec![(Part::Logo, BLUE), (Part::Logo, ORANGE)],
            zones: vec![
                Zone {
                    keys: Vec::new(),
                    parts: vec![Part::Bar, Part::Logo],
                    effect: breathing.clone(),
                    color: Some(BLUE),
                },
                Zone {
                    keys: vec![POWER],
                    parts: Vec::new(),
                    effect: breathing.clone(),
                    color: None,
                },
            ],
            ..lighting(Effect::Static, Some(ORANGE))
        };
        assert_eq!(zoned.part(Part::Logo), PartLight::Still(ORANGE));
        assert_eq!(
            zoned.part(Part::Bar),
            PartLight::Effect {
                zone: Some(0),
                effect: &breathing,
                color: Some(BLUE)
            }
        );
        let plain = lighting(breathing.clone(), Some(ORANGE));
        assert_eq!(
            plain.part(Part::Bar),
            PartLight::Effect {
                zone: None,
                effect: &breathing,
                color: Some(ORANGE)
            }
        );
        assert_eq!(Part::named("bar"), Some(Part::Bar));
        assert_eq!(Part::named("logo"), Some(Part::Logo));
        assert_eq!(Part::named("power"), None);
    }
}
