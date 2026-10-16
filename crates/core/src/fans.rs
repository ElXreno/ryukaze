//! Fan curves and tachometers in EC RAM. msi-ec has neither, so these go
//! through [`RawEc`], and only on firmware whose layout has been checked:
//! a wrong address on another EC could be anything (on 17KKIMS1, 0xF3 is
//! the platform watchdog).

use std::{fmt, io};

use crate::raw_ec::RawEc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fan {
    Cpu,
    Gpu,
}

impl Fan {
    pub const ALL: [Fan; 2] = [Fan::Cpu, Fan::Gpu];
}

impl fmt::Display for Fan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        })
    }
}

/// A fan curve as the EC keeps it: below `temps[0]` °C the fan runs at
/// `speeds[0]` percent, from `temps[i]` up at `speeds[i + 1]`. The EC
/// steps between them, it doesn't interpolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Curve {
    pub temps: [u8; 6],
    pub speeds: [u8; 7],
}

/// What the EC runs a fan at under Cooler Boost; nothing goes higher.
pub const MAX_SPEED: u8 = 150;

impl Curve {
    /// Temperatures must rise and speeds mustn't fall: a fan that slows
    /// down as it gets hotter is a mistake, not a preference.
    pub fn new(temps: &[u8], speeds: &[u8]) -> Result<Self, String> {
        let temps: [u8; 6] = temps
            .try_into()
            .map_err(|_| format!("a curve has 6 temperatures, not {}", temps.len()))?;
        let speeds: [u8; 7] = speeds
            .try_into()
            .map_err(|_| format!("a curve has 7 speeds, not {}", speeds.len()))?;
        if !temps.is_sorted_by(|a, b| a < b) {
            return Err(format!("temperatures {temps:?} must rise"));
        }
        if let Some(&hot) = temps.iter().find(|&&temp| temp > 100) {
            return Err(format!("{hot} °C is past what a fan curve covers (100)"));
        }
        if !speeds.is_sorted() {
            return Err(format!("speeds {speeds:?} must not fall as it gets hotter"));
        }
        if let Some(&fast) = speeds.iter().find(|&&speed| speed > MAX_SPEED) {
            return Err(format!(
                "{fast}% is past the most a fan runs at ({MAX_SPEED}%)"
            ));
        }
        Ok(Self { temps, speeds })
    }
}

/// Where one fan's curve and tachometer live.
struct Table {
    temps: u8,
    speeds: u8,
    /// Big-endian; the time per turn, so it falls as the fan speeds up.
    tach: u8,
    /// What the EC loads from its ROM at power-up and after S3.
    factory: Curve,
}

pub struct Layout {
    cpu: Table,
    gpu: Table,
}

/// Checked against the EC's own firmware and live: MSI Alpha 17 C7VG.
static ALPHA_17: Layout = Layout {
    cpu: Table {
        temps: 0x6a,
        speeds: 0x72,
        tach: 0xc8,
        factory: Curve {
            temps: [50, 57, 64, 71, 78, 85],
            speeds: [0, 40, 45, 50, 57, 65, 80],
        },
    },
    gpu: Table {
        temps: 0x82,
        speeds: 0x8a,
        tach: 0xca,
        factory: Curve {
            temps: [55, 60, 65, 70, 75, 80],
            speeds: [0, 48, 55, 62, 68, 77, 85],
        },
    },
};

/// The layout for this EC firmware (msi-ec's `fw_version`), if known.
pub fn layout(firmware: &str) -> Option<&'static Layout> {
    match firmware {
        "17KKIMS1.115" => Some(&ALPHA_17),
        _ => None,
    }
}

impl Layout {
    fn table(&self, fan: Fan) -> &Table {
        match fan {
            Fan::Cpu => &self.cpu,
            Fan::Gpu => &self.gpu,
        }
    }

    pub fn factory(&self, fan: Fan) -> Curve {
        self.table(fan).factory
    }

    pub fn read_curve(&self, ec: &RawEc, fan: Fan) -> io::Result<Curve> {
        let table = self.table(fan);
        let mut curve = Curve {
            temps: [0; 6],
            speeds: [0; 7],
        };
        ec.read(table.temps, &mut curve.temps)?;
        ec.read(table.speeds, &mut curve.speeds)?;
        Ok(curve)
    }

    pub fn write_curve(&self, ec: &RawEc, fan: Fan, curve: &Curve) -> io::Result<()> {
        let table = self.table(fan);
        ec.write(table.temps, &curve.temps)?;
        ec.write(table.speeds, &curve.speeds)
    }

    /// Revolutions per minute, 0 when the fan stands. The constant is the
    /// one MControlCenter uses; the firmware doesn't say, but it lands on
    /// plausible numbers (about 2800 rpm at 50%).
    pub fn rpm(&self, ec: &RawEc, fan: Fan) -> io::Result<u16> {
        let mut tach = [0; 2];
        ec.read(self.table(fan).tach, &mut tach)?;
        Ok(match u16::from_be_bytes(tach) {
            0 => 0,
            period => (470_000 / u32::from(period)).min(u16::MAX.into()) as u16,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// EC RAM as dumped on the Alpha 17: turbo, auto, fans at ~2800 rpm.
    fn alpha17() -> (tempfile::TempDir, RawEc) {
        let sysfs = tempfile::tempdir().unwrap();
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        ram[0x6a..0x70].copy_from_slice(&[50, 57, 64, 71, 78, 85]);
        ram[0x72..0x79].copy_from_slice(&[0, 40, 45, 50, 57, 65, 80]);
        ram[0x82..0x88].copy_from_slice(&[55, 60, 65, 70, 75, 80]);
        ram[0x8a..0x91].copy_from_slice(&[0, 48, 55, 62, 68, 77, 85]);
        ram[0xc8..0xcc].copy_from_slice(&[0x00, 0xa6, 0x00, 0x00]);
        ram[0xf3] = 0x5a;
        fs::write(&io, ram).unwrap();
        let ec = RawEc::new(sysfs.path());
        (sysfs, ec)
    }

    #[test]
    fn the_dump_is_the_factory_curve() {
        let (_sysfs, ec) = alpha17();
        let layout = layout("17KKIMS1.115").unwrap();
        for fan in Fan::ALL {
            assert_eq!(layout.read_curve(&ec, fan).unwrap(), layout.factory(fan));
        }
        assert_eq!(layout.rpm(&ec, Fan::Cpu).unwrap(), 2831);
        assert_eq!(layout.rpm(&ec, Fan::Gpu).unwrap(), 0);
    }

    #[test]
    fn writes_only_the_table() {
        let (sysfs, ec) = alpha17();
        let layout = layout("17KKIMS1.115").unwrap();
        let before = fs::read(sysfs.path().join("kernel/debug/ec/ec0/io")).unwrap();
        let quiet = Curve::new(&[55, 62, 69, 76, 83, 90], &[0, 30, 35, 40, 50, 60, 100]).unwrap();
        layout.write_curve(&ec, Fan::Cpu, &quiet).unwrap();
        assert_eq!(layout.read_curve(&ec, Fan::Cpu).unwrap(), quiet);

        let after = fs::read(sysfs.path().join("kernel/debug/ec/ec0/io")).unwrap();
        let changed: Vec<usize> = (0..256).filter(|&i| before[i] != after[i]).collect();
        assert!(
            changed
                .iter()
                .all(|&i| (0x6a..0x70).contains(&i) || (0x72..0x79).contains(&i))
        );
        assert_eq!(after[0xf3], 0x5a);
    }

    #[test]
    fn unknown_firmware_has_no_layout() {
        assert!(layout("17KKIMS1.114").is_none());
        assert!(layout("").is_none());
    }

    #[test]
    fn rejects_bad_curves() {
        assert!(Curve::new(&[50, 57, 64], &[0, 40, 45, 50, 57, 65, 80]).is_err());
        assert!(Curve::new(&[50, 57, 57, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 80]).is_err());
        assert!(Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 30, 57, 65, 80]).is_err());
        assert!(Curve::new(&[50, 57, 64, 71, 78, 120], &[0; 7]).is_err());
        assert!(Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 200]).is_err());
        assert!(Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 150]).is_ok());
    }
}
