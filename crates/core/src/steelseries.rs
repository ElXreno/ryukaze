//! SteelSeries' lighting controllers in MSI laptops, on USB: the keyboard's
//! (KLC) and the light bar's (ALC). Both take feature reports and output
//! reports on their first interface, through hidraw, and run effects laid
//! out alike.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rustix::ioctl::{Opcode, Updater, ioctl, opcode};

use crate::lighting::{Color, Effect, Origin, Wave};

pub const VENDOR: u16 = 0x1038;

/// A feature report, without its ID (0, none).
pub const FEATURE_LEN: usize = 524;

/// An output report, without its ID.
const OUTPUT_LEN: usize = 64;

/// HIDIOCSFEATURE: the report ID and then the report.
const SET_FEATURE: Opcode = opcode::read_write::<[u8; FEATURE_LEN + 1]>(b'H', 0x06);

/// A table slot's mode: still in its colour.
pub const STILL: u8 = 1;

/// What `restart` asks for: the effects started over, or the tables drawn
/// again as they now are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Effects,
    Tables,
}

/// The first interface of `vendor:product` under `sysfs`, as a node under
/// `dev`.
pub fn find(sysfs: &Path, dev: &Path, product: u16) -> Option<PathBuf> {
    let id = format!("HID_ID=0003:{VENDOR:08X}:{product:08X}");
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
        let name = node.file_name()?;
        (uevent.lines().any(|line| line == id) && interface.trim() == "00").then(|| dev.join(name))
    })
}

/// The board's name, as DMI has it: a controller's product ID is shared by
/// laptops with other keyboards and lights.
pub fn board(sysfs: &Path) -> Option<String> {
    let name = fs::read_to_string(sysfs.join("class/dmi/id/board_name")).ok()?;
    Some(name.trim().to_owned())
}

/// The least time between two reports, as MSI's software keeps for every
/// controller: closer, the later one may be dropped without a word.
const PACE: Duration = Duration::from_millis(10);

/// A controller opened.
pub struct Controller {
    node: File,
    last: Option<Instant>,
}

impl Controller {
    pub fn open(node: &Path) -> io::Result<Self> {
        let node = OpenOptions::new().read(true).write(true).open(node)?;
        Ok(Self { node, last: None })
    }

    pub fn feature(&mut self, report: &[u8; FEATURE_LEN]) -> io::Result<()> {
        let mut data = [0; FEATURE_LEN + 1];
        data[1..].copy_from_slice(report);
        self.wait();
        let node = &self.node;
        let sent = patiently(|| {
            // SAFETY: HIDIOCSFEATURE reads the report from `data`, which is as
            // long as its opcode says, and writes nothing past it.
            unsafe { ioctl(node, Updater::<SET_FEATURE, _>::new(&mut data)) }?;
            Ok(())
        });
        self.last = Some(Instant::now());
        sent
    }

    /// `report`'s bytes, the rest of the output report zero.
    pub fn output(&mut self, report: &[u8]) -> io::Result<()> {
        let mut data = [0; OUTPUT_LEN + 1];
        data[1..=report.len()].copy_from_slice(report);
        self.wait();
        let sent = patiently(|| self.node.write_all(&data));
        self.last = Some(Instant::now());
        sent
    }

    fn wait(&self) {
        if let Some(since) = self.last.map(|last| last.elapsed()) {
            std::thread::sleep(PACE.saturating_sub(since));
        }
    }

    /// `0d 00 [what]`. Not `09`, which saves to the controller's flash.
    pub fn restart(&mut self, what: Restart) -> io::Result<()> {
        let what = match what {
            Restart::Effects => 0x00,
            Restart::Tables => 0x02,
        };
        self.output(&[0x0d, 0x00, what])
    }

    /// Every light black over what it shows, until a restart: `50 00 r g
    /// b`, which sets the colour every light shows over the rest; then
    /// long enough for that to reach the lights (see [`BLACKOUT`]).
    pub fn blackout(&mut self) -> io::Result<()> {
        self.output(&[0x50, 0x00, 0, 0, 0])?;
        std::thread::sleep(BLACKOUT);
        Ok(())
    }

    /// The controller's colour scale: how far each channel goes at full.
    pub fn scale(&mut self, [r, g, b]: [u8; 3]) -> io::Result<()> {
        self.output(&[0x51, 0x00, r, g, b])
    }
}

/// A slot's mode the controller follows live, unlike [`STILL`], which it
/// reads only at a restart: meant for a flash on a press, with the flash's
/// colour the slot's and no length, there's none.
pub const LIVE: u8 = 8;

/// Black over every light before a restart, for this long: the controller
/// draws a frame every 10 ms and scans its matrix in 1. It restarts inside
/// its USB interrupt, for some 5 ms, with the scan stopped, and the row lit
/// just then stays lit, many times brighter: black, it stays dark.
pub(crate) const BLACKOUT: Duration = Duration::from_millis(30);

/// Whether a light follows an effect or is still, as the controller sets
/// it up at a restart: only then does one turn from one to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Effect,
    Live,
}

impl Class {
    pub(crate) fn of(slot: &[u8; 12]) -> Class {
        if slot[9] & LIVE != 0 || slot[9] == STILL {
            Class::Live
        } else {
            Class::Effect
        }
    }
}

/// What of an effect the controller takes only at a restart: its wave
/// and its length; its steps, however many, and its start colour it reads
/// live. A colour's steps depend on the colours, so their count changes
/// with them.
pub(crate) fn shape(packet: &[u8; FEATURE_LEN]) -> [&[u8]; 3] {
    [&packet[..2], &packet[0x8c..0x96], &packet[0x98..]]
}

/// A table slot for `target`, a key's HID usage or a zone: `[colour] [flash
/// colour] [flash ticks] [effect] [mode] 00 [target]`.
pub fn slot(
    color: [u8; 3],
    flash: [u8; 3],
    flash_ticks: u16,
    effect: u8,
    mode: u8,
    target: u8,
) -> [u8; 12] {
    let [r, g, b] = color;
    let [fr, fg, fb] = flash;
    let [ticks_low, ticks_high] = flash_ticks.to_le_bytes();
    [
        r, g, b, fr, fg, fb, ticks_low, ticks_high, effect, mode, 0, target,
    ]
}

/// The effects' time unit.
pub const TICK_MS: u32 = 10;

/// The plane a KLC lays its keys on, in its units: 0x105c across, 0x040d
/// down, about 239 to a key.
pub const ACROSS: u16 = 0x105c;
pub const DOWN: u16 = 0x040d;

/// How far a round of a wave's colours lies, in the plane's units, for
/// each unit of the wavelength it's sent: measured on the Titan 18 HX's
/// 1122, a round in 28.5 times 273 at 25 %.
const WAVE_SCALE: f64 = 28.5;

/// One step of an effect: each channel's change per tick, in 1/16, and
/// how many ticks it lasts.
pub(crate) type Step = ([i8; 3], u16);

/// The controller holds 16 steps an effect.
pub(crate) const STEPS: usize = 16;

pub(crate) fn ticks(ms: u32) -> u16 {
    (ms / TICK_MS).clamp(1, u16::MAX.into()) as u16
}

/// The colours an effect goes through, round and round: where it starts,
/// each channel in 1/16, and its steps from there.
#[derive(Debug, Clone, PartialEq)]
struct Round {
    start: [i32; 3],
    steps: Vec<Step>,
}

impl Round {
    fn of(effect: &Effect, color: Option<Color>) -> Round {
        let color = color.unwrap_or(Color::OFF).0;
        let (start, steps) = match effect {
            Effect::Breathe { period_ms, fade_to } => {
                let half = ticks(period_ms / 2);
                let mut steps = ramp(color, fade_to.0, half);
                steps.extend(ramp(fade_to.0, color, half));
                (color, steps)
            }
            Effect::Cycle { period_ms, colors }
            | Effect::Wave {
                period_ms, colors, ..
            } => (colors[0].0, round(colors, *period_ms)),
            // Still, or still between flashes: one long step that changes
            // nothing.
            Effect::Static | Effect::Reactive { .. } => (color, vec![([0; 3], 100)]),
        };
        debug_assert!(steps.len() <= STEPS);
        Round {
            start: start.map(|value| i32::from(value) * 16),
            steps,
        }
    }

    /// The same round, `lag` of it behind (ahead, if negative): started
    /// where it would be that far before its start. A step is cut in two
    /// there if there's room for one more; or else it starts at the nearer
    /// end of that step.
    fn lagging(self, lag: f64) -> Round {
        let total: u32 = self
            .steps
            .iter()
            .map(|&(_, length)| u32::from(length))
            .sum();
        let cut = ((-lag).rem_euclid(1.0) * f64::from(total)).round() as u32 % total;
        let room = self.steps.len() < STEPS;
        let (mut head, mut tail) = (Vec::new(), Vec::new());
        let mut done = 0;
        for &(rates, length) in &self.steps {
            let length = u32::from(length);
            let before = if done + length <= cut {
                length
            } else if done >= cut {
                0
            } else if room {
                cut - done
            } else if cut - done < done + length - cut {
                0
            } else {
                length
            };
            if before > 0 {
                head.push((rates, before as u16));
            }
            if length > before {
                tail.push((rates, (length - before) as u16));
            }
            done += length;
        }
        let start = head.iter().fold(self.start, |mut at, &(rates, length)| {
            for c in 0..3 {
                at[c] += i32::from(rates[c]) * i32::from(length);
            }
            at
        });
        tail.extend(head);
        Round { start, steps: tail }
    }
}

/// Effect `id` as `effect` and `color` make it.
pub(crate) fn effect_packet(id: u8, effect: &Effect, color: Option<Color>) -> [u8; FEATURE_LEN] {
    let wave = match effect {
        Effect::Wave { wave, .. } => Some(*wave),
        _ => None,
    };
    packet(id, &Round::of(effect, color), wave)
}

/// Effect `id`: `effect`'s colours `lag` of a round behind, as a point
/// that far from a wave's origin shows them, with no wave of its own: for
/// a controller that runs every zone alike.
pub(crate) fn lagging_packet(
    id: u8,
    effect: &Effect,
    color: Option<Color>,
    lag: f64,
) -> [u8; FEATURE_LEN] {
    packet(id, &Round::of(effect, color).lagging(lag), None)
}

/// Where a wave's origin lies in the KLC's plane, and whether it counts
/// distance across and down.
fn origin(origin: Origin) -> ([u16; 2], bool, bool) {
    match origin {
        Origin::Middle => ([ACROSS / 2, DOWN / 2], true, true),
        Origin::Left => ([0, DOWN / 2], true, false),
        Origin::Right => ([ACROSS, DOWN / 2], true, false),
        Origin::Top => ([ACROSS / 2, 0], false, true),
        Origin::Bottom => ([ACROSS / 2, DOWN], false, true),
    }
}

/// The wavelength as sent: 31 to 1001, as the vendor's app had it.
fn wavelength(percent: u8) -> u16 {
    (31 + u32::from(percent) * 970 / 100) as u16
}

/// How far behind its origin `wave` reaches `at`, a point in the KLC's
/// plane (beyond the keys, too), in rounds: out of the origin later, into
/// it sooner.
pub fn wave_lag(wave: &Wave, at: [u16; 2]) -> f64 {
    let ([x, y], across, down) = origin(wave.origin);
    let off = |counts: bool, from: u16, to: u16| {
        if counts {
            f64::from(to) - f64::from(from)
        } else {
            0.0
        }
    };
    let distance = off(across, x, at[0]).hypot(off(down, y, at[1]));
    let lag = distance / (WAVE_SCALE * f64::from(wavelength(wave.wavelength)));
    if wave.inward { -lag } else { lag }
}

/// Effect `id`: a packet `0b 00`, then up to 16 steps of `[n] 00 [r] [g]
/// [b] 00 [ticks]` (n the effect's number first, then the step's), the
/// start colour at 0x84 in 1/16, the wave at 0x8c, the steps' count at
/// 0x96 and their length at 0x98, which the controller runs round and
/// round, from the start colour each time.
fn packet(id: u8, path: &Round, wave: Option<Wave>) -> [u8; FEATURE_LEN] {
    let mut packet = [0; FEATURE_LEN];
    packet[0] = 0x0b;
    for (i, &([r, g, b], length)) in path.steps.iter().enumerate() {
        let at = 2 + 8 * i;
        packet[at] = if i == 0 { id } else { i as u8 };
        packet[at + 2..at + 5].copy_from_slice(&[r as u8, g as u8, b as u8]);
        packet[at + 6..at + 8].copy_from_slice(&length.to_le_bytes());
    }
    for (channel, value) in path.start.into_iter().enumerate() {
        let at = 0x84 + 2 * channel;
        let value = value.clamp(0, 0xfff) as u16;
        packet[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    packet[0x8a] = 0xff;
    if let Some(wave) = wave {
        let ([x, y], across, down) = origin(wave.origin);
        packet[0x8c..0x8e].copy_from_slice(&x.to_le_bytes());
        packet[0x8e..0x90].copy_from_slice(&y.to_le_bytes());
        packet[0x90] = u8::from(across);
        packet[0x92] = u8::from(down);
        packet[0x94..0x96].copy_from_slice(&wavelength(wave.wavelength).to_le_bytes());
        // 1 runs out of the origin, 0 into it.
        packet[0x9a] = u8::from(!wave.inward);
    }
    packet[0x96] = path.steps.len() as u8;
    let length: u16 = path.steps.iter().map(|&(_, length)| length).sum();
    packet[0x98..0x9a].copy_from_slice(&length.to_le_bytes());
    packet
}

/// An effect sent only as MSI's software would send one: 1 to 16 steps, none
/// empty, their length the sum of theirs, `ff 00` at 0x8a and each step's
/// spare bytes clear. One with no steps has left a controller dark for good.
pub(crate) fn check_effect(packet: &[u8; FEATURE_LEN]) -> io::Result<()> {
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

#[cfg(test)]
pub(crate) mod testing {
    use std::{fs, os::unix::fs::symlink, path::Path};

    use super::{FEATURE_LEN, Step};

    /// How far a channel moves over the steps, in 1/16.
    pub fn moved(steps: &[Step], channel: usize) -> i32 {
        steps
            .iter()
            .map(|&(rates, length)| i32::from(rates[channel]) * i32::from(length))
            .sum()
    }

    /// The steps effect `id`'s packet holds, checking they're numbered:
    /// the effect's number first, then the steps'.
    pub fn steps(packet: &[u8; FEATURE_LEN], id: u8) -> Vec<Step> {
        (0..usize::from(packet[0x96]))
            .map(|i| {
                let at = 2 + 8 * i;
                assert_eq!(packet[at], if i == 0 { id } else { i as u8 });
                let rates = [packet[at + 2], packet[at + 3], packet[at + 4]].map(|r| r as i8);
                (rates, u16::from_le_bytes([packet[at + 6], packet[at + 7]]))
            })
            .collect()
    }

    /// `class/hidraw/<name>` → its HID device, inside a USB interface.
    pub fn hidraw(sysfs: &Path, name: &str, id: &str, interface: &str) {
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

    pub fn board(sysfs: &Path, name: &str) {
        let dmi = sysfs.join("class/dmi/id");
        fs::create_dir_all(&dmi).unwrap();
        fs::write(dmi.join("board_name"), format!("{name}\n")).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::{FLASHES_MS, PERIODS_MS, RAINBOW, shortest_period_ms};
    use testing::{hidraw, moved, steps};

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
                check_effect(&effect_packet(1, effect, color)).unwrap();
                for lag in [0.0, 0.3, -0.7, 0.999, 2.5] {
                    check_effect(&lagging_packet(1, effect, color, lag)).unwrap();
                }
            }
        }

        let whole = effect_packet(1, &breathe(4000), Some(ORANGE));
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
        let steps_of = |packet: &[u8; FEATURE_LEN]| steps(packet, 0);
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

    /// Where a path is after `ticks`, tick by tick.
    fn walked(path: &Round, ticks: u32) -> [i32; 3] {
        let mut at = path.start;
        let mut left = ticks;
        for &(rates, length) in &path.steps {
            let run = left.min(u32::from(length));
            for c in 0..3 {
                at[c] += i32::from(rates[c]) * run as i32;
            }
            left -= run;
        }
        at
    }

    /// A round started part way: the same steps, from where the round
    /// would be then, a step cut in two if there's room.
    #[test]
    fn rounds_start_where_their_lag_puts_them() {
        let effects = [
            (breathe(4000), Some(ORANGE)),
            (
                Effect::Cycle {
                    period_ms: 6000,
                    colors: RAINBOW.to_vec(),
                },
                None,
            ),
        ];
        for (effect, color) in &effects {
            let path = Round::of(effect, *color);
            let total: u32 = path.steps.iter().map(|&(_, l)| u32::from(l)).sum();
            assert_eq!(path.clone().lagging(0.0), path);
            assert_eq!(path.clone().lagging(1.0), path);
            for lag in [0.25, 0.5, 0.1, -0.1, 0.37] {
                let lagging = path.clone().lagging(lag);
                let cut = ((-lag).rem_euclid(1.0) * f64::from(total)).round() as u32;
                assert_eq!(lagging.start, walked(&path, cut), "{effect:?} {lag}");
                assert_eq!(
                    lagging
                        .steps
                        .iter()
                        .map(|&(_, l)| u32::from(l))
                        .sum::<u32>(),
                    total
                );
                assert!(lagging.steps.len() <= path.steps.len() + 1);
                assert!(lagging.steps.iter().all(|&(_, l)| l > 0));
                // Round and round: back where it started.
                assert!((0..3).all(|c| moved(&lagging.steps, c) == moved(&path.steps, c)));
                // A tick after the start, as the round a tick after the cut.
                assert_eq!(walked(&lagging, 1), walked(&path, cut + 1));
            }
        }
        // A quarter behind a breath out of orange and back: on the way
        // back, half way.
        let breath = Round::of(&breathe(4000), Some(ORANGE)).lagging(0.25);
        let half_back = walked(&Round::of(&breathe(4000), Some(ORANGE)), 300);
        assert_eq!(breath.start, half_back);

        // Sixteen steps already: no room to cut one, so from its nearer end.
        let full = Round {
            start: [0; 3],
            steps: vec![([1, 0, 0], 10); STEPS],
        };
        let lagging = full.clone().lagging(-0.33);
        assert_eq!(lagging.steps.len(), STEPS);
        assert_eq!(lagging.start, [50, 0, 0]);
    }

    #[test]
    fn waves_reach_further_points_later() {
        let wave = Wave::default();
        let middle = [ACROSS / 2, DOWN / 2];
        assert_eq!(wave_lag(&wave, middle), 0.0);
        // A round in 28.5 times 273 (25 %) of the plane's units.
        let top = Wave {
            origin: Origin::Top,
            ..wave
        };
        let round = 28.5 * 273.0;
        let lag = wave_lag(&top, [ACROSS / 2, round as u16]);
        assert!((lag - 1.0).abs() < 1e-3, "{lag}");
        // From the top, only how far down counts; from the left, across.
        assert_eq!(wave_lag(&top, [0, 900]), wave_lag(&top, [ACROSS, 900]));
        let left = Wave {
            origin: Origin::Left,
            ..wave
        };
        assert_eq!(wave_lag(&left, [100, 0]), wave_lag(&left, [100, 2000]));
        // Out of the middle both ways, as far either way.
        let both = wave_lag(&wave, [ACROSS / 2 + 300, DOWN / 2 + 400]);
        assert!((both - 500.0 / round).abs() < 1e-9);
        // Into it: sooner.
        let inward = Wave {
            inward: true,
            ..top
        };
        assert_eq!(wave_lag(&inward, [0, 900]), -wave_lag(&top, [0, 900]));
    }

    #[test]
    fn finds_the_first_interface() {
        let sysfs = tempfile::tempdir().unwrap();
        hidraw(sysfs.path(), "hidraw0", "0003:0000046D:0000C52B", "00");
        hidraw(sysfs.path(), "hidraw3", "0003:00001038:0000113A", "00");
        hidraw(sysfs.path(), "hidraw4", "0003:00001038:0000113A", "01");
        let dev = Path::new("/dev");
        assert_eq!(find(sysfs.path(), dev, 0x113a), Some(dev.join("hidraw3")));
        assert_eq!(find(sysfs.path(), dev, 0x1122), None);
        assert_eq!(find(Path::new("/nonexistent"), dev, 0x113a), None);
    }

    #[test]
    fn reads_the_board() {
        let sysfs = tempfile::tempdir().unwrap();
        assert_eq!(super::board(sysfs.path()), None);
        testing::board(sysfs.path(), "MS-1824");
        assert_eq!(super::board(sysfs.path()).as_deref(), Some("MS-1824"));
    }

    #[test]
    fn keeps_reports_apart() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut controller = Controller::open(file.path()).unwrap();
        let start = Instant::now();
        for _ in 0..4 {
            controller.output(&[0x0d, 0x00, 0x00]).unwrap();
        }
        assert!(
            start.elapsed() >= Duration::from_millis(30),
            "{:?}",
            start.elapsed()
        );
        let written = fs::read(file.path()).unwrap();
        assert_eq!(written.len(), 4 * (OUTPUT_LEN + 1));
        assert_eq!(&written[..4], &[0x00, 0x0d, 0x00, 0x00]);
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
    fn slots_name_their_target_last() {
        assert_eq!(
            slot([1, 2, 3], [4, 5, 6], 0x0102, 7, STILL, 0x04),
            [1, 2, 3, 4, 5, 6, 0x02, 0x01, 7, 1, 0, 0x04]
        );
    }
}
