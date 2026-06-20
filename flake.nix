{
  description = "niri-groom — a niri workspace/window survey & kill overlay";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          rustfmt
          clippy
          rust-analyzer
          pkg-config
          wrapGAppsHook4
        ];

        buildInputs = with pkgs; [
          gtk4
          gtk4-layer-shell
          glib
          gdk-pixbuf
          graphene
          cairo
          pango
          harfbuzz
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;

          shellHook = ''
            echo "niri-groom dev shell — rust $(rustc --version | cut -d' ' -f2), gtk4 $(pkg-config --modversion gtk4)"
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "niri-groom";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          inherit nativeBuildInputs buildInputs;
        };
      }
    );
}
