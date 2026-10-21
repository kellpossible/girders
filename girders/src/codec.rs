//! The codec traits, primitives and the basic combinators.

use crate::context::{DecodeContext, EncodeContext};
use crate::error::{ContextFrame, DecodeError, Problem};
use crate::reader::{Kind, Reader};
use crate::serializer::Serializer;
use crate::span::Spanned;
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Display;
use core::marker::PhantomData;

pub use crate::reader::DecodeResult;

/// Decodes a `Self::Output` from any [`Reader`].
pub trait Decoder {
    type Output;

    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R>;
}

/// Decode a child codec. Codec implementations call their children
/// through this rather than `child.decode(...)` directly.
///
/// It is never inlined, so each codec type's decoding exists once and is
/// shared by every place that uses it, instead of being copied into every
/// parent. That keeps binaries small and release builds fast, for a few
/// percent of decoding speed.
#[inline(never)]
pub fn decode_child<C: Decoder + ?Sized, R: Reader + ?Sized>(
    c: &C,
    r: &mut R,
    cx: &mut DecodeContext,
) -> DecodeResult<C::Output, R> {
    c.decode(r, cx)
}

/// Encodes a `&Self::Input` to any [`Serializer`].
pub trait Encoder {
    type Input: ?Sized;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error>;
}

/// Both directions for one type. Implemented automatically.
pub trait Codec: Decoder + Encoder<Input = <Self as Decoder>::Output> {}
impl<T> Codec for T where T: Decoder + Encoder<Input = <T as Decoder>::Output> {}

impl<D: Decoder + ?Sized> Decoder for &D {
    type Output = D::Output;
    #[inline(always)]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<D::Output, R> {
        (**self).decode(r, cx)
    }
}

impl<E: Encoder + ?Sized> Encoder for &E {
    type Input = E::Input;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &E::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        (**self).encode(v, s, cx)
    }
}

// -- Primitives ----------------------------------------------------------------

macro_rules! unit_codec {
    ($(#[$m:meta])* $name:ident, $fn:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;
        $(#[$m])*
        pub const fn $fn() -> $name {
            $name
        }
    };
}

unit_codec!(
    /// `String`.
    Str, string
);
unit_codec!(
    /// `bool`.
    BoolCodec, bool
);
unit_codec!(
    /// `null` as `()`.
    NullCodec, null
);
unit_codec!(
    /// `f64`; integers are accepted and converted.
    F64Codec, f64
);
unit_codec!(
    /// `f32`; `OutOfRange` if the value doesn't fit a finite `f32`.
    F32Codec, f32
);

unit_codec!(
    /// `Vec<u8>` as a byte string. Formats without byte strings use an
    /// array of numbers from 0 to 255.
    BytesCodec, bytes
);

impl Decoder for BytesCodec {
    type Output = Vec<u8>;
    #[inline]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<Vec<u8>, R> {
        let mut b = Vec::new();
        r.read_bytes(&mut b)?;
        Ok(b)
    }
}
impl Encoder for BytesCodec {
    type Input = Vec<u8>;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Vec<u8>,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_bytes(v)
    }
}

impl Decoder for Str {
    type Output = String;
    #[inline]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<String, R> {
        let mut s = String::new();
        r.read_str(&mut s)?;
        Ok(s)
    }
}
impl Encoder for Str {
    type Input = String;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &String,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_str(v)
    }
}

impl Decoder for BoolCodec {
    type Output = bool;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<bool, R> {
        r.read_bool()
    }
}
impl Encoder for BoolCodec {
    type Input = bool;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &bool,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_bool(*v)
    }
}

impl Decoder for NullCodec {
    type Output = ();
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<(), R> {
        r.read_null()
    }
}
impl Encoder for NullCodec {
    type Input = ();
    fn encode<S: Serializer + ?Sized>(
        &self,
        _v: &(),
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_null()
    }
}

impl Decoder for F64Codec {
    type Output = f64;
    #[inline]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<f64, R> {
        r.read_f64()
    }
}
impl Encoder for F64Codec {
    type Input = f64;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &f64,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_f64(*v)
    }
}

impl Decoder for F32Codec {
    type Output = f32;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<f32, R> {
        let start = r.here();
        let v = r.read_f64()?;
        let f = v as f32;
        if v.is_finite() && !f.is_finite() {
            return Err(DecodeError::at(
                Problem::OutOfRange("number does not fit f32".into()),
                start,
            ));
        }
        Ok(f)
    }
}
impl Encoder for F32Codec {
    type Input = f32;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &f32,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_f32(*v)
    }
}

/// Integer codecs read through `i64`/`u64` and range-check.
macro_rules! int_codec {
    ($name:ident, $fn:ident, $t:ty, $read:ident, $write:ident, $wide:ty) => {
        #[doc = concat!("`", stringify!($t), "`, range-checked.")]
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;
        #[doc = concat!("`", stringify!($t), "`, range-checked.")]
        pub const fn $fn() -> $name {
            $name
        }
        impl Decoder for $name {
            type Output = $t;
            #[inline]
            fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<$t, R> {
                let start = r.here();
                let v = r.$read()?;
                <$t>::try_from(v).map_err(|_| {
                    DecodeError::at(
                        Problem::OutOfRange(alloc::format!(
                            "{v} is out of range for {} ({}..={})",
                            stringify!($t),
                            <$t>::MIN,
                            <$t>::MAX
                        )),
                        start,
                    )
                })
            }
        }
        impl Encoder for $name {
            type Input = $t;
            #[inline]
            fn encode<S: Serializer + ?Sized>(
                &self,
                v: &$t,
                s: &mut S,
                _cx: &mut EncodeContext,
            ) -> Result<(), S::Error> {
                s.$write(*v as $wide)
            }
        }
    };
}
int_codec!(I8Codec, i8, i8, read_i64, write_i64, i64);
int_codec!(I16Codec, i16, i16, read_i64, write_i64, i64);
int_codec!(I32Codec, i32, i32, read_i64, write_i64, i64);
int_codec!(I64Codec, i64, i64, read_i64, write_i64, i64);
int_codec!(U8Codec, u8, u8, read_u64, write_u64, u64);
int_codec!(U16Codec, u16, u16, read_u64, write_u64, u64);
int_codec!(U32Codec, u32, u32, read_u64, write_u64, u64);
int_codec!(U64Codec, u64, u64, read_u64, write_u64, u64);
int_codec!(UsizeCodec, usize, usize, read_u64, write_u64, u64);

/// `i128`/`u128` go through the number's text.
macro_rules! wide_int_codec {
    ($name:ident, $fn:ident, $t:ty) => {
        #[doc = concat!("`", stringify!($t), "`, via the number's text.")]
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;
        #[doc = concat!("`", stringify!($t), "`, via the number's text.")]
        pub const fn $fn() -> $name {
            $name
        }
        impl Decoder for $name {
            type Output = $t;
            fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<$t, R> {
                let start = r.here();
                let mut text = String::new();
                r.read_number_text(&mut text)?;
                text.parse::<$t>().map_err(|_| {
                    let problem = if text.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                        Problem::OutOfRange(alloc::format!(
                            "{text} is out of range for {}",
                            stringify!($t)
                        ))
                    } else {
                        Problem::mismatch("integer", "number")
                    };
                    DecodeError::at(problem, start)
                })
            }
        }
        impl Encoder for $name {
            type Input = $t;
            fn encode<S: Serializer + ?Sized>(
                &self,
                v: &$t,
                s: &mut S,
                _cx: &mut EncodeContext,
            ) -> Result<(), S::Error> {
                s.write_number_text(&v.to_string())
            }
        }
    };
}
wide_int_codec!(I128Codec, i128, i128);
wide_int_codec!(U128Codec, u128, u128);

// -- Tags ------------------------------------------------------------------------

/// A value carrying tag number `tag` (CBOR tags, such as 1 for epoch
/// times). Decoding requires the tag, in formats that have tags. Formats
/// without tags skip the check on decoding and drop the tag on encoding.
#[derive(Debug, Clone, Copy)]
pub struct Tagged<C> {
    tag: u64,
    inner: C,
}

pub const fn tagged<C: Codec>(tag: u64, inner: C) -> Tagged<C> {
    Tagged { tag, inner }
}

impl<C: Decoder> Decoder for Tagged<C> {
    type Output = C::Output;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<C::Output, R> {
        if r.supports_tags() {
            let at = r.here();
            let found = r.read_tag()?;
            if found != self.tag {
                return Err(DecodeError::at(
                    Problem::mismatch(alloc::format!("tag {}", self.tag), alloc::format!("tag {found}")),
                    at,
                ));
            }
        }
        decode_child(&self.inner, r, cx)
    }
}

impl<C: Encoder> Encoder for Tagged<C> {
    type Input = C::Input;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &C::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.write_tag(self.tag)?;
        self.inner.encode(v, s, cx)
    }
}

// -- Option / Box / Spanned ------------------------------------------------------

/// `null` ↔ `None`.
#[derive(Debug, Clone, Copy)]
pub struct Optional<C>(pub C);
pub const fn optional<C>(inner: C) -> Optional<C> {
    Optional(inner)
}

impl<C: Decoder> Decoder for Optional<C> {
    type Output = Option<C::Output>;
    #[inline]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
        if r.peek_kind()? == Kind::Null {
            r.read_null()?;
            Ok(None)
        } else {
            decode_child(&self.0, r, cx).map(Some)
        }
    }
}
impl<C: Encoder<Input: Sized>> Encoder for Optional<C> {
    type Input = Option<C::Input>;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        match v {
            None => s.write_null(),
            Some(x) => self.0.encode(x, s, cx),
        }
    }
}

/// `Box<T>`, for recursive enums.
#[derive(Debug, Clone, Copy)]
pub struct Boxed<C>(pub C);
pub const fn boxed<C>(inner: C) -> Boxed<C> {
    Boxed(inner)
}

impl<C: Decoder> Decoder for Boxed<C> {
    type Output = Box<C::Output>;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
        decode_child(&self.0, r, cx).map(Box::new)
    }
}
impl<C: Encoder<Input: Sized>> Encoder for Boxed<C> {
    type Input = Box<C::Input>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.0.encode(v, s, cx)
    }
}

/// Keep the decoded value's byte span. Encoding ignores the span.
#[derive(Debug, Clone, Copy)]
pub struct SpannedCodec<C>(pub C);
pub const fn spanned<C>(inner: C) -> SpannedCodec<C> {
    SpannedCodec(inner)
}

impl<C: Decoder> Decoder for SpannedCodec<C> {
    type Output = Spanned<C::Output>;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
        let start = r.here();
        let value = decode_child(&self.0, r, cx)?;
        let span = start.and_then(|s| {
            let loc = r.since(&s);
            r.byte_span(&loc)
        });
        Ok(Spanned { value, span })
    }
}
impl<C: Encoder<Input: Sized>> Encoder for SpannedCodec<C> {
    type Input = Spanned<C::Input>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.0.encode(&v.value, s, cx)
    }
}

// -- Mapping -----------------------------------------------------------------

/// A total isomorphism: `to` decodes, `from` encodes.
#[derive(Debug, Clone, Copy)]
pub struct BiMap<C, F, G> {
    inner: C,
    to: F,
    from: G,
}

/// Wrap a codec in a total isomorphism (newtypes).
pub fn bimap<C, F, G, B>(inner: C, to: F, from: G) -> BiMap<C, F, G>
where
    C: Codec<Output: Sized>,
    F: Fn(C::Output) -> B,
    G: Fn(&B) -> C::Output,
{
    BiMap { inner, to, from }
}

impl<C: Codec<Output: Sized>, F: Fn(C::Output) -> B, G: Fn(&B) -> C::Output, B> Decoder
    for BiMap<C, F, G>
{
    type Output = B;
    #[inline]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<B, R> {
        decode_child(&self.inner, r, cx).map(&self.to)
    }
}
impl<C: Codec<Output: Sized>, F: Fn(C::Output) -> B, G: Fn(&B) -> C::Output, B> Encoder
    for BiMap<C, F, G>
{
    type Input = B;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &B,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.inner.encode(&(self.from)(v), s, cx)
    }
}

/// A fallible decode with a total encode. The `Err` message becomes a
/// `ValidationFailed` at the offending value's location.
#[derive(Debug, Clone, Copy)]
pub struct TryBiMap<C, F, G> {
    inner: C,
    to: F,
    from: G,
}

pub fn try_bimap<C, F, G, B, E>(inner: C, to: F, from: G) -> TryBiMap<C, F, G>
where
    C: Codec<Output: Sized>,
    F: Fn(C::Output) -> Result<B, E>,
    E: Display,
    G: Fn(&B) -> C::Output,
{
    TryBiMap { inner, to, from }
}

impl<C, F, G, B, E> Decoder for TryBiMap<C, F, G>
where
    C: Codec<Output: Sized>,
    F: Fn(C::Output) -> Result<B, E>,
    E: Display,
    G: Fn(&B) -> C::Output,
{
    type Output = B;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<B, R> {
        let start = r.here();
        let a = decode_child(&self.inner, r, cx)?;
        (self.to)(a).map_err(|e| {
            let loc = start.map(|s| r.since(&s));
            DecodeError::at(Problem::ValidationFailed(e.to_string()), loc)
        })
    }
}
impl<C, F, G, B, E> Encoder for TryBiMap<C, F, G>
where
    C: Codec<Output: Sized>,
    F: Fn(C::Output) -> Result<B, E>,
    E: Display,
    G: Fn(&B) -> C::Output,
{
    type Input = B;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &B,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.inner.encode(&(self.from)(v), s, cx)
    }
}

/// Validate without changing the type: `refine(u16(), |p| ...)`.
#[derive(Debug, Clone, Copy)]
pub struct Refine<C, V> {
    inner: C,
    validate: V,
}

pub fn refine<C, V, E>(inner: C, validate: V) -> Refine<C, V>
where
    C: Codec<Output: Sized>,
    V: Fn(&C::Output) -> Result<(), E>,
    E: Display,
{
    Refine { inner, validate }
}

impl<C: Codec<Output: Sized>, V: Fn(&C::Output) -> Result<(), E>, E: Display> Decoder
    for Refine<C, V>
{
    type Output = C::Output;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<C::Output, R> {
        let start = r.here();
        let a = decode_child(&self.inner, r, cx)?;
        match (self.validate)(&a) {
            Ok(()) => Ok(a),
            Err(e) => {
                let loc = start.map(|s| r.since(&s));
                Err(DecodeError::at(
                    Problem::ValidationFailed(e.to_string()),
                    loc,
                ))
            }
        }
    }
}
impl<C: Codec<Output: Sized>, V: Fn(&C::Output) -> Result<(), E>, E: Display> Encoder
    for Refine<C, V>
{
    type Input = C::Output;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &C::Output,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.inner.encode(v, s, cx)
    }
}

/// Attach a context frame to errors from the inner codec.
#[derive(Debug, Clone)]
pub struct Context<C> {
    inner: C,
    description: Cow<'static, str>,
}

/// Errors inside `inner` get a "while decoding …" context frame.
pub fn context<C>(description: impl Into<Cow<'static, str>>, inner: C) -> Context<C> {
    Context {
        inner,
        description: description.into(),
    }
}

impl<C: Decoder> Decoder for Context<C> {
    type Output = C::Output;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<C::Output, R> {
        let start = r.here();
        decode_child(&self.inner, r, cx).map_err(|e| {
            e.push_context(ContextFrame {
                location: start,
                description: self.description.clone(),
            })
        })
    }
}
impl<C: Encoder> Encoder for Context<C> {
    type Input = C::Input;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &C::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.inner.encode(v, s, cx)
    }
}

// -- Decoder-only combinators --------------------------------------------------

/// Decoder-only map (legacy versions, normalising decoders).
#[derive(Debug, Clone, Copy)]
pub struct Map<D, F>(D, F);
pub fn map<D: Decoder, F: Fn(D::Output) -> B, B>(decoder: D, f: F) -> Map<D, F> {
    Map(decoder, f)
}
impl<D: Decoder, F: Fn(D::Output) -> B, B> Decoder for Map<D, F> {
    type Output = B;
    #[inline]
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<B, R> {
        decode_child(&self.0, r, cx).map(&self.1)
    }
}

/// Decoder-only fallible map.
#[derive(Debug, Clone, Copy)]
pub struct TryMap<D, F>(D, F);
pub fn try_map<D: Decoder, F: Fn(D::Output) -> Result<B, E>, B, E: Display>(
    decoder: D,
    f: F,
) -> TryMap<D, F> {
    TryMap(decoder, f)
}
impl<D: Decoder, F: Fn(D::Output) -> Result<B, E>, B, E: Display> Decoder for TryMap<D, F> {
    type Output = B;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<B, R> {
        let start = r.here();
        let a = decode_child(&self.0, r, cx)?;
        (self.1)(a).map_err(|e| {
            let loc = start.map(|s| r.since(&s));
            DecodeError::at(Problem::ValidationFailed(e.to_string()), loc)
        })
    }
}

/// Decoder-only: choose the next decoder from a decoded value.
#[derive(Debug, Clone, Copy)]
pub struct Then<D, F>(D, F);
pub fn then<D: Decoder, F: Fn(D::Output) -> D2, D2: Decoder>(decoder: D, f: F) -> Then<D, F> {
    Then(decoder, f)
}
impl<D: Decoder, F: Fn(D::Output) -> D2, D2: Decoder> Decoder for Then<D, F> {
    type Output = D2::Output;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<D2::Output, R> {
        let a = decode_child(&self.0, r, cx)?;
        decode_child(&(self.1)(a), r, cx)
    }
}

/// Decoder-only: produce a value without reading input.
#[derive(Debug, Clone, Copy)]
pub struct Success<T>(T);
pub fn success<T: Clone>(value: T) -> Success<T> {
    Success(value)
}
impl<T: Clone> Decoder for Success<T> {
    type Output = T;
    fn decode<R: Reader + ?Sized>(&self, _r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<T, R> {
        Ok(self.0.clone())
    }
}

/// Decoder-only: always fail with `problem`.
#[derive(Debug, Clone)]
pub struct Fail<T>(Problem, PhantomData<fn() -> T>);
pub fn fail<T>(problem: Problem) -> Fail<T> {
    Fail(problem, PhantomData)
}
impl<T> Decoder for Fail<T> {
    type Output = T;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<T, R> {
        Err(DecodeError::at(self.0.clone(), r.here()))
    }
}

// -- string enums ----------------------------------------------------------------

/// A fieldless enum as one of a fixed set of strings:
/// `string_enum([("red", Color::Red), ("green", Color::Green)])`.
#[derive(Debug, Clone, Copy)]
pub struct StringEnum<T, const N: usize>([(&'static str, T); N]);

pub fn string_enum<T: Clone + PartialEq, const N: usize>(
    variants: [(&'static str, T); N],
) -> StringEnum<T, N> {
    StringEnum(variants)
}

impl<T: Clone + PartialEq, const N: usize> Decoder for StringEnum<T, N> {
    type Output = T;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, _cx: &mut DecodeContext) -> DecodeResult<T, R> {
        let at = r.here();
        let mut s = String::new();
        r.read_str(&mut s)?;
        match self.0.iter().find(|(name, _)| *name == s) {
            Some((_, v)) => Ok(v.clone()),
            None => Err(DecodeError::at(
                Problem::UnknownVariant {
                    name: s,
                    expected: self.0.iter().map(|(n, _)| *n).collect(),
                },
                at.map(|a| r.since(&a)),
            )),
        }
    }
}

impl<T: Clone + PartialEq, const N: usize> Encoder for StringEnum<T, N> {
    type Input = T;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &T,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        let name = self
            .0
            .iter()
            .find(|(_, x)| x == v)
            .map(|(n, _)| *n)
            .expect("every value is listed in string_enum");
        s.write_str(name)
    }
}
