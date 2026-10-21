//! CBOR writer producing RFC 8949 preferred serialization: the shortest
//! head for every argument, and the shortest float width that represents
//! the value exactly. Containers of unknown length use indefinite-length
//! encoding.

use crate::bignum;
use alloc::vec::Vec;
use core::convert::Infallible;
use girders::Serializer;
use girders::half;

pub struct CborSerializer {
    out: Vec<u8>,
    /// For each open container: whether it has indefinite length.
    open: Vec<bool>,
}

impl Default for CborSerializer {
    fn default() -> Self {
        Self::new()
    }
}

impl CborSerializer {
    pub fn new() -> Self {
        CborSerializer {
            out: Vec::new(),
            open: Vec::new(),
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.out
    }

    /// An initial byte and argument in the shortest form.
    fn head(&mut self, major: u8, arg: u64) {
        let m = major << 5;
        match arg {
            0..=23 => self.out.push(m | arg as u8),
            24..=0xff => self.out.extend_from_slice(&[m | 24, arg as u8]),
            0x100..=0xffff => {
                self.out.push(m | 25);
                self.out.extend_from_slice(&(arg as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.out.push(m | 26);
                self.out.extend_from_slice(&(arg as u32).to_be_bytes());
            }
            _ => {
                self.out.push(m | 27);
                self.out.extend_from_slice(&arg.to_be_bytes());
            }
        }
    }

    fn float(&mut self, v: f64) {
        if let Some(bits) = half::f64_to_f16_exact(v) {
            self.out.push(0xf9);
            self.out.extend_from_slice(&bits.to_be_bytes());
        } else if let Some(f) = half::f64_to_f32_exact(v) {
            self.out.push(0xfa);
            self.out.extend_from_slice(&f.to_bits().to_be_bytes());
        } else {
            self.out.push(0xfb);
            self.out.extend_from_slice(&v.to_bits().to_be_bytes());
        }
    }

    /// An integer given as optional `-` and decimal digits.
    fn integer_text(&mut self, negative: bool, digits: &str) {
        let digits = digits.trim_start_matches('0');
        let magnitude = bignum::from_decimal(digits);
        if !negative {
            match bignum::to_u128(&magnitude).and_then(|v| u64::try_from(v).ok()) {
                Some(v) => self.head(0, v),
                None => self.bignum(2, &magnitude),
            }
        } else if magnitude.is_empty() {
            // "-0" is zero.
            self.head(0, 0);
        } else {
            // -n is encoded as n - 1.
            let n = bignum::sub_one(&magnitude);
            match bignum::to_u128(&n).and_then(|v| u64::try_from(v).ok()) {
                Some(v) => self.head(1, v),
                None => self.bignum(3, &n),
            }
        }
    }

    fn bignum(&mut self, tag: u64, magnitude: &[u8]) {
        self.head(6, tag);
        self.head(2, magnitude.len() as u64);
        self.out.extend_from_slice(magnitude);
    }

    fn open(&mut self, major: u8, len: Option<usize>) {
        match len {
            Some(n) => self.head(major, n as u64),
            None => self.out.push((major << 5) | 31),
        }
        self.open.push(len.is_none());
    }

    fn close(&mut self) {
        if self.open.pop() == Some(true) {
            self.out.push(0xff);
        }
    }
}

impl Serializer for CborSerializer {
    type Error = Infallible;

    fn write_bool(&mut self, v: bool) -> Result<(), Infallible> {
        self.out.push(if v { 0xf5 } else { 0xf4 });
        Ok(())
    }

    fn write_null(&mut self) -> Result<(), Infallible> {
        self.out.push(0xf6);
        Ok(())
    }

    #[inline]
    fn write_i64(&mut self, v: i64) -> Result<(), Infallible> {
        if v >= 0 {
            self.head(0, v as u64);
        } else {
            // -1 - n == !n
            self.head(1, !(v as u64));
        }
        Ok(())
    }

    #[inline]
    fn write_u64(&mut self, v: u64) -> Result<(), Infallible> {
        self.head(0, v);
        Ok(())
    }

    fn write_f64(&mut self, v: f64) -> Result<(), Infallible> {
        self.float(v);
        Ok(())
    }

    fn write_f32(&mut self, v: f32) -> Result<(), Infallible> {
        self.float(v as f64);
        Ok(())
    }

    /// Integers become plain integers or bignums (tags 2 and 3); numbers
    /// with a fraction or exponent become exact decimal fractions (tag 4).
    fn write_number_text(&mut self, v: &str) -> Result<(), Infallible> {
        let (negative, rest) = match v.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, v),
        };
        let (mantissa_part, exponent) = match rest.find(['e', 'E']) {
            Some(i) => (&rest[..i], rest[i + 1..].parse::<i64>().unwrap_or(0)),
            None => (rest, 0),
        };
        let (whole, fraction) = mantissa_part.split_once('.').unwrap_or((mantissa_part, ""));
        if fraction.is_empty() && exponent == 0 {
            self.integer_text(negative, whole);
            return Ok(());
        }
        let mut digits = alloc::string::String::with_capacity(whole.len() + fraction.len());
        digits.push_str(whole);
        digits.push_str(fraction);
        self.head(6, 4);
        self.head(4, 2);
        self.write_i64(exponent - fraction.len() as i64)?;
        self.integer_text(negative, &digits);
        Ok(())
    }

    #[inline]
    fn write_str(&mut self, v: &str) -> Result<(), Infallible> {
        self.head(3, v.len() as u64);
        self.out.extend_from_slice(v.as_bytes());
        Ok(())
    }

    fn write_bytes(&mut self, v: &[u8]) -> Result<(), Infallible> {
        self.head(2, v.len() as u64);
        self.out.extend_from_slice(v);
        Ok(())
    }

    fn supports_tags(&self) -> bool {
        true
    }

    fn write_tag(&mut self, tag: u64) -> Result<(), Infallible> {
        self.head(6, tag);
        Ok(())
    }

    fn write_simple(&mut self, v: u8) -> Result<(), Infallible> {
        if v < 24 {
            self.out.push(0xe0 | v);
        } else {
            self.out.extend_from_slice(&[0xf8, v]);
        }
        Ok(())
    }

    fn begin_array(&mut self, len: Option<usize>) -> Result<(), Infallible> {
        self.open(4, len);
        Ok(())
    }

    #[inline]
    fn array_element(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn end_array(&mut self) -> Result<(), Infallible> {
        self.close();
        Ok(())
    }

    fn begin_object(&mut self, len: Option<usize>) -> Result<(), Infallible> {
        self.open(5, len);
        Ok(())
    }

    #[inline]
    fn object_key(&mut self, key: &str) -> Result<(), Infallible> {
        self.write_str(key)
    }

    fn object_key_value(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn end_object(&mut self) -> Result<(), Infallible> {
        self.close();
        Ok(())
    }
}
