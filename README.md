# ats2-rs

A Rust library and command-line tool for future native, cross-platform control of Audio Precision ATS-2 instruments.

## Current scope

The `ats2 list` and `ats2 info` CLI remains descriptor-only. The library now exposes an explicit USB path for VID `0x16D0` / PID `0x0AF0`: opening discovers and validates the target descriptors, while a separate call claims interface 0 only if configuration 1 and alternate setting 0 are already active. No configuration or alternate setting is changed. Bulk IN/OUT operations occur only when the caller explicitly invokes the transport/API transfer API; opening does not send APIB data. The implementation does not issue vendor-control requests, load firmware, reset the device, or automatically detach kernel drivers.

The logical, framing, and transfer modules form distinct boundaries: logical encodings are explicit, framing is pure, and transfer orchestration is opt-in.

## Build and test

Requires Rust 1.85 or newer. The reproducible Nix shell provides the pinned compiler, GUI toolchain, and Linux native libraries; see [docs/development.md](docs/development.md).

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
cargo test --offline --locked --all-targets
```

All tests and the GUI mock run without connected hardware.

## Reproducible Nix environment

The `flake.nix`/`flake.lock` pin Nixpkgs and Rust 1.85.0, plus the Glow renderer, X11/Wayland, OpenGL, libusb, compiler, linker, and development tools. From this checkout, enter the shell with:

```sh
nix develop "path:$PWD"
```

The explicit `path:` selector is needed here because the flake files are intentionally not staged; Nix's default Git flake source omits untracked files. In a checkout where the flake files are tracked, use `nix develop`. After entering the shell, fetch Cargo dependencies if needed with `cargo fetch --locked`. Run the GUI only in a graphical session:

```sh
cargo run --offline --locked --bin ats2-gui -- --mock
```

## CLI

```sh
cargo run -- list  # enumerate USB devices; mark matching adapter
cargo run -- info  # show descriptors for matching adapter(s)
```

Only standard USB descriptor discovery is performed. Linux may require appropriate udev permissions to see USB devices.

## Project boundaries

Only this directory is intended to be the standalone GitHub repository. Proprietary installers, vendor binaries, device captures, and the separate reverse-engineering workspace are not part of this repository.

## Project status and hardware validation

This is an independently written Rust implementation, not vendor software and not affiliated with or endorsed by Audio Precision. Some software models are based on documented/static-analysis findings; this is not a claim of clean-room provenance. The project is experimental and still under development. It has **not been validated against physical ATS-2 hardware**; the automated tests are synthetic/offline and establish software behavior only, not instrument behavior.

## License

This project is licensed under the Apache License, Version 2.0; see [LICENSE](LICENSE). Third-party dependencies retain their respective licenses.
