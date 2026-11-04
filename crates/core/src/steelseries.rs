//! SteelSeries' lighting controllers in MSI laptops, on USB: the keyboard's
//! (KLC) and the light bar's (ALC). Both take feature reports and output
//! reports on their first interface, through hidraw.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use rustix::ioctl::{Opcode, Updater, ioctl, opcode};

pub const VENDOR: u16 = 0x1038;

/// A feature report, without its ID (0, none).
pub const FEATURE_LEN: usize = 524;

/// An output report, without its ID.
const OUTPUT_LEN: usize = 64;

/// HIDIOCSFEATURE: the report ID and then the report.
const SET_FEATURE: Opcode = opcode::read_write::<[u8; FEATURE_LEN + 1]>(b'H', 0x06);

/// A table slot's mode: still in its colour.
pub const STILL: u8 = 1;

/// What `restart` asks for: the effects started over, or the tables drawn
/// again as they now are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Effects,
    Tables,
}

/// The first interface of `vendor:product` under `sysfs`, as a node under
/// `dev`.
pub fn find(sysfs: &Path, dev: &Path, product: u16) -> Option<PathBuf> {
    let id = format!("HID_ID=0003:{VENDOR:08X}:{product:08X}");
    let mut nodes: Vec<PathBuf> = fs::read_dir(sysfs.join("class/hidraw"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    nodes.sort();
    nodes.into_iter().find_map(|node| {
        let device = node.join("device");
        let uevent = fs::read_to_string(device.join("uevent")).ok()?;
        // The HID device's parent is the USB interface.
        let interface = fs::read_to_string(device.join("../bInterfaceNumber")).ok()?;
        let name = node.file_name()?;
        (uevent.lines().any(|line| line == id) && interface.trim() == "00").then(|| dev.join(name))
    })
}

/// The board's name, as DMI has it: a controller's product ID is shared by
/// laptops with other keyboards and lights.
pub fn board(sysfs: &Path) -> Option<String> {
    let name = fs::read_to_string(sysfs.join("class/dmi/id/board_name")).ok()?;
    Some(name.trim().to_owned())
}

/// A controller opened.
pub struct Controller {
    node: File,
}

impl Controller {
    pub fn open(node: &Path) -> io::Result<Self> {
        let node = OpenOptions::new().read(true).write(true).open(node)?;
        Ok(Self { node })
    }

    pub fn feature(&self, report: &[u8; FEATURE_LEN]) -> io::Result<()> {
        let mut data = [0; FEATURE_LEN + 1];
        data[1..].copy_from_slice(report);
        patiently(|| {
            // SAFETY: HIDIOCSFEATURE reads the report from `data`, which is as
            // long as its opcode says, and writes nothing past it.
            unsafe { ioctl(&self.node, Updater::<SET_FEATURE, _>::new(&mut data)) }?;
            Ok(())
        })
    }

    /// `report`'s bytes, the rest of the output report zero.
    pub fn output(&mut self, report: &[u8]) -> io::Result<()> {
        let mut data = [0; OUTPUT_LEN + 1];
        data[1..=report.len()].copy_from_slice(report);
        patiently(|| self.node.write_all(&data))
    }

    /// `0d 00 [what]`. Not `09`, which saves to the controller's flash.
    pub fn restart(&mut self, what: Restart) -> io::Result<()> {
        let what = match what {
            Restart::Effects => 0x00,
            Restart::Tables => 0x02,
        };
        self.output(&[0x0d, 0x00, what])
    }

    /// The controller's colour scale: how far each channel goes at full.
    pub fn scale(&mut self, [r, g, b]: [u8; 3]) -> io::Result<()> {
        self.output(&[0x51, 0x00, r, g, b])
    }
}

/// A table slot for `target`, a key's HID usage or a zone: `[colour] [flash
/// colour] [flash ticks] [effect] [mode] 00 [target]`.
pub fn slot(
    color: [u8; 3],
    flash: [u8; 3],
    flash_ticks: u16,
    effect: u8,
    mode: u8,
    target: u8,
) -> [u8; 12] {
    let [r, g, b] = color;
    let [fr, fg, fb] = flash;
    let [ticks_low, ticks_high] = flash_ticks.to_le_bytes();
    [
        r, g, b, fr, fg, fb, ticks_low, ticks_high, effect, mode, 0, target,
    ]
}

/// While the controller sets up an effect, a wave more slowly than the
/// others, it stalls what comes next: EPIPE. A little later it takes it.
fn patiently(mut send: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    const TRIES: u32 = 10;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(20);
    for _ in 1..TRIES {
        match send() {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => std::thread::sleep(PAUSE),
            done => return done,
        }
    }
    send()
}

#[cfg(test)]
pub(crate) mod testing {
    use std::{fs, os::unix::fs::symlink, path::Path};

    /// `class/hidraw/<name>` → its HID device, inside a USB interface.
    pub fn hidraw(sysfs: &Path, name: &str, id: &str, interface: &str) {
        let usb = sysfs.join(format!("devices/usb/{name}-if"));
        let hid = usb.join("hid");
        fs::create_dir_all(&hid).unwrap();
        fs::write(usb.join("bInterfaceNumber"), format!("{interface}\n")).unwrap();
        fs::write(
            hid.join("uevent"),
            format!("DRIVER=hid-generic\nHID_ID={id}\n"),
        )
        .unwrap();
        let class = sysfs.join("class/hidraw").join(name);
        fs::create_dir_all(&class).unwrap();
        symlink(&hid, class.join("device")).unwrap();
    }

    pub fn board(sysfs: &Path, name: &str) {
        let dmi = sysfs.join("class/dmi/id");
        fs::create_dir_all(&dmi).unwrap();
        fs::write(dmi.join("board_name"), format!("{name}\n")).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::hidraw;

    #[test]
    fn finds_the_first_interface() {
        let sysfs = tempfile::tempdir().unwrap();
        hidraw(sysfs.path(), "hidraw0", "0003:0000046D:0000C52B", "00");
        hidraw(sysfs.path(), "hidraw3", "0003:00001038:0000113A", "00");
        hidraw(sysfs.path(), "hidraw4", "0003:00001038:0000113A", "01");
        let dev = Path::new("/dev");
        assert_eq!(find(sysfs.path(), dev, 0x113a), Some(dev.join("hidraw3")));
        assert_eq!(find(sysfs.path(), dev, 0x1122), None);
        assert_eq!(find(Path::new("/nonexistent"), dev, 0x113a), None);
    }

    #[test]
    fn reads_the_board() {
        let sysfs = tempfile::tempdir().unwrap();
        assert_eq!(super::board(sysfs.path()), None);
        testing::board(sysfs.path(), "MS-1824");
        assert_eq!(super::board(sysfs.path()).as_deref(), Some("MS-1824"));
    }

    #[test]
    fn waits_out_a_busy_controller() {
        let mut stalls = 3;
        let mut tries = 0;
        let sent = patiently(|| {
            tries += 1;
            if stalls > 0 {
                stalls -= 1;
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            Ok(())
        });
        assert!(sent.is_ok());
        assert_eq!(tries, 4);
        // Gone for good, or never coming back: said so.
        let mut tries = 0;
        let stalled = patiently(|| {
            tries += 1;
            Err(io::ErrorKind::BrokenPipe.into())
        });
        assert_eq!(stalled.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(tries, 10);
        let mut tries = 0;
        let gone = patiently(|| {
            tries += 1;
            Err(io::ErrorKind::NotFound.into())
        });
        assert!(gone.is_err());
        assert_eq!(tries, 1);
    }

    #[test]
    fn slots_name_their_target_last() {
        assert_eq!(
            slot([1, 2, 3], [4, 5, 6], 0x0102, 7, STILL, 0x04),
            [1, 2, 3, 4, 5, 6, 0x02, 0x01, 7, 1, 0, 0x04]
        );
    }
}
