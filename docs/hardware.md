# Hardware notes

## Tested on

- MSI Alpha 17 C7VG, EC firmware 17KKIMS1.115. msi-ec has the Fn key on
  the wrong side on it, which ryukaze turns round, and its camera bits
  (`webcam`, `webcam_block`) leave the camera working, so ryukaze doesn't
  offer them there. Its keyboard's backlight is a SteelSeries KLC
  (1038:113a), and Fn+F9 needs a hwdb entry (see
  [using ryukaze](usage.md#the-fn-keys)).

## The EC's RAM

The fan curves, the speeds and the rpm are all in the EC's RAM, which
msi-ec doesn't cover: ryukaze uses ec_sys, and only on EC firmware whose
layout has been checked against the firmware itself, as a wrong address
elsewhere could be anything. So far that's 17KKIMS1.115 (Alpha 17 C7VG),
where USB Power Share (0xBF bit 5) and the GPU MUX (below) come from the
same map. Only the curve bytes, that bit and the MUX's request are ever
written, and everything is read back; rpm comes from the tachometer
(approximate: the constant is MControlCenter's).

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

## The keyboard's controller

Still colours go in a HID feature report on the controller's first
interface: `0c 00 66 00`, then (key, red, green, blue) for up to about a
hundred keys, each key its HID usage. A report changes the keys it lists
and leaves the others.

The effects go in the older protocol msi-perkeyrgb found on earlier MSI
laptops: a `0b` report sets out an effect as a start colour and up to
sixteen steps (each channel's change per 10 ms tick, in sixteenths, and
the step's length in ticks), and a wave if it has one, which the
controller runs round and round, from the start colour each time. `0e`
reports put each key of a region on an effect, still, following it or
flashing when pressed, and an output report `09` starts them. A still
report then stops the keys it lists. While it sets up an effect the
controller may turn the next report down (EPIPE); ryukazed waits a little
and sends it again.

On a brightness key the controller draws every key again from the
regions' tables, and what a still report alone painted goes: a still
profile over a reactive one, say, comes back reactive. So ryukazed keeps
still keys in the tables too, mode 1 with their colour: a still profile's,
a key's own, the numpad dark for NumLock and the power button's own. A
still profile goes there without the refresh, which would blink, and its
still reports show it at once.

The regions' lists of keys came from msi-perkeyrgb, which leaves the power
button out; it takes the effects in region `0x24`, with the F keys from F7,
the navigation keys and the numpad (found by trying every region number
and every free key code, 2026-10-19).

A key flashes on its press and again on its release, and no setting of a
key's slot makes it stay lit while held instead: of its mode byte's bits
only `0x08`, flashing, did anything (each bit tried, and 26 values in
all), the zero byte after it did nothing whatever its bits, nor did the
effect a flashing key is on, and the flash's length only makes both flashes
longer or shorter (0 ticks, no flash; 0xffff, lit for good after the first
press).
