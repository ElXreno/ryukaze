//! Kernel uevents: the AC adapter coming and going, msi-ec binding, and
//! SteelSeries hidraw devices appearing (the backlight controllers).
//! The same events udev rules would match, read straight from netlink.

use std::{io, thread};

use rustix::{
    io::Errno,
    net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, netlink, recv, socket_with},
};
use ryukaze_core::steelseries;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A Mains supply was added or changed state.
    Mains,
    /// msi-ec bound to its device: the driver was (re)loaded.
    MsiEcBound,
    /// A SteelSeries hidraw device appeared.
    Hidraw,
}

/// Starts a thread that reads the kernel's uevents and sends the ones
/// that matter. It ends when the receiver is dropped.
pub fn listen(events: UnboundedSender<Event>) -> io::Result<()> {
    let socket = socket_with(
        AddressFamily::NETLINK,
        SocketType::DGRAM,
        SocketFlags::CLOEXEC,
        Some(netlink::KOBJECT_UEVENT),
    )?;
    // Group 1 is the kernel's own broadcast; udev re-sends on group 2.
    bind(&socket, &netlink::SocketAddrNetlink::new(0, 1))?;

    thread::Builder::new()
        .name("uevent".into())
        .spawn(move || {
            let mut buf = vec![0; 16 * 1024];
            loop {
                let event = match recv(&socket, &mut buf[..], RecvFlags::empty()) {
                    Ok((len, _)) => parse(&buf[..len]),
                    // The socket overflowed and events were lost: look again.
                    Err(Errno::NOBUFS) => Some(Event::Mains),
                    Err(Errno::INTR) => None,
                    Err(err) => {
                        warn!("uevents stopped: {err}");
                        return;
                    }
                };
                if let Some(event) = event
                    && events.send(event).is_err()
                {
                    return;
                }
            }
        })?;
    Ok(())
}

/// A uevent is `ACTION@DEVPATH` and then `KEY=VALUE` fields, all
/// NUL-terminated.
fn parse(message: &[u8]) -> Option<Event> {
    let mut fields = message.split(|&byte| byte == 0);
    let path = std::str::from_utf8(fields.next()?)
        .ok()?
        .split_once('@')
        .map_or("", |(_, path)| path);
    let (mut action, mut subsystem, mut driver, mut supply_type) = ("", "", "", "");
    for field in fields {
        let Some((key, value)) = std::str::from_utf8(field).ok()?.split_once('=') else {
            continue;
        };
        match key {
            "ACTION" => action = value,
            "SUBSYSTEM" => subsystem = value,
            "DRIVER" => driver = value,
            "POWER_SUPPLY_TYPE" => supply_type = value,
            _ => {}
        }
    }
    match (subsystem, action) {
        ("power_supply", "add" | "change") if supply_type == "Mains" => Some(Event::Mains),
        ("platform", "bind") if driver == "msi-ec" => Some(Event::MsiEcBound),
        ("hidraw", "add") if steelseries_made(path) => Some(Event::Hidraw),
        _ => None,
    }
}

/// Whether a device path runs through a SteelSeries HID device, whose
/// directory is `BUS:VENDOR:PRODUCT.INSTANCE` in hex.
fn steelseries_made(path: &str) -> bool {
    path.split('/').any(|part| {
        let mut ids = part.split(':');
        matches!(
            (ids.next(), ids.next(), ids.next(), ids.next()),
            (Some(bus), Some(vendor), Some(_), None)
                if bus.len() == 4 && u16::from_str_radix(vendor, 16) == Ok(steelseries::VENDOR)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(fields: &[&str]) -> Vec<u8> {
        fields
            .iter()
            .flat_map(|f| [f.as_bytes(), b"\0"].concat())
            .collect()
    }

    #[test]
    fn picks_mains_and_msi_ec() {
        let plug = message(&[
            "change@/devices/LNXSYSTM:00/LNXSYBUS:00/ACPI0003:00/power_supply/ADP1",
            "ACTION=change",
            "DEVPATH=/devices/LNXSYSTM:00/LNXSYBUS:00/ACPI0003:00/power_supply/ADP1",
            "SUBSYSTEM=power_supply",
            "POWER_SUPPLY_NAME=ADP1",
            "POWER_SUPPLY_TYPE=Mains",
            "POWER_SUPPLY_ONLINE=1",
            "SEQNUM=4242",
        ]);
        assert_eq!(parse(&plug), Some(Event::Mains));

        let bind = message(&[
            "bind@/devices/platform/msi-ec",
            "ACTION=bind",
            "DEVPATH=/devices/platform/msi-ec",
            "SUBSYSTEM=platform",
            "DRIVER=msi-ec",
            "MODALIAS=platform:msi-ec",
            "SEQNUM=4243",
        ]);
        assert_eq!(parse(&bind), Some(Event::MsiEcBound));

        let keyboard = message(&[
            "add@/devices/pci0000:00/0000:00:08.1/0000:07:00.0/usb5/5-1/5-1.2/5-1.2:1.0/0003:1038:113A.0004/hidraw/hidraw3",
            "ACTION=add",
            "SUBSYSTEM=hidraw",
            "MAJOR=242",
            "MINOR=3",
            "DEVNAME=hidraw3",
            "SEQNUM=5150",
        ]);
        assert_eq!(parse(&keyboard), Some(Event::Hidraw));
    }

    #[test]
    fn ignores_the_rest() {
        let battery = message(&[
            "change@/devices/LNXSYSTM:00/LNXSYBUS:00/PNP0C0A:00/power_supply/BAT1",
            "ACTION=change",
            "SUBSYSTEM=power_supply",
            "POWER_SUPPLY_TYPE=Battery",
        ]);
        assert_eq!(parse(&battery), None);

        let unbind = message(&[
            "unbind@/devices/platform/msi-ec",
            "ACTION=unbind",
            "SUBSYSTEM=platform",
        ]);
        assert_eq!(parse(&unbind), None);

        let security_key = message(&[
            "add@/devices/pci0000:80/0000:80:14.0/usb1/1-6/1-6:1.0/0003:1050:0407.0021/hidraw/hidraw0",
            "ACTION=add",
            "SUBSYSTEM=hidraw",
            "DEVNAME=hidraw0",
        ]);
        assert_eq!(parse(&security_key), None);
        assert_eq!(parse(b"garbage"), None);
    }
}
