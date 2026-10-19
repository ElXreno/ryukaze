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

The window and the editor in `docs/images` are screenshots of the window
in its built-in colours: with `XDG_CONFIG_DIRS` at an empty directory and
`XDG_CONFIG_HOME` at one with only `ryukaze/theme.toml` saying `mode`,
open ten minutes for the graphs, a second one for the other mode under
`dbus-run-session` so it doesn't hand over to the first. The card is drawn:

```sh
XDG_CONFIG_HOME=/var/empty XDG_CONFIG_DIRS=/var/empty \
  cargo test -p ryukaze screenshots -- --ignored   # into target/shots
```
