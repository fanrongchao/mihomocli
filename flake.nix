{
  description = "mihomo-cli development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" ];
        };

        mihomo-cli = pkgs.rustPlatform.buildRustPackage {
          pname = "mihomo-cli";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.openssl
            pkgs.curl
            pkgs.zlib
          ];
        };
      in {
        packages.default = mihomo-cli;
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [
            rustToolchain
            pkgs.pkg-config
            pkgs.rust-analyzer
          ];
          buildInputs = [
            pkgs.openssl
            pkgs.curl
            pkgs.zlib
          ];
          shellHook = ''
            export CARGO_HOME=$PWD/.cargo
            export RUSTUP_HOME=$PWD/.rustup
          '';
        };

        apps.default = flake-utils.lib.mkApp {
          drv = mihomo-cli;
        };
      }
    );
}
