{
  description = "NexusRelay: A WebSocket Pub/Sub Relay for Wallet Coordination";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    cargo2nix = {
      url = "github:cargo2nix/cargo2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, cargo2nix, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            cargo2nix.overlays.default
            rust-overlay.overlays.default
          ];
        };

        rustPkgs = pkgs.rustBuilder.makePackageSet {
          rustVersion = "1.82.0";
          packageFun = import ./Cargo.nix;
          extraRustComponents = [ "rustfmt" "clippy" ];
        };

        nexusRelay = (rustPkgs.workspace.nexus_relay {}).bin;
        
        # Python environment with zmq
        pythonWithZmq = pkgs.python3.withPackages (ps: [ ps.pyzmq ]);
      in
      {
        packages = {
          default = nexusRelay;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            (rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" "rustfmt" "clippy" ];
            })
            cargo2nix.packages.${system}.cargo2nix
            pkg-config
            openssl
            openssl.dev
            websocat  # Added websocat for testing WebSocket connections
            elementsd # Added elementsd for testing
            pythonWithZmq # Added Python with ZMQ support
          ];
          
          # Set environment variables the Nix way
          env = {
            ELEMENTSD_EXEC = "${pkgs.elementsd}/bin/elementsd";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig:$PKG_CONFIG_PATH";
          };
        };
      });
} 

