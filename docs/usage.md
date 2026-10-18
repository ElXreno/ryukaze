# Using ryukaze

## The window

`ryukaze` shows the CPU and GPU temperatures and fan speeds, the
scenarios, the shift and fan modes, cooler boost, the battery (its charge,
what it's doing, how worn it is), super battery and the charge limit,
whichever this EC has, and polls the daemon every two seconds, so picks
made elsewhere (Fn keys, the power profiles) show up. It draws on the GPU
the desktop is on, with OpenGL through Mesa alone: any other vendor's
library, NVIDIA's, would open its GPU and wake a sleeping discrete one for
a settings window. Without Mesa it draws in software, as it does when
`ICED_BACKEND=tiny-skia` asks it to; `WGPU_BACKEND`, `WGPU_POWER_PREF` or
`__EGL_VENDOR_LIBRARY_FILENAMES` in the environment leave the choice to
them. It's plain black on white or white on black, as the desktop prefers
(the XDG settings portal's `color-scheme`), or in your
[colours](#colours).

Under each temperature, a graph has it and the fan's duty over the last
ten minutes, and under the battery one has the watts out of it and into
it, read off the same polls, and every ten seconds while only the tray is
up; the pointer over one reads out a moment.

In the advanced fan mode the window shows each fan's curve: drag a dot
sideways for where a step starts, up or down for its speed. It's drawn as
the fans take it, straight between the dots when adaptive, in steps when
the EC goes through it, and it's on the fan when you let go (see
[the advanced fan mode](fans.md)). Adaptive and the GPU fan's help are
switches under it, and "Cooler boost when hot" sits under the cooler boost
switch, with its marks while it's on.

Devices has which side of Win the Fn key is on, USB Power Share, the camera
where its switch does something, and the
[keyboard light's](keyboard.md) profiles. Graphics has the GPU MUX, and on
a hybrid laptop whether the dGPU sleeps, how long, and which processes hold
it open while it doesn't (ryukazed reads the kernel's runtime PM status,
which doesn't wake it, and looks through /proc only while it's up).

What the window changes lasts until the daemon restarts. Copy as Nix, at
the bottom, puts the settings on the clipboard in the shape of
`services.ryukaze.settings`, to keep them.

Edit, by the keyboard light's profiles, opens its editor on the profile
shown, or on what was left there unsaved: a picture of the keyboard, an
effect with its colours, and a colour, from sliders, a hex code or a
swatch, for whichever of them is picked. Keys picked on the picture, by a
click, by dragging over them or by group, take the colour as their own;
with none picked, it's the effect's. What it has shows on the keyboard as
it goes, until another profile is picked or the scenario changes. Save
keeps it with the profiles under its name, over one of the same name,
until the daemon restarts; Copy as Nix puts it on the clipboard as a
profile for `services.ryukaze.settings.keyboard.profiles`, to keep. A
profile's zones it keeps as they are.

## The tray

The tray icon has the scenarios, the modes, the switches, the keyboard
light's profiles and the GPU MUX in its menu, the scenario and the
temperatures in its tooltip, and opens the window on a click.
`ryukaze --hidden` starts with the tray only, as at login; starting
`ryukaze` again while it runs brings its window up.

## The command line

`ryukaze --next-scenario` steps to the next scenario (see the
[config](config.md)), prints its name and exits: for the key MSI Center had
it on, Fn+F7. `ryukaze --next-keyboard-profile` does the same for the
keyboard light's profiles, for the key MSI had lighting on, Fn+F9.

`ryukaze --osd` shows a card at the bottom of the screen for a moment
whenever the modes change, whatever changed them: the scenario among the
others, cooler boost on or off, the mode itself, or the keyboard light's
profile. It needs a compositor with the layer shell, and draws in the
window's colours with iced's software renderer, off screen, onto a surface
of its own: winit has no layer shell, and its software windows no alpha.

`ryukaze --status` prints what the window shows and exits, the current
one of each choice in brackets as sysfs has it (`eco [comfort] turbo`).
With `--json` it prints one JSON object instead, the dictionary `Status`
returns over [D-Bus](dbus.md) (crates/core/src/status.rs says what each key
is), for scripts and bars: `ryukaze --status --json | jq .cpu_temp`.

`--session` talks to a daemon on the session bus instead, for
[development](development.md).

## The Fn keys

Fn+F7 reaches the desktop as `XF86Tools`. On the Alpha 17, Fn+F9 sends a
scancode the kernel has no key for (`e016`, in the kernel log as an
unknown key); this hwdb entry makes it the keyboard light key,
`XF86KbdLightOnOff`:

```
evdev:atkbd:dmi:bvn*:bvr*:bd*:svnMicro-Star*:pnAlpha17C7V*:*
 KEYBOARD_KEY_96=kbdillumtoggle
```

A session that was running when the entry came in only sees the key from
the next login: libinput reads a keyboard's keys once. Then, in niri:

```kdl
binds {
    XF86Tools { spawn "ryukaze" "--next-scenario"; }
    XF86KbdLightOnOff { spawn "ryukaze" "--next-keyboard-profile"; }
}
```

Fn+F8, cooler boost, the EC switches by itself; the window, the tray and
the card follow it.

## power-profiles-daemon

Where power-profiles-daemon runs, its profile goes with the shift mode
(see the [config](config.md)), so a desktop's power menu (GNOME's, KDE's,
Noctalia's) picks scenarios too, and shows the one Fn+F7 picked.

## Colours

`ryukaze/theme.toml`, the first found in the XDG config dirs
(`~/.config`, then `/etc/xdg`), is read at start; the NixOS module writes
`/etc/xdg/ryukaze/theme.toml` from `services.ryukaze.theme`. `mode` fixes
the window light or dark whatever the desktop prefers, `font` names a font
family the system has, and `[light]` and `[dark]` take any of these colours
over the built-in ones, as `#rrggbb` or `#rrggbbaa`:

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
