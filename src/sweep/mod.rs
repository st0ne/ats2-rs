//! Hardware-independent sweep plans and execution primitives.
//!
//! Axis, stimulus, measurement request, settling policy, error policy, and
//! result data are separate. Execution is delegated to a caller-provided
//! backend; this module contains no device or timing implementation.

pub mod demo;
pub mod export;

use std::{collections::HashSet, sync::Arc, time::Duration};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepAxis {
    pub name: String,
    pub unit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisEndpoint {
    Start,
    End,
}

/// Value-generation strategy for a sweep axis.
#[derive(Debug, Clone, PartialEq)]
pub enum SweepAxisSpec {
    Linear { start: f64, end: f64, points: usize },
    Logarithmic { start: f64, end: f64, points: usize },
    Explicit { values: Vec<f64> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepOrder {
    /// Keep generated endpoint direction or the explicit input order.
    AsSpecified,
    Ascending,
    Descending,
}

impl SweepAxisSpec {
    /// Generate a validated axis sequence without performing timing or I/O.
    pub fn generate_values(&self, order: SweepOrder) -> Result<Vec<f64>, SweepPlanError> {
        let mut values = match self {
            Self::Linear { start, end, points } => {
                validate_interval(*start, *end, *points, false)?;
                interpolate(*start, *end, *points, false)?
            }
            Self::Logarithmic { start, end, points } => {
                validate_interval(*start, *end, *points, true)?;
                interpolate(*start, *end, *points, true)?
            }
            Self::Explicit { values } => {
                if values.is_empty() {
                    return Err(SweepPlanError::EmptyAxisValues);
                }
                if let Some((index, _)) = values
                    .iter()
                    .enumerate()
                    .find(|(_, value)| !value.is_finite())
                {
                    return Err(SweepPlanError::NonFiniteAxisValue { index });
                }
                values.clone()
            }
        };

        match order {
            SweepOrder::AsSpecified => {}
            SweepOrder::Ascending => values.sort_by(f64::total_cmp),
            SweepOrder::Descending => {
                values.sort_by(f64::total_cmp);
                values.reverse();
            }
        }
        Ok(values)
    }
}

fn validate_interval(
    start: f64,
    end: f64,
    points: usize,
    logarithmic: bool,
) -> Result<(), SweepPlanError> {
    for (endpoint, value) in [(AxisEndpoint::Start, start), (AxisEndpoint::End, end)] {
        if !value.is_finite() {
            return Err(SweepPlanError::NonFiniteAxisEndpoint { endpoint });
        }
        if logarithmic && value <= 0.0 {
            return Err(SweepPlanError::NonPositiveLogEndpoint { endpoint });
        }
    }
    if start == end {
        return Err(SweepPlanError::DegenerateInterval);
    }
    if points < 2 {
        return Err(SweepPlanError::InvalidPointCount { count: points });
    }
    Ok(())
}

fn interpolate(
    start: f64,
    end: f64,
    points: usize,
    logarithmic: bool,
) -> Result<Vec<f64>, SweepPlanError> {
    let mut values = Vec::with_capacity(points);
    let start_log = if logarithmic { Some(start.ln()) } else { None };
    let end_log = if logarithmic { Some(end.ln()) } else { None };
    let denominator = (points - 1) as f64;

    for index in 0..points {
        let value = if index == 0 {
            start
        } else if index + 1 == points {
            end
        } else {
            let fraction = index as f64 / denominator;
            if let (Some(start_log), Some(end_log)) = (start_log, end_log) {
                (start_log * (1.0 - fraction) + end_log * fraction).exp()
            } else {
                start * (1.0 - fraction) + end * fraction
            }
        };
        if !value.is_finite() {
            return Err(SweepPlanError::NonFiniteAxisValue { index });
        }
        values.push(value);
    }
    Ok(values)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementRequest {
    /// Extensible function identifier, e.g. `rms` or `thd+n`.
    pub function: String,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlingPolicy {
    None,
    /// Requested duration only; the backend decides how to realize it.
    Delay(Duration),
    /// The backend applies its own explicitly configured settling behavior.
    BackendDefined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePolicy {
    Abort,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSamplePolicy {
    /// Return an error at the first failed or non-finite sample.
    Reject,
    /// Exclude failed/non-finite samples, but report the omitted count.
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSampleReason {
    MeasurementFailure,
    NonFiniteValue,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SweepStatisticsError {
    #[error("cannot calculate statistics from an empty sample set")]
    EmptySamples,
    #[error("sample {index} is invalid: {reason:?}")]
    InvalidSample {
        index: usize,
        reason: InvalidSampleReason,
    },
    #[error("no valid samples remain after omitting {invalid_samples} invalid samples")]
    NoValidSamples { invalid_samples: usize },
}

/// Descriptive population statistics for the observed valid values only.
/// This is not an uncertainty estimate, accuracy claim, or independence model.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementStatistics {
    pub sample_count_total: usize,
    pub valid_sample_count: usize,
    pub invalid_sample_count: usize,
    pub mean: Option<f64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub population_standard_deviation: Option<f64>,
}

/// Summarize numeric samples; measurement errors and non-finite values are invalid.
pub fn summarize_samples<E>(
    samples: &[Result<f64, E>],
    invalid_policy: InvalidSamplePolicy,
) -> Result<MeasurementStatistics, SweepStatisticsError> {
    if samples.is_empty() {
        return Err(SweepStatisticsError::EmptySamples);
    }

    let mut valid = Vec::with_capacity(samples.len());
    let mut invalid_count = 0;
    for (index, sample) in samples.iter().enumerate() {
        let value = match sample {
            Ok(value) if value.is_finite() => Some(*value),
            Ok(_) => {
                invalid_count += 1;
                if invalid_policy == InvalidSamplePolicy::Reject {
                    return Err(SweepStatisticsError::InvalidSample {
                        index,
                        reason: InvalidSampleReason::NonFiniteValue,
                    });
                }
                None
            }
            Err(_) => {
                invalid_count += 1;
                if invalid_policy == InvalidSamplePolicy::Reject {
                    return Err(SweepStatisticsError::InvalidSample {
                        index,
                        reason: InvalidSampleReason::MeasurementFailure,
                    });
                }
                None
            }
        };
        if let Some(value) = value {
            valid.push(value);
        }
    }

    if valid.is_empty() {
        return Err(SweepStatisticsError::NoValidSamples {
            invalid_samples: invalid_count,
        });
    }

    let scale = valid
        .iter()
        .fold(0.0_f64, |largest, value| largest.max(value.abs()));
    let (mean, population_standard_deviation) = if scale == 0.0 {
        (0.0, 0.0)
    } else {
        let normalized_mean = (valid.iter().map(|value| value / scale).sum::<f64>()
            / valid.len() as f64)
            .clamp(-1.0, 1.0);
        let normalized_variance = valid
            .iter()
            .map(|value| {
                let difference = value / scale - normalized_mean;
                difference * difference
            })
            .sum::<f64>()
            / valid.len() as f64;
        (
            normalized_mean * scale,
            normalized_variance.clamp(0.0, 1.0).sqrt() * scale,
        )
    };

    Ok(MeasurementStatistics {
        sample_count_total: samples.len(),
        valid_sample_count: valid.len(),
        invalid_sample_count: invalid_count,
        mean: Some(mean),
        minimum: valid.iter().copied().reduce(f64::min),
        maximum: valid.iter().copied().reduce(f64::max),
        population_standard_deviation: Some(population_standard_deviation),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatisticMetric {
    Mean,
    Minimum,
    Maximum,
    PopulationStandardDeviation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdComparison {
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatisticsThreshold {
    pub measurement_id: String,
    pub metric: StatisticMetric,
    pub comparison: ThresholdComparison,
    pub value: f64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RepeatedSweepPlanError {
    #[error("repetition count must be at least one; got {count}")]
    InvalidRepetitionCount { count: usize },
    #[error("threshold at index {index} is not finite")]
    NonFiniteThreshold { index: usize },
    #[error("threshold references unknown measurement identifier: {id}")]
    UnknownMeasurementId { id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepPoint<S> {
    pub axis_value: f64,
    pub stimulus: S,
    pub measurement: MeasurementRequest,
    pub settling: SettlingPolicy,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SweepPlanError {
    #[error("a sweep plan must contain at least one point")]
    Empty,
    #[error("a multi-measurement sweep must request at least one measurement")]
    EmptyMeasurements,
    #[error("explicit axis values must not be empty")]
    EmptyAxisValues,
    #[error("measurement identifier at index {index} is empty")]
    EmptyMeasurementId { index: usize },
    #[error("measurement identifier is duplicated: {id}")]
    DuplicateMeasurementId { id: String },
    #[error("axis value at index {index} is not finite")]
    NonFiniteAxisValue { index: usize },
    #[error("axis {endpoint:?} endpoint must be finite")]
    NonFiniteAxisEndpoint { endpoint: AxisEndpoint },
    #[error("logarithmic axis {endpoint:?} endpoint must be positive")]
    NonPositiveLogEndpoint { endpoint: AxisEndpoint },
    #[error("linear and logarithmic axes require at least two points; got {count}")]
    InvalidPointCount { count: usize },
    #[error("linear and logarithmic axis endpoints must differ")]
    DegenerateInterval,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepPlan<S> {
    axis: SweepAxis,
    points: Vec<SweepPoint<S>>,
    failure_policy: FailurePolicy,
}

impl<S> SweepPlan<S> {
    pub fn new(
        axis: SweepAxis,
        points: Vec<SweepPoint<S>>,
        failure_policy: FailurePolicy,
    ) -> Result<Self, SweepPlanError> {
        if points.is_empty() {
            return Err(SweepPlanError::Empty);
        }
        if let Some((index, _)) = points
            .iter()
            .enumerate()
            .find(|(_, point)| !point.axis_value.is_finite())
        {
            return Err(SweepPlanError::NonFiniteAxisValue { index });
        }
        Ok(Self {
            axis,
            points,
            failure_policy,
        })
    }

    #[must_use]
    pub fn axis(&self) -> &SweepAxis {
        &self.axis
    }

    #[must_use]
    pub fn points(&self) -> &[SweepPoint<S>] {
        &self.points
    }

    #[must_use]
    pub const fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

/// A measurement request with a stable identifier within one sweep plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedMeasurementRequest {
    pub id: String,
    pub request: MeasurementRequest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiSweepPoint<S> {
    pub axis_value: f64,
    pub stimulus: S,
    pub settling: SettlingPolicy,
}

/// A sweep plan that performs every identified measurement at each point.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiSweepPlan<S> {
    axis: SweepAxis,
    points: Vec<MultiSweepPoint<S>>,
    measurements: Vec<IdentifiedMeasurementRequest>,
    failure_policy: FailurePolicy,
}

impl<S> MultiSweepPlan<S> {
    pub fn new(
        axis: SweepAxis,
        points: Vec<MultiSweepPoint<S>>,
        measurements: Vec<IdentifiedMeasurementRequest>,
        failure_policy: FailurePolicy,
    ) -> Result<Self, SweepPlanError> {
        if points.is_empty() {
            return Err(SweepPlanError::Empty);
        }
        if measurements.is_empty() {
            return Err(SweepPlanError::EmptyMeasurements);
        }
        if let Some((index, _)) = points
            .iter()
            .enumerate()
            .find(|(_, point)| !point.axis_value.is_finite())
        {
            return Err(SweepPlanError::NonFiniteAxisValue { index });
        }

        let mut identifiers = HashSet::with_capacity(measurements.len());
        for (index, measurement) in measurements.iter().enumerate() {
            if measurement.id.trim().is_empty() {
                return Err(SweepPlanError::EmptyMeasurementId { index });
            }
            if !identifiers.insert(measurement.id.as_str()) {
                return Err(SweepPlanError::DuplicateMeasurementId {
                    id: measurement.id.clone(),
                });
            }
        }

        Ok(Self {
            axis,
            points,
            measurements,
            failure_policy,
        })
    }

    #[must_use]
    pub fn axis(&self) -> &SweepAxis {
        &self.axis
    }

    #[must_use]
    pub fn points(&self) -> &[MultiSweepPoint<S>] {
        &self.points
    }

    #[must_use]
    pub fn measurements(&self) -> &[IdentifiedMeasurementRequest] {
        &self.measurements
    }

    #[must_use]
    pub const fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedMultiSweepPlan<S> {
    sweep: MultiSweepPlan<S>,
    repetitions: usize,
    invalid_sample_policy: InvalidSamplePolicy,
    thresholds: Vec<StatisticsThreshold>,
}

impl<S> RepeatedMultiSweepPlan<S> {
    pub fn new(
        sweep: MultiSweepPlan<S>,
        repetitions: usize,
        invalid_sample_policy: InvalidSamplePolicy,
        thresholds: Vec<StatisticsThreshold>,
    ) -> Result<Self, RepeatedSweepPlanError> {
        if repetitions == 0 {
            return Err(RepeatedSweepPlanError::InvalidRepetitionCount { count: repetitions });
        }
        for (index, threshold) in thresholds.iter().enumerate() {
            if !threshold.value.is_finite() {
                return Err(RepeatedSweepPlanError::NonFiniteThreshold { index });
            }
            if !sweep
                .measurements
                .iter()
                .any(|measurement| measurement.id == threshold.measurement_id)
            {
                return Err(RepeatedSweepPlanError::UnknownMeasurementId {
                    id: threshold.measurement_id.clone(),
                });
            }
        }
        Ok(Self {
            sweep,
            repetitions,
            invalid_sample_policy,
            thresholds,
        })
    }

    #[must_use]
    pub fn sweep(&self) -> &MultiSweepPlan<S> {
        &self.sweep
    }

    #[must_use]
    pub const fn repetitions(&self) -> usize {
        self.repetitions
    }

    #[must_use]
    pub const fn invalid_sample_policy(&self) -> InvalidSamplePolicy {
        self.invalid_sample_policy
    }

    #[must_use]
    pub fn thresholds(&self) -> &[StatisticsThreshold] {
        &self.thresholds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepStage {
    Stimulus,
    Settling,
    Measurement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepFailure<E> {
    pub stage: SweepStage,
    pub source: E,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepRecord<M, E> {
    pub axis_value: f64,
    pub measurement: MeasurementRequest,
    pub result: Result<M, SweepFailure<E>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepDataset<M, E> {
    pub axis: SweepAxis,
    pub records: Vec<SweepRecord<M, E>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepOutcome<M, E> {
    pub dataset: SweepDataset<M, E>,
    /// Index of the first failed point when `FailurePolicy::Abort` is used.
    pub aborted_at: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiSweepMeasurementRecord<M, E> {
    pub id: String,
    pub request: MeasurementRequest,
    /// `Ok` means the backend completed this request; an analyzer reading may
    /// still carry its own explicit status. Failures are isolated per request.
    pub result: Result<M, SweepFailure<Arc<E>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiSweepRecord<M, E> {
    pub axis_value: f64,
    pub measurements: Vec<MultiSweepMeasurementRecord<M, E>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiSweepDataset<M, E> {
    pub axis: SweepAxis,
    pub records: Vec<MultiSweepRecord<M, E>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiSweepOutcome<M, E> {
    pub dataset: MultiSweepDataset<M, E>,
    /// Index of the first failed point when `FailurePolicy::Abort` is used.
    pub aborted_at: Option<usize>,
}

/// Point-level progress emitted by a controlled sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepProgress {
    pub completed_points: usize,
    pub total_points: usize,
}

/// Result of a sweep that can be cancelled between points.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlledMultiSweepOutcome<M, E> {
    pub outcome: MultiSweepOutcome<M, E>,
    /// Index of the next point when cancellation was observed.
    pub cancelled_at: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatedMeasurementState {
    Attempted,
    Skipped { stage: SweepStage },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedMeasurementRecord<E> {
    pub id: String,
    pub request: MeasurementRequest,
    pub state: RepeatedMeasurementState,
    pub samples: Vec<Result<f64, SweepFailure<Arc<E>>>>,
    /// `None` means the point failed before measurement began.
    pub statistics: Option<Result<MeasurementStatistics, SweepStatisticsError>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedMultiSweepRecord<E> {
    pub axis_value: f64,
    pub point_failure: Option<SweepFailure<Arc<E>>>,
    pub measurements: Vec<RepeatedMeasurementRecord<E>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedMultiSweepDataset<E> {
    pub axis: SweepAxis,
    pub records: Vec<RepeatedMultiSweepRecord<E>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatisticsThresholdHit {
    pub measurement_id: String,
    pub metric: StatisticMetric,
    pub comparison: ThresholdComparison,
    pub threshold: f64,
    pub observed_value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedMultiSweepOutcome<E> {
    pub dataset: RepeatedMultiSweepDataset<E>,
    pub aborted_at: Option<usize>,
    pub threshold_triggered: Option<StatisticsThresholdHit>,
}

/// Backend boundary for stimulus, settling, and measurement operations.
///
/// A backend owns all external side effects. The sweep core only calls these
/// methods in order and does not sleep or communicate with hardware itself.
pub trait SweepBackend<S> {
    type Reading;
    type Error;

    fn apply_stimulus(&mut self, stimulus: &S) -> Result<(), Self::Error>;
    fn settle(&mut self, policy: SettlingPolicy) -> Result<(), Self::Error>;
    fn measure(&mut self, request: &MeasurementRequest) -> Result<Self::Reading, Self::Error>;
}

/// Execute a prevalidated plan while retaining each result and its failure stage.
pub fn run_sweep<S, B>(plan: &SweepPlan<S>, backend: &mut B) -> SweepOutcome<B::Reading, B::Error>
where
    B: SweepBackend<S>,
{
    let mut records = Vec::with_capacity(plan.points.len());
    let mut aborted_at = None;

    for (index, point) in plan.points.iter().enumerate() {
        let result = match backend.apply_stimulus(&point.stimulus) {
            Err(source) => Err(SweepFailure {
                stage: SweepStage::Stimulus,
                source,
            }),
            Ok(()) => match backend.settle(point.settling) {
                Err(source) => Err(SweepFailure {
                    stage: SweepStage::Settling,
                    source,
                }),
                Ok(()) => backend
                    .measure(&point.measurement)
                    .map_err(|source| SweepFailure {
                        stage: SweepStage::Measurement,
                        source,
                    }),
            },
        };
        let failed = result.is_err();
        records.push(SweepRecord {
            axis_value: point.axis_value,
            measurement: point.measurement.clone(),
            result,
        });
        if failed && plan.failure_policy == FailurePolicy::Abort {
            aborted_at = Some(index);
            break;
        }
    }

    SweepOutcome {
        dataset: SweepDataset {
            axis: plan.axis.clone(),
            records,
        },
        aborted_at,
    }
}

/// Execute all identified measurements per point, keeping failures isolated.
///
/// A stimulus or settling failure is shared by all measurement records at that
/// point. A measurement failure does not skip the other requests at that point.
/// `Arc` preserves one non-cloneable backend error while attaching the shared
/// point-stage failure to every affected measurement.
pub fn run_multi_sweep<S, B>(
    plan: &MultiSweepPlan<S>,
    backend: &mut B,
) -> MultiSweepOutcome<B::Reading, B::Error>
where
    B: SweepBackend<S>,
{
    run_multi_sweep_controlled(plan, backend, || false, |_| {}).outcome
}

/// Execute all identified measurements per point with progress and safe-point cancellation.
///
/// Cancellation is checked before each point. It cannot interrupt a backend operation already in
/// progress; after cancellation, completed point data is retained and remaining points are omitted.
pub fn run_multi_sweep_controlled<S, B>(
    plan: &MultiSweepPlan<S>,
    backend: &mut B,
    mut is_cancelled: impl FnMut() -> bool,
    mut on_progress: impl FnMut(SweepProgress),
) -> ControlledMultiSweepOutcome<B::Reading, B::Error>
where
    B: SweepBackend<S>,
{
    let mut records = Vec::with_capacity(plan.points.len());
    let mut aborted_at = None;
    let mut cancelled_at = None;

    for (index, point) in plan.points.iter().enumerate() {
        if is_cancelled() {
            cancelled_at = Some(index);
            break;
        }

        let point_failure = match backend.apply_stimulus(&point.stimulus) {
            Err(source) => Some(SweepFailure {
                stage: SweepStage::Stimulus,
                source: Arc::new(source),
            }),
            Ok(()) => backend
                .settle(point.settling)
                .err()
                .map(|source| SweepFailure {
                    stage: SweepStage::Settling,
                    source: Arc::new(source),
                }),
        };

        let measurements: Vec<MultiSweepMeasurementRecord<B::Reading, B::Error>> =
            if let Some(failure) = point_failure {
                plan.measurements
                    .iter()
                    .map(|request| MultiSweepMeasurementRecord {
                        id: request.id.clone(),
                        request: request.request.clone(),
                        result: Err(SweepFailure {
                            stage: failure.stage,
                            source: Arc::clone(&failure.source),
                        }),
                    })
                    .collect()
            } else {
                plan.measurements
                    .iter()
                    .map(|request| MultiSweepMeasurementRecord {
                        id: request.id.clone(),
                        request: request.request.clone(),
                        result: backend
                            .measure(&request.request)
                            .map_err(|source| SweepFailure {
                                stage: SweepStage::Measurement,
                                source: Arc::new(source),
                            }),
                    })
                    .collect()
            };
        let failed = measurements.iter().any(
            |measurement: &MultiSweepMeasurementRecord<B::Reading, B::Error>| {
                measurement.result.is_err()
            },
        );
        records.push(MultiSweepRecord {
            axis_value: point.axis_value,
            measurements,
        });
        on_progress(SweepProgress {
            completed_points: index + 1,
            total_points: plan.points.len(),
        });

        if failed && plan.failure_policy == FailurePolicy::Abort {
            aborted_at = Some(index);
            break;
        }
    }

    ControlledMultiSweepOutcome {
        outcome: MultiSweepOutcome {
            dataset: MultiSweepDataset {
                axis: plan.axis.clone(),
                records,
            },
            aborted_at,
        },
        cancelled_at,
    }
}

/// Repeat every numeric measurement at each point and compute descriptive stats.
/// Stimulus and settling operations run once per point; each measurement is then
/// requested `repetitions` times. Thresholds stop subsequent points after the
/// complete current point has been recorded.
pub fn run_repeated_multi_sweep<S, B>(
    plan: &RepeatedMultiSweepPlan<S>,
    backend: &mut B,
) -> RepeatedMultiSweepOutcome<B::Error>
where
    B: SweepBackend<S, Reading = f64>,
{
    let sweep = &plan.sweep;
    let mut records = Vec::with_capacity(sweep.points.len());
    let mut aborted_at = None;
    let mut threshold_triggered = None;

    for (point_index, point) in sweep.points.iter().enumerate() {
        let point_failure = match backend.apply_stimulus(&point.stimulus) {
            Err(source) => Some(SweepFailure {
                stage: SweepStage::Stimulus,
                source: Arc::new(source),
            }),
            Ok(()) => backend
                .settle(point.settling)
                .err()
                .map(|source| SweepFailure {
                    stage: SweepStage::Settling,
                    source: Arc::new(source),
                }),
        };

        let mut failed = point_failure.is_some();
        let measurements: Vec<RepeatedMeasurementRecord<B::Error>> =
            if let Some(failure) = point_failure.as_ref() {
                sweep
                    .measurements
                    .iter()
                    .map(|measurement| RepeatedMeasurementRecord {
                        id: measurement.id.clone(),
                        request: measurement.request.clone(),
                        state: RepeatedMeasurementState::Skipped {
                            stage: failure.stage,
                        },
                        samples: Vec::new(),
                        statistics: None,
                    })
                    .collect()
            } else {
                sweep
                    .measurements
                    .iter()
                    .map(|measurement| {
                        let samples: Vec<Result<f64, SweepFailure<Arc<B::Error>>>> = (0..plan
                            .repetitions)
                            .map(|_| {
                                backend.measure(&measurement.request).map_err(|source| {
                                    SweepFailure {
                                        stage: SweepStage::Measurement,
                                        source: Arc::new(source),
                                    }
                                })
                            })
                            .collect();
                        let numeric_samples: Vec<Result<f64, ()>> = samples
                            .iter()
                            .map(|sample| sample.as_ref().map(|value| *value).map_err(|_| ()))
                            .collect();
                        let statistics =
                            summarize_samples(&numeric_samples, plan.invalid_sample_policy);
                        failed |= samples.iter().any(Result::is_err);
                        failed |= statistics.is_err();
                        RepeatedMeasurementRecord {
                            id: measurement.id.clone(),
                            request: measurement.request.clone(),
                            state: RepeatedMeasurementState::Attempted,
                            samples,
                            statistics: Some(statistics),
                        }
                    })
                    .collect()
            };

        if threshold_triggered.is_none() {
            threshold_triggered = plan.thresholds.iter().find_map(|threshold| {
                let measurement = measurements
                    .iter()
                    .find(|measurement| measurement.id == threshold.measurement_id)?;
                let summary = measurement.statistics.as_ref()?.as_ref().ok()?;
                threshold_match(threshold, summary)
            });
        }

        records.push(RepeatedMultiSweepRecord {
            axis_value: point.axis_value,
            point_failure,
            measurements,
        });

        if threshold_triggered.is_some() || (failed && sweep.failure_policy == FailurePolicy::Abort)
        {
            aborted_at = Some(point_index);
            break;
        }
    }

    RepeatedMultiSweepOutcome {
        dataset: RepeatedMultiSweepDataset {
            axis: sweep.axis.clone(),
            records,
        },
        aborted_at,
        threshold_triggered,
    }
}

fn threshold_match(
    threshold: &StatisticsThreshold,
    statistics: &MeasurementStatistics,
) -> Option<StatisticsThresholdHit> {
    let observed_value = match threshold.metric {
        StatisticMetric::Mean => statistics.mean?,
        StatisticMetric::Minimum => statistics.minimum?,
        StatisticMetric::Maximum => statistics.maximum?,
        StatisticMetric::PopulationStandardDeviation => statistics.population_standard_deviation?,
    };
    let triggered = match threshold.comparison {
        ThresholdComparison::GreaterThan => observed_value > threshold.value,
        ThresholdComparison::GreaterThanOrEqual => observed_value >= threshold.value,
        ThresholdComparison::LessThan => observed_value < threshold.value,
        ThresholdComparison::LessThanOrEqual => observed_value <= threshold.value,
    };
    triggered.then(|| StatisticsThresholdHit {
        measurement_id: threshold.measurement_id.clone(),
        metric: threshold.metric,
        comparison: threshold.comparison,
        threshold: threshold.value,
        observed_value,
    })
}
