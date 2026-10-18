//! What the window draws with: the GPU the desktop is on, through Mesa's
//! EGL alone, so that on a hybrid laptop it never wakes the discrete GPU,
//! or the CPU where there's no Mesa. Both go through wgpu and iced's
//! software renderer, which iced falls back to when wgpu finds nothing.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// Where glvnd looks for EGL vendors, NixOS's drivers first.
const VENDORS: [&str; 4] = [
    "/run/opengl-driver/share/glvnd/egl_vendor.d",
    "/etc/glvnd/egl_vendor.d",
    "/usr/share/glvnd/egl_vendor.d",
    "/usr/local/share/glvnd/egl_vendor.d",
];

/// The variables iced, wgpu and glvnd read when the window opens: wgpu's
/// GL on the low-power GPU, through Mesa; another vendor, NVIDIA's, would
/// open its GPU to answer. Left alone if any of them is set already.
pub fn choose() -> Vec<(&'static str, OsString)> {
    let set = [
        "ICED_BACKEND",
        "WGPU_BACKEND",
        "WGPU_POWER_PREF",
        "__EGL_VENDOR_LIBRARY_FILENAMES",
    ];
    if set.iter().any(|var| env::var_os(var).is_some()) {
        return Vec::new();
    }
    match mesa(&VENDORS.map(Path::new)) {
        Some(vendor) => vec![
            ("WGPU_BACKEND", "gl".into()),
            ("WGPU_POWER_PREF", "low".into()),
            ("__EGL_VENDOR_LIBRARY_FILENAMES", vendor.into()),
        ],
        None => vec![("ICED_BACKEND", "tiny-skia".into())],
    }
}

/// Mesa's EGL vendor file, the first of `dirs` that has one.
fn mesa(dirs: &[&Path]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(|entry| Some(entry.ok()?.path()))
            .collect();
        files.sort();
        files
            .into_iter()
            .find(|file| fs::read_to_string(file).is_ok_and(|text| text.contains("libEGL_mesa")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mesa_among_the_vendors() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = |name: &str, library: &str| {
            let text = format!("{{\"ICD\": {{\"library_path\": \"{library}\"}}}}");
            fs::write(dir.path().join(name), text).unwrap();
        };
        let none = tempfile::tempdir().unwrap();
        vendor("10_nvidia.json", "libEGL_nvidia.so.0");
        assert_eq!(mesa(&[none.path(), dir.path()]), None);
        vendor("50_mesa.json", "/nix/store/x-mesa/lib/libEGL_mesa.so.0");
        assert_eq!(
            mesa(&[Path::new("/nowhere"), dir.path()]),
            Some(dir.path().join("50_mesa.json"))
        );
    }
}
