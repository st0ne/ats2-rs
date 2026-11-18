//! Errors exposed by the ATS-2 library.

/// Errors produced by USB discovery and descriptor reads.
#[derive(Debug, thiserror::Error)]
pub enum Ats2Error {
    /// The host USB stack failed to enumerate a device or read a descriptor.
    #[error("USB error: {0}")]
    Usb(#[from] rusb::Error),
}
