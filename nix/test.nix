# nix flake check: in a VM without msi-ec, the daemon starts, anyone can read
# its status, and only the listed users get past the bus to the setters.
{ pkgs, self }:

pkgs.testers.runNixOSTest {
  name = "ryukaze";

  nodes = {
    machine = {
      imports = [ self.nixosModules.default ];

      services = {
        # Kept in step with the shift mode; without msi-ec there is none.
        power-profiles-daemon = {
          enable = true;
        };
        ryukaze = {
          enable = true;
          users = [ "alice" ];
          tray = {
            enable = true;
          };
          osd = {
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

      # ppd asks polkit before it switches profiles, root included.
      security = {
        polkit = {
          enable = true;
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
    import json

    call = "busctl call io.github.kontonkara.ryukaze /io/github/kontonkara/ryukaze io.github.kontonkara.ryukaze1"

    machine.wait_for_unit("ryukazed.service")

    with subtest("everyone reads the status"):
        machine.succeed(f"su bob -c '{call} Status'")
        machine.succeed("su bob -c 'ryukaze --status'")
        status = json.loads(machine.succeed("su bob -c 'ryukaze --status --json'"))
        # No msi-ec, so no modes.
        assert "shift_mode" not in status, status

    with subtest("the bus turns away users not listed"):
        out = machine.fail(f"su bob -c '{call} SetCoolerBoost b true' 2>&1")
        assert "Access denied" in out, out

    with subtest("listed users reach the daemon"):
        # No msi-ec in the VM, so the daemon itself answers.
        out = machine.fail(f"su alice -c '{call} SetCoolerBoost b true' 2>&1")
        assert "msi-ec doesn't expose it" in out, out
        # Nor a keyboard backlight it knows.
        out = machine.fail(f"su alice -c '{call} SetKeyboardColor s off' 2>&1")
        assert "no backlight controller" in out, out

    with subtest("the key binding's command reaches the daemon"):
        # Without msi-ec there are no modes, so no built-in scenarios.
        out = machine.fail("su alice -c 'ryukaze --next-scenario' 2>&1")
        assert "lacks the built-in ones' modes" in out, out

    with subtest("graphical sessions get the tray and the OSD, in its colours"):
        machine.succeed("grep -q 'ryukaze --hidden' /etc/systemd/user/ryukaze.service")
        machine.succeed("grep -q 'ryukaze --osd' /etc/systemd/user/ryukaze-osd.service")
        machine.succeed("test -e /run/current-system/sw/share/applications/ryukaze.desktop")
        machine.succeed("grep -q 'strong = \"#fab387\"' /etc/xdg/ryukaze/theme.toml")

    with subtest("ec_sys is there for the fans"):
        machine.succeed("grep -q 'options ec_sys write_support=1' /etc/modprobe.d/nixos.conf")
        # No msi-ec here, so no firmware and no layout: the fans are left alone.
        machine.succeed("journalctl -u ryukazed.service | grep -q \"layout isn't known\"")

    with subtest("the daemon logs what it couldn't do"):
        machine.succeed("journalctl -u ryukazed.service | grep -q 'msi-ec is not bound'")

    with subtest("a power profile picked in ppd reaches the daemon"):
        machine.wait_until_succeeds("journalctl -u ryukazed.service | grep -q 'in step with power-profiles-daemon'")
        machine.succeed("powerprofilesctl set power-saver")
        # It tries the shift mode for it; there is no msi-ec to take it.
        machine.wait_until_succeeds("journalctl -u ryukazed.service | grep -q 'power-profiles-daemon: power-saver: '")
        # ppd says what it starts at to the daemon that woke it: no news.
        machine.fail("journalctl -u ryukazed.service | grep -q 'power-profiles-daemon: balanced'")
  '';
}
