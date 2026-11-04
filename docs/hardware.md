# Hardware notes

## Tested on

- MSI Alpha 17 C7VG, EC firmware 17KKIMS1.115. msi-ec has the Fn key on
  the wrong side on it, which ryukaze turns round, and its camera bits
  (`webcam`, `webcam_block`) leave the camera working, so ryukaze doesn't
  offer them there. Its keyboard's backlight is a SteelSeries KLC
  (1038:113a), and Fn+F9 needs a hwdb entry (see
  [using ryukaze](usage.md#the-fn-keys)).
- MSI Titan 18 HX A2WJ (MS-1824), EC firmware 1824EMS1.509: the fan
  curves and rpm, [the GPU MUX](#the-gpu-mux), [Apex](#apex),
  [local dimming](#local-dimming), and the keyboard's light with its
  effects, zones and the power button's own colour, the touchpad's light
  and the logo too (see [the keyboard's controller](#the-keyboards-controller)).

## The EC's RAM

The fan curves, the speeds and the rpm are all in the EC's RAM, which
msi-ec doesn't cover: ryukaze uses ec_sys, and only on EC firmware whose
layout has been checked, as a wrong address elsewhere could be anything.
So far that's 17KKIMS1.115 (Alpha 17 C7VG), against the firmware itself,
where USB Power Share (0xBF bit 5) and the GPU MUX (below) come from the
same map; and 1824EMS1.509 (Titan 18 HX), live against its EC RAM and
msi-wmi-platform's fan readings, with Apex and local dimming. Only the
curve bytes, those bits and the MUX's request are ever written, and
everything is read back; rpm comes from the tachometer (approximate: the
constant is MControlCenter's on the Alpha, msi-wmi-platform's on the
Titan).

## The GPU MUX

In hybrid the panel hangs off the iGPU, and the dGPU renders through it
when asked and sleeps otherwise; in discrete it hangs off the dGPU, which
stays awake; in integrated the dGPU is off altogether, and the outputs
wired to it with it. The switch takes a restart. On 17KKIMS1.115, as the
firmware does it: ryukazed sets 0xD1 bit 0, the EC passes that on through
ACPI (`_Q90`) to the BIOS, which flips the mode it keeps in CMOS and
answers in 0xD1 (bit 1, and bit 2 for discrete); at power-off the EC
copies the answer to 0x2E bit 6, which it sets the MUX from at power-on,
and at the next boot the BIOS brings its Setup in line and restarts once
more by itself. The BIOS flips rather than sets, so ryukazed asks only
when the mode after a restart isn't already the one wanted, and waits for
the answer. It doesn't write the `MsiDCVarData` EFI variable there: that
firmware reads it for the Win/Fn swap, not for the MUX.

On 1824EMS1.509 the ask goes the same way, `_Q90` raising software SMI
0x11, but the BIOS's handler for it (`MsiSGSmm` in E1824IMS.50E) takes the
mode from bits 0 and 1 of `MsiDCVarData`'s data byte 5 (0 hybrid,
1 discrete, 2 integrated), keeps it, and answers as above; asked for the
mode it keeps already, it changes nothing. So ryukazed writes the mode
there first, the rest of the variable as it was, and takes it back if no
answer comes. The same byte has the mode set at power-on in bits 2 and 3,
and, as MSI Center reads them, says the BIOS works this way in bit 4,
that it has integrated in bit 5 and that it lacks discrete in bit 6:
ryukazed offers the modes it has, and leaves the MUX out without bit 4.
A plain restart is enough.

## The BIOS's turbo

The BIOS may start the laptop in turbo as 0xC0 in the shift mode byte,
where MSI Center and msi-ec write 0xC4; msi-ec reads it as `unknown
(192)`, and the Alpha's ACPI picks the dGPU's limits by 0xC1, 0xC2 and
0xC4 alone. ryukazed writes turbo again whenever it reads 0xC0, so the
modes, the scenario and power-profiles-daemon say turbo.

## Apex

MSI Center's Apex is one more value of the shift mode byte, which msi-ec
doesn't know and reads back as `unknown (197)`. On the Titan 18 HX A2WJ
(1824EMS1.509) MSI Center writes 0xC5 into 0xD2, where turbo is 0xC4, and
nothing else. The EC then puts 7 into the mode byte ACPI reads (`OEV1`,
at 0xFC000701) instead of turbo's 4, and the ACPI table it picks for the
dGPU (NVPCF) raises its limit from 150 to 175 W. MSI has it on AC only,
and so does ryukazed: off AC it takes turbo instead, and doesn't offer it.

The Titan's EC (an ENE one, its ACPI space at 0xF400 of its XRAM) ties
super battery to the shift mode where the Alpha's doesn't: going into eco
turns super battery on, and any other mode turns it off (its handler for
the shift mode byte, read from E1824IMS.50E's EC image).

## Local dimming

The Titan 18 HX's mini-LED panel dims its backlight zones behind dark parts
of the picture only when the EC says so. On 1824EMS1.509 MSI Center's
switch for it (`SDRpin`, `HDRpin` with Windows' HDR on) is bit 2 of 0xD0,
and bit 1 of the same byte is set where the panel has zones; ryukazed shows
the switch only then. The TCON's own registers over the DisplayPort AUX
channel (Intel's segmented backlight bit) change nothing without it. Dark
tinted backgrounds lose some of their colour with it on.

## The keyboard's controller

Still colours go in a HID feature report on the controller's first
interface: `0c 00 66 00`, then (key, red, green, blue) for up to about a
hundred keys, each key its HID usage. A report changes the keys it lists
and leaves the others.

SteelSeries GG's description of the controller (`ss-klc-base`, where this
one is "KLC496", 1038:113a, its bootloader 113b) names the commands. GG
encrypts its device descriptions; they were read for this and aren't
kept here. Commands go on the output report and answer on the input
report: `10 00` reads the firmware version (minor, then major: 1.14 on the
Alpha 17), `0d 00 00` restarts the lighting, and `09 00` saves it to the
controller's flash, which ryukazed sends only as the laptop goes off or to
sleep, so it shows from power-on. There's no command to read
the keyboard light's brightness: the EC passes the brightness keys on to
the controller, which keeps the level to itself (the EC sends it every
key's state over SPI, the brightness keys as F13 and F14).

The effects go in the older protocol msi-perkeyrgb found on earlier MSI
laptops: a `0b` report sets out an effect as a start colour and up to
sixteen steps (each channel's change per 10 ms tick, in sixteenths, and
the step's length in ticks), and a wave if it has one, which the
controller runs round and round, from the start colour each time.
`0e 00 [slots] 00` reports put up to 42 keys each on an effect, still,
following it or flashing when pressed, a slot a key, and a restart
(`0d 00 00`) starts them: msi-perkeyrgb, and ryukaze until 2026-10-19,
sent `09` instead, which does it too but saves to flash as well and blinks
the whole keyboard. A still report then stops the keys it lists. While it sets up an effect the
controller may turn the next report down (EPIPE); ryukazed waits a little
and sends it again.

On a brightness key the controller draws every key again from the
tables, and what a still report alone painted goes: a still
profile over a reactive one, say, comes back reactive. So ryukazed keeps
still keys in the tables too: a still profile's, a key's own, the numpad
dark for NumLock and the power button's own (see [Restarts](#restarts)
for how).

The tables' third byte is how many slots the controller reads, not a
region: msi-perkeyrgb's four lists of keys, with it as their numbers,
happen to fit. Its lists leave the power button out; it goes in like any
other key, in a table that counts it (found 2026-10-19: in one that
didn't, it showed effect 0, whatever its slot said).

On the 113a the tables show only at the next redraw or restart, and the
restart starts every effect over, so ryukazed keeps effect 0 for the power button:
an effect sent again shows at once, with the others running on, and stays
through a redraw. It never goes in a still report, which would hold it:
the restart (`0d 00 00`) lets the other keys go again, but not the power
button; `0d 00 02` does, starting the effects over too. A key on an effect
in the tables, but held still by a still report, as the numpad is while
NumLock is off, rejoins it only with a restart.

A key flashes on its press and again on its release, and no setting of a
key's slot makes it stay lit while held instead: of its mode byte's bits
only `0x08`, flashing, did anything (each bit tried, and 26 values in
all), the zero byte after it did nothing whatever its bits, nor did the
effect a flashing key is on, and the flash's length only makes both flashes
longer or shorter (0 ticks, no flash; 0xffff, lit for good after the first
press).

On the Titan 18 HX A2WJ (board MS-1824) the keyboard's controller is a KLC
1038:1122, and the touchpad's light (lit from below, MSI's light bar
there) and the logo on the lid are on an ALC beside it, 1038:1169, both
with firmware 2.61 (`10 00` reads it, minor then major) and the same
reports as the 113a. 1122 is on many MSI laptops with other keyboards, so
ryukazed takes it only on that board. Checked on it, live: a still report
there colours the keys it lists, its third byte how many entries the
controller reads, until the next brightness key redraws the keyboard a few
seconds later; a table of still slots, then `0d 00 02`, shows at once and
stays through that. There a table's count picks one of four regions,
whose keys alone it takes: 42 for the letters, digits and F1 to F6, 11 for
Enter, `\`, the key by right Alt and the power button, 24 for the other
keys of the main board, Win, Copilot and Fn among them, and 36 for F7 on,
the navigation keys, the arrows and the numpad; a key in another region's
table never shows, and the slots left over go to key 0xff. A table changes
only the keys it lists. Its effects (`0b`) are laid out as the 113a's, as
MSI's software writes them there; one built as that software would, a
breath on effect 1 with a key on it in the tables and `0d 00 00` after,
ran there as it should. One effect with no steps once left another laptop's
1122 dark for good, so ryukazed sends an effect only with 1 to 16 steps,
none empty, their length the sum of theirs and `ff 00` at 0x8a, on any
controller. It doesn't send `09` there: how its flash takes a save isn't
known. `51 00 [r] [g] [b]` scales each channel at full; ryukazed gives it
MSI's scale for the Titan at every start, which it already had.

A report sent right after another may be dropped there without a word,
no EPIPE as on the 113a: the effects and four tables of a profile, sent
back to back, left the last table out, the one from F7 on, while 1 ms
between them was enough. So ryukazed keeps 10 ms between reports to any
controller, as MSI's software does for all of its devices.

The ALC's zones are 0 to 4 for the touchpad, in rows from the keyboard's
edge to the front, and 5 for the logo: one report of `0e 00 06 00` and a
slot each, as above but for the zone in the key's place, then `0d 00 02`,
shows them still; zones left out keep their colours. MSI's software
knows it as the Titan 18 Dragon Edition's ALC. It runs the same effects
(`0b`, then `0d 00 00`), but every zone alike: a wave's origin, and the
places MSI's software gives the zones (`0d 00 06 00` and an x and a y
each, as a feature report), change nothing there, wherever they're put.
So ryukazed sends a wave to each zone as a round of its own, started as
far into it as the zone lies from the origin, and the zones as many
effects. How far that is comes from the 1122: the keys there lie about
239 of its units apart, and a round of a wave lies across 28.5 times the
wavelength it's sent, 273 at 25 %.

## Restarts

Every restart of the lighting flashes a few keys, brighter than anything
the keys can show, at full brightness and with the colour scale at full
too: `0d 00 00` and `0d 00 02` both, and `51`, while tables, effects and
still reports sent alone don't. The firmware SteelSeries GG carries for
the GE76's 1122 (2.39; the Titan's 2.61 isn't in it) says why. There's no
LED driver: the controller scans a matrix of 22 rows itself, a row at a
time from a timer, a thousand times a second. It restarts the lighting
inside its USB interrupt, which outranks that timer, for some 5 ms (6.5
with a wave), and the row lit just then stays lit all that time, about 22
times its share; it's a few keys spread over the board. The mode byte
after `0d` is never read, so `0d 00 00` and `0d 00 02` are the same, and
`51`, the layout's `06` and an output `0c 00 [level]`, which sets the
brightness (nothing reads it), restart the same way. GG's own name for
an `08` restart has no handler there.

The same firmware tells what changes without one:

- A slot in mode 8, a flash on a press, follows its table live, every
  10 ms; with the flash's colour the slot's and no flash length, it's a
  still key, and its colour changes with a table alone. Mode 1 is read
  only at a restart.
- An effect's steps and start colour are read live: a `0b` with other
  colours shows at once. The rest of it, a wave's origin and width, the
  steps' count and length, only at a restart, as is whether a key is on
  an effect at all.
- A still report (`0c 00 [n] 00` and key, red, green, blue) holds its keys
  over their effects until `83 [key]` lets one go, `52` all, or a restart;
  `50 00 [r] [g] [b]` holds every key so, at once.

So ryukazed restarts only where it must, at the first show, after sleep,
for another effect or a wave's other shape, or a key turning from still to
an effect, and then under black: `50 00 00 00 00`, 30 ms for it to reach
the keys, then the restart, and the row left lit is dark. Still keys go in
the tables in mode 8; a key's own colour over a running effect goes over
it in a still report, and comes off with `83` or `52`. The Titan's ALC
takes the same: checked by eye on both, with 2.61, as the firmware says.
`40` puts the tables back to the firmware's own, every key red and
flashing, and isn't sent.

None of this is checked on the Alpha's 113a, whose firmware 1.14 is another
one, and whose restart shows no flash. There ryukazed sends all of it each
time, as measured there and told above: still keys in mode 1 and in still
reports, every effect's start a restart, and none of `50`, `52` or `83`.

### The commands

What the 1122's firmware 2.39 does with each report, from its code; ryukazed
sends only those it says so for. An output report is the command and its
bytes, 64 in all; a feature report starts `0b`, `0c` or `0e`, then 00, 524
in all. "Restarts" means the flash above.

| Output | Does | ryukazed |
|---|---|---|
| `0d 00 [x]` | restarts the lighting; `x` isn't read | when it must, under black |
| `51 00 [r] [g] [b]` | the colour scale, then restarts | once, at the first show |
| `50 00 [r] [g] [b]` | every key in that colour over the rest, at once | the black before a restart |
| `52` | lets every key go of its own colour | yes |
| `83 [key]` | lets one key go of its own colour | yes |
| `09` | saves to flash, restarts with the scan stopped, dark for some 0.2 s | where the model saves |
| `10` | the firmware version: minor, then major | yes |
| `03 [key] [r] [g] [b]` | one key in its own colour, at once | no: `0c` takes many |
| `0a 00 [key] 00 [slot]` | one key's slot, shown at the next restart | no |
| `0c 00 [level]` | the brightness, then restarts; nothing reads it | no |
| `06 [layout]` | the layout, then restarts | no |
| `da [u32]` | how fast the matrix is scanned | no |
| `fd 00 [on]` | the keys held, as input reports | no |
| `ff 00 [key]` | a press for that key's flash | no |
| `15`, `22`, `80`, `86` | the firmware's CRC, a backup register, the unique ID's address, the layout's bytes | no |
| `40` | the factory tables: every key red, flashing | never |
| `01`, `02`, `f1` | restart the controller, the second for its bootloader | never |
| `08` | nothing: GG names it, the firmware has no such command | no |

| Feature | Does | ryukazed |
|---|---|---|
| `0b 00 [id] 00` | an effect: its colours live, the rest at the next restart | yes |
| `0c 00 [n] 00` | up to 110 keys in their own colours over their effects, until `83`, `52` or a restart | over running effects |
| `0e 00 [n] 00` | up to 42 slots: mode 8 live, mode 1 at the next restart | yes |

