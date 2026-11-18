/// Numerically evidenced 24-bit analyzer payload interpretations.
///
/// The caller must select the format explicitly from known runtime state. This
/// decoder does not apply range, unit, calibration, or physical-unit scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzerRawFormat {
    /// Signed two's-complement 24-bit integer multiplied by 2^-23.
    SignedQ1_23,
    /// Signed 18-bit mantissa (bits 23..6) and signed 6-bit exponent.
    SignedMantissa18Exponent6,
}

/// Failure modes evidenced in the ATS-1.5 decoder path.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    /// The packed mantissa/exponent helper rejects an exactly zero DWORD.
    #[error("zero DWORD is invalid in the signed mantissa/exponent format")]
    ZeroWordInExponentFormat,
}

/// Decode one 32-bit host container using an explicitly selected 24-bit format.
///
/// The result is the numeric output of the raw decoder only; it is not Vrms.
/// The upper byte is ignored for both formats, except that the exponent format
/// tests the complete DWORD for zero before masking, matching the static path.
pub fn decode_raw_word(raw: u32, format: AnalyzerRawFormat) -> Result<f64, DecodeError> {
    match format {
        AnalyzerRawFormat::SignedQ1_23 => {
            let signed_24 = sign_extend(raw & 0x00ff_ffff, 24);
            Ok(f64::from(signed_24) / 8_388_608.0)
        }
        AnalyzerRawFormat::SignedMantissa18Exponent6 => {
            if raw == 0 {
                return Err(DecodeError::ZeroWordInExponentFormat);
            }

            let exponent = sign_extend(raw & 0x3f, 6);
            let mantissa = sign_extend((raw >> 6) & 0x3ffff, 18);
            Ok(f64::from(mantissa) * 2.0_f64.powi(exponent - 17))
        }
    }
}

fn sign_extend(value: u32, width: u32) -> i32 {
    let shift = 32 - width;
    ((value << shift) as i32) >> shift
}

#[cfg(test)]
mod tests {
    use super::{AnalyzerRawFormat, DecodeError, decode_raw_word};

    fn encode_mantissa_exp(mantissa: i32, exponent: i32) -> u32 {
        (((mantissa as u32) & 0x3ffff) << 6) | ((exponent as u32) & 0x3f)
    }

    #[test]
    fn q1_23_decodes_zero_unit_and_positive_limit() {
        assert_eq!(decode_raw_word(0, AnalyzerRawFormat::SignedQ1_23), Ok(0.0));
        assert_eq!(
            decode_raw_word(1, AnalyzerRawFormat::SignedQ1_23),
            Ok(1.0 / 8_388_608.0)
        );
        assert_eq!(
            decode_raw_word(0x007f_ffff, AnalyzerRawFormat::SignedQ1_23),
            Ok(8_388_607.0 / 8_388_608.0)
        );
    }

    #[test]
    fn q1_23_sign_extends_twenty_four_bit_boundaries_and_ignores_upper_byte() {
        assert_eq!(
            decode_raw_word(0x0080_0000, AnalyzerRawFormat::SignedQ1_23),
            Ok(-1.0)
        );
        assert_eq!(
            decode_raw_word(0x00ff_ffff, AnalyzerRawFormat::SignedQ1_23),
            Ok(-1.0 / 8_388_608.0)
        );
        assert_eq!(
            decode_raw_word(0xff80_0000, AnalyzerRawFormat::SignedQ1_23),
            Ok(-1.0)
        );
    }

    #[test]
    fn mantissa_exponent_decodes_signed_mantissa_boundaries() {
        assert_eq!(
            decode_raw_word(
                encode_mantissa_exp(131_071, 0),
                AnalyzerRawFormat::SignedMantissa18Exponent6
            ),
            Ok(131_071.0 / 131_072.0)
        );
        assert_eq!(
            decode_raw_word(
                encode_mantissa_exp(-131_072, 0),
                AnalyzerRawFormat::SignedMantissa18Exponent6
            ),
            Ok(-1.0)
        );
    }

    #[test]
    fn mantissa_exponent_decodes_signed_six_bit_exponent_boundaries() {
        assert_eq!(
            decode_raw_word(
                encode_mantissa_exp(1, -32),
                AnalyzerRawFormat::SignedMantissa18Exponent6
            ),
            Ok(2.0_f64.powi(-49))
        );
        assert_eq!(
            decode_raw_word(
                encode_mantissa_exp(1, 31),
                AnalyzerRawFormat::SignedMantissa18Exponent6
            ),
            Ok(16_384.0)
        );
    }

    #[test]
    fn exponent_format_rejects_only_the_exact_zero_dword_before_masking() {
        assert_eq!(
            decode_raw_word(0, AnalyzerRawFormat::SignedMantissa18Exponent6),
            Err(DecodeError::ZeroWordInExponentFormat)
        );
        assert_eq!(
            decode_raw_word(0x0100_0000, AnalyzerRawFormat::SignedMantissa18Exponent6),
            Ok(0.0)
        );
    }

    #[test]
    fn format_is_explicit_and_exponent_branch_ignores_upper_byte() {
        assert_eq!(
            decode_raw_word(1, AnalyzerRawFormat::SignedQ1_23),
            Ok(1.0 / 8_388_608.0)
        );
        assert_eq!(
            decode_raw_word(1, AnalyzerRawFormat::SignedMantissa18Exponent6),
            Ok(0.0)
        );
        assert_eq!(
            decode_raw_word(0xff00_005f, AnalyzerRawFormat::SignedMantissa18Exponent6),
            Ok(16_384.0)
        );
    }
}
