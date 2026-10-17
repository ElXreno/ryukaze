//! NumLock, as the keyboard's own light has it, for the numpad's backlight.
//! The daemon reads the keyboard's events with the kernel holding back all
//! but LED changes: no key press is ever queued for it.

use std::{
    fs::{self, File},
    io::{self, Read},
    mem::size_of,
    path::{Path, PathBuf},
    thread,
};

use rustix::ioctl::{Opcode, Setter, ioctl, opcode};
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

const EV_LED: u16 = 0x11;
const LED_NUML: u16 = 0x00;

/// struct input_mask, for EVIOCSMASK: which codes of `kind` to deliver;
/// kind 0 masks the event types themselves.
#[repr(C)]
struct InputMask {
    kind: u32,
    codes_size: u32,
    codes_ptr: u64,
}

const SET_MASK: Opcode = opcode::write::<InputMask>(b'E', 0x93);

/// struct input_event: a struct timeval, then type, code and value.
const TIMEVAL: usize = 2 * size_of::<usize>();
const EVENT: usize = TIMEVAL + 8;

/// NumLock now, and a thread that sends it whenever it changes. The
/// compositor sets the light on every keyboard alike.
pub fn watch(sysfs: &Path, events: UnboundedSender<bool>) -> io::Result<bool> {
    let (light, node) = find(sysfs, Path::new("/dev")).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no keyboard with a NumLock light")
    })?;
    let mut keyboard = File::open(&node)?;
    let mut types: u64 = 1 << EV_LED;
    let mask = InputMask {
        kind: 0,
        codes_size: size_of::<u64>() as u32,
        codes_ptr: (&raw mut types) as usize as u64,
    };
    // SAFETY: EVIOCSMASK reads an input_mask, and from it `types`, which
    // outlives the call; the kernel copies both.
    unsafe { ioctl(&keyboard, Setter::<SET_MASK, _>::new(mask)) }?;
    let on = fs::read_to_string(&light)?.trim() != "0";

    thread::Builder::new()
        .name("numlock".into())
        .spawn(move || {
            let mut event = [0; EVENT];
            loop {
                if let Err(err) = keyboard.read_exact(&mut event) {
                    warn!("NumLock: {}: {err}", node.display());
                    return;
                }
                if let Some(on) = numlock(&event)
                    && events.send(on).is_err()
                {
                    return;
                }
            }
        })?;
    Ok(on)
}

/// The NumLock light's state file and its keyboard's event node: the
/// laptop's own keyboard (on the i8042) if it has one, or else the first.
fn find(sysfs: &Path, dev: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut lights: Vec<PathBuf> = fs::read_dir(sysfs.join("class/leds"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|light| light.to_string_lossy().ends_with("::numlock"))
        .collect();
    lights.sort();
    let own = |light: &&PathBuf| {
        fs::read_to_string(light.join("device/phys"))
            .is_ok_and(|phys| phys.starts_with("isa0060/serio"))
    };
    let light = lights.iter().find(own).or(lights.first())?;
    let event = fs::read_dir(light.join("device"))
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|name| name.starts_with("event"))?;
    Some((light.join("brightness"), dev.join("input").join(event)))
}

/// Whether an event says NumLock went on or off.
fn numlock(event: &[u8; EVENT]) -> Option<bool> {
    let field = |at: usize| u16::from_ne_bytes([event[at], event[at + 1]]);
    let value = i32::from_ne_bytes([
        event[TIMEVAL + 4],
        event[TIMEVAL + 5],
        event[TIMEVAL + 6],
        event[TIMEVAL + 7],
    ]);
    (field(TIMEVAL) == EV_LED && field(TIMEVAL + 2) == LED_NUML).then_some(value != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT] {
        let mut event = [0; EVENT];
        event[TIMEVAL..TIMEVAL + 2].copy_from_slice(&kind.to_ne_bytes());
        event[TIMEVAL + 2..TIMEVAL + 4].copy_from_slice(&code.to_ne_bytes());
        event[TIMEVAL + 4..].copy_from_slice(&value.to_ne_bytes());
        event
    }

    #[test]
    fn reads_numlock_off_the_lights() {
        assert_eq!(numlock(&event(EV_LED, LED_NUML, 1)), Some(true));
        assert_eq!(numlock(&event(EV_LED, LED_NUML, 0)), Some(false));
        // Caps Lock's light, and a key, were they ever let through.
        assert_eq!(numlock(&event(EV_LED, 0x01, 1)), None);
        assert_eq!(numlock(&event(0x01, 0x45, 1)), None);
    }

    /// `class/leds/<input>::numlock` → its input device, with `phys` and
    /// an event node.
    fn keyboard(sysfs: &Path, input: &str, event: &str, phys: &str) {
        let device = sysfs.join("devices").join(input);
        fs::create_dir_all(device.join(event)).unwrap();
        fs::write(device.join("phys"), format!("{phys}\n")).unwrap();
        let light = sysfs.join(format!("class/leds/{input}::numlock"));
        fs::create_dir_all(&light).unwrap();
        fs::write(light.join("brightness"), "0\n").unwrap();
        symlink(&device, light.join("device")).unwrap();
    }

    #[test]
    fn finds_the_laptops_own_keyboard() {
        let sysfs = tempfile::tempdir().unwrap();
        keyboard(
            sysfs.path(),
            "input386",
            "event5",
            "usb-0000:06:00.4-1/input0",
        );
        keyboard(sysfs.path(), "input4", "event4", "isa0060/serio0/input0");
        let (light, node) = find(sysfs.path(), Path::new("/dev")).unwrap();
        assert!(light.ends_with("input4::numlock/brightness"));
        assert_eq!(node, Path::new("/dev/input/event4"));

        // A USB one, without the laptop's.
        let usb = tempfile::tempdir().unwrap();
        keyboard(
            usb.path(),
            "input386",
            "event5",
            "usb-0000:06:00.4-1/input0",
        );
        let (_, node) = find(usb.path(), Path::new("/dev")).unwrap();
        assert_eq!(node, Path::new("/dev/input/event5"));
        assert_eq!(find(Path::new("/nonexistent"), Path::new("/dev")), None);
    }
}
