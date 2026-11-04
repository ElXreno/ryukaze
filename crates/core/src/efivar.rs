//! MSI's EFI variable `MsiDCVarData` through efivarfs: four bytes of
//! attributes, then the data. Writing takes root.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use rustix::{
    fs::{IFlags, ioctl_getflags, ioctl_setflags},
    io::Errno,
};

const NAME: &str = "MsiDCVarData-dd96baaf-145e-4f56-b1cf-193256298e99";
const ATTRIBUTES: usize = 4;

pub struct DcVar {
    path: PathBuf,
}

impl DcVar {
    /// `sysfs` as for [`crate::msi_ec::MsiEc::new`].
    pub fn new(sysfs: &Path) -> Self {
        Self {
            path: sysfs.join("firmware/efi/efivars").join(NAME),
        }
    }

    /// Byte `index` of the data.
    pub fn get(&self, index: usize) -> io::Result<u8> {
        let raw = fs::read(&self.path)?;
        raw.get(ATTRIBUTES + index)
            .copied()
            .ok_or_else(|| short(index))
    }

    /// Only that byte changes, and it's read back. efivarfs keeps the
    /// variable immutable: the flag is off for the write only.
    pub fn set(&self, index: usize, value: u8) -> io::Result<()> {
        let mut raw = fs::read(&self.path)?;
        *raw.get_mut(ATTRIBUTES + index)
            .ok_or_else(|| short(index))? = value;

        let file = File::open(&self.path)?;
        let immutable = flags(&file)?.filter(|flags| flags.contains(IFlags::IMMUTABLE));
        if let Some(flags) = immutable {
            ioctl_setflags(&file, flags - IFlags::IMMUTABLE)?;
        }
        let written = write_once(&self.path, &raw);
        if let Some(flags) = immutable {
            ioctl_setflags(&file, flags)?;
        }
        written?;

        if fs::read(&self.path)? != raw {
            return Err(io::Error::other(
                "MsiDCVarData: the firmware didn't keep it",
            ));
        }
        Ok(())
    }
}

fn short(index: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("MsiDCVarData has no byte {index}"),
    )
}

/// Filesystems without inode flags have nothing to clear.
fn flags(file: &File) -> io::Result<Option<IFlags>> {
    match ioctl_getflags(file) {
        Ok(flags) => Ok(Some(flags)),
        Err(err) if err == Errno::NOTTY || err == Errno::OPNOTSUPP => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// efivarfs takes the attributes and the data in one write(2).
fn write_once(path: &Path, raw: &[u8]) -> io::Result<()> {
    if OpenOptions::new().write(true).open(path)?.write(raw)? != raw.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "MsiDCVarData: short write",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var_with(sysfs: &Path, raw: &[u8]) -> DcVar {
        let var = DcVar::new(sysfs);
        fs::create_dir_all(var.path.parent().unwrap()).unwrap();
        fs::write(&var.path, raw).unwrap();
        var
    }

    #[test]
    fn sets_one_byte() {
        let sysfs = tempfile::tempdir().unwrap();
        let var = var_with(sysfs.path(), &[0x07, 0, 0, 0, 0x01, 0x01, 0, 0x04, 0, 0x35]);
        assert_eq!(var.get(5).unwrap(), 0x35);
        var.set(5, 0x34).unwrap();
        assert_eq!(
            fs::read(&var.path).unwrap(),
            [0x07, 0, 0, 0, 0x01, 0x01, 0, 0x04, 0, 0x34]
        );
        assert!(var.get(6).is_err());
        assert!(var.set(6, 0).is_err());
    }

    /// Needs CAP_LINUX_IMMUTABLE, so it passes without trying elsewhere.
    #[test]
    fn writes_through_the_immutable_flag() {
        let sysfs = tempfile::tempdir().unwrap();
        let var = var_with(sysfs.path(), &[0x07, 0, 0, 0, 0x01, 0x35]);
        let file = File::open(&var.path).unwrap();
        let Some(flags) = flags(&file).unwrap() else {
            return;
        };
        match ioctl_setflags(&file, flags | IFlags::IMMUTABLE) {
            Err(err) if err == Errno::PERM => return,
            other => other.unwrap(),
        }

        let set = var.set(1, 0x34);
        let after = ioctl_getflags(&file).unwrap();
        ioctl_setflags(&file, flags).unwrap();
        set.unwrap();
        assert_eq!(fs::read(&var.path).unwrap(), [0x07, 0, 0, 0, 0x01, 0x34]);
        assert!(after.contains(IFlags::IMMUTABLE));
    }
}
