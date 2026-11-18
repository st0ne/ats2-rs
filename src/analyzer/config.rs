//! Typed representation of requested analyzer API state only.
//!
//! This is a data model, not a hardware state readback or a command sequence.

/// Instrument channel selector used by the documented A/B properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Analyzer input format property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Analog,
    Digital,
}

/// The documented AnalogIn.Source choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalogInputSource {
    XlrBalanced,
    BncUnbalanced,
    GeneratorMonitor,
}

/// Coupling state as exposed by the API; `NotDc` does not assert a particular
/// analog filter implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coupling {
    Dc,
    NotDc,
}

/// Documented impedance selector. It is not a resistance value: LowZ can map
/// to different physical values depending on the instrument option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impedance {
    LowZ,
    HighZ,
}

/// Unit accepted by the AnalogIn.Range API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRangeUnit {
    Vp,
    Dbu,
    Dbv,
}

/// Unit accepted by the Analyzer.Range API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzerRangeUnit {
    Ffs,
    PercentFullScale,
    Dbfs,
    DbRelativeToA,
    DbRelativeToB,
}

/// An API range request. Its value is retained as supplied; no range selection
/// or nominal-full-scale conversion is performed by this type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalogInputRange {
    pub value: f64,
    pub unit: InputRangeUnit,
}

/// An Analyzer.Range API request, kept separate from AnalogIn.Range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyzerRange {
    pub value: f64,
    pub unit: AnalyzerRangeUnit,
}

/// Retains both API signature shapes documented for an autorange property.
///
/// The manual syntax is unscoped for Analyzer.RangeAuto and FuncRangeAuto,
/// while examples pass channel arguments. If both fields are set, this model
/// preserves both requests and does not invent precedence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AmbiguousAutoRangeRequest {
    pub unscoped: Option<bool>,
    pub per_channel: [Option<bool>; 2],
}

/// Requested settings for one channel, with analog-input and analyzer
/// properties kept distinct.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnalyzerChannelConfig {
    pub source: Option<AnalogInputSource>,
    pub analog_input_coupling: Option<Coupling>,
    pub analyzer_coupling: Option<Coupling>,
    pub impedance: Option<Impedance>,
    pub analog_input_range: Option<AnalogInputRange>,
    pub analog_input_range_auto: Option<bool>,
    pub analyzer_range: Option<AnalyzerRange>,
}

/// Requested software/API configuration; `None` means not specified, not zero
/// or a device default. No cross-channel constraints or physical effects are
/// inferred.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnalyzerConfig {
    pub input_format: Option<InputFormat>,
    pub channels: [AnalyzerChannelConfig; 2],
    pub analyzer_range_auto: AmbiguousAutoRangeRequest,
    pub function_range_auto: AmbiguousAutoRangeRequest,
}

impl AnalyzerConfig {
    #[must_use]
    pub fn channel(&self, channel: Channel) -> &AnalyzerChannelConfig {
        &self.channels[channel.index()]
    }

    pub fn channel_mut(&mut self, channel: Channel) -> &mut AnalyzerChannelConfig {
        &mut self.channels[channel.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnalogInputRange, AnalogInputSource, AnalyzerConfig, AnalyzerRange, AnalyzerRangeUnit,
        Channel, Coupling, Impedance, InputFormat, InputRangeUnit,
    };

    #[test]
    fn channel_a_and_b_keep_independent_requested_settings() {
        let mut config = AnalyzerConfig::default();
        config.channel_mut(Channel::A).source = Some(AnalogInputSource::XlrBalanced);
        config.channel_mut(Channel::A).analog_input_coupling = Some(Coupling::NotDc);
        config.channel_mut(Channel::A).impedance = Some(Impedance::LowZ);
        config.channel_mut(Channel::A).analyzer_coupling = Some(Coupling::Dc);
        config.channel_mut(Channel::A).analog_input_range = Some(AnalogInputRange {
            value: -0.088,
            unit: InputRangeUnit::Dbv,
        });
        config.channel_mut(Channel::A).analog_input_range_auto = Some(false);
        config.channel_mut(Channel::A).analyzer_range = Some(AnalyzerRange {
            value: -6.021,
            unit: AnalyzerRangeUnit::Dbfs,
        });
        config.channel_mut(Channel::B).source = Some(AnalogInputSource::BncUnbalanced);

        assert_eq!(
            config.channel(Channel::A).source,
            Some(AnalogInputSource::XlrBalanced)
        );
        assert_eq!(
            config.channel(Channel::A).analog_input_coupling,
            Some(Coupling::NotDc)
        );
        assert_eq!(config.channel(Channel::A).impedance, Some(Impedance::LowZ));
        assert_eq!(
            config.channel(Channel::A).analyzer_coupling,
            Some(Coupling::Dc)
        );
        assert_eq!(
            config.channel(Channel::A).analog_input_range,
            Some(AnalogInputRange {
                value: -0.088,
                unit: InputRangeUnit::Dbv,
            })
        );
        assert_eq!(
            config.channel(Channel::A).analog_input_range_auto,
            Some(false)
        );
        assert_eq!(
            config.channel(Channel::A).analyzer_range,
            Some(AnalyzerRange {
                value: -6.021,
                unit: AnalyzerRangeUnit::Dbfs,
            })
        );
        assert_eq!(
            config.channel(Channel::B).source,
            Some(AnalogInputSource::BncUnbalanced)
        );
        assert_eq!(config.channel(Channel::B).impedance, None);
        assert_eq!(config.channel(Channel::B).analyzer_coupling, None);
    }

    #[test]
    fn analog_input_and_analyzer_ranges_have_distinct_units() {
        let input_range = AnalogInputRange {
            value: -0.088,
            unit: InputRangeUnit::Dbv,
        };
        let analyzer_range = AnalyzerRange {
            value: -6.021,
            unit: AnalyzerRangeUnit::Dbfs,
        };

        assert_eq!(input_range.unit, InputRangeUnit::Dbv);
        assert_eq!(analyzer_range.unit, AnalyzerRangeUnit::Dbfs);
        assert_ne!(input_range.value, analyzer_range.value);
    }

    #[test]
    fn input_format_and_autorange_api_scopes_remain_explicit() {
        let mut config = AnalyzerConfig {
            input_format: Some(InputFormat::Analog),
            ..AnalyzerConfig::default()
        };
        config.analyzer_range_auto.unscoped = Some(true);
        config.analyzer_range_auto.per_channel[1] = Some(false);
        config.function_range_auto.unscoped = Some(false);

        assert_eq!(config.input_format, Some(InputFormat::Analog));
        assert_eq!(config.analyzer_range_auto.unscoped, Some(true));
        assert_eq!(config.analyzer_range_auto.per_channel[1], Some(false));
        assert_eq!(config.function_range_auto.unscoped, Some(false));
    }
}
