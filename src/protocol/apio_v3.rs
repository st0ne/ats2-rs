//! Pure APIO-v3 codec for `ap_write_to_dsp_addr` (logical tag `0x0c`).
//!
//! This module models the logical tag-`0x0c` payload and its one-byte logical
//! response only. The three-byte fields are checked raw 24-bit values emitted
//! most-significant byte first; they are not assigned address, register, or
//! device semantics. This module contains no framing, transfer, retry, status,
//! or I/O behavior and does not imply that a device accepts the operation.

use thiserror::Error;

/// Logical APIO-v3 tag selected by this codec.
pub const AP_WRITE_TO_DSP_ADDR_TAG: u8 = 0x0c;
/// Number of logical payload bytes encoded for this operation.
pub const AP_WRITE_TO_DSP_ADDR_PAYLOAD_LEN: usize = 10;
/// Number of logical response bytes expected for this operation.
pub const AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN: usize = 1;

/// Codec error for the statically established `ap_write_to_dsp_addr` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ApioV3WriteToDspAddrError {
    /// A raw source value cannot be represented by the callback's low-24-bit
    /// byte extraction without silently discarding bits.
    #[error("value 0x{value:08x} exceeds the checked raw 24-bit field")]
    ValueOutsideU24 {
        /// Caller-provided raw source value.
        value: u32,
    },
    /// The logical response did not have the exact callback-planned length.
    #[error("invalid ap_write_to_dsp_addr response length: expected {expected}, got {actual}")]
    InvalidResponseLength {
        /// Required logical response length.
        expected: usize,
        /// Received logical response length.
        actual: usize,
    },
}

/// A checked raw 24-bit callback source field.
///
/// This is intentionally not a signed integer: the concrete encoder only loads
/// bytes, so a signed interpretation is not directly established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct U24(u32);

impl TryFrom<u32> for U24 {
    type Error = ApioV3WriteToDspAddrError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value > 0x00ff_ffff {
            return Err(ApioV3WriteToDspAddrError::ValueOutsideU24 { value });
        }
        Ok(Self(value))
    }
}

impl U24 {
    /// Returns the checked raw field value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    const fn to_msb_bytes(self) -> [u8; 3] {
        [(self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8]
    }
}

/// Raw six-argument input record for APIO-v3 `ap_write_to_dsp_addr`.
///
/// The field names describe this host API's raw input order and do not assign
/// device-level field semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApWriteToDspAddr {
    first_u24: U24,
    second_u8: u8,
    third_u8: u8,
    fourth_u8: u8,
    fifth_u24: U24,
    sixth_u8: u8,
}

impl ApWriteToDspAddr {
    /// Creates a typed raw record for the only evidenced tag-`0x0c` codec.
    #[must_use]
    pub const fn new(
        first_u24: U24,
        second_u8: u8,
        third_u8: u8,
        fourth_u8: u8,
        fifth_u24: U24,
        sixth_u8: u8,
    ) -> Self {
        Self {
            first_u24,
            second_u8,
            third_u8,
            fourth_u8,
            fifth_u24,
            sixth_u8,
        }
    }

    /// Encodes the ten callback-defined logical payload bytes, without a tag or
    /// any packet framing.
    #[must_use]
    pub const fn encode_payload(self) -> [u8; AP_WRITE_TO_DSP_ADDR_PAYLOAD_LEN] {
        let first = self.first_u24.to_msb_bytes();
        let fifth = self.fifth_u24.to_msb_bytes();
        [
            self.second_u8,
            self.sixth_u8,
            first[0],
            first[1],
            first[2],
            self.third_u8,
            fifth[0],
            fifth[1],
            fifth[2],
            self.fourth_u8,
        ]
    }

    /// Encodes the tag and ten logical payload bytes, without USB framing.
    #[must_use]
    pub const fn encode(self) -> [u8; AP_WRITE_TO_DSP_ADDR_PAYLOAD_LEN + 1] {
        let payload = self.encode_payload();
        [
            AP_WRITE_TO_DSP_ADDR_TAG,
            payload[0],
            payload[1],
            payload[2],
            payload[3],
            payload[4],
            payload[5],
            payload[6],
            payload[7],
            payload[8],
            payload[9],
        ]
    }
}

/// Decodes the sole logical response byte as a nonzero boolean.
///
/// `true` means only that the byte was nonzero; no acceptance, status, or
/// device-effect meaning is inferred.
pub fn decode_ap_write_to_dsp_addr_response(
    response: &[u8],
) -> Result<bool, ApioV3WriteToDspAddrError> {
    if response.len() != AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN {
        return Err(ApioV3WriteToDspAddrError::InvalidResponseLength {
            expected: AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN,
            actual: response.len(),
        });
    }
    Ok(response[0] != 0)
}

#[cfg(test)]
mod tests {
    use super::{
        AP_WRITE_TO_DSP_ADDR_TAG, ApWriteToDspAddr, ApioV3WriteToDspAddrError, U24,
        decode_ap_write_to_dsp_addr_response,
    };

    #[test]
    fn encodes_tag_and_all_ten_payload_offsets_from_the_callback_fields() {
        let operation = ApWriteToDspAddr::new(
            U24::try_from(0x0012_3456).unwrap(),
            0xa2,
            0xb3,
            0xc4,
            U24::try_from(0x0000_d5e6).unwrap(),
            0xf6,
        );

        assert_eq!(
            operation.encode(),
            [
                AP_WRITE_TO_DSP_ADDR_TAG,
                0xa2, // payload[0]: second_u8
                0xf6, // payload[1]: sixth_u8
                0x12, // payload[2]: first_u24 bits 23..16
                0x34, // payload[3]: first_u24 bits 15..8
                0x56, // payload[4]: first_u24 bits 7..0
                0xb3, // payload[5]: third_u8
                0x00, // payload[6]: fifth_u24 bits 23..16
                0xd5, // payload[7]: fifth_u24 bits 15..8
                0xe6, // payload[8]: fifth_u24 bits 7..0
                0xc4, // payload[9]: fourth_u8
            ]
        );
    }

    #[test]
    fn encodes_zero_and_maximum_checked_u24_values_msb_first() {
        let operation = ApWriteToDspAddr::new(
            U24::try_from(0).unwrap(),
            0,
            0,
            0,
            U24::try_from(0x00ff_ffff).unwrap(),
            0,
        );

        assert_eq!(
            operation.encode_payload(),
            [0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0]
        );
    }

    #[test]
    fn rejects_values_that_do_not_fit_the_callback_u24_sources() {
        assert_eq!(
            U24::try_from(0x0100_0000),
            Err(ApioV3WriteToDspAddrError::ValueOutsideU24 { value: 0x0100_0000 })
        );
    }

    #[test]
    fn decodes_the_callback_response_as_nonzero_boolean_only_at_exact_length() {
        assert_eq!(decode_ap_write_to_dsp_addr_response(&[0]), Ok(false));
        assert_eq!(decode_ap_write_to_dsp_addr_response(&[0x80]), Ok(true));

        for response in [&[][..], &[0, 1][..]] {
            assert_eq!(
                decode_ap_write_to_dsp_addr_response(response),
                Err(ApioV3WriteToDspAddrError::InvalidResponseLength {
                    expected: 1,
                    actual: response.len(),
                })
            );
        }
    }
}
