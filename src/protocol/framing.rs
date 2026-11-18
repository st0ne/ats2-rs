//! Hardware-independent APIB-to-pipe framing derived from the verified serializer
//! and response loop. This module accepts already encoded logical records; it
//! does not perform USB I/O or choose endpoint addresses.
//!
//! The serializer ORs marker bits into each record's first byte. `0x80` is set
//! when a record payload reaches an OUT packet boundary, when the serializer
//! skips an OUT tail to fit the next record (on the preceding tag), and on the
//! final record. `0x40` is set when a response ends at an IN packet boundary;
//! if a response cannot fit in the current IN packet remainder, that remainder
//! is skipped and `0x40` is ORed into the preceding record's tag. When the final
//! aggregate IN cursor (including skipped packet tails) is not packet-aligned,
//! `0x60` (`0x20 | 0x40`) is ORed into the last response-bearing tag and the
//! requested IN length is
//! rounded up to the next MPS multiple. These are observed set conditions, not
//! interpretations of device-side meaning.
//!
//! The DLL uses `MPS - 1` bit masks, so this implementation accepts only
//! power-of-two MPS values of at least four bytes (the largest supported
//! logical payload is three bytes). It also enforces the observed 8 KiB pipe
//! buffer capacity. The DLL does not write bytes in an OUT packet tail skipped
//! to fit the next record; this module therefore returns an error instead of
//! inventing padding for such a frame. Operation-specific segmentation for
//! larger records is not implemented.

use thiserror::Error;

/// Maximum size of either statically allocated pipe buffer in the analyzed DLL.
///
/// This is a Rust safety bound, not a claim that every DLL path validates its
/// final length consistently.
pub const MAX_TRANSFER_BUFFER_LEN: usize = 0x2000;

const OUT_MARKER_BIT: u8 = 0x80;
const IN_MARKER_BIT: u8 = 0x40;
const IN_PADDING_MARKER_BITS: u8 = 0x60;

/// Direction whose packet size or buffer length was invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    /// Host-to-device pipe.
    Out,
    /// Device-to-host pipe.
    In,
}

/// One already encoded logical APIB record and its expected logical response
/// length. `bytes[0]` is the operation tag; later bytes are its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalRecord<'a> {
    /// Exact logical tag-plus-payload bytes produced by the logical encoder.
    pub bytes: &'a [u8],
    /// Number of response bytes the operation-specific decoder consumes.
    pub response_len: usize,
}

/// Byte range assigned to one operation's logical response in the IN buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseSpan {
    /// Index of the corresponding record passed to [`frame_records`].
    pub operation_index: usize,
    /// Offset in the received IN buffer, after any packet-tail skip.
    pub offset: usize,
    /// Logical response length; transfer padding is not part of this span.
    pub length: usize,
}

/// A hardware-independent transfer plan; no USB transfer has been performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPlan {
    /// Complete deterministic buffer to pass as the WinUSB/Bulk OUT payload.
    pub out_buffer: Vec<u8>,
    /// Requested IN buffer length, including packet-tail skips and final padding.
    pub expected_in_len: usize,
    /// Response ranges for records with nonzero logical response lengths.
    pub responses: Vec<ResponseSpan>,
}

impl TransferPlan {
    /// Returns a checked logical response slice for an operation.
    ///
    /// The actual read may be shorter than `expected_in_len` (for example, a
    /// short USB packet); only the bytes needed by this operation's decoder are
    /// required here. The caller should pass the returned slice to the logical
    /// decoder, which performs its own exact-length check.
    pub fn response_bytes<'a>(
        &self,
        operation_index: usize,
        received: &'a [u8],
    ) -> Result<&'a [u8], FramingError> {
        let Some(span) = self
            .responses
            .iter()
            .find(|span| span.operation_index == operation_index)
        else {
            return Err(FramingError::NoResponseForOperation { operation_index });
        };
        let Some(required_end) = span.offset.checked_add(span.length) else {
            return Err(FramingError::ResponseOffsetOverflow { operation_index });
        };
        if required_end > received.len() {
            return Err(FramingError::InsufficientResponseData {
                operation_index,
                required_end,
                actual: received.len(),
            });
        }
        Ok(&received[span.offset..required_end])
    }
}

/// Failure while constructing or consuming a verified framing plan.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FramingError {
    /// The DLL's packet arithmetic requires a power-of-two MPS of at least 4.
    #[error("invalid {direction:?} MPS {value}; expected a power of two >= 4")]
    InvalidMps {
        /// Pipe direction for the invalid value.
        direction: TransferDirection,
        /// Supplied maximum packet size.
        value: u16,
    },
    /// No serializer invocation is needed for an empty operation queue.
    #[error("cannot frame an empty operation queue")]
    EmptyOperationQueue,
    /// A record must contain at least its tag byte.
    #[error("logical record {operation_index} is empty")]
    EmptyLogicalRecord {
        /// Index of the invalid record.
        operation_index: usize,
    },
    /// The record's payload cannot fit in any single packet and no generic
    /// segmentation rule is established here.
    #[error(
        "record {operation_index} payload ({payload_len} bytes) exceeds the per-packet limit ({maximum})"
    )]
    RecordPayloadTooLarge {
        /// Index of the record that cannot be represented.
        operation_index: usize,
        /// Payload length, excluding the tag byte.
        payload_len: usize,
        /// Maximum payload length after reserving one tag byte.
        maximum: usize,
    },
    /// The serializer does not assign these OUT bytes while advancing to the
    /// next packet; their values cannot safely be fabricated as zero padding.
    #[error(
        "OUT packet-tail bytes at offset {offset} (length {length}) are not written by the serializer"
    )]
    UndeterminedOutPadding {
        /// First unwritten byte.
        offset: usize,
        /// Number of unwritten bytes before the next packet boundary.
        length: usize,
    },
    /// One operation's response is larger than one IN packet; operation-level
    /// chunking is not represented by this record abstraction.
    #[error(
        "response for operation {operation_index} ({length} bytes) exceeds one IN packet ({mps})"
    )]
    ResponseTooLargeForPacket {
        /// Index of the operation with the unsupported response size.
        operation_index: usize,
        /// Response length.
        length: usize,
        /// IN packet size.
        mps: usize,
    },
    /// A length exceeds the fixed pipe buffer capacity.
    #[error("{direction:?} buffer length {attempted} exceeds the {maximum}-byte capacity")]
    BufferTooLarge {
        /// Pipe direction whose buffer would overflow.
        direction: TransferDirection,
        /// Calculated length.
        attempted: usize,
        /// Fixed buffer capacity.
        maximum: usize,
    },
    /// Internal control flow would require marking a preceding record that is
    /// absent. This is not expected for supported operations and valid MPS.
    #[error("IN packet boundary for operation {operation_index} has no preceding record")]
    MissingPreviousRecordForInBoundary {
        /// Index of the operation whose response crosses the first boundary.
        operation_index: usize,
    },
    /// The selected operation has no logical response to slice.
    #[error("operation {operation_index} has no response span")]
    NoResponseForOperation {
        /// Index of the operation without a response.
        operation_index: usize,
    },
    /// The response range exceeds the bytes actually received.
    #[error(
        "response for operation {operation_index} needs bytes through offset {required_end}, but only {actual} byte(s) were received"
    )]
    InsufficientResponseData {
        /// Index of the operation whose response is incomplete.
        operation_index: usize,
        /// Exclusive end offset required by the response span.
        required_end: usize,
        /// Actual received buffer length.
        actual: usize,
    },
    /// A response offset was corrupted or overflowed.
    #[error("response offset overflow for operation {operation_index}")]
    ResponseOffsetOverflow {
        /// Index of the affected operation.
        operation_index: usize,
    },
    /// A checked length calculation overflowed `usize`.
    #[error("{direction:?} length arithmetic overflowed")]
    LengthOverflow {
        /// Pipe direction whose arithmetic overflowed.
        direction: TransferDirection,
    },
}

/// Builds the known APIB record framing for explicit IN/OUT MPS values.
///
/// Each record is kept intact; records are packed in order. The function
/// applies only the marker and response-offset rules visible in the analyzed
/// serializer/decoder. It rejects packet-tail OUT gaps because their bytes are
/// not written on the analyzed path, and rejects per-record segmentation that
/// would require operation-specific behavior.
pub fn frame_records(
    records: &[LogicalRecord<'_>],
    out_mps: u16,
    in_mps: u16,
) -> Result<TransferPlan, FramingError> {
    let out_mps = validate_mps(TransferDirection::Out, out_mps)?;
    let in_mps = validate_mps(TransferDirection::In, in_mps)?;
    if records.is_empty() {
        return Err(FramingError::EmptyOperationQueue);
    }

    let mut out_buffer = Vec::new();
    let mut response_spans = Vec::new();
    let mut in_cursor = 0_usize;
    let mut last_tag_offset = None;
    let mut last_response_tag_offset = None;

    for (operation_index, record) in records.iter().enumerate() {
        if record.bytes.is_empty() {
            return Err(FramingError::EmptyLogicalRecord { operation_index });
        }
        let payload_len = record.bytes.len() - 1;
        let maximum_payload = out_mps - 1;
        if payload_len > maximum_payload {
            return Err(FramingError::RecordPayloadTooLarge {
                operation_index,
                payload_len,
                maximum: maximum_payload,
            });
        }
        if record.response_len > in_mps {
            return Err(FramingError::ResponseTooLargeForPacket {
                operation_index,
                length: record.response_len,
                mps: in_mps,
            });
        }

        let out_cursor = out_buffer.len();
        let out_tail = out_mps - (out_cursor & (out_mps - 1));
        let out_payload_capacity = out_tail - 1;
        if payload_len > out_payload_capacity {
            return Err(FramingError::UndeterminedOutPadding {
                offset: out_cursor,
                length: out_payload_capacity + 1,
            });
        }

        let mut marker_bits = if payload_len == out_payload_capacity {
            OUT_MARKER_BIT
        } else {
            0
        };

        if record.response_len != 0 {
            let in_tail = in_mps - (in_cursor & (in_mps - 1));
            if record.response_len > in_tail {
                let Some(previous_tag_offset) = last_tag_offset else {
                    return Err(FramingError::MissingPreviousRecordForInBoundary {
                        operation_index,
                    });
                };
                out_buffer[previous_tag_offset] |= IN_MARKER_BIT;
                in_cursor = in_cursor
                    .checked_add(in_tail)
                    .ok_or(FramingError::LengthOverflow {
                        direction: TransferDirection::In,
                    })?;
            }

            let current_in_tail = in_mps - (in_cursor & (in_mps - 1));
            if record.response_len == current_in_tail {
                marker_bits |= IN_MARKER_BIT;
            }
            let response_offset = in_cursor;
            in_cursor =
                in_cursor
                    .checked_add(record.response_len)
                    .ok_or(FramingError::LengthOverflow {
                        direction: TransferDirection::In,
                    })?;
            ensure_capacity(TransferDirection::In, in_cursor)?;
            response_spans.push(ResponseSpan {
                operation_index,
                offset: response_offset,
                length: record.response_len,
            });
            last_response_tag_offset = Some(out_cursor);
        }

        let out_end =
            out_cursor
                .checked_add(record.bytes.len())
                .ok_or(FramingError::LengthOverflow {
                    direction: TransferDirection::Out,
                })?;
        ensure_capacity(TransferDirection::Out, out_end)?;
        out_buffer.extend_from_slice(record.bytes);
        out_buffer[out_cursor] |= marker_bits;
        last_tag_offset = Some(out_cursor);
    }

    let Some(last_tag_offset) = last_tag_offset else {
        return Err(FramingError::EmptyOperationQueue);
    };
    out_buffer[last_tag_offset] |= OUT_MARKER_BIT;

    let expected_in_len = if in_cursor == 0 {
        0
    } else {
        let remainder = in_cursor & (in_mps - 1);
        if remainder == 0 {
            in_cursor
        } else {
            let padding = in_mps - remainder;
            let padded_len =
                in_cursor
                    .checked_add(padding)
                    .ok_or(FramingError::LengthOverflow {
                        direction: TransferDirection::In,
                    })?;
            ensure_capacity(TransferDirection::In, padded_len)?;
            let Some(last_response_tag_offset) = last_response_tag_offset else {
                return Err(FramingError::LengthOverflow {
                    direction: TransferDirection::In,
                });
            };
            out_buffer[last_response_tag_offset] |= IN_PADDING_MARKER_BITS;
            padded_len
        }
    };

    Ok(TransferPlan {
        out_buffer,
        expected_in_len,
        responses: response_spans,
    })
}

fn validate_mps(direction: TransferDirection, value: u16) -> Result<usize, FramingError> {
    let mps = usize::from(value);
    if mps < 4 || !mps.is_power_of_two() {
        return Err(FramingError::InvalidMps { direction, value });
    }
    Ok(mps)
}

fn ensure_capacity(direction: TransferDirection, attempted: usize) -> Result<(), FramingError> {
    if attempted > MAX_TRANSFER_BUFFER_LEN {
        return Err(FramingError::BufferTooLarge {
            direction,
            attempted,
            maximum: MAX_TRANSFER_BUFFER_LEN,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        FramingError, LogicalRecord, MAX_TRANSFER_BUFFER_LEN, TransferDirection, frame_records,
    };
    use crate::protocol::logical::ApibOperation;

    fn frame_one(
        operation: ApibOperation,
        response_len: usize,
        out_mps: u16,
        in_mps: u16,
    ) -> super::TransferPlan {
        let bytes = operation.encode();
        frame_records(
            &[LogicalRecord {
                bytes: &bytes,
                response_len,
            }],
            out_mps,
            in_mps,
        )
        .expect("single operation should fit")
    }

    #[test]
    fn emits_exact_single_operation_buffers_at_both_packet_sizes() {
        for mps in [64, 512] {
            let read_byte = frame_one(ApibOperation::ReadByte { argument: 0x12 }, 1, mps, mps);
            assert_eq!(read_byte.out_buffer, [0xeb, 0x01, 0x12]);
            assert_eq!(read_byte.expected_in_len, usize::from(mps));
            assert_eq!(read_byte.responses[0].operation_index, 0);
            assert_eq!(read_byte.responses[0].offset, 0);
            assert_eq!(read_byte.responses[0].length, 1);

            let write_byte = frame_one(
                ApibOperation::WriteByte {
                    argument: 0x1234_5678,
                    value: 0x2a,
                },
                0,
                mps,
                mps,
            );
            assert_eq!(write_byte.out_buffer, [0x8a, 0x01, 0x2a, 0x78]);
            assert_eq!(write_byte.expected_in_len, 0);
            assert!(write_byte.responses.is_empty());

            let read_word = frame_one(ApibOperation::ReadWord { argument: 0x12 }, 2, mps, mps);
            assert_eq!(read_word.out_buffer, [0xeb, 0x02, 0x12]);
            assert_eq!(read_word.expected_in_len, usize::from(mps));
            assert_eq!(read_word.responses[0].offset, 0);
            assert_eq!(read_word.responses[0].length, 2);

            let any_ready = frame_one(ApibOperation::AnyReady, 1, mps, mps);
            assert_eq!(any_ready.out_buffer, [0xe0, 0x01]);
            assert_eq!(any_ready.expected_in_len, usize::from(mps));
            assert_eq!(any_ready.responses[0].offset, 0);
            assert_eq!(any_ready.responses[0].length, 1);
        }
    }

    #[test]
    fn packs_operations_across_out_packet_boundaries_when_records_fit_exactly() {
        for mps in [64_usize, 512] {
            let per_packet = mps / 4;
            let encoded: Vec<Vec<u8>> = (0..=per_packet)
                .map(|_| {
                    ApibOperation::WriteByte {
                        argument: 0x1234_5678,
                        value: 0x2a,
                    }
                    .encode()
                })
                .collect();
            let records: Vec<_> = encoded
                .iter()
                .map(|bytes| LogicalRecord {
                    bytes,
                    response_len: 0,
                })
                .collect();
            let plan = frame_records(&records, mps as u16, mps as u16).unwrap();

            assert_eq!(plan.out_buffer.len(), (per_packet + 1) * 4);
            assert_eq!(plan.out_buffer[per_packet * 4 - 4], 0x8a);
            assert_eq!(plan.out_buffer[per_packet * 4], 0x8a);
            assert_eq!(plan.expected_in_len, 0);
        }
    }

    #[test]
    fn refuses_out_packet_gaps_whose_bytes_are_not_written_by_the_dll_path() {
        let encoded: Vec<Vec<u8>> = (0..22)
            .map(|_| ApibOperation::ReadByte { argument: 0x12 }.encode())
            .collect();
        let records: Vec<_> = encoded
            .iter()
            .map(|bytes| LogicalRecord {
                bytes,
                response_len: 1,
            })
            .collect();

        assert_eq!(
            frame_records(&records, 64, 64),
            Err(FramingError::UndeterminedOutPadding {
                offset: 63,
                length: 1,
            })
        );
    }

    #[test]
    fn skips_in_packet_tail_for_a_response_that_would_cross_the_boundary() {
        let mut encoded: Vec<Vec<u8>> = (0..63).map(|_| ApibOperation::AnyReady.encode()).collect();
        encoded.push(ApibOperation::ReadWord { argument: 0x12 }.encode());
        let records: Vec<_> = encoded
            .iter()
            .enumerate()
            .map(|(index, bytes)| LogicalRecord {
                bytes,
                response_len: if index == 63 { 2 } else { 1 },
            })
            .collect();

        let plan = frame_records(&records, 512, 64).unwrap();
        assert_eq!(plan.responses[63].operation_index, 63);
        assert_eq!(plan.responses[63].offset, 64);
        assert_eq!(plan.responses[63].length, 2);
        assert_eq!(plan.expected_in_len, 128);
        assert_eq!(plan.out_buffer[124], 0x40);
        assert_eq!(plan.out_buffer[126], 0xeb);
    }

    #[test]
    fn validates_received_length_before_exposing_a_response_slice() {
        let plan = frame_one(ApibOperation::ReadWord { argument: 0x12 }, 2, 512, 512);
        assert_eq!(
            plan.response_bytes(0, &[0x34]),
            Err(FramingError::InsufficientResponseData {
                operation_index: 0,
                required_end: 2,
                actual: 1,
            })
        );
        assert_eq!(plan.response_bytes(0, &[0x34, 0x12]), Ok(&[0x34, 0x12][..]));
        assert_eq!(
            plan.response_bytes(9, &[]),
            Err(FramingError::NoResponseForOperation { operation_index: 9 })
        );
    }

    #[test]
    fn rejects_invalid_packet_sizes_before_calculating_frames() {
        let bytes = ApibOperation::ReadByte { argument: 0x12 }.encode();
        let records = [LogicalRecord {
            bytes: &bytes,
            response_len: 1,
        }];
        for mps in [0, 1, 2, 3, 5, 63] {
            assert_eq!(
                frame_records(&records, mps, 64),
                Err(FramingError::InvalidMps {
                    direction: TransferDirection::Out,
                    value: mps,
                })
            );
        }
        for mps in [0, 1, 2, 3, 5, 63] {
            assert_eq!(
                frame_records(&records, 64, mps),
                Err(FramingError::InvalidMps {
                    direction: TransferDirection::In,
                    value: mps,
                })
            );
        }
    }

    #[test]
    fn enforces_both_fixed_buffer_capacities() {
        let write = ApibOperation::WriteByte {
            argument: 0,
            value: 0,
        }
        .encode();
        let writes: Vec<_> = (0..MAX_TRANSFER_BUFFER_LEN / write.len())
            .map(|_| LogicalRecord {
                bytes: &write,
                response_len: 0,
            })
            .collect();
        let plan = frame_records(&writes, 512, 512).unwrap();
        assert_eq!(plan.out_buffer.len(), MAX_TRANSFER_BUFFER_LEN);

        let too_many_writes: Vec<_> = (0..MAX_TRANSFER_BUFFER_LEN / write.len() + 1)
            .map(|_| LogicalRecord {
                bytes: &write,
                response_len: 0,
            })
            .collect();
        assert_eq!(
            frame_records(&too_many_writes, 512, 512),
            Err(FramingError::BufferTooLarge {
                direction: TransferDirection::Out,
                attempted: MAX_TRANSFER_BUFFER_LEN + write.len(),
                maximum: MAX_TRANSFER_BUFFER_LEN,
            })
        );

        let synthetic_tag = [0x00];
        let max_responses: Vec<_> = (0..MAX_TRANSFER_BUFFER_LEN / 2)
            .map(|_| LogicalRecord {
                bytes: &synthetic_tag,
                response_len: 2,
            })
            .collect();
        let max_in_plan = frame_records(&max_responses, 512, 512).unwrap();
        assert_eq!(max_in_plan.expected_in_len, MAX_TRANSFER_BUFFER_LEN);

        let responses: Vec<_> = (0..MAX_TRANSFER_BUFFER_LEN / 2 + 1)
            .map(|_| LogicalRecord {
                bytes: &synthetic_tag,
                response_len: 2,
            })
            .collect();
        assert!(matches!(
            frame_records(&responses, 512, 512),
            Err(FramingError::BufferTooLarge {
                direction: TransferDirection::In,
                maximum: MAX_TRANSFER_BUFFER_LEN,
                ..
            })
        ));
    }

    #[test]
    fn rejects_an_empty_queue_and_empty_logical_records() {
        assert_eq!(
            frame_records(&[], 64, 64),
            Err(FramingError::EmptyOperationQueue)
        );
        assert_eq!(
            frame_records(
                &[LogicalRecord {
                    bytes: &[],
                    response_len: 0,
                }],
                64,
                64
            ),
            Err(FramingError::EmptyLogicalRecord { operation_index: 0 })
        );
    }
}
