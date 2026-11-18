use thiserror::Error;

/// One of the logical APIB operations established by static analysis.
///
/// The argument names are intentionally neutral: their device-level meaning is
/// not established. Encoded bytes contain only the operation tag and payload;
/// USB framing and transport are separate layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApibOperation {
    /// Read an 8-bit result using the supplied one-byte argument.
    ReadByte { argument: u8 },
    /// Write `value` with a 32-bit first argument; only that argument's low byte
    /// is serialized by the analyzed DLL operation.
    WriteByte { argument: u32, value: u8 },
    /// Read a 16-bit result using the supplied one-byte argument.
    ReadWord { argument: u8 },
    /// Query the one-byte status used by the DLL's `ap_any_ready` wrapper.
    AnyReady,
}

/// Decoded response of one supported logical APIB operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApibResponse {
    /// The operation has no logical response payload.
    NoData,
    /// An 8-bit logical result.
    Byte(u8),
    /// A 16-bit little-endian logical result.
    Word(u16),
    /// The 0/1 value returned by the DLL's active-low bit-three expression.
    Ready(bool),
}

/// A malformed logical APIB response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ApibDecodeError {
    /// The response length did not match the exact logical payload length.
    #[error("invalid logical APIB response length: expected {expected} byte(s), got {actual}")]
    InvalidResponseLength { expected: usize, actual: usize },
}

impl ApibOperation {
    /// Encodes the verified logical tag and payload, without USB framing.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        match self {
            Self::ReadByte { argument } => vec![0x0b, 0x01, argument],
            Self::WriteByte { argument, value } => {
                vec![0x0a, 0x01, value, argument.to_le_bytes()[0]]
            }
            Self::ReadWord { argument } => vec![0x0b, 0x02, argument],
            Self::AnyReady => vec![0x00, 0x01],
        }
    }

    /// Decodes this operation's logical response, rejecting any length mismatch.
    pub fn decode_response(self, response: &[u8]) -> Result<ApibResponse, ApibDecodeError> {
        match self {
            Self::ReadByte { .. } => decode_u8(response).map(ApibResponse::Byte),
            Self::WriteByte { .. } => {
                validate_length(response, 0)?;
                Ok(ApibResponse::NoData)
            }
            Self::ReadWord { .. } => decode_u16_le(response).map(ApibResponse::Word),
            Self::AnyReady => decode_ready_status(response).map(ApibResponse::Ready),
        }
    }
}

/// Decodes exactly one byte as a logical APIB result.
pub fn decode_u8(response: &[u8]) -> Result<u8, ApibDecodeError> {
    validate_length(response, 1)?;
    Ok(response[0])
}

/// Decodes exactly two bytes as an unsigned little-endian logical APIB result.
pub fn decode_u16_le(response: &[u8]) -> Result<u16, ApibDecodeError> {
    validate_length(response, 2)?;
    Ok(u16::from_le_bytes([response[0], response[1]]))
}

/// Applies the DLL's documented `(~value >> 3) & 1` ready-status expression.
///
/// `true` means the expression returned 1; no additional hardware semantics are
/// inferred here.
pub fn decode_ready_status(response: &[u8]) -> Result<bool, ApibDecodeError> {
    let value = decode_u8(response)?;
    Ok(((!value >> 3) & 1) != 0)
}

fn validate_length(response: &[u8], expected: usize) -> Result<(), ApibDecodeError> {
    if response.len() != expected {
        return Err(ApibDecodeError::InvalidResponseLength {
            expected,
            actual: response.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ApibDecodeError, ApibOperation, ApibResponse, decode_ready_status, decode_u8, decode_u16_le,
    };

    #[test]
    fn encodes_the_documented_logical_vectors() {
        assert_eq!(
            ApibOperation::ReadByte { argument: 0x42 }.encode(),
            [0x0b, 0x01, 0x42]
        );
        assert_eq!(
            ApibOperation::WriteByte {
                argument: 0x1234_5678,
                value: 0x2a,
            }
            .encode(),
            [0x0a, 0x01, 0x2a, 0x78]
        );
        assert_eq!(
            ApibOperation::ReadWord { argument: 0xa5 }.encode(),
            [0x0b, 0x02, 0xa5]
        );
        assert_eq!(ApibOperation::AnyReady.encode(), [0x00, 0x01]);
    }

    #[test]
    fn decodes_u8_responses() {
        assert_eq!(decode_u8(&[0xab]), Ok(0xab));
        assert_eq!(
            ApibOperation::ReadByte { argument: 0 }.decode_response(&[0xab]),
            Ok(ApibResponse::Byte(0xab))
        );
    }

    #[test]
    fn decodes_u16_responses_as_little_endian() {
        assert_eq!(decode_u16_le(&[0x34, 0x12]), Ok(0x1234));
        assert_eq!(
            ApibOperation::ReadWord { argument: 0 }.decode_response(&[0x34, 0x12]),
            Ok(ApibResponse::Word(0x1234))
        );
    }

    #[test]
    fn rejects_incomplete_and_oversized_responses_with_typed_errors() {
        for response in [&[][..], &[0x01, 0x02][..]] {
            assert_eq!(
                decode_u8(response),
                Err(ApibDecodeError::InvalidResponseLength {
                    expected: 1,
                    actual: response.len(),
                })
            );
        }

        for response in [&[][..], &[0x01][..], &[0x01, 0x02, 0x03][..]] {
            assert_eq!(
                decode_u16_le(response),
                Err(ApibDecodeError::InvalidResponseLength {
                    expected: 2,
                    actual: response.len(),
                })
            );
        }

        assert_eq!(
            ApibOperation::WriteByte {
                argument: 0,
                value: 0,
            }
            .decode_response(&[0x00]),
            Err(ApibDecodeError::InvalidResponseLength {
                expected: 0,
                actual: 1,
            })
        );
    }

    #[test]
    fn ready_status_uses_the_documented_active_low_bit_three() {
        assert_eq!(decode_ready_status(&[0xf7]), Ok(true));
        assert_eq!(decode_ready_status(&[0xff]), Ok(false));
        assert_eq!(
            ApibOperation::AnyReady.decode_response(&[0xf7]),
            Ok(ApibResponse::Ready(true))
        );
        assert_eq!(
            ApibOperation::AnyReady.decode_response(&[0xff]),
            Ok(ApibResponse::Ready(false))
        );
    }

    #[test]
    fn encodes_argument_boundaries_without_widening_or_semantic_assumptions() {
        assert_eq!(
            ApibOperation::ReadByte { argument: 0 }.encode(),
            [0x0b, 1, 0]
        );
        assert_eq!(
            ApibOperation::ReadByte { argument: u8::MAX }.encode(),
            [0x0b, 1, 0xff]
        );
        assert_eq!(
            ApibOperation::ReadWord { argument: u8::MAX }.encode(),
            [0x0b, 2, 0xff]
        );
        assert_eq!(
            ApibOperation::WriteByte {
                argument: 0,
                value: 0,
            }
            .encode(),
            [0x0a, 1, 0, 0]
        );
        assert_eq!(
            ApibOperation::WriteByte {
                argument: u32::MAX,
                value: u8::MAX,
            }
            .encode(),
            [0x0a, 1, 0xff, 0xff]
        );
    }
}
