{
  description = "cryptoprice: Rust CLI for cryptocurrency prices";

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
        cryptoprice = pkgs.callPackage ./nix/package.nix { };
        cryptopriceApp = {
          type = "app";
          program = "${cryptoprice}/bin/cryptoprice";
          meta = {
            description = "Run the cryptoprice CLI";
          };
        };
      in
      {
        packages = {
          inherit cryptoprice;
          default = cryptoprice;
        };

        apps = {
          cryptoprice = cryptopriceApp;
          default = cryptopriceApp;
        };

        checks = {
          inherit cryptoprice;
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
