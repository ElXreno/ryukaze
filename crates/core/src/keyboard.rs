//! The keyboard's backlight: a SteelSeries KLC (see [`crate::steelseries`]),
//! which colours keys from feature reports, the keys by their HID usage.
//! Only on controllers checked by hand, as `MODELS` lists them. A report of
//! `0c 00 [entries] 00` and then (key, red, green, blue) entries colours
//! the keys it lists and leaves the others.

use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::steelseries::{self, Controller, FEATURE_LEN as REPORT_LEN, Restart, STILL};

/// How a controller keeps its tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tables {
    /// Any key in any table, each counting its keys, and `08 39` at the end.
    Fitted,
    /// A table for each region of keys, which its count picks: a key goes
    /// only in its region's, the slots left over for key 0xff.
    Regions,
}

/// What sits where the right Ctrl key would be, at 0xe4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightOfAlt {
    Copilot,
    RightCtrl,
}

/// A controller as checked on a laptop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Model {
    product: u16,
    /// The board, where the controller's product ID is shared by laptops
    /// with other keyboards.
    board: Option<&'static str>,
    /// Effects the controller runs: without them every key is still in
    /// the tables.
    effects: bool,
    /// What it shows goes to its flash as the laptop goes off or to sleep.
    saves: bool,
    tables: Tables,
    /// The colour scale MSI's software gives it at every start.
    scale: Option<[u8; 3]>,
    right_of_alt: RightOfAlt,
}

const MODELS: [Model; 2] = [
    // Alpha 17 C7VG.
    Model {
        product: 0x113a,
        board: None,
        effects: true,
        saves: true,
        tables: Tables::Fitted,
        scale: None,
        right_of_alt: RightOfAlt::Copilot,
    },
    // Titan 18 HX A2WJ: how its flash takes a save isn't known.
    Model {
        product: 0x1122,
        board: Some("MS-1824"),
        effects: true,
        saves: false,
        tables: Tables::Regions,
        scale: Some([0x89, 0xe2, 0xea]),
        right_of_alt: RightOfAlt::Copilot,
    },
];

/// The controller reads about the first hundred entries of a report.
const KEYS_PER_REPORT: usize = 96;

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

/// A controller found: its hidraw node, and how it was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Klc {
    node: PathBuf,
    model: &'static Model,
}

/// The first known controller's first interface, under `sysfs`, as a node
/// under `dev`.
pub fn find(sysfs: &Path, dev: &Path) -> Option<Klc> {
    let board = steelseries::board(sysfs);
    MODELS
        .iter()
        .filter(|model| {
            model
                .board
                .is_none_or(|name| board.as_deref() == Some(name))
        })
        .find_map(|model| {
            let node = steelseries::find(sysfs, dev, model.product)?;
            Some(Klc { node, model })
        })
}

impl Klc {
    pub fn node(&self) -> &Path {
        &self.node
    }

    /// Whether it runs effects of its own; without, a profile's are shown
    /// still.
    pub fn effects(&self) -> bool {
        self.model.effects
    }

    pub fn right_of_alt(&self) -> RightOfAlt {
        self.model.right_of_alt
    }

    /// The keys as `lighting` has them, and `over` it; without one, as they
    /// are, `over` too: with no tables of ryukaze's to keep it in, nothing
    /// would bring the numpad back or let the power button go.
    pub fn show(&self, lighting: Option<&Lighting>, over: Over) -> io::Result<()> {
        let plan = plan(self.model, lighting, over);
        for packet in plan.packets.iter().filter(|packet| packet[0] == 0x0b) {
            check_effect(packet)?;
        }
        let mut klc = Controller::open(&self.node)?;
        if let Some(scale) = self.model.scale {
            klc.scale(scale)?;
        }
        for packet in &plan.packets {
            klc.feature(packet)?;
        }
        if let Some(restart) = plan.restart {
            klc.restart(restart)?;
        }
        for report in reports(&plan.still) {
            klc.feature(&report)?;
        }
        Ok(())
    }

    /// What it shows now, saved to the controller's flash, which it shows
    /// from at power-on: `09`, which starts the effects over too, and blinks.
    /// Only where the model saves: elsewhere how its flash takes it isn't
    /// known.
    pub fn save(&self) -> io::Result<()> {
        if !self.model.saves {
            return Ok(());
        }
        Controller::open(&self.node)?.output(&[0x09])
    }

    /// Only `over` changed: its keys still at once, and in the tables, so
    /// they stay so, without the restart that would start every effect over.
    /// The power button's colour goes in its effect alone, which shows at
    /// once. Without effects, the tables drawn again.
    pub fn overlay(&self, lighting: Option<&Lighting>, over: Over) -> io::Result<()> {
        let Some(lighting) = lighting else {
            return Ok(());
        };
        let mut klc = Controller::open(&self.node)?;
        if !self.model.effects {
            for packet in table_packets(self.model, &lighting.still(), over) {
                klc.feature(&packet)?;
            }
            return klc.restart(Restart::Tables);
        }
        if over.power.is_some() {
            let power = power_packet(lighting, over);
            check_effect(&power)?;
            klc.feature(&power)?;
        }
        for packet in table_packets(self.model, lighting, over) {
            klc.feature(&packet)?;
        }
        for report in reports(&without_power(over.keys())) {
            klc.feature(&report)?;
        }
        Ok(())
    }
}

/// An effect sent only as MSI's software would send one: 1 to 16 steps, none
/// empty, their length the sum of theirs, `ff 00` at 0x8a and each step's
/// spare bytes clear. One with no steps has left a controller dark for good.
fn check_effect(packet: &[u8; REPORT_LEN]) -> io::Result<()> {
    let steps = usize::from(packet[0x96]);
    let length = |i: usize| u16::from_le_bytes([packet[2 + 8 * i + 6], packet[2 + 8 * i + 7]]);
    let total: u32 = (0..steps).map(|i| u32::from(length(i))).sum();
    let fine = packet[..2] == [0x0b, 0]
        && (1..=STEPS).contains(&steps)
        && packet[0x97] == 0
        && (0..steps)
            .all(|i| length(i) > 0 && packet[2 + 8 * i + 1] == 0 && packet[2 + 8 * i + 5] == 0)
        && total == u32::from(u16::from_le_bytes([packet[0x98], packet[0x99]]))
        && packet[0x8a..0x8c] == [0xff, 0];
    if fine {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an effect the controller might not take, not sent",
        ))
    }
}

/// Still keys but the power button, which has its effect: a still report
/// would hold it, and the restart doesn't let it go, unlike the others.
fn without_power(keys: Vec<(u8, Color)>) -> Vec<(u8, Color)> {
    keys.into_iter().filter(|&(key, _)| key != POWER).collect()
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

/// Part of the board with an effect of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub keys: Vec<u8>,
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
    /// Parts of the board with effects of their own, a later one over an
    /// earlier.
    pub zones: Vec<Zone>,
}

impl Lighting {
    /// The same light with no effect running: the keys without a colour of
    /// their own still in the profile's, or the first its effect goes
    /// through; a zone's keys in the zone's colour, if it has one.
    pub fn still(&self) -> Lighting {
        let color = self.color.or(match &self.effect {
            Effect::Cycle { colors, .. } | Effect::Wave { colors, .. } => colors.first().copied(),
            _ => None,
        });
        let zones = self
            .zones
            .iter()
            .filter_map(|zone| Some((zone.keys.clone(), zone.color?)));
        Lighting {
            effect: Effect::Static,
            color,
            keys: zones.chain(self.keys.iter().cloned()).collect(),
            zones: Vec::new(),
        }
    }
}

/// What goes to the controller: the effects' packets, then still colours.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    /// The effects, then the keys' tables, which the controller draws
    /// the keys from again whenever it redraws them, as on a brightness key.
    packets: Vec<[u8; REPORT_LEN]>,
    /// A restart after them: the effects started, or the tables drawn.
    restart: Option<Restart>,
    /// Keys still at once, over what's there.
    still: Vec<(u8, Color)>,
}

/// What goes still over a profile: the numpad dark but for NumLock, and the
/// power button in a colour of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Over {
    pub numpad_dark: bool,
    pub power: Option<Color>,
}

impl Over {
    fn keys(&self) -> Vec<(u8, Color)> {
        let mut keys = Vec::new();
        if self.numpad_dark {
            keys.extend(numpad_but_numlock().map(|key| (key, Color::OFF)));
        }
        if let Some(color) = self.power {
            keys.push((POWER, color));
        }
        keys
    }
}

/// The power button's light, in the F keys' and the numpad's block.
pub const POWER: u8 = 0x66;

/// The power button's effect, its own so its colour changes at once and
/// with the others' running on, without a still report or a refresh; the
/// profile's is 1 and the zones' come after.
const POWER_EFFECT: u8 = 0;
const PROFILE_EFFECT: u8 = 1;

/// Effect 0, the power button's: its colour of its own, `over`'s or the
/// profile's, held; or its zone's effect, or the profile's.
fn power_packet(lighting: &Lighting, over: Over) -> [u8; REPORT_LEN] {
    let own = paint(None, &lighting.keys)
        .into_iter()
        .rev()
        .find(|&(key, _)| key == POWER)
        .map(|(_, color)| color);
    if let Some(color) = over.power.or(own) {
        return effect_packet(POWER_EFFECT, &Effect::Static, Some(color));
    }
    let zone = lighting
        .zones
        .iter()
        .rev()
        .find(|zone| zone.keys.contains(&POWER));
    match zone {
        Some(zone) => effect_packet(POWER_EFFECT, &zone.effect, zone.color),
        None => effect_packet(POWER_EFFECT, &lighting.effect, lighting.color),
    }
}

fn plan(model: &Model, lighting: Option<&Lighting>, over: Over) -> Plan {
    let Some(lighting) = lighting else {
        return Plan::default();
    };
    // No effects: every key still in the tables, the power button too,
    // which the controller draws at once when asked.
    if !model.effects {
        return Plan {
            packets: table_packets(model, &lighting.still(), over),
            restart: Some(Restart::Tables),
            still: Vec::new(),
        };
    }
    // All still: in the tables for the next redraw, and still reports to
    // show it now, without a restart: no effect runs. The power button's
    // effect shows at once.
    if lighting.effect == Effect::Static && lighting.zones.is_empty() {
        let mut still = paint(lighting.color, &lighting.keys);
        for (key, color) in over.keys() {
            match still.iter_mut().rev().find(|(still, _)| *still == key) {
                Some(entry) => entry.1 = color,
                None => still.push((key, color)),
            }
        }
        let mut packets = vec![power_packet(lighting, over)];
        packets.extend(table_packets(model, lighting, over));
        return Plan {
            packets,
            restart: None,
            still: without_power(still),
        };
    }
    let mut packets = vec![
        power_packet(lighting, over),
        effect_packet(PROFILE_EFFECT, &lighting.effect, lighting.color),
    ];
    for (i, zone) in lighting.zones.iter().enumerate() {
        packets.push(effect_packet(zone_effect(i), &zone.effect, zone.color));
    }
    packets.extend(table_packets(model, lighting, over));
    Plan {
        packets,
        restart: Some(Restart::Effects),
        still: Vec::new(),
    }
}

/// The effects' time unit.
const TICK_MS: u32 = 10;

/// Wave origins in the controller's units: 0x105c across, 0x040d down.
const ACROSS: u16 = 0x105c;
const DOWN: u16 = 0x040d;

/// One step of an effect: each channel's change per tick, in 1/16, and
/// how many ticks it lasts.
type Step = ([i8; 3], u16);

/// The controller holds 16 steps an effect.
const STEPS: usize = 16;

fn ticks(ms: u32) -> u16 {
    (ms / TICK_MS).clamp(1, u16::MAX.into()) as u16
}

/// Effect `id`: a packet `0b 00`, then up to 16 steps of `[n] 00 [r] [g]
/// [b] 00 [ticks]` (n the effect's number first, then the step's), the
/// start colour at 0x84 in 1/16, the wave at 0x8c, the steps' count at
/// 0x96 and their length at 0x98, which the controller runs round and
/// round, from the start colour each time.
fn effect_packet(id: u8, effect: &Effect, color: Option<Color>) -> [u8; REPORT_LEN] {
    let color = color.unwrap_or(Color::OFF).0;
    let (start, steps, wave) = match effect {
        Effect::Breathe { period_ms, fade_to } => {
            let half = ticks(period_ms / 2);
            let mut steps = ramp(color, fade_to.0, half);
            steps.extend(ramp(fade_to.0, color, half));
            (color, steps, None)
        }
        Effect::Cycle { period_ms, colors } => (colors[0].0, round(colors, *period_ms), None),
        Effect::Wave {
            period_ms,
            colors,
            wave,
        } => (colors[0].0, round(colors, *period_ms), Some(*wave)),
        // Still, or still between flashes: one long step that changes
        // nothing.
        Effect::Static | Effect::Reactive { .. } => (color, vec![([0; 3], 100)], None),
    };
    debug_assert!(steps.len() <= STEPS);

    let mut packet = [0; REPORT_LEN];
    packet[0] = 0x0b;
    for (i, &([r, g, b], length)) in steps.iter().enumerate() {
        let at = 2 + 8 * i;
        packet[at] = if i == 0 { id } else { i as u8 };
        packet[at + 2..at + 5].copy_from_slice(&[r as u8, g as u8, b as u8]);
        packet[at + 6..at + 8].copy_from_slice(&length.to_le_bytes());
    }
    for (channel, value) in start.into_iter().enumerate() {
        let at = 0x84 + 2 * channel;
        packet[at..at + 2].copy_from_slice(&(u16::from(value) << 4).to_le_bytes());
    }
    packet[0x8a] = 0xff;
    if let Some(wave) = wave {
        let (x, y, across, down) = match wave.origin {
            Origin::Middle => (ACROSS / 2, DOWN / 2, 1, 1),
            Origin::Left => (0, DOWN / 2, 1, 0),
            Origin::Right => (ACROSS, DOWN / 2, 1, 0),
            Origin::Top => (ACROSS / 2, 0, 0, 1),
            Origin::Bottom => (ACROSS / 2, DOWN, 0, 1),
        };
        packet[0x8c..0x8e].copy_from_slice(&x.to_le_bytes());
        packet[0x8e..0x90].copy_from_slice(&y.to_le_bytes());
        packet[0x90] = across;
        packet[0x92] = down;
        // 31 to 1001, as the vendor's app had it.
        let wavelength = (31 + u32::from(wave.wavelength) * 970 / 100) as u16;
        packet[0x94..0x96].copy_from_slice(&wavelength.to_le_bytes());
        // 1 runs out of the origin, 0 into it.
        packet[0x9a] = u8::from(!wave.inward);
    }
    packet[0x96] = steps.len() as u8;
    let length: u16 = steps.iter().map(|&(_, length)| length).sum();
    packet[0x98..0x9a].copy_from_slice(&length.to_le_bytes());
    packet
}

/// Through `colors` and back to the first in `period_ms`: exact ramps if
/// they fit in the steps there are. Or else one step a colour, each aimed
/// from where the last one really ended so the error doesn't pile up, and
/// a short last one back onto the first colour, where the next round
/// starts.
fn round(colors: &[Color], period_ms: u32) -> Vec<Step> {
    let leg = ticks(period_ms / colors.len() as u32);
    let stops: Vec<[u8; 3]> = colors.iter().chain(&colors[..1]).map(|c| c.0).collect();
    let exact: Vec<Step> = stops
        .windows(2)
        .flat_map(|pair| ramp(pair[0], pair[1], leg))
        .collect();
    if exact.len() <= STEPS {
        return exact;
    }
    let start = stops[0].map(|value| i32::from(value) * 16);
    let mut at = start;
    let mut steps: Vec<Step> = stops[1..]
        .iter()
        .map(|stop| {
            let rates = std::array::from_fn(|c| {
                let change = i32::from(stop[c]) * 16 - at[c];
                (change as f32 / f32::from(leg))
                    .round()
                    .clamp(-127.0, 127.0) as i32
            });
            for c in 0..3 {
                at[c] += rates[c] * i32::from(leg);
            }
            (rates.map(|rate| rate as i8), leg)
        })
        .collect();
    let off: [i32; 3] = std::array::from_fn(|c| start[c] - at[c]);
    let length = off
        .iter()
        .map(|off| off.unsigned_abs().div_ceil(127))
        .max()
        .unwrap_or(0);
    if length > 0 {
        let rates = off.map(|off| (off as f32 / length as f32).round() as i8);
        steps.push((rates, length as u16));
    }
    steps
}

/// A straight change from one colour to another over `length` ticks,
/// exact: each channel moves at r or r + 1 sixteenths a tick, the faster
/// first, so it ends where it should.
fn ramp(from: [u8; 3], to: [u8; 3], length: u16) -> Vec<Step> {
    let length = i32::from(length);
    // Per channel: the slower rate, and the ticks at one more.
    let channels: [(i32, i32); 3] = std::array::from_fn(|c| {
        let change = (i32::from(to[c]) - i32::from(from[c])) * 16;
        let slow = change.div_euclid(length);
        (slow, change - slow * length)
    });
    let mut cuts: Vec<i32> = channels
        .iter()
        .map(|&(_, faster)| faster)
        .filter(|&faster| faster > 0)
        .chain([length])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut steps = Vec::new();
    let mut at = 0;
    for cut in cuts {
        let rates = channels.map(|(slow, faster)| (slow + i32::from(at < faster)) as i8);
        steps.push((rates, (cut - at) as u16));
        at = cut;
    }
    steps
}

/// The keys in blocks of up to 42, a table each, with the count each
/// table's header gives: the keys' regions, the power button with Enter's
/// where tables keep to them, after the numpad where they don't.
fn blocks(tables: &Tables) -> [(u8, Vec<u8>); 4] {
    let letters: Vec<u8> = (0x04..=0x27).chain(0x3a..=0x3f).collect();
    let enter = vec![0x28, 0x31, 0x32, 0x64];
    let others: Vec<u8> = (0x29..=0x30)
        .chain(0x33..=0x39)
        .chain([0x65])
        .chain(0xe0..=0xe6)
        .chain([0xf0])
        .collect();
    let right: Vec<u8> = (0x40..=0x63).collect();
    match tables {
        Tables::Fitted => [letters, enter, others, [right, vec![POWER]].concat()]
            .map(|keys| (keys.len() as u8, keys)),
        Tables::Regions => [
            (42, letters),
            (11, [enter, vec![POWER]].concat()),
            (24, others),
            (36, right),
        ],
    }
}

/// Zone `i`'s effect.
fn zone_effect(i: usize) -> u8 {
    PROFILE_EFFECT + 1 + i as u8
}

/// Every key on its effect, the profile's or its zone's, or still in a
/// colour of its own or `over`'s, a packet a block: `0e 00 [slots] 00`,
/// then 42 slots (see [`steelseries::slot`]: mode 0 follows the effect, 1
/// is still in the colour, 8 flashes on presses), as the model keeps them.
/// Without effects every key is still, the power button too, on effect 0
/// as MSI's software leaves still keys.
fn table_packets(model: &Model, lighting: &Lighting, over: Over) -> Vec<[u8; REPORT_LEN]> {
    let slot = |id: u8, effect: &Effect, color: Option<Color>, key: u8| -> [u8; 12] {
        let color = color.unwrap_or(Color::OFF).0;
        let id = if model.effects { id } else { 0 };
        match effect {
            Effect::Static => steelseries::slot(color, [0; 3], 0, id, STILL, key),
            Effect::Reactive { flash, flash_ms } => {
                steelseries::slot(color, flash.0, ticks(*flash_ms), id, 8, key)
            }
            _ => steelseries::slot(color, [0; 3], 0, id, 0, key),
        }
    };
    // Keys of their own colours, and `over`'s: still in them.
    let mut still: BTreeMap<u8, Color> = paint(None, &lighting.keys).into_iter().collect();
    still.extend(over.keys());
    blocks(&model.tables)
        .into_iter()
        .map(|(count, keys)| {
            let mut packet = [0; REPORT_LEN];
            let (slots, _) = packet[4..4 + 42 * 12].as_chunks_mut::<12>();
            if model.tables == Tables::Regions {
                for slot in slots.iter_mut() {
                    slot[11] = 0xff;
                }
            }
            for (place, &key) in slots.iter_mut().zip(&keys) {
                let zone = lighting
                    .zones
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, zone)| zone.keys.contains(&key));
                *place = match (still.get(&key), zone) {
                    // Only so it's lit at all: effect 0 alone colours it.
                    _ if key == POWER && model.effects => {
                        steelseries::slot([0; 3], [0; 3], 0, POWER_EFFECT, 0, POWER)
                    }
                    (Some(&color), _) => slot(PROFILE_EFFECT, &Effect::Static, Some(color), key),
                    (None, Some((i, zone))) => slot(zone_effect(i), &zone.effect, zone.color, key),
                    (None, None) => slot(PROFILE_EFFECT, &lighting.effect, lighting.color, key),
                };
            }
            // The controller reads as many slots as the header says.
            packet[..4].copy_from_slice(&[0x0e, 0x00, count, 0x00]);
            if model.tables == Tables::Fitted {
                packet[REPORT_LEN - 2..].copy_from_slice(&[0x08, 0x39]);
            }
            packet
        })
        .collect()
}

/// The HID usages of every key there may be: modifiers, Fn (0xf0), and
/// the rest from A to the numpad and Power.
fn keys() -> impl Iterator<Item = u8> {
    (0xe0..=0xe7).chain([0xf0]).chain(0x04..=0x66)
}

/// Each key's name, as the config writes it, and its HID usage. Checked
/// on the Alpha 17's keyboard, which has no Home, End or Pause of their
/// own (they share PgUp, PgDn and the numpad), a Copilot key where right
/// Ctrl would be (`rctrl` names the same light, for keyboards with one: see
/// [`RightOfAlt`]), and `102nd`, the `\` between AltGr and Fn.
pub fn names() -> Vec<(String, u8)> {
    let mut names: Vec<(String, u8)> = Vec::new();
    names.extend((b'a'..=b'z').map(|c| (char::from(c).to_string(), c - b'a' + 0x04)));
    names.extend((1..=9).map(|n| (n.to_string(), 0x1d + n)));
    names.push(("0".into(), 0x27));
    names.extend((1..=12).map(|n| (format!("f{n}"), 0x39 + n)));
    names.extend((1..=9).map(|n| (format!("kp{n}"), 0x58 + n)));
    names.push(("kp0".into(), 0x62));
    names.extend(
        [
            ("enter", 0x28),
            ("esc", 0x29),
            ("backspace", 0x2a),
            ("tab", 0x2b),
            ("space", 0x2c),
            ("minus", 0x2d),
            ("equal", 0x2e),
            ("lbracket", 0x2f),
            ("rbracket", 0x30),
            ("backslash", 0x31),
            ("semicolon", 0x33),
            ("apostrophe", 0x34),
            ("grave", 0x35),
            ("comma", 0x36),
            ("dot", 0x37),
            ("slash", 0x38),
            ("capslock", 0x39),
            ("prtsc", 0x46),
            ("scrolllock", 0x47),
            ("insert", 0x49),
            ("pageup", 0x4b),
            ("delete", 0x4c),
            ("pagedown", 0x4e),
            ("right", 0x4f),
            ("left", 0x50),
            ("down", 0x51),
            ("up", 0x52),
            ("numlock", 0x53),
            ("kpslash", 0x54),
            ("kpasterisk", 0x55),
            ("kpminus", 0x56),
            ("kpplus", 0x57),
            ("kpenter", 0x58),
            ("kpdot", 0x63),
            ("102nd", 0x64),
            ("power", 0x66),
            ("lctrl", 0xe0),
            ("lshift", 0xe1),
            ("lalt", 0xe2),
            ("win", 0xe3),
            ("copilot", 0xe4),
            ("rctrl", 0xe4),
            ("rshift", 0xe5),
            ("ralt", 0xe6),
            ("fn", 0xf0),
        ]
        .map(|(name, key)| (name.to_owned(), key)),
    );
    names
}

/// Named groups of keys.
fn groups() -> [(&'static str, Vec<u8>); 8] {
    [
        ("letters", (0x04..=0x1d).collect()),
        ("digits", (0x1e..=0x27).collect()),
        ("function", (0x3a..=0x45).collect()),
        ("navigation", vec![0x46, 0x47, 0x49, 0x4b, 0x4c, 0x4e]),
        ("arrows", (0x4f..=0x52).collect()),
        ("numpad", (0x53..=0x63).collect()),
        (
            "modifiers",
            vec![0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xf0],
        ),
        ("wasd", vec![0x1a, 0x04, 0x16, 0x07]),
    ]
}

/// The numpad's keys but NumLock, which stays lit to be found while the
/// others are dark.
pub fn numpad_but_numlock() -> std::ops::RangeInclusive<u8> {
    0x54..=0x63
}

/// The keys a key's or a group's name stands for.
pub fn named(name: &str) -> Option<Vec<u8>> {
    let key = names()
        .into_iter()
        .find(|(known, _)| known == name)
        .map(|(_, key)| vec![key]);
    key.or_else(|| {
        groups()
            .into_iter()
            .find(|(group, _)| *group == name)
            .map(|(_, keys)| keys)
    })
}

/// What to send: every key in `color`, if there is one, then the keys with
/// their own, the larger groups first so the smaller and single keys win.
pub fn paint(color: Option<Color>, own: &[(Vec<u8>, Color)]) -> Vec<(u8, Color)> {
    let mut painted: Vec<(u8, Color)> = color
        .map(|color| keys().map(|key| (key, color)).collect())
        .unwrap_or_default();
    let mut own: Vec<&(Vec<u8>, Color)> = own.iter().collect();
    own.sort_by_key(|(keys, _)| std::cmp::Reverse(keys.len()));
    for (keys, color) in own {
        for &key in keys {
            match painted.iter_mut().find(|(painted, _)| *painted == key) {
                Some(entry) => entry.1 = *color,
                None => painted.push((key, *color)),
            }
        }
    }
    painted
}

/// The reports for `keys`, as many to each as the controller reads.
fn reports(keys: &[(u8, Color)]) -> Vec<[u8; REPORT_LEN]> {
    keys.chunks(KEYS_PER_REPORT)
        .map(|chunk| {
            let mut report = [0; REPORT_LEN];
            report[..4].copy_from_slice(&[0x0c, 0x00, 0x66, 0x00]);
            let (entries, _) = report[4..].as_chunks_mut::<4>();
            // An entry with key 0xff is unused.
            for entry in entries.iter_mut() {
                entry[0] = 0xff;
            }
            for (entry, &(key, Color([r, g, b]))) in entries.iter_mut().zip(chunk) {
                *entry = [key, r, g, b];
            }
            report
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steelseries::testing::{board, hidraw};

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

    #[test]
    fn names_and_groups() {
        assert_eq!(named("a"), Some(vec![0x04]));
        assert_eq!(named("0"), Some(vec![0x27]));
        assert_eq!(named("f7"), Some(vec![0x40]));
        assert_eq!(named("kp9"), Some(vec![0x61]));
        assert_eq!(named("copilot"), Some(vec![0xe4]));
        assert_eq!(named("wasd"), Some(vec![0x1a, 0x04, 0x16, 0x07]));
        assert_eq!(named("numpad").unwrap().len(), 17);
        assert_eq!(named("home"), None);
        // Every name once, and every key it names one there is.
        let names = names();
        let all: Vec<u8> = keys().collect();
        for (i, (name, key)) in names.iter().enumerate() {
            assert!(names[..i].iter().all(|(other, _)| other != name), "{name}");
            assert!(all.contains(key), "{name}");
        }
        for (group, keys) in groups() {
            assert!(names.iter().all(|(name, _)| name != group), "{group}");
            assert!(keys.iter().all(|key| all.contains(key)), "{group}");
        }
    }

    #[test]
    fn single_keys_over_smaller_groups_over_larger() {
        let red = Color([255, 0, 0]);
        let white = Color([255; 3]);
        let own = [
            (named("w").unwrap(), white),
            (named("letters").unwrap(), Color::OFF),
            (named("wasd").unwrap(), red),
        ];
        let painted = paint(Some(Color([0, 0, 255])), &own);
        let at = |name: &str| {
            let key = named(name).unwrap()[0];
            painted
                .iter()
                .find(|(painted, _)| *painted == key)
                .unwrap()
                .1
        };
        assert_eq!(painted.len(), keys().count());
        assert_eq!(at("w"), white);
        assert_eq!(at("a"), red);
        assert_eq!(at("q"), Color::OFF);
        assert_eq!(at("1"), Color([0, 0, 255]));
        // Without a colour for all, only the keys named.
        assert_eq!(paint(None, &own).len(), 26);
        assert!(paint(None, &[]).is_empty());
    }

    /// How far a channel moves over the steps, in 1/16.
    fn moved(steps: &[Step], channel: usize) -> i32 {
        steps
            .iter()
            .map(|&(rates, length)| i32::from(rates[channel]) * i32::from(length))
            .sum()
    }

    /// The steps effect `id`'s packet holds, checking they're numbered:
    /// the effect's number first, then the steps'.
    fn steps(packet: &[u8; REPORT_LEN], id: u8) -> Vec<Step> {
        (0..usize::from(packet[0x96]))
            .map(|i| {
                let at = 2 + 8 * i;
                assert_eq!(packet[at], if i == 0 { id } else { i as u8 });
                let rates = [packet[at + 2], packet[at + 3], packet[at + 4]].map(|r| r as i8);
                (rates, u16::from_le_bytes([packet[at + 6], packet[at + 7]]))
            })
            .collect()
    }

    #[test]
    fn ramps_end_where_they_should() {
        for (from, to, length) in [
            ([255, 96, 0], [0, 0, 0], 100),
            ([0, 0, 0], [250, 179, 135], 37),
            ([255, 0, 0], [255, 255, 0], 33),
            ([10, 20, 30], [10, 20, 30], 50),
        ] {
            let steps = ramp(from, to, length);
            assert!(steps.len() <= 4, "{steps:?}");
            assert_eq!(steps.iter().map(|&(_, l)| l).sum::<u16>(), length);
            for channel in 0..3 {
                let change = (i32::from(to[channel]) - i32::from(from[channel])) * 16;
                assert_eq!(moved(&steps, channel), change, "{from:?} → {to:?}");
            }
        }
    }

    fn lighting(effect: Effect, color: Option<Color>) -> Lighting {
        Lighting {
            effect,
            color,
            keys: Vec::new(),
            zones: Vec::new(),
        }
    }

    const ORANGE: Color = Color([0xff, 0x60, 0x00]);
    const BLUE: Color = Color([0x00, 0x40, 0xff]);

    fn breathe(period_ms: u32) -> Effect {
        Effect::Breathe {
            period_ms,
            fade_to: Color::OFF,
        }
    }

    fn rainbow_wave(period_ms: u32, wave: Wave) -> Effect {
        Effect::Wave {
            period_ms,
            colors: RAINBOW.to_vec(),
            wave,
        }
    }

    #[test]
    fn breathes_out_and_back_in_its_period() {
        let packet = effect_packet(0, &breathe(4000), Some(ORANGE));
        assert_eq!(&packet[..2], &[0x0b, 0x00]);
        // The start colour in 1/16.
        assert_eq!(&packet[0x84..0x8a], &[0xf0, 0x0f, 0x00, 0x06, 0x00, 0x00]);
        assert_eq!(packet[0x8a], 0xff);
        // 400 ticks of 10 ms; out and back, so nowhere in the end.
        assert_eq!(u16::from_le_bytes([packet[0x98], packet[0x99]]), 400);
        let breath = steps(&packet, 0);
        assert!((0..3).all(|channel| moved(&breath, channel) == 0));
        // All keys alike: no wave.
        assert_eq!(&packet[0x8c..0x96], &[0; 10]);
        assert_eq!(packet[0x9a], 0);

        // Between two colours: out to blue at half time.
        let two = Effect::Breathe {
            period_ms: 4000,
            fade_to: BLUE,
        };
        let packet = effect_packet(0, &two, Some(ORANGE));
        let steps = steps(&packet, 0);
        let half: Vec<Step> = {
            let mut at = 0;
            steps
                .iter()
                .copied()
                .take_while(|&(_, length)| {
                    at += length;
                    at <= 200
                })
                .collect()
        };
        for channel in 0..3 {
            let change = (i32::from(BLUE.0[channel]) - i32::from(ORANGE.0[channel])) * 16;
            assert_eq!(moved(&half, channel), change);
            assert_eq!(moved(&steps, channel), 0);
        }
    }

    /// Every effect ryukazed can make passes the check before sending; one
    /// with no steps, or steps that don't add up, doesn't.
    #[test]
    fn only_whole_effects_are_sent() {
        let eight: Vec<Color> = RAINBOW.iter().copied().chain([ORANGE, BLUE]).collect();
        let effects = [
            Effect::Static,
            breathe(*PERIODS_MS.start()),
            breathe(*PERIODS_MS.end()),
            Effect::Cycle {
                period_ms: shortest_period_ms(eight.len()),
                colors: eight.clone(),
            },
            Effect::Cycle {
                period_ms: *PERIODS_MS.end(),
                colors: eight,
            },
            rainbow_wave(3000, Wave::default()),
            Effect::Reactive {
                flash: ORANGE,
                flash_ms: *FLASHES_MS.start(),
            },
        ];
        for effect in &effects {
            for color in [None, Some(ORANGE), Some(Color([255; 3]))] {
                check_effect(&effect_packet(PROFILE_EFFECT, effect, color)).unwrap();
            }
        }

        let whole = effect_packet(PROFILE_EFFECT, &breathe(4000), Some(ORANGE));
        let mut stepless = whole;
        stepless[0x96] = 0;
        stepless[0x98..0x9a].copy_from_slice(&[0, 0]);
        assert!(check_effect(&stepless).is_err());
        let mut short = whole;
        short[0x98] ^= 1;
        assert!(check_effect(&short).is_err());
        let mut empty_step = whole;
        empty_step[2 + 6..2 + 8].copy_from_slice(&[0, 0]);
        assert!(check_effect(&empty_step).is_err());
        let mut unmarked = whole;
        unmarked[0x8a] = 0;
        assert!(check_effect(&unmarked).is_err());
    }

    #[test]
    fn colours_go_round_exactly_or_near_enough() {
        let cycle = |colors: &[Color], period_ms| Effect::Cycle {
            period_ms,
            colors: colors.to_vec(),
        };
        let rainbow = effect_packet(0, &cycle(&RAINBOW, 6000), None);
        assert_eq!(rainbow[0x96], 12);
        assert_eq!(u16::from_le_bytes([rainbow[0x98], rainbow[0x99]]), 600);
        assert_eq!(&rainbow[0x84..0x8a], &[0xf0, 0x0f, 0, 0, 0, 0]);
        assert_eq!(&rainbow[0x8c..0x96], &[0; 10]);
        let steps_of = |packet: &[u8; REPORT_LEN]| steps(packet, 0);
        assert!((0..3).all(|channel| moved(&steps_of(&rainbow), channel) == 0));

        // Two colours of all three channels: exact, in up to eight steps.
        let two = effect_packet(0, &cycle(&[ORANGE, BLUE], 4000), None);
        assert!(two[0x96] <= 8);
        assert!((0..3).all(|channel| moved(&steps_of(&two), channel) == 0));

        // Eight such: too many steps to be exact, so one a colour, close.
        let eight: Vec<Color> = (0..8u8)
            .map(|i| Color([i * 31 + 7, 255 - i * 29, i * 17 + 3]))
            .collect();
        let shortest = shortest_period_ms(8);
        assert_eq!(shortest, 2640);
        let packet = effect_packet(0, &cycle(&eight, shortest), None);
        let steps = steps_of(&packet);
        assert!((8..=9).contains(&steps.len()), "{steps:?}");
        assert!(steps[..8].iter().all(|&(_, length)| length == 33));
        // Back on the start colour, but for a sixteenth or so; a long
        // round too, where rounding a colour's step errs most.
        let slow = effect_packet(0, &cycle(&eight, 60000), None);
        for steps in [steps, steps_of(&slow)] {
            assert!(steps.len() <= 16);
            for channel in 0..3 {
                let off = moved(&steps, channel);
                assert!(off.abs() <= 2, "{off}");
            }
        }
        assert_eq!(shortest_period_ms(6), 2000);
    }

    #[test]
    fn waves_from_where_they_are_told() {
        let middle = effect_packet(0, &rainbow_wave(3000, Wave::default()), None);
        assert_eq!(u16::from_le_bytes([middle[0x98], middle[0x99]]), 300);
        assert_eq!(
            &middle[0x8c..0x96],
            &[0x2e, 0x08, 0x06, 0x02, 1, 0, 1, 0, 0x11, 0x01]
        );
        assert_eq!(middle[0x9a], 1);

        // From the left edge into it, straight across, as wide as can be.
        let left = Wave {
            origin: Origin::Left,
            inward: true,
            wavelength: 100,
        };
        let packet = effect_packet(0, &rainbow_wave(3000, left), None);
        assert_eq!(
            &packet[0x8c..0x96],
            &[0, 0, 0x06, 0x02, 1, 0, 0, 0, 0xe9, 0x03]
        );
        assert_eq!(packet[0x9a], 0);
        let bottom = Wave {
            origin: Origin::Bottom,
            ..Wave::default()
        };
        let packet = effect_packet(0, &rainbow_wave(3000, bottom), None);
        assert_eq!(&packet[0x8c..0x94], &[0x2e, 0x08, 0x0d, 0x04, 0, 0, 1, 0]);
    }

    #[test]
    fn keys_follow_the_effect_or_flash() {
        let flash = Color([255; 3]);
        let reactive = Effect::Reactive {
            flash,
            flash_ms: 500,
        };
        let packets = table_packets(ALPHA, &lighting(reactive, Some(ORANGE)), Over::default());
        assert_eq!(packets.len(), 4);
        let alphanum = &packets[0];
        assert_eq!(&alphanum[..4], &[0x0e, 0x00, 0x2a, 0x00]);
        // A: orange, white for 50 ticks, effect 1, flashing.
        assert_eq!(
            &alphanum[4..16],
            &[0xff, 0x60, 0, 255, 255, 255, 50, 0, 1, 8, 0, 0x04]
        );
        assert_eq!(&alphanum[REPORT_LEN - 2..], &[0x08, 0x39]);

        let waving = table_packets(
            ALPHA,
            &lighting(rainbow_wave(3000, Wave::default()), None),
            Over::default(),
        );
        assert_eq!(&waving[0][4..16], &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0x04]);
        // Every key there may be, once.
        for tables in [Tables::Fitted, Tables::Regions] {
            let mut keys: Vec<u8> = blocks(&tables)
                .into_iter()
                .flat_map(|(_, keys)| keys)
                .collect();
            keys.sort_unstable();
            let mut all: Vec<u8> = super::keys().filter(|&key| key != 0xe7).collect();
            all.sort_unstable();
            assert_eq!(keys, all, "{tables:?}");
        }
    }

    #[test]
    fn zones_run_effects_of_their_own() {
        // Still orange, the numpad breathing blue, its 5 waving over that.
        let breathing = Zone {
            keys: named("numpad").unwrap(),
            effect: breathe(3000),
            color: Some(BLUE),
        };
        let waving = Zone {
            keys: named("kp5").unwrap(),
            effect: rainbow_wave(3000, Wave::default()),
            color: None,
        };
        let split = Lighting {
            zones: vec![breathing, waving],
            ..lighting(Effect::Static, Some(ORANGE))
        };
        let plan = plan(ALPHA, Some(&split), Over::default());
        assert_eq!(plan.restart, Some(Restart::Effects));
        let packets = plan.packets;
        // Four effects, the power button's, the profile's and the zones',
        // numbered in turn, then the four tables.
        assert_eq!(packets.len(), 4 + 4);
        for (id, packet) in packets[..4].iter().enumerate() {
            assert_eq!(packet[0], 0x0b);
            assert_eq!(packet[2], id as u8);
        }
        // Still orange: one step that holds it.
        assert_eq!(packets[1][0x96], 1);
        assert_eq!(&steps(&packets[1], 1), &[([0; 3], 100)]);
        let numpad = &packets[4 + 3];
        // As many slots as it has keys: F7 on, the numpad and Power.
        assert_eq!(numpad[2], 37);
        let (slots, _) = numpad[4..4 + 42 * 12].as_chunks::<12>();
        let effect_of = |key: u8| slots.iter().find(|slot| slot[11] == key).unwrap()[8];
        // Still in the tables: mode 1.
        assert_eq!(slots.iter().find(|slot| slot[11] == 0x40).unwrap()[9], 1);
        assert_eq!(effect_of(0x59), 2, "numpad 1");
        assert_eq!(effect_of(0x5d), 3, "numpad 5");
        assert_eq!(effect_of(POWER), 0, "Power");
        assert!(plan.still.is_empty());
    }

    /// A key's slot in the tables: its mode and colour.
    fn slot_of(packets: &[[u8; REPORT_LEN]], key: u8) -> (u8, [u8; 3]) {
        packets
            .iter()
            .filter(|packet| packet[0] == 0x0e)
            .flat_map(|packet| packet[4..4 + 42 * 12].as_chunks::<12>().0.to_vec())
            .find(|slot| slot[11] == key)
            .map(|slot| (slot[9], [slot[0], slot[1], slot[2]]))
            .unwrap()
    }

    /// What's still goes in the tables too, which the controller draws the
    /// keys from again on a brightness key.
    #[test]
    fn keeps_still_keys_in_the_tables() {
        let red = Color([255, 0, 0]);
        let wasd = vec![(named("wasd").unwrap(), red)];

        // All still: the tables and still reports, no refresh.
        let still = plan(
            ALPHA,
            Some(&lighting(Effect::Static, Some(ORANGE))),
            Over::default(),
        );
        assert_eq!(still.restart, None);
        assert_eq!(still.packets.len(), 1 + 4);
        assert_eq!(slot_of(&still.packets, 0x04), (1, ORANGE.0));
        // All but the power button, which its effect holds.
        assert_eq!(still.still.len(), keys().count() - 1);
        assert!(!still.still.iter().any(|&(key, _)| key == POWER));

        // A wave, WASD still red in the tables over it.
        let wave = Lighting {
            keys: wasd,
            ..lighting(rainbow_wave(3000, Wave::default()), None)
        };
        let waving = plan(ALPHA, Some(&wave), Over::default());
        assert_eq!(waving.packets.len(), 2 + 4);
        assert!(waving.restart == Some(Restart::Effects) && waving.still.is_empty());
        assert_eq!(slot_of(&waving.packets, 0x1a), (1, red.0));
        assert_eq!(slot_of(&waving.packets, 0x05).0, 0);

        // NumLock off, still in the tables; at once too where all is still.
        let over = Over {
            numpad_dark: true,
            power: Some(red),
        };
        let dark = plan(ALPHA, Some(&wave), over);
        assert_eq!(slot_of(&dark.packets, 0x5d), (1, [0; 3]));
        assert_eq!(slot_of(&dark.packets, 0x53).0, 0, "NumLock");
        let still_dark = plan(ALPHA, Some(&lighting(Effect::Static, Some(ORANGE))), over);
        assert!(still_dark.still.contains(&(0x5d, Color::OFF)));
        assert!(!still_dark.still.iter().any(|&(key, _)| key == POWER));
        // Without a profile, nothing.
        assert_eq!(plan(ALPHA, None, over), Plan::default());
    }

    /// Effect 0, the power button's, holding `color`.
    fn holds(packet: &[u8; REPORT_LEN], color: Color) -> bool {
        packet[..3] == [0x0b, 0, POWER_EFFECT]
            && steps(packet, POWER_EFFECT) == [([0; 3], 100)]
            && packet[0x84..0x8a]
                .chunks(2)
                .map(|channel| (u16::from_le_bytes([channel[0], channel[1]]) >> 4) as u8)
                .eq(color.0)
    }

    /// The same effect, but for its number.
    fn same_but_number(a: &[u8; REPORT_LEN], b: &[u8; REPORT_LEN]) -> bool {
        a[..2] == b[..2] && a[3..] == b[3..]
    }

    /// The controller lights the power button by effect 0 alone, whatever
    /// its slot says, and shows a change to it at once.
    #[test]
    fn lights_the_power_button_by_effect_0() {
        let red = Color([255, 0, 0]);
        let green = Color([0, 255, 0]);
        let none = Over::default();
        let power = Over {
            power: Some(red),
            ..none
        };
        let packets = |lighting: &Lighting, over| plan(ALPHA, Some(lighting), over).packets;

        // Still: the profile's colour, or the button's own, held.
        let still = lighting(Effect::Static, Some(ORANGE));
        assert!(holds(&packets(&still, none)[0], ORANGE));
        assert!(holds(&packets(&still, power)[0], red));
        assert_eq!(slot_of(&packets(&still, power), POWER).0, 0);

        // Breathing: the profile's effect, as effect 0 and 1 alike.
        let breathing = lighting(breathe(4000), Some(ORANGE));
        let effects = packets(&breathing, none);
        assert!(same_but_number(&effects[0], &effects[1]));
        assert!(holds(&packets(&breathing, power)[0], red));

        // Its own colour over a wave, and `over`'s over that.
        let own = Lighting {
            keys: vec![(vec![POWER], green)],
            ..lighting(rainbow_wave(3000, Wave::default()), None)
        };
        assert!(holds(&packets(&own, none)[0], green));
        assert!(holds(&packets(&own, power)[0], red));

        // In a zone, the zone's effect.
        let zoned = Lighting {
            zones: vec![Zone {
                keys: vec![POWER],
                effect: breathe(3000),
                color: Some(BLUE),
            }],
            ..lighting(Effect::Static, Some(ORANGE))
        };
        let effects = packets(&zoned, none);
        assert!(same_but_number(&effects[0], &effects[2]));
    }

    #[test]
    fn every_key_once_in_reports_it_reads() {
        let reports = reports(&paint(Some(Color([1, 2, 3])), &[]));
        assert_eq!(reports.len(), 2);
        let mut seen = Vec::new();
        for report in &reports {
            assert_eq!(&report[..4], &[0x0c, 0x00, 0x66, 0x00]);
            let (entries, _) = report[4..].as_chunks::<4>();
            let used: Vec<&[u8; 4]> = entries.iter().filter(|entry| entry[0] != 0xff).collect();
            assert!(used.len() <= KEYS_PER_REPORT);
            assert!(used.iter().all(|entry| entry[1..] == [1, 2, 3]));
            seen.extend(used.iter().map(|entry| entry[0]));
        }
        assert_eq!(seen, keys().collect::<Vec<_>>());
        // Numpad 9, 0 and ., and the key by right Alt, which a single
        // report left alone on the Alpha.
        for key in [0x61, 0x62, 0x63, 0x64] {
            assert!(seen.contains(&key));
        }
    }

    const ALPHA: &Model = &MODELS[0];
    const TITAN: &Model = &MODELS[1];

    #[test]
    fn finds_the_first_interface_of_one_it_knows() {
        let dev = Path::new("/dev");
        let sysfs = tempfile::tempdir().unwrap();
        hidraw(sysfs.path(), "hidraw0", "0003:0000046D:0000C52B", "00");
        hidraw(sysfs.path(), "hidraw3", "0003:00001038:0000113A", "00");
        hidraw(sysfs.path(), "hidraw4", "0003:00001038:0000113A", "01");
        let klc = find(sysfs.path(), dev).unwrap();
        assert_eq!(klc.node(), Path::new("/dev/hidraw3"));
        assert!(klc.effects());

        // 1122 is on many laptops: only on the board it was checked on.
        let titan = tempfile::tempdir().unwrap();
        hidraw(titan.path(), "hidraw0", "0003:00001038:00001122", "00");
        hidraw(titan.path(), "hidraw2", "0003:00001038:00001169", "00");
        assert_eq!(find(titan.path(), dev), None);
        board(titan.path(), "MS-1824");
        let klc = find(titan.path(), dev).unwrap();
        assert_eq!(klc.node(), Path::new("/dev/hidraw0"));
        assert!(klc.effects() && !klc.model.saves);
        assert_eq!(klc.right_of_alt(), RightOfAlt::Copilot);
        assert_eq!(find(Path::new("/nonexistent"), dev), None);
    }

    /// The slots of a table, but those left over.
    fn used(packet: &[u8; REPORT_LEN]) -> Vec<[u8; 12]> {
        let count = usize::from(packet[2]);
        packet[4..4 + count * 12]
            .as_chunks::<12>()
            .0
            .iter()
            .filter(|slot| slot[11] != 0xff)
            .copied()
            .collect()
    }

    /// The Titan's tables, but on a controller without effects.
    const STILL_ONLY: &Model = &Model {
        effects: false,
        ..MODELS[1]
    };

    /// Without effects: every key still, in its region's table, as found
    /// on the Titan (a key in another region's table never shows), the
    /// tables drawn at once, and no effect packet.
    #[test]
    fn without_effects_every_key_is_still_in_its_regions_table() {
        let red = Color([255, 0, 0]);
        let wave = Lighting {
            keys: vec![(named("wasd").unwrap(), red)],
            ..lighting(rainbow_wave(3000, Wave::default()), None)
        };
        let over = Over {
            numpad_dark: true,
            power: Some(BLUE),
        };
        let plan = plan(STILL_ONLY, Some(&wave), over);
        assert_eq!(plan.restart, Some(Restart::Tables));
        assert!(plan.still.is_empty());
        let regions: Vec<(u8, Vec<u8>)> = vec![
            (42, (0x04..=0x27).chain(0x3a..=0x3f).collect()),
            (11, vec![0x28, 0x31, 0x32, 0x64, POWER]),
            (
                24,
                (0x29..=0x30)
                    .chain(0x33..=0x39)
                    .chain([0x65])
                    .chain(0xe0..=0xe6)
                    .chain([0xf0])
                    .collect(),
            ),
            (36, (0x40..=0x63).collect()),
        ];
        assert_eq!(plan.packets.len(), regions.len());
        for (packet, (count, keys)) in plan.packets.iter().zip(&regions) {
            assert_eq!(&packet[..4], &[0x0e, 0x00, *count, 0x00]);
            let used = used(packet);
            assert_eq!(used.iter().map(|slot| slot[11]).collect::<Vec<_>>(), *keys);
            assert!(used.iter().all(|slot| slot[9] == STILL && slot[8] == 0));
            let (slots, _) = packet[4..4 + 42 * 12].as_chunks::<12>();
            assert!(
                slots[keys.len()..]
                    .iter()
                    .all(|slot| slot[..11] == [0; 11] && slot[11] == 0xff)
            );
            assert!(packet[4 + 42 * 12..].iter().all(|&byte| byte == 0));
        }
        // The wave's first colour where a key has none of its own.
        assert_eq!(slot_of(&plan.packets, 0x05), (STILL, RAINBOW[0].0));
        assert_eq!(slot_of(&plan.packets, 0x1a), (STILL, red.0));
        assert_eq!(slot_of(&plan.packets, 0x5d), (STILL, [0; 3]));
        assert_eq!(
            slot_of(&plan.packets, 0x53),
            (STILL, RAINBOW[0].0),
            "NumLock"
        );
        assert_eq!(slot_of(&plan.packets, POWER), (STILL, BLUE.0));
    }

    /// The Titan runs effects as the Alpha does, its keys in their regions'
    /// tables: the power button on effect 0 in Enter's, and the effects
    /// started over after them.
    #[test]
    fn titan_runs_effects_from_its_regions_tables() {
        let red = Color([255, 0, 0]);
        let wave = Lighting {
            keys: vec![(named("wasd").unwrap(), red)],
            ..lighting(rainbow_wave(3000, Wave::default()), None)
        };
        let plan = plan(TITAN, Some(&wave), Over::default());
        assert_eq!(plan.restart, Some(Restart::Effects));
        assert!(plan.still.is_empty());
        let (effects, tables) = plan.packets.split_at(2);
        assert!(effects.iter().all(|packet| check_effect(packet).is_ok()));
        assert_eq!(effects[0][2], POWER_EFFECT);
        assert_eq!(effects[1][2], PROFILE_EFFECT);
        let counts: Vec<u8> = tables.iter().map(|table| table[2]).collect();
        assert_eq!(counts, [42, 11, 24, 36]);
        assert!(
            used(&tables[1])
                .iter()
                .any(|slot| slot[11] == POWER && slot[8] == POWER_EFFECT && slot[9] == 0)
        );
        assert_eq!(slot_of(tables, 0x05), (0, [0; 3]));
        assert_eq!(slot_of(tables, 0x1a), (STILL, red.0));
        assert!(tables.iter().all(|table| table[REPORT_LEN - 2..] == [0, 0]));
    }

    #[test]
    fn still_keeps_the_colours_without_the_effects() {
        let wave = lighting(rainbow_wave(3000, Wave::default()), None);
        assert_eq!(wave.still().color, Some(RAINBOW[0]));
        let zoned = Lighting {
            zones: vec![Zone {
                keys: vec![POWER],
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
    }
}
