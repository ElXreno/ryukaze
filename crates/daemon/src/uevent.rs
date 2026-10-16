//! Kernel uevents: the AC adapter coming and going, and msi-ec binding.
//! The same events udev rules would match, read straight from netlink.

use std::{io, thread};

use rustix::{
    io::Errno,
    net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, netlink, recv, socket_with},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A Mains supply was added or changed state.
    Mains,
    /// msi-ec bound to its device: the driver was (re)loaded.
    MsiEcBound,
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
    let (mut action, mut subsystem, mut driver, mut supply_type) = ("", "", "", "");
    for field in message.split(|&byte| byte == 0).skip(1) {
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
        _ => None,
    }
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
        assert_eq!(parse(b"garbage"), None);
    }
}
