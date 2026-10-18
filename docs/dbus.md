# D-Bus API

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
| `SetKeyboardProfile` | `s` | one of `keyboard_profiles`, until the scenario changes |
| `NextKeyboardProfile` | | shows the keyboard profile after the one shown, and names it: for a key binding |
| `SetGpuMode` | `s` | `hybrid` or `discrete`, from the next restart |
| `SetScenario` | `s` | one of `scenarios` |
| `NextScenario` | | applies the next scenario; returns its name (`s`) |

`Status`'s keys are in
[crates/core/src/status.rs](../crates/core/src/status.rs), each with what
it means; `ryukaze --status --json` prints the same dictionary.

Every setter reads the value back and fails if the EC didn't take it.
The signal `ModesChanged` (`a{sv}`) carries `shift_mode`, `fan_mode`,
`super_battery`, `cooler_boost`, `scenario` and `keyboard_profile`, with
`scenarios` and `keyboard_profiles`, as in `Status`, whenever they change,
whatever changed them: a call (at once), a power profile, the automatic
boost or the EC itself on Fn+F8 (within two seconds). The window, the tray and the on-screen card take it as it comes.

[data/io.github.kontonkara.ryukaze.conf](../data/io.github.kontonkara.ryukaze.conf)
lets everyone call `Status` and only root call the setters; the NixOS
module adds the users in `services.ryukaze.users`.
