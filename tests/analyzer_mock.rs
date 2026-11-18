use std::time::Duration;

use ats2_rs::analyzer::{
    config::{AnalogInputRange, Channel, InputRangeUnit},
    decoder::AnalyzerRawFormat,
    measurement::{Measurement, MeasurementRange, MeasurementStatus},
    mock::{MockAnalyzer, MockError, MockFailure, MockScenario},
};

fn measurement(channel: Channel, raw_word: u32, range_dbv: f64) -> Measurement {
    Measurement {
        channel,
        raw_word: Some(raw_word),
        raw_format: Some(AnalyzerRawFormat::SignedQ1_23),
        decoded_value: Some(0.5),
        scaled_value: None,
        status: MeasurementStatus::Numeric,
        range: Some(MeasurementRange::InputRequest(AnalogInputRange {
            value: range_dbv,
            unit: InputRangeUnit::Dbv,
        })),
    }
}

fn scenario(channel: Channel, pending_polls: usize, reading: Measurement) -> MockScenario {
    MockScenario {
        channel,
        pending_polls,
        delay: Duration::ZERO,
        completion: Ok(reading),
    }
}

#[test]
fn configured_pending_polls_precede_a_reproducible_reading() {
    let configured = scenario(Channel::A, 2, measurement(Channel::A, 0x0010_0000, 0.0));
    let mut mock = MockAnalyzer::new([configured]);

    assert_eq!(mock.trigger(Channel::A), Ok(()));
    assert_eq!(mock.poll_ready(), Ok(false));
    assert_eq!(mock.poll_ready(), Ok(false));
    assert_eq!(mock.poll_ready(), Ok(true));
    assert_eq!(mock.read(), Ok(measurement(Channel::A, 0x0010_0000, 0.0)));
}

#[test]
fn sequence_replays_raw_values_and_requested_range_changes() {
    let scenarios = [
        scenario(Channel::A, 0, measurement(Channel::A, 0x0000_0100, 0.0)),
        scenario(Channel::A, 1, measurement(Channel::A, 0x0000_0200, -6.0)),
    ];
    let mut first = MockAnalyzer::new(scenarios);
    let mut second = MockAnalyzer::new(scenarios);

    for mock in [&mut first, &mut second] {
        let mut readings = Vec::new();
        for _ in 0..2 {
            mock.trigger(Channel::A).unwrap();
            while !mock.poll_ready().unwrap() {}
            readings.push(mock.read().unwrap());
        }
        assert_eq!(readings[0].raw_word, Some(0x0000_0100));
        assert_eq!(readings[1].raw_word, Some(0x0000_0200));
        assert_ne!(readings[0].range, readings[1].range);
    }
    assert_eq!(first, second);
}

#[test]
fn scenario_failures_are_returned_as_simulated_errors() {
    let failing = MockScenario {
        channel: Channel::B,
        pending_polls: 0,
        delay: Duration::ZERO,
        completion: Err(MockFailure::Timeout),
    };
    let mut mock = MockAnalyzer::new([failing]);

    assert_eq!(mock.trigger(Channel::B), Ok(()));
    assert_eq!(mock.poll_ready(), Ok(true));
    assert_eq!(
        mock.read(),
        Err(MockError::ScenarioFailure(MockFailure::Timeout))
    );
}

#[test]
fn wrong_channel_does_not_consume_the_next_scenario() {
    let mut mock = MockAnalyzer::new([scenario(
        Channel::A,
        0,
        measurement(Channel::A, 0x0000_0001, 0.0),
    )]);

    assert_eq!(mock.trigger(Channel::B), Err(MockError::UnexpectedChannel));
    assert_eq!(mock.trigger(Channel::A), Ok(()));
    assert_eq!(mock.poll_ready(), Ok(true));
}

#[test]
fn configured_virtual_duration_advances_only_when_explicitly_requested() {
    let mut configured = scenario(Channel::A, 0, measurement(Channel::A, 0x0000_0001, 0.0));
    configured.delay = Duration::from_millis(5);
    let mut mock = MockAnalyzer::new([configured]);

    mock.trigger(Channel::A).unwrap();
    assert_eq!(mock.poll_ready(), Ok(false));
    mock.advance_time(Duration::from_millis(4)).unwrap();
    assert_eq!(mock.poll_ready(), Ok(false));
    mock.advance_time(Duration::from_millis(1)).unwrap();
    assert_eq!(mock.poll_ready(), Ok(true));
}

#[test]
fn polling_before_trigger_fails_closed() {
    let mut mock = MockAnalyzer::new([scenario(
        Channel::A,
        0,
        measurement(Channel::A, 0x0000_0001, 0.0),
    )]);

    assert_eq!(mock.poll_ready(), Err(MockError::NotTriggered));
}
