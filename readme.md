# ryukaze

竜風, "dragon wind": control for MSI laptops on Linux, an alternative to
[MControlCenter](https://github.com/dmitry-s93/MControlCenter) written
from scratch in Rust.

`ryukazed` is a small root daemon and the only process that touches the
embedded controller. It works through the upstream
[msi-ec](https://github.com/BeardOverflow/msi-ec) driver and serves a
narrow D-Bus API on the system bus. `ryukaze` is a window onto it.

Early days: the daemon reports the EC's state, switches its modes and
follows the power source; the window and the tray icon switch them by
hand.

## The window

`ryukaze` shows the CPU and GPU temperatures and fan speeds, the
scenarios, the shift and fan modes, cooler boost, the battery (its charge,
what it's doing, how worn it is), super battery and the charge limit,
whichever this EC has, and polls the daemon every two seconds, so picks
made elsewhere (Fn keys, the power profiles) show up. It renders in
software: a GPU renderer would open Vulkan or GL, which on a hybrid laptop
can wake the discrete GPU for a settings window. It's plain black on white
or white on black, as the desktop prefers (the XDG settings portal's
`color-scheme`), or in your colours (below).

Under each temperature, a graph has it and the fan's duty over the last
ten minutes, and under the battery one has the watts out of it and into
it, read off the same polls, and every ten seconds while only the tray is
up; the pointer over one reads out a moment.

In the advanced fan mode the window shows each fan's curve: drag a dot
sideways for where a step starts, up or down for its speed. It's drawn as
the fans take it, straight between the dots when adaptive, in steps when
the EC goes through it, and it's on the fan when you let go. Adaptive and
the GPU fan's help are switches under it, and "Cooler boost when hot"
sits under the cooler boost switch, with its marks while it's on. Devices
has which side of Win the Fn key is on, USB Power Share, and the camera
where its switch does something; Graphics has the GPU MUX, and on a
hybrid laptop whether the dGPU sleeps, how long, and which processes hold
it open while it doesn't (ryukazed reads the kernel's runtime PM status,
which doesn't wake it, and looks through /proc only while it's up).

What the window changes lasts until the daemon restarts. Copy as Nix, at
the bottom, puts the settings on the clipboard in the shape of
`services.ryukaze.settings`, to keep them.

The tray icon has the scenarios, the modes, the switches and the GPU MUX
in its menu, the scenario and the temperatures in its tooltip, and opens
the window on a click. `ryukaze --hidden` starts with the tray only, as at
login; starting `ryukaze` again while it runs brings its window up.

`ryukaze --next-scenario` steps to the next scenario (see the config),
prints its name and exits: for the key MSI Center had it on, Fn+F7, which
reaches the desktop as `XF86Tools`. In niri:

```kdl
binds {
    XF86Tools { spawn "ryukaze" "--next-scenario"; }
}
```

Where power-profiles-daemon runs, its profile goes with the shift mode
(see the config), so a desktop's power menu (GNOME's, KDE's, Noctalia's)
picks scenarios too, and shows the one Fn+F7 picked.

`ryukaze --osd` shows a card at the bottom of the screen for a moment
whenever the modes change, whatever changed them: the scenario among the
others, cooler boost on or off, or the mode itself. It needs a compositor
with the layer shell, and draws in the window's colours with iced's
software renderer, off screen, onto a surface of its own: winit has no
layer shell, and its software windows no alpha.

`ryukaze --status` prints what the window shows and exits, the current
one of each choice in brackets as sysfs has it (`eco [comfort] turbo`).
With `--json` it prints one JSON object instead, the dictionary `Status`
returns over D-Bus (crates/core/src/status.rs says what each key is), for
scripts and bars: `ryukaze --status --json | jq .cpu_temp`.

### Colours

`ryukaze/theme.toml`, the first found in the XDG config dirs
(`~/.config`, then `/etc/xdg`), is read at start. `mode` fixes the window
light or dark whatever the desktop prefers, `font` names a font family
the system has, and `[light]` and `[dark]` take any of these colours over
the built-in ones, as `#rrggbb` or `#rrggbbaa`:

```toml
mode = "dark"
font = "Inter"

[dark]
background = "#11111b" # behind everything
card = "#1e1e2e"       # the panels
border = "#313244"
text = "#cdd6f4"
muted = "#a6adc8"      # titles, hints
idle = "#313244"       # a choice not taken
hover = "#45475a"
strong = "#fab387"     # the choice taken, a switch on, a filled bar
on_strong = "#1e1e2e"  # text on it
track = "#45475a"      # an empty bar, a switch off
track_hover = "#585b70"
knob = "#a6adc8"       # a switch's knob while off
danger = "#f38ba8"     # what went wrong
```

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

# Applied when the laptop goes on AC or on battery: a scenario by name
# (below), the modes themselves, or both, the modes over the scenario's. A
# mode switched over D-Bus in between stays until the next plug or unplug.
[profiles.ac]
scenario = "games"

[profiles.battery]
shift_mode = "eco"
super_battery = true

# What NextScenario steps through, in this order: from the one whose modes
# the EC is in, to the first if it's in none. Each sets any of the three
# above. Without any: silent (eco, silent fans), balanced (comfort, auto)
# and performance (turbo, auto), super battery off, those this EC has.
[[scenarios]]
name = "quiet"
shift_mode = "eco"
fan_mode = "silent"

[[scenarios]]
name = "games"
shift_mode = "turbo"
fan_mode = "advanced"
super_battery = false

# Cooler boost by itself, in any fan mode: on once the CPU or GPU has
# stayed at its mark for 6 s, off once both have been 10 °C below for
# 10 s. Only a boost it started does it end, and one switched off by hand
# stays off until it has cooled down. Off by default; the marks below,
# from 60 to 100 °C.
[cooler_boost]
auto = false
cpu_c = 95
gpu_c = 85

# power-profiles-daemon's profile and the shift mode in step, both ways:
# eco is power-saver, comfort balanced, sport and turbo performance. A
# profile picked there (a desktop's power menu) applies the first scenario
# with a shift mode for it, or the mode alone. On by default; without ppd
# on the bus it does nothing.
[power_profiles_daemon]
sync = true

# The keyboard's backlight (see below), every key alike: #rrggbb, or off.
# Set at start, after sleep and when its controller comes back.
[keyboard]
color = "#ff6000"

# Keys and groups of keys in their own colours, over that one: a single key
# over a group, a smaller group over a larger one. Without a colour for all,
# the other keys keep theirs.
[keyboard.keys]
wasd = "#ff0000"
arrows = "#ffffff"

# The colour for all while the EC is in a scenario's modes, over the one
# above: the keyboard shows the scenario, whatever picked it.
[keyboard.scenarios]
quiet = "#0040ff"
games = "#ff0000"

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
mode (eco 43 %, comfort 60 %) and with the two fans running together: each
fan's register still holds what its own curve asks (0 % from a cool GPU),
so ryukaze reports the higher of the two for both.
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
Power Share (0xBF bit 5) and the GPU MUX (below) come from the same map.
Only the curve bytes, that bit and the MUX's request are ever written, and
everything is read back; rpm comes from the tachometer (approximate: the
constant is MControlCenter's).

## The GPU MUX

In hybrid the panel hangs off the iGPU, and the dGPU renders through it
when asked and sleeps otherwise; in discrete it hangs off the dGPU, which
stays awake. The switch takes a restart. On 17KKIMS1.115, as the firmware
does it: ryukazed sets 0xD1 bit 0, the EC passes that on through ACPI
(`_Q90`) to the BIOS, which flips the mode it keeps in CMOS and answers
in 0xD1 (bit 1, and bit 2 for discrete); at power-off the EC copies the
answer to 0x2E bit 6, which it sets the MUX from at power-on, and at the
next boot the BIOS brings its Setup in line and restarts once more by
itself. The BIOS flips rather than sets, so ryukazed asks only when the
mode after a restart isn't already the one wanted, and waits for the
answer. It doesn't write the `MsiDCVarData` EFI variable, as other MUX
switchers do: the firmware reads that one for the Win/Fn swap, not for
the MUX.

## The keyboard's backlight

On a controller checked, the SteelSeries KLC (1038:113a) of the Alpha 17
C7VG, ryukazed colours the keys: all of them with `keyboard.color`, or
with a scenario's colour from `keyboard.scenarios` while the EC is in its
modes, and some by name with `keyboard.keys`. A colour set over D-Bus
lasts until the scenario changes. The colour goes to the LEDs as it is,
and they show light shades nearly white and blue strong: a `#fab387`
peach on screen is about `#ff6000` there.

Keys: `a` to `z`, `0` to `9`, `f1` to `f12`, `esc`, `tab`, `capslock`,
`enter`, `backspace`, `space`, `minus`, `equal`, `lbracket`, `rbracket`,
`backslash`, `semicolon`, `apostrophe`, `grave`, `comma`, `dot`, `slash`,
`102nd` (the `\` between AltGr and Fn), `prtsc`, `scrolllock`, `insert`,
`delete`, `pageup`, `pagedown` (Home and End share them), `up`, `down`,
`left`, `right`, `numlock`, `kp0` to `kp9`, `kpslash`, `kpasterisk`,
`kpminus`, `kpplus`, `kpenter`, `kpdot`, `lctrl`, `lshift`, `lalt`, `win`,
`ralt`, `rshift`, `fn`, `copilot`, and `power`, the power button's light.

Groups: `letters`, `digits`, `function` (F1 to F12), `navigation` (PrtSc,
Scroll Lock, Insert, Delete, PgUp, PgDn), `arrows`, `numpad`, `modifiers`
(Ctrl, Shift, Alt, Win, Copilot, Fn) and `wasd`.

The controller takes the colours from a HID feature report on its first
interface: `0c 00 66 00`, then (key, red, green, blue) for up to about a
hundred keys, each key its HID usage. A report changes the keys it lists
and leaves the others, so all of them take two.

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
| `SetCoolerBoostMarks` | `y y` | the CPU and GPU °C (60 to 100) it comes on at, until the daemon restarts |
| `SetWebcam` | `b` | the camera on or off |
| `SetFnKey` | `s` | `left` or `right` of the Win key |
| `SetUsbShare` | `b` | USB ports powered while the laptop is off |
| `SetKeyboardColor` | `s` | the keyboard's backlight, `#rrggbb` or `off` for every key, until the daemon restarts |
| `SetGpuMode` | `s` | `hybrid` or `discrete`, from the next restart |
| `SetScenario` | `s` | one of `scenarios` |
| `NextScenario` | | applies the next scenario; returns its name (`s`) |

Every setter reads the value back and fails if the EC didn't take it.
The signal `ModesChanged` (`a{sv}`) carries `shift_mode`, `fan_mode`,
`super_battery`, `cooler_boost` and `scenario`, as in `Status`, whenever
they change, whatever changed them: a call (at once), a power profile, the
automatic boost or the EC itself on Fn+F8 (within two seconds). The window
and the tray take it as it comes.
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
    osd.enable = true; # a card on screen when the modes change
    settings = {
      charge_limit = 80;
      profiles.ac.scenario = "performance";
      profiles.battery.scenario = "silent";
    };
  };
}
```

The module doesn't install msi-ec: the driver has to be there and know
your EC firmware. `nix flake check` runs the module in a VM; CI runs it on
every push, with rustfmt, nixfmt and clippy.

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
  offer them there. Its keyboard's backlight is a SteelSeries KLC
  (1038:113a).
