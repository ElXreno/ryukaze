//! Apex, as MSI Center calls it ("Berserk" inside it): turbo with a higher
//! power table, on AC only. msi-ec doesn't have it; it's one more value of
//! the shift mode byte, which msi-ec reads back as `unknown (197)`. On the
//! Titan 18 HX the EC then puts 7 into the mode byte ACPI reads (`OEV1`)
//! instead of turbo's 4, and the dGPU's limit goes from 150 to 175 W.

use std::io;

use crate::raw_ec::RawEc;

/// Its name among the shift modes.
pub const APEX: &str = "apex";

pub struct Apex {
    address: u8,
    value: u8,
}

/// Where this EC firmware takes it, if known.
pub fn apex(firmware: &str) -> Option<Apex> {
    match firmware {
        // MSI Center writes 0xC5 where turbo is 0xC4.
        "1824EMS1.509" => Some(Apex {
            address: 0xd2,
            value: 0xc5,
        }),
        _ => None,
    }
}

impl Apex {
    pub fn on(&self, ec: &RawEc) -> io::Result<bool> {
        let mut byte = [0];
        ec.read(self.address, &mut byte)?;
        Ok(byte[0] == self.value)
    }

    /// Out of it is any other shift mode, through msi-ec.
    pub fn enter(&self, ec: &RawEc) -> io::Result<()> {
        ec.write(self.address, &[self.value])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn enters_on_the_titan_only() {
        let sysfs = tempfile::tempdir().unwrap();
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        // Turbo, as read on the Titan 18 HX.
        ram[0xd2] = 0xc4;
        ram[0xd3] = 0x81;
        fs::write(&io, ram).unwrap();
        let ec = RawEc::new(sysfs.path());

        let apex = apex("1824EMS1.509").unwrap();
        assert!(!apex.on(&ec).unwrap());
        apex.enter(&ec).unwrap();
        assert!(apex.on(&ec).unwrap());
        let ram = fs::read(&io).unwrap();
        assert_eq!(ram[0xd2], 0xc5);
        assert_eq!(ram[0xd3], 0x81);

        assert!(super::apex("1824EMS1.108").is_none());
        assert!(super::apex("17KKIMS1.115").is_none());
    }
}
