//! Typed arrays: a list of numbers of one type, packed into a byte string
//! in formats that support it (CBOR, RFC 8746).
//!
//! `packed(u16())` decodes and encodes `Vec<u16>`. Where the format has
//! tags, it is written as the RFC 8746 tag for little-endian `u16` over a
//! byte string, 2 bytes per element, so writing and reading are copies
//! rather than per-element encoding. Reading accepts either byte order,
//! and a plain array too. Formats without tags (JSON) read and write a
//! plain array of numbers.

use crate::codec::{
    Codec, DecodeResult, Decoder, Encoder, F16Codec, F32Codec, F64Codec, I8Codec, I16Codec,
    I32Codec, I64Codec, U8Codec, U16Codec, U32Codec, U64Codec, decode_child,
};
use crate::collections::list;
use crate::context::{DecodeContext, EncodeContext};
use crate::error::{DecodeError, Problem};
use crate::half;
use crate::reader::{Kind, Reader};
use crate::serializer::Serializer;
use alloc::vec::Vec;

/// A number codec whose values can be packed into a typed array.
pub trait PackedElement: Codec<Output: Copy> + Copy {
    /// Bytes per element.
    const SIZE: usize;
    /// The RFC 8746 tag for a big-endian array of this type.
    const TAG_BIG_ENDIAN: u64;
    /// The RFC 8746 tag for a little-endian array (the same as big-endian
    /// for one-byte types).
    const TAG_LITTLE_ENDIAN: u64;
    fn from_big_endian(bytes: &[u8]) -> Self::Output;
    fn from_little_endian(bytes: &[u8]) -> Self::Output;
    fn push_little_endian(v: Self::Output, out: &mut Vec<u8>);
}

macro_rules! packed_element {
    ($($codec:ty, $t:ty, $big:literal, $little:literal;)*) => {$(
        impl PackedElement for $codec {
            const SIZE: usize = core::mem::size_of::<$t>();
            const TAG_BIG_ENDIAN: u64 = $big;
            const TAG_LITTLE_ENDIAN: u64 = $little;
            fn from_big_endian(bytes: &[u8]) -> $t {
                <$t>::from_be_bytes(bytes.try_into().expect("chunk of SIZE bytes"))
            }
            fn from_little_endian(bytes: &[u8]) -> $t {
                <$t>::from_le_bytes(bytes.try_into().expect("chunk of SIZE bytes"))
            }
            fn push_little_endian(v: $t, out: &mut Vec<u8>) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    )*};
}

packed_element! {
    U8Codec, u8, 64, 64;
    U16Codec, u16, 65, 69;
    U32Codec, u32, 66, 70;
    U64Codec, u64, 67, 71;
    I8Codec, i8, 72, 72;
    I16Codec, i16, 73, 77;
    I32Codec, i32, 74, 78;
    I64Codec, i64, 75, 79;
    F32Codec, f32, 81, 85;
    F64Codec, f64, 82, 86;
}

impl PackedElement for F16Codec {
    const SIZE: usize = 2;
    const TAG_BIG_ENDIAN: u64 = 80;
    const TAG_LITTLE_ENDIAN: u64 = 84;
    fn from_big_endian(bytes: &[u8]) -> f32 {
        half::f16_to_f64(u16::from_be_bytes([bytes[0], bytes[1]])) as f32
    }
    fn from_little_endian(bytes: &[u8]) -> f32 {
        half::f16_to_f64(u16::from_le_bytes([bytes[0], bytes[1]])) as f32
    }
    fn push_little_endian(v: f32, out: &mut Vec<u8>) {
        out.extend_from_slice(&half::f64_to_f16(v as f64).to_le_bytes());
    }
}

/// A typed array whose byte length isn't a whole number of elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedArrayLengthError {
    pub element_size: usize,
    pub length: usize,
}

impl core::fmt::Display for TypedArrayLengthError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "a typed array of {}-byte elements is {} bytes long",
            self.element_size, self.length
        )
    }
}

impl core::error::Error for TypedArrayLengthError {}

/// See the [module documentation](self).
#[derive(Debug, Clone, Copy)]
pub struct Packed<E>(E);

/// A list of `element` values, packed into a typed array where the format
/// supports it.
pub const fn packed<E: PackedElement>(element: E) -> Packed<E> {
    Packed(element)
}

/// RFC 8746 tag 68: `u8`, clamped (a hint for the producer; the bytes
/// read the same).
const TAG_U8_CLAMPED: u64 = 68;

impl<E: PackedElement> Decoder for Packed<E> {
    type Output = Vec<E::Output>;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<Vec<E::Output>, R> {
        if !(r.supports_tags() && r.peek_kind()? == Kind::Tag) {
            return decode_child(&list(self.0), r, cx);
        }
        let at = r.here();
        let tag = r.read_tag()?;
        let little = tag == E::TAG_LITTLE_ENDIAN;
        let clamped = E::TAG_BIG_ENDIAN == 64 && tag == TAG_U8_CLAMPED;
        if !(little || clamped || tag == E::TAG_BIG_ENDIAN) {
            return Err(DecodeError::at(
                Problem::mismatch(
                    alloc::format!(
                        "typed array (tag {} or {})",
                        E::TAG_LITTLE_ENDIAN,
                        E::TAG_BIG_ENDIAN
                    ),
                    alloc::format!("tag {tag}"),
                ),
                at,
            ));
        }
        let mut bytes = Vec::new();
        r.read_bytes(&mut bytes)?;
        if bytes.len() % E::SIZE != 0 {
            let error = TypedArrayLengthError {
                element_size: E::SIZE,
                length: bytes.len(),
            };
            return Err(DecodeError::invalid(error, at));
        }
        let chunks = bytes.chunks_exact(E::SIZE);
        Ok(if little {
            chunks.map(E::from_little_endian).collect()
        } else {
            chunks.map(E::from_big_endian).collect()
        })
    }
}

impl<E: PackedElement> Encoder for Packed<E> {
    type Input = Vec<E::Output>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Vec<E::Output>,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        if !s.supports_tags() {
            return list(self.0).encode(v, s, cx);
        }
        let mut bytes = Vec::with_capacity(v.len() * E::SIZE);
        for &x in v {
            E::push_little_endian(x, &mut bytes);
        }
        s.write_tag(E::TAG_LITTLE_ENDIAN)?;
        s.write_bytes(&bytes)
    }
}
