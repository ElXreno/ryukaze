# ryukaze

竜風, "dragon wind": control for MSI laptops on Linux, a from-scratch
rewrite of [MControlCenter](https://github.com/dmitry-s93/MControlCenter)
in Rust.

`ryukazed` is a small root daemon and the only process that touches the
embedded controller. It works through the upstream
[msi-ec](https://github.com/BeardOverflow/msi-ec) driver and serves a
narrow D-Bus API on the system bus.

Early days: for now the daemon only reports the EC's state.

## Try it

```sh
nix develop
cargo run -p ryukazed -- --session
busctl --user call io.github.kontonkara.ryukaze /io/github/kontonkara/ryukaze \
  io.github.kontonkara.ryukaze1 Status
```

`--session` serves on your session bus, so no root is needed. Reading
msi-ec's attributes is open to every user. `--sysfs <dir>` points the
daemon at a fake sysfs tree.

## Tested on

- MSI Alpha 17 C7VG, EC firmware 17KKIMS1.115
