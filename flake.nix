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
          rustVersion = "1.76.0";
          packageFun = import ./Cargo.nix;
          extraRustComponents = [ "rustfmt" "clippy" ];
        };

        nexusRelay = (rustPkgs.workspace.nexus_relay {}).bin;
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
            websocat  # Added websocat for testing WebSocket connections
          ];
          
          # Set environment variables if needed
          shellHook = ''
            echo "NexusRelay development environment"
            echo "Use 'cargo build' to build the project"
            echo "Use 'cargo run' to run the server"
            echo "Use 'cargo2nix' to regenerate the Cargo.nix file after updating dependencies"
            echo "Use 'websocat' to test WebSocket connections"
          '';
        };
      });
} 

