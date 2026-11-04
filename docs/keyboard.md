# The keyboard's backlight

On a controller checked, the SteelSeries KLC (1038:113a) of the Alpha 17
C7VG, or the KLC (1038:1122) of the Titan 18 HX A2WJ with its light bar and
logo, ryukazed lights the keys in profiles: a standard one for each effect
(static, breathe, cycle, wave and reactive, white or the rainbow), the
config's (see the [config](config.md)), and those saved in the window. The
one shown is a scenario's own, from `keyboard.scenarios` or picked under
the profiles in the window, while the EC is in its modes, and otherwise the one picked last, by `SetKeyboardProfile` or
`NextKeyboardProfile`, or else `keyboard.profile`. The tray, the window,
`ryukaze --next-keyboard-profile` and, with a hwdb entry, Fn+F9 pick them
(see [using ryukaze](usage.md#the-fn-keys)); the window's editor makes new
ones (see [using ryukaze](usage.md#the-window)).

## Effects

A profile's effect is one of:

- `static`: the keys still in `color`;
- `breathe`: `color` fading to `fade_to`, black unless told, and back, a
  breath every `period_ms`;
- `cycle`: every key through `colors` together, the rainbow unless told
  (2 to 8 of them), a round every `period_ms`;
- `wave`: `colors` as a wave across the board, out of its `origin`
  (`middle`, the default, `left`, `right`, `top` or `bottom`) or, with
  `direction = "in"`, into it; `wavelength` is how wide a round of the
  colours lies, 1 to 100 percent of the board (25 unless told);
- `reactive`: the keys still in `color`, each flashing `flash` for
  `flash_ms` when pressed and again when let go: the controller has no
  flash that lasts while a key is held (see the
  [hardware notes](hardware.md#the-keyboards-controller)).

The effects are the controller's own: ryukazed sends them, and the
controller runs them by itself, flashes included, so the computer has no
part in them and the daemon never sees a key press. `period_ms` goes from
2000 to 60000, and with more than six colours from 330 a colour;
`flash_ms` goes from 100 to 5000.

## Keys and zones

A profile's `keys` give keys and groups of keys colours of their own, still
over any effect: a single key over a group, a smaller group over a larger
one. The power button takes the effects too, and its own colour or a
zone's.

Its `zones` give keys an effect of their own beside its own: each zone a
list of `keys`, by name or group, and an effect with what it takes, a later
zone over an earlier one. There may be eight. Still colours go in `keys`
instead. The window's editor
makes them as layers over the keyboard's.

Keys: `a` to `z`, `0` to `9`, `f1` to `f12`, `esc`, `tab`, `capslock`,
`enter`, `backspace`, `space`, `minus`, `equal`, `lbracket`, `rbracket`,
`backslash`, `semicolon`, `apostrophe`, `grave`, `comma`, `dot`, `slash`,
`102nd` (the `\` between AltGr and Fn), `prtsc`, `scrolllock`, `insert`,
`delete`, `pageup`, `pagedown` (Home and End share them), `up`, `down`,
`left`, `right`, `numlock`, `kp0` to `kp9`, `kpslash`, `kpasterisk`,
`kpminus`, `kpplus`, `kpenter`, `kpdot`, `lctrl`, `lshift`, `lalt`, `win`,
`ralt`, `rshift`, `fn`, `copilot` (or `rctrl`, where right Ctrl is),
`power`, the power button's light, and beside the keys, where there are,
`bar`, the light bar, and `logo`, the logo on the lid.

Groups: `letters`, `digits`, `function` (F1 to F12), `navigation` (PrtSc,
Scroll Lock, Insert, Delete, PgUp, PgDn), `arrows`, `numpad`, `modifiers`
(Ctrl, Shift, Alt, Win, Copilot, Fn) and `wasd`.

## The picture

The window draws the keyboard from a layout: the Titan 18 HX's for its
board, `MS-1824`, and the Alpha 17 C7VG's for any other laptop. A layout
is a TOML file: its `name`, the DMI board names it's for in `boards`
(`/sys/class/dmi/id/board_name`) and the CHIDs in `chids` (Windows'
HardwareID-9, of `sys_vendor` and `product_name`; with neither, any
laptop's), each key in `keys`
with its HID usage `hid`, its `label` and its box, `x` and `y` from the
top left and `w` and `h`, in key widths, and where there are the
touchpad's box and its rows of light in `[touchpad]`, the deck's box in
`[deck]`, keys and touchpad on it, and in `[logo]` the lid's width to its
height, `lid`, and the logo's box on it as parts of the lid seen from the
keys, drawn when the laptop has them. With a deck, the keys and the
touchpad are drawn on it where they are, and the lid above it, as wide;
without, the touchpad under the keys and the lid over them, close. Keys
are the ones the config names, each once, none over another:

```toml
name = "My laptop"
boards = ["MS-1234"]
keys = [
  { hid = 0x29, label = "Esc", x = 0, y = 0, w = 1, h = 0.6 },
  { hid = 0x3a, label = "F1", x = 1, y = 0, w = 1, h = 0.6 },
]

[touchpad]
x = 6
y = 7.5
w = 7
h = 4.3
rows = 5

[deck]
x = -1.5
y = -1.2
w = 21.4
h = 13.8

[logo]
lid = 1.3
x = 0.1
y = 0.35
w = 0.11
h = 0.2
```

A new laptop takes a file and no rebuild: the window reads the `.toml`
files in `ryukaze/keyboards` under `$XDG_DATA_HOME` (`~/.local/share`),
then under the `share` beside the program's own `bin` (its package's, on
NixOS too), then under each of `$XDG_DATA_DIRS`
(`/usr/local/share:/usr/share`), at
start, and the first for the laptop's board or CHID wins, over the
built-in ones too; the first with no `boards` and no `chids` stands in
for any other. One that doesn't
read or check out is told of on stderr and left out. The built-in ones are
[`data/keyboards`](../data/keyboards), installed to
`share/ryukaze/keyboards`, made from SteelSeries GG's pictures by
[ryukaze-gg-layouts](gg-layouts.md), which makes them for other MSI
laptops too.

## The Titan 18 HX

Its controller runs the same effects, but nothing goes to its flash, as
how its flash takes a save isn't known: at power-on it shows what it
kept, until ryukazed lights it again. Its touchpad, lit from below (its
light bar, `bar`), and the logo on the lid (`logo`) go with the profile,
its effect too, unless `keys` or a zone names them. A wave runs on from
the keys down the touchpad's rows, and reaches the logo as if it lay
above the keys. A reactive profile leaves them still in its `color`: no
key is pressed there.

## The power button

The power button's light goes with the profile, its effect too, unless it
shows something of its own instead (`keyboard.power` in the
[config](config.md), or its choice under the profiles in the window): the
shift mode, a colour each, and if asked the dGPU's colour for five seconds
as it wakes, or whether the dGPU is awake, in colours the window changes
too. Then it holds that colour on an effect of its own, which changes
with an effect on the others running on undisturbed and stays through a
brightness key, within two seconds of what it follows. Back on the
profile, the keyboard is lit again, which starts the button's effect with
the others'.

## Saved profiles

ryukazed keeps the profile picked last and those saved in the window's
editor (`SaveKeyboardProfile`) in `keyboard.toml` in its state directory,
/var/lib/ryukaze (`StateDirectory=` in its unit, or `--state`), as `picked`
and `[[profiles]]` written the way the config has them, and reads them
back as it starts. A saved profile comes after the config's, under a name
of its own: the standard ones and the config's can't be saved over or
deleted (`DeleteKeyboardProfile` takes only the saved), and one saved
before the config took its name waits in the file, unshown.

As the laptop goes off or to sleep, ryukazed saves what the keyboard shows
to the controller's flash, which it shows from power-on until ryukazed
starts. Only then, where the blink that saving brings goes unseen, and
not so often as to wear the flash: not as profiles are picked, nor when
ryukazed is only restarted.

## NumLock

With `numpad_follows_numlock`, or its switch in the window's Devices,
the numpad goes dark while NumLock is off, NumLock itself aside. ryukazed
reads the laptop keyboard's events for it with the kernel holding back all
but its lights' changes (`EVIOCSMASK`), so no key press reaches the daemon
this way either. Going dark leaves an
effect running; coming back on starts it over on every key: the
controller takes keys back into an effect only with the refresh, which
restarts it.

## Colours

The colours go to the LEDs as they are, and they show light shades nearly
white and blue strong: a `#fab387` peach on screen is about `#ff6000`
there.

How the controller takes all this is in the
[hardware notes](hardware.md#the-keyboards-controller).
