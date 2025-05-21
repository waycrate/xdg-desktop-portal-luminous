{
  description = "xdg-desktop-portal-luminous devel and build";

  # Unstable required until Rust 1.85 (2024 edition) is on stable
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # shell.nix compatibility
  inputs.flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.tar.gz";

  outputs = { self, nixpkgs, ... }:
    let
      # System types to support.
      targetSystems = [ "x86_64-linux" "aarch64-linux" ];

      # Helper function to generate an attrset '{ x86_64-linux = f "x86_64-linux"; ... }'.
      forAllSystems = nixpkgs.lib.genAttrs targetSystems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          # rustPlatform.buildRustPackage is not used because we build with Meson+Ninja
          default = pkgs.stdenv.mkDerivation rec {
            pname = "xdg-desktop-portal-luminous";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

            src = ./.;

            nativeBuildInputs = with pkgs; [
              rustPlatform.cargoSetupHook # Make Cargo find cargoDeps
              rustPlatform.bindgenHook
              cargo rustc

              meson
              ninja
              pkg-config
            ];

            cargoDeps = pkgs.rustPlatform.importCargoLock {
              lockFile = ./Cargo.lock;
            };

            buildInputs = with pkgs; [
              pipewire
              libxkbcommon
              pango
              cairo
            ];

            meta = with nixpkgs.lib; {
              description = "An alternative to xdg-desktop-portal-wlr for wlroots compositors";
              homepage = "https://github.com/waycrate/xdg-desktop-portal-luminous";
            };
          };
        }
      );
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            strictDeps = true;
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            nativeBuildInputs = with pkgs; [
              cargo
              rustc
              rustPlatform.bindgenHook
              pkg-config
              meson
              ninja

              rustfmt
              clippy
              rust-analyzer
            ];

            inherit (self.packages.${system}.default) buildInputs;
          };
        }
      );
    };
}
