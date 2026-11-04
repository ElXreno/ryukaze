//! The lights beside the keyboard's: a SteelSeries ALC (see
//! [`crate::steelseries`]) colours zones, the light bar's and the logo's,
//! from one feature report: `0e 00 [zones] 00` and a table slot a zone,
//! then a restart that draws them. Only on controllers checked by hand, as
//! `MODELS` lists them.

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::{
    keyboard::Color,
    steelseries::{self, Controller, FEATURE_LEN, Restart, STILL},
};

/// A controller as checked on a laptop: which zone is which.
#[derive(Debug, PartialEq, Eq)]
struct Model {
    product: u16,
    board: &'static str,
    bar: &'static [u8],
    logo: &'static [u8],
}

const MODELS: [Model; 1] = [
    // Titan 18 HX A2WJ: the touchpad lit in five rows, from the keyboard
    // to the front, and the lid's logo.
    Model {
        product: 0x1169,
        board: "MS-1824",
        bar: &[0, 1, 2, 3, 4],
        logo: &[5],
    },
];

/// What it shows: still, a colour a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lights {
    pub bar: Color,
    pub logo: Color,
}

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

    pub fn show(&self, lights: Lights) -> io::Result<()> {
        let mut alc = Controller::open(&self.node)?;
        alc.feature(&packet(self.model, lights))?;
        alc.restart(Restart::Tables)
    }
}

/// Every zone still in its part's colour, as many slots as there are zones.
fn packet(model: &Model, lights: Lights) -> [u8; FEATURE_LEN] {
    let zones: Vec<(u8, Color)> = model
        .bar
        .iter()
        .map(|&zone| (zone, lights.bar))
        .chain(model.logo.iter().map(|&zone| (zone, lights.logo)))
        .collect();
    let mut packet = [0; FEATURE_LEN];
    packet[..4].copy_from_slice(&[0x0e, 0x00, zones.len() as u8, 0x00]);
    let (slots, _) = packet[4..].as_chunks_mut::<12>();
    for (place, (zone, Color(color))) in slots.iter_mut().zip(zones) {
        *place = steelseries::slot(color, [0; 3], 0, 0, STILL, zone);
    }
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steelseries::testing::{board, hidraw};

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
    }

    /// The bytes that turned the Titan's logo green and left the bar.
    #[test]
    fn colours_the_bar_and_the_logo() {
        let lights = Lights {
            bar: Color([0xff, 0, 0]),
            logo: Color([0, 0xff, 0]),
        };
        let packet = packet(&MODELS[0], lights);
        assert_eq!(&packet[..4], &[0x0e, 0x00, 0x06, 0x00]);
        let (slots, _) = packet[4..4 + 6 * 12].as_chunks::<12>();
        for (zone, slot) in slots.iter().enumerate() {
            let color = if zone == 5 {
                [0, 0xff, 0]
            } else {
                [0xff, 0, 0]
            };
            let mut want = [0; 12];
            want[..3].copy_from_slice(&color);
            want[9] = STILL;
            want[11] = zone as u8;
            assert_eq!(slot, &want, "zone {zone}");
        }
        assert!(packet[4 + 6 * 12..].iter().all(|&byte| byte == 0));
    }
}
