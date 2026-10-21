//! Arbitrary-size unsigned integers as big-endian bytes, for CBOR bignums
//! (tags 2 and 3), converted to and from decimal text.

use alloc::string::String;
use alloc::vec::Vec;

/// Decimal text of a big-endian unsigned integer.
pub fn to_decimal(bytes: &[u8]) -> String {
    // Base 10^9 limbs, least significant first.
    let mut limbs: Vec<u32> = Vec::new();
    for &byte in bytes {
        let mut carry = byte as u64;
        for limb in &mut limbs {
            let v = (*limb as u64) * 256 + carry;
            *limb = (v % 1_000_000_000) as u32;
            carry = v / 1_000_000_000;
        }
        while carry > 0 {
            limbs.push((carry % 1_000_000_000) as u32);
            carry /= 1_000_000_000;
        }
    }
    let Some((top, rest)) = limbs.split_last() else {
        return "0".into();
    };
    let mut out = alloc::format!("{top}");
    for limb in rest.iter().rev() {
        out.push_str(&alloc::format!("{limb:09}"));
    }
    out
}

/// Big-endian bytes of a decimal digit string, without leading zeros.
pub fn from_decimal(digits: &str) -> Vec<u8> {
    // Little-endian base 256 while accumulating.
    let mut le: Vec<u8> = Vec::new();
    for d in digits.bytes() {
        let mut carry = (d - b'0') as u32;
        for byte in &mut le {
            let v = (*byte as u32) * 10 + carry;
            *byte = v as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            le.push(carry as u8);
            carry >>= 8;
        }
    }
    le.reverse();
    le
}

/// `bytes + 1`, big-endian.
pub fn add_one(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for b in out.iter_mut().rev() {
        let (v, overflow) = b.overflowing_add(1);
        *b = v;
        if !overflow {
            return out;
        }
    }
    out.insert(0, 1);
    out
}

/// `bytes - 1`, big-endian, for a non-zero value; leading zeros dropped.
pub fn sub_one(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for b in out.iter_mut().rev() {
        let (v, borrow) = b.overflowing_sub(1);
        *b = v;
        if !borrow {
            break;
        }
    }
    let zeros = out.iter().take_while(|&&b| b == 0).count();
    out.drain(..zeros);
    out
}

/// The value as a `u128`, if it fits.
pub fn to_u128(bytes: &[u8]) -> Option<u128> {
    let significant = &bytes[bytes.iter().take_while(|&&b| b == 0).count()..];
    if significant.len() > 16 {
        return None;
    }
    Some(
        significant
            .iter()
            .fold(0u128, |acc, &b| (acc << 8) | b as u128),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_round_trip() {
        for text in [
            "0",
            "1",
            "255",
            "256",
            "18446744073709551616",
            "340282366920938463463374607431768211456",
        ] {
            let bytes = from_decimal(text);
            assert_eq!(to_decimal(&bytes), text);
        }
        assert_eq!(
            from_decimal("18446744073709551616"),
            [1, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn increments() {
        assert_eq!(add_one(&[0xff, 0xff]), [1, 0, 0]);
        assert_eq!(sub_one(&[1, 0, 0]), [0xff, 0xff]);
        assert_eq!(sub_one(&[1]), Vec::<u8>::new());
    }
}
