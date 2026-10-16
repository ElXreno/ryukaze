# ryukaze

竜風, "dragon wind": control for MSI laptops on Linux, a from-scratch
rewrite of [MControlCenter](https://github.com/dmitry-s93/MControlCenter)
in Rust.

`ryukazed` is a small root daemon and the only process that touches the
embedded controller. It works through the upstream
[msi-ec](https://github.com/BeardOverflow/msi-ec) driver and serves a
narrow D-Bus API on the system bus.

Early days: the daemon reports the EC's state and switches its modes;
there is no GUI yet.

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

Every setter reads the value back and fails if the EC didn't take it.
[data/io.github.kontonkara.ryukaze.conf](data/io.github.kontonkara.ryukaze.conf)
lets everyone call `Status` and only root call the setters.

## Try it

```sh
nix develop
cargo run -p ryukazed -- --session
busctl --user call io.github.kontonkara.ryukaze /io/github/kontonkara/ryukaze \
  io.github.kontonkara.ryukaze1 Status
```

`--session` serves on your session bus, so reading needs no root, but
writing to the real `/sys` does. `--sysfs <dir>` points the daemon at a fake
sysfs tree.

## Tested on

- MSI Alpha 17 C7VG, EC firmware 17KKIMS1.115
