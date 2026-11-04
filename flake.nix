{
  description = "ryukaze: MSI laptop EC control (modes, charge limit, fans) for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs [
          "x86_64-linux"
          "aarch64-linux"
        ] (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.callPackage ./nix/package.nix { craneLib = crane.mkLib pkgs; };
        gg-layouts = pkgs.callPackage ./nix/gg-layouts.nix { craneLib = crane.mkLib pkgs; };
      });

      nixosModules.default = import ./nix/module.nix self;

      # nix flake check: the tests, clippy, and the module in a VM
      # (nix/test.nix).
      checks =
        nixpkgs.lib.recursiveUpdate
          (forAllSystems (
            pkgs:
            let
              packages = self.packages.${pkgs.stdenv.hostPlatform.system};
            in
            packages.default.tests
            // nixpkgs.lib.mapAttrs' (
              name: nixpkgs.lib.nameValuePair "gg-layouts-${name}"
            ) packages.gg-layouts.tests
          ))
          {
            x86_64-linux.vm = import ./nix/test.nix {
              pkgs = nixpkgs.legacyPackages.x86_64-linux;
              inherit self;
            };
          };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
          ];

          # winit and wgpu open these at run time; the package sets an rpath
          # instead.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
            with pkgs;
            [
              vulkan-loader
              wayland
              libxkbcommon
            ]
          );

          # The tests draw in software: wgpu, off screen, takes the fastest
          # GPU, a sleeping discrete one too.
          ICED_TEST_BACKEND = "tiny-skia";

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
