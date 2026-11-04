//! What the laptop and its system are: the model and BIOS from DMI, the
//! distribution from os-release, the kernel.

use std::{fs, path::Path};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Machine {
    /// The maker, as DMI has it (`sys_vendor`).
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub board: Option<String>,
    pub bios: Option<String>,
    pub bios_date: Option<String>,
    /// The distribution, as it calls itself.
    pub os: Option<String>,
    pub nixos: bool,
    pub kernel: Option<String>,
}

/// This one's.
pub fn this() -> Machine {
    read(Path::new("/"))
}

fn read(root: &Path) -> Machine {
    let line = |path: &str| {
        fs::read_to_string(root.join(path))
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let dmi = |name: &str| line(&format!("sys/class/dmi/id/{name}"));
    let release = fs::read_to_string(root.join("etc/os-release")).unwrap_or_default();
    let os_id = field(&release, "ID");
    Machine {
        vendor: dmi("sys_vendor"),
        model: dmi("product_name"),
        board: dmi("board_name"),
        bios: dmi("bios_version"),
        bios_date: dmi("bios_date"),
        os: field(&release, "PRETTY_NAME").or_else(|| field(&release, "NAME")),
        nixos: os_id.as_deref() == Some("nixos") || root.join("etc/NIXOS").exists(),
        kernel: line("proc/sys/kernel/osrelease"),
    }
}

/// `KEY=value` from os-release, the quotes taken off.
fn field(release: &str, key: &str) -> Option<String> {
    release
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().trim_matches('"').trim_matches('\'').to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_machine() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "sys/class/dmi/id/product_name",
            "Titan 18 HX Dragon Edition Draco Epic A2WJ\n",
        );
        write("sys/class/dmi/id/board_name", "MS-1824\n");
        write(
            "sys/class/dmi/id/sys_vendor",
            "Micro-Star International Co., Ltd.\n",
        );
        write("sys/class/dmi/id/bios_version", "E1824IMS.50E\n");
        write("sys/class/dmi/id/bios_date", "05/26/2026\n");
        write(
            "etc/os-release",
            "NAME=NixOS\nID=nixos\nPRETTY_NAME=\"NixOS 26.11 (Zokor)\"\n",
        );
        write("proc/sys/kernel/osrelease", "7.2.14\n");
        let machine = read(root.path());
        assert_eq!(
            machine.model.as_deref(),
            Some("Titan 18 HX Dragon Edition Draco Epic A2WJ")
        );
        assert_eq!(machine.board.as_deref(), Some("MS-1824"));
        assert_eq!(
            machine.vendor.as_deref(),
            Some("Micro-Star International Co., Ltd.")
        );
        assert_eq!(machine.os.as_deref(), Some("NixOS 26.11 (Zokor)"));
        assert!(machine.nixos);
        assert_eq!(machine.kernel.as_deref(), Some("7.2.14"));

        let other = tempfile::tempdir().unwrap();
        fs::create_dir_all(other.path().join("etc")).unwrap();
        fs::write(
            other.path().join("etc/os-release"),
            "ID=arch\nNAME='Arch Linux'\n",
        )
        .unwrap();
        let arch = read(other.path());
        assert!(!arch.nixos);
        assert_eq!(arch.os.as_deref(), Some("Arch Linux"));
        assert_eq!(arch.model, None);
    }
}
