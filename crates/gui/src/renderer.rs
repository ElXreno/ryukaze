//! What the window draws with: the integrated GPU, through Mesa's Vulkan
//! driver for it alone, so that on a hybrid laptop it never wakes the
//! discrete GPU; or the CPU where that can't be had. iced falls back to its
//! software renderer, too, when wgpu finds nothing.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// Where the Vulkan loader finds drivers, NixOS's first.
const DRIVER_DIRS: [&str; 4] = [
    "/run/opengl-driver/share/vulkan/icd.d",
    "/etc/vulkan/icd.d",
    "/usr/share/vulkan/icd.d",
    "/usr/local/share/vulkan/icd.d",
];

/// Mesa's Vulkan drivers for integrated GPUs, by the PCI vendor whose GPUs
/// each opens as it starts: every one of them, a sleeping one too.
const DRIVERS: [(&str, &str); 2] = [
    ("libvulkan_radeon", "0x1002"),
    ("libvulkan_intel", "0x8086"),
];

/// The variables iced, wgpu and the Vulkan loader read when the window
/// opens: wgpu's Vulkan on the low-power GPU, through Mesa's driver for a
/// vendor with only that GPU here, and no implicit layers, NVIDIA's among
/// them. Left alone if any of them is set already.
pub fn choose() -> Vec<(&'static str, OsString)> {
    let set = [
        "ICED_BACKEND",
        "WGPU_BACKEND",
        "WGPU_POWER_PREF",
        "VK_DRIVER_FILES",
        "VK_ICD_FILENAMES",
    ];
    if set.iter().any(|var| env::var_os(var).is_some()) {
        return Vec::new();
    }
    let gpus = gpus(Path::new("/sys/bus/pci/devices"));
    let drivers = drivers(&DRIVER_DIRS.map(Path::new), &gpus);
    let files = match env::join_paths(&drivers) {
        Ok(files) if !drivers.is_empty() => files,
        _ => return vec![("ICED_BACKEND", "tiny-skia".into())],
    };
    vec![
        ("WGPU_BACKEND", "vulkan".into()),
        ("WGPU_POWER_PREF", "low".into()),
        ("VK_DRIVER_FILES", files.clone()),
        // What loaders before 1.3.207 read.
        ("VK_ICD_FILENAMES", files),
        ("VK_LOADER_LAYERS_DISABLE", "~implicit~".into()),
    ]
}

/// The PCI vendor of each display controller in `pci`.
fn gpus(pci: &Path) -> Vec<String> {
    let Ok(devices) = fs::read_dir(pci) else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let device = device.ok()?.path();
            let class = fs::read_to_string(device.join("class")).ok()?;
            if !class.trim().starts_with("0x03") {
                return None;
            }
            let vendor = fs::read_to_string(device.join("vendor")).ok()?;
            Some(vendor.trim().to_owned())
        })
        .collect()
}

/// The driver files in the first of `dirs` with any, for vendors with a
/// single GPU among `gpus`.
fn drivers(dirs: &[&Path], gpus: &[String]) -> Vec<PathBuf> {
    let alone = |vendor: &str| gpus.iter().filter(|gpu| *gpu == vendor).count() == 1;
    dirs.iter()
        .map(|dir| {
            let Ok(entries) = fs::read_dir(dir) else {
                return Vec::new();
            };
            let mut files: Vec<PathBuf> = entries
                .filter_map(|entry| Some(entry.ok()?.path()))
                .filter(|file| {
                    let text = fs::read_to_string(file).unwrap_or_default();
                    DRIVERS
                        .iter()
                        .any(|(library, vendor)| text.contains(library) && alone(vendor))
                })
                .collect();
            files.sort();
            files
        })
        .find(|files| !files.is_empty())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_mesa_for_a_vendor_with_one_gpu() {
        let dir = tempfile::tempdir().unwrap();
        let driver = |name: &str, library: &str| {
            let text =
                format!("{{\"ICD\": {{\"library_path\": \"/nix/store/x/lib/{library}.so\"}}}}");
            fs::write(dir.path().join(name), text).unwrap();
        };
        driver("nvidia_icd.json", "libGLX_nvidia");
        driver("lvp_icd.x86_64.json", "libvulkan_lvp");
        let empty = tempfile::tempdir().unwrap();
        let dirs = [Path::new("/nowhere"), empty.path(), dir.path()];
        let nvidia_and_amd = ["0x10de".to_owned(), "0x1002".to_owned()];
        assert!(drivers(&dirs, &nvidia_and_amd).is_empty());

        driver("radeon_icd.x86_64.json", "libvulkan_radeon");
        assert_eq!(
            drivers(&dirs, &nvidia_and_amd),
            [dir.path().join("radeon_icd.x86_64.json")]
        );
        // Two AMD GPUs: radv would open the sleeping one too.
        assert!(drivers(&dirs, &["0x1002".to_owned(), "0x1002".to_owned()]).is_empty());
    }

    #[test]
    fn finds_the_display_controllers() {
        let pci = tempfile::tempdir().unwrap();
        for (slot, class, vendor) in [
            ("0000:01:00.0", "0x030000", "0x10de"),
            ("0000:01:00.1", "0x040300", "0x10de"),
            ("0000:06:00.0", "0x030000", "0x1002"),
        ] {
            let device = pci.path().join(slot);
            fs::create_dir(&device).unwrap();
            fs::write(device.join("class"), format!("{class}\n")).unwrap();
            fs::write(device.join("vendor"), format!("{vendor}\n")).unwrap();
        }
        let mut found = gpus(pci.path());
        found.sort();
        assert_eq!(found, ["0x1002", "0x10de"]);
    }
}
