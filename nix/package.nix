{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "pricr";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter =
      path: type:
      lib.cleanSourceFilter path type
      && builtins.all (name: name != builtins.baseNameOf path) [
        ".git"
        ".venv"
        "target"
        "result"
      ];
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  doCheck = true;
  strictDeps = true;

  meta = {
    description = "A modern Rust CLI for cryptocurrency prices and conversions";
    homepage = "https://github.com/CaddyGlow/pricr";
    license = lib.licenses.mit;
    mainProgram = "pricr";
    platforms = lib.platforms.unix;
  };
}
