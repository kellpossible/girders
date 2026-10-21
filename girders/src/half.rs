//! Half-precision (IEEE 754 binary16) floats, as bits in a `u16`. Rust has
//! no stable `f16`, so these convert by hand. Formats such as CBOR store
//! halves natively; [`f16`](crate::codec::f16) uses them to quantize.

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

/// The nearest half-precision float to `v`, rounding ties to even, as
/// IEEE 754 conversion does. Values beyond the half range become
/// infinities; every NaN becomes the canonical quiet NaN.
pub fn f64_to_f16(v: f64) -> u16 {
    if v.is_nan() {
        return 0x7e00;
    }
    let bits = v.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let biased = ((bits >> 52) & 0x7ff) as i32;
    if biased == 0x7ff {
        return sign | 0x7c00;
    }
    if biased == 0 {
        // Zero or an f64 subnormal: far below the smallest half.
        return sign;
    }
    let mut exponent = biased - 1023;
    let significand = (1u64 << 52) | (bits & ((1 << 52) - 1));
    // Keep 11 significant bits for a normal half, fewer for a subnormal one
    // (whose quantum is fixed at 2^-24).
    let subnormal = exponent < -14;
    let shift = if subnormal { 28 - exponent } else { 42 } as u32;
    if shift >= 64 {
        return sign;
    }
    let kept = significand >> shift;
    let rest = significand & ((1 << shift) - 1);
    let halfway = 1u64 << (shift - 1);
    let mut rounded = if rest > halfway || (rest == halfway && kept & 1 == 1) {
        kept + 1
    } else {
        kept
    };
    if subnormal {
        // 1024 rounds up into the smallest normal, which has these bits.
        return sign | rounded as u16;
    }
    if rounded == 2048 {
        rounded = 1024;
        exponent += 1;
    }
    if exponent > 15 {
        return sign | 0x7c00;
    }
    sign | (((exponent + 15) as u16) << 10) | (rounded - 1024) as u16
}

/// `v` as a half-precision float, if that represents it exactly (NaN
/// included). Cheaper than rounding: most values fail the exponent or
/// mantissa check straight away.
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
    fn exact_agrees_with_rounding() {
        let mut v = 1e-9_f64;
        while v < 1e6 {
            for x in [v, -v, v * 1.000_000_1] {
                let rounded = f64_to_f16(x);
                let exact = f16_to_f64(rounded) == x;
                assert_eq!(f64_to_f16_exact(x), exact.then_some(rounded), "{x}");
            }
            v *= 1.001;
        }
    }

    #[test]
    fn inexact_values_are_rejected() {
        for v in [0.1, 65520.0, 1e-8, 1.0 + 1.0 / 2048.0, 3.0e-8] {
            assert_eq!(f64_to_f16_exact(v), None, "{v}");
        }
    }

    #[test]
    fn rounding_is_to_nearest_even() {
        // Between each pair of neighbouring positive halves: just below the
        // midpoint rounds down, just above rounds up, and the midpoint goes
        // to the even one.
        for bits in 0u16..0x7bff {
            let (lo, hi) = (f16_to_f64(bits), f16_to_f64(bits + 1));
            let mid = (lo + hi) / 2.0;
            let even = if bits & 1 == 0 { bits } else { bits + 1 };
            assert_eq!(f64_to_f16(mid), even, "midpoint of {bits:#06x}");
            assert_eq!(f64_to_f16(lo + (mid - lo) / 2.0), bits);
            assert_eq!(f64_to_f16(mid + (hi - mid) / 2.0), bits + 1);
            assert_eq!(f64_to_f16(-mid), 0x8000 | even);
        }
        assert_eq!(f64_to_f16(65519.0), 0x7bff);
        assert_eq!(f64_to_f16(65520.0), 0x7c00, "ties to even: infinity");
        assert_eq!(f64_to_f16(1e300), 0x7c00);
        assert_eq!(f64_to_f16(pow2(-25)), 0, "ties to even: zero");
        assert_eq!(f64_to_f16(pow2(-25) * 1.5), 1);
        assert_eq!(f64_to_f16(1.1), 0x3c66);
    }
}
