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

/// Largest address value that can be passed to the observed legacy
/// `ap_write_to_dsp_addr` byte/status path.
pub const LOW_24_BIT_MAX: u32 = 0x00ff_ffff;

/// The only retry-limit argument established for the observed byte/status path.
pub const AP_WRITE_TO_DSP_RETRY_LIMIT: u8 = 10;

/// Explicit caller-supplied arguments for one conditional DSP data-byte step.
///
/// This is a host-side model only. In particular, callers remain responsible for
/// the DspObj-presence, status, and legacy-backend preconditions established by
/// static analysis; this type neither checks nor invents those conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApibWriteToDspByteInput {
    /// APIO object argument, whose device-level meaning is not established.
    pub object: u8,
    /// Address argument, constrained to the observed low-24-bit domain.
    pub address: u32,
    /// First caller-supplied APIO context token.
    pub token9: u8,
    /// One DSP data byte.
    pub data: u8,
    /// Second caller-supplied APIO context token.
    pub token13: u8,
    /// Explicit APIO retry-limit argument; the observed sequence uses 10.
    pub retry_limit: u8,
}

/// Complete logical argument record for the legacy `ap_write_tx` primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApWriteTxArguments {
    /// Caller-supplied APIO object argument.
    pub object: u8,
    /// The primitive's 24-bit value argument represented losslessly as `u32`.
    pub value: u32,
    /// Explicit APIO retry-limit argument.
    pub retry_limit: u8,
}

/// Complete logical argument record for the legacy `ap_hv` primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApHvArguments {
    /// Caller-supplied APIO object argument.
    pub object: u8,
    /// Caller-supplied APIO token; no device meaning is inferred.
    pub token: u8,
    /// Explicit APIO retry-limit argument.
    pub retry_limit: u8,
}

/// One non-serialized logical primitive in the observed legacy byte/status path.
///
/// These records intentionally do not reuse [`ApibOperation`]: its variants are
/// established Common-APIB tag encodings, whereas no USB/APIB frame encoding is
/// established here for `ap_write_tx` or `ap_hv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyApibOperation {
    /// A legacy `ap_write_tx` call with its complete host arguments.
    WriteTx(ApWriteTxArguments),
    /// A legacy `ap_hv` call with its complete host arguments.
    Hv(ApHvArguments),
}

/// A fixed four-record host transaction plan for exactly one DSP data byte.
///
/// The plan represents the static modes-0/2 call order only. It performs no
/// serialization, USB/APIB transfer, retry, sleep, poll, or status handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyApibWriteToDspBytePlan {
    operations: [LegacyApibOperation; 4],
}

/// Failure to construct the bounded static `ap_write_to_dsp_addr` byte plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ApibWriteToDspPlanError {
    /// The address cannot be represented by the observed low-24-bit argument.
    #[error("DSP address 0x{address:08x} exceeds the low-24-bit domain")]
    AddressOutsideLow24 {
        /// Caller-supplied address.
        address: u32,
    },
    /// The static sequence establishes only the literal retry limit of 10.
    #[error(
        "retry limit {supplied} is not established for the ap_write_to_dsp_addr byte plan; expected {AP_WRITE_TO_DSP_RETRY_LIMIT}"
    )]
    UnsupportedRetryLimit {
        /// Caller-supplied retry limit.
        supplied: u8,
    },
}

impl LegacyApibWriteToDspBytePlan {
    /// Builds the exact four legacy primitive records for one conditional byte.
    ///
    /// The caller explicitly supplies all APIO arguments. This method validates
    /// only the low-24-bit address domain and the observed retry limit; it does
    /// not infer object presence, tokens, backend selection, or status results.
    pub fn build(input: ApibWriteToDspByteInput) -> Result<Self, ApibWriteToDspPlanError> {
        if input.address > LOW_24_BIT_MAX {
            return Err(ApibWriteToDspPlanError::AddressOutsideLow24 {
                address: input.address,
            });
        }
        if input.retry_limit != AP_WRITE_TO_DSP_RETRY_LIMIT {
            return Err(ApibWriteToDspPlanError::UnsupportedRetryLimit {
                supplied: input.retry_limit,
            });
        }

        Ok(Self {
            operations: [
                LegacyApibOperation::WriteTx(ApWriteTxArguments {
                    object: input.object,
                    value: input.address,
                    retry_limit: input.retry_limit,
                }),
                LegacyApibOperation::Hv(ApHvArguments {
                    object: input.object,
                    token: input.token9,
                    retry_limit: input.retry_limit,
                }),
                LegacyApibOperation::WriteTx(ApWriteTxArguments {
                    object: input.object,
                    value: u32::from(input.data),
                    retry_limit: input.retry_limit,
                }),
                LegacyApibOperation::Hv(ApHvArguments {
                    object: input.object,
                    token: input.token13,
                    retry_limit: input.retry_limit,
                }),
            ],
        })
    }

    /// Returns the fixed four records in the statically established call order.
    #[must_use]
    pub const fn operations(&self) -> &[LegacyApibOperation; 4] {
        &self.operations
    }
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
        ApHvArguments, ApWriteTxArguments, ApibDecodeError, ApibOperation, ApibResponse,
        ApibWriteToDspByteInput, ApibWriteToDspPlanError, LegacyApibOperation,
        LegacyApibWriteToDspBytePlan, decode_ready_status, decode_u8, decode_u16_le,
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

    #[test]
    fn builds_one_write_to_dsp_byte_step_in_the_established_legacy_order() {
        let input = ApibWriteToDspByteInput {
            object: 0x40,
            address: 0x00ff_ff26,
            token9: 0x10,
            data: 0xa5,
            token13: 0x27,
            retry_limit: 10,
        };

        let plan = LegacyApibWriteToDspBytePlan::build(input).unwrap();

        assert_eq!(
            plan.operations(),
            &[
                LegacyApibOperation::WriteTx(ApWriteTxArguments {
                    object: 0x40,
                    value: 0x00ff_ff26,
                    retry_limit: 10,
                }),
                LegacyApibOperation::Hv(ApHvArguments {
                    object: 0x40,
                    token: 0x10,
                    retry_limit: 10,
                }),
                LegacyApibOperation::WriteTx(ApWriteTxArguments {
                    object: 0x40,
                    value: 0x0000_00a5,
                    retry_limit: 10,
                }),
                LegacyApibOperation::Hv(ApHvArguments {
                    object: 0x40,
                    token: 0x27,
                    retry_limit: 10,
                }),
            ]
        );
    }

    #[test]
    fn accepts_the_full_low_24_bit_address_boundary() {
        let plan = LegacyApibWriteToDspBytePlan::build(ApibWriteToDspByteInput {
            object: 0,
            address: 0x00ff_ffff,
            token9: 0,
            data: 0,
            token13: 0,
            retry_limit: 10,
        })
        .unwrap();

        assert_eq!(
            plan.operations()[0],
            LegacyApibOperation::WriteTx(ApWriteTxArguments {
                object: 0,
                value: 0x00ff_ffff,
                retry_limit: 10,
            })
        );
    }

    #[test]
    fn rejects_addresses_outside_the_low_24_bit_input_domain() {
        assert_eq!(
            LegacyApibWriteToDspBytePlan::build(ApibWriteToDspByteInput {
                object: 0,
                address: 0x0100_0000,
                token9: 0,
                data: 0,
                token13: 0,
                retry_limit: 10,
            }),
            Err(ApibWriteToDspPlanError::AddressOutsideLow24 {
                address: 0x0100_0000,
            })
        );
    }

    #[test]
    fn rejects_a_retry_limit_not_established_for_this_plan() {
        assert_eq!(
            LegacyApibWriteToDspBytePlan::build(ApibWriteToDspByteInput {
                object: 0,
                address: 0,
                token9: 0,
                data: 0,
                token13: 0,
                retry_limit: 9,
            }),
            Err(ApibWriteToDspPlanError::UnsupportedRetryLimit { supplied: 9 })
        );
    }
}
