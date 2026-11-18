use std::{collections::VecDeque, time::Duration};

use ats2_rs::{
    analyzer::{
        config::Channel,
        decoder::AnalyzerRawFormat,
        measurement::{Measurement, MeasurementStatus},
        mock::{MockAnalyzer, MockError, MockFailure, MockScenario},
    },
    sweep::{
        AxisEndpoint, FailurePolicy, IdentifiedMeasurementRequest, InvalidSamplePolicy,
        MeasurementRequest, MultiSweepPlan, MultiSweepPoint, RepeatedMultiSweepPlan,
        SettlingPolicy, StatisticMetric, StatisticsThreshold, SweepAxis, SweepAxisSpec,
        SweepBackend, SweepFailure, SweepOrder, SweepPlan, SweepPlanError, SweepPoint, SweepStage,
        SweepStatisticsError, ThresholdComparison, run_multi_sweep, run_repeated_multi_sweep,
        run_sweep, summarize_samples,
    },
};

fn axis() -> SweepAxis {
    SweepAxis {
        name: "frequency".into(),
        unit: "Hz".into(),
    }
}

#[test]
fn empty_sweep_plan_is_rejected() {
    assert_eq!(
        SweepPlan::<u32>::new(axis(), Vec::new(), FailurePolicy::Abort),
        Err(SweepPlanError::Empty)
    );
}

#[test]
fn non_finite_axis_value_is_rejected() {
    let result = SweepPlan::new(
        axis(),
        vec![SweepPoint {
            axis_value: f64::NAN,
            stimulus: (),
            measurement: MeasurementRequest {
                function: "rms".into(),
                unit: Some("V".into()),
            },
            settling: SettlingPolicy::None,
        }],
        FailurePolicy::Abort,
    );
    assert_eq!(result, Err(SweepPlanError::NonFiniteAxisValue { index: 0 }));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendError {
    Injected,
}

struct RecordingBackend {
    events: Vec<String>,
    results: VecDeque<Result<f64, BackendError>>,
    fail_stage: Option<SweepStage>,
}

impl SweepBackend<String> for RecordingBackend {
    type Reading = f64;
    type Error = BackendError;

    fn apply_stimulus(&mut self, stimulus: &String) -> Result<(), Self::Error> {
        self.events.push(format!("stimulus:{stimulus}"));
        if self.fail_stage == Some(SweepStage::Stimulus) {
            Err(BackendError::Injected)
        } else {
            Ok(())
        }
    }

    fn settle(&mut self, policy: SettlingPolicy) -> Result<(), Self::Error> {
        self.events.push(format!("settle:{policy:?}"));
        if self.fail_stage == Some(SweepStage::Settling) {
            Err(BackendError::Injected)
        } else {
            Ok(())
        }
    }

    fn measure(&mut self, request: &MeasurementRequest) -> Result<Self::Reading, Self::Error> {
        self.events.push(format!(
            "measure:{}:{}",
            request.function,
            request.unit.as_deref().unwrap_or("unspecified")
        ));
        self.results.pop_front().expect("test provides a result")
    }
}

#[test]
fn sweep_execution_orders_stimulus_settling_and_measurement() {
    let point = SweepPoint {
        axis_value: 1_000.0,
        stimulus: "tone-1".to_owned(),
        measurement: MeasurementRequest {
            function: "custom.thdn".into(),
            unit: Some("%".into()),
        },
        settling: SettlingPolicy::Delay(Duration::from_millis(3)),
    };
    let plan = SweepPlan::new(axis(), vec![point], FailurePolicy::Abort).unwrap();
    let mut backend = RecordingBackend {
        events: Vec::new(),
        results: VecDeque::from([Ok(1.25)]),
        fail_stage: None,
    };

    let outcome = run_sweep(&plan, &mut backend);

    assert_eq!(
        backend.events,
        [
            "stimulus:tone-1",
            "settle:Delay(3ms)",
            "measure:custom.thdn:%"
        ]
    );
    assert_eq!(outcome.dataset.axis, axis());
    assert_eq!(outcome.dataset.records.len(), 1);
    assert_eq!(outcome.dataset.records[0].axis_value, 1_000.0);
    assert_eq!(
        outcome.dataset.records[0].measurement.function,
        "custom.thdn"
    );
    assert_eq!(outcome.dataset.records[0].result, Ok(1.25));
    assert_eq!(outcome.aborted_at, None);
}

fn point(axis_value: f64, stimulus: &str) -> SweepPoint<String> {
    SweepPoint {
        axis_value,
        stimulus: stimulus.to_owned(),
        measurement: MeasurementRequest {
            function: "rms".into(),
            unit: Some("V".into()),
        },
        settling: SettlingPolicy::None,
    }
}

#[test]
fn continue_policy_records_failure_and_runs_later_points() {
    let plan = SweepPlan::new(
        axis(),
        vec![point(1.0, "first"), point(2.0, "second")],
        FailurePolicy::Continue,
    )
    .unwrap();
    let mut backend = RecordingBackend {
        events: Vec::new(),
        results: VecDeque::from([Err(BackendError::Injected), Ok(2.0)]),
        fail_stage: None,
    };

    let outcome = run_sweep(&plan, &mut backend);

    assert_eq!(outcome.aborted_at, None);
    assert_eq!(outcome.dataset.records.len(), 2);
    assert_eq!(
        outcome.dataset.records[0].result,
        Err(SweepFailure {
            stage: SweepStage::Measurement,
            source: BackendError::Injected,
        })
    );
    assert_eq!(outcome.dataset.records[1].result, Ok(2.0));
    assert_eq!(backend.events.len(), 6);
}

#[test]
fn abort_policy_preserves_first_failure_and_skips_remaining_points() {
    let plan = SweepPlan::new(
        axis(),
        vec![point(1.0, "first"), point(2.0, "second")],
        FailurePolicy::Abort,
    )
    .unwrap();
    let mut backend = RecordingBackend {
        events: Vec::new(),
        results: VecDeque::from([Err(BackendError::Injected), Ok(2.0)]),
        fail_stage: None,
    };

    let outcome = run_sweep(&plan, &mut backend);

    assert_eq!(outcome.aborted_at, Some(0));
    assert_eq!(outcome.dataset.records.len(), 1);
    assert_eq!(backend.events.len(), 3);
    assert_eq!(
        outcome.dataset.records[0].result,
        Err(SweepFailure {
            stage: SweepStage::Measurement,
            source: BackendError::Injected,
        })
    );
}

#[test]
fn stimulus_and_settling_failures_are_classified_and_short_circuit() {
    for (stage, expected_events) in [(SweepStage::Stimulus, 1), (SweepStage::Settling, 2)] {
        let plan = SweepPlan::new(axis(), vec![point(1.0, "first")], FailurePolicy::Abort).unwrap();
        let mut backend = RecordingBackend {
            events: Vec::new(),
            results: VecDeque::new(),
            fail_stage: Some(stage),
        };

        let outcome = run_sweep(&plan, &mut backend);

        assert_eq!(outcome.aborted_at, Some(0));
        assert_eq!(backend.events.len(), expected_events);
        assert_eq!(
            outcome.dataset.records[0].result,
            Err(SweepFailure {
                stage,
                source: BackendError::Injected,
            })
        );
    }
}

struct AnalyzerSweepBackend {
    analyzer: MockAnalyzer,
    channel: Channel,
    requests: Vec<String>,
}

impl SweepBackend<String> for AnalyzerSweepBackend {
    type Reading = Measurement;
    type Error = MockError;

    fn apply_stimulus(&mut self, _stimulus: &String) -> Result<(), Self::Error> {
        Ok(())
    }

    fn settle(&mut self, _policy: SettlingPolicy) -> Result<(), Self::Error> {
        Ok(())
    }

    fn measure(&mut self, request: &MeasurementRequest) -> Result<Self::Reading, Self::Error> {
        self.requests.push(request.function.clone());
        self.analyzer.trigger(self.channel)?;
        for _ in 0..64 {
            if self.analyzer.poll_ready()? {
                return self.analyzer.read();
            }
        }
        Err(MockError::NotReady)
    }
}

fn mock_measurement(raw_word: u32, decoded_value: f64) -> Measurement {
    Measurement {
        channel: Channel::A,
        raw_word: Some(raw_word),
        raw_format: Some(AnalyzerRawFormat::SignedQ1_23),
        decoded_value: Some(decoded_value),
        scaled_value: None,
        status: MeasurementStatus::Numeric,
        range: None,
    }
}

fn identified(id: &str, function: &str, unit: &str) -> IdentifiedMeasurementRequest {
    IdentifiedMeasurementRequest {
        id: id.into(),
        request: MeasurementRequest {
            function: function.into(),
            unit: Some(unit.into()),
        },
    }
}

#[test]
fn multi_sweep_keeps_partial_mock_measurement_failures_per_identifier() {
    let plan = MultiSweepPlan::new(
        axis(),
        vec![
            MultiSweepPoint {
                axis_value: 1.0,
                stimulus: "tone-1".into(),
                settling: SettlingPolicy::None,
            },
            MultiSweepPoint {
                axis_value: 2.0,
                stimulus: "tone-2".into(),
                settling: SettlingPolicy::None,
            },
        ],
        vec![
            identified("level", "rms", "V"),
            identified("distortion", "thd+n", "%"),
        ],
        FailurePolicy::Continue,
    )
    .unwrap();
    let mut backend = AnalyzerSweepBackend {
        analyzer: MockAnalyzer::new([
            MockScenario {
                channel: Channel::A,
                pending_polls: 1,
                delay: Duration::ZERO,
                completion: Err(MockFailure::Timeout),
            },
            MockScenario {
                channel: Channel::A,
                pending_polls: 0,
                delay: Duration::ZERO,
                completion: Ok(mock_measurement(0x20, 0.25)),
            },
            MockScenario {
                channel: Channel::A,
                pending_polls: 0,
                delay: Duration::ZERO,
                completion: Ok(mock_measurement(0x40, 0.5)),
            },
            MockScenario {
                channel: Channel::A,
                pending_polls: 0,
                delay: Duration::ZERO,
                completion: Ok(mock_measurement(0x60, 0.75)),
            },
        ]),
        channel: Channel::A,
        requests: Vec::new(),
    };

    let outcome = run_multi_sweep(&plan, &mut backend);

    assert_eq!(outcome.aborted_at, None);
    assert_eq!(outcome.dataset.records.len(), 2);
    assert_eq!(backend.requests, ["rms", "thd+n", "rms", "thd+n"]);
    let first_point = &outcome.dataset.records[0];
    assert_eq!(first_point.measurements.len(), 2);
    assert_eq!(first_point.measurements[0].id, "level");
    assert_eq!(
        first_point.measurements[0].request.unit.as_deref(),
        Some("V")
    );
    assert!(matches!(
        first_point.measurements[0].result,
        Err(SweepFailure {
            stage: SweepStage::Measurement,
            ..
        })
    ));
    assert_eq!(
        first_point.measurements[1].result,
        Ok(mock_measurement(0x20, 0.25))
    );
    assert_eq!(
        outcome.dataset.records[1].measurements[0].result,
        Ok(mock_measurement(0x40, 0.5))
    );
    assert_eq!(
        outcome.dataset.records[1].measurements[1].result,
        Ok(mock_measurement(0x60, 0.75))
    );
}

#[test]
fn multi_sweep_rejects_duplicate_measurement_identifiers() {
    let result = MultiSweepPlan::new(
        axis(),
        vec![MultiSweepPoint {
            axis_value: 1.0,
            stimulus: (),
            settling: SettlingPolicy::None,
        }],
        vec![
            identified("same", "rms", "V"),
            identified("same", "thd+n", "%"),
        ],
        FailurePolicy::Abort,
    );

    assert_eq!(
        result,
        Err(SweepPlanError::DuplicateMeasurementId { id: "same".into() })
    );
}

#[test]
fn linear_axis_generates_inclusive_points_in_requested_order() {
    let axis = SweepAxisSpec::Linear {
        start: 1.0,
        end: 9.0,
        points: 3,
    };

    assert_eq!(
        axis.generate_values(SweepOrder::Ascending).unwrap(),
        [1.0, 5.0, 9.0]
    );
    assert_eq!(
        axis.generate_values(SweepOrder::Descending).unwrap(),
        [9.0, 5.0, 1.0]
    );
    assert_eq!(
        axis.generate_values(SweepOrder::AsSpecified).unwrap(),
        [1.0, 5.0, 9.0]
    );
}

#[test]
fn logarithmic_axis_generates_geometric_steps_and_descending_order() {
    let axis = SweepAxisSpec::Logarithmic {
        start: 10.0,
        end: 1_000.0,
        points: 3,
    };
    let ascending = axis.generate_values(SweepOrder::Ascending).unwrap();
    let descending = axis.generate_values(SweepOrder::Descending).unwrap();

    assert_eq!(ascending[0], 10.0);
    assert!((ascending[1] - 100.0).abs() < 1e-10);
    assert_eq!(ascending[2], 1_000.0);
    assert_eq!(descending, [1_000.0, ascending[1], 10.0]);
}

#[test]
fn explicit_axis_values_preserve_or_sort_requested_order() {
    let axis = SweepAxisSpec::Explicit {
        values: vec![4.0, 1.0, 2.0],
    };

    assert_eq!(
        axis.generate_values(SweepOrder::AsSpecified).unwrap(),
        [4.0, 1.0, 2.0]
    );
    assert_eq!(
        axis.generate_values(SweepOrder::Ascending).unwrap(),
        [1.0, 2.0, 4.0]
    );
    assert_eq!(
        axis.generate_values(SweepOrder::Descending).unwrap(),
        [4.0, 2.0, 1.0]
    );
}

#[test]
fn axis_generators_reject_invalid_boundaries_and_parameters() {
    assert_eq!(
        SweepAxisSpec::Linear {
            start: 1.0,
            end: 2.0,
            points: 1,
        }
        .generate_values(SweepOrder::AsSpecified),
        Err(SweepPlanError::InvalidPointCount { count: 1 })
    );
    assert_eq!(
        SweepAxisSpec::Linear {
            start: 2.0,
            end: 2.0,
            points: 3,
        }
        .generate_values(SweepOrder::AsSpecified),
        Err(SweepPlanError::DegenerateInterval)
    );
    assert_eq!(
        SweepAxisSpec::Linear {
            start: f64::NAN,
            end: 2.0,
            points: 3,
        }
        .generate_values(SweepOrder::AsSpecified),
        Err(SweepPlanError::NonFiniteAxisEndpoint {
            endpoint: AxisEndpoint::Start,
        })
    );
    assert_eq!(
        SweepAxisSpec::Logarithmic {
            start: 0.0,
            end: 10.0,
            points: 3,
        }
        .generate_values(SweepOrder::AsSpecified),
        Err(SweepPlanError::NonPositiveLogEndpoint {
            endpoint: AxisEndpoint::Start,
        })
    );
    assert_eq!(
        SweepAxisSpec::Explicit { values: Vec::new() }.generate_values(SweepOrder::AsSpecified),
        Err(SweepPlanError::EmptyAxisValues)
    );
    assert_eq!(
        SweepAxisSpec::Explicit {
            values: vec![1.0, f64::INFINITY],
        }
        .generate_values(SweepOrder::AsSpecified),
        Err(SweepPlanError::NonFiniteAxisValue { index: 1 })
    );
}

#[test]
fn repeated_statistics_compute_descriptive_population_values() {
    let summary = summarize_samples(
        &[Ok::<_, ()>(1.0), Ok(3.0), Ok(5.0)],
        InvalidSamplePolicy::Reject,
    )
    .unwrap();

    assert_eq!(summary.sample_count_total, 3);
    assert_eq!(summary.valid_sample_count, 3);
    assert_eq!(summary.invalid_sample_count, 0);
    assert_eq!(summary.mean, Some(3.0));
    assert_eq!(summary.minimum, Some(1.0));
    assert_eq!(summary.maximum, Some(5.0));
    assert!(
        (summary.population_standard_deviation.unwrap() - (8.0_f64 / 3.0).sqrt()).abs() < 1e-12
    );
}

#[test]
fn invalid_samples_are_rejected_or_omitted_explicitly() {
    let samples = [Ok::<_, ()>(1.0), Err(()), Ok(f64::NAN)];

    assert_eq!(
        summarize_samples(&samples, InvalidSamplePolicy::Reject),
        Err(SweepStatisticsError::InvalidSample {
            index: 1,
            reason: ats2_rs::sweep::InvalidSampleReason::MeasurementFailure,
        })
    );
    let omitted = summarize_samples(&samples, InvalidSamplePolicy::Omit).unwrap();
    assert_eq!(omitted.sample_count_total, 3);
    assert_eq!(omitted.valid_sample_count, 1);
    assert_eq!(omitted.invalid_sample_count, 2);
    assert_eq!(omitted.mean, Some(1.0));
    assert_eq!(
        summarize_samples::<()>(&[Err(()), Ok(f64::INFINITY)], InvalidSamplePolicy::Omit),
        Err(SweepStatisticsError::NoValidSamples { invalid_samples: 2 })
    );
}

#[test]
fn repeated_multi_sweep_repeats_each_measurement_and_aborts_on_threshold() {
    let sweep = MultiSweepPlan::new(
        axis(),
        vec![
            MultiSweepPoint {
                axis_value: 1.0,
                stimulus: "first".into(),
                settling: SettlingPolicy::None,
            },
            MultiSweepPoint {
                axis_value: 2.0,
                stimulus: "second".into(),
                settling: SettlingPolicy::Delay(Duration::from_millis(4)),
            },
            MultiSweepPoint {
                axis_value: 3.0,
                stimulus: "third".into(),
                settling: SettlingPolicy::None,
            },
        ],
        vec![identified("level", "rms", "V")],
        FailurePolicy::Continue,
    )
    .unwrap();
    let plan = RepeatedMultiSweepPlan::new(
        sweep,
        2,
        InvalidSamplePolicy::Reject,
        vec![StatisticsThreshold {
            measurement_id: "level".into(),
            metric: StatisticMetric::Mean,
            comparison: ThresholdComparison::GreaterThan,
            value: 2.0,
        }],
    )
    .unwrap();
    let mut backend = RecordingBackend {
        events: Vec::new(),
        results: VecDeque::from([Ok(1.0), Ok(3.0), Ok(2.0), Ok(4.0), Ok(99.0)]),
        fail_stage: None,
    };

    let outcome = run_repeated_multi_sweep(&plan, &mut backend);

    assert_eq!(outcome.aborted_at, Some(1));
    assert_eq!(outcome.dataset.records.len(), 2);
    assert_eq!(
        outcome.dataset.records[0].measurements[0].samples,
        [Ok(1.0), Ok(3.0)]
    );
    let summary = outcome.dataset.records[0].measurements[0]
        .statistics
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(summary.mean, Some(2.0));
    assert_eq!(
        outcome.dataset.records[1].measurements[0]
            .statistics
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .mean,
        Some(3.0)
    );
    assert_eq!(outcome.threshold_triggered.unwrap().observed_value, 3.0);
    assert_eq!(
        backend
            .events
            .iter()
            .filter(|event| event.starts_with("measure:"))
            .count(),
        4
    );
}
