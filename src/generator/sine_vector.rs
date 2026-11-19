//! Conditional, offline-only configuration vectors for one 1-kHz sine request.
//!
//! This module combines existing frequency and level encoders. It only returns
//! calculated values; it does not frame APIB data, access USB, sleep, mutate
//! generator state, or establish an analog output. The normal-sine-to-mode-0
//! association is conditional static evidence, not a runtime observation.

use thiserror::Error;

use super::{
    frequency::{
        DspWriteData, FrequencyError, FrequencyRuntimeState, calculate_dsp_write_data_from_state,
        encode_frequency,
    },
    level::{EncodedLevel, GeneratorState, LevelError, LevelRequest, encode_level},
};

/// Fixed frequency of the offline vector in hertz.
pub const ONE_KHZ_HZ: f64 = 1_000.0;

/// The only waveform-mode assertion supported for this vector.
///
/// Local static evidence identifies mode 0 as a *conditional* normal-sine
/// candidate. This value does not establish that a device currently produces
/// a sine wave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SineModeAssumption {
    /// Caller supplied frequency state whose waveform code is the conditional
    /// normal-sine candidate, mode 0.
    ConditionalNormalSineMode0,
}

/// Purely calculated configuration values for one conditional 1-kHz sine request.
///
/// `level.channel` is preserved from the explicit [`LevelRequest`]. The
/// frequency payloads do not carry a channel association: their internal
/// identifiers are not confirmed hardware addresses and no A/B mapping is
/// inferred here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Single1KhzSineVector {
    /// Always [`ONE_KHZ_HZ`].
    pub frequency_hz: f64,
    /// Explicit limitation of the waveform-mode evidence used by this vector.
    pub sine_mode_assumption: SineModeAssumption,
    /// Existing fixed-point frequency quantizer result.
    pub frequency_code: i32,
    /// Existing checked `q-C` and `q+C` software payload values.
    pub frequency_write_data: [DspWriteData; 2],
    /// Existing level quantization result for the caller-selected channel.
    pub level: EncodedLevel,
}

/// Fail-closed builder for [`Single1KhzSineVector`].
///
/// All runtime state is required explicitly. The builder deliberately has no
/// defaults for rate selection, frequency gates, offset `C`, output mode,
/// enable flags, level, range rows, F15, or channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Single1KhzSineBuilder {
    frequency_state: Option<FrequencyRuntimeState>,
    level_state: Option<GeneratorState>,
    level_request: Option<LevelRequest>,
}

impl Single1KhzSineBuilder {
    /// Start an empty builder with no implied runtime values.
    pub const fn new() -> Self {
        Self {
            frequency_state: None,
            level_state: None,
            level_request: None,
        }
    }

    /// Supply the explicit frequency/rate/gate/offset state.
    pub const fn frequency_state(mut self, state: FrequencyRuntimeState) -> Self {
        self.frequency_state = Some(state);
        self
    }

    /// Supply the explicit output and level-mode state.
    pub const fn level_state(mut self, state: GeneratorState) -> Self {
        self.level_state = Some(state);
        self
    }

    /// Supply the explicit level and selected logical channel.
    pub const fn level_request(mut self, request: LevelRequest) -> Self {
        self.level_request = Some(request);
        self
    }

    /// Build a software-only vector, rejecting absent or unsupported inputs.
    pub fn build(self) -> Result<Single1KhzSineVector, Single1KhzSineError> {
        let frequency_state = self
            .frequency_state
            .ok_or(Single1KhzSineError::MissingFrequencyState)?;
        let level_state = self
            .level_state
            .ok_or(Single1KhzSineError::MissingLevelState)?;
        let level_request = self
            .level_request
            .ok_or(Single1KhzSineError::MissingLevelRequest)?;

        if frequency_state.waveform_mode != 0 {
            return Err(Single1KhzSineError::UnsupportedWaveformMode {
                waveform_mode: frequency_state.waveform_mode,
            });
        }

        let frequency_code = encode_frequency(ONE_KHZ_HZ, &frequency_state)?;
        let frequency_write_data =
            calculate_dsp_write_data_from_state(frequency_code, &frequency_state)?;
        let level = encode_level(&level_state, level_request)?;

        Ok(Single1KhzSineVector {
            frequency_hz: ONE_KHZ_HZ,
            sine_mode_assumption: SineModeAssumption::ConditionalNormalSineMode0,
            frequency_code,
            frequency_write_data,
            level,
        })
    }
}

/// Errors while composing the two already-reconstructed pure encoders.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum Single1KhzSineError {
    /// Required rate/mode/gate/offset state was not supplied.
    #[error("frequency runtime state is required")]
    MissingFrequencyState,
    /// Required explicit output/level mode state was not supplied.
    #[error("level/output state is required")]
    MissingLevelState,
    /// Required explicit level/channel/F15 request was not supplied.
    #[error("level request is required")]
    MissingLevelRequest,
    /// No alternate waveform-mode mapping is inferred for this vector.
    #[error("waveform mode {waveform_mode} is not the conditional normal-sine mode 0")]
    UnsupportedWaveformMode { waveform_mode: u32 },
    /// Existing frequency encoder rejected a prerequisite or numerical input.
    #[error(transparent)]
    Frequency(#[from] FrequencyError),
    /// Existing level encoder rejected a prerequisite or numerical input.
    #[error(transparent)]
    Level(#[from] LevelError),
}
