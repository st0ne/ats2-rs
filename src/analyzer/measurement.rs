use crate::analyzer::{
    config::{AnalogInputRange, AnalyzerRange, Channel},
    decoder::AnalyzerRawFormat,
};
use thiserror::Error;

/// RMS voltage reference used by documented voltage decibel units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoltageReference {
    /// 1 V RMS.
    Dbv,
    /// 0.7746 V RMS.
    Dbu,
}

impl VoltageReference {
    const fn volts(self) -> f64 {
        match self {
            Self::Dbv => 1.0,
            Self::Dbu => 0.7746,
        }
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum UnitConversionError {
    #[error("input must be finite")]
    NonFiniteInput,
    #[error("RMS voltage cannot be negative")]
    NegativeRmsVoltage,
    #[error("converted RMS voltage is not finite")]
    NonFiniteResult,
}

/// Explicitly named unit attached to a scaled numeric value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalUnit {
    VoltRms,
    Vp,
    Vpp,
    Dbu,
    Dbv,
    Percent,
    DecibelRatio,
    PartsPerMillion,
    Ratio,
    FullScale,
    PercentFullScale,
    Dbfs,
    Bits,
    Watt,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaledValue {
    pub value: f64,
    pub unit: PhysicalUnit,
}

/// A range request associated with a measurement; this is not hardware readback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeasurementRange {
    InputRequest(AnalogInputRange),
    AnalyzerRequest(AnalyzerRange),
}

/// Software-visible classification; callers must not infer overload from a
/// numeric value unless a separate source provides that classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementStatus {
    Numeric,
    NotReady,
    InvalidSentinel,
    Overload,
    Error(MeasurementFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementFailure {
    Timeout,
    Transport,
    External,
}

/// Keeps acquisition provenance and numeric stages separate. `scaled_value`
/// and its unit are absent until a caller has explicit evidence for both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    pub channel: Channel,
    pub raw_word: Option<u32>,
    pub raw_format: Option<AnalyzerRawFormat>,
    pub decoded_value: Option<f64>,
    pub scaled_value: Option<ScaledValue>,
    pub status: MeasurementStatus,
    pub range: Option<MeasurementRange>,
}

/// Convert an explicitly known RMS voltage to the selected voltage reference.
///
/// This performs only the documented mathematical unit conversion. It does
/// not infer or apply analyzer calibration, range, or raw-word scaling.
pub fn vrms_to_decibels(
    vrms: f64,
    reference: VoltageReference,
) -> Result<f64, UnitConversionError> {
    if !vrms.is_finite() {
        return Err(UnitConversionError::NonFiniteInput);
    }
    if vrms < 0.0 {
        return Err(UnitConversionError::NegativeRmsVoltage);
    }
    if vrms == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    Ok(20.0 * (vrms / reference.volts()).log10())
}

/// Convert a voltage level in dBV/dBu to RMS volts.
pub fn decibels_to_vrms(
    decibels: f64,
    reference: VoltageReference,
) -> Result<f64, UnitConversionError> {
    if !decibels.is_finite() {
        return Err(UnitConversionError::NonFiniteInput);
    }
    let vrms = reference.volts() * 10.0_f64.powf(decibels / 20.0);
    if !vrms.is_finite() {
        return Err(UnitConversionError::NonFiniteResult);
    }
    Ok(vrms)
}

#[cfg(test)]
mod tests {
    use super::{
        Measurement, MeasurementRange, MeasurementStatus, PhysicalUnit, ScaledValue,
        VoltageReference, decibels_to_vrms, vrms_to_decibels,
    };
    use crate::analyzer::{
        config::{AnalogInputRange, Channel, InputRangeUnit},
        decoder::AnalyzerRawFormat,
    };

    #[test]
    fn documented_reference_voltages_map_to_zero_decibels() {
        assert_eq!(vrms_to_decibels(1.0, VoltageReference::Dbv), Ok(0.0));
        assert_eq!(vrms_to_decibels(0.7746, VoltageReference::Dbu), Ok(0.0));
    }

    #[test]
    fn voltage_ratio_uses_twenty_log_ten() {
        assert_eq!(vrms_to_decibels(0.01, VoltageReference::Dbv), Ok(-40.0));
    }

    #[test]
    fn decibels_convert_back_to_rms_voltage() {
        assert_eq!(decibels_to_vrms(0.0, VoltageReference::Dbv), Ok(1.0));
        assert_eq!(decibels_to_vrms(0.0, VoltageReference::Dbu), Ok(0.7746));
    }

    #[test]
    fn partial_pipeline_keeps_unscaled_value_and_unknown_physical_unit_explicit() {
        let measurement = Measurement {
            channel: Channel::A,
            raw_word: Some(0x00ff_ffff),
            raw_format: Some(AnalyzerRawFormat::SignedQ1_23),
            decoded_value: Some(-1.0 / 8_388_608.0),
            scaled_value: None,
            status: MeasurementStatus::Numeric,
            range: None,
        };

        assert_eq!(measurement.channel, Channel::A);
        assert_eq!(measurement.raw_word, Some(0x00ff_ffff));
        assert_eq!(measurement.raw_format, Some(AnalyzerRawFormat::SignedQ1_23));
        assert_eq!(measurement.decoded_value, Some(-1.0 / 8_388_608.0));
        assert_eq!(measurement.scaled_value, None);
        assert_eq!(measurement.status, MeasurementStatus::Numeric);
        assert_eq!(measurement.range, None);
    }

    #[test]
    fn scaled_value_requires_explicit_unit_and_range_request_is_not_readback() {
        let measurement = Measurement {
            channel: Channel::B,
            raw_word: None,
            raw_format: None,
            decoded_value: None,
            scaled_value: Some(ScaledValue {
                value: 1.0,
                unit: PhysicalUnit::VoltRms,
            }),
            status: MeasurementStatus::Numeric,
            range: Some(MeasurementRange::InputRequest(AnalogInputRange {
                value: -0.088,
                unit: InputRangeUnit::Dbv,
            })),
        };

        assert_eq!(measurement.channel, Channel::B);
        assert_eq!(
            measurement.scaled_value,
            Some(ScaledValue {
                value: 1.0,
                unit: PhysicalUnit::VoltRms,
            })
        );
        assert_eq!(
            measurement.range,
            Some(MeasurementRange::InputRequest(AnalogInputRange {
                value: -0.088,
                unit: InputRangeUnit::Dbv,
            }))
        );
    }
}
