//! The keyboard's backlight: a SteelSeries KLC on USB, which colours keys
//! from a HID feature report on its first interface. Only on controllers
//! checked by hand: 1038:113a (Alpha 17 C7VG). There one report of
//! `0c 00 66 00` and then (key, red, green, blue) entries, the keys by
//! their HID usage, colours the keys it lists and leaves the others.

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    str::FromStr,
};

use rustix::ioctl::{Opcode, Updater, ioctl, opcode};

/// (vendor, product) of the controllers checked.
const KNOWN: [(u16, u16); 1] = [(0x1038, 0x113a)];

const REPORT_LEN: usize = 524;

/// The controller reads about the first hundred entries of a report.
const KEYS_PER_REPORT: usize = 96;

/// HIDIOCSFEATURE: the report ID (0, none) and then the report.
const SET_FEATURE: Opcode = opcode::read_write::<[u8; REPORT_LEN + 1]>(b'H', 0x06);

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

/// A controller found: its hidraw node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Klc {
    node: PathBuf,
}

/// The first known controller's first interface, under `sysfs`, as a node
/// under `dev`.
pub fn find(sysfs: &Path, dev: &Path) -> Option<Klc> {
    let ids: Vec<String> = KNOWN
        .iter()
        .map(|(vendor, product)| format!("HID_ID=0003:{vendor:08X}:{product:08X}"))
        .collect();
    let mut nodes: Vec<PathBuf> = fs::read_dir(sysfs.join("class/hidraw"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    nodes.sort();
    nodes.into_iter().find_map(|node| {
        let device = node.join("device");
        let uevent = fs::read_to_string(device.join("uevent")).ok()?;
        // The HID device's parent is the USB interface.
        let interface = fs::read_to_string(device.join("../bInterfaceNumber")).ok()?;
        let known = uevent.lines().any(|line| ids.iter().any(|id| id == line));
        let name = node.file_name()?;
        (known && interface.trim() == "00").then(|| Klc {
            node: dev.join(name),
        })
    })
}

impl Klc {
    pub fn node(&self) -> &Path {
        &self.node
    }

    /// The numpad dark but for NumLock, a still report over whatever runs:
    /// an effect goes on undisturbed.
    pub fn darken_numpad(&self) -> io::Result<()> {
        let node = OpenOptions::new().read(true).write(true).open(&self.node)?;
        let dark: Vec<(u8, Color)> = numpad_but_numlock().map(|key| (key, Color::OFF)).collect();
        for report in reports(&dark) {
            set_feature(&node, &report)?;
        }
        Ok(())
    }

    /// The keys as `lighting` has them, or as they are without one; with
    /// `numpad_dark`, the numpad dark but for NumLock.
    pub fn show(&self, lighting: Option<&Lighting>, numpad_dark: bool) -> io::Result<()> {
        let mut node = OpenOptions::new().read(true).write(true).open(&self.node)?;
        let plan = plan(lighting, numpad_dark);
        if let Some(packets) = &plan.effect {
            for packet in packets {
                set_feature(&node, packet)?;
            }
            // The refresh, an output report: report ID 0 and 64 bytes.
            let mut refresh = [0; 65];
            refresh[1] = 0x09;
            patiently(|| node.write_all(&refresh))?;
        }
        for report in reports(&plan.still) {
            set_feature(&node, &report)?;
        }
        Ok(())
    }
}

fn set_feature(node: &File, report: &[u8; REPORT_LEN]) -> io::Result<()> {
    let mut data = [0; REPORT_LEN + 1];
    data[1..].copy_from_slice(report);
    patiently(|| {
        // SAFETY: HIDIOCSFEATURE reads the report from `data`, which is as
        // long as its opcode says, and writes nothing past it.
        unsafe { ioctl(node, Updater::<SET_FEATURE, _>::new(&mut data)) }?;
        Ok(())
    })
}

/// While the controller sets up an effect, a wave more slowly than the
/// others, it stalls what comes next: EPIPE. A little later it takes it.
fn patiently(mut send: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    const TRIES: u32 = 10;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(20);
    for _ in 1..TRIES {
        match send() {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => std::thread::sleep(PAUSE),
            done => return done,
        }
    }
    send()
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

/// How many zones a profile may have: the controller holds nine effects,
/// and the profile's own is one.
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
    /// flashes; with a cycle or a wave, the power button's.
    pub color: Option<Color>,
    /// Keys with their own colours, still whatever the effects.
    pub keys: Vec<(Vec<u8>, Color)>,
    /// Parts of the board with effects of their own, a later one over an
    /// earlier.
    pub zones: Vec<Zone>,
}

/// What goes to the controller: the effects' packets, then still colours.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    effect: Option<Vec<[u8; REPORT_LEN]>>,
    still: Vec<(u8, Color)>,
}

/// The power button's light, which the effects pass by.
const POWER: u8 = 0x66;

fn plan(lighting: Option<&Lighting>, numpad_dark: bool) -> Plan {
    let mut plan = match lighting {
        None => Plan::default(),
        Some(lighting) if lighting.effect == Effect::Static && lighting.zones.is_empty() => Plan {
            effect: None,
            still: paint(lighting.color, &lighting.keys),
        },
        // Still with zones: the still colour goes in as an effect that
        // holds it, so the zones can run beside it.
        Some(lighting) => {
            let mut packets = vec![effect_packet(0, &lighting.effect, lighting.color)];
            for (i, zone) in lighting.zones.iter().enumerate() {
                packets.push(effect_packet(i as u8 + 1, &zone.effect, zone.color));
            }
            packets.extend(region_packets(lighting));
            let mut still = paint(None, &lighting.keys);
            if let Some(color) = lighting.color
                && !still.iter().any(|&(key, _)| key == POWER)
            {
                still.push((POWER, color));
            }
            Plan {
                effect: Some(packets),
                still,
            }
        }
    };
    if numpad_dark {
        for key in numpad_but_numlock() {
            match plan.still.iter_mut().find(|(still, _)| *still == key) {
                Some(entry) => entry.1 = Color::OFF,
                None => plan.still.push((key, Color::OFF)),
            }
        }
    }
    plan
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

/// The old protocol's regions, with the keys each holds; Power is in none.
fn regions() -> [(u8, Vec<u8>); 4] {
    [
        (0x2a, (0x04..=0x27).chain(0x3a..=0x3f).collect()),
        (0x0b, vec![0x28, 0x31, 0x32, 0x64]),
        (
            0x18,
            (0x29..=0x30)
                .chain(0x33..=0x39)
                .chain([0x65])
                .chain(0xe0..=0xe6)
                .chain([0xf0])
                .collect(),
        ),
        (0x24, (0x40..=0x63).collect()),
    ]
}

/// Every key on its effect, the profile's (0) or its zone's, a packet a
/// region: `0e 00 [region] 00`, then 42 slots of `[colour] [flash colour]
/// [flash ticks] [effect] [mode] 00 [key]` (mode 0 follows the effect, 8
/// flashes on presses), and `08 39` at the end.
fn region_packets(lighting: &Lighting) -> Vec<[u8; REPORT_LEN]> {
    let slot = |id: u8, effect: &Effect, color: Option<Color>, key: u8| -> [u8; 12] {
        let [r, g, b] = color.unwrap_or(Color::OFF).0;
        let (mode, [fr, fg, fb], flash_ticks) = match effect {
            Effect::Reactive { flash, flash_ms } => (8, flash.0, ticks(*flash_ms)),
            _ => (0, [0; 3], 0),
        };
        let [ticks_low, ticks_high] = flash_ticks.to_le_bytes();
        [r, g, b, fr, fg, fb, ticks_low, ticks_high, id, mode, 0, key]
    };
    regions()
        .into_iter()
        .map(|(region, keys)| {
            let mut packet = [0; REPORT_LEN];
            packet[..4].copy_from_slice(&[0x0e, 0x00, region, 0x00]);
            let (slots, _) = packet[4..4 + 42 * 12].as_chunks_mut::<12>();
            for (place, key) in slots.iter_mut().zip(keys) {
                let zone = lighting
                    .zones
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, zone)| zone.keys.contains(&key));
                *place = match zone {
                    Some((i, zone)) => slot(i as u8 + 1, &zone.effect, zone.color, key),
                    None => slot(0, &lighting.effect, lighting.color, key),
                };
            }
            packet[REPORT_LEN - 2..].copy_from_slice(&[0x08, 0x39]);
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
/// Ctrl would be, and `102nd`, the `\` between AltGr and Fn.
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
    use std::os::unix::fs::symlink;

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
    fn waits_out_a_busy_controller() {
        let mut stalls = 3;
        let mut tries = 0;
        let sent = patiently(|| {
            tries += 1;
            if stalls > 0 {
                stalls -= 1;
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            Ok(())
        });
        assert!(sent.is_ok());
        assert_eq!(tries, 4);
        // Gone for good, or never coming back: said so.
        let mut tries = 0;
        let stalled = patiently(|| {
            tries += 1;
            Err(io::ErrorKind::BrokenPipe.into())
        });
        assert_eq!(stalled.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(tries, 10);
        let mut tries = 0;
        let gone = patiently(|| {
            tries += 1;
            Err(io::ErrorKind::NotFound.into())
        });
        assert!(gone.is_err());
        assert_eq!(tries, 1);
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
        let packets = region_packets(&lighting(reactive, Some(ORANGE)));
        assert_eq!(packets.len(), 4);
        let alphanum = &packets[0];
        assert_eq!(&alphanum[..4], &[0x0e, 0x00, 0x2a, 0x00]);
        // A: orange, white for 50 ticks, effect 0, flashing.
        assert_eq!(
            &alphanum[4..16],
            &[0xff, 0x60, 0, 255, 255, 255, 50, 0, 0, 8, 0, 0x04]
        );
        assert_eq!(&alphanum[REPORT_LEN - 2..], &[0x08, 0x39]);

        let waving = region_packets(&lighting(rainbow_wave(3000, Wave::default()), None));
        assert_eq!(&waving[0][4..16], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x04]);
        // Every key there may be but Power, once.
        let mut keys: Vec<u8> = regions().into_iter().flat_map(|(_, keys)| keys).collect();
        keys.sort_unstable();
        let mut all: Vec<u8> = super::keys()
            .filter(|&key| key != POWER && key != 0xe7)
            .collect();
        all.sort_unstable();
        assert_eq!(keys, all);
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
        let plan = plan(Some(&split), false);
        let packets = plan.effect.unwrap();
        // Three effects, numbered in turn, then the four regions.
        assert_eq!(packets.len(), 3 + 4);
        for (id, packet) in packets[..3].iter().enumerate() {
            assert_eq!(packet[0], 0x0b);
            assert_eq!(packet[2], id as u8);
        }
        // Still orange: one step that holds it.
        assert_eq!(packets[0][0x96], 1);
        assert_eq!(&steps(&packets[0], 0), &[([0; 3], 100)]);
        let numpad = &packets[3 + 3];
        assert_eq!(numpad[2], 0x24);
        let (slots, _) = numpad[4..4 + 42 * 12].as_chunks::<12>();
        let effect_of = |key: u8| slots.iter().find(|slot| slot[11] == key).unwrap()[8];
        assert_eq!(effect_of(0x40), 0, "F7");
        assert_eq!(effect_of(0x59), 1, "numpad 1");
        assert_eq!(effect_of(0x5d), 2, "numpad 5");
        assert_eq!(plan.still, [(POWER, ORANGE)]);
    }

    #[test]
    fn plans_still_keys_over_effects() {
        let red = Color([255, 0, 0]);
        let wasd = vec![(named("wasd").unwrap(), red)];

        let still = plan(Some(&lighting(Effect::Static, Some(ORANGE))), false);
        assert_eq!(still.effect, None);
        assert_eq!(still.still.len(), keys().count());

        // A wave, WASD still over it.
        let wave = Lighting {
            keys: wasd.clone(),
            ..lighting(rainbow_wave(3000, Wave::default()), None)
        };
        let waving = plan(Some(&wave), false);
        assert_eq!(waving.effect.as_ref().map(Vec::len), Some(5));
        assert_eq!(waving.still.len(), 4);
        // Breathing: Power, which no effect reaches, in the colour.
        let breathing = plan(Some(&lighting(breathe(4000), Some(ORANGE))), false);
        assert_eq!(breathing.still, [(POWER, ORANGE)]);

        // NumLock off: the numpad dark over all of it, but NumLock.
        let dark = plan(Some(&wave), true);
        assert_eq!(dark.still.len(), 4 + 16);
        assert!(dark.still.contains(&(0x5d, Color::OFF)));
        assert!(!dark.still.iter().any(|&(key, _)| key == 0x53));
        assert_eq!(plan(None, true).still.len(), 16);
        assert_eq!(plan(None, false), Plan::default());
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

    /// `class/hidraw/<name>` → its HID device, inside a USB interface.
    fn hidraw(sysfs: &Path, name: &str, id: &str, interface: &str) {
        let usb = sysfs.join(format!("devices/usb/{name}-if"));
        let hid = usb.join("hid");
        fs::create_dir_all(&hid).unwrap();
        fs::write(usb.join("bInterfaceNumber"), format!("{interface}\n")).unwrap();
        fs::write(
            hid.join("uevent"),
            format!("DRIVER=hid-generic\nHID_ID={id}\n"),
        )
        .unwrap();
        let class = sysfs.join("class/hidraw").join(name);
        fs::create_dir_all(&class).unwrap();
        symlink(&hid, class.join("device")).unwrap();
    }

    #[test]
    fn finds_the_first_interface_of_one_it_knows() {
        let sysfs = tempfile::tempdir().unwrap();
        hidraw(sysfs.path(), "hidraw0", "0003:0000046D:0000C52B", "00");
        hidraw(sysfs.path(), "hidraw3", "0003:00001038:0000113A", "00");
        hidraw(sysfs.path(), "hidraw4", "0003:00001038:0000113A", "01");
        let klc = find(sysfs.path(), Path::new("/dev")).unwrap();
        assert_eq!(klc.node(), Path::new("/dev/hidraw3"));

        let other = tempfile::tempdir().unwrap();
        hidraw(other.path(), "hidraw0", "0003:00001038:00001122", "00");
        assert_eq!(find(other.path(), Path::new("/dev")), None);
        assert_eq!(find(Path::new("/nonexistent"), Path::new("/dev")), None);
    }
}
