{
  lib,
  rustPlatform,
  libGL,
  wayland,
  libxkbcommon,
  libX11,
  libXcursor,
  libXi,
  libXrandr,
}:

let
  cargo = lib.importTOML ../Cargo.toml;

  # winit and wgpu open these at run time rather than linking them: the
  # window system's, and glvnd's libEGL.
  windowLibs = [
    libGL
    wayland
    libxkbcommon
    libX11
    libXcursor
    libXi
    libXrandr
  ];
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
      # The config and theme tests parse the examples on its pages.
      ../docs
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  postInstall = ''
    install -Dm644 data/io.github.kontonkara.ryukaze.conf \
      $out/share/dbus-1/system.d/io.github.kontonkara.ryukaze.conf
    install -Dm644 data/ryukaze.desktop $out/share/applications/ryukaze.desktop
    install -Dm644 data/ryukaze.svg $out/share/icons/hicolor/scalable/apps/ryukaze.svg
  '';

  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath windowLibs} $out/bin/ryukaze
  '';

  meta = {
    description = "MSI laptop EC control: modes, charge limit, fans";
    homepage = "https://github.com/kontonkara/ryukaze";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "ryukaze";
  };
}
