# D-Bus API

The daemon is `io.github.kontonkara.ryukaze` on the system bus, its
interface `io.github.kontonkara.ryukaze1` at `/io/github/kontonkara/ryukaze`:

| Method | Arguments | |
|---|---|---|
| `Status` | | `a{sv}`; a key is missing when the EC lacks the feature |
| `SetShiftMode` | `s` | one of `shift_modes` |
| `SetFanMode` | `s` | one of `fan_modes` |
| `SetSuperBattery` | `b` | |
| `SetCoolerBoost` | `b` | |
| `SetChargeLimit` | `y` | 10 to 100; charging resumes 10 points below |
| `SetFanCurve` | `s ay ay` | `cpu` or `gpu`, 6 temperatures, 7 speeds: the curve advanced runs on |
| `ResetFanCurve` | `s` | back to the configured curve, or the EC's own |
| `SetAdaptiveFans` | `b` | adaptive or stepping in advanced |
| `SetGpuFanFollowsCpu` | `b` | while adaptive, the GPU fan at least at the CPU curve's speed |
| `SetAutoCoolerBoost` | `b` | cooler boost by itself when hot |
| `SetCoolerBoostMarks` | `y y` | the CPU and GPU °C (60 to 100) it comes on at |
| `SetWebcam` | `b` | the camera on or off |
| `SetFnKey` | `s` | `left` or `right` of the Win key |
| `SetUsbShare` | `b` | USB ports powered while the laptop is off |
| `SetNumpadFollowsNumlock` | `b` | the numpad dark while NumLock is off |
| `SetPowerLight` | `s` | what the power button's light follows: `profile`, `shift_mode` or `dgpu` |
| `SetPowerLightDgpu` | `b` | following the shift mode, the dGPU's colour for five seconds as it wakes |
| `SetPowerColor` | `s s` | the power button's colour for a shift mode, or `dgpu` for the dGPU's awake: `#rrggbb` or `off` |
| `SetPowerProfilesSync` | `b` | power-profiles-daemon's profile with the shift mode, both ways |
| `SetKeyboardProfile` | `s` | one of `keyboard_profiles`, shown but in a scenario with its own, and kept for the next start |
| `NextKeyboardProfile` | | picks the keyboard profile after the one shown, as `SetKeyboardProfile` does, and names it: for a key binding |
| `PreviewKeyboardProfile` | `s` | a profile in TOML, as `[[keyboard.profiles]]` takes it, shown until another is picked or the scenario changes; empty, the one picked again |
| `ReadKeyboardProfile` | `s` | one of `keyboard_profiles` as the config writes it, in TOML (`s`): for an editor to start from |
| `SaveKeyboardProfile` | `s` | a profile in TOML, as `[[keyboard.profiles]]` takes it, under a name of its own or a saved one's, kept with the others and on the disk (see [saved profiles](keyboard.md#saved-profiles)), and picked |
| `DeleteKeyboardProfile` | `s` | one of `keyboard_saved_profiles` gone |
| `SetGpuMode` | `s` | `hybrid` or `discrete`, from the next restart |
| `SetScenario` | `s` | one of `scenarios` |
| `ReadScenarios` | | the scenarios, `[[scenarios]]` in TOML as the config writes them (`s`): for an editor to start from |
| `SetScenarios` | `s` | all of them, in order, the same way; one the config names for AC, battery or a keyboard light has to stay |
| `SetScenarioKeyboardProfile` | `s s` | one of `scenarios`, and one of `keyboard_profiles` it lights, or `""` for the one picked |
| `SetSourceScenario` | `s s` | `ac` or `battery`, and one of `scenarios` to apply on it, or `""` for nothing; applied at once while on it |
| `NextScenario` | | applies the next scenario; returns its name (`s`) |

`Status`'s keys are in
[crates/core/src/status.rs](../crates/core/src/status.rs), each with what
it means; `ryukaze --status --json` prints the same dictionary.

Every setter reads the value back and fails if the EC didn't take it.
What `SetChargeLimit`, `SetWebcam`, `SetFnKey`, `SetUsbShare`,
`SetFanCurve` (and `ResetFanCurve`), `SetAdaptiveFans`,
`SetGpuFanFollowsCpu`, `SetAutoCoolerBoost`, `SetCoolerBoostMarks`,
`SetNumpadFollowsNumlock`, `SetPowerLight`, `SetPowerLightDgpu`,
`SetPowerColor`, `SetPowerProfilesSync`, `SetSourceScenario`,
`SetScenarioKeyboardProfile` and `SetScenarios` set is kept for the next
start but for what the config sets (see
[using ryukaze](usage.md#the-window)).
The signal `ModesChanged` (`a{sv}`) carries `shift_mode`, `fan_mode`,
`super_battery`, `cooler_boost`, `scenario` and `keyboard_profile`, with
`scenarios` and `keyboard_profiles`, as in `Status`, whenever they change,
whatever changed them: a call (at once), a power profile, the automatic
boost or the EC itself on Fn+F8 (within two seconds). The window, the tray and the on-screen card take it as it comes.

[data/io.github.kontonkara.ryukaze.conf](../data/io.github.kontonkara.ryukaze.conf)
lets everyone call `Status`, `ReadScenarios` and `ReadKeyboardProfile`
and only root call the setters; the NixOS module adds the users in `services.ryukaze.users`.
