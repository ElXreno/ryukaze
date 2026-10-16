# nix flake check: in a VM without msi-ec, the daemon starts, anyone can read
# its status, and only the listed users get past the bus to the setters.
{ pkgs, self }:

pkgs.testers.runNixOSTest {
  name = "ryukaze";

  nodes = {
    machine = {
      imports = [ self.nixosModules.default ];

      services = {
        ryukaze = {
          enable = true;
          users = [ "alice" ];
          tray = {
            enable = true;
          };
          theme = {
            mode = "dark";
            dark = {
              strong = "#fab387";
            };
          };
          settings = {
            profiles = {
              ac = {
                shift_mode = "turbo";
              };
            };
            fans = {
              cpu = {
                temps = [
                  55
                  62
                  69
                  76
                  83
                  90
                ];
                speeds = [
                  0
                  30
                  35
                  40
                  50
                  60
                  100
                ];
              };
            };
          };
        };
      };

      users = {
        users = {
          alice = {
            isNormalUser = true;
          };
          bob = {
            isNormalUser = true;
          };
        };
      };
    };
  };

  testScript = ''
    call = "busctl call io.github.kontonkara.ryukaze /io/github/kontonkara/ryukaze io.github.kontonkara.ryukaze1"

    machine.wait_for_unit("ryukazed.service")

    with subtest("everyone reads the status"):
        machine.succeed(f"su bob -c '{call} Status'")

    with subtest("the bus turns away users not listed"):
        out = machine.fail(f"su bob -c '{call} SetCoolerBoost b true' 2>&1")
        assert "Access denied" in out, out

    with subtest("listed users reach the daemon"):
        # No msi-ec in the VM, so the daemon itself answers.
        out = machine.fail(f"su alice -c '{call} SetCoolerBoost b true' 2>&1")
        assert "msi-ec doesn't expose it" in out, out

    with subtest("graphical sessions get the tray, in its colours"):
        machine.succeed("grep -q 'ryukaze --hidden' /etc/systemd/user/ryukaze.service")
        machine.succeed("test -e /run/current-system/sw/share/applications/ryukaze.desktop")
        machine.succeed("grep -q 'strong = \"#fab387\"' /etc/xdg/ryukaze/theme.toml")

    with subtest("ec_sys is there for the fans"):
        machine.succeed("grep -q 'options ec_sys write_support=1' /etc/modprobe.d/nixos.conf")
        # No msi-ec here, so no firmware and no layout: the fans are left alone.
        machine.succeed("journalctl -u ryukazed.service | grep -q \"layout isn't known\"")

    with subtest("the daemon logs what it couldn't do"):
        machine.succeed("journalctl -u ryukazed.service | grep -q 'msi-ec is not bound'")
  '';
}
