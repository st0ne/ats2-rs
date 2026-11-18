# Protocol status

## Implemented

- Read-only USB device enumeration and the descriptor-only `list`/`info` CLI.
- Explicit opening of the unique adapter matching VID `0x16D0` / PID `0x0AF0`; descriptor validation for configuration 1/interface 0/alternate setting 0 and bulk endpoints `0x06` OUT / `0x82` IN, including descriptor-derived MPS checks.
- Explicit interface claim that verifies the active configuration and current alternate setting without changing either; claim errors do not trigger kernel-driver detachment.
- Explicit, timeout-configurable bulk transfers with full OUT byte-count checks and actual IN byte counts preserved.
- Hardware-independent USB-handle and APIB transport mocks for claim failure, release, endpoint validation, short transfers, and framing-buffer handoff.
- An explicit `protocol::transfer::execute_records` library call; the CLI does not invoke it or perform APIB communication.
- Pure logical APIB codecs for `ap_read_byte`, `ap_write_byte`, `ap_read_word`, and `ap_any_ready`.
- Exact-length decoders for 8-bit, 16-bit little-endian, and the documented ready-bit expression; invalid lengths return a typed error.
- Hardware-independent framing plans for encoded logical records with explicit IN/OUT MPS values. The plan contains the deterministic OUT bytes, expected IN request length, and checked logical response spans; it does not perform transfers.

The library can open the supported adapter and explicitly claim interface 0 after verifying that configuration 1 and alternate setting 0 are already active. It never sets a configuration or alternate setting, detaches a kernel driver, loads firmware, resets the device, or issues vendor-control requests. Bulk transfers happen only through an explicit call; neither opening nor claiming sends APIB bulk data, and the CLI has no communication command.

The logical codecs encode only the analyzed operation tag and payload. USB marker bits and MPS-dependent calculations are applied only by `protocol::framing`; `protocol::transfer::execute_records` is the separate, explicit I/O bridge and validates actual response spans before returning.

## Verified framing behavior and test vectors

The serializer places each operation tag at the first byte of its logical record and appends that record's payload. The marker bits are applied by OR according to observed control flow:

- `0x80`: set when a record payload exactly consumes the remaining OUT packet space, when the serializer advances past a packet tail to fit the next record, and on the final record.
- `0x40`: set on a record whose response ends exactly at an IN packet boundary. If a response is larger than the remaining IN packet tail, the tail is skipped and `0x40` is ORed into the preceding record's tag.
- `0x20`: not set alone in the analyzed path. When the aggregate IN cursor (including skipped tails) is not MPS-aligned, the DLL ORs `0x60` (`0x20 | 0x40`) into the last response-bearing tag and rounds the requested IN length up to the next MPS multiple.

For the four single-operation vectors below, the OUT bytes are the same with MPS 64 and 512; the expected IN length is one MPS for reads and zero for `ap_write_byte`. Response spans begin at offset 0.

| Operation | Framed OUT bytes | Expected IN (MPS 64) | Expected IN (MPS 512) | Response span |
|---|---|---:|---:|---|
| `ap_read_byte(0x12)` | `EB 01 12` | 64 | 512 | offset 0, length 1 |
| `ap_write_byte(0x12345678, 0x2A)` | `8A 01 2A 78` | 0 | 0 | none |
| `ap_read_word(0x12)` | `EB 02 12` | 64 | 512 | offset 0, length 2 |
| `ap_any_ready()` | `E0 01` | 64 | 512 | offset 0, length 1 |

The boundary tests also cover 63 one-byte responses followed by a two-byte response with IN MPS 64. The last response starts at offset 64 because the one-byte packet tail is skipped; the requested IN length becomes 128 after final alignment, and the preceding record's tag receives `0x40`.

Both static pipe buffers are 8 KiB. Rust rejects larger calculated buffers; the DLL's own length checks are not uniform across all control-flow paths. The serializer does not write bytes in an OUT tail it skips, so such a queue is rejected rather than filled with guessed zeros.

These are DLL-derived buffers and requested lengths, not proof that the device accepts or interprets them as intended. The DLL checks the completed OUT byte count, but the analyzed read path does not compare the actual IN byte count before response decoding. `TransferPlan::response_bytes` instead verifies that the received slice contains the complete logical span before passing it to a logical decoder; it does not require a short USB read to equal the padded request length.

## Deliberately not implemented

- Operation-specific segmentation for records larger than one packet, and deterministic OUT bytes for packet tails the DLL skips without writing.
- Device-side meaning of framing marker bits, response status/error bytes, checksums, transaction sequencing, retries, or initialization.
- Device-level meaning of operation arguments and response values beyond the exact transformations implemented in the pure codec.
- Generator, analyzer, or measurement operations.
- Vendor-specific descriptor requests or USB string-descriptor fetching.

The legacy APIB function surface is a roadmap, not a wire-protocol specification. Unknown values are not inferred from names or descriptor sizes.

## Evidence required before expanding protocol/transport support

1. Determine whether skipped OUT packet-tail bytes are initialized outside the serializer and reconstruct operation-specific segmentation before widening `protocol::framing` beyond the implemented record subset.
2. Confirm response status/error behavior and actual short-read semantics before connecting logical decoders to a transport.
3. Confirm any proposed transport behavior with authorized bus evidence; the pure framing plan does not send data, while the separately invoked `protocol::transfer::execute_records` performs only the supplied explicit bulk batch. No automatic initialization or CLI communication path exists.
4. Keep tests synthetic and independent of hardware; do not commit proprietary binaries or unapproved captures.

## Hardware validation

A later, separately authorized Beagle USB 480 session should begin with read-only discovery/status operations and explicitly document device state. Any write operation requires an isolated test plan. Hardware tests must be opt-in and excluded from the default test suite.
