# Keyboard layouts from GG

`ryukaze-gg-layouts` makes the window's [layout files](keyboard.md#the-picture)
from a SteelSeries GG install of one's own: the keys, the touchpad's rows
and the logo on the lid as GG draws them, and the laptops GG takes each
keyboard for. GG's files stay where they are: it reads them as it runs and
writes only TOML, the keys' places and sizes, which may be shared.

It's a package of its own, apart from ryukaze's:

```sh
nix run github:kontonkara/ryukaze#gg-layouts -- --out layouts ~/GG
# or from the tree
cargo run --release -p ryukaze-gg-layouts -- --out layouts ~/GG
```

`~/GG` is GG's install directory, copied from Windows, or its installer
unpacked (`7z x SteelSeriesGG*.exe`), or only its `resources/app.asar`,
without which no laptops are named. It writes a file for each keyboard GG
draws, by GG's name for it, such as `msi_titan18_klc.toml`, and says on
stderr what it measured and what it left out. A file goes in
`~/.local/share/ryukaze/keyboards` for the window to read.

- `--device NAME`: only this keyboard, again for more;
- `--label HID=TEXT`: the key of this HID usage labelled so, such as
  `0xe4=Copilot`, as GG has no labels;
- `--file FILE`, `--name NAME`, `--board BOARD` and `--for-any`, with one
  `--device`: the layout to `FILE`, named `NAME`, for the laptops of DMI
  board `BOARD` too, or for any laptop, with no CHIDs.

## What it reads

- The keys: GG's style sheet in `app.asar` places a marker over each key,
  by HID usage, on a picture of the keyboard in which each key is drawn
  ringed by clear pixels. The markers are fitted onto the keys found in
  each of the keyboard's pictures, and the one they fit best is measured:
  across in the letters' pitch, down in the rows', from Esc.
- The touchpad and the logo, where the laptop has a light bar
  (`<name>_alc` beside `<name>_klc`): its zones over a picture of the lid
  and the deck side by side. The touchpad's rows are placed by the keys
  drawn on the deck, fitted to the keyboard's; the logo is given as parts
  of the lid, seen from the keys, GG showing it from behind.
- The laptops: the engine, `SteelSeriesEngine.exe`, takes a laptop's CHID
  (Windows' HardwareID-9, of the DMI `sys_vendor` and `product_name`) to a
  controller in a table built into it; its database's migrations name the
  controller. A layout lists in `chids` those of its keyboard, and the
  window matches them against the laptop's own. The table is found by its
  shape, not at an address, so another GG version reads too.

## The built-in layouts

[`data/keyboards`](../data/keyboards) is made so, from GG 122:

```sh
ryukaze-gg-layouts --device msi_titan18_dragon_edition_klc --name "Titan 18 HX" \
  --board MS-1824 --label 0xe4=Copilot --file data/keyboards/ms-1824.toml ~/GG
ryukaze-gg-layouts --device msi_gp76_klc --name "Alpha 17 C7VG" --for-any \
  --label 0xe4=Copilot --file data/keyboards/default.toml ~/GG
```

and checked against GG again with
`RYUKAZE_GG=~/GG cargo test --release -p ryukaze-gg-layouts -- --ignored`.
