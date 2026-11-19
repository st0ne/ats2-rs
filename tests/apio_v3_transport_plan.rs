//! Pure host-plan coverage for the APIO-v3 tag-0x0C logical record.
//!
//! These assertions cover deterministic host-side framing only. They do not
//! assert USB transport, device wire acceptance, response acceptance, or any
//! device effect.

use ats2_rs::generator::{
    ActiveRateSource, Channel, DspWriteData, ExecutionGate, FrequencyRuntimeState, GeneratorState,
    ImpedanceState, LevelRequest, ModeStatus, OutputConfig, Single1KhzSineBuilder,
};
use ats2_rs::protocol::{
    apio_v3::{ApWriteToDspAddr, U24},
    apio_v3_transport_plan::{
        FrequencyPairPlanInputs, LevelPairPlanError, LevelPairPlanInputs,
        build_generator_level_records, build_single_1khz_sine_frequency_records,
        plan_ap_write_to_dsp_addr, plan_generator_level_pair, plan_single_1khz_sine_frequency_pair,
    },
    framing::{FramingError, ResponseSpan, TransferDirection},
};

fn record() -> ApWriteToDspAddr {
    ApWriteToDspAddr::new(
        U24::try_from(0x0012_3456).expect("fixture fits the checked raw U24 field"),
        0xa2,
        0xb3,
        0xc4,
        U24::try_from(0x0000_d5e6).expect("fixture fits the checked raw U24 field"),
        0xf6,
    )
}

fn sine_vector() -> ats2_rs::generator::Single1KhzSineVector {
    Single1KhzSineBuilder::new()
        .frequency_state(FrequencyRuntimeState {
            rate_source: ActiveRateSource::explicit(1, 55_000)
                .expect("fixture has an explicit nonzero sample rate"),
            waveform_mode: 0,
            mode_status: ModeStatus::Zero,
            system_check: ExecutionGate::Ready,
            rate_source_initialization: ExecutionGate::Ready,
            offset_c: Some(305_040),
        })
        .level_state(GeneratorState {
            master_enabled: true,
            channel_a_enabled: true,
            channel_b_enabled: false,
            channel_b_tracks_a: false,
            config: OutputConfig::Balanced,
            impedance: ImpedanceState::Unknown,
            range_rows: [Some(2), Some(2)],
        })
        .level_request(LevelRequest {
            channel: Channel::A,
            normalized_vrms: 1.0,
            tracked_a_vrms: None,
            f15: false,
        })
        .build()
        .expect("fixture builds the conditional software-only 1-kHz vector")
}

fn pair_inputs(base: u32, out_mps: u16, in_mps: u16) -> FrequencyPairPlanInputs {
    FrequencyPairPlanInputs {
        base: U24::try_from(base).expect("fixture base fits U24"),
        object: 0x40,
        token9: 0x10,
        token12: 0x26,
        retry_limit: 10,
        out_mps,
        in_mps,
    }
}

fn level_bytes(low_byte: i32, high_byte: i32) -> [DspWriteData; 2] {
    [
        DspWriteData {
            internal_address: 0x06,
            data: low_byte,
        },
        DspWriteData {
            internal_address: 0x07,
            data: high_byte,
        },
    ]
}

fn level_inputs(base: u32, out_mps: u16, in_mps: u16) -> LevelPairPlanInputs {
    LevelPairPlanInputs {
        base: U24::try_from(base).expect("fixture base fits U24"),
        object: 0x40,
        token9: 0x10,
        token13: 0x27,
        retry_limit: 10,
        out_mps,
        in_mps,
    }
}

#[test]
fn builds_the_two_generator_level_records_in_low_then_high_byte_order() {
    let records = build_generator_level_records(
        level_bytes(0x34, 0x12),
        U24::try_from(0x0012_3400).expect("fixture base fits U24"),
        0x40,
        0x10,
        0x27,
        10,
    )
    .expect("the explicit bytes and base are representable");

    assert_eq!(
        records,
        [
            ApWriteToDspAddr::new(
                U24::try_from(0x0012_3406).unwrap(),
                0x40,
                0x10,
                0x27,
                U24::try_from(0x34).unwrap(),
                10,
            ),
            ApWriteToDspAddr::new(
                U24::try_from(0x0012_3407).unwrap(),
                0x40,
                0x10,
                0x27,
                U24::try_from(0x12).unwrap(),
                10,
            ),
        ]
    );
}

#[test]
fn frames_the_complete_generator_level_pair_as_one_host_only_plan_at_mps_64_and_512() {
    for mps in [64_u16, 512] {
        let plan =
            plan_generator_level_pair(level_bytes(0x34, 0x12), level_inputs(0x0012_3400, mps, mps))
                .expect("the exact level-byte pair fits both explicit packet sizes");

        assert_eq!(
            plan.out_buffer,
            [
                0x0c, 0x40, 10, 0x12, 0x34, 0x06, 0x10, 0x00, 0x00, 0x34, 0x27, 0xec, 0x40, 10,
                0x12, 0x34, 0x07, 0x10, 0x00, 0x00, 0x12, 0x27,
            ]
        );
        assert_eq!(plan.expected_in_len, usize::from(mps));
        assert_eq!(
            plan.responses,
            [
                ResponseSpan {
                    operation_index: 0,
                    offset: 0,
                    length: 1,
                },
                ResponseSpan {
                    operation_index: 1,
                    offset: 1,
                    length: 1,
                },
            ]
        );
    }
}

#[test]
fn fails_closed_for_generator_level_pair_invalid_inputs_and_framing() {
    assert_eq!(
        plan_generator_level_pair(level_bytes(0x34, 0x12), level_inputs(0x00ff_fffa, 64, 64)),
        Err(LevelPairPlanError::AddressOverflow {
            base: 0x00ff_fffa,
            offset: 0x06,
        })
    );

    let mut wrong_order = level_bytes(0x34, 0x12);
    wrong_order[1].internal_address = 0x06;
    assert_eq!(
        build_generator_level_records(
            wrong_order,
            U24::try_from(0x0012_3400).unwrap(),
            0x40,
            0x10,
            0x27,
            10,
        ),
        Err(LevelPairPlanError::UnexpectedLevelOffset {
            operation_index: 1,
            expected: 0x07,
            actual: 0x06,
        })
    );
    assert_eq!(
        build_generator_level_records(
            level_bytes(0x100, 0x12),
            U24::try_from(0x0012_3400).unwrap(),
            0x40,
            0x10,
            0x27,
            10,
        ),
        Err(LevelPairPlanError::DataOutsideByte {
            operation_index: 0,
            data: 0x100,
        })
    );
    assert_eq!(
        build_generator_level_records(
            level_bytes(0x34, 0x12),
            U24::try_from(0x0012_3400).unwrap(),
            0x40,
            0x10,
            0x27,
            9,
        ),
        Err(LevelPairPlanError::UnsupportedRetryLimit { supplied: 9 })
    );
    assert_eq!(
        plan_generator_level_pair(level_bytes(0x34, 0x12), level_inputs(0x0012_3400, 63, 64)),
        Err(LevelPairPlanError::Framing(FramingError::InvalidMps {
            direction: TransferDirection::Out,
            value: 63,
        }))
    );
    assert_eq!(
        plan_generator_level_pair(level_bytes(0x34, 0x12), level_inputs(0x0012_3400, 64, 63)),
        Err(LevelPairPlanError::Framing(FramingError::InvalidMps {
            direction: TransferDirection::In,
            value: 63,
        }))
    );
    assert_eq!(
        plan_generator_level_pair(level_bytes(0x34, 0x12), level_inputs(0x0012_3400, 16, 64)),
        Err(LevelPairPlanError::Framing(
            FramingError::UndeterminedOutPadding {
                offset: 11,
                length: 5,
            }
        ))
    );
}

#[test]
fn builds_the_two_frequency_pair_records_in_the_established_order() {
    let records = build_single_1khz_sine_frequency_records(
        sine_vector(),
        U24::try_from(0x0012_3400).expect("fixture base fits U24"),
        0x40,
        0x10,
        0x26,
        10,
    )
    .expect("the explicit vector and base are representable");

    assert_eq!(
        records,
        [
            ApWriteToDspAddr::new(
                U24::try_from(0x0012_3430).unwrap(),
                0x40,
                0x10,
                0x26,
                U24::try_from(0).unwrap(),
                10,
            ),
            ApWriteToDspAddr::new(
                U24::try_from(0x0012_3431).unwrap(),
                0x40,
                0x10,
                0x26,
                U24::try_from(0x0009_4f20).unwrap(),
                10,
            ),
        ]
    );
}

#[test]
fn frames_the_complete_frequency_pair_as_one_host_only_plan_at_mps_64_and_512() {
    for mps in [64_u16, 512] {
        let plan =
            plan_single_1khz_sine_frequency_pair(sine_vector(), pair_inputs(0x0012_3400, mps, mps))
                .expect("the exact pair fits both explicit packet sizes");

        assert_eq!(
            plan.out_buffer,
            [
                0x0c, 0x40, 10, 0x12, 0x34, 0x30, 0x10, 0x00, 0x00, 0x00, 0x26, 0xec, 0x40, 10,
                0x12, 0x34, 0x31, 0x10, 0x09, 0x4f, 0x20, 0x26,
            ]
        );
        assert_eq!(plan.expected_in_len, usize::from(mps));
        assert_eq!(
            plan.responses,
            [
                ResponseSpan {
                    operation_index: 0,
                    offset: 0,
                    length: 1,
                },
                ResponseSpan {
                    operation_index: 1,
                    offset: 1,
                    length: 1,
                },
            ]
        );
    }
}

#[test]
fn fails_closed_for_frequency_pair_framing_and_base_overflow() {
    assert_eq!(
        plan_single_1khz_sine_frequency_pair(sine_vector(), pair_inputs(0x00ff_ffd0, 64, 64)),
        Err(
            ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::AddressOverflow {
                base: 0x00ff_ffd0,
                offset: 0x30,
            }
        )
    );
    let mut wrong_order = sine_vector();
    wrong_order.frequency_write_data[1].internal_address = 0x30;
    assert_eq!(
        build_single_1khz_sine_frequency_records(
            wrong_order,
            U24::try_from(0x0012_3400).unwrap(),
            0x40,
            0x10,
            0x26,
            10,
        ),
        Err(
            ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::UnexpectedFrequencyAddress {
                operation_index: 1,
                expected: 0x31,
                actual: 0x30,
            }
        )
    );

    let mut negative_data = sine_vector();
    negative_data.frequency_write_data[0].data = -1;
    assert_eq!(
        build_single_1khz_sine_frequency_records(
            negative_data,
            U24::try_from(0x0012_3400).unwrap(),
            0x40,
            0x10,
            0x26,
            10,
        ),
        Err(
            ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::DataOutsideU24 {
                operation_index: 0,
                data: -1,
            }
        )
    );
    assert_eq!(
        build_single_1khz_sine_frequency_records(
            sine_vector(),
            U24::try_from(0x0012_3400).unwrap(),
            0x40,
            0x10,
            0x26,
            9,
        ),
        Err(ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::UnsupportedRetryLimit {
            supplied: 9,
        })
    );
    assert_eq!(
        plan_single_1khz_sine_frequency_pair(sine_vector(), pair_inputs(0x0012_3400, 63, 64)),
        Err(
            ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::Framing(
                FramingError::InvalidMps {
                    direction: TransferDirection::Out,
                    value: 63,
                }
            )
        )
    );
    assert_eq!(
        plan_single_1khz_sine_frequency_pair(sine_vector(), pair_inputs(0x0012_3400, 64, 63)),
        Err(
            ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::Framing(
                FramingError::InvalidMps {
                    direction: TransferDirection::In,
                    value: 63,
                }
            )
        )
    );
    assert_eq!(
        plan_single_1khz_sine_frequency_pair(sine_vector(), pair_inputs(0x0012_3400, 16, 64)),
        Err(
            ats2_rs::protocol::apio_v3_transport_plan::FrequencyPairPlanError::Framing(
                FramingError::UndeterminedOutPadding {
                    offset: 11,
                    length: 5,
                }
            )
        )
    );
}

#[test]
fn creates_the_exact_single_record_host_plan_at_mps_64_and_512() {
    for mps in [64_u16, 512] {
        let plan = plan_ap_write_to_dsp_addr(record(), mps, mps)
            .expect("the exact tag-0x0C record fits in both explicit MPS values");

        assert_eq!(
            plan.out_buffer,
            [
                0xec, 0xa2, 0xf6, 0x12, 0x34, 0x56, 0xb3, 0x00, 0xd5, 0xe6, 0xc4
            ]
        );
        assert_eq!(plan.expected_in_len, usize::from(mps));
        assert_eq!(
            plan.responses,
            [ResponseSpan {
                operation_index: 0,
                offset: 0,
                length: 1,
            }]
        );
    }
}

#[test]
fn rejects_invalid_explicit_out_and_in_mps() {
    assert_eq!(
        plan_ap_write_to_dsp_addr(record(), 63, 64),
        Err(FramingError::InvalidMps {
            direction: TransferDirection::Out,
            value: 63,
        })
    );
    assert_eq!(
        plan_ap_write_to_dsp_addr(record(), 64, 63),
        Err(FramingError::InvalidMps {
            direction: TransferDirection::In,
            value: 63,
        })
    );
}

#[test]
fn fails_closed_when_the_explicit_out_mps_cannot_hold_the_known_record() {
    assert_eq!(
        plan_ap_write_to_dsp_addr(record(), 8, 64),
        Err(FramingError::RecordPayloadTooLarge {
            operation_index: 0,
            payload_len: 10,
            maximum: 7,
        })
    );
}
