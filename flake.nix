{
  description = "ryukaze: MSI laptop EC control (modes, charge limit, fans) for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
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
        default = pkgs.callPackage ./nix/package.nix { };
      });

      nixosModules.default = import ./nix/module.nix self;

      # nix flake check: the module in a VM (nix/test.nix).
      checks.x86_64-linux.vm = import ./nix/test.nix {
        pkgs = nixpkgs.legacyPackages.x86_64-linux;
        inherit self;
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
