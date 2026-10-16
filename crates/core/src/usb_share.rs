//! USB Power Share: the USB ports keep powering devices while the laptop
//! is off. msi-ec doesn't have it; it's one bit in EC RAM, and only on
//! firmware whose map has been checked (on 17KKIMS1, 0xBF bit 5; the BIOS
//! sets bit 3 of the same byte at every boot).

use std::io;

use crate::raw_ec::RawEc;

pub struct UsbShare {
    address: u8,
    mask: u8,
}

/// Where this EC firmware keeps it, if known.
pub fn usb_share(firmware: &str) -> Option<UsbShare> {
    match firmware {
        "17KKIMS1.115" => Some(UsbShare {
            address: 0xbf,
            mask: 1 << 5,
        }),
        _ => None,
    }
}

impl UsbShare {
    pub fn get(&self, ec: &RawEc) -> io::Result<bool> {
        let mut byte = [0];
        ec.read(self.address, &mut byte)?;
        Ok(byte[0] & self.mask != 0)
    }

    /// Only its bit changes; the rest of the byte is left as read.
    pub fn set(&self, ec: &RawEc, on: bool) -> io::Result<()> {
        let mut byte = [0];
        ec.read(self.address, &mut byte)?;
        let value = if on {
            byte[0] | self.mask
        } else {
            byte[0] & !self.mask
        };
        if value != byte[0] {
            ec.write(self.address, &[value])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn flips_one_bit() {
        let sysfs = tempfile::tempdir().unwrap();
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        let mut ram = [0u8; 256];
        // As read on the Alpha 17: bit 3 set by the BIOS, sharing off.
        ram[0xbf] = 0x08;
        fs::write(&io, ram).unwrap();
        let ec = RawEc::new(sysfs.path());

        let share = usb_share("17KKIMS1.115").unwrap();
        assert!(!share.get(&ec).unwrap());
        share.set(&ec, true).unwrap();
        assert!(share.get(&ec).unwrap());
        assert_eq!(fs::read(&io).unwrap()[0xbf], 0x28);
        share.set(&ec, false).unwrap();
        assert_eq!(fs::read(&io).unwrap()[0xbf], 0x08);

        assert!(usb_share("17KKIMS1.114").is_none());
    }
}
