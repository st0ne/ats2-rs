//! Dependency-free CSV/JSON export for multi-measurement sweep results.

use std::{
    collections::{BTreeMap, HashSet},
    fmt::Display,
};

use thiserror::Error;

use super::{MultiSweepOutcome, MultiSweepPlan, SettlingPolicy, SweepAxis, SweepStage};

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SweepExportError {
    #[error("outcome axis does not match the sweep plan")]
    AxisMismatch,
    #[error("outcome contains {observed} records for a {planned}-point plan")]
    TooManyRecords { observed: usize, planned: usize },
    #[error("aborted point index {index} is inconsistent with recorded points")]
    InvalidAbortIndex { index: usize },
    #[error("outcome axis value differs from the plan at point {index}")]
    AxisValueMismatch { index: usize },
    #[error("unexpected measurement identifier at point {point_index}: {id}")]
    UnexpectedMeasurementId { point_index: usize, id: String },
    #[error("duplicate measurement identifier at point {point_index}: {id}")]
    DuplicateMeasurementId { point_index: usize, id: String },
    #[error("measurement request differs from its planned request at point {point_index}: {id}")]
    MeasurementRequestMismatch { point_index: usize, id: String },
}

#[derive(Debug)]
struct ExportRow {
    point_index: usize,
    axis_value: f64,
    stimulus: String,
    measurement_id: String,
    function: String,
    unit: Option<String>,
    status: &'static str,
    value: Option<f64>,
    failure_stage: Option<SweepStage>,
    error: Option<String>,
}

/// Export planned and actual results as JSON, including explicit missing rows.
pub fn export_multi_sweep_json<S: Display, E: Display>(
    plan: &MultiSweepPlan<S>,
    outcome: &MultiSweepOutcome<f64, E>,
    metadata: &BTreeMap<String, String>,
) -> Result<String, SweepExportError> {
    validate(plan, outcome)?;
    let rows = collect_rows(plan, outcome);
    let config = configuration_json(plan);
    let metadata = metadata_json(metadata);
    let aborted_at = outcome
        .aborted_at
        .map_or_else(|| "null".to_owned(), |index| index.to_string());
    let records = rows
        .iter()
        .map(|row| {
            format!(
                "{{\"point_index\":{},\"axis_value\":{},\"stimulus\":{},\"measurement_id\":{},\"function\":{},\"unit\":{},\"status\":{},\"value\":{},\"failure_stage\":{},\"error\":{}}}",
                row.point_index,
                row.axis_value,
                json_string(&row.stimulus),
                json_string(&row.measurement_id),
                json_string(&row.function),
                row.unit.as_deref().map_or_else(|| "null".to_owned(), json_string),
                json_string(row.status),
                row.value.map_or_else(|| "null".to_owned(), |value| value.to_string()),
                row.failure_stage
                    .map_or_else(|| "null".to_owned(), |stage| json_string(&format!("{stage:?}"))),
                row.error.as_deref().map_or_else(|| "null".to_owned(), json_string),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "{{\"schema_version\":1,\"configuration\":{config},\"metadata\":{metadata},\"aborted_at\":{aborted_at},\"records\":[{records}]}}"
    ))
}

/// Export one CSV row per planned measurement per point.
///
/// Nested sweep configuration and metadata are encoded as JSON in their
/// respective columns. Failed and unexecuted measurements remain explicit rows.
pub fn export_multi_sweep_csv<S: Display, E: Display>(
    plan: &MultiSweepPlan<S>,
    outcome: &MultiSweepOutcome<f64, E>,
    metadata: &BTreeMap<String, String>,
) -> Result<String, SweepExportError> {
    validate(plan, outcome)?;
    let rows = collect_rows(plan, outcome);
    let config = configuration_json(plan);
    let metadata = metadata_json(metadata);
    let aborted_at = outcome
        .aborted_at
        .map_or_else(String::new, |index| index.to_string());
    let mut csv = String::from(
        "point_index,axis_value,axis_name,axis_unit,stimulus,measurement_id,function,unit,status,value,failure_stage,error,aborted_at,configuration_json,metadata_json\n",
    );
    for row in rows {
        let fields = [
            row.point_index.to_string(),
            row.axis_value.to_string(),
            plan.axis().name.clone(),
            plan.axis().unit.clone(),
            row.stimulus,
            row.measurement_id,
            row.function,
            row.unit.unwrap_or_default(),
            row.status.to_owned(),
            row.value
                .map_or_else(String::new, |value| value.to_string()),
            row.failure_stage
                .map_or_else(String::new, |stage| format!("{stage:?}")),
            row.error.unwrap_or_default(),
            aborted_at.clone(),
            config.clone(),
            metadata.clone(),
        ];
        csv.push_str(
            &fields
                .iter()
                .map(|field| csv_string(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    Ok(csv)
}

fn validate<S, E>(
    plan: &MultiSweepPlan<S>,
    outcome: &MultiSweepOutcome<f64, E>,
) -> Result<(), SweepExportError> {
    if &outcome.dataset.axis != plan.axis() {
        return Err(SweepExportError::AxisMismatch);
    }
    if outcome.dataset.records.len() > plan.points().len() {
        return Err(SweepExportError::TooManyRecords {
            observed: outcome.dataset.records.len(),
            planned: plan.points().len(),
        });
    }
    if let Some(index) = outcome.aborted_at {
        if index >= outcome.dataset.records.len() || index + 1 != outcome.dataset.records.len() {
            return Err(SweepExportError::InvalidAbortIndex { index });
        }
    }

    let expected_ids: HashSet<_> = plan
        .measurements()
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    for (point_index, record) in outcome.dataset.records.iter().enumerate() {
        if record.axis_value != plan.points()[point_index].axis_value {
            return Err(SweepExportError::AxisValueMismatch { index: point_index });
        }
        let mut seen = HashSet::new();
        for measurement in &record.measurements {
            if !expected_ids.contains(measurement.id.as_str()) {
                return Err(SweepExportError::UnexpectedMeasurementId {
                    point_index,
                    id: measurement.id.clone(),
                });
            }
            if !seen.insert(measurement.id.as_str()) {
                return Err(SweepExportError::DuplicateMeasurementId {
                    point_index,
                    id: measurement.id.clone(),
                });
            }
            let Some(expected) = plan
                .measurements()
                .iter()
                .find(|item| item.id == measurement.id)
            else {
                return Err(SweepExportError::UnexpectedMeasurementId {
                    point_index,
                    id: measurement.id.clone(),
                });
            };
            if measurement.request != expected.request {
                return Err(SweepExportError::MeasurementRequestMismatch {
                    point_index,
                    id: measurement.id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn collect_rows<S: Display, E: Display>(
    plan: &MultiSweepPlan<S>,
    outcome: &MultiSweepOutcome<f64, E>,
) -> Vec<ExportRow> {
    let mut rows = Vec::with_capacity(plan.points().len() * plan.measurements().len());
    for (point_index, point) in plan.points().iter().enumerate() {
        let actual = outcome.dataset.records.get(point_index);
        for expected in plan.measurements() {
            let found = actual.and_then(|record| {
                record
                    .measurements
                    .iter()
                    .find(|measurement| measurement.id == expected.id)
            });
            let (status, value, failure_stage, error) = match found {
                Some(record) => match &record.result {
                    Ok(value) if value.is_finite() => ("success", Some(*value), None, None),
                    Ok(value) => (
                        "invalid_value",
                        None,
                        None,
                        Some(format!("non-finite value: {value:?}")),
                    ),
                    Err(failure) => (
                        "failure",
                        None,
                        Some(failure.stage),
                        Some(failure.source.to_string()),
                    ),
                },
                None if actual.is_some() => ("missing", None, None, None),
                None => ("not_executed", None, None, None),
            };
            rows.push(ExportRow {
                point_index,
                axis_value: point.axis_value,
                stimulus: point.stimulus.to_string(),
                measurement_id: expected.id.clone(),
                function: expected.request.function.clone(),
                unit: expected.request.unit.clone(),
                status,
                value,
                failure_stage,
                error,
            });
        }
    }
    rows
}

fn configuration_json<S: Display>(plan: &MultiSweepPlan<S>) -> String {
    let points = plan
        .points()
        .iter()
        .map(|point| {
            format!(
                "{{\"axis_value\":{},\"stimulus\":{},\"settling\":{}}}",
                point.axis_value,
                json_string(&point.stimulus.to_string()),
                settling_json(point.settling)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let measurements = plan
        .measurements()
        .iter()
        .map(|measurement| {
            format!(
                "{{\"id\":{},\"function\":{},\"unit\":{}}}",
                json_string(&measurement.id),
                json_string(&measurement.request.function),
                measurement
                    .request
                    .unit
                    .as_deref()
                    .map_or_else(|| "null".to_owned(), json_string)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"axis\":{},\"failure_policy\":{},\"points\":[{points}],\"measurements\":[{measurements}]}}",
        axis_json(plan.axis()),
        json_string(&format!("{:?}", plan.failure_policy()))
    )
}

fn axis_json(axis: &SweepAxis) -> String {
    format!(
        "{{\"name\":{},\"unit\":{}}}",
        json_string(&axis.name),
        json_string(&axis.unit)
    )
}

fn settling_json(policy: SettlingPolicy) -> String {
    match policy {
        SettlingPolicy::None => json_string("none"),
        SettlingPolicy::BackendDefined => json_string("backend_defined"),
        SettlingPolicy::Delay(duration) => format!(
            "{{\"kind\":\"delay\",\"seconds\":{},\"nanoseconds\":{}}}",
            duration.as_secs(),
            duration.subsec_nanos()
        ),
    }
}

fn metadata_json(metadata: &BTreeMap<String, String>) -> String {
    let fields = metadata
        .iter()
        .map(|(key, value)| format!("{}:{}", json_string(key), json_string(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{fields}}}")
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character <= '\u{001f}' => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn csv_string(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
