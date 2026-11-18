//! Pure ATS-1.5 generator amplitude coding and range-transition planning.
//!
//! This module models software values and write *intents* only. It performs no
//! USB, APIB, DSP, or hardware I/O and makes no claim about physical output,
//! relays, or attenuators. Unknown control values must be supplied explicitly.

use thiserror::Error;

/// The nine statically recovered S2DLL range rows, in selection order.
pub const RANGE_TABLE: [RangeRow; 9] = [
    RangeRow {
        threshold: 2.50015260651935,
        selectors: RangeSelectors([0, 0, 0, 0, 4, 0]),
        scale: 511.9375,
    },
    RangeRow {
        threshold: 1.00015260651935,
        selectors: RangeSelectors([1, 0, 0, 0, 4, 0]),
        scale: 3276.4,
    },
    RangeRow {
        threshold: 0.312519075814919,
        selectors: RangeSelectors([0, 1, 0, 1, 1, 0]),
        scale: 4095.5,
    },
    RangeRow {
        threshold: 0.125019075814919,
        selectors: RangeSelectors([1, 1, 0, 1, 1, 0]),
        scale: 26211.2,
    },
    RangeRow {
        threshold: 0.0390648844768648,
        selectors: RangeSelectors([0, 0, 1, 2, 2, 0]),
        scale: 32764.0,
    },
    RangeRow {
        threshold: 0.0156273844768649,
        selectors: RangeSelectors([1, 0, 1, 2, 2, 0]),
        scale: 209689.6,
    },
    RangeRow {
        threshold: 0.00488311055960811,
        selectors: RangeSelectors([0, 1, 1, 2, 2, 0]),
        scale: 262112.0,
    },
    RangeRow {
        threshold: f64::MIN_POSITIVE,
        selectors: RangeSelectors([1, 1, 1, 2, 2, 0]),
        scale: 1677516.8,
    },
    RangeRow {
        threshold: 0.0,
        selectors: RangeSelectors([1, 1, 1, 2, 2, 0]),
        scale: 1677516.8,
    },
];

const MAX_CODE_13_BIT: u16 = 0x1fff;
const MARKER_BIT: u16 = 0x2000;
const MAX_BALANCED_VRMS: f64 = 16.0;
const MAX_UNBALANCED_OR_CMTST_VRMS: f64 = 8.0;
const VALIDATION_TOLERANCE: f64 = 1.00025;
const RANGE_SWITCH_DELAY_MS: u8 = 5;

/// Bal/Unbal/CMTST configuration used by the reconstructed normalization rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputConfig {
    /// Balanced output; the table input is the supplied Vrms value.
    Balanced,
    /// Unbalanced output; the table input is twice the supplied Vrms value.
    Unbalanced,
    /// CMTST output; the table input is twice the supplied Vrms value.
    Cmtst,
}

/// Generator channel. This is a software channel selector, not a physical
/// connector or DSP-base mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    A,
    B,
}

impl Channel {
    const fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
}

/// Impedance state is retained without assigning the undocumented internal
/// values 1/2 to a physical resistance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImpedanceState {
    /// The runtime impedance code was not resolved.
    Unknown,
    /// Raw internal value, with no physical interpretation attached.
    InternalCode(u32),
}

/// Logical generator state. It says nothing about actual hardware effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeneratorState {
    pub master_enabled: bool,
    pub channel_a_enabled: bool,
    pub channel_b_enabled: bool,
    pub channel_b_tracks_a: bool,
    pub config: OutputConfig,
    pub impedance: ImpedanceState,
    /// Current internal range row for A/B; `None` means unknown, never row 0.
    pub range_rows: [Option<usize>; 2],
}

impl GeneratorState {
    /// Whether the software master and selected channel jointly enable output.
    pub const fn effective_enabled(self, channel: Channel) -> bool {
        self.master_enabled
            && match channel {
                Channel::A => self.channel_a_enabled,
                Channel::B => self.channel_b_enabled,
            }
    }
}

/// Explicit amplitude setter inputs. When B tracking is active, pass A's
/// normalized Vrms in `tracked_a_vrms`; it is not guessed from B's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelRequest {
    pub channel: Channel,
    /// Vrms already normalized by the AP-Basic unit layer.
    pub normalized_vrms: f64,
    /// Required only for channel B while `channel_b_tracks_a` is true.
    pub tracked_a_vrms: Option<f64>,
    /// Existing software bit 15; preserved exactly in the output data word.
    pub f15: bool,
}

/// Raw selector fields copied from one documented table row. Their physical
/// meaning and per-key mapping are intentionally unspecified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeSelectors(pub [u32; 6]);

/// One row of the statically recovered amplitude/range table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeRow {
    /// Inclusive threshold compared with the normalized table input `x`.
    pub threshold: f64,
    /// Six raw selector DWORDs; not labeled as physical attenuator settings.
    pub selectors: RangeSelectors,
    /// Linear quantizer scale for this row.
    pub scale: f64,
}

/// Pure quantized value and resulting software word for one channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncodedLevel {
    pub channel: Channel,
    pub enabled: bool,
    pub input_vrms: f64,
    /// `x = V` for balanced, `x = 2*V` for unbalanced/CMTST.
    pub normalized_table_input: f64,
    pub range_row: usize,
    pub q: u16,
    pub word: u16,
    pub low_byte: u8,
    pub high_byte: u8,
    pub selectors: RangeSelectors,
}

/// Fail-closed level validation and planning errors.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum LevelError {
    #[error("amplitude must be finite")]
    NonFiniteAmplitude,
    #[error("amplitude Vrms must be nonnegative")]
    NegativeAmplitude,
    #[error("amplitude {value} Vrms exceeds the accepted {maximum} Vrms limit")]
    AboveMaximum { value: f64, maximum: f64 },
    #[error("channel B tracking is active but tracked channel-A Vrms was not supplied")]
    MissingTrackedChannelALevel,
    #[error("range row {0} is outside the documented table 0..9")]
    InvalidRangeRow(usize),
    #[error("current range for channel {0:?} is unknown")]
    UnknownCurrentRange(Channel),
    #[error("target state has no explicit range row for channel {0:?}")]
    MissingTargetRange(Channel),
    #[error("target range row {target} does not match encoded row {encoded}")]
    TargetRangeMismatch { target: usize, encoded: usize },
    #[error("encoded enable state does not match the target logical state")]
    EnableStateMismatch,
    #[error("configuration, impedance, or tracking changed; this transition is not reconstructed")]
    UnsupportedStateTransition,
    #[error("range transition requires explicit control values for keys 0, 4, 5, 8, and 9")]
    MissingControlValues,
}

/// Select the first table row whose inclusive threshold is at or below `x`.
pub fn select_range(x: f64) -> Result<usize, LevelError> {
    if !x.is_finite() {
        return Err(LevelError::NonFiniteAmplitude);
    }
    if x < 0.0 {
        return Err(LevelError::NegativeAmplitude);
    }
    RANGE_TABLE
        .iter()
        .position(|row| row.threshold <= x)
        .ok_or(LevelError::NegativeAmplitude)
}

/// Encode a channel amplitude as a software-side 16-bit DSP word.
///
/// The AP-Basic dBu/dBV/W/dBm/dBr conversion is not implemented. Supply the
/// already-normalized Vrms value. The reported 16/8 Vrms limits and 1.00025
/// tolerance are applied before table normalization; hardware effects are not.
pub fn encode_level(
    state: &GeneratorState,
    request: LevelRequest,
) -> Result<EncodedLevel, LevelError> {
    let enabled = state.effective_enabled(request.channel);
    let amplitude = if request.channel == Channel::B && state.channel_b_tracks_a && enabled {
        request
            .tracked_a_vrms
            .ok_or(LevelError::MissingTrackedChannelALevel)?
    } else {
        request.normalized_vrms
    };
    validate_amplitude(amplitude, state.config)?;

    if !enabled {
        let row = RANGE_TABLE.len() - 1;
        return Ok(make_encoded(
            request.channel,
            false,
            amplitude,
            0.0,
            row,
            0,
            request.f15,
        ));
    }

    let normalized_table_input = match state.config {
        OutputConfig::Balanced => amplitude,
        OutputConfig::Unbalanced | OutputConfig::Cmtst => amplitude * 2.0,
    };
    let row_index = select_range(normalized_table_input)?;
    let row = RANGE_TABLE[row_index];
    let rounded = (normalized_table_input * row.scale + 0.5).trunc();
    let q = rounded.min(f64::from(MAX_CODE_13_BIT)) as u16;

    Ok(make_encoded(
        request.channel,
        true,
        amplitude,
        normalized_table_input,
        row_index,
        q,
        request.f15,
    ))
}

fn validate_amplitude(amplitude: f64, config: OutputConfig) -> Result<(), LevelError> {
    if !amplitude.is_finite() {
        return Err(LevelError::NonFiniteAmplitude);
    }
    if amplitude < 0.0 {
        return Err(LevelError::NegativeAmplitude);
    }
    let nominal_max = match config {
        OutputConfig::Balanced => MAX_BALANCED_VRMS,
        OutputConfig::Unbalanced | OutputConfig::Cmtst => MAX_UNBALANCED_OR_CMTST_VRMS,
    };
    let accepted_max = nominal_max * VALIDATION_TOLERANCE;
    if amplitude > accepted_max {
        return Err(LevelError::AboveMaximum {
            value: amplitude,
            maximum: accepted_max,
        });
    }
    Ok(())
}

fn make_encoded(
    channel: Channel,
    enabled: bool,
    input_vrms: f64,
    normalized_table_input: f64,
    range_row: usize,
    q: u16,
    f15: bool,
) -> EncodedLevel {
    let row = RANGE_TABLE[range_row];
    let selector_bit14 = (row.selectors.0[0] as u16) << 14;
    let word = (u16::from(f15) << 15) | selector_bit14 | MARKER_BIT | ((!q) & MAX_CODE_13_BIT);
    EncodedLevel {
        channel,
        enabled,
        input_vrms,
        normalized_table_input,
        range_row,
        q,
        word,
        low_byte: word as u8,
        high_byte: (word >> 8) as u8,
        selectors: row.selectors,
    }
}

/// Values for control-key write intents. These must come from a known runtime
/// state or fixture; this module does not synthesize defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlWriteValues {
    pub key0: u32,
    pub key4: u32,
    pub key5: u32,
    pub key8: u32,
    pub key9: u32,
}

/// Software-only sequence action. A grouped control/selector step deliberately
/// does not promise an order among its individual keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceStep {
    /// Temporary low-13-bit sentinel intent; exact full control word is unknown.
    WriteTemporarySentinel { channel: Channel, low13: u16 },
    /// Explicit control values plus the target row's six raw selectors.
    WriteControlAndSelectors {
        channel: Channel,
        controls: ControlWriteValues,
        selectors: RangeSelectors,
    },
    /// Explicit delay operation, not a sleep or hardware access.
    WaitMilliseconds(u8),
    /// Final software DSP word split low-byte first at the recovered offsets.
    WriteTargetWord {
        channel: Channel,
        word: u16,
        low_byte: u8,
        high_byte: u8,
    },
    /// Output-off sentinel request; not proof of a physical zero output.
    WriteOffSentinel {
        channel: Channel,
        word: u16,
        low_byte: u8,
        high_byte: u8,
    },
}

/// Plan the software-side output-word sequence; this function never executes it.
///
/// A range change on an enabled channel requires explicit control-key values.
/// Unchanged ranges yield one final-word action and no sentinel/delay sequence.
pub fn plan_output_sequence(
    before: &GeneratorState,
    target: &GeneratorState,
    encoded: EncodedLevel,
    controls: Option<ControlWriteValues>,
) -> Result<Vec<SequenceStep>, LevelError> {
    let channel = encoded.channel;
    if !target.effective_enabled(channel) && encoded.enabled {
        return Err(LevelError::EnableStateMismatch);
    }
    if target.effective_enabled(channel) && !encoded.enabled {
        return Err(LevelError::EnableStateMismatch);
    }
    if before.config != target.config
        || before.impedance != target.impedance
        || before.channel_b_tracks_a != target.channel_b_tracks_a
        || before.master_enabled != target.master_enabled
        || before.channel_a_enabled != target.channel_a_enabled
        || before.channel_b_enabled != target.channel_b_enabled
    {
        return Err(LevelError::UnsupportedStateTransition);
    }

    let target_row =
        target.range_rows[channel.index()].ok_or(LevelError::MissingTargetRange(channel))?;
    if target_row >= RANGE_TABLE.len() {
        return Err(LevelError::InvalidRangeRow(target_row));
    }
    if target_row != encoded.range_row {
        return Err(LevelError::TargetRangeMismatch {
            target: target_row,
            encoded: encoded.range_row,
        });
    }
    if !target.effective_enabled(channel) {
        return Ok(vec![SequenceStep::WriteOffSentinel {
            channel,
            word: encoded.word,
            low_byte: encoded.low_byte,
            high_byte: encoded.high_byte,
        }]);
    }

    let old_row =
        before.range_rows[channel.index()].ok_or(LevelError::UnknownCurrentRange(channel))?;
    if old_row >= RANGE_TABLE.len() {
        return Err(LevelError::InvalidRangeRow(old_row));
    }
    if old_row == target_row {
        return Ok(vec![SequenceStep::WriteTargetWord {
            channel,
            word: encoded.word,
            low_byte: encoded.low_byte,
            high_byte: encoded.high_byte,
        }]);
    }

    let controls = controls.ok_or(LevelError::MissingControlValues)?;
    Ok(vec![
        SequenceStep::WriteTemporarySentinel {
            channel,
            low13: MAX_CODE_13_BIT,
        },
        SequenceStep::WriteControlAndSelectors {
            channel,
            controls,
            selectors: encoded.selectors,
        },
        SequenceStep::WaitMilliseconds(RANGE_SWITCH_DELAY_MS),
        SequenceStep::WriteTargetWord {
            channel,
            word: encoded.word,
            low_byte: encoded.low_byte,
            high_byte: encoded.high_byte,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next_lower_positive_f64(value: f64) -> f64 {
        assert!(value.is_finite() && value > 0.0);
        f64::from_bits(value.to_bits() - 1)
    }

    fn state() -> GeneratorState {
        GeneratorState {
            master_enabled: true,
            channel_a_enabled: true,
            channel_b_enabled: false,
            channel_b_tracks_a: false,
            config: OutputConfig::Balanced,
            impedance: ImpedanceState::Unknown,
            range_rows: [Some(2), Some(2)],
        }
    }

    fn request(channel: Channel, normalized_vrms: f64, f15: bool) -> LevelRequest {
        LevelRequest {
            channel,
            normalized_vrms,
            tracked_a_vrms: None,
            f15,
        }
    }

    fn controls() -> ControlWriteValues {
        ControlWriteValues {
            key0: 0x10,
            key4: 0x20,
            key5: 0x30,
            key8: 0x40,
            key9: 0x50,
        }
    }

    #[test]
    fn one_vrms_balanced_vector_is_reproduced() {
        let encoded = encode_level(&state(), request(Channel::A, 1.0, false)).unwrap();
        assert_eq!(encoded.range_row, 2);
        assert_eq!(encoded.q, 4096);
        assert_eq!(encoded.word, 0x2fff);
        assert_eq!((encoded.low_byte, encoded.high_byte), (0xff, 0x2f));
    }

    #[test]
    fn explicit_zero_dbu_vrms_reference_matches_conditional_vector() {
        let encoded = encode_level(&state(), request(Channel::A, 0.7746, false)).unwrap();
        assert_eq!(encoded.range_row, 2);
        assert_eq!(encoded.q, 3172);
        assert_eq!(encoded.word, 0x339b);
        assert_eq!((encoded.low_byte, encoded.high_byte), (0x9b, 0x33));
    }

    #[test]
    fn disabled_channel_uses_conditional_sentinel_word() {
        let mut target = state();
        target.channel_a_enabled = false;
        target.range_rows[0] = Some(8);
        let encoded = encode_level(&target, request(Channel::A, 1.0, false)).unwrap();
        assert!(!encoded.enabled);
        assert_eq!(encoded.range_row, 8);
        assert_eq!(encoded.q, 0);
        assert_eq!(encoded.word, 0x7fff);
    }

    #[test]
    fn bit_15_is_preserved_for_enabled_and_disabled_words() {
        let active = encode_level(&state(), request(Channel::A, 1.0, true)).unwrap();
        assert_eq!(active.word, 0xafff);
        let mut disabled_state = state();
        disabled_state.master_enabled = false;
        let disabled = encode_level(&disabled_state, request(Channel::A, 1.0, true)).unwrap();
        assert_eq!(disabled.word, 0xffff);
    }

    #[test]
    fn range_selector_and_marker_bits_are_packed_separately_from_the_13_bit_code() {
        let encoded = encode_level(&state(), request(Channel::A, 1.5, false)).unwrap();
        assert_eq!(encoded.range_row, 1);
        assert_ne!(encoded.word & 0x4000, 0); // table selector 0 -> bit 14
        assert_ne!(encoded.word & 0x2000, 0); // software marker bit
        assert_eq!(encoded.word & 0x8000, 0); // explicit F15 was false
    }

    #[test]
    fn unbalanced_and_cmtst_double_the_table_input() {
        let mut configured = state();
        configured.config = OutputConfig::Unbalanced;
        let unbalanced = encode_level(&configured, request(Channel::A, 0.5, false)).unwrap();
        assert_eq!(unbalanced.normalized_table_input, 1.0);
        assert_eq!(unbalanced.q, 4096);
        configured.config = OutputConfig::Cmtst;
        let cmtst = encode_level(&configured, request(Channel::A, 0.5, false)).unwrap();
        assert_eq!(cmtst, unbalanced);
    }

    #[test]
    fn channel_b_tracking_requires_and_uses_explicit_channel_a_value() {
        let mut configured = state();
        configured.channel_b_enabled = true;
        configured.channel_b_tracks_a = true;
        assert_eq!(
            encode_level(&configured, request(Channel::B, 0.2, false)),
            Err(LevelError::MissingTrackedChannelALevel)
        );
        let encoded = encode_level(
            &configured,
            LevelRequest {
                channel: Channel::B,
                normalized_vrms: 0.2,
                tracked_a_vrms: Some(1.0),
                f15: false,
            },
        )
        .unwrap();
        assert_eq!(encoded.input_vrms, 1.0);
        assert_eq!(encoded.q, 4096);
    }

    #[test]
    fn all_nine_table_records_and_threshold_boundaries_are_preserved() {
        assert_eq!(RANGE_TABLE.len(), 9);
        for (row_index, row) in RANGE_TABLE.iter().enumerate() {
            assert_eq!(select_range(row.threshold), Ok(row_index));
        }
        assert_eq!(select_range(2.50015260651935), Ok(0));
        assert_eq!(
            select_range(next_lower_positive_f64(2.50015260651935)),
            Ok(1)
        );
        assert_eq!(select_range(f64::MIN_POSITIVE), Ok(7));
        assert_eq!(
            select_range(next_lower_positive_f64(f64::MIN_POSITIVE)),
            Ok(8)
        );
        assert_eq!(select_range(0.0), Ok(8));
        assert_eq!(RANGE_TABLE[2].selectors, RangeSelectors([0, 1, 0, 1, 1, 0]));
    }

    #[test]
    fn rejects_invalid_amplitudes_and_documented_upper_limit_overflow() {
        let state = state();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                encode_level(&state, request(Channel::A, value, false)),
                Err(LevelError::NonFiniteAmplitude)
            );
        }
        assert_eq!(
            encode_level(&state, request(Channel::A, -0.1, false)),
            Err(LevelError::NegativeAmplitude)
        );
        assert!(matches!(
            encode_level(&state, request(Channel::A, 16.01, false)),
            Err(LevelError::AboveMaximum { .. })
        ));
        let max_encoded = encode_level(&state, request(Channel::A, 16.0 * 1.00025, false)).unwrap();
        assert_eq!(max_encoded.q, 8191);
    }

    #[test]
    fn range_change_plan_orders_sentinel_controls_delay_and_final_bytes() {
        let before = state();
        let mut target = state();
        target.range_rows[0] = Some(3);
        let encoded = encode_level(&target, request(Channel::A, 0.2, false)).unwrap();
        let plan = plan_output_sequence(&before, &target, encoded, Some(controls())).unwrap();
        assert_eq!(
            plan,
            vec![
                SequenceStep::WriteTemporarySentinel {
                    channel: Channel::A,
                    low13: 0x1fff,
                },
                SequenceStep::WriteControlAndSelectors {
                    channel: Channel::A,
                    controls: controls(),
                    selectors: RANGE_TABLE[3].selectors,
                },
                SequenceStep::WaitMilliseconds(5),
                SequenceStep::WriteTargetWord {
                    channel: Channel::A,
                    word: encoded.word,
                    low_byte: encoded.low_byte,
                    high_byte: encoded.high_byte,
                },
            ]
        );
    }

    #[test]
    fn unchanged_range_has_no_sentinel_or_delay() {
        let unchanged = state();
        let encoded = encode_level(&unchanged, request(Channel::A, 1.0, false)).unwrap();
        assert_eq!(
            plan_output_sequence(&unchanged, &unchanged, encoded, None).unwrap(),
            vec![SequenceStep::WriteTargetWord {
                channel: Channel::A,
                word: 0x2fff,
                low_byte: 0xff,
                high_byte: 0x2f,
            }]
        );
    }

    #[test]
    fn missing_current_range_and_control_values_fail_closed() {
        let mut before = state();
        before.range_rows[0] = None;
        let mut target = state();
        target.range_rows[0] = Some(3);
        let encoded = encode_level(&target, request(Channel::A, 0.2, false)).unwrap();
        assert_eq!(
            plan_output_sequence(&before, &target, encoded, Some(controls())),
            Err(LevelError::UnknownCurrentRange(Channel::A))
        );

        before.range_rows[0] = Some(2);
        assert_eq!(
            plan_output_sequence(&before, &target, encoded, None),
            Err(LevelError::MissingControlValues)
        );
    }

    #[test]
    fn planner_rejects_unknown_state_transitions_and_mismatched_target_row() {
        let before = state();
        let mut target = state();
        target.impedance = ImpedanceState::InternalCode(2);
        let encoded = encode_level(&target, request(Channel::A, 1.0, false)).unwrap();
        assert_eq!(
            plan_output_sequence(&before, &target, encoded, None),
            Err(LevelError::UnsupportedStateTransition)
        );

        let mut target = state();
        target.range_rows[0] = Some(1);
        let encoded = encode_level(&target, request(Channel::A, 1.0, false)).unwrap();
        assert_eq!(
            plan_output_sequence(&before, &target, encoded, None),
            Err(LevelError::TargetRangeMismatch {
                target: 1,
                encoded: 2,
            })
        );
    }

    #[test]
    fn disabled_sequence_is_a_sentinel_intent_without_delay() {
        let mut state = state();
        state.channel_a_enabled = false;
        state.range_rows[0] = Some(8);
        let encoded = encode_level(&state, request(Channel::A, 1.0, false)).unwrap();
        assert_eq!(
            plan_output_sequence(&state, &state, encoded, None).unwrap(),
            vec![SequenceStep::WriteOffSentinel {
                channel: Channel::A,
                word: 0x7fff,
                low_byte: 0xff,
                high_byte: 0x7f,
            }]
        );
    }
}
