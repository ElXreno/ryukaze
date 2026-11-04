//! The keyboard's backlight: a SteelSeries KLC (see [`crate::steelseries`]),
//! which colours keys from feature reports, the keys by their HID usage.
//! Only on controllers checked by hand, as `MODELS` lists them. A report of
//! `0c 00 [entries] 00` and then (key, red, green, blue) entries colours
//! the keys it lists and leaves the others.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use crate::{
    lighting::{Color, Effect, Lighting, Zone},
    steelseries::{
        self, Class, Controller, FEATURE_LEN as REPORT_LEN, LIVE, Restart, STILL, check_effect,
        effect_packet, shape, ticks,
    },
};

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
    /// would bring the numpad back or let the power button go. Only what
    /// differs from what `sent` says the controller has goes.
    ///
    /// The controller restarts its lighting inside its USB interrupt, for
    /// some 5 ms, with the matrix's scan stopped: the row lit just then
    /// stays lit, its keys flashing many times brighter. So, where it
    /// runs effects, nothing restarts but what can't go on without: the
    /// first show, a wave's shape, a key turning from still to an effect.
    /// Still keys follow their tables live (see [`LIVE`]), effects take new
    /// colours live, a key's own colour over an effect goes over it at once
    /// and comes off again; and a restart goes under black, `50` first, so
    /// the row left lit is dark.
    pub fn show(&self, sent: &mut Sent, lighting: Option<&Lighting>, over: Over) -> io::Result<()> {
        let lighting = lighting.map(keys_only);
        let plan = plan(self.model, lighting.as_ref(), over);
        for packet in plan.packets.iter().filter(|packet| packet[0] == 0x0b) {
            check_effect(packet)?;
        }
        let changes = if self.model.effects {
            changes(&plan, sent, self.model.scale)
        } else {
            changes_still(&plan, sent, self.model.scale)
        };
        let shown = (|| {
            let mut klc = Controller::open(&self.node)?;
            if changes.blackout {
                klc.blackout()?;
            }
            for packet in &changes.packets {
                klc.feature(packet)?;
            }
            match (changes.scale, changes.restart) {
                // Setting the scale restarts as well.
                (Some(scale), Some(_)) => klc.scale(scale)?,
                (Some(scale), None) => klc.scale(scale)?,
                (None, Some(restart)) => klc.restart(restart)?,
                (None, None) => {}
            }
            for report in &changes.paint {
                klc.feature(report)?;
            }
            match &changes.release {
                Release::None => {}
                Release::All => klc.output(&[0x52])?,
                Release::Keys(keys) => {
                    for &key in keys {
                        klc.output(&[0x83, key])?;
                    }
                }
            }
            Ok(())
        })();
        match shown {
            Ok(()) => *sent = changes.sent,
            Err(_) => sent.forget(),
        }
        shown
    }

    /// What it shows now, saved to the controller's flash, which it shows
    /// from at power-on: `09`, which starts the lighting over with the scan
    /// stopped, and blinks. Only where the model saves: elsewhere how its
    /// flash takes it isn't known.
    pub fn save(&self, sent: &mut Sent) -> io::Result<()> {
        if !self.model.saves {
            return Ok(());
        }
        let saved = Controller::open(&self.node)?.output(&[0x09]);
        match saved {
            // Started over from its tables: no key's own colour over them.
            Ok(()) => {
                sent.built = classes(&sent.tables);
                sent.overrides.clear();
            }
            Err(_) => sent.forget(),
        }
        saved
    }

    /// Only `over` changed: as any other change (see [`Klc::show`]).
    pub fn overlay(
        &self,
        sent: &mut Sent,
        lighting: Option<&Lighting>,
        over: Over,
    ) -> io::Result<()> {
        if lighting.is_none() {
            return Ok(());
        }
        self.show(sent, lighting, over)
    }
}

/// Still keys but the power button, which has its effect: a still report
/// would hold it, and the restart doesn't let it go, unlike the others.
fn without_power(keys: Vec<(u8, Color)>) -> Vec<(u8, Color)> {
    keys.into_iter().filter(|&(key, _)| key != POWER).collect()
}

/// What goes to the controller: the effects' packets, then still colours.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    /// The effects, then the keys' tables, which the controller draws
    /// the keys from again whenever it redraws them, as on a brightness key.
    packets: Vec<[u8; REPORT_LEN]>,
    /// A restart after them: the effects started, or the tables drawn.
    restart: Option<Restart>,
    /// Keys still at once, over what's there: where no effects run.
    still: Vec<(u8, Color)>,
}

/// What goes to the controller for `plan`, given what it was `sent`, and
/// what it then has.
#[derive(Debug, PartialEq)]
struct Changes {
    /// Black over every key first, for a restart.
    blackout: bool,
    scale: Option<[u8; 3]>,
    packets: Vec<[u8; REPORT_LEN]>,
    restart: Option<Restart>,
    /// Keys' own colours over their effects, at once.
    paint: Vec<[u8; REPORT_LEN]>,
    /// Keys' own colours taken off again.
    release: Release,
    sent: Sent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Release {
    None,
    All,
    Keys(Vec<u8>),
}

/// Where the controller runs effects: see [`Klc::show`].
fn changes(plan: &Plan, sent: &Sent, scale: Option<[u8; 3]>) -> Changes {
    let (effects, tables): (Vec<_>, Vec<_>) =
        plan.packets.iter().partition(|packet| packet[0] == 0x0b);
    let new = slots(&tables);
    let class = |slot: &[u8; 12]| Class::of(slot);
    let reshaped = effects.len() != sent.effects.len()
        || effects
            .iter()
            .zip(&sent.effects)
            .any(|(new, old)| shape(new) != shape(old));
    let restart = !sent.restarted
        || reshaped
        || new.iter().any(|(key, slot)| {
            let built = sent.built.get(key).copied();
            match class(slot) {
                Class::Effect => built != Some(Class::Effect),
                Class::Live => !pure(slot) && built == Some(Class::Effect),
            }
        });
    if restart {
        let mut packets = effects.clone();
        packets.extend(&tables);
        return Changes {
            blackout: true,
            scale: scale.filter(|_| !sent.scaled),
            packets,
            restart: Some(Restart::Effects),
            paint: Vec::new(),
            release: Release::None,
            sent: Sent {
                scaled: true,
                restarted: true,
                built: new.iter().map(|(&key, slot)| (key, class(slot))).collect(),
                overrides: BTreeMap::new(),
                effects,
                tables,
            },
        };
    }
    let mut packets: Vec<[u8; REPORT_LEN]> = effects
        .iter()
        .zip(&sent.effects)
        .filter(|(new, old)| new != old)
        .map(|(new, _)| *new)
        .collect();
    packets.extend(
        tables
            .iter()
            .enumerate()
            .filter(|&(at, packet)| sent.tables.get(at) != Some(packet))
            .map(|(_, packet)| *packet),
    );
    // Still keys still running an effect, from the last restart: their
    // colours go over it.
    let overrides: BTreeMap<u8, Color> = new
        .iter()
        .filter(|&(key, slot)| {
            *key != POWER
                && class(slot) == Class::Live
                && sent.built.get(key) == Some(&Class::Effect)
        })
        .map(|(&key, slot)| (key, Color([slot[0], slot[1], slot[2]])))
        .collect();
    let paint: Vec<(u8, Color)> = overrides
        .iter()
        .filter(|&(key, color)| sent.overrides.get(key) != Some(color))
        .map(|(&key, &color)| (key, color))
        .collect();
    let off: Vec<u8> = sent
        .overrides
        .keys()
        .filter(|key| !overrides.contains_key(key))
        .copied()
        .collect();
    let release = if off.is_empty() {
        Release::None
    } else if overrides.is_empty() {
        Release::All
    } else {
        Release::Keys(off)
    };
    Changes {
        blackout: false,
        scale: None,
        packets,
        restart: None,
        paint: reports(&paint),
        release,
        sent: Sent {
            scaled: sent.scaled,
            restarted: true,
            built: sent.built.clone(),
            overrides,
            effects,
            tables,
        },
    }
}

/// Where it runs no effects: the tables, drawn again when they change.
fn changes_still(plan: &Plan, sent: &Sent, scale: Option<[u8; 3]>) -> Changes {
    let tables: Vec<[u8; REPORT_LEN]> = plan.packets.clone();
    let packets: Vec<[u8; REPORT_LEN]> = tables
        .iter()
        .enumerate()
        .filter(|&(at, packet)| sent.tables.get(at) != Some(packet))
        .map(|(_, packet)| *packet)
        .collect();
    let restart = (!packets.is_empty() || !sent.restarted)
        .then_some(plan.restart)
        .flatten();
    Changes {
        blackout: false,
        scale: scale.filter(|_| !sent.scaled),
        packets,
        restart,
        paint: Vec::new(),
        release: Release::None,
        sent: Sent {
            scaled: true,
            restarted: sent.restarted || restart.is_some(),
            tables,
            ..Sent::default()
        },
    }
}

/// A still slot with nothing to do on a press: what a key's own colour
/// over an effect stands for until the next restart.
fn pure(slot: &[u8; 12]) -> bool {
    slot[9] == STILL || (slot[0..3] == slot[3..6] && slot[6..8] == [0, 0])
}

/// The slots of `tables`, by key.
fn slots(tables: &[[u8; REPORT_LEN]]) -> BTreeMap<u8, [u8; 12]> {
    tables
        .iter()
        .flat_map(|packet| {
            let count = usize::from(packet[2]).min(42);
            let (slots, _) = packet[4..4 + 42 * 12].as_chunks::<12>();
            slots[..count].to_vec()
        })
        .filter(|slot| slot[11] != 0xff && slot[11] != 0)
        .map(|slot| (slot[11], slot))
        .collect()
}

fn classes(tables: &[[u8; REPORT_LEN]]) -> BTreeMap<u8, Class> {
    slots(tables)
        .iter()
        .map(|(&key, slot)| (key, Class::of(slot)))
        .collect()
}

/// What of `lighting` is the keys': zones with keys of their own, the
/// lights beside them left to their controller. A zone of the touchpad
/// alone would otherwise start the keys' effects over.
fn keys_only(lighting: &Lighting) -> Lighting {
    Lighting {
        parts: Vec::new(),
        zones: lighting
            .zones
            .iter()
            .filter(|zone| !zone.keys.is_empty())
            .map(|zone| Zone {
                parts: Vec::new(),
                ..zone.clone()
            })
            .collect(),
        ..lighting.clone()
    }
}

/// What a controller was last sent, as far as ryukaze knows: forgotten
/// where it may have lost it, as in sleep, so all of it goes again.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Sent {
    scaled: bool,
    /// Started over since forgotten, with each key's class as then.
    restarted: bool,
    built: BTreeMap<u8, Class>,
    /// Keys' own colours over their effects, as sent since.
    overrides: BTreeMap<u8, Color>,
    effects: Vec<[u8; REPORT_LEN]>,
    tables: Vec<[u8; REPORT_LEN]>,
}

impl Sent {
    pub fn forget(&mut self) {
        *self = Sent::default();
    }
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
            ..Plan::default()
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
            Effect::Static if model.effects => steelseries::slot(color, color, 0, id, LIVE, key),
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
    use crate::{
        lighting::{Part, RAINBOW, Wave},
        steelseries::testing::{board, hidraw, steps},
    };

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

    fn lighting(effect: Effect, color: Option<Color>) -> Lighting {
        Lighting {
            effect,
            color,
            keys: Vec::new(),
            parts: Vec::new(),
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

    /// The touchpad's own zone, or its colour, is nothing to the keys'
    /// controller.
    #[test]
    fn leaves_the_touchpad_to_its_controller() {
        let titan = MODELS
            .iter()
            .find(|model| model.board == Some("MS-1824"))
            .unwrap();
        let keys = lighting(Effect::Static, Some(BLUE));
        let with_bar = Lighting {
            parts: vec![(Part::Logo, ORANGE)],
            zones: vec![Zone {
                keys: Vec::new(),
                parts: vec![Part::Bar],
                effect: Effect::Breathe {
                    period_ms: 2000,
                    fade_to: Color::OFF,
                },
                color: Some(ORANGE),
            }],
            ..keys.clone()
        };
        let plan_of =
            |lighting: &Lighting| plan(titan, Some(&keys_only(lighting)), Over::default());
        assert_eq!(plan_of(&with_bar), plan_of(&keys));
    }

    /// Nothing restarts but what can't go on without, and that under
    /// black: a key's own colour over an effect, and taken off, at once;
    /// an effect's colours live; still keys by their tables.
    #[test]
    fn restarts_only_when_it_must() {
        let titan = MODELS
            .iter()
            .find(|model| model.board == Some("MS-1824"))
            .unwrap();
        let breathe = |color: Color| Lighting {
            effect: Effect::Breathe {
                period_ms: 2000,
                fade_to: Color::OFF,
            },
            ..lighting(Effect::Static, Some(color))
        };
        let send = |lighting: &Lighting, sent: &Sent| {
            changes(
                &plan(titan, Some(lighting), Over::default()),
                sent,
                titan.scale,
            )
        };
        let first = send(&breathe(BLUE), &Sent::default());
        assert!(first.blackout);
        assert_eq!(first.scale, titan.scale);
        assert_eq!(first.restart, Some(Restart::Effects));

        let again = send(&breathe(BLUE), &first.sent);
        assert!(again.packets.is_empty() && again.paint.is_empty());
        assert_eq!((again.restart, &again.release), (None, &Release::None));

        // Its own colour over the effect: a table and a still report.
        let painted = Lighting {
            keys: vec![(vec![0x04], ORANGE)],
            ..breathe(BLUE)
        };
        let paint = send(&painted, &first.sent);
        assert_eq!(paint.restart, None);
        assert!(!paint.blackout);
        assert!(paint.packets.iter().all(|packet| packet[0] != 0x0b));
        assert_eq!(paint.paint.len(), 1);

        // Back to the effect: taken off, no restart.
        let back = send(&breathe(BLUE), &paint.sent);
        assert_eq!(back.restart, None);
        assert_eq!(back.release, Release::All);

        // Another colour for the effect: live.
        let recolored = send(&breathe(ORANGE), &back.sent);
        assert_eq!(recolored.restart, None);
        assert!(recolored.packets.iter().any(|packet| packet[0] == 0x0b));

        // Another effect: started over, under black.
        let cycle = Lighting {
            effect: Effect::Cycle {
                period_ms: 2000,
                colors: vec![BLUE, ORANGE],
            },
            ..breathe(BLUE)
        };
        let cycled = send(&cycle, &recolored.sent);
        assert_eq!(cycled.restart, Some(Restart::Effects));
        assert!(cycled.blackout);
        assert_eq!(cycled.scale, None);

        // All still: its tables, live, whatever the colours.
        let still = send(&lighting(Effect::Static, Some(BLUE)), &cycled.sent);
        assert_eq!(still.restart, Some(Restart::Effects));
        let orange = send(&lighting(Effect::Static, Some(ORANGE)), &still.sent);
        assert_eq!(orange.restart, None);
        assert!(orange.paint.is_empty());
        assert!(!orange.packets.is_empty());

        let mut forgot = orange.sent.clone();
        forgot.forget();
        assert_eq!(send(&breathe(BLUE), &forgot).scale, titan.scale);
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
            parts: Vec::new(),
            effect: breathe(3000),
            color: Some(BLUE),
        };
        let waving = Zone {
            keys: named("kp5").unwrap(),
            parts: Vec::new(),
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
        // Still in the tables, followed live.
        assert_eq!(slots.iter().find(|slot| slot[11] == 0x40).unwrap()[9], LIVE);
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
        assert_eq!(slot_of(&still.packets, 0x04), (LIVE, ORANGE.0));
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
        assert_eq!(slot_of(&waving.packets, 0x1a), (LIVE, red.0));
        assert_eq!(slot_of(&waving.packets, 0x05).0, 0);

        // NumLock off, still in the tables; at once too where all is still.
        let over = Over {
            numpad_dark: true,
            power: Some(red),
        };
        let dark = plan(ALPHA, Some(&wave), over);
        assert_eq!(slot_of(&dark.packets, 0x5d), (LIVE, [0; 3]));
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
                parts: Vec::new(),
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
        assert_eq!(slot_of(tables, 0x1a), (LIVE, red.0));
        assert!(tables.iter().all(|table| table[REPORT_LEN - 2..] == [0, 0]));
    }
}
