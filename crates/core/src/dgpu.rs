//! The discrete GPU of a hybrid laptop: whether it sleeps, and who holds
//! it open while it doesn't. Read without waking it: the kernel's runtime
//! PM status and /proc, never the device's own registers or config space.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// A display controller that isn't the one the firmware booted on.
pub struct Dgpu {
    device: PathBuf,
    /// Its own device nodes, as /proc shows them: the DRM card and render
    /// node, and NVIDIA's numbered nodes where NVIDIA's driver has it.
    nodes: Vec<String>,
    nvidia: bool,
}

/// PCI class 0x03: display controllers (VGA 0x0300, 3D 0x0302).
fn is_display(device: &Path) -> bool {
    fs::read_to_string(device.join("class")).is_ok_and(|class| class.trim().starts_with("0x03"))
}

/// The dGPU under `sysfs`, on a laptop with two display controllers.
pub fn find(sysfs: &Path) -> Option<Dgpu> {
    let devices = fs::read_dir(sysfs.join("bus/pci/devices")).ok()?;
    let displays: Vec<PathBuf> = devices
        .flatten()
        .map(|entry| entry.path())
        .filter(|device| is_display(device))
        .collect();
    if displays.len() < 2 {
        return None;
    }
    let device = displays.into_iter().find(|device| {
        fs::read_to_string(device.join("boot_vga")).is_ok_and(|boot| boot.trim() == "0")
    })?;
    let nodes = fs::read_dir(device.join("drm"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| format!("/dev/dri/{}", entry.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();
    let nvidia = fs::read_link(device.join("driver"))
        .is_ok_and(|driver| driver.file_name().is_some_and(|name| name == "nvidia"));
    Some(Dgpu {
        device,
        nodes,
        nvidia,
    })
}

impl Dgpu {
    /// Not suspended: running, or on its way in or out.
    pub fn awake(&self) -> io::Result<bool> {
        let status = fs::read_to_string(self.device.join("power/runtime_status"))?;
        Ok(status.trim() != "suspended")
    }

    fn is_node(&self, path: &str) -> bool {
        self.nodes.iter().any(|node| node == path)
            || (self.nvidia
                && path
                    .strip_prefix("/dev/nvidia")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
    }

    /// The names of the processes under `proc` with one of its nodes open,
    /// each once, sorted. Those of other users only as root.
    pub fn users(&self, proc: &Path) -> Vec<String> {
        let Ok(processes) = fs::read_dir(proc) else {
            return Vec::new();
        };
        let mut users: Vec<String> = processes
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().parse::<u32>().is_ok())
            .filter(|entry| {
                fs::read_dir(entry.path().join("fd")).is_ok_and(|fds| {
                    fds.flatten().any(|fd| {
                        fs::read_link(fd.path())
                            .is_ok_and(|target| self.is_node(&target.to_string_lossy()))
                    })
                })
            })
            .filter_map(|entry| fs::read_to_string(entry.path().join("comm")).ok())
            .map(|comm| comm.trim().to_owned())
            .collect();
        users.sort();
        users.dedup();
        users
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// The Alpha 17: the Radeon booted, the GeForce on NVIDIA's driver.
    fn laptop(sysfs: &Path, runtime: &str) {
        for (address, class, boot, drm) in [
            ("0000:06:00.0", "0x030000", "1", ["card0", "renderD128"]),
            ("0000:01:00.0", "0x030000", "0", ["card1", "renderD129"]),
        ] {
            let device = sysfs.join("bus/pci/devices").join(address);
            fs::create_dir_all(device.join("power")).unwrap();
            fs::write(device.join("class"), format!("{class}\n")).unwrap();
            fs::write(device.join("boot_vga"), format!("{boot}\n")).unwrap();
            fs::write(device.join("power/runtime_status"), format!("{runtime}\n")).unwrap();
            for node in drm {
                fs::create_dir_all(device.join("drm").join(node)).unwrap();
            }
        }
        let nvidia = sysfs.join("bus/pci/drivers/nvidia");
        fs::create_dir_all(&nvidia).unwrap();
        symlink(&nvidia, sysfs.join("bus/pci/devices/0000:01:00.0/driver")).unwrap();
        // Not a display: left alone.
        let audio = sysfs.join("bus/pci/devices/0000:01:00.1");
        fs::create_dir_all(&audio).unwrap();
        fs::write(audio.join("class"), "0x040300\n").unwrap();
    }

    fn process(proc: &Path, pid: u32, comm: &str, open: &[&str]) {
        let dir = proc.join(pid.to_string());
        fs::create_dir_all(dir.join("fd")).unwrap();
        fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        for (fd, target) in open.iter().enumerate() {
            symlink(target, dir.join("fd").join(fd.to_string())).unwrap();
        }
    }

    #[test]
    fn finds_the_one_not_booted_on() {
        let sysfs = tempfile::tempdir().unwrap();
        laptop(sysfs.path(), "suspended");
        let dgpu = find(sysfs.path()).unwrap();
        assert!(dgpu.device.ends_with("0000:01:00.0"));
        assert!(!dgpu.awake().unwrap());
        assert!(dgpu.is_node("/dev/dri/renderD129") && dgpu.is_node("/dev/nvidia0"));
        assert!(!dgpu.is_node("/dev/dri/renderD128") && !dgpu.is_node("/dev/nvidiactl"));

        let one = tempfile::tempdir().unwrap();
        fs::create_dir_all(one.path().join("bus/pci/devices")).unwrap();
        assert!(find(one.path()).is_none());
    }

    #[test]
    fn names_who_holds_it() {
        let sysfs = tempfile::tempdir().unwrap();
        laptop(sysfs.path(), "active");
        let dgpu = find(sysfs.path()).unwrap();
        assert!(dgpu.awake().unwrap());

        let proc = tempfile::tempdir().unwrap();
        process(
            proc.path(),
            10,
            "steam",
            &["/dev/nvidia0", "/dev/nvidiactl"],
        );
        process(proc.path(), 11, "steam", &["/dev/dri/renderD129"]);
        process(proc.path(), 12, "firefox", &["/dev/dri/renderD129"]);
        process(
            proc.path(),
            13,
            "niri",
            &["/dev/dri/card0", "/dev/nvidiactl"],
        );
        fs::create_dir_all(proc.path().join("self")).unwrap();
        assert_eq!(dgpu.users(proc.path()), ["firefox", "steam"]);
    }
}
