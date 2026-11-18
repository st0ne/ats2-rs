use std::{
    env,
    error::Error,
    io::{self, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ats2_rs::{
    protocol::{
        framing::{FramingError, LogicalRecord, TransferPlan, frame_records},
        logical::{ApibOperation, ApibResponse},
    },
    transport::usb::{BulkTransport, open_supported_adapter_with_timeout},
};

const TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.as_slice() != ["--send-once"] {
        return Err("usage: cargo run --example apib_any_ready_once -- --send-once".into());
    }

    let start = Instant::now();
    log_event(
        start,
        "operation=ap_any_ready stage=open-device target=16d0:0af0 timeout_ms=2000 action=one-attempt no-config-change no-kernel-driver-detach",
    );
    let opened = open_supported_adapter_with_timeout(TIMEOUT)?;
    let endpoints = opened.endpoints();
    let plan = build_plan(
        endpoints.out.max_packet_size,
        endpoints.input.max_packet_size,
    )?;
    log_event(
        start,
        &format!(
            "operation=ap_any_ready stage=preflight interface=0 config=1 alt=0 out_ep=0x{:02x} tx_len={} tx_hex={} in_ep=0x{:02x} in_requested={} response_span=1 timeout_ms=2000 retries=0 effect=host_status_query_no_setpoint_payload_device_side_effects_unverified",
            endpoints.out.address,
            plan.out_buffer.len(),
            hex(&plan.out_buffer),
            endpoints.input.address,
            plan.expected_in_len,
        ),
    );
    io::stderr().flush()?;

    log_event(
        start,
        "operation=ap_any_ready stage=claim-interface interface=0 expected_active_config=1 expected_alt=0 no-detach no-setting-change",
    );
    let mut claimed = opened.claim_interface()?;

    log_event(
        start,
        &format!(
            "operation=ap_any_ready direction=OUT endpoint=0x{:02x} requested={} bytes_hex={} timeout_ms=2000 attempt=1",
            endpoints.out.address,
            plan.out_buffer.len(),
            hex(&plan.out_buffer),
        ),
    );
    io::stderr().flush()?;
    if let Err(error) = claimed.bulk_out(&plan.out_buffer) {
        log_event(
            start,
            &format!("operation=ap_any_ready direction=OUT result=error error={error}"),
        );
        return Err(error.into());
    }
    log_event(
        start,
        &format!(
            "operation=ap_any_ready direction=OUT endpoint=0x{:02x} actual={} result=ok",
            endpoints.out.address,
            plan.out_buffer.len(),
        ),
    );

    log_event(
        start,
        &format!(
            "operation=ap_any_ready direction=IN endpoint=0x{:02x} requested={} timeout_ms=2000 attempt=1",
            endpoints.input.address, plan.expected_in_len,
        ),
    );
    io::stderr().flush()?;
    let received = match claimed.bulk_in(plan.expected_in_len) {
        Ok(bytes) => bytes,
        Err(error) => {
            log_event(
                start,
                &format!("operation=ap_any_ready direction=IN result=error error={error}"),
            );
            return Err(error.into());
        }
    };
    log_event(
        start,
        &format!(
            "operation=ap_any_ready direction=IN endpoint=0x{:02x} actual={} bytes_hex={} result=ok",
            endpoints.input.address,
            received.len(),
            hex(&received),
        ),
    );

    let response_bytes = plan.response_bytes(0, &received)?;
    match ApibOperation::AnyReady.decode_response(response_bytes)? {
        ApibResponse::Ready(value) => log_event(
            start,
            &format!(
                "operation=ap_any_ready decoded_dll_expression={value} raw_hex={}",
                hex(response_bytes)
            ),
        ),
        other => return Err(format!("unexpected response variant: {other:?}").into()),
    }
    claimed.release()?;
    log_event(
        start,
        "operation=ap_any_ready stage=release-interface result=ok",
    );
    Ok(())
}

fn build_plan(out_mps: u16, in_mps: u16) -> Result<TransferPlan, FramingError> {
    let operation = ApibOperation::AnyReady;
    let bytes = operation.encode();
    frame_records(
        &[LogicalRecord {
            bytes: &bytes,
            response_len: 1,
        }],
        out_mps,
        in_mps,
    )
}

fn log_event(start: Instant, message: &str) {
    let utc_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    eprintln!(
        "utc_unix_ms={utc_ms} monotonic_elapsed_us={} {message}",
        start.elapsed().as_micros()
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::build_plan;

    #[test]
    fn any_ready_plan_is_one_status_request_with_one_response_byte() {
        let plan = build_plan(512, 512).unwrap();

        assert_eq!(plan.out_buffer, [0xe0, 0x01]);
        assert_eq!(plan.expected_in_len, 512);
        assert_eq!(plan.responses.len(), 1);
        assert_eq!(plan.responses[0].offset, 0);
        assert_eq!(plan.responses[0].length, 1);
    }
}
