use std::{cell::Cell, collections::BTreeMap};

use ats2_rs::sweep::{
    SweepAxisSpec, SweepProgress,
    demo::{DemoConfig, DemoSweepVariable, DemoXAxis, SIMULATION_NOTICE, simulate_demo_controlled},
};

#[test]
fn configurable_mock_demo_reports_progress_cancels_and_exports_partial_results() {
    let config = DemoConfig {
        display_axis: DemoXAxis::GeneratorLevelVrms,
        sweep_variable: DemoSweepVariable::GeneratorLevelVrms,
        axis: SweepAxisSpec::Linear {
            start: 0.2,
            end: 0.8,
            points: 4,
        },
        include_thd: true,
        load_resistance_ohms: None,
    };
    let cancel = Cell::new(false);
    let mut progress = Vec::<SweepProgress>::new();

    let run = simulate_demo_controlled(
        &config,
        || cancel.get(),
        |update| {
            progress.push(update);
            if update.completed_points == 1 {
                cancel.set(true);
            }
        },
    )
    .unwrap();

    assert_eq!(run.cancelled_at, Some(1));
    assert_eq!(run.demonstration.provenance, SIMULATION_NOTICE);
    assert_eq!(run.demonstration.points.len(), 1);
    assert_eq!(run.demonstration.simulated_stimuli_vrms, [0.2]);
    assert_eq!(
        progress,
        [SweepProgress {
            completed_points: 1,
            total_points: 4,
        }]
    );
    let json = run.export_json(&BTreeMap::new()).unwrap();
    assert!(json.contains("not_executed"));
    assert!(json.contains("\"aborted_at\":null"));
}

#[test]
fn frequency_sweep_exposes_frequency_response_and_resistive_power_as_mock_data() {
    let config = DemoConfig {
        display_axis: DemoXAxis::MeasuredPowerW,
        sweep_variable: DemoSweepVariable::FrequencyHz,
        axis: SweepAxisSpec::Logarithmic {
            start: 20.0,
            end: 20_000.0,
            points: 3,
        },
        include_thd: true,
        load_resistance_ohms: Some(8.0),
    };
    let run = simulate_demo_controlled(&config, || false, |_| {}).unwrap();

    assert_eq!(run.demonstration.provenance, SIMULATION_NOTICE);
    assert_eq!(run.demonstration.display_axis, DemoXAxis::MeasuredPowerW);
    assert_eq!(
        run.demonstration.sweep.dataset.axis.name,
        "simulated frequency"
    );
    assert_eq!(run.demonstration.sweep.dataset.axis.unit, "Hz");
    assert_eq!(run.demonstration.points.len(), 3);
    assert_eq!(run.demonstration.points[0].frequency_hz, 20.0);
    assert_eq!(run.demonstration.points[2].frequency_hz, 20_000.0);
    assert!(
        run.demonstration
            .points
            .iter()
            .enumerate()
            .all(|(index, point)| {
                point.generator_level_vrms == 0.5
                    && run.demonstration.measured_power_w(index)
                        == Some(point.measured_vrms * point.measured_vrms / 8.0)
            })
    );
}

#[test]
fn frequency_sweep_rejects_nonpositive_frequency_values() {
    let config = DemoConfig {
        display_axis: DemoXAxis::GeneratorLevelVrms,
        sweep_variable: DemoSweepVariable::FrequencyHz,
        axis: SweepAxisSpec::Linear {
            start: 0.0,
            end: 1_000.0,
            points: 3,
        },
        include_thd: true,
        load_resistance_ohms: None,
    };

    assert!(matches!(
        simulate_demo_controlled(&config, || false, |_| {}),
        Err(ats2_rs::sweep::demo::DemoError::InvalidFrequency)
    ));
}

#[test]
fn measured_power_display_axis_requires_explicit_load_resistance() {
    let config = DemoConfig {
        display_axis: DemoXAxis::MeasuredPowerW,
        sweep_variable: DemoSweepVariable::GeneratorLevelVrms,
        axis: SweepAxisSpec::Linear {
            start: 0.1,
            end: 1.0,
            points: 3,
        },
        include_thd: true,
        load_resistance_ohms: None,
    };

    assert!(matches!(
        simulate_demo_controlled(&config, || false, |_| {}),
        Err(ats2_rs::sweep::demo::DemoError::PowerAxisRequiresLoadResistance)
    ));
}
