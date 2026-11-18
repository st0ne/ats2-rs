# Development

## Requirements

- Stable Rust and Cargo; crate MSRV is 1.85.
- libusb 1.0 development files and `pkg-config` on systems that build `rusb` against system libusb.
- No Windows toolchain or proprietary Audio Precision files are needed to build or test this project.

## Reproducible Nix development shell

The project flake pins Nixpkgs and Rust 1.85.0, including rustfmt, Clippy, rust-analyzer, the Glow renderer, X11/Wayland/OpenGL libraries, libusb, compiler/linker, and debugging tools. From this checkout, use:

```sh
nix develop "path:$PWD"
```

The `path:` prefix makes Nix include the local flake files without changing the Git index; Nix's default Git flake source omits untracked files. Once the flake files are tracked, ordinary `nix develop` works. Inside the shell, run `cargo fetch --locked` when dependencies are not cached, then use the offline checks below. To start the mock-only GUI in a graphical session:

```sh
cargo run --offline --locked --bin ats2-gui -- --mock
```

## Quality checks

Run from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
cargo test --offline --locked --all-targets
```

The default unit tests are hardware-independent. The CLI can be invoked with `cargo run -- list` or `cargo run -- info`; those commands read USB descriptors on the host, so run them only when descriptor discovery is intended.

## Safety and repository hygiene

- Never add files from `originalSW/`, `extracted/`, `docs/reverse-engineering/`, or captures to this repository.
- Do not commit proprietary DLLs, EXEs, SYS files, installers, vendor manuals, USB traces, or unpublished reverse-engineering notes.
- Keep APIB protocol changes test-driven and require verified vectors/traces; do not invent formats.
- Do not add code that writes to the adapter without an explicit, reviewed protocol specification and separate authorization.
