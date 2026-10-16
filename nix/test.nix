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
          settings = {
            profiles = {
              ac = {
                shift_mode = "turbo";
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

    with subtest("the daemon logs what it couldn't do"):
        machine.succeed("journalctl -u ryukazed.service | grep -q 'msi-ec is not bound'")
  '';
}
