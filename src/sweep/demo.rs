//! Deterministic mock measurement demonstrations using only `MockAnalyzer`.
//!
//! All readings below are synthetic fixture values. They are not measurements
//! from an ATS-2 and carry no ATS calibration or hardware-behavior claim.

use std::{
    collections::BTreeMap,
    fmt::{Display, Formatter},
    time::Duration,
};

use thiserror::Error;

use crate::{
    analyzer::{
        config::Channel,
        measurement::{Measurement, MeasurementStatus, PhysicalUnit, ScaledValue},
        mock::{MockAnalyzer, MockError, MockScenario},
    },
    sweep::{
        FailurePolicy, IdentifiedMeasurementRequest, MeasurementRequest, MultiSweepOutcome,
        MultiSweepPlan, MultiSweepPoint, SettlingPolicy, SweepAxis, SweepAxisSpec, SweepBackend,
        SweepOrder, SweepPlanError, SweepProgress, export::SweepExportError,
        export::export_multi_sweep_csv, export::export_multi_sweep_json,
        run_multi_sweep_controlled,
    },
};

pub const SIMULATION_NOTICE: &str =
    "SIMULATION ONLY — deterministic MockAnalyzer values; not an ATS-2 measurement.";
pub const POWER_ASSUMPTIONS: &str = "P = Vrms^2 / R; assumes the simulated RMS voltage is across the explicitly supplied, purely resistive load and estimates average real power for that model only. Reactive load behavior and ATS calibration are not modeled.";
const POWER_MEASUREMENT_ID: &str = "power.w";
const POWER_MEASUREMENT_FUNCTION: &str = "simulated.output_power";

const SIMULATED_GENERATOR_LEVELS_VRMS: [f64; 3] = [0.1, 0.5, 1.0];
// Columns are RMS voltage, THD+N percentage, and THD percentage. These are
// fixed teaching fixtures, not predicted or observed ATS-2 behavior.
const SYNTHETIC_READINGS: [[f64; 3]; 3] = [[0.095, 1.2, 0.2], [0.49, 0.6, 0.1], [0.98, 0.9, 0.15]];
const MAX_VIRTUAL_POLL_STEPS: usize = 128;
const VIRTUAL_POLL_STEP: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Error, PartialEq)]
pub enum DemoError {
    #[error(transparent)]
    Mock(#[from] MockError),
    #[error(transparent)]
    Plan(#[from] SweepPlanError),
    #[error("simulated generator level must be finite and nonnegative")]
    InvalidGeneratorLevel,
    #[error("simulated frequency must be finite and greater than zero")]
    InvalidFrequency,
    #[error("measured-power display axis requires an explicit load resistance")]
    PowerAxisRequiresLoadResistance,
    #[error("unknown simulated measurement function or unit: {0}")]
    UnknownMeasurement(String),
    #[error("simulated measurement returned non-numeric status: {0:?}")]
    NonNumericStatus(MeasurementStatus),
    #[error("simulated measurement {0} has no explicitly-unitized value")]
    MissingScaledValue(String),
    #[error("simulated measurement {0} has a non-finite value")]
    NonFiniteReading(String),
    #[error("simulated RMS voltage must be finite and nonnegative")]
    InvalidRms,
    #[error("simulated output power requires a preceding RMS measurement")]
    MissingRmsForPower,
    #[error("simulated measurement {function} returned unit {actual:?}, expected {expected:?}")]
    UnitMismatch {
        function: String,
        actual: PhysicalUnit,
        expected: PhysicalUnit,
    },
    #[error("load resistance must be finite and greater than zero")]
    InvalidLoadResistance,
    #[error("simulated power calculation is not finite")]
    NonFinitePower,
    #[error("simulated sweep aborted at point {0}")]
    SweepAborted(usize),
    #[error("simulated sweep is missing measurement {0}")]
    MissingMeasurement(String),
    #[error("simulated sweep measurement {0} failed")]
    MeasurementFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SimulatedStimulus {
    requested_vrms: f64,
    frequency_hz: f64,
}

impl Display for SimulatedStimulus {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "simulated {:.6} V RMS at {:.6} Hz",
            self.requested_vrms, self.frequency_hz
        )
    }
}

fn calculate_power_w(measured_vrms: f64, resistance_ohms: f64) -> Result<f64, DemoError> {
    if !measured_vrms.is_finite() || measured_vrms < 0.0 {
        return Err(DemoError::InvalidRms);
    }
    if !resistance_ohms.is_finite() || resistance_ohms <= 0.0 {
        return Err(DemoError::InvalidLoadResistance);
    }
    let power = measured_vrms * measured_vrms / resistance_ohms;
    if !power.is_finite() {
        return Err(DemoError::NonFinitePower);
    }
    Ok(power)
}

fn interpolate_fixture_row(point_index: usize, point_count: usize) -> [f64; 3] {
    if point_count <= 1 {
        return SYNTHETIC_READINGS[0];
    }
    let position =
        point_index as f64 * (SYNTHETIC_READINGS.len() - 1) as f64 / (point_count - 1) as f64;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(SYNTHETIC_READINGS.len() - 1);
    let fraction = position - lower as f64;
    std::array::from_fn(|column| {
        SYNTHETIC_READINGS[lower][column] * (1.0 - fraction)
            + SYNTHETIC_READINGS[upper][column] * fraction
    })
}

struct SimulatedBackend {
    analyzer: MockAnalyzer,
    applied_levels_vrms: Vec<f64>,
    applied_frequencies_hz: Vec<f64>,
    load_resistance_ohms: Option<f64>,
    last_measured_vrms: Option<f64>,
}

impl SimulatedBackend {
    fn new(point_count: usize, include_thd: bool, load_resistance_ohms: Option<f64>) -> Self {
        let measurement_count = if include_thd { 3 } else { 2 };
        let scenarios = (0..point_count).flat_map(|point_index| {
            let readings = interpolate_fixture_row(point_index, point_count);
            readings
                .into_iter()
                .take(measurement_count)
                .enumerate()
                .map(move |(measurement_index, value)| {
                    let unit = if measurement_index == 0 {
                        PhysicalUnit::VoltRms
                    } else {
                        PhysicalUnit::Percent
                    };
                    MockScenario {
                        channel: Channel::A,
                        pending_polls: (point_index * measurement_count + measurement_index) % 3,
                        delay: Duration::ZERO,
                        completion: Ok(Measurement {
                            channel: Channel::A,
                            raw_word: None,
                            raw_format: None,
                            decoded_value: None,
                            scaled_value: Some(ScaledValue { value, unit }),
                            status: MeasurementStatus::Numeric,
                            range: None,
                        }),
                    }
                })
        });
        Self {
            analyzer: MockAnalyzer::new(scenarios),
            applied_levels_vrms: Vec::new(),
            applied_frequencies_hz: Vec::new(),
            load_resistance_ohms,
            last_measured_vrms: None,
        }
    }
}

impl SweepBackend<SimulatedStimulus> for SimulatedBackend {
    type Reading = f64;
    type Error = DemoError;

    fn apply_stimulus(&mut self, stimulus: &SimulatedStimulus) -> Result<(), Self::Error> {
        if !stimulus.requested_vrms.is_finite() || stimulus.requested_vrms < 0.0 {
            return Err(DemoError::InvalidGeneratorLevel);
        }
        if !stimulus.frequency_hz.is_finite() || stimulus.frequency_hz <= 0.0 {
            return Err(DemoError::InvalidFrequency);
        }
        // Record a requested synthetic level; this does not drive a generator.
        self.applied_levels_vrms.push(stimulus.requested_vrms);
        self.applied_frequencies_hz.push(stimulus.frequency_hz);
        self.last_measured_vrms = None;
        Ok(())
    }

    fn settle(&mut self, _policy: SettlingPolicy) -> Result<(), Self::Error> {
        // No wall-clock sleep: MockAnalyzer advances only through virtual polls.
        Ok(())
    }

    fn measure(&mut self, request: &MeasurementRequest) -> Result<Self::Reading, Self::Error> {
        if request.function == POWER_MEASUREMENT_FUNCTION {
            if request.unit.as_deref() != Some("W") {
                return Err(DemoError::UnknownMeasurement(request.function.clone()));
            }
            let resistance = self
                .load_resistance_ohms
                .ok_or(DemoError::InvalidLoadResistance)?;
            let measured_vrms = self
                .last_measured_vrms
                .ok_or(DemoError::MissingRmsForPower)?;
            return calculate_power_w(measured_vrms, resistance);
        }

        let (expected_unit, expected_label) = match request.function.as_str() {
            "simulated.rms" => (PhysicalUnit::VoltRms, "V RMS"),
            "simulated.thd+n" | "simulated.thd" => (PhysicalUnit::Percent, "%"),
            other => return Err(DemoError::UnknownMeasurement(other.to_owned())),
        };
        if request.unit.as_deref() != Some(expected_label) {
            return Err(DemoError::UnknownMeasurement(request.function.clone()));
        }

        self.analyzer.trigger(Channel::A)?;
        let mut ready = false;
        for _ in 0..MAX_VIRTUAL_POLL_STEPS {
            if self.analyzer.poll_ready()? {
                ready = true;
                break;
            }
            self.analyzer.advance_time(VIRTUAL_POLL_STEP)?;
        }
        if !ready {
            return Err(MockError::NotReady.into());
        }

        let reading = self.analyzer.read()?;
        if reading.status != MeasurementStatus::Numeric {
            return Err(DemoError::NonNumericStatus(reading.status));
        }
        let Some(scaled) = reading.scaled_value else {
            return Err(DemoError::MissingScaledValue(request.function.clone()));
        };
        if scaled.unit != expected_unit {
            return Err(DemoError::UnitMismatch {
                function: request.function.clone(),
                actual: scaled.unit,
                expected: expected_unit,
            });
        }
        if !scaled.value.is_finite() {
            return Err(DemoError::NonFiniteReading(request.function.clone()));
        }
        if request.function == "simulated.rms" {
            if scaled.value < 0.0 {
                return Err(DemoError::InvalidRms);
            }
            self.last_measured_vrms = Some(scaled.value);
        }
        Ok(scaled.value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DemonstrationPoint {
    pub record_index: usize,
    pub axis_value: f64,
    pub generator_level_vrms: f64,
    pub frequency_hz: f64,
    pub measured_vrms: f64,
    pub thd_n_percent: f64,
    pub thd_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DemonstrationResult {
    pub provenance: &'static str,
    pub sweep_variable: DemoSweepVariable,
    pub display_axis: DemoXAxis,
    pub load_resistance_ohms: Option<f64>,
    pub power_assumptions: Option<&'static str>,
    pub simulated_stimuli_vrms: Vec<f64>,
    pub simulated_frequencies_hz: Vec<f64>,
    pub points: Vec<DemonstrationPoint>,
    pub sweep: MultiSweepOutcome<f64, DemoError>,
}

impl DemonstrationResult {
    /// Return derived power from the canonical sweep record, if it was planned
    /// and completed with a finite, nonnegative value.
    pub fn measured_power_w(&self, record_index: usize) -> Option<f64> {
        let value = self
            .sweep
            .dataset
            .records
            .get(record_index)?
            .measurements
            .iter()
            .find(|measurement| measurement.id == POWER_MEASUREMENT_ID)?
            .result
            .as_ref()
            .ok()
            .copied()?;
        (value.is_finite() && value >= 0.0).then_some(value)
    }
}

/// Synthetic variable adjusted during a demonstration sweep.
///
/// This enum does not assert that an ATS-2 exposes either control through the
/// currently verified protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoSweepVariable {
    GeneratorLevelVrms,
    FrequencyHz,
}

/// Selects a result/display axis. Measured power is derived from the simulated
/// RMS result and an explicitly supplied resistive load; it is not a commanded
/// power sweep variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoXAxis {
    GeneratorLevelVrms,
    FrequencyHz,
    MeasuredVrms,
    MeasuredPowerW,
}

/// User-selected parameters for an explicitly synthetic demonstration sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct DemoConfig {
    pub sweep_variable: DemoSweepVariable,
    pub display_axis: DemoXAxis,
    pub axis: SweepAxisSpec,
    pub include_thd: bool,
    pub load_resistance_ohms: Option<f64>,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            sweep_variable: DemoSweepVariable::GeneratorLevelVrms,
            display_axis: DemoXAxis::GeneratorLevelVrms,
            axis: SweepAxisSpec::Explicit {
                values: SIMULATED_GENERATOR_LEVELS_VRMS.to_vec(),
            },
            include_thd: true,
            load_resistance_ohms: None,
        }
    }
}

/// A completed or safely cancelled demo together with its original export plan.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlledDemonstrationResult {
    pub demonstration: DemonstrationResult,
    pub cancelled_at: Option<usize>,
    plan: MultiSweepPlan<SimulatedStimulus>,
}

impl ControlledDemonstrationResult {
    fn export_metadata(&self, metadata: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut combined = metadata.clone();
        combined.remove("power_load_resistance_ohms");
        combined.remove("power_calculation_basis");
        combined.remove("power_source_measurement_id");
        if let Some(resistance) = self.demonstration.load_resistance_ohms {
            combined.insert(
                "power_load_resistance_ohms".to_owned(),
                resistance.to_string(),
            );
            combined.insert(
                "power_calculation_basis".to_owned(),
                POWER_ASSUMPTIONS.to_owned(),
            );
            combined.insert(
                "power_source_measurement_id".to_owned(),
                "voltage.rms".to_owned(),
            );
        }
        combined
    }

    pub fn export_csv(
        &self,
        metadata: &BTreeMap<String, String>,
    ) -> Result<String, SweepExportError> {
        let metadata = self.export_metadata(metadata);
        export_multi_sweep_csv(&self.plan, &self.demonstration.sweep, &metadata)
    }

    pub fn export_json(
        &self,
        metadata: &BTreeMap<String, String>,
    ) -> Result<String, SweepExportError> {
        let metadata = self.export_metadata(metadata);
        export_multi_sweep_json(&self.plan, &self.demonstration.sweep, &metadata)
    }
}

/// Run a deterministic, simulation-only generator-level sweep.
///
/// Pass `Some(resistance_ohms)` to request the explicitly modeled resistive-load
/// power measurement in the sweep dataset. `None` leaves power unplanned. `include_thd` selects
/// whether a separate THD result is included alongside RMS and THD+N.
pub fn simulate_demo(
    load_resistance_ohms: Option<f64>,
    include_thd: bool,
) -> Result<DemonstrationResult, DemoError> {
    let config = DemoConfig {
        include_thd,
        load_resistance_ohms,
        ..DemoConfig::default()
    };
    simulate_demo_controlled(&config, || false, |_| {}).map(|run| run.demonstration)
}

/// Run a configurable synthetic sweep with point-level progress and cancellation.
///
/// Cancellation is observed between sweep points. It does not interrupt a mock operation already
/// in progress. Fixture values are interpolated deterministically when the requested point count
/// differs from the three reference fixture rows; they remain synthetic, not ATS-2 measurements.
/// Measurement failures remain in the returned outcome so their status and missing values can be
/// exported rather than discarded.
pub fn simulate_demo_controlled(
    config: &DemoConfig,
    is_cancelled: impl FnMut() -> bool,
    on_progress: impl FnMut(SweepProgress),
) -> Result<ControlledDemonstrationResult, DemoError> {
    if config
        .load_resistance_ohms
        .is_some_and(|resistance| !resistance.is_finite() || resistance <= 0.0)
    {
        return Err(DemoError::InvalidLoadResistance);
    }
    if config.display_axis == DemoXAxis::MeasuredPowerW && config.load_resistance_ohms.is_none() {
        return Err(DemoError::PowerAxisRequiresLoadResistance);
    }

    let values = config.axis.generate_values(SweepOrder::AsSpecified)?;
    let (axis_name, axis_unit) = match config.sweep_variable {
        DemoSweepVariable::GeneratorLevelVrms => {
            if values.iter().any(|value| *value < 0.0) {
                return Err(DemoError::InvalidGeneratorLevel);
            }
            ("simulated generator level", "V RMS")
        }
        DemoSweepVariable::FrequencyHz => {
            if values.iter().any(|value| *value <= 0.0) {
                return Err(DemoError::InvalidFrequency);
            }
            ("simulated frequency", "Hz")
        }
    };
    let stimuli: Vec<_> = values
        .iter()
        .map(|value| match config.sweep_variable {
            DemoSweepVariable::GeneratorLevelVrms => SimulatedStimulus {
                requested_vrms: *value,
                frequency_hz: 1_000.0,
            },
            DemoSweepVariable::FrequencyHz => SimulatedStimulus {
                requested_vrms: 0.5,
                frequency_hz: *value,
            },
        })
        .collect();

    let mut measurements = vec![identified("voltage.rms", "simulated.rms", "V RMS")];
    measurements.push(identified("distortion.thd+n", "simulated.thd+n", "%"));
    if config.include_thd {
        measurements.push(identified("distortion.thd", "simulated.thd", "%"));
    }
    if config.load_resistance_ohms.is_some() {
        measurements.push(identified(
            POWER_MEASUREMENT_ID,
            POWER_MEASUREMENT_FUNCTION,
            "W",
        ));
    }
    let points = values
        .iter()
        .copied()
        .zip(stimuli.iter().copied())
        .map(|(axis_value, stimulus)| MultiSweepPoint {
            axis_value,
            stimulus,
            settling: SettlingPolicy::None,
        })
        .collect();
    let plan = MultiSweepPlan::new(
        SweepAxis {
            name: axis_name.into(),
            unit: axis_unit.into(),
        },
        points,
        measurements,
        FailurePolicy::Abort,
    )?;
    let mut backend = SimulatedBackend::new(
        values.len(),
        config.include_thd,
        config.load_resistance_ohms,
    );
    let controlled = run_multi_sweep_controlled(&plan, &mut backend, is_cancelled, on_progress);
    let sweep = controlled.outcome;

    let demonstration_points = sweep
        .dataset
        .records
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            let measured_vrms = extract_numeric(record, "voltage.rms")?;
            let thd_n_percent = extract_numeric(record, "distortion.thd+n")?;
            let thd_percent = config
                .include_thd
                .then(|| extract_numeric(record, "distortion.thd"))
                .flatten();
            if config.include_thd && thd_percent.is_none() {
                return None;
            }
            let generator_level_vrms = *backend.applied_levels_vrms.get(index)?;
            let frequency_hz = *backend.applied_frequencies_hz.get(index)?;
            Some(DemonstrationPoint {
                record_index: index,
                axis_value: record.axis_value,
                generator_level_vrms,
                frequency_hz,
                measured_vrms,
                thd_n_percent,
                thd_percent,
            })
        })
        .collect();

    Ok(ControlledDemonstrationResult {
        demonstration: DemonstrationResult {
            provenance: SIMULATION_NOTICE,
            sweep_variable: config.sweep_variable,
            display_axis: config.display_axis,
            load_resistance_ohms: config.load_resistance_ohms,
            power_assumptions: config.load_resistance_ohms.map(|_| POWER_ASSUMPTIONS),
            simulated_stimuli_vrms: backend.applied_levels_vrms,
            simulated_frequencies_hz: backend.applied_frequencies_hz,
            points: demonstration_points,
            sweep,
        },
        cancelled_at: controlled.cancelled_at,
        plan,
    })
}

fn identified(id: &str, function: &str, unit: &str) -> IdentifiedMeasurementRequest {
    IdentifiedMeasurementRequest {
        id: id.to_owned(),
        request: MeasurementRequest {
            function: function.to_owned(),
            unit: Some(unit.to_owned()),
        },
    }
}

fn extract_numeric(
    record: &crate::sweep::MultiSweepRecord<f64, DemoError>,
    id: &str,
) -> Option<f64> {
    record
        .measurements
        .iter()
        .find(|measurement| measurement.id == id)?
        .result
        .as_ref()
        .ok()
        .copied()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::{DemoError, SimulatedBackend, SimulatedStimulus, calculate_power_w};
    use crate::{
        analyzer::{
            config::Channel,
            measurement::{Measurement, MeasurementStatus, PhysicalUnit, ScaledValue},
            mock::{MockAnalyzer, MockScenario},
        },
        sweep::{MeasurementRequest, SweepBackend},
    };
    use std::time::Duration;

    #[test]
    fn derived_power_rejects_invalid_rms_and_load_values_but_accepts_zero_rms() {
        assert_eq!(calculate_power_w(0.0, 8.0), Ok(0.0));
        for rms in [-0.1, f64::NAN, f64::INFINITY] {
            assert_eq!(calculate_power_w(rms, 8.0), Err(DemoError::InvalidRms));
        }
        for resistance in [0.0, -8.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                calculate_power_w(1.0, resistance),
                Err(DemoError::InvalidLoadResistance)
            );
        }
        assert_eq!(
            calculate_power_w(f64::MAX, f64::MIN_POSITIVE),
            Err(DemoError::NonFinitePower)
        );
    }

    #[test]
    fn derived_power_fails_if_no_rms_measurement_precedes_it() {
        let mut backend = SimulatedBackend::new(1, false, Some(8.0));
        backend
            .apply_stimulus(&SimulatedStimulus {
                requested_vrms: 0.5,
                frequency_hz: 1_000.0,
            })
            .unwrap();
        let request = MeasurementRequest {
            function: "simulated.output_power".to_owned(),
            unit: Some("W".to_owned()),
        };

        assert_eq!(
            backend.measure(&request),
            Err(DemoError::MissingRmsForPower)
        );
    }

    #[test]
    fn negative_mock_rms_is_a_failure_and_does_not_produce_power() {
        let mut backend = SimulatedBackend::new(1, false, Some(8.0));
        backend.analyzer = MockAnalyzer::new([MockScenario {
            channel: Channel::A,
            pending_polls: 0,
            delay: Duration::ZERO,
            completion: Ok(Measurement {
                channel: Channel::A,
                raw_word: None,
                raw_format: None,
                decoded_value: None,
                scaled_value: Some(ScaledValue {
                    value: -0.5,
                    unit: PhysicalUnit::VoltRms,
                }),
                status: MeasurementStatus::Numeric,
                range: None,
            }),
        }]);
        backend
            .apply_stimulus(&SimulatedStimulus {
                requested_vrms: 0.5,
                frequency_hz: 1_000.0,
            })
            .unwrap();
        let rms = MeasurementRequest {
            function: "simulated.rms".to_owned(),
            unit: Some("V RMS".to_owned()),
        };
        assert_eq!(backend.measure(&rms), Err(DemoError::InvalidRms));

        let power = MeasurementRequest {
            function: "simulated.output_power".to_owned(),
            unit: Some("W".to_owned()),
        };
        assert_eq!(backend.measure(&power), Err(DemoError::MissingRmsForPower));
    }
}
