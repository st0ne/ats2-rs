//! Logical APIB operation encodings and response decoders, separate from USB.
//!
//! [`logical`] contains only fields established by static analysis. [`framing`]
//! applies verified marker, packet-alignment, and response-offset rules without
//! I/O. [`transfer`] performs transfers only after an explicit caller invocation.

pub mod apio_v3;
pub mod apio_v3_transport_plan;
pub mod framing;
pub mod logical;
pub mod transfer;
