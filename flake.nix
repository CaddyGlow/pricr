{
  description = "pricr: Rust CLI for cryptocurrency prices";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { flake-utils, nixpkgs, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        pricr = pkgs.callPackage ./nix/package.nix { };
        pricrApp = {
          type = "app";
          program = "${pricr}/bin/pricr";
          meta = {
            description = "Run the pricr CLI";
          };
        };
      in
      {
        packages = {
          inherit pricr;
          default = pricr;
        };

        apps = {
          pricr = pricrApp;
          default = pricrApp;
        };

        checks = {
          inherit pricr;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            clippy
            rustc
            rustfmt
          ];
        };
      }
    );
}
