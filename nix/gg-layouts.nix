{
  lib,
  craneLib,
}:

let
  cargo = lib.importTOML ../Cargo.toml;

  common = {
    pname = "ryukaze-gg-layouts";
    inherit (cargo.workspace.package) version;
    strictDeps = true;
    cargoExtraArgs = "--locked --package ryukaze-gg-layouts";

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../crates
      ];
    };
  };

  cargoArtifacts = craneLib.buildDepsOnly common;
  checked = common // {
    inherit cargoArtifacts;
  };
in
craneLib.buildPackage (
  common
  // {
    inherit cargoArtifacts;
    # In the flake's checks instead.
    doCheck = false;

    passthru.tests = {
      cargo = craneLib.cargoTest checked;
      clippy = craneLib.cargoClippy (
        checked // { cargoClippyExtraArgs = "--all-targets -- --deny warnings"; }
      );
    };

    meta = {
      description = "Keyboard layouts for ryukaze, from a SteelSeries GG install";
      homepage = "https://github.com/kontonkara/ryukaze";
      license = lib.licenses.mit;
      platforms = lib.platforms.linux;
      mainProgram = "ryukaze-gg-layouts";
    };
  }
)
