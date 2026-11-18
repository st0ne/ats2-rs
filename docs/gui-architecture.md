# Native desktop GUI architecture and dependency gate (ATS-17)

**Status (2026-11-18):** egui/eframe architecture selected and implemented as a mock-only native GUI. The Nix shell and Cargo dependencies are locked; Rust 1.85.0 compilation has been verified for the GUI binary. GUI launch still requires a graphical session.

## Decision

Use **egui/eframe 0.31.1** with **egui 0.31.1** and **egui_plot 0.31.0**. All three crates declare Rust 1.81 or newer; their compatible version range and the full selected dependency graph were resolved and locked for the project's Rust 1.85 MSRV. The native renderer is eframe's Glow/OpenGL backend, with both X11 and Wayland enabled. Plot transforms use log10 coordinates for selected logarithmic axes, while saved values remain unchanged.

The GUI binary and full test suite build on x86_64-linux with the pinned toolchain. The flake declares x86_64-linux and aarch64-linux shells; the latter, macOS, and Windows were not tested in this session.

## Local dependency and toolchain check

- The package uses Rust edition 2024 and declares `rust-version = "1.85"`.
- The project flake pins Nixpkgs and the Rust overlay; `nix develop path:$PWD` provides Rust/Cargo/rustfmt/Clippy/rust-analyzer 1.85.0, `pkg-config`, GCC, libusb, Mesa/libGL, Wayland, X11, and debugging tools.
- The selected GUI crate versions report Rust 1.81 MSRV. `cargo fetch` resolved 300 packages using Cargo's Rust-1.85-compatible fallback, updated `Cargo.lock`, and downloaded the sources.
- The offline locked GUI build, `cargo fmt --all -- --check`, Clippy with warnings denied, and all-target tests succeeded inside the pinned Rust 1.85.0 shell. The test run reported 111 passing tests and no failures.

**Gate cleared:** GUI sources are present and the locked Linux native build resolves offline. The flake supports x86_64-linux and aarch64-linux; macOS/Windows builds are not claimed or run here.

## Existing project boundary

The current CLI depends on a presentation-independent library. `device::UsbDeviceInfo` is a detached USB descriptor snapshot, not a typed instrument-control session. The generator and analyzer modules are reserved for future operations. The sweep module provides a generic `SweepBackend<S>` boundary with stimulus, settle, and measure methods, and stores a multi-measurement dataset. The legacy `run_multi_sweep` API remains synchronous; `run_multi_sweep_controlled` adds progress and cancellation checks between points without changing the legacy API. No in-flight synchronous backend call is preempted.

The existing deterministic demonstration uses `MockAnalyzer` and a synthetic stimulus through `SweepBackend`; its values are explicitly identified as simulation, and it performs no hardware operation. It is a useful mock-only proof of concept, but it is not yet a reusable typed generator/analyzer device backend.

The existing CSV/JSON export functions accept both the `MultiSweepPlan` and `MultiSweepOutcome`. A GUI should therefore retain the immutable plan/configuration alongside the result, rather than attempting to reconstruct export metadata or repeat the sweep.

## Target architecture

```text
Desktop presentation (eframe/egui)
    ├── app state, form validation, plot selection, status/error display
    ├── command/event channel ───────────────────────────────┐
    └── plot adapter consumes saved result data only         │
                                                              ▼
Worker/controller (std thread; no UI-thread measurements)   library backend
    ├── progress/result/error events                          ├── typed instrument requests
    ├── cancellation checked at safe boundaries              ├── Mock backend first
    └── owns one backend for one run                           └── future ATS-2 backend
```

### Device and measurement API

- Keep `egui` types, widget state, and plot objects out of the library.
- Add a small typed backend/session boundary in the library before connecting a physical instrument. It should use domain values for generator stimulus, analyzer request, units, status, and errors; it must not expose `rusb` handles or make the GUI aware of USB/APIB details.
- Use an explicit device choice such as `Mock` versus `ATS-2 unavailable`. Until a verified ATS-2 control backend exists, the latter is a disabled “not implemented” option. The GUI must never issue direct USB commands.
- Route the GUI's mock mode through the existing sweep abstraction and deterministic mock data. Keep a persistent **SIMULATION / MOCK** indicator in the window and carry simulation provenance into saved results and exports.

### Worker, progress, and cancellation

The UI thread should only validate inputs, submit a run command, receive worker events, update state, and request repaint. A worker thread owns the selected backend and executes the sweep. Standard-library threads and channels are sufficient initially; no async runtime is required merely to keep the UI responsive.

The sweep core now exposes `run_multi_sweep_controlled`, which emits point progress and checks cancellation before each point. The GUI runs it on a worker thread and retains completed data. Cancellation cannot preempt an in-flight synchronous backend call; the UI reports that safe-point limitation. The original `run_multi_sweep` API and its failure semantics remain available.

### Reusable result plots (ATS-19)

Build plot series from the saved demonstration result, not from a measurement backend. X selection switches between simulated generator level, frequency, recorded RMS, and explicitly modeled measured power; Y selection toggles recorded RMS, THD+N, and THD. Percent series share a chart; RMS remains on its own unit axis. Linear/log transforms, zoom, pan, point selection/details, unit labels, and status are presentation-only, so changing the view never re-runs a sweep.

The measured-power X axis is derived from the mock RMS result and a user-supplied resistive-load assumption (`P = V_rms² / R`). When enabled, that single derived value is stored as the planned `power.w` measurement in the sweep dataset, so GUI plots and CSV/JSON exports share the same value and status. Exports include the resistance, formula, and RMS source measurement in metadata. Missing/failed power remains a failed/null measurement, never 0 W. This is simulation-only and does not command a power setpoint or establish the ATS-2's real output power. A commanded-power sweep and real output-power measurement remain unsupported until generator/analyzer control semantics and load measurement behavior are verified.

### Result/error and export handling

Preserve each point's measurement status, unit, failure stage, and error in the view. A failed point is displayed as failed/invalid rather than as a numeric zero. Reuse `export_multi_sweep_csv` and `export_multi_sweep_json` with the original plan, outcome, and explicit metadata; export must never trigger measurement.

## Scope and blockers

- **ATS-17:** architecture selected; exact crate versions, renderer, native libraries, MSRV, and locked build verified.
- **ATS-18:** mock-only main window, disabled future ATS-2 choice, generator/sweep controls, analyzer settings, result table, and status/error display implemented.
- **ATS-19:** saved-result XY charts, selectable X/Y metrics, unit-grouped series, linear/log axes, zoom/pan, and point detail implemented.
- **ATS-20:** worker-thread execution, point progress, safe-boundary cancellation, retained partial results, and CSV/JSON export implemented.

No hardware/USB operations or Windows execution were performed. Cargo dependencies and the lockfile were updated. No staging or commits were performed. The GUI process was not launched because this environment has neither X11 `DISPLAY` nor `WAYLAND_DISPLAY` set.
