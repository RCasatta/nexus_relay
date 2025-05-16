{
  description = "NexusRelay: A WebSocket Pub/Sub Relay for Wallet Coordination";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "nexus_relay";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "lwk_wollet-0.9.0" = "sha256-HfXMXOM8jyU/dV3nYApVA3GsegW2pOX50ZfEDavbwoM=";
              "lwk_common-0.9.0" = "sha256-HfXMXOM8jyU/dV3nYApVA3GsegW2pOX50ZfEDavbwoM=";
            };
          };
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          buildInputs = with pkgs; [
            # Add any runtime dependencies here
          ];
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
            websocat  # Added websocat for testing WebSocket connections
          ];
          
          # Set environment variables if needed
          shellHook = ''
            echo "NexusRelay development environment"
            echo "Use 'cargo build' to build the project"
            echo "Use 'cargo run' to run the server"
            echo "Use 'websocat' to test WebSocket connections"
          '';
        };
      });
} 

