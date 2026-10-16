//! The EC's registers through ec_sys (`/sys/kernel/debug/ec/ec0/io`), for
//! what msi-ec doesn't cover. Reading takes root; writing also takes
//! `ec_sys write_support=1`.

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

pub struct RawEc {
    path: PathBuf,
}

impl RawEc {
    /// `sysfs` as for [`crate::msi_ec::MsiEc::new`].
    pub fn new(sysfs: &Path) -> Self {
        Self {
            path: sysfs.join("kernel/debug/ec/ec0/io"),
        }
    }

    pub fn read(&self, address: u8, bytes: &mut [u8]) -> io::Result<()> {
        File::open(&self.path)?.read_exact_at(bytes, address.into())
    }

    /// ec_sys turns this into one EC write per byte.
    pub fn write(&self, address: u8, bytes: &[u8]) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .open(&self.path)?
            .write_all_at(bytes, address.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_and_writes_at_an_address() {
        let sysfs = tempfile::tempdir().unwrap();
        let io = sysfs.path().join("kernel/debug/ec/ec0/io");
        fs::create_dir_all(io.parent().unwrap()).unwrap();
        fs::write(&io, [0u8; 256]).unwrap();

        let ec = RawEc::new(sysfs.path());
        ec.write(0x72, &[1, 2, 3]).unwrap();
        let mut bytes = [0; 4];
        ec.read(0x71, &mut bytes).unwrap();
        assert_eq!(bytes, [0, 1, 2, 3]);
        assert_eq!(fs::metadata(&io).unwrap().len(), 256);
    }
}
