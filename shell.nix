# shell.nix

with (import <nixpkgs> { });

let
  libPath =
    with pkgs;
    lib.makeLibraryPath [
      #You can load external libraries that you need in your rust project here
    ];
  moz_overlay = import (builtins.fetchTarball "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");

  nixpkgs = import <nixpkgs> {
    overlays = [
      moz_overlay
    ];
  };

in
mkShell {
  name = "moz_overlay_shell";
  buildInputs = [
    # Compilers
    clang

    # Libs
    cairo
    pango
    libxkbcommon
    stdenv
    glib
    pipewire
    wayland
	  pkg-config
    nixpkgs.latest.rustChannels.nightly.rust
  ];
  LD_LIBRARY_PATH = libPath;
  RUST_BACKTRACE = 1;
  shellHook = ''
    export RUST_SRC_PATH="${nixpkgs.latest.rustChannels.nightly.rust-src}/lib/rustlib/src/rust/library"
    export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
  '';
  BINDGEN_EXTRA_CLANG_ARGS =
    (builtins.map (a: ''-I"${a}/include"'') [
      # Add include paths for other libraries here
    ])
    ++ [
      # Special directories
    ];
}
