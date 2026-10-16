//! The GPU MUX: whether the panel hangs off the iGPU (hybrid: the dGPU
//! renders through it and sleeps when idle) or off the dGPU (discrete).
//! msi-ec doesn't have it. On 17KKIMS1, as the firmware does it:
//!
//! - 0x2E bit 6 is where the panel is now; the EC sets the MUX from it at
//!   power-on.
//! - 0xD1 bit 0 asks for a switch. The EC passes it on through ACPI
//!   (`_Q90`) to the BIOS's SMM, which flips the mode it keeps in CMOS and
//!   answers in 0xD1: bit 1 set, bit 2 the mode. It flips rather than sets,
//!   so a second ask takes the first back.
//! - At power-off the EC copies bit 2 into 0x2E bit 6; the BIOS brings its
//!   Setup in line at the next boot.

use std::{fmt, io};

use crate::raw_ec::RawEc;

const PANEL: u8 = 0x2e;
const ON_DGPU: u8 = 1 << 6;
const SWITCH: u8 = 0xd1;
const ASK: u8 = 1 << 0;
const ANSWERED: u8 = 1 << 1;
const TO_DGPU: u8 = 1 << 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMode {
    Hybrid,
    Discrete,
}

impl GpuMode {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "hybrid" => Some(Self::Hybrid),
            "discrete" => Some(Self::Discrete),
            _ => None,
        }
    }

    fn from_bit(set: bool) -> Self {
        if set { Self::Discrete } else { Self::Hybrid }
    }
}

impl fmt::Display for GpuMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Hybrid => "hybrid",
            Self::Discrete => "discrete",
        })
    }
}

pub struct GpuMux(());

/// Whether this EC firmware's MUX is known.
pub fn gpu_mux(firmware: &str) -> Option<GpuMux> {
    match firmware {
        "17KKIMS1.115" => Some(GpuMux(())),
        _ => None,
    }
}

impl GpuMux {
    /// Where the panel is now.
    pub fn now(&self, ec: &RawEc) -> io::Result<GpuMode> {
        Ok(GpuMode::from_bit(read(ec, PANEL)? & ON_DGPU != 0))
    }

    /// The mode the BIOS took for the next power-on, once it has answered
    /// an ask.
    pub fn answer(&self, ec: &RawEc) -> io::Result<Option<GpuMode>> {
        let switch = read(ec, SWITCH)?;
        Ok((switch & (ASK | ANSWERED) == ANSWERED)
            .then(|| GpuMode::from_bit(switch & TO_DGPU != 0)))
    }

    /// Where the panel will be after a restart.
    pub fn next(&self, ec: &RawEc) -> io::Result<GpuMode> {
        match self.answer(ec)? {
            Some(mode) => Ok(mode),
            None => self.now(ec),
        }
    }

    /// Asks for the other mode than [`Self::next`], and clears the last
    /// answer; the rest of 0xD1 stays as read.
    pub fn ask(&self, ec: &RawEc) -> io::Result<()> {
        let switch = read(ec, SWITCH)?;
        ec.write(SWITCH, &[switch & !(ASK | ANSWERED | TO_DGPU) | ASK])
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
    use std::{fs, path::Path};

    fn ec_with(sysfs: &Path, panel: u8, switch: u8) -> RawEc {
        let io = sysfs.join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        ram[PANEL as usize] = panel;
        ram[SWITCH as usize] = switch;
        fs::write(&io, ram).unwrap();
        RawEc::new(sysfs)
    }

    #[test]
    fn reads_now_and_next() {
        let sysfs = tempfile::tempdir().unwrap();
        let mux = gpu_mux("17KKIMS1.115").unwrap();

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

        assert!(gpu_mux("17KKIMS1.114").is_none());
    }

    #[test]
    fn asks_in_its_own_bits() {
        let sysfs = tempfile::tempdir().unwrap();
        let ec = ec_with(sysfs.path(), 0x0b, 0xf0 | ANSWERED | TO_DGPU);
        gpu_mux("17KKIMS1.115").unwrap().ask(&ec).unwrap();
        let ram = fs::read(sysfs.path().join("kernel/debug/ec/ec0/io")).unwrap();
        assert_eq!(ram[SWITCH as usize], 0xf0 | ASK);
        assert_eq!(ram[PANEL as usize], 0x0b);
    }
}
