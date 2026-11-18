//! Pure, offline frequency coding reconstructed from the ATS-1.5 disassembly.
//!
//! This module computes software-side frequency codes and DSP payload values only.
//! It performs no USB/APIB I/O, does not model device behavior, and does not treat
//! the internal address identifiers `0x30` and `0x31` as confirmed hardware registers.

use std::num::NonZeroU32;

const FIXED_POINT_SCALE: f64 = 8_388_608.0;
const QUANTIZATION_BIAS: f64 = 0.49999;
const MAX_FREQUENCY_CODE: i32 = 8_388_607;
const DSP_ADDRESS_Q_MINUS_C: u32 = 0x30;
const DSP_ADDRESS_Q_PLUS_C: u32 = 0x31;

/// The mode-status global used by the special mode-5 conversion path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeStatus {
    /// `[0x1005e26c] == 0`.
    Zero,
    /// `[0x1005e26c] != 0`.
    NonZero,
    /// Runtime state was not observed and must not be guessed.
    Unknown,
}

/// Result of a runtime gate required before the setter reaches quantization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionGate {
    /// The prerequisite is known to have succeeded.
    Ready,
    /// The prerequisite failed, so the setter must not proceed.
    Blocked,
    /// The prerequisite has not been established.
    Unknown,
}

/// Active sample-rate pointer selection and its explicitly resolved value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveRateSource {
    /// A runtime/static selection whose sample rate is known explicitly.
    Resolved {
        selector: u32,
        sample_rate_hz: NonZeroU32,
    },
    /// A pointer/selection exists, but its sample-rate value is unknown.
    Unresolved { selector: Option<u32> },
}

impl ActiveRateSource {
    /// Construct a rate source from an explicitly supplied runtime sample rate.
    pub fn explicit(selector: u32, sample_rate_hz: u32) -> Result<Self, FrequencyError> {
        let sample_rate_hz =
            NonZeroU32::new(sample_rate_hz).ok_or(FrequencyError::InvalidSampleRate)?;
        Ok(Self::Resolved {
            selector,
            sample_rate_hz,
        })
    }

    /// Represent an unresolved runtime selection without inventing an Fs value.
    pub const fn unresolved(selector: Option<u32>) -> Self {
        Self::Unresolved { selector }
    }

    /// Resolve only the sample-rate branches established by the static analysis.
    ///
    /// Selector 6 additionally requires nonzero mode status and a clear
    /// `[0x1005e270]` gate. Other/unsupported selectors are not assigned defaults.
    pub fn resolve_static_selection(
        selector: u32,
        mode_status: ModeStatus,
        selector6_gate_clear: Option<bool>,
    ) -> Result<Self, FrequencyError> {
        let sample_rate_hz = match selector {
            0 => 48_000,
            1 | 2 => 55_000,
            3 => 65_536,
            6 => match (mode_status, selector6_gate_clear) {
                (ModeStatus::NonZero, Some(true)) => 131_072,
                (ModeStatus::NonZero, Some(false)) | (ModeStatus::Zero, _) => {
                    return Err(FrequencyError::RateSourceUnavailable { selector });
                }
                (ModeStatus::Unknown, _) | (ModeStatus::NonZero, None) => {
                    return Err(FrequencyError::RateSourceConditionsUnknown { selector });
                }
            },
            _ => return Err(FrequencyError::UnsupportedRateSource { selector }),
        };

        Self::explicit(selector, sample_rate_hz)
    }

    fn sample_rate_hz(self) -> Result<NonZeroU32, FrequencyError> {
        match self {
            Self::Resolved { sample_rate_hz, .. } => Ok(sample_rate_hz),
            Self::Unresolved { .. } => Err(FrequencyError::UnknownSampleRate),
        }
    }
}

/// Explicit runtime conditions for the pure frequency-code calculation.
///
/// `offset_c` is optional because the binary's offset is history-dependent; the
/// quantizer does not consume it, while [`calculate_dsp_write_data_from_state`]
/// refuses to proceed if it is unknown. APIB/transport status is intentionally
/// outside this arithmetic state model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrequencyRuntimeState {
    /// Active rate-source selector and explicitly resolved sample rate.
    pub rate_source: ActiveRateSource,
    /// Waveform/mode code stored at `[0x1005e6a8]`.
    pub waveform_mode: u32,
    /// Mode-status global `[0x1005e26c]`.
    pub mode_status: ModeStatus,
    /// Outcome of the system/frequency check preceding quantization.
    pub system_check: ExecutionGate,
    /// Whether RateSource initialized `[0x1005e4f0]` before it is dereferenced.
    pub rate_source_initialization: ExecutionGate,
    /// Runtime offset DWORD `[0x1005e6c8]`; `None` means unknown, not zero.
    pub offset_c: Option<i32>,
}

/// Errors from fail-closed frequency coding and checked DSP-data construction.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum FrequencyError {
    /// The selected rate source has no statically established mapping.
    #[error("unsupported statically resolved rate-source selector {selector}")]
    UnsupportedRateSource { selector: u32 },
    /// A known selector cannot enter its required runtime branch.
    #[error("rate-source selector {selector} is unavailable in this state")]
    RateSourceUnavailable { selector: u32 },
    /// Runtime conditions required to resolve a rate-source selector are unknown.
    #[error("runtime conditions for rate-source selector {selector} are unknown")]
    RateSourceConditionsUnknown { selector: u32 },
    /// The active pointer/sample-rate value was not resolved.
    #[error("active sample rate is unknown")]
    UnknownSampleRate,
    /// An explicitly supplied sample rate was zero.
    #[error("sample rate must be nonzero")]
    InvalidSampleRate,
    /// The system/device-dataset check failed before quantization.
    #[error("system/frequency prerequisite check failed")]
    SystemCheckFailed,
    /// The system/device-dataset check result is unknown.
    #[error("system/frequency prerequisite check is unknown")]
    SystemCheckUnknown,
    /// RateSource did not initialize the pointer before use.
    #[error("RateSource was not initialized before frequency calculation")]
    RateSourceNotInitialized,
    /// RateSource initialization is unknown.
    #[error("RateSource initialization state is unknown")]
    RateSourceInitializationUnknown,
    /// NaN and infinite frequency values are rejected.
    #[error("frequency must be finite")]
    NonFiniteFrequency,
    /// The frequency is below the mode's statically reconstructed lower guard.
    #[error("frequency {frequency_hz} Hz is below the mode lower bound {minimum_hz} Hz")]
    BelowMinimum { frequency_hz: f64, minimum_hz: f64 },
    /// The frequency is above the mode's statically reconstructed upper guard.
    #[error("frequency {frequency_hz} Hz is above the mode upper bound {maximum_hz} Hz")]
    AboveMaximum { frequency_hz: f64, maximum_hz: f64 },
    /// Mode 5 requires the status global to choose its conversion routine.
    #[error("mode-5 conversion path depends on unknown mode status")]
    ModeStatusUnknown,
    /// Intermediate fixed-point arithmetic became non-finite or out of range.
    #[error("frequency quantization exceeded the supported numeric range")]
    QuantizationOverflow,
    /// A supplied code is outside the 23-bit nonnegative quantizer range.
    #[error("frequency code {code} is outside 0..={MAX_FREQUENCY_CODE}")]
    InvalidFrequencyCode { code: i32 },
    /// `q±C` exceeded signed 32-bit DSP-data arithmetic.
    #[error("checked DSP-data arithmetic overflowed")]
    DspArithmeticOverflow,
    /// The offset is unknown and cannot be replaced with a default.
    #[error("DSP offset C is unknown")]
    UnknownOffset,
}

/// Purely quantize a positive frequency using explicit runtime state.
///
/// Static modes are based on `fcn.10007144`: modes 4/12/13 use a 2,000-Hz
/// lower reference; mode 5 uses 4,500 Hz and a status-dependent scaler; mode 11
/// uses a `0.235*Fs` validation ceiling; mode 18 uses `min(0.47*Fs, 40,000)`;
/// other mode codes use the standard branch. The quantizer uses the documented
/// `[0, Fs/(2*g)]` clamp, `s=A*g/Fs`, `2^23`, `+0.49999`, and positive truncation.
pub fn encode_frequency(
    frequency_hz: f64,
    state: &FrequencyRuntimeState,
) -> Result<i32, FrequencyError> {
    match state.rate_source_initialization {
        ExecutionGate::Ready => {}
        ExecutionGate::Blocked => return Err(FrequencyError::RateSourceNotInitialized),
        ExecutionGate::Unknown => return Err(FrequencyError::RateSourceInitializationUnknown),
    }
    match state.system_check {
        ExecutionGate::Ready => {}
        ExecutionGate::Blocked => return Err(FrequencyError::SystemCheckFailed),
        ExecutionGate::Unknown => return Err(FrequencyError::SystemCheckUnknown),
    }
    if !frequency_hz.is_finite() {
        return Err(FrequencyError::NonFiniteFrequency);
    }

    let sample_rate = f64::from(state.rate_source.sample_rate_hz()?.get());
    let (lower_reference_hz, validation_ceiling_hz, numerator, gain, high_divisor) =
        match state.waveform_mode {
            4 | 12 | 13 => (2_000.0, 0.47 * sample_rate, Some(2_000.0), 1.0, 2.0),
            5 => (4_500.0, 0.47 * sample_rate, None, 1.0, 2.0),
            11 => (2.0, 0.235 * sample_rate, Some(2.0), 1.0, 2.0),
            18 => (2.0, (0.47 * sample_rate).min(40_000.0), Some(2.0), 2.0, 4.0),
            _ => (2.0, 0.47 * sample_rate, Some(2.0), 1.0, 2.0),
        };

    let minimum_hz = lower_reference_hz * 0.999;
    let maximum_hz = validation_ceiling_hz * 1.00025;
    if frequency_hz < minimum_hz {
        return Err(FrequencyError::BelowMinimum {
            frequency_hz,
            minimum_hz,
        });
    }
    if frequency_hz > maximum_hz {
        return Err(FrequencyError::AboveMaximum {
            frequency_hz,
            maximum_hz,
        });
    }

    // Mode 5 chooses its quantizer only after the shared 4,500-Hz validation.
    let (numerator, gain) = match (numerator, state.waveform_mode, state.mode_status) {
        (Some(numerator), _, _) => (numerator, gain),
        (None, 5, ModeStatus::Zero) => (4_500.0, gain),
        (None, 5, ModeStatus::NonZero) => (2.0, gain),
        (None, 5, ModeStatus::Unknown) => return Err(FrequencyError::ModeStatusUnknown),
        (None, _, _) => return Err(FrequencyError::QuantizationOverflow),
    };

    let quantizer_high_hz = sample_rate / high_divisor;
    let scale = numerator * gain / sample_rate;
    let clamped_frequency_hz = frequency_hz.clamp(0.0, quantizer_high_hz);
    let scaled_code = clamped_frequency_hz * scale * FIXED_POINT_SCALE + QUANTIZATION_BIAS;
    if !scaled_code.is_finite() || scaled_code < 0.0 {
        return Err(FrequencyError::QuantizationOverflow);
    }

    // The x87/Integer conversion path truncates positive values toward zero;
    // the reconstructed path caps the positive code at 2^23-1.
    let truncated_code = scaled_code.trunc();
    if truncated_code >= f64::from(MAX_FREQUENCY_CODE) {
        Ok(MAX_FREQUENCY_CODE)
    } else {
        Ok(truncated_code as i32)
    }
}

/// One software-side DSP payload and its internal address identifier.
///
/// `internal_address` is an internal `DspAddr` low value, not a confirmed
/// ATS-2 hardware register address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DspWriteData {
    /// Internal `DspAddr` low identifier (`0x30` or `0x31`).
    pub internal_address: u32,
    /// Signed 32-bit result of the legacy `q-C` or `q+C` arithmetic.
    pub data: i32,
}

/// Calculate checked software DSP payloads for the internal identifiers 0x30/0x31.
pub fn calculate_dsp_write_data(
    q: i32,
    offset_c: i32,
) -> Result<[DspWriteData; 2], FrequencyError> {
    if !(0..=MAX_FREQUENCY_CODE).contains(&q) {
        return Err(FrequencyError::InvalidFrequencyCode { code: q });
    }
    let q_minus_c = q
        .checked_sub(offset_c)
        .ok_or(FrequencyError::DspArithmeticOverflow)?;
    let q_plus_c = q
        .checked_add(offset_c)
        .ok_or(FrequencyError::DspArithmeticOverflow)?;
    Ok([
        DspWriteData {
            internal_address: DSP_ADDRESS_Q_MINUS_C,
            data: q_minus_c,
        },
        DspWriteData {
            internal_address: DSP_ADDRESS_Q_PLUS_C,
            data: q_plus_c,
        },
    ])
}

/// Build DSP payload values only when the state's history-dependent `C` is known.
pub fn calculate_dsp_write_data_from_state(
    q: i32,
    state: &FrequencyRuntimeState,
) -> Result<[DspWriteData; 2], FrequencyError> {
    let offset_c = state.offset_c.ok_or(FrequencyError::UnknownOffset)?;
    calculate_dsp_write_data(q, offset_c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal_sine_state() -> FrequencyRuntimeState {
        FrequencyRuntimeState {
            rate_source: ActiveRateSource::resolve_static_selection(1, ModeStatus::Zero, None)
                .expect("static RateSource 1 maps to 55 kHz"),
            waveform_mode: 0,
            mode_status: ModeStatus::Zero,
            system_check: ExecutionGate::Ready,
            rate_source_initialization: ExecutionGate::Ready,
            offset_c: Some(305_040),
        }
    }

    #[test]
    fn normal_sine_1khz_matches_the_conditional_55khz_vector() {
        let state = normal_sine_state();
        let q = encode_frequency(1_000.0, &state).expect("valid normal-sine frequency");
        assert_eq!(q, 305_040);

        let writes = calculate_dsp_write_data_from_state(q, &state)
            .expect("known C and checked DSP arithmetic");
        assert_eq!(writes[0].internal_address, 0x30);
        assert_eq!(writes[0].data, 0);
        assert_eq!(writes[1].internal_address, 0x31);
        assert_eq!(writes[1].data, 610_080);
    }

    #[test]
    fn unresolved_rate_source_is_not_replaced_with_a_default() {
        let mut state = normal_sine_state();
        state.rate_source = ActiveRateSource::unresolved(Some(1));

        assert_eq!(
            encode_frequency(1_000.0, &state),
            Err(FrequencyError::UnknownSampleRate)
        );
    }

    #[test]
    fn mode_5_one_khz_is_rejected_by_its_lower_guard_before_status_branching() {
        let mut state = normal_sine_state();
        state.waveform_mode = 5;
        state.mode_status = ModeStatus::Unknown;

        assert!(matches!(
            encode_frequency(1_000.0, &state),
            Err(FrequencyError::BelowMinimum { minimum_hz, .. }) if minimum_hz == 4_495.5
        ));
    }

    #[test]
    fn standard_mode_11_and_18_use_the_documented_scalers() {
        let mut state = normal_sine_state();
        state.rate_source = ActiveRateSource::resolve_static_selection(0, ModeStatus::Zero, None)
            .expect("RateSource 0 is 48 kHz");

        assert_eq!(encode_frequency(1_000.0, &state), Ok(349_525));
        state.waveform_mode = 11;
        assert_eq!(encode_frequency(1_000.0, &state), Ok(349_525));
        state.waveform_mode = 18;
        assert_eq!(encode_frequency(1_000.0, &state), Ok(699_051));
    }

    #[test]
    fn high_minimum_modes_reject_one_khz() {
        let mut state = normal_sine_state();
        for mode in [4, 12, 13, 5] {
            state.waveform_mode = mode;
            state.mode_status = ModeStatus::Zero;
            assert!(matches!(
                encode_frequency(1_000.0, &state),
                Err(FrequencyError::BelowMinimum { .. })
            ));
        }
    }

    #[test]
    fn mode_18_quantizer_saturates_and_validation_ceiling_is_enforced() {
        let mut state = normal_sine_state();
        state.rate_source = ActiveRateSource::resolve_static_selection(0, ModeStatus::Zero, None)
            .expect("RateSource 0 is 48 kHz");
        state.waveform_mode = 18;

        assert_eq!(encode_frequency(22_560.0, &state), Ok(8_388_607));
        assert!(matches!(
            encode_frequency(22_566.0, &state),
            Err(FrequencyError::AboveMaximum { .. })
        ));
    }

    #[test]
    fn mode_5_status_selects_its_distinct_quantizer_after_validation() {
        let mut state = normal_sine_state();
        state.waveform_mode = 5;

        state.mode_status = ModeStatus::Zero;
        assert_eq!(encode_frequency(5_000.0, &state), Ok(8_388_607));

        state.mode_status = ModeStatus::NonZero;
        assert_eq!(encode_frequency(5_000.0, &state), Ok(1_525_201));

        state.mode_status = ModeStatus::Unknown;
        assert_eq!(
            encode_frequency(5_000.0, &state),
            Err(FrequencyError::ModeStatusUnknown)
        );
    }

    #[test]
    fn static_rate_source_mapping_keeps_selector_6_conditions_explicit() {
        for (selector, expected) in [(0, 48_000), (1, 55_000), (2, 55_000), (3, 65_536)] {
            let source =
                ActiveRateSource::resolve_static_selection(selector, ModeStatus::Zero, None)
                    .expect("documented static selection");
            assert_eq!(source.sample_rate_hz().map(NonZeroU32::get), Ok(expected));
        }

        let source = ActiveRateSource::resolve_static_selection(6, ModeStatus::NonZero, Some(true))
            .expect("selector 6 gate is open");
        assert_eq!(source.sample_rate_hz().map(NonZeroU32::get), Ok(131_072));
        assert_eq!(
            ActiveRateSource::resolve_static_selection(6, ModeStatus::Unknown, None),
            Err(FrequencyError::RateSourceConditionsUnknown { selector: 6 })
        );
        assert_eq!(
            ActiveRateSource::resolve_static_selection(6, ModeStatus::NonZero, Some(false)),
            Err(FrequencyError::RateSourceUnavailable { selector: 6 })
        );
        assert_eq!(
            ActiveRateSource::resolve_static_selection(9, ModeStatus::Zero, None),
            Err(FrequencyError::UnsupportedRateSource { selector: 9 })
        );
        assert_eq!(
            ActiveRateSource::explicit(1, 0),
            Err(FrequencyError::InvalidSampleRate)
        );
    }

    #[test]
    fn calculation_fails_closed_when_initialization_or_system_gate_is_unknown() {
        let mut state = normal_sine_state();
        state.rate_source_initialization = ExecutionGate::Unknown;
        assert_eq!(
            encode_frequency(1_000.0, &state),
            Err(FrequencyError::RateSourceInitializationUnknown)
        );

        state.rate_source_initialization = ExecutionGate::Ready;
        state.system_check = ExecutionGate::Unknown;
        assert_eq!(
            encode_frequency(1_000.0, &state),
            Err(FrequencyError::SystemCheckUnknown)
        );

        state.system_check = ExecutionGate::Blocked;
        assert_eq!(
            encode_frequency(1_000.0, &state),
            Err(FrequencyError::SystemCheckFailed)
        );
    }

    #[test]
    fn invalid_frequency_and_dsp_arithmetic_are_reported() {
        let state = normal_sine_state();
        for frequency in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                encode_frequency(frequency, &state),
                Err(FrequencyError::NonFiniteFrequency)
            );
        }
        assert!(matches!(
            encode_frequency(-1.0, &state),
            Err(FrequencyError::BelowMinimum { .. })
        ));
        assert_eq!(
            calculate_dsp_write_data(8_388_607, i32::MAX),
            Err(FrequencyError::DspArithmeticOverflow)
        );
        assert_eq!(
            calculate_dsp_write_data(-1, 0),
            Err(FrequencyError::InvalidFrequencyCode { code: -1 })
        );
        assert_eq!(
            calculate_dsp_write_data(8_388_608, 0),
            Err(FrequencyError::InvalidFrequencyCode { code: 8_388_608 })
        );
    }

    #[test]
    fn unknown_offset_is_not_treated_as_zero() {
        let mut state = normal_sine_state();
        state.offset_c = None;
        assert_eq!(encode_frequency(1_000.0, &state), Ok(305_040));
        assert_eq!(
            calculate_dsp_write_data_from_state(305_040, &state),
            Err(FrequencyError::UnknownOffset)
        );
    }

    #[test]
    fn mode_11_and_mode_18_apply_their_distinct_upper_guards() {
        let mut state = normal_sine_state();
        state.waveform_mode = 11;
        assert!(matches!(
            encode_frequency(12_929.0, &state),
            Err(FrequencyError::AboveMaximum { .. })
        ));

        state.waveform_mode = 18;
        state.rate_source =
            ActiveRateSource::resolve_static_selection(6, ModeStatus::NonZero, Some(true))
                .expect("source 6 is conditionally available");
        assert_eq!(encode_frequency(40_000.0, &state), Ok(8_388_607));
        assert!(matches!(
            encode_frequency(40_011.0, &state),
            Err(FrequencyError::AboveMaximum { .. })
        ));
    }

    #[test]
    fn mode_4_lower_guard_includes_the_documented_tolerance() {
        let mut state = normal_sine_state();
        state.waveform_mode = 4;
        state.rate_source = ActiveRateSource::resolve_static_selection(0, ModeStatus::Zero, None)
            .expect("RateSource 0 is 48 kHz");

        assert_eq!(encode_frequency(1_998.0, &state), Ok(8_388_607));
        assert!(matches!(
            encode_frequency(1_997.0, &state),
            Err(FrequencyError::BelowMinimum { .. })
        ));
    }
}
