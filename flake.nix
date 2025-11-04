{
  description = "mihomo-cli development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" ];
        };

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        mihomo-cli = rustPlatform.buildRustPackage {
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
        packages = {
          default = mihomo-cli;
          mihomo-cli = mihomo-cli;
        };
        defaultPackage = mihomo-cli;
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

        apps = {
          default = flake-utils.lib.mkApp {
            drv = mihomo-cli;
          };
          mihomo-cli = flake-utils.lib.mkApp {
            drv = mihomo-cli;
          };
        };
        defaultApp = flake-utils.lib.mkApp {
          drv = mihomo-cli;
        };
      }
    );
}
