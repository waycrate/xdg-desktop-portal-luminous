{
  description = "xdg-desktop-portal-luminous devel";

  inputs = { nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable"; };

  outputs = { self, nixpkgs, ... }:
    let
      pkgsFor = system:
        import nixpkgs {
          inherit system;
          overlays = [ ];
        };

      targetSystems = [ "aarch64-linux" "x86_64-linux" ];
    in {
      devShells = nixpkgs.lib.genAttrs targetSystems (system:
        let pkgs = pkgsFor system;
        in {
          default = pkgs.mkShell {
            name = "xdg-desktop-portal-luminous-devel";
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            nativeBuildInputs = with pkgs; [
              # Compilers
              clang
              cargo
              rustc

              # Libs
              pipewire
              wayland
              libxkbcommon
              stdenv
              glib
              pango
              cairo

              # Tools
              meson
              ninja
              gdb
              pkg-config
              rust-analyzer
              rustfmt
              strace
              valgrind
              wayland-scanner
            ];
          };
        });
    };
}
