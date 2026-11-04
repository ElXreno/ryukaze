//! The GPU MUX: the panel off the iGPU (hybrid), off the dGPU (discrete),
//! or the dGPU off altogether (integrated). msi-ec doesn't have it; how
//! each firmware takes an ask is in docs/hardware.md.

use std::{fmt, io, path::Path};

use crate::{efivar::DcVar, raw_ec::RawEc};

const PANEL: u8 = 0x2e;
const ON_DGPU: u8 = 1 << 6;
const SWITCH: u8 = 0xd1;
const ASK: u8 = 1 << 0;
const ANSWERED: u8 = 1 << 1;
const TO_DGPU: u8 = 1 << 2;

const GPU: usize = 5;
const REQUEST: u8 = 0b11;
const SET: u32 = 2;
const KEEPS: u8 = 1 << 4;
const HAS_INTEGRATED: u8 = 1 << 5;
const LACKS_DISCRETE: u8 = 1 << 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMode {
    Hybrid,
    Discrete,
    Integrated,
}

impl GpuMode {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "hybrid" => Some(Self::Hybrid),
            "discrete" => Some(Self::Discrete),
            "integrated" => Some(Self::Integrated),
            _ => None,
        }
    }

    fn from_bit(set: bool) -> Self {
        if set { Self::Discrete } else { Self::Hybrid }
    }

    fn from_code(bits: u8) -> io::Result<Self> {
        match bits & REQUEST {
            0 => Ok(Self::Hybrid),
            1 => Ok(Self::Discrete),
            2 => Ok(Self::Integrated),
            code => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("MsiDCVarData: no GPU mode {code}"),
            )),
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Hybrid => 0,
            Self::Discrete => 1,
            Self::Integrated => 2,
        }
    }
}

impl fmt::Display for GpuMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Hybrid => "hybrid",
            Self::Discrete => "discrete",
            Self::Integrated => "integrated",
        })
    }
}

/// How the BIOS takes an ask.
#[derive(Clone, Copy)]
enum Kind {
    /// Flips the mode it keeps in CMOS; the panel's place is in 0x2E.
    Flips,
    /// Takes the mode asked for in `MsiDCVarData` and keeps it.
    Keeps,
}

/// The EC firmware whose MUX is known.
const FIRMWARES: [(&str, Kind); 2] = [("17KKIMS1.115", Kind::Flips), ("1824EMS1.509", Kind::Keeps)];

const FLIPS_BETWEEN: [GpuMode; 2] = [GpuMode::Hybrid, GpuMode::Discrete];

/// Byte 5's bits for the modes past hybrid, and whether a set bit gives
/// the mode or takes it away.
const MODE_BITS: [(GpuMode, u8, bool); 2] = [
    (GpuMode::Discrete, LACKS_DISCRETE, false),
    (GpuMode::Integrated, HAS_INTEGRATED, true),
];

enum Bios {
    Flips,
    Keeps(DcVar),
}

pub struct GpuMux {
    bios: Bios,
    modes: Vec<GpuMode>,
}

/// Whether this EC firmware's MUX is known and, where the BIOS keeps a
/// mode, whether it says so.
pub fn gpu_mux(firmware: &str, sysfs: &Path) -> Option<GpuMux> {
    let (_, kind) = FIRMWARES
        .into_iter()
        .find(|(known, _)| *known == firmware)?;
    match kind {
        Kind::Flips => Some(GpuMux {
            bios: Bios::Flips,
            modes: FLIPS_BETWEEN.to_vec(),
        }),
        Kind::Keeps => {
            let var = DcVar::new(sysfs);
            let gpu = var.get(GPU).ok().filter(|gpu| gpu & KEEPS != 0)?;
            Some(GpuMux {
                bios: Bios::Keeps(var),
                modes: kept_modes(gpu),
            })
        }
    }
}

fn kept_modes(gpu: u8) -> Vec<GpuMode> {
    let past_hybrid = MODE_BITS
        .into_iter()
        .filter(|&(_, bit, gives)| (gpu & bit != 0) == gives)
        .map(|(mode, _, _)| mode);
    std::iter::once(GpuMode::Hybrid)
        .chain(past_hybrid)
        .collect()
}

impl GpuMux {
    /// The modes this laptop has.
    pub fn modes(&self) -> &[GpuMode] {
        &self.modes
    }

    /// Where the panel is now.
    pub fn now(&self, ec: &RawEc) -> io::Result<GpuMode> {
        match &self.bios {
            Bios::Keeps(var) => GpuMode::from_code(var.get(GPU)? >> SET),
            Bios::Flips => Ok(GpuMode::from_bit(read(ec, PANEL)? & ON_DGPU != 0)),
        }
    }

    /// The mode the BIOS took for the next power-on, once it has answered
    /// an ask.
    pub fn answer(&self, ec: &RawEc) -> io::Result<Option<GpuMode>> {
        let switch = read(ec, SWITCH)?;
        if switch & (ASK | ANSWERED) != ANSWERED {
            return Ok(None);
        }
        match &self.bios {
            Bios::Keeps(var) => GpuMode::from_code(var.get(GPU)?).map(Some),
            Bios::Flips => Ok(Some(GpuMode::from_bit(switch & TO_DGPU != 0))),
        }
    }

    /// Where the panel will be after a restart.
    pub fn next(&self, ec: &RawEc) -> io::Result<GpuMode> {
        match &self.bios {
            Bios::Keeps(var) => GpuMode::from_code(var.get(GPU)?),
            Bios::Flips => match self.answer(ec)? {
                Some(mode) => Ok(mode),
                None => self.now(ec),
            },
        }
    }

    /// Asks for `mode` where the BIOS keeps one, for the other mode than
    /// [`Self::next`] where it flips. Clears the last answer; the rest of
    /// 0xD1 stays as read.
    pub fn ask(&self, ec: &RawEc, mode: GpuMode) -> io::Result<()> {
        let switch = read(ec, SWITCH)?;
        self.request(mode)?;
        ec.write(SWITCH, &[switch & !(ASK | ANSWERED | TO_DGPU) | ASK])
    }

    /// Puts back the mode asked for before an ask the BIOS didn't answer.
    pub fn withdraw(&self, before: GpuMode) -> io::Result<()> {
        self.request(before)
    }

    fn request(&self, mode: GpuMode) -> io::Result<()> {
        match &self.bios {
            Bios::Keeps(var) => var.set(GPU, var.get(GPU)? & !REQUEST | mode.code()),
            Bios::Flips => Ok(()),
        }
    }
}

fn read(ec: &RawEc, address: u8) -> io::Result<u8> {
    let mut byte = [0];
    ec.read(address, &mut byte)?;
    Ok(byte[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const VAR: &str = "firmware/efi/efivars/MsiDCVarData-dd96baaf-145e-4f56-b1cf-193256298e99";

    fn ec_with(sysfs: &Path, panel: u8, switch: u8) -> RawEc {
        let io = sysfs.join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        ram[PANEL as usize] = panel;
        ram[SWITCH as usize] = switch;
        fs::write(&io, ram).unwrap();
        RawEc::new(sysfs)
    }

    /// `MsiDCVarData` as on the Titan 18 HX, with its byte 5 at `gpu`.
    fn var_with(sysfs: &Path, gpu: u8) {
        let path = sysfs.join(VAR);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut raw = [0u8; 24];
        raw[..11].copy_from_slice(&[0x07, 0, 0, 0, 0x01, 0x01, 0, 0x04, 0, gpu, 0x01]);
        fs::write(path, raw).unwrap();
    }

    fn gpu_byte(sysfs: &Path) -> u8 {
        fs::read(sysfs.join(VAR)).unwrap()[4 + GPU]
    }

    #[test]
    fn reads_now_and_next() {
        let sysfs = tempfile::tempdir().unwrap();
        let mux = gpu_mux("17KKIMS1.115", sysfs.path()).unwrap();
        assert_eq!(mux.modes(), [GpuMode::Hybrid, GpuMode::Discrete]);

        // Hybrid, the camera bit and the rest of 0x2E as the EC starts them.
        let ec = ec_with(sysfs.path(), 0x0b, 0);
        assert_eq!(mux.now(&ec).unwrap(), GpuMode::Hybrid);
        assert_eq!(mux.answer(&ec).unwrap(), None);
        assert_eq!(mux.next(&ec).unwrap(), GpuMode::Hybrid);

        // The BIOS took discrete for the next power-on.
        let ec = ec_with(sysfs.path(), 0x0b, ANSWERED | TO_DGPU);
        assert_eq!(mux.now(&ec).unwrap(), GpuMode::Hybrid);
        assert_eq!(mux.next(&ec).unwrap(), GpuMode::Discrete);

        // Asked, not answered yet: the old answer doesn't count.
        let ec = ec_with(sysfs.path(), 0x4b, ASK | ANSWERED);
        assert_eq!(mux.now(&ec).unwrap(), GpuMode::Discrete);
        assert_eq!(mux.answer(&ec).unwrap(), None);

        assert!(gpu_mux("17KKIMS1.114", sysfs.path()).is_none());
    }

    #[test]
    fn asks_in_its_own_bits() {
        let sysfs = tempfile::tempdir().unwrap();
        let ec = ec_with(sysfs.path(), 0x0b, 0xf0 | ANSWERED | TO_DGPU);
        gpu_mux("17KKIMS1.115", sysfs.path())
            .unwrap()
            .ask(&ec, GpuMode::Discrete)
            .unwrap();
        let ram = fs::read(sysfs.path().join("kernel/debug/ec/ec0/io")).unwrap();
        assert_eq!(ram[SWITCH as usize], 0xf0 | ASK);
        assert_eq!(ram[PANEL as usize], 0x0b);
    }

    /// Where the BIOS flips, an ask is the EC's alone: the variable is for
    /// the Win/Fn swap there, and stays as it is even if it's there.
    #[test]
    fn the_alpha_leaves_the_variable_alone() {
        let sysfs = tempfile::tempdir().unwrap();
        let ec = ec_with(sysfs.path(), 0x0b, 0);
        var_with(sysfs.path(), 0x35);
        let before = fs::read(sysfs.path().join(VAR)).unwrap();
        let mux = gpu_mux("17KKIMS1.115", sysfs.path()).unwrap();
        mux.ask(&ec, GpuMode::Discrete).unwrap();
        mux.withdraw(GpuMode::Hybrid).unwrap();
        assert_eq!(fs::read(sysfs.path().join(VAR)).unwrap(), before);
    }

    #[test]
    fn titan_reads_what_the_bios_keeps() {
        let sysfs = tempfile::tempdir().unwrap();
        let ec = ec_with(sysfs.path(), 0x5b, 0);
        for (gpu, now, next) in [
            (0x35, GpuMode::Discrete, GpuMode::Discrete),
            (0x30, GpuMode::Hybrid, GpuMode::Hybrid),
            (0x3a, GpuMode::Integrated, GpuMode::Integrated),
            (0x34, GpuMode::Discrete, GpuMode::Hybrid),
        ] {
            var_with(sysfs.path(), gpu);
            let mux = gpu_mux("1824EMS1.509", sysfs.path()).unwrap();
            assert_eq!(
                mux.modes(),
                [GpuMode::Hybrid, GpuMode::Discrete, GpuMode::Integrated]
            );
            assert_eq!(mux.now(&ec).unwrap(), now, "{gpu:#04x}");
            assert_eq!(mux.next(&ec).unwrap(), next, "{gpu:#04x}");
        }

        var_with(sysfs.path(), 0x15);
        let mux = gpu_mux("1824EMS1.509", sysfs.path()).unwrap();
        assert_eq!(mux.modes(), [GpuMode::Hybrid, GpuMode::Discrete]);
        var_with(sysfs.path(), 0x75);
        let mux = gpu_mux("1824EMS1.509", sysfs.path()).unwrap();
        assert_eq!(mux.modes(), [GpuMode::Hybrid, GpuMode::Integrated]);

        // Without bit 4, or without the variable, it isn't this MUX.
        var_with(sysfs.path(), 0x05);
        assert!(gpu_mux("1824EMS1.509", sysfs.path()).is_none());
        fs::remove_file(sysfs.path().join(VAR)).unwrap();
        assert!(gpu_mux("1824EMS1.509", sysfs.path()).is_none());
    }

    #[test]
    fn titan_asks_through_the_variable() {
        let sysfs = tempfile::tempdir().unwrap();
        let ec = ec_with(sysfs.path(), 0x5b, 0xf0 | ANSWERED | TO_DGPU);
        var_with(sysfs.path(), 0x35);
        let mux = gpu_mux("1824EMS1.509", sysfs.path()).unwrap();

        mux.ask(&ec, GpuMode::Integrated).unwrap();
        assert_eq!(gpu_byte(sysfs.path()), 0x36);
        let ram = fs::read(sysfs.path().join("kernel/debug/ec/ec0/io")).unwrap();
        assert_eq!(ram[SWITCH as usize], 0xf0 | ASK);
        assert_eq!(ram[PANEL as usize], 0x5b);
        assert_eq!(mux.answer(&ec).unwrap(), None);

        // Answered without bit 2: integrated isn't discrete.
        let ec = ec_with(sysfs.path(), 0x5b, ANSWERED);
        assert_eq!(mux.answer(&ec).unwrap(), Some(GpuMode::Integrated));
        assert_eq!(mux.now(&ec).unwrap(), GpuMode::Discrete);
        assert_eq!(mux.next(&ec).unwrap(), GpuMode::Integrated);

        mux.withdraw(GpuMode::Discrete).unwrap();
        assert_eq!(gpu_byte(sysfs.path()), 0x35);
    }
}
