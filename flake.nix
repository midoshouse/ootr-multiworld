{
    inputs = {
        nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/*.tar.gz";
        rust-overlay = {
            url = "github:oxalica/rust-overlay";
            inputs.nixpkgs.follows = "nixpkgs";
        };
    };
    outputs = attrs: let
        supportedSystems = [
            "aarch64-darwin"
            "aarch64-linux"
            "x86_64-darwin"
            "x86_64-linux"
        ];
        forEachSupportedSystem = f: attrs.nixpkgs.lib.genAttrs supportedSystems (system: f {
            pkgs = import attrs.nixpkgs {
                inherit system;
                overlays = [
                    attrs.rust-overlay.overlays.default # required for cargo-script
                ];
            };
        });
    in {
        devShells = forEachSupportedSystem ({ pkgs, ... }: {
            default = pkgs.mkShell {
                buildInputs = with pkgs; [
                    glib
                    openssl
                ];
                nativeBuildInputs = with pkgs; [
                    clang
                    dotnet-sdk
                    glib.dev
                    pkg-config
                ];
                packages = with pkgs; [
                    rust-bin.stable.latest.default # stable cargo, required for Cargo config import
                ];
            };
        });
    };
}
