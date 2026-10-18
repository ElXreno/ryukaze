# Config

`/etc/ryukaze/config.toml` (or `ryukazed --config <file>`); the NixOS module
writes it from `services.ryukaze.settings`. Every key is optional, and
what's unset the daemon leaves alone, or sets as the window last left it
(see [using ryukaze](usage.md#the-window)); what's set it sets again at
every start. The [advanced fan mode](fans.md) and the
[keyboard's backlight](keyboard.md) have pages of their own.

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
# The window picks a scenario for each too, unless they're set here.
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

# The keyboard's backlight (see keyboard.md), in profiles, one shown at a
# time: set at start, after sleep and when its controller comes back.
[keyboard]
# The one shown outside the scenarios with their own, unless another was
# picked since; the first of the profiles below if unset. With neither, the
# keyboard is left as it is until one is picked.
profile = "static"
# The numpad dark while NumLock is off, but for NumLock itself.
numpad_follows_numlock = true

# A profile by scenario, shown while the EC is in its modes; the window
# picks them too, for scenarios not set here.
[keyboard.scenarios]
games = "wave"

# What NextKeyboardProfile steps through, in this order, after the standard
# ones (static, breathe, cycle, wave and reactive: white, or the rainbow),
# each in place of the standard one of its name. The effect is static (the
# default), breathe, cycle, wave or reactive; keys and groups of keys can
# have colours of their own, still whatever the effect.
[[keyboard.profiles]]
name = "static"
color = "#ff6000"
keys = { wasd = "#ff0000", arrows = "#ffffff" }

[[keyboard.profiles]]
name = "breathe"
effect = "breathe"
color = "#ff6000"
fade_to = "#0040ff"
period_ms = 4000

[[keyboard.profiles]]
name = "wave"
effect = "wave"
period_ms = 3000

[[keyboard.profiles]]
name = "reactive"
effect = "reactive"
color = "#301000"
flash = "#ffffff"
flash_ms = 500

# Zones: keys with an effect of their own beside the profile's.
[[keyboard.profiles]]
name = "split"
color = "#ff6000"

[[keyboard.profiles.zones]]
keys = ["numpad"]
effect = "breathe"
color = "#0040ff"

[[keyboard.profiles.zones]]
keys = ["letters"]
effect = "wave"
colors = ["#ff0000", "#0000ff"]
origin = "left"

# The advanced fan mode (see fans.md). These are the defaults.
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
