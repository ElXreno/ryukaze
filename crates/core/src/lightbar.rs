//! The lights beside the keyboard's: a SteelSeries ALC (see
//! [`crate::steelseries`]) lights zones, the light bar's and the logo's,
//! from a table as the keyboard's controller takes them (`0e 00 [zones]
//! 00`, a slot a zone), and runs the same effects. It runs them on every
//! zone alike, wherever the zone is: a wave goes to each zone as a round of
//! its own, behind as far as the zone lies from the wave's origin. Only on
//! controllers checked by hand, as `MODELS` lists them.

use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
};

use crate::{
    lighting::{Color, Effect, Lighting, Part, PartLight},
    steelseries::{
        self, ACROSS, Controller, FEATURE_LEN, Restart, STILL, check_effect, effect_packet,
        lagging_packet, wave_lag,
    },
};

/// A zone: the part it lights, and where it lies in the keyboard's plane,
/// which goes on beyond the keys (see [`steelseries::ACROSS`]).
#[derive(Debug, PartialEq, Eq)]
struct Zone {
    id: u8,
    part: Part,
    at: [u16; 2],
}

/// A controller as checked on a laptop.
#[derive(Debug, PartialEq, Eq)]
struct Model {
    product: u16,
    board: &'static str,
    /// Effects the controller runs: without them every zone is still.
    effects: bool,
    zones: &'static [Zone],
}

/// How many rows the Titan's touchpad lights.
pub const BAR_ROWS: u8 = 5;

/// Where the Titan's touchpad row `id` lies, from the keyboard to the front:
/// 0.86 of a key apart, the first 1.4 keys below the last row of keys.
pub const fn bar_row_at(id: u8) -> [u16; 2] {
    [ACROSS / 2, 1379 + 206 * id as u16]
}

/// Where the Titan's logo lies: on the lid, above the keys, about a tenth
/// of the way across them from the left as seen from the keys (MSI's
/// picture of the lid has it a sixth of the way across a lid wider than
/// the keys).
pub const LOGO_AT: [u16; 2] = [ACROSS / 10, 0];

const fn bar_row(id: u8) -> Zone {
    Zone {
        id,
        part: Part::Bar,
        at: bar_row_at(id),
    }
}

const MODELS: [Model; 1] = [
    // Titan 18 HX A2WJ: MSI's software calls it the Titan 18 Dragon Edition's.
    Model {
        product: 0x1169,
        board: "MS-1824",
        effects: true,
        zones: &[
            bar_row(0),
            bar_row(1),
            bar_row(2),
            bar_row(3),
            bar_row(4),
            Zone {
                id: 5,
                part: Part::Logo,
                at: LOGO_AT,
            },
        ],
    },
];

/// A controller found: its hidraw node, and how it was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alc {
    node: PathBuf,
    model: &'static Model,
}

/// The known controller's first interface, under `sysfs`, as a node under
/// `dev`.
pub fn find(sysfs: &Path, dev: &Path) -> Option<Alc> {
    let board = steelseries::board(sysfs)?;
    MODELS
        .iter()
        .filter(|model| model.board == board)
        .find_map(|model| {
            let node = steelseries::find(sysfs, dev, model.product)?;
            Some(Alc { node, model })
        })
}

impl Alc {
    pub fn node(&self) -> &Path {
        &self.node
    }

    /// The parts it lights, each once, in the order profiles name them.
    pub fn parts(&self) -> Vec<Part> {
        Part::ALL
            .into_iter()
            .filter(|part| self.model.zones.iter().any(|zone| zone.part == *part))
            .collect()
    }

    /// The bar and the logo as `lighting` has them.
    pub fn show(&self, lighting: &Lighting) -> io::Result<()> {
        let plan = plan(self.model, lighting);
        for packet in &plan.effects {
            check_effect(packet)?;
        }
        let mut alc = Controller::open(&self.node)?;
        for packet in plan.effects.iter().chain([&plan.table]) {
            alc.feature(packet)?;
        }
        alc.restart(plan.restart)
    }
}

/// What goes to the controller: the effects, the table, then the restart.
#[derive(Debug, PartialEq)]
struct Plan {
    effects: Vec<[u8; FEATURE_LEN]>,
    table: [u8; FEATURE_LEN],
    restart: Restart,
}

/// Every zone on its part's effect, or still: the same effect sent once,
/// numbered from 0, but a wave's round once a zone, behind as far as the
/// zone lies from its origin. A reactive part stays still: the controller
/// has no keys to flash.
fn plan(model: &Model, lighting: &Lighting) -> Plan {
    let lighting = if model.effects {
        Cow::Borrowed(lighting)
    } else {
        Cow::Owned(lighting.still())
    };
    let mut effects: Vec<[u8; FEATURE_LEN]> = Vec::new();
    let mut table = [0; FEATURE_LEN];
    table[..4].copy_from_slice(&[0x0e, 0x00, model.zones.len() as u8, 0x00]);
    let (slots, _) = table[4..].as_chunks_mut::<12>();
    for (place, zone) in slots.iter_mut().zip(model.zones) {
        let still = |color: Option<Color>| {
            steelseries::slot(color.unwrap_or(Color::OFF).0, [0; 3], 0, 0, STILL, zone.id)
        };
        *place = match lighting.part(zone.part) {
            PartLight::Still(color) => still(Some(color)),
            PartLight::Effect {
                effect: Effect::Static | Effect::Reactive { .. },
                color,
                ..
            } => still(color),
            PartLight::Effect { effect, color, .. } => {
                let id = effects.len() as u8;
                let packet = match effect {
                    Effect::Wave { wave, .. } => {
                        lagging_packet(id, effect, color, wave_lag(wave, zone.at))
                    }
                    _ => effect_packet(id, effect, color),
                };
                let same = effects
                    .iter()
                    .find(|sent| sent[..2] == packet[..2] && sent[3..] == packet[3..]);
                let id = match same {
                    Some(sent) => sent[2],
                    None => {
                        effects.push(packet);
                        id
                    }
                };
                let color = color.unwrap_or(Color::OFF).0;
                steelseries::slot(color, [0; 3], 0, id, 0, zone.id)
            }
        };
    }
    let restart = if effects.is_empty() {
        Restart::Tables
    } else {
        Restart::Effects
    };
    Plan {
        effects,
        table,
        restart,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lighting::{Origin, RAINBOW, Wave},
        steelseries::testing::{board, hidraw},
    };

    #[test]
    fn finds_it_on_the_board_it_was_checked_on() {
        let dev = Path::new("/dev");
        let sysfs = tempfile::tempdir().unwrap();
        hidraw(sysfs.path(), "hidraw0", "0003:00001038:00001122", "00");
        hidraw(sysfs.path(), "hidraw2", "0003:00001038:00001169", "00");
        hidraw(sysfs.path(), "hidraw3", "0003:00001038:00001169", "01");
        assert_eq!(find(sysfs.path(), dev), None);
        board(sysfs.path(), "MS-182L");
        assert_eq!(find(sysfs.path(), dev), None);
        board(sysfs.path(), "MS-1824");
        let alc = find(sysfs.path(), dev).unwrap();
        assert_eq!(alc.node(), Path::new("/dev/hidraw2"));
        assert_eq!(alc.parts(), [Part::Bar, Part::Logo]);
    }

    const TITAN: &Model = &MODELS[0];
    const RED: Color = Color([0xff, 0, 0]);
    const GREEN: Color = Color([0, 0xff, 0]);

    fn lighting(effect: Effect, color: Option<Color>) -> Lighting {
        Lighting {
            effect,
            color,
            keys: Vec::new(),
            parts: Vec::new(),
            zones: Vec::new(),
        }
    }

    fn breathe() -> Effect {
        Effect::Breathe {
            period_ms: 4000,
            fade_to: Color::OFF,
        }
    }

    fn slots(sent: &Plan) -> Vec<[u8; 12]> {
        sent.table[4..4 + 6 * 12].as_chunks::<12>().0.to_vec()
    }

    /// The bytes that turned the Titan's logo green and left the bar.
    #[test]
    fn colours_the_bar_and_the_logo() {
        let still = Lighting {
            parts: vec![(Part::Logo, GREEN)],
            ..lighting(Effect::Static, Some(RED))
        };
        let sent = plan(TITAN, &still);
        assert!(sent.effects.is_empty());
        assert_eq!(sent.restart, Restart::Tables);
        assert_eq!(&sent.table[..4], &[0x0e, 0x00, 0x06, 0x00]);
        for (zone, slot) in slots(&sent).iter().enumerate() {
            let color = if zone == 5 { GREEN } else { RED };
            let mut want = [0; 12];
            want[..3].copy_from_slice(&color.0);
            want[9] = STILL;
            want[11] = zone as u8;
            assert_eq!(slot, &want, "zone {zone}");
        }
        assert!(sent.table[4 + 6 * 12..].iter().all(|&byte| byte == 0));
    }

    /// One breath for all: sent once, every zone on it; still where the
    /// controller runs no effects.
    #[test]
    fn breathes_all_together() {
        let sent = plan(TITAN, &lighting(breathe(), Some(RED)));
        assert_eq!(sent.effects, vec![effect_packet(0, &breathe(), Some(RED))]);
        assert_eq!(sent.restart, Restart::Effects);
        for (zone, slot) in slots(&sent).iter().enumerate() {
            assert_eq!(slot, &[0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, zone as u8]);
        }
        let still_only = Model {
            effects: false,
            ..MODELS[0]
        };
        let still = plan(&still_only, &lighting(breathe(), Some(RED)));
        assert!(still.effects.is_empty() && still.restart == Restart::Tables);
        assert!(
            slots(&still)
                .iter()
                .all(|slot| slot[9] == STILL && slot[..3] == RED.0)
        );
    }

    /// From the top, the rows further from the keys later: a round each,
    /// with no wave of its own.
    #[test]
    fn waves_on_down_the_rows() {
        let wave = Wave {
            origin: Origin::Top,
            ..Wave::default()
        };
        let waving = Effect::Wave {
            period_ms: 3000,
            colors: RAINBOW.to_vec(),
            wave,
        };
        let sent = plan(TITAN, &lighting(waving.clone(), None));
        assert_eq!(sent.restart, Restart::Effects);
        assert_eq!(sent.effects.len(), 6);
        for (zone, slot) in TITAN.zones.iter().zip(slots(&sent)) {
            let id = slot[8];
            let lag = wave_lag(&wave, zone.at);
            assert_eq!(
                sent.effects[usize::from(id)],
                lagging_packet(id, &waving, None, lag)
            );
            assert_eq!(slot[9], 0);
        }
        let lags: Vec<f64> = TITAN.zones[..5]
            .iter()
            .map(|zone| wave_lag(&wave, zone.at))
            .collect();
        assert!(lags.windows(2).all(|pair| pair[0] < pair[1]), "{lags:?}");
        assert!(
            sent.effects
                .iter()
                .all(|packet| packet[0x8c..0x96] == [0; 10] && check_effect(packet).is_ok())
        );
    }

    /// Its own colour over its zone's effect over the profile's; a
    /// reactive profile leaves them still.
    #[test]
    fn parts_take_their_own_or_their_zones() {
        let reactive = Effect::Reactive {
            flash: Color([0xff; 3]),
            flash_ms: 500,
        };
        let zoned = Lighting {
            parts: vec![(Part::Logo, GREEN)],
            zones: vec![crate::lighting::Zone {
                keys: Vec::new(),
                parts: vec![Part::Bar],
                effect: breathe(),
                color: Some(RED),
            }],
            ..lighting(reactive.clone(), Some(Color([0x20; 3])))
        };
        let sent = plan(TITAN, &zoned);
        assert_eq!(sent.effects, vec![effect_packet(0, &breathe(), Some(RED))]);
        let zoned_slots = slots(&sent);
        assert!(
            zoned_slots[..5]
                .iter()
                .all(|slot| slot[8] == 0 && slot[9] == 0)
        );
        assert_eq!(&zoned_slots[5][..3], &GREEN.0);
        assert_eq!(zoned_slots[5][9], STILL);

        let reacting = plan(TITAN, &lighting(reactive, Some(RED)));
        assert!(reacting.effects.is_empty());
        assert!(
            slots(&reacting)
                .iter()
                .all(|slot| slot[9] == STILL && slot[..3] == RED.0)
        );
    }
}
