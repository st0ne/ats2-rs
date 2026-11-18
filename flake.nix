{
  description = "Reproducible ATS-2 Rust and native GUI development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/da6dc0e7b26060f2f71ce3ef7088fb56ee44fa36";
    rust-overlay = {
      url = "github:oxalica/rust-overlay/2776e42828203ec89511699c8dcdedec69ab98e1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          rustToolchain = pkgs.rust-bin.stable."1.85.0".default.override {
            extensions = [ "clippy" "rust-analyzer" "rust-src" "rustfmt" ];
          };
          nativeLibraries = with pkgs; [
            libGL
            libusb1
            libxkbcommon
            mesa
            wayland
          ] ++ (with pkgs; [
            libx11
            libxcursor
            libxext
            libxfixes
            libxi
            libxrandr
            libxrender
            libxcb
          ]);
        in
        {
          default = pkgs.mkShell {
            name = "ats2-rs-dev";
            packages = [
              rustToolchain
              pkgs.gdb
              pkgs.strace
              pkgs.xvfb-run
              pkgs.xdotool
              pkgs.xwininfo
              pkgs.mesa-demos
              pkgs.imagemagick
            ];
            nativeBuildInputs = [ pkgs.pkg-config pkgs.stdenv.cc ];
            buildInputs = nativeLibraries;

            # Native GUI/USB libraries must also be discoverable when running
            # Cargo-built binaries outside Nix's build phase.
            shellHook = ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath nativeLibraries}:''${LD_LIBRARY_PATH:-}"
              export RUST_BACKTRACE=1
            '';
          };
        });
    };
}
