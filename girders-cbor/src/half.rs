//! Half-precision (IEEE 754 binary16) floats, which CBOR stores natively.
//! Rust has no stable `f16`, so these convert by hand.

/// `2^k` for `k` in the normal `f64` exponent range.
fn pow2(k: i32) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// The value of a half-precision float.
pub fn f16_to_f64(bits: u16) -> f64 {
    let negative = bits & 0x8000 != 0;
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let mantissa = (bits & 0x3ff) as f64;
    let magnitude = match exponent {
        // Subnormal: mantissa × 2^-24.
        0 => mantissa * pow2(-24),
        31 if mantissa == 0.0 => f64::INFINITY,
        31 => return f64::NAN,
        // Normal: (1 + mantissa / 1024) × 2^(exponent - 15).
        e => (1024.0 + mantissa) * pow2(e - 25),
    };
    if negative { -magnitude } else { magnitude }
}

/// `v` as a half-precision float, if that represents it exactly. Every NaN
/// maps to the canonical quiet NaN.
pub fn f64_to_f16_exact(v: f64) -> Option<u16> {
    if v.is_nan() {
        return Some(0x7e00);
    }
    let bits = v.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    if v.is_infinite() {
        return Some(sign | 0x7c00);
    }
    if v == 0.0 {
        return Some(sign);
    }
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    let mantissa = bits & ((1 << 52) - 1);
    if !(-24..=15).contains(&exponent) {
        return None;
    }
    if exponent >= -14 {
        // Normal: the mantissa's low 42 bits must be zero.
        if mantissa & ((1 << 42) - 1) != 0 {
            return None;
        }
        Some(sign | (((exponent + 15) as u16) << 10) | (mantissa >> 42) as u16)
    } else {
        // Subnormal: value = m × 2^-24 with m < 1024.
        let significand = (1 << 52) | mantissa;
        let shift = 28 - exponent;
        if significand & ((1 << shift) - 1) != 0 {
            return None;
        }
        Some(sign | (significand >> shift) as u16)
    }
}

/// `v` as an `f32`, if that represents it exactly (NaN included).
pub fn f64_to_f32_exact(v: f64) -> Option<f32> {
    let f = v as f32;
    (v.is_nan() || f as f64 == v).then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_half_round_trips() {
        for bits in 0..=u16::MAX {
            let v = f16_to_f64(bits);
            let back = f64_to_f16_exact(v).expect("every half is exact");
            if v.is_nan() {
                assert_eq!(back, 0x7e00);
            } else {
                assert_eq!(back, bits, "{bits:#06x} -> {v} -> {back:#06x}");
            }
        }
    }

    #[test]
    fn inexact_values_are_rejected() {
        for v in [0.1, 65520.0, 1e-8, 1.0 + 1.0 / 2048.0, 3.0e-8] {
            assert_eq!(f64_to_f16_exact(v), None, "{v}");
        }
    }
}
