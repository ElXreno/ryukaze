# The keyboard's backlight

On a controller checked, the SteelSeries KLC (1038:113a) of the Alpha 17
C7VG, ryukazed lights the keys in profiles: a standard one for each effect
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
`ralt`, `rshift`, `fn`, `copilot`, and `power`, the power button's light.

Groups: `letters`, `digits`, `function` (F1 to F12), `navigation` (PrtSc,
Scroll Lock, Insert, Delete, PgUp, PgDn), `arrows`, `numpad`, `modifiers`
(Ctrl, Shift, Alt, Win, Copilot, Fn) and `wasd`.

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

## NumLock

With `numpad_follows_numlock`, or its switch in the window's Devices,
the numpad goes dark while NumLock is off, NumLock itself aside. ryukazed
reads the laptop keyboard's events for it with the kernel holding back all
but its lights' changes (`EVIOCSMASK`), so no key press reaches the daemon
this way either. Going dark leaves an
effect running; coming back on starts it over on every key, a blink: the
controller takes keys back into an effect only with the refresh, which
restarts it.

## Colours

The colours go to the LEDs as they are, and they show light shades nearly
white and blue strong: a `#fab387` peach on screen is about `#ff6000`
there.

How the controller takes all this is in the
[hardware notes](hardware.md#the-keyboards-controller).
