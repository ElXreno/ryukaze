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

`ryukaze` shows the CPU and GPU temperatures and fan speeds, the shift and
fan modes, cooler boost, the battery (its charge, what it's doing, how worn
it is), super battery and the charge limit, whichever this EC has, and
polls the daemon every two seconds, so picks made elsewhere (Fn keys, the
power profiles) show up. It renders in software: a GPU renderer would open
Vulkan or GL, which on a hybrid laptop can wake the discrete GPU for a
settings window. It's plain black on white or white on black, as the
desktop prefers (the XDG settings portal's `color-scheme`).

In the advanced fan mode the window shows each fan's curve: drag a dot
sideways for where a step starts, up or down for its speed. It's drawn as
the fans take it, straight between the dots when adaptive, in steps when
the EC goes through it. It's on the fan when you let go, until the daemon
restarts; Copy as Nix puts the curves on the clipboard in the shape of
`services.ryukaze.settings`, to keep them. Adaptive and the GPU fan's help
are switches under it, and "Cooler boost when hot" sits under the cooler
boost switch. Devices has which side of Win the Fn key is on, USB Power
Share, and the camera where its switch does something.

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

# The camera at start (the EC turns it on at power-up), and where Fn sits
# next to Win (the BIOS sets it again from Setup at every boot).
webcam = true
fn_key = "left"

# USB ports powered while the laptop is off (USB Power Share), set at start.
usb_share = false

# Applied when the laptop goes on AC or on battery. A mode switched over
# D-Bus in between stays until the next plug or unplug.
[profiles.ac]
shift_mode = "turbo"
fan_mode = "auto"
super_battery = false

[profiles.battery]
shift_mode = "eco"
super_battery = true

# Cooler boost by itself, in any fan mode: on once the CPU or GPU has
# stayed at its mark for 6 s, off once both have been 10 °C below for
# 10 s. Only a boost it started does it end, and one switched off by hand
# stays off until it has cooled down. Off by default; the marks below.
[cooler_boost]
auto = false
cpu_c = 95
gpu_c = 85

# The advanced fan mode, below. These are the defaults.
[fans]
adaptive = true
gpu_follows_cpu = true

[fans.smoothing]
rise_secs = 10
fall_secs = 60
panic_c = 90

# A fan's curve: dots at (20 °C, speeds[0]) and (temps[i], speeds[i + 1]).
# Temperatures rise, speeds don't fall, 150 % is the most.
[fans.cpu]
temps = [55, 62, 69, 76, 83, 90]
speeds = [0, 30, 35, 40, 50, 60, 100]
```

The daemon learns about the adapter from kernel uevents, and applies
everything again when msi-ec binds (a reload, or a late module load). Over
suspend it keeps logind waiting long enough to note the modes, and puts
them back after resume: through S3 the EC drops advanced, cooler boost and
super battery and reloads its own curves.

## The advanced fan mode

Auto and silent run the fans on the EC's own curves, capped by the shift
mode (eco 43 %, comfort 60 %) and with the two fans running together.
Advanced lifts the caps, lets each fan go its own way, and runs them on
your curves (or the EC's, where you set none), in one of two ways:

- **Adaptive** (the default): ryukazed reads the temperatures every two
  seconds and smooths them, quick on the way up (10 s) and slow on the way
  down (60 s), so a burst of load doesn't send the fans chasing it; from
  90 °C a reading goes straight through. The speed comes off the curve
  straight between its dots, and the EC is held at it, with a step to full
  speed from 90 °C (CPU) or 85 °C (GPU) left in its table in case ryukazed
  isn't there to react. When ryukazed stops it hands the curves back to
  the EC. With `gpu_follows_cpu` (the default) the GPU fan runs at least as
  fast as the CPU's curve asks; otherwise it would stand still whenever the
  dGPU sleeps, as the EC reads its temperature as 0.
- **Stepping**: the curves go into the EC, which steps through them on its
  own temperatures.

The curves, the speeds and the rpm are all in the EC's RAM, which msi-ec
doesn't cover: ryukaze uses ec_sys, and only on EC firmware whose layout
has been checked against the firmware itself, as a wrong address elsewhere
could be anything. So far that's 17KKIMS1.115 (Alpha 17 C7VG), where USB
Power Share (0xBF bit 5) comes from the same map. Only the curve bytes and
that one bit are ever written, and everything is read back; rpm comes from
the tachometer (approximate: the constant is MControlCenter's).

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
| `SetFanCurve` | `s ay ay` | `cpu` or `gpu`, 6 temperatures, 7 speeds: the curve advanced runs on until the daemon restarts |
| `ResetFanCurve` | `s` | back to the configured curve, or the EC's own |
| `SetAdaptiveFans` | `b` | adaptive or stepping in advanced, until the daemon restarts |
| `SetGpuFanFollowsCpu` | `b` | while adaptive, the GPU fan at least at the CPU curve's speed |
| `SetAutoCoolerBoost` | `b` | cooler boost by itself when hot, until the daemon restarts |
| `SetWebcam` | `b` | the camera on or off |
| `SetFnKey` | `s` | `left` or `right` of the Win key |
| `SetUsbShare` | `b` | USB ports powered while the laptop is off |

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

- MSI Alpha 17 C7VG, EC firmware 17KKIMS1.115. msi-ec has the Fn key on
  the wrong side on it, which ryukaze turns round, and its camera bits
  (`webcam`, `webcam_block`) leave the camera working, so ryukaze doesn't
  offer them there.
