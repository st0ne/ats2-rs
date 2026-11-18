use std::{collections::BTreeMap, sync::Arc};

use ats2_rs::sweep::{
    FailurePolicy, IdentifiedMeasurementRequest, MeasurementRequest, MultiSweepDataset,
    MultiSweepMeasurementRecord, MultiSweepOutcome, MultiSweepPlan, MultiSweepPoint,
    MultiSweepRecord, SettlingPolicy, SweepAxis, SweepFailure, SweepStage,
    export::{export_multi_sweep_csv, export_multi_sweep_json},
};

fn axis() -> SweepAxis {
    SweepAxis {
        name: "level".into(),
        unit: "V RMS".into(),
    }
}

fn request(id: &str, function: &str, unit: &str) -> IdentifiedMeasurementRequest {
    IdentifiedMeasurementRequest {
        id: id.into(),
        request: MeasurementRequest {
            function: function.into(),
            unit: Some(unit.into()),
        },
    }
}

fn plan() -> MultiSweepPlan<String> {
    MultiSweepPlan::new(
        axis(),
        vec![
            MultiSweepPoint {
                axis_value: 1.0,
                stimulus: "tone, \"one\"".into(),
                settling: SettlingPolicy::Delay(std::time::Duration::from_millis(5)),
            },
            MultiSweepPoint {
                axis_value: 2.0,
                stimulus: "tone-two".into(),
                settling: SettlingPolicy::None,
            },
        ],
        vec![
            request("voltage.rms", "simulated.rms", "V RMS"),
            request("distortion.thd+n", "simulated.thd+n", "%"),
            request("distortion.thd", "simulated.thd", "%"),
        ],
        FailurePolicy::Abort,
    )
    .unwrap()
}

fn incomplete_outcome() -> MultiSweepOutcome<f64, String> {
    let requests = plan().measurements().to_vec();
    MultiSweepOutcome {
        dataset: MultiSweepDataset {
            axis: axis(),
            records: vec![MultiSweepRecord {
                axis_value: 1.0,
                measurements: vec![
                    MultiSweepMeasurementRecord {
                        id: requests[0].id.clone(),
                        request: requests[0].request.clone(),
                        result: Ok(1.25),
                    },
                    MultiSweepMeasurementRecord {
                        id: requests[1].id.clone(),
                        request: requests[1].request.clone(),
                        result: Err(SweepFailure {
                            stage: SweepStage::Measurement,
                            source: Arc::new("bad, \"sample\"".to_owned()),
                        }),
                    },
                ],
            }],
        },
        aborted_at: Some(0),
    }
}

#[test]
fn json_export_preserves_configuration_metadata_units_and_partial_statuses() {
    let configuration = plan();
    let mut metadata = BTreeMap::new();
    metadata.insert("seed".into(), "fixed \"fixture\"\nline".into());
    let json = export_multi_sweep_json(&configuration, &incomplete_outcome(), &metadata).unwrap();

    assert!(json.contains("\"axis\":{\"name\":\"level\",\"unit\":\"V RMS\"}"));
    assert!(json.contains("\"measurement_id\":\"voltage.rms\""));
    assert!(json.contains("\"unit\":\"V RMS\""));
    assert!(json.contains("\"status\":\"success\",\"value\":1.25"));
    assert!(json.contains("\"status\":\"failure\""));
    assert!(json.contains("\"failure_stage\":\"Measurement\""));
    assert!(json.contains("bad, \\\"sample\\\""));
    assert!(json.contains("\"status\":\"missing\""));
    assert!(json.contains("\"status\":\"not_executed\""));
    assert!(json.contains("\"metadata\":{\"seed\":\"fixed \\\"fixture\\\"\\nline\"}"));
}

#[test]
fn csv_export_has_one_row_per_expected_measurement_and_escapes_fields() {
    let configuration = plan();
    let metadata = BTreeMap::from([("source".to_owned(), "mock,only".to_owned())]);
    let csv = export_multi_sweep_csv(&configuration, &incomplete_outcome(), &metadata).unwrap();
    let lines: Vec<_> = csv.lines().collect();

    assert_eq!(lines.len(), 7);
    assert!(lines[0].contains("measurement_id"));
    assert!(csv.contains("\"tone, \"\"one\"\"\""));
    assert!(csv.contains("\"voltage.rms\""));
    assert!(csv.contains("\"success\""));
    assert!(csv.contains("\"failure\""));
    assert!(csv.contains("\"missing\""));
    assert!(csv.contains("\"not_executed\""));
    assert!(csv.contains("\"V RMS\""));
    assert!(csv.contains("\"mock,only\""));
}
