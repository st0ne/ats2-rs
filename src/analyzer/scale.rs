//! The evidenced primary multiplication stage of the ATS-1.5 analyzer scale path.
//!
//! This module does not select runtime factors or claim physical units.
//! Unknown range, unit, and calibration stages are intentionally not modeled.

/// Applies an explicitly supplied primary scale factor to a decoded number.
///
/// In the statically analyzed ATS-1.5 path, the scalar decode helper multiplies
/// by the double stored at object offset `+0x08`. The runtime value is selected
/// elsewhere and is not inferred here. The result has no assigned physical unit.
#[must_use]
pub fn apply_primary_scale(decoded: f64, object_primary_factor: f64) -> f64 {
    decoded * object_primary_factor
}

#[cfg(test)]
mod tests {
    use super::apply_primary_scale;

    #[test]
    fn applies_an_explicit_primary_factor() {
        assert_eq!(apply_primary_scale(0.25, 4.0), 1.0);
    }

    #[test]
    fn factor_one_preserves_the_decoded_value() {
        assert_eq!(apply_primary_scale(-0.375, 1.0), -0.375);
    }
}
