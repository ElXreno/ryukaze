# Development

```sh
nix develop
cargo run -p ryukazed -- --session
cargo run -p ryukaze -- --session   # in another terminal
busctl --user call io.github.kontonkara.ryukaze /io/github/kontonkara/ryukaze \
  io.github.kontonkara.ryukaze1 Status
```

`--session` serves on your session bus, so reading needs no root, but
writing to the real `/sys` does. `--sysfs <dir>` points the daemon at a
fake sysfs tree.

`cargo test` runs the tests, among them two that parse the examples on
these pages: the config in [config.md](config.md) and the colours in
[usage.md](usage.md#colours). `nix flake check` builds the package, which
runs them too, and runs the NixOS module in a VM; CI does all of it on
every push, with rustfmt, nixfmt and clippy.

## The readme's pictures

The pictures in `docs/images`, the window, the keyboard light's editor
and the card, are drawn without a screen, over the demo's laptop
(`ryukaze --demo`) in the built-in colours, with the logo the firmware
gives where it gives one:

```sh
XDG_CONFIG_HOME=/var/empty XDG_CONFIG_DIRS=/var/empty \
  cargo test -p ryukaze screenshots -- --ignored   # into target/shots
```
