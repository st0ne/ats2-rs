use std::sync::Arc;

use ats2_rs::sweep::{
    SweepFailure, SweepStage,
    demo::{
        DemoConfig, DemoError, DemoXAxis, POWER_ASSUMPTIONS, SIMULATION_NOTICE, simulate_demo,
        simulate_demo_controlled,
    },
};

#[test]
fn demo_is_explicitly_simulated_and_reproducible() {
    let first = simulate_demo(None, true).unwrap();
    let second = simulate_demo(None, true).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.provenance, SIMULATION_NOTICE);
    assert_eq!(first.points.len(), 3);
    assert_eq!(first.simulated_stimuli_vrms, [0.1, 0.5, 1.0]);
    assert!((0..first.points.len()).all(|index| first.measured_power_w(index).is_none()));
    assert!(first.points.iter().all(|point| point.thd_percent.is_some()));
    assert_eq!(first.sweep.dataset.records.len(), 3);
    assert_eq!(
        first.sweep.dataset.records[0].measurements[0].id,
        "voltage.rms"
    );
    assert_eq!(
        first.sweep.dataset.records[0].measurements[0]
            .request
            .unit
            .as_deref(),
        Some("V RMS")
    );
    assert_eq!(
        first.sweep.dataset.records[0].measurements[1].id,
        "distortion.thd+n"
    );
}

#[test]
fn power_requires_explicit_load_and_declares_resistive_assumption() {
    let without_load = simulate_demo(None, false).unwrap();
    let with_load = simulate_demo(Some(8.0), false).unwrap();

    assert!(
        (0..without_load.points.len()).all(|index| without_load.measured_power_w(index).is_none())
    );
    assert_eq!(without_load.power_assumptions, None);
    assert!((0..with_load.points.len()).all(|index| {
        with_load.measured_power_w(index)
            == Some(with_load.points[index].measured_vrms.powi(2) / 8.0)
    }));
    assert_eq!(with_load.power_assumptions, Some(POWER_ASSUMPTIONS));
    assert!(
        with_load
            .points
            .iter()
            .all(|point| point.thd_percent.is_none())
    );
    assert_eq!(with_load.provenance, SIMULATION_NOTICE);
}

#[test]
fn exports_include_derived_power_and_calculation_metadata_only_when_requested() {
    let config = DemoConfig {
        display_axis: DemoXAxis::MeasuredPowerW,
        load_resistance_ohms: Some(8.0),
        ..DemoConfig::default()
    };
    let with_power = simulate_demo_controlled(&config, || false, |_| {}).unwrap();
    let csv = with_power.export_csv(&Default::default()).unwrap();
    let json = with_power.export_json(&Default::default()).unwrap();

    assert!(csv.contains("power.w"));
    assert!(csv.contains("\"W\",\"success\",\""));
    assert!(csv.contains("power_load_resistance_ohms"));
    assert!(csv.contains("power_calculation_basis"));
    assert!(json.contains("\"measurement_id\":\"power.w\""));
    assert!(json.contains("\"unit\":\"W\""));
    assert!(json.contains("\"power_load_resistance_ohms\":\"8\""));
    assert!(json.contains("\"power_calculation_basis\":"));
    assert!(csv.contains(POWER_ASSUMPTIONS));
    assert!(json.contains(POWER_ASSUMPTIONS));
    for index in 0..with_power.demonstration.points.len() {
        let power = with_power.demonstration.measured_power_w(index).unwrap();
        assert!(csv.contains(&format!(
            "\"power.w\",\"simulated.output_power\",\"W\",\"success\",\"{power}\""
        )));
        assert!(json.contains(&format!(
            "\"measurement_id\":\"power.w\",\"function\":\"simulated.output_power\",\"unit\":\"W\",\"status\":\"success\",\"value\":{power}"
        )));
    }

    let without_power = simulate_demo_controlled(&DemoConfig::default(), || false, |_| {}).unwrap();
    let csv = without_power.export_csv(&Default::default()).unwrap();
    let json = without_power.export_json(&Default::default()).unwrap();
    assert!(!csv.contains("power.w"));
    assert!(!json.contains("power.w"));
    assert!(!csv.contains("power_load_resistance_ohms"));
    assert!(!json.contains("power_load_resistance_ohms"));
}

#[test]
fn power_measurement_is_not_zero_when_no_rms_power_calculation_exists() {
    let result = simulate_demo(None, true).unwrap();

    assert!((0..result.points.len()).all(|index| result.measured_power_w(index).is_none()));
    assert!(result.sweep.dataset.records.iter().all(|record| {
        record
            .measurements
            .iter()
            .all(|measurement| measurement.id != "power.w")
    }));
}

#[test]
fn failed_rms_and_derived_power_export_as_failures_not_zero_watts() {
    let config = DemoConfig {
        load_resistance_ohms: Some(8.0),
        ..DemoConfig::default()
    };
    let mut run = simulate_demo_controlled(&config, || false, |_| {}).unwrap();
    run.demonstration.sweep.dataset.records.truncate(1);
    run.demonstration.sweep.aborted_at = Some(0);
    run.demonstration.points.clear();
    let measurements = &mut run.demonstration.sweep.dataset.records[0].measurements;
    let rms = measurements
        .iter_mut()
        .find(|measurement| measurement.id == "voltage.rms")
        .unwrap();
    rms.result = Err(SweepFailure {
        stage: SweepStage::Measurement,
        source: Arc::new(DemoError::InvalidRms),
    });
    let power = measurements
        .iter_mut()
        .find(|measurement| measurement.id == "power.w")
        .unwrap();
    power.result = Err(SweepFailure {
        stage: SweepStage::Measurement,
        source: Arc::new(DemoError::MissingRmsForPower),
    });
    assert_eq!(run.demonstration.measured_power_w(0), None);

    let csv = run.export_csv(&Default::default()).unwrap();
    let json = run.export_json(&Default::default()).unwrap();

    assert!(csv.contains("\"power.w\",\"simulated.output_power\",\"W\",\"failure\",\"\""));
    assert!(json.contains("\"measurement_id\":\"power.w\""));
    assert!(json.contains("\"status\":\"failure\",\"value\":null"));
    assert!(csv.contains("power_load_resistance_ohms"));
    assert!(json.contains("power_calculation_basis"));
    assert!(!json.contains("\"measurement_id\":\"power.w\",\"function\":\"simulated.output_power\",\"unit\":\"W\",\"status\":\"success\",\"value\":0"));
}

#[test]
fn demo_rejects_nonpositive_or_nonfinite_load_resistance() {
    for resistance in [0.0, -8.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            simulate_demo(Some(resistance), false),
            Err(DemoError::InvalidLoadResistance)
        );
    }
}
