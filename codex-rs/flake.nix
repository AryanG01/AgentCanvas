{
  description = "codex-rs - Rust implementation with cross-compilation support";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Common build inputs needed for the project
        nativeBuildInputs = with pkgs; [
          pkg-config
          cmake
          llvmPackages.clang
          llvmPackages.libclang.lib
          rustPlatform.bindgenHook
        ];

        # Runtime dependencies
        buildInputs = with pkgs; [
          openssl
          libcap
          libseccomp
        ] ++ lib.optionals stdenv.isDarwin [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
        ];

        # Rust toolchain
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

      in
      {
        # Development shell
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;

          packages = with pkgs; [
            rustToolchain
            cargo-watch
            cargo-edit
            cargo-audit
            rust-analyzer
          ];

          # Environment variables for build
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          PKG_CONFIG_PATH = pkgs.lib.makeSearchPathOutput "dev" "lib/pkgconfig" (
            [ pkgs.openssl ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.libcap ]
          );

          # Ensure pkg-config can find libcap
          PKG_CONFIG_ALLOW_SYSTEM_LIBS = "1";
          PKG_CONFIG_ALLOW_SYSTEM_CFLAGS = "1";

          shellHook = ''
            echo "codex-rs development environment"
            echo "Rust toolchain: ${rustToolchain.name}"
            echo "libcap available: ${pkgs.libcap}"
            echo ""
            echo "Ready to build with: cargo build"
          '';
        };

        # Package definition
        packages.default = pkgs.callPackage ./default.nix {
          inherit (pkgs) cmake llvmPackages openssl libcap rustPlatform pkg-config lib stdenv;
          version = self.rev or "dirty";
        };

        # Cross-compilation shells for common targets
        devShells.cross-linux-musl = pkgs.mkShell {
          inherit nativeBuildInputs;

          buildInputs = with pkgs.pkgsStatic; [
            openssl
            libcap
            libseccomp
          ];

          packages = [ rustToolchain ];

          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          PKG_CONFIG_PATH = pkgs.lib.makeSearchPathOutput "dev" "lib/pkgconfig" [
            pkgs.pkgsStatic.openssl
            pkgs.pkgsStatic.libcap
          ];
          CC = "${pkgs.pkgsStatic.stdenv.cc}/bin/${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
          PKG_CONFIG_ALLOW_CROSS = "1";
          PKG_CONFIG_ALLOW_SYSTEM_LIBS = "1";
          PKG_CONFIG_ALLOW_SYSTEM_CFLAGS = "1";

          shellHook = ''
            echo "Cross-compilation environment for x86_64-unknown-linux-musl"
            rustup target add x86_64-unknown-linux-musl 2>/dev/null || true
          '';
        };

        devShells.cross-aarch64 = pkgs.mkShell {
          inherit nativeBuildInputs;

          buildInputs = with pkgs.pkgsCross.aarch64-multiplatform; [
            openssl
            libcap
            libseccomp
          ];

          packages = [ rustToolchain ];

          CARGO_BUILD_TARGET = "aarch64-unknown-linux-gnu";
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          PKG_CONFIG_PATH = pkgs.lib.makeSearchPathOutput "dev" "lib/pkgconfig" [
            pkgs.pkgsCross.aarch64-multiplatform.openssl
            pkgs.pkgsCross.aarch64-multiplatform.libcap
          ];
          CC = "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc.targetPrefix}cc";
          PKG_CONFIG_ALLOW_CROSS = "1";
          PKG_CONFIG_ALLOW_SYSTEM_LIBS = "1";
          PKG_CONFIG_ALLOW_SYSTEM_CFLAGS = "1";

          shellHook = ''
            echo "Cross-compilation environment for aarch64-unknown-linux-gnu"
            rustup target add aarch64-unknown-linux-gnu 2>/dev/null || true
          '';
        };
      }
    );
}
