//! A format-neutral number, for readers that must report a number
//! without knowing the target type (dynamic `Value` trees).

use alloc::string::String;
use core::fmt;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Number {
    I64(i64),
    /// Only for integers above `i64::MAX`.
    U64(u64),
    F64(f64),
    /// An integer that fits neither `i64` nor `u64`, kept as its text.
    BigInt(String),
}

impl Number {
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Number::I64(i) => Some(i),
            Number::U64(u) => i64::try_from(u).ok(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Number::I64(i) => u64::try_from(i).ok(),
            Number::U64(u) => Some(u),
            _ => None,
        }
    }

    /// Lossy for integers beyond 2^53.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Number::I64(i) => Some(*i as f64),
            Number::U64(u) => Some(*u as f64),
            Number::F64(f) => Some(*f),
            Number::BigInt(s) => s.parse().ok(),
        }
    }

    pub fn is_integer(&self) -> bool {
        !matches!(self, Number::F64(_))
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::I64(i) => write!(f, "{i}"),
            Number::U64(u) => write!(f, "{u}"),
            Number::F64(x) => write!(f, "{x:?}"),
            Number::BigInt(s) => f.write_str(s),
        }
    }
}
