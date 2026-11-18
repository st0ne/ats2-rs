//! Explicit APIB record transfer orchestration over a mockable bulk transport.
//!
//! Constructing or opening a USB device never calls this module. A caller must
//! explicitly invoke [`execute_records`] to perform bulk transfers.

use thiserror::Error;

use crate::{
    protocol::framing::{FramingError, LogicalRecord, TransferPlan, frame_records},
    transport::usb::{BulkTransport, UsbTransportError},
};

/// Transfer result containing the exact plan and the actual bytes received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedTransfer {
    /// Deterministic framing plan used for this explicit call.
    pub plan: TransferPlan,
    /// Actual IN bytes returned by the transport, which can be shorter than the requested length.
    pub received: Vec<u8>,
}

/// Failure while framing records or moving them through a bulk transport.
#[derive(Debug, Error)]
pub enum TransferExecutionError {
    /// Could not build a safe transfer plan from the supplied logical records.
    #[error(transparent)]
    Framing(#[from] FramingError),
    /// The explicit bulk transport operation failed.
    #[error(transparent)]
    Transport(#[from] UsbTransportError),
}

/// Explicitly frame, write, and read a batch of logical APIB records.
///
/// The plan uses endpoint MPS values obtained from the transport's descriptors.
/// OUT writes must complete in full. IN reads retain their actual length, and
/// every logical response span is checked before this function succeeds.
pub fn execute_records<T: BulkTransport>(
    transport: &mut T,
    records: &[LogicalRecord<'_>],
) -> Result<ExecutedTransfer, TransferExecutionError> {
    let plan = frame_records(
        records,
        transport.out_max_packet_size(),
        transport.in_max_packet_size(),
    )?;
    transport.bulk_out(&plan.out_buffer)?;

    let received = if plan.expected_in_len == 0 {
        Vec::new()
    } else {
        transport.bulk_in(plan.expected_in_len)?
    };
    for span in &plan.responses {
        plan.response_bytes(span.operation_index, &received)?;
    }

    Ok(ExecutedTransfer { plan, received })
}

#[cfg(test)]
mod tests {
    use crate::{
        protocol::{
            framing::{FramingError, LogicalRecord},
            logical::ApibOperation,
        },
        transport::usb::{BulkTransport, UsbTransportError},
    };

    use super::{TransferExecutionError, execute_records};

    #[derive(Default)]
    struct MockTransport {
        out_mps: u16,
        in_mps: u16,
        written: Vec<u8>,
        input: Vec<u8>,
        read_requested: Option<usize>,
    }

    impl BulkTransport for MockTransport {
        fn out_max_packet_size(&self) -> u16 {
            self.out_mps
        }

        fn in_max_packet_size(&self) -> u16 {
            self.in_mps
        }

        fn bulk_out(&mut self, bytes: &[u8]) -> Result<(), UsbTransportError> {
            self.written = bytes.to_vec();
            Ok(())
        }

        fn bulk_in(&mut self, length: usize) -> Result<Vec<u8>, UsbTransportError> {
            self.read_requested = Some(length);
            Ok(self.input.clone())
        }
    }

    #[test]
    fn explicit_operation_execution_passes_the_framing_buffer_to_the_mock() {
        let operation = ApibOperation::ReadByte { argument: 0x12 };
        let logical = operation.encode();
        let records = [LogicalRecord {
            bytes: &logical,
            response_len: 1,
        }];
        let mut transport = MockTransport {
            out_mps: 64,
            in_mps: 64,
            input: vec![0x7a],
            ..MockTransport::default()
        };

        let result = execute_records(&mut transport, &records).unwrap();

        assert_eq!(transport.written, result.plan.out_buffer);
        assert_eq!(result.plan.out_buffer, [0xeb, 0x01, 0x12]);
        assert_eq!(transport.read_requested, Some(result.plan.expected_in_len));
        assert_eq!(result.received, [0x7a]);
        assert_eq!(
            result.plan.response_bytes(0, &result.received).unwrap(),
            [0x7a]
        );
    }

    #[test]
    fn a_short_read_is_accepted_only_when_all_logical_response_spans_arrived() {
        let operation = ApibOperation::ReadWord { argument: 0x02 };
        let logical = operation.encode();
        let records = [LogicalRecord {
            bytes: &logical,
            response_len: 2,
        }];
        let mut transport = MockTransport {
            out_mps: 64,
            in_mps: 64,
            input: vec![0x34],
            ..MockTransport::default()
        };

        let error = execute_records(&mut transport, &records).unwrap_err();
        assert!(matches!(
            error,
            TransferExecutionError::Framing(FramingError::InsufficientResponseData {
                operation_index: 0,
                required_end: 2,
                actual: 1
            })
        ));
    }

    #[test]
    fn write_only_operation_does_not_issue_a_bulk_in_read() {
        let operation = ApibOperation::WriteByte {
            argument: 0x1234_5678,
            value: 0x2a,
        };
        let logical = operation.encode();
        let records = [LogicalRecord {
            bytes: &logical,
            response_len: 0,
        }];
        let mut transport = MockTransport {
            out_mps: 64,
            in_mps: 64,
            ..MockTransport::default()
        };

        let result = execute_records(&mut transport, &records).unwrap();

        assert!(result.received.is_empty());
        assert_eq!(transport.read_requested, None);
        assert_eq!(transport.written, result.plan.out_buffer);
    }
}
