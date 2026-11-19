//! Pure host-side framing plan for one APIO-v3 `ap_write_to_dsp_addr` record.
//!
//! This module composes the evidenced logical tag-`0x0c` encoder with the
//! separately evidenced APIB framing rules. It returns deterministic host
//! buffers and response spans only; it performs no USB I/O, does not invoke the
//! transport bridge, and does not decode or assign device-level meaning to the
//! one planned response byte.

use thiserror::Error;

use crate::generator::{DspWriteData, Single1KhzSineVector};

use super::{
    apio_v3::{AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN, ApWriteToDspAddr, U24},
    framing::{FramingError, LogicalRecord, TransferPlan, frame_records},
};

const LEVEL_LOW_BYTE_OFFSET: u32 = 0x06;
const LEVEL_HIGH_BYTE_OFFSET: u32 = 0x07;
const Q_MINUS_C_OFFSET: u32 = 0x30;
const Q_PLUS_C_OFFSET: u32 = 0x31;
const AP_WRITE_TO_DSP_ADDR_RETRY_LIMIT: u8 = 10;

/// Explicit APIO-v3 framing inputs for the ordered generator level-byte pair.
///
/// The caller supplies every field. No level is quantized or derived, and no
/// channel, output, or device-base mapping is inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelPairPlanInputs {
    /// Explicit checked low-24-bit base to which the static byte offsets are added.
    pub base: U24,
    /// Explicit APIO-v3 object byte.
    pub object: u8,
    /// Explicit APIO-v3 token passed in the codec's ninth-byte position.
    pub token9: u8,
    /// Explicit APIO-v3 token passed in the codec's thirteenth-byte position.
    pub token13: u8,
    /// Explicit retry-limit argument; only the statically evidenced value 10 is accepted.
    pub retry_limit: u8,
    /// Explicit OUT maximum packet size for framing.
    pub out_mps: u16,
    /// Explicit IN maximum packet size for framing.
    pub in_mps: u16,
}

/// Fail-closed errors while translating two encoded generator level bytes into
/// APIO-v3 tag-`0x0c` host-plan records.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LevelPairPlanError {
    /// The explicit low-24-bit base cannot be advanced by the required offset.
    #[error("base 0x{base:06x} plus required offset 0x{offset:02x} exceeds U24")]
    AddressOverflow {
        /// Caller-supplied checked base field.
        base: u32,
        /// Required static pair offset.
        offset: u32,
    },
    /// The encoded bytes did not retain the established low-then-high offsets.
    #[error(
        "level byte entry {operation_index} has internal address 0x{actual:08x}; expected 0x{expected:02x}"
    )]
    UnexpectedLevelOffset {
        /// Index within the expected `(low, high)` pair.
        operation_index: usize,
        /// Required internal low identifier.
        expected: u32,
        /// Actual caller-supplied identifier.
        actual: u32,
    },
    /// An already-encoded level datum cannot losslessly represent one byte.
    #[error("level byte entry {operation_index} data {data} is outside u8")]
    DataOutsideByte {
        /// Index within the expected `(low, high)` pair.
        operation_index: usize,
        /// Caller-provided encoded software byte.
        data: i32,
    },
    /// The static sequence establishes only this explicit retry-limit argument.
    #[error(
        "retry limit {supplied} is not established for the level-pair plan; expected {AP_WRITE_TO_DSP_ADDR_RETRY_LIMIT}"
    )]
    UnsupportedRetryLimit {
        /// Caller-provided limit.
        supplied: u8,
    },
    /// The generic APIB framing model rejected the resulting two-record plan.
    #[error(transparent)]
    Framing(#[from] FramingError),
}

/// Explicit APIO-v3 framing inputs for the ordered ATS-33 frequency pair.
///
/// The caller selects every value. No base, object, token, retry limit, or
/// packet size is derived from a channel, device, adapter, or runtime mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrequencyPairPlanInputs {
    /// Explicit checked low-24-bit base to which the pair offsets are added.
    pub base: U24,
    /// Explicit APIO-v3 object byte.
    pub object: u8,
    /// Explicit APIO-v3 token passed in the codec's ninth-byte position.
    pub token9: u8,
    /// Explicit APIO-v3 token passed in the codec's twelfth-byte position.
    pub token12: u8,
    /// Explicit retry-limit argument; only the statically evidenced value 10 is accepted.
    pub retry_limit: u8,
    /// Explicit OUT maximum packet size for framing.
    pub out_mps: u16,
    /// Explicit IN maximum packet size for framing.
    pub in_mps: u16,
}

/// Fail-closed errors while translating the ATS-33 frequency pair into the
/// APIO-v3 tag-`0x0c` host-plan inputs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrequencyPairPlanError {
    /// The explicit low-24-bit base cannot be advanced by the required offset.
    #[error("base 0x{base:06x} plus required offset 0x{offset:02x} exceeds U24")]
    AddressOverflow {
        /// Caller-supplied checked base field.
        base: u32,
        /// Required static pair offset.
        offset: u32,
    },
    /// The vector did not retain the statically established pair ordering.
    #[error(
        "frequency vector entry {operation_index} has internal address 0x{actual:08x}; expected 0x{expected:02x}"
    )]
    UnexpectedFrequencyAddress {
        /// Index within the expected `(q-C, q+C)` pair.
        operation_index: usize,
        /// Required internal low identifier.
        expected: u32,
        /// Actual vector identifier.
        actual: u32,
    },
    /// A signed ATS-33 software result has no lossless APIO-v3 U24 encoding.
    #[error("frequency vector entry {operation_index} data {data} is outside U24")]
    DataOutsideU24 {
        /// Index within the expected `(q-C, q+C)` pair.
        operation_index: usize,
        /// Caller-provided calculated software value.
        data: i32,
    },
    /// The static sequence establishes only this explicit retry-limit argument.
    #[error(
        "retry limit {supplied} is not established for the frequency-pair plan; expected {AP_WRITE_TO_DSP_ADDR_RETRY_LIMIT}"
    )]
    UnsupportedRetryLimit {
        /// Caller-provided limit.
        supplied: u8,
    },
    /// The generic APIB framing model rejected the resulting two-record plan.
    #[error(transparent)]
    Framing(#[from] FramingError),
}

/// Builds a hardware-independent host plan for exactly one encoded tag-`0x0c`
/// `ap_write_to_dsp_addr` record.
///
/// Both maximum-packet-size values are explicit because framing is MPS
/// dependent. The returned [`TransferPlan`] requests exactly the codec's one
/// logical response byte, while leaving response decoding and any device
/// interpretation separate.
pub fn plan_ap_write_to_dsp_addr(
    operation: ApWriteToDspAddr,
    out_mps: u16,
    in_mps: u16,
) -> Result<TransferPlan, FramingError> {
    let encoded = operation.encode();
    frame_records(
        &[LogicalRecord {
            bytes: &encoded,
            response_len: AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN,
        }],
        out_mps,
        in_mps,
    )
}

/// Builds exactly two APIO-v3 records for already-encoded generator level
/// bytes, in established low-byte then high-byte order.
///
/// `level_bytes` is supplied directly by the caller and is validated only for
/// the static offsets `0x06` and `0x07` and lossless byte representation. This
/// function performs no level quantization and makes no base-to-output or
/// channel assignment.
pub fn build_generator_level_records(
    level_bytes: [DspWriteData; 2],
    base: U24,
    object: u8,
    token9: u8,
    token13: u8,
    retry_limit: u8,
) -> Result<[ApWriteToDspAddr; 2], LevelPairPlanError> {
    if retry_limit != AP_WRITE_TO_DSP_ADDR_RETRY_LIMIT {
        return Err(LevelPairPlanError::UnsupportedRetryLimit {
            supplied: retry_limit,
        });
    }

    let [low_byte, high_byte] = level_bytes;
    validate_level_entry(0, low_byte, LEVEL_LOW_BYTE_OFFSET)?;
    validate_level_entry(1, high_byte, LEVEL_HIGH_BYTE_OFFSET)?;

    Ok([
        ApWriteToDspAddr::new(
            add_level_offset(base, LEVEL_LOW_BYTE_OFFSET)?,
            object,
            token9,
            token13,
            level_data_as_u24(0, low_byte.data)?,
            retry_limit,
        ),
        ApWriteToDspAddr::new(
            add_level_offset(base, LEVEL_HIGH_BYTE_OFFSET)?,
            object,
            token9,
            token13,
            level_data_as_u24(1, high_byte.data)?,
            retry_limit,
        ),
    ])
}

/// Frames the two generator level-byte records in one deterministic,
/// host-only APIO-v3 transfer plan.
///
/// The plan reserves one logical response byte per record. It performs no
/// transport, USB, APIB, retry, sleep, firmware, or hardware action; framing
/// success does not establish device acceptance or analog level behavior.
pub fn plan_generator_level_pair(
    level_bytes: [DspWriteData; 2],
    inputs: LevelPairPlanInputs,
) -> Result<TransferPlan, LevelPairPlanError> {
    let [first, second] = build_generator_level_records(
        level_bytes,
        inputs.base,
        inputs.object,
        inputs.token9,
        inputs.token13,
        inputs.retry_limit,
    )?;
    let first_encoded = first.encode();
    let second_encoded = second.encode();

    Ok(frame_records(
        &[
            LogicalRecord {
                bytes: &first_encoded,
                response_len: AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN,
            },
            LogicalRecord {
                bytes: &second_encoded,
                response_len: AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN,
            },
        ],
        inputs.out_mps,
        inputs.in_mps,
    )?)
}

/// Builds exactly the two APIO-v3 records for ATS-33's ordered `(q-C, q+C)`
/// frequency vector values.
///
/// Every non-vector field is caller supplied. `base` is a checked raw U24 value;
/// this function applies only the static offsets `0x30` then `0x31`, without
/// assigning a channel, APIO mode, adapter route, or device meaning. `object`,
/// both tokens, and the statically evidenced retry limit remain explicit inputs.
pub fn build_single_1khz_sine_frequency_records(
    vector: Single1KhzSineVector,
    base: U24,
    object: u8,
    token9: u8,
    token12: u8,
    retry_limit: u8,
) -> Result<[ApWriteToDspAddr; 2], FrequencyPairPlanError> {
    if retry_limit != AP_WRITE_TO_DSP_ADDR_RETRY_LIMIT {
        return Err(FrequencyPairPlanError::UnsupportedRetryLimit {
            supplied: retry_limit,
        });
    }

    let [q_minus_c, q_plus_c] = vector.frequency_write_data;
    validate_frequency_entry(0, q_minus_c, Q_MINUS_C_OFFSET)?;
    validate_frequency_entry(1, q_plus_c, Q_PLUS_C_OFFSET)?;

    Ok([
        ApWriteToDspAddr::new(
            add_offset(base, Q_MINUS_C_OFFSET)?,
            object,
            token9,
            token12,
            data_as_u24(0, q_minus_c.data)?,
            retry_limit,
        ),
        ApWriteToDspAddr::new(
            add_offset(base, Q_PLUS_C_OFFSET)?,
            object,
            token9,
            token12,
            data_as_u24(1, q_plus_c.data)?,
            retry_limit,
        ),
    ])
}

/// Frames the two ATS-33 frequency-pair records in one deterministic host-only
/// APIO-v3 transfer plan.
///
/// The plan has one logical response byte per record. It performs no transport,
/// USB, APIB, retry, sleep, firmware, or hardware action; framing success says
/// nothing about device acceptance or an analog 1-kHz output.
pub fn plan_single_1khz_sine_frequency_pair(
    vector: Single1KhzSineVector,
    inputs: FrequencyPairPlanInputs,
) -> Result<TransferPlan, FrequencyPairPlanError> {
    let [first, second] = build_single_1khz_sine_frequency_records(
        vector,
        inputs.base,
        inputs.object,
        inputs.token9,
        inputs.token12,
        inputs.retry_limit,
    )?;
    let first_encoded = first.encode();
    let second_encoded = second.encode();

    Ok(frame_records(
        &[
            LogicalRecord {
                bytes: &first_encoded,
                response_len: AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN,
            },
            LogicalRecord {
                bytes: &second_encoded,
                response_len: AP_WRITE_TO_DSP_ADDR_RESPONSE_LEN,
            },
        ],
        inputs.out_mps,
        inputs.in_mps,
    )?)
}

fn validate_level_entry(
    operation_index: usize,
    entry: DspWriteData,
    expected_offset: u32,
) -> Result<(), LevelPairPlanError> {
    if entry.internal_address != expected_offset {
        return Err(LevelPairPlanError::UnexpectedLevelOffset {
            operation_index,
            expected: expected_offset,
            actual: entry.internal_address,
        });
    }
    Ok(())
}

fn add_level_offset(base: U24, offset: u32) -> Result<U24, LevelPairPlanError> {
    let value = base.get();
    let Some(address) = value.checked_add(offset) else {
        return Err(LevelPairPlanError::AddressOverflow {
            base: value,
            offset,
        });
    };
    U24::try_from(address).map_err(|_| LevelPairPlanError::AddressOverflow {
        base: value,
        offset,
    })
}

fn level_data_as_u24(operation_index: usize, data: i32) -> Result<U24, LevelPairPlanError> {
    let byte = u8::try_from(data).map_err(|_| LevelPairPlanError::DataOutsideByte {
        operation_index,
        data,
    })?;
    U24::try_from(u32::from(byte)).map_err(|_| LevelPairPlanError::DataOutsideByte {
        operation_index,
        data,
    })
}

fn validate_frequency_entry(
    operation_index: usize,
    entry: DspWriteData,
    expected_offset: u32,
) -> Result<(), FrequencyPairPlanError> {
    if entry.internal_address != expected_offset {
        return Err(FrequencyPairPlanError::UnexpectedFrequencyAddress {
            operation_index,
            expected: expected_offset,
            actual: entry.internal_address,
        });
    }
    Ok(())
}

fn add_offset(base: U24, offset: u32) -> Result<U24, FrequencyPairPlanError> {
    let value = base.get();
    let Some(address) = value.checked_add(offset) else {
        return Err(FrequencyPairPlanError::AddressOverflow {
            base: value,
            offset,
        });
    };
    U24::try_from(address).map_err(|_| FrequencyPairPlanError::AddressOverflow {
        base: value,
        offset,
    })
}

fn data_as_u24(operation_index: usize, data: i32) -> Result<U24, FrequencyPairPlanError> {
    let value = u32::try_from(data).map_err(|_| FrequencyPairPlanError::DataOutsideU24 {
        operation_index,
        data,
    })?;
    U24::try_from(value).map_err(|_| FrequencyPairPlanError::DataOutsideU24 {
        operation_index,
        data,
    })
}
