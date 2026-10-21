//! A dynamic CBOR data item.

use crate::bignum;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use girders::{
    DecodeContext, DecodeError, Decoder, EncodeContext, Encoder, Kind, Number, Problem, Reader,
    Serializer,
};

/// Any CBOR data item.
///
/// Integers cover CBOR's whole native range (−2^64 to 2^64 − 1) and
/// bignums within `i128`; larger bignums stay as tag 2 or 3 over their
/// bytes. Floats keep their value, not their width: encoding picks the
/// shortest exact width. Maps keep their entries in order, with keys of
/// any kind.
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i128),
    Float(f64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Tag(u64, Box<Value>),
    Bool(bool),
    Null,
    Undefined,
    /// Any other simple value.
    Simple(u8),
}

/// Floats compare by bit pattern, so `NaN == NaN` and `0.0 != -0.0`.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Integer(a), Integer(b)) => a == b,
            (Float(a), Float(b)) => a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
            (Bytes(a), Bytes(b)) => a == b,
            (Text(a), Text(b)) => a == b,
            (Array(a), Array(b)) => a == b,
            (Map(a), Map(b)) => a == b,
            (Tag(t, a), Tag(u, b)) => t == u && a == b,
            (Bool(a), Bool(b)) => a == b,
            (Null, Null) | (Undefined, Undefined) => true,
            (Simple(a), Simple(b)) => a == b,
            _ => false,
        }
    }
}

/// The codec for [`Value`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ValueCodec;

pub const fn value() -> ValueCodec {
    ValueCodec
}

impl Decoder for ValueCodec {
    type Output = Value;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> Result<Value, DecodeError<R::Location>> {
        decode_value(r, cx)
    }
}

fn decode_value<R: Reader + ?Sized>(
    r: &mut R,
    cx: &mut DecodeContext,
) -> Result<Value, DecodeError<R::Location>> {
    Ok(match r.peek_kind()? {
        Kind::Number => match r.read_number()? {
            Number::I64(i) => Value::Integer(i as i128),
            Number::U64(u) => Value::Integer(u as i128),
            Number::F64(f) => Value::Float(f),
            Number::BigInt(text) => Value::Integer(text.parse().map_err(|_| {
                DecodeError::at(Problem::OutOfRange("integer beyond i128".into()), r.here())
            })?),
        },
        Kind::Bytes => {
            let mut b = Vec::new();
            r.read_bytes(&mut b)?;
            Value::Bytes(b)
        }
        Kind::String => {
            let mut s = String::new();
            r.read_str(&mut s)?;
            Value::Text(s)
        }
        Kind::Array => {
            r.begin_array()?;
            let capacity = girders::reader::cautious_capacity::<Value>(r.size_hint());
            let items = cx.nested(|cx| {
                let mut items = Vec::with_capacity(capacity);
                let mut first = true;
                while r.array_has_next(first)? {
                    first = false;
                    items.push(decode_value(r, cx)?);
                }
                Ok(items)
            })?;
            r.end_array()?;
            Value::Array(items)
        }
        Kind::Object => {
            r.begin_object()?;
            let capacity = girders::reader::cautious_capacity::<(Value, Value)>(r.size_hint());
            let entries = cx.nested(|cx| {
                let mut entries = Vec::with_capacity(capacity);
                let mut first = true;
                while r.object_next_entry(first)? {
                    first = false;
                    let key = decode_value(r, cx)?;
                    entries.push((key, decode_value(r, cx)?));
                }
                Ok(entries)
            })?;
            r.end_object()?;
            Value::Map(entries)
        }
        Kind::Tag => {
            let tag = r.read_tag()?;
            let content = cx.nested(|cx| decode_value(r, cx))?;
            match (tag, &content) {
                // Bignums that fit become plain integers.
                (2 | 3, Value::Bytes(b)) => {
                    match bignum::to_u128(b).and_then(|m| i128::try_from(m).ok()) {
                        Some(m) => Value::Integer(if tag == 3 { -1 - m } else { m }),
                        None => Value::Tag(tag, Box::new(content)),
                    }
                }
                _ => Value::Tag(tag, Box::new(content)),
            }
        }
        Kind::Bool => Value::Bool(r.read_bool()?),
        Kind::Null => {
            r.read_null()?;
            Value::Null
        }
        Kind::Simple => match r.read_simple()? {
            23 => Value::Undefined,
            n => Value::Simple(n),
        },
        Kind::Eof => return Err(girders::reader::mismatch(r, "value")),
    })
}

impl Encoder for ValueCodec {
    type Input = Value;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Value,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        match v {
            Value::Integer(i) => {
                if let Ok(i) = i64::try_from(*i) {
                    s.write_i64(i)
                } else if let Ok(u) = u64::try_from(*i) {
                    s.write_u64(u)
                } else {
                    s.write_number_text(&i.to_string())
                }
            }
            Value::Float(f) => s.write_f64(*f),
            Value::Bytes(b) => s.write_bytes(b),
            Value::Text(t) => s.write_str(t),
            Value::Array(items) => {
                s.begin_array(Some(items.len()))?;
                for item in items {
                    s.array_element()?;
                    self.encode(item, s, cx)?;
                }
                s.end_array()
            }
            Value::Map(entries) => {
                s.begin_object(Some(entries.len()))?;
                for (key, value) in entries {
                    match key {
                        Value::Text(k) => s.object_key(k)?,
                        _ => {
                            s.object_key_value()?;
                            self.encode(key, s, cx)?;
                        }
                    }
                    self.encode(value, s, cx)?;
                }
                s.end_object()
            }
            Value::Tag(tag, content) => {
                s.write_tag(*tag)?;
                self.encode(content, s, cx)
            }
            Value::Bool(b) => s.write_bool(*b),
            Value::Null => s.write_null(),
            Value::Undefined => s.write_simple(23),
            Value::Simple(n) => s.write_simple(*n),
        }
    }
}

/// RFC 8949 diagnostic notation, for example `{1: h'01ff', "a": [1.5, null]}`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Integer(i) => write!(f, "{i}"),
            Value::Float(x) if x.is_nan() => f.write_str("NaN"),
            Value::Float(x) if x.is_infinite() => {
                f.write_str(if *x > 0.0 { "Infinity" } else { "-Infinity" })
            }
            Value::Float(x) => write!(f, "{x:?}"),
            Value::Bytes(b) => {
                f.write_str("h'")?;
                for byte in b {
                    write!(f, "{byte:02x}")?;
                }
                f.write_str("'")
            }
            Value::Text(t) => write!(f, "{t:?}"),
            Value::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                f.write_str("}")
            }
            Value::Tag(tag, content) => write!(f, "{tag}({content})"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Null => f.write_str("null"),
            Value::Undefined => f.write_str("undefined"),
            Value::Simple(n) => write!(f, "simple({n})"),
        }
    }
}
