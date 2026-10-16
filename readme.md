# ryukaze

竜風, "dragon wind": control for MSI laptops on Linux, a from-scratch
rewrite of [MControlCenter](https://github.com/dmitry-s93/MControlCenter)
in Rust.

`ryukazed` is a small root daemon and the only process that touches the
embedded controller. It works through the upstream
[msi-ec](https://github.com/BeardOverflow/msi-ec) driver and serves a
narrow D-Bus API on the system bus. `ryukaze` is a window onto it.

Early days: the daemon reports the EC's state, switches its modes and
follows the power source; the window and the tray icon switch them by
hand.

## The window

`ryukaze` shows the shift and fan modes, cooler boost, super battery and
the charge limit, whichever this EC has, and polls the daemon every two
seconds, so picks made elsewhere (Fn keys, the power profiles) show up. It
renders in software: a GPU renderer would open Vulkan or GL, which on a
hybrid laptop can wake the discrete GPU for a settings window. It's plain
black on white or white on black, as the desktop prefers (the XDG settings
portal's `color-scheme`).

The tray icon has the modes and switches in its menu and opens the window
on a click. `ryukaze --hidden` starts with the tray only, as at login;
starting `ryukaze` again while it runs brings its window up.

## Config

`/etc/ryukaze/config.toml` (or `ryukazed --config <file>`). Every key is
optional, and what's unset the daemon leaves alone:

```toml
# Battery charge end threshold, set at start. The EC keeps it across
# reboots, so removing the line doesn't lift it: set 100.
charge_limit = 80

# Applied when the laptop goes on AC or on battery. A mode switched over
# D-Bus in between stays until the next plug or unplug.
[profiles.ac]
shift_mode = "turbo"
fan_mode = "auto"
super_battery = false

[profiles.battery]
shift_mode = "eco"
super_battery = true
```

The daemon learns about the adapter from kernel uevents, and applies
everything again when msi-ec binds (a reload, or a late module load).

## D-Bus API

`io.github.kontonkara.ryukaze1` at `/io/github/kontonkara/ryukaze`:

| Method | Arguments | |
|---|---|---|
| `Status` | | `a{sv}`; a key is missing when the EC lacks the feature |
| `SetShiftMode` | `s` | one of `shift_modes` |
| `SetFanMode` | `s` | one of `fan_modes` |
| `SetSuperBattery` | `b` | |
| `SetCoolerBoost` | `b` | |
| `SetChargeLimit` | `y` | 10 to 100; charging resumes 10 points below |

Every setter reads the value back and fails if the EC didn't take it.
[data/io.github.kontonkara.ryukaze.conf](data/io.github.kontonkara.ryukaze.conf)
lets everyone call `Status` and only root call the setters.

## NixOS

```nix
{
  inputs.ryukaze.url = "github:kontonkara/ryukaze";

  # in the system configuration:
  imports = [ inputs.ryukaze.nixosModules.default ];
  services.ryukaze = {
    enable = true;
    users = [ "alice" ]; # may change the EC; everyone may read it
    tray.enable = true; # the icon in graphical sessions
    settings = {
      charge_limit = 80;
      profiles.ac.shift_mode = "turbo";
      profiles.battery.shift_mode = "eco";
    };
  };
}
```

The module doesn't install msi-ec: the driver has to be there and know
your EC firmware. `nix flake check` runs the module in a VM.

## Try it

```sh
nix develop
cargo run -p ryukazed -- --session
cargo run -p ryukaze -- --session   # in another terminal
busctl --user call io.github.kontonkara.ryukaze /io/github/kontonkara/ryukaze \
  io.github.kontonkara.ryukaze1 Status
```

`--session` serves on your session bus, so reading needs no root, but
writing to the real `/sys` does. `--sysfs <dir>` points the daemon at a fake
sysfs tree.

## Tested on

- MSI Alpha 17 C7VG, EC firmware 17KKIMS1.115
