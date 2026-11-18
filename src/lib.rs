//! Native ATS-2 control library.
//!
//! This crate provides USB discovery and an opt-in bulk transport. Opening a
//! device does not claim an interface or execute APIB operations; callers must
//! explicitly claim and invoke the library transfer API.

pub mod analyzer;
pub mod device;
pub mod error;
pub mod generator;
pub mod protocol;
pub mod sweep;
pub mod transport;

#[cfg(test)]
mod tests {
    use crate::device::is_supported_adapter;

    #[test]
    fn recognizes_only_the_documented_adapter_vid_pid() {
        assert!(is_supported_adapter(0x16d0, 0x0af0));
        assert!(!is_supported_adapter(0x16d0, 0x0af1));
        assert!(!is_supported_adapter(0x0000, 0x0000));
    }
}
