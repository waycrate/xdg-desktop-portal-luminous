{
  description = "xdg-desktop-portal-luminous devel";

  # Use nix develop --impure for this to work

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (system:
    {
      devShells.default = import ./shell.nix;
    });
}
