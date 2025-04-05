{
  description = "xdg-desktop-portal-luminous devel and build";

  # Unstable required until Rust 1.85 (2024 edition) is on stable
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs, ... }:
    let
      # System types to support.
      targetSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

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
            version = "0.1.8";

            src = ./.;

            nativeBuildInputs = with pkgs; [
              rustPlatform.cargoSetupHook # Make Cargo find cargoDeps
              rustPlatform.bindgenHook
              cargo rustc

              meson
              ninja
              pkg-config
            ];

            cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
              inherit src;
              hash = "sha256-g0OxK7H6BwGQwQ940TlNR6s7axpX4e6KtzPUMgTJ1nU=";
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
            name = "xdg-desktop-portal-luminous-devel";
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

              gdb
              strace
              valgrind
              wayland-scanner
            ];

            inherit (self.packages.${system}.default) buildInputs;
          };
        }
      );
    };
}
