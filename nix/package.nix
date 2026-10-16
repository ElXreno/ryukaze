{
  lib,
  rustPlatform,
}:

let
  cargo = lib.importTOML ../Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "ryukaze";
  inherit (cargo.workspace.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../data
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  postInstall = ''
    install -Dm644 data/io.github.kontonkara.ryukaze.conf \
      $out/share/dbus-1/system.d/io.github.kontonkara.ryukaze.conf
  '';

  meta = {
    description = "MSI laptop EC control: modes, charge limit, fans";
    homepage = "https://github.com/kontonkara/ryukaze";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "ryukazed";
  };
}
