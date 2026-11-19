use ats2_rs::generator::{
    ActiveRateSource, Channel, ExecutionGate, FrequencyError, FrequencyRuntimeState,
    GeneratorState, ImpedanceState, LevelError, LevelRequest, ModeStatus, OutputConfig,
    Single1KhzSineBuilder, Single1KhzSineError,
};

fn valid_frequency_state() -> FrequencyRuntimeState {
    FrequencyRuntimeState {
        rate_source: ActiveRateSource::resolve_static_selection(1, ModeStatus::Zero, None).unwrap(),
        waveform_mode: 0,
        mode_status: ModeStatus::Zero,
        system_check: ExecutionGate::Ready,
        rate_source_initialization: ExecutionGate::Ready,
        offset_c: Some(305_040),
    }
}

fn valid_level_state() -> GeneratorState {
    GeneratorState {
        master_enabled: true,
        channel_a_enabled: true,
        channel_b_enabled: false,
        channel_b_tracks_a: false,
        config: OutputConfig::Balanced,
        impedance: ImpedanceState::Unknown,
        range_rows: [Some(2), Some(2)],
    }
}

fn valid_level_request() -> LevelRequest {
    LevelRequest {
        channel: Channel::A,
        normalized_vrms: 1.0,
        tracked_a_vrms: None,
        f15: false,
    }
}

#[test]
fn explicit_inputs_build_the_conditional_one_khz_normal_sine_vector() {
    let vector = Single1KhzSineBuilder::new()
        .frequency_state(valid_frequency_state())
        .level_state(valid_level_state())
        .level_request(valid_level_request())
        .build()
        .unwrap();

    assert_eq!(vector.frequency_hz, 1_000.0);
    assert_eq!(vector.frequency_code, 305_040);
    assert_eq!(vector.frequency_write_data[0].internal_address, 0x30);
    assert_eq!(vector.frequency_write_data[0].data, 0);
    assert_eq!(vector.frequency_write_data[1].internal_address, 0x31);
    assert_eq!(vector.frequency_write_data[1].data, 610_080);
    assert_eq!(vector.level.channel, Channel::A);
    assert_eq!(vector.level.word, 0x2fff);
}

#[test]
fn missing_builder_inputs_fail_closed_without_defaults() {
    assert_eq!(
        Single1KhzSineBuilder::new().build(),
        Err(Single1KhzSineError::MissingFrequencyState)
    );
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(valid_frequency_state())
            .build(),
        Err(Single1KhzSineError::MissingLevelState)
    );
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(valid_frequency_state())
            .level_state(valid_level_state())
            .build(),
        Err(Single1KhzSineError::MissingLevelRequest)
    );
}

#[test]
fn unknown_or_invalid_prerequisites_and_nonfinite_level_fail_closed() {
    let mut unknown_gate = valid_frequency_state();
    unknown_gate.system_check = ExecutionGate::Unknown;
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(unknown_gate)
            .level_state(valid_level_state())
            .level_request(valid_level_request())
            .build(),
        Err(Single1KhzSineError::Frequency(
            FrequencyError::SystemCheckUnknown
        ))
    );

    let mut blocked_gate = valid_frequency_state();
    blocked_gate.system_check = ExecutionGate::Blocked;
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(blocked_gate)
            .level_state(valid_level_state())
            .level_request(valid_level_request())
            .build(),
        Err(Single1KhzSineError::Frequency(
            FrequencyError::SystemCheckFailed
        ))
    );

    let mut unknown_offset = valid_frequency_state();
    unknown_offset.offset_c = None;
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(unknown_offset)
            .level_state(valid_level_state())
            .level_request(valid_level_request())
            .build(),
        Err(Single1KhzSineError::Frequency(
            FrequencyError::UnknownOffset
        ))
    );

    let mut nonfinite_level = valid_level_request();
    nonfinite_level.normalized_vrms = f64::NAN;
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(valid_frequency_state())
            .level_state(valid_level_state())
            .level_request(nonfinite_level)
            .build(),
        Err(Single1KhzSineError::Level(LevelError::NonFiniteAmplitude))
    );
}

#[test]
fn mode_and_channel_are_never_inferred() {
    let mut non_sine_mode = valid_frequency_state();
    non_sine_mode.waveform_mode = 11;
    assert_eq!(
        Single1KhzSineBuilder::new()
            .frequency_state(non_sine_mode)
            .level_state(valid_level_state())
            .level_request(valid_level_request())
            .build(),
        Err(Single1KhzSineError::UnsupportedWaveformMode { waveform_mode: 11 })
    );

    let mut b_state = valid_level_state();
    b_state.channel_a_enabled = false;
    b_state.channel_b_enabled = true;
    let mut b_request = valid_level_request();
    b_request.channel = Channel::B;
    let vector = Single1KhzSineBuilder::new()
        .frequency_state(valid_frequency_state())
        .level_state(b_state)
        .level_request(b_request)
        .build()
        .unwrap();
    assert_eq!(vector.level.channel, Channel::B);
}

#[test]
fn builder_module_has_no_transport_or_sleep_dependency() {
    let source = include_str!("../src/generator/sine_vector.rs");
    for forbidden in [
        "use crate::protocol",
        "use crate::transport",
        "rusb::",
        "std::thread",
        ".sleep(",
    ] {
        assert!(
            !source.contains(forbidden),
            "offline vector module must not depend on {forbidden}"
        );
    }
}
