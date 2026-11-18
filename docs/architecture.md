# Architecture

## Layering

```text
CLI (clap) / future GUI (egui/eframe)
  └── Application interface (thin presentation adapters)
       └── Instrument-control API (library; presentation-independent)
            ├── device       identity and immutable descriptor snapshots
            ├── generator    future instrument-level generator operations
            ├── analyzer     future instrument-level analyzer/acquisition operations
            ├── protocol     logical APIB records → pure framing → explicit transfer bridge
            ├── transport    USB access and descriptor/endpoint I/O
            └── error        shared, user-readable error types
```

The product is intended to be implemented entirely in Rust and to target Linux, Windows, and macOS. The initial CLI is only one frontend. A GUI, preferably using egui/eframe, is a later presentation layer; no GUI dependency or workspace restructuring is introduced now.

### Instrument-control boundary

Instrument operations belong to the Rust library, not to the CLI or a GUI. Frontends may collect user input, invoke the library's typed instrument API, and present results/errors; they must not implement APIB command semantics, USB transfer policy, or device state transitions themselves. The library must not depend on clap, egui/eframe, or frontend-specific state. This keeps the same control API usable from the CLI, a future GUI, and automated tests.

The intended dependency direction is one-way:

```text
CLI / GUI / tests → instrument-control library → protocol → transport
```

`device`, `generator`, and `analyzer` are library-facing domain layers. They may use verified protocol operations through the library, but must not import frontend code. Keep GUI and CLI as adapters around this API; do not move instrument control into their command handlers or event callbacks.

## Current data flow

`ats2 list|info` → `transport::usb::enumerate_devices()` → rusb/libusb enumeration and standard descriptor reads → detached `UsbDeviceInfo` snapshots → CLI formatting.

The USB library can open the uniquely matched adapter, validate configuration 1/interface 0/alternate setting 0 descriptors, and return a distinct opened-handle type. Interface claim is a separate explicit call; it verifies that configuration 1 and alternate setting 0 are already active without changing either. Bulk I/O is available only on the resulting claimed type and only when its transfer methods are explicitly invoked. No kernel driver is automatically detached.

## Protocol and control boundaries

- **Logical APIB operations** (`protocol::logical`) encode only the established operation tag and payload and decode exact-length logical responses. This layer has no endpoint, packet-size, marker, or transport assumptions.
- **USB framing** (`protocol::framing`) consumes already encoded logical records and explicit IN/OUT MPS values. It computes the verified marker bits, response offsets, and IN transfer length without USB I/O. It rejects OUT packet-tail gaps whose bytes the analyzed serializer does not write, and does not implement operation-specific segmentation.
- **USB transport** (`transport`) owns host APIs, discovery, descriptor validation, interface claiming, and bulk byte I/O mechanics. It must not assign meaning to unknown payload bytes. It never changes the active configuration/alternate setting or detaches a kernel driver.
- **Instrument control** remains a higher-level library domain for future generator/analyzer workflows. The current explicit transfer bridge can send caller-supplied logical records, but it adds no high-level measurement workflow or implicit device state transition.

The pure framing plan can be passed to the explicit `protocol::transfer::execute_records` bridge, which uses the mockable bulk transport and validates the actual returned response spans. This bridge is not called by `ats2 list` or `ats2 info`; no hardware integration test or automatic instrument initialization is connected.

## Separation rules

- **Transport** owns host APIs, discovery, descriptor snapshots, and later endpoint I/O mechanics. It must not assign meaning to unknown payload bytes.
- **Protocol** owns the verified logical APIB records, exact-length response decoding, and the separately parameterized framing plan. Unknown padding bytes, unverified device-side flag meanings, and transport timeouts remain outside the implementation.
- **Device** models stable identity and descriptor information, and later binds a protocol session to an instrument without exposing libusb types to higher layers.
- **Generator / analyzer** are intended as higher-level measurement operations, independent of USB implementation details. They must not duplicate transport concerns.
- **Error** carries context across layers without hiding transport/protocol distinctions.
- **CLI / future GUI** are presentation adapters only; do not put instrument control in frontend-specific handlers.

## API evolution

The existing 29 APIB export names are a planning reference only; they do not prove command encoding or instrument semantics. Public Rust APIs should expose coherent operations rather than reproduce a Windows DLL ABI. Add each protocol capability only when supported by verified static analysis or authorized bus traces.

## Suggested future tests

- Pure parsing/encoding tests with verified vectors before adding protocol code.
- Transport tests against descriptor snapshots or a fake transport, independent of physical hardware.
- Instrument API tests independent of CLI/GUI frameworks.
- Opt-in hardware integration tests, separately marked and never part of default CI.
