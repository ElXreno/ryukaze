//! Local dimming of a mini-LED panel: the backlight zones behind dark parts
//! of the picture go down. msi-ec doesn't have it; MSI Center flips one bit
//! of an EC byte (its `SDRpin`/`HDRpin` switch), and another bit of the
//! same byte says the panel has it.

use std::io;

use crate::raw_ec::RawEc;

pub struct LocalDimming {
    address: u8,
    mask: u8,
    supported: u8,
}

/// Where this EC firmware keeps it, if known.
pub fn local_dimming(firmware: &str) -> Option<LocalDimming> {
    match firmware {
        // 0xD0: bit 1 set where the panel has zones, bit 2 turns them on.
        "1824EMS1.509" => Some(LocalDimming {
            address: 0xd0,
            mask: 1 << 2,
            supported: 1 << 1,
        }),
        _ => None,
    }
}

impl LocalDimming {
    /// Whether this unit's panel has it, as the EC tells.
    pub fn supported(&self, ec: &RawEc) -> io::Result<bool> {
        Ok(self.byte(ec)? & self.supported != 0)
    }

    pub fn get(&self, ec: &RawEc) -> io::Result<bool> {
        Ok(self.byte(ec)? & self.mask != 0)
    }

    /// Only its bit changes; the rest of the byte is left as read.
    pub fn set(&self, ec: &RawEc, on: bool) -> io::Result<()> {
        let byte = self.byte(ec)?;
        let value = if on {
            byte | self.mask
        } else {
            byte & !self.mask
        };
        if value != byte {
            ec.write(self.address, &[value])?;
        }
        Ok(())
    }

    fn byte(&self, ec: &RawEc) -> io::Result<u8> {
        let mut byte = [0];
        ec.read(self.address, &mut byte)?;
        Ok(byte[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ec_with(value: u8) -> (tempfile::TempDir, RawEc) {
        let sysfs = tempfile::tempdir().unwrap();
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        ram[0xd0] = value;
        fs::write(&io, ram).unwrap();
        let ec = RawEc::new(sysfs.path());
        (sysfs, ec)
    }

    #[test]
    fn flips_one_bit() {
        // As read on the Titan 18 HX: zones there, off.
        let (sysfs, ec) = ec_with(0x02);
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");

        let dimming = local_dimming("1824EMS1.509").unwrap();
        assert!(dimming.supported(&ec).unwrap());
        assert!(!dimming.get(&ec).unwrap());
        dimming.set(&ec, true).unwrap();
        assert!(dimming.get(&ec).unwrap());
        assert_eq!(fs::read(&io).unwrap()[0xd0], 0x06);
        dimming.set(&ec, false).unwrap();
        assert_eq!(fs::read(&io).unwrap()[0xd0], 0x02);

        assert!(local_dimming("1824EMS1.108").is_none());
    }

    #[test]
    fn tells_a_panel_without_zones() {
        let (_sysfs, ec) = ec_with(0x00);
        let dimming = local_dimming("1824EMS1.509").unwrap();
        assert!(!dimming.supported(&ec).unwrap());
    }
}
