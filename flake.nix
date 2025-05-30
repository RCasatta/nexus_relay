{
  description = "NexusRelay: A WebSocket Pub/Sub Relay for Wallet Coordination";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Add rust-overlay for better Rust toolchain management
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        
        # Create a custom Rust toolchain with the components we need
        rustToolchain = pkgs.rust-bin.stable."1.82.0".default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
        
        # Python environment with zmq
        pythonWithZmq = pkgs.python3.withPackages (ps: [ ps.pyzmq ]);
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "nexus_relay";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            # TODO remove once released
            outputHashes = {
              "lwk_wollet-0.9.0" = "sha256-qskba+TrMJAQlm7jE/08BjsuFktZyMLs2tx4jGNLJb0=";
              "lwk_common-0.9.0" = "sha256-qskba+TrMJAQlm7jE/08BjsuFktZyMLs2tx4jGNLJb0=";
              "elements-0.25.2" = "sha256-pUbvYi1LZn73w4owjVjOvBSTeAaL1/44zSsEpT6i4EE=";

            };
          };
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          buildInputs = with pkgs; [
            openssl
            elementsd # Add elementsd to buildInputs
          ];
          
          # Pass environment variables to the build and test process
          ELEMENTSD_EXEC = "${pkgs.elementsd}/bin/elementsd";
          
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Use our custom Rust toolchain that includes rust-src
            rustToolchain
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
            # Make sure rust-analyzer can find the Rust sources
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          };
        };
      });
} 

