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
      in {
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
      }
    );
}
