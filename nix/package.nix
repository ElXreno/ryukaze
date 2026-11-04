{
  lib,
  craneLib,
  vulkan-loader,
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
  # window system's, and the Vulkan loader.
  windowLibs = [
    vulkan-loader
    wayland
    libxkbcommon
    libX11
    libXcursor
    libXi
    libXrandr
  ];

  common = {
    pname = "ryukaze";
    inherit (cargo.workspace.package) version;
    strictDeps = true;

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
  };

  # The dependencies alone, built again only when Cargo.lock or a crate's
  # manifest changes.
  cargoArtifacts = craneLib.buildDepsOnly common;

  # The tests draw in software, as in the dev shell.
  checked = common // {
    inherit cargoArtifacts;
    ICED_TEST_BACKEND = "tiny-skia";
  };
in
craneLib.buildPackage (
  common
  // {
    inherit cargoArtifacts;
    # In the flake's checks instead.
    doCheck = false;

    postInstall = ''
      install -Dm644 data/io.github.kontonkara.ryukaze.conf \
        $out/share/dbus-1/system.d/io.github.kontonkara.ryukaze.conf
      install -Dm644 data/ryukaze.desktop $out/share/applications/ryukaze.desktop
      install -Dm644 data/ryukaze.svg $out/share/icons/hicolor/scalable/apps/ryukaze.svg
    '';

    postFixup = ''
      patchelf --add-rpath ${lib.makeLibraryPath windowLibs} $out/bin/ryukaze
    '';

    passthru.tests = {
      cargo = craneLib.cargoTest checked;
      clippy = craneLib.cargoClippy (
        checked // { cargoClippyExtraArgs = "--all-targets -- --deny warnings"; }
      );
    };

    meta = {
      description = "MSI laptop EC control: modes, charge limit, fans";
      homepage = "https://github.com/kontonkara/ryukaze";
      license = lib.licenses.mit;
      platforms = lib.platforms.linux;
      mainProgram = "ryukaze";
    };
  }
)
