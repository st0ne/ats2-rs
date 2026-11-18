use std::cell::Cell;

use ats2_rs::sweep::{
    FailurePolicy, IdentifiedMeasurementRequest, MeasurementRequest, MultiSweepPlan,
    MultiSweepPoint, SettlingPolicy, SweepAxis, SweepBackend, SweepProgress,
    run_multi_sweep_controlled,
};

struct RecordingBackend {
    stimuli: Vec<String>,
}

impl SweepBackend<String> for RecordingBackend {
    type Reading = f64;
    type Error = std::convert::Infallible;

    fn apply_stimulus(&mut self, stimulus: &String) -> Result<(), Self::Error> {
        self.stimuli.push(stimulus.clone());
        Ok(())
    }

    fn settle(&mut self, _policy: SettlingPolicy) -> Result<(), Self::Error> {
        Ok(())
    }

    fn measure(&mut self, _request: &MeasurementRequest) -> Result<Self::Reading, Self::Error> {
        Ok(1.0)
    }
}

#[test]
fn controlled_multi_sweep_reports_progress_and_cancels_between_points() {
    let plan = MultiSweepPlan::new(
        SweepAxis {
            name: "level".into(),
            unit: "V RMS".into(),
        },
        ["first", "second", "third"]
            .into_iter()
            .enumerate()
            .map(|(index, stimulus)| MultiSweepPoint {
                axis_value: index as f64 + 1.0,
                stimulus: stimulus.to_owned(),
                settling: SettlingPolicy::None,
            })
            .collect(),
        vec![IdentifiedMeasurementRequest {
            id: "rms".into(),
            request: MeasurementRequest {
                function: "simulated.rms".into(),
                unit: Some("V RMS".into()),
            },
        }],
        FailurePolicy::Abort,
    )
    .unwrap();
    let cancel = Cell::new(false);
    let mut progress = Vec::<SweepProgress>::new();
    let mut backend = RecordingBackend {
        stimuli: Vec::new(),
    };

    let controlled = run_multi_sweep_controlled(
        &plan,
        &mut backend,
        || cancel.get(),
        |update| {
            progress.push(update);
            if update.completed_points == 1 {
                cancel.set(true);
            }
        },
    );

    assert_eq!(controlled.cancelled_at, Some(1));
    assert_eq!(controlled.outcome.dataset.records.len(), 1);
    assert_eq!(backend.stimuli, ["first"]);
    assert_eq!(
        progress,
        [SweepProgress {
            completed_points: 1,
            total_points: 3,
        }]
    );
}
