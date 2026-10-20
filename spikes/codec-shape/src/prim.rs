//! Primitives and simple structural combinators.

use crate::core::*;
#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec, vec::Vec};


pub struct Str;
pub fn string() -> Str {
    Str
}
impl Decoder for Str {
    type Output = String;
    fn decode<R: Reader>(&self, r: &mut R, _cx: &mut Cx) -> Res<String, R> {
        let mut s = String::new();
        r.read_str(&mut s)?;
        Ok(s)
    }
    fn schema(&self, _s: &mut SchemaCx) -> Schema {
        Schema::String
    }
}
impl Encoder for Str {
    type Input = String;
    fn encode<S: Serializer>(&self, v: &String, s: &mut S, _cx: &mut Ecx) -> Result<(), S::Error> {
        s.write_str(v)
    }
}

/// Integer codecs, range-checked against i64 reads.
macro_rules! int_codec {
    ($name:ident, $fn:ident, $t:ty) => {
        pub struct $name;
        pub fn $fn() -> $name {
            $name
        }
        impl Decoder for $name {
            type Output = $t;
            fn decode<R: Reader>(&self, r: &mut R, _cx: &mut Cx) -> Res<$t, R> {
                let i = r.read_i64()?;
                <$t>::try_from(i).map_err(|_| Error::new(Problem::OutOfRange))
            }
            fn schema(&self, _s: &mut SchemaCx) -> Schema {
                Schema::Integer { min: <$t>::MIN as i64, max: <$t>::MAX as i64 }
            }
        }
        impl Encoder for $name {
            type Input = $t;
            fn encode<S: Serializer>(&self, v: &$t, s: &mut S, _cx: &mut Ecx) -> Result<(), S::Error> {
                s.write_i64(*v as i64)
            }
        }
    };
}
int_codec!(I64, i64, i64);
int_codec!(U32, u32, u32);
int_codec!(U8, u8, u8);

pub struct List<C>(C);
pub fn list<C>(c: C) -> List<C> {
    List(c)
}
impl<C: Decoder> Decoder for List<C> {
    type Output = Vec<C::Output>;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Self::Output, R> {
        r.begin_array()?;
        let mut out = Vec::new();
        let mut first = true;
        while r.array_has_next(first)? {
            first = false;
            let i = out.len();
            out.push(self.0.decode(r, cx).map_err(|e| e.push(Seg::Index(i)))?);
        }
        r.end_array()?;
        Ok(out)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        Schema::Array(Box::new(self.0.schema(s)))
    }
}
impl<C: Encoder> Encoder for List<C>
where
    C::Input: Sized,
{
    type Input = Vec<C::Input>;
    fn encode<S: Serializer>(&self, v: &Self::Input, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        s.begin_array(Some(v.len()))?;
        for x in v {
            s.array_element()?;
            self.0.encode(x, s, cx)?;
        }
        s.end_array()
    }
}

pub struct Optional<C>(C);
pub fn optional<C>(c: C) -> Optional<C> {
    Optional(c)
}
impl<C: Decoder> Decoder for Optional<C> {
    type Output = Option<C::Output>;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Self::Output, R> {
        if r.peek_kind()? == Kind::Null {
            r.read_null()?;
            Ok(None)
        } else {
            self.0.decode(r, cx).map(Some)
        }
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        Schema::Nullable(Box::new(self.0.schema(s)))
    }
}
impl<C: Encoder> Encoder for Optional<C>
where
    C::Input: Sized,
{
    type Input = Option<C::Input>;
    fn encode<S: Serializer>(&self, v: &Self::Input, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        match v {
            None => s.write_null(),
            Some(x) => self.0.encode(x, s, cx),
        }
    }
}

/// Bidirectional map. `to` decodes, `from` encodes (owned intermediate).
pub struct BiMap<C, F, G> {
    c: C,
    to: F,
    from: G,
}
pub fn bimap<C, F, G, B>(c: C, to: F, from: G) -> BiMap<C, F, G>
where
    C: Codec,
    F: Fn(C::Output) -> B,
    G: Fn(&B) -> C::Output,
{
    BiMap { c, to, from }
}
impl<C: Codec, F, G, B> Decoder for BiMap<C, F, G>
where
    F: Fn(C::Output) -> B,
    G: Fn(&B) -> C::Output,
{
    type Output = B;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<B, R> {
        self.c.decode(r, cx).map(&self.to)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        self.c.schema(s)
    }
}
impl<C: Codec, F, G, B> Encoder for BiMap<C, F, G>
where
    F: Fn(C::Output) -> B,
    G: Fn(&B) -> C::Output,
{
    type Input = B;
    fn encode<S: Serializer>(&self, v: &B, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        self.c.encode(&(self.from)(v), s, cx)
    }
}

/// Decoder-only map, used for legacy versions.
pub struct Map<D, F>(D, F);
pub fn map<D: Decoder, F: Fn(D::Output) -> B, B>(d: D, f: F) -> Map<D, F> {
    Map(d, f)
}
impl<D: Decoder, F: Fn(D::Output) -> B, B> Decoder for Map<D, F> {
    type Output = B;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<B, R> {
        self.0.decode(r, cx).map(&self.1)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        self.0.schema(s)
    }
}

/// Schema naming for recursive positions: emits `Ref(name)` and registers
/// the body once. Decoding and encoding pass straight through. Recursion
/// itself needs a nominal codec type (see tests/spike.rs `TreeCodec`).
pub struct Named<C> {
    name: &'static str,
    c: C,
}
pub fn named<C: Codec>(name: &'static str, c: C) -> Named<C> {
    Named { name, c }
}
impl<C: Decoder> Decoder for Named<C> {
    type Output = C::Output;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Self::Output, R> {
        self.c.decode(r, cx)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        if !s.defs.iter().any(|(n, _)| *n == self.name) {
            s.defs.push((self.name, None));
            let body = self.c.schema(s);
            for d in s.defs.iter_mut() {
                if d.0 == self.name {
                    d.1 = Some(body.clone());
                }
            }
        }
        Schema::Ref(self.name)
    }
}
impl<C: Encoder> Encoder for Named<C> {
    type Input = C::Input;
    fn encode<S: Serializer>(&self, v: &Self::Input, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        self.c.encode(v, s, cx)
    }
}

/// Macro-free recursion (option A). The user implements this on a unit
/// (or config-carrying) struct; `recursive(def)` is the codec. Proving
/// `Rec<D>: Decoder` needs only `D: Recursive`, a nominal fact, so the
/// trait solver never walks the body's structure and no cycle arises.
pub trait Recursive {
    type Output;
    /// Schema name; recursive positions emit `Ref(NAME)`.
    const NAME: &'static str;
    fn codec(&self) -> impl Codec<Output = Self::Output>;
}

pub struct Rec<D>(D);

pub fn recursive<D: Recursive>(def: D) -> Rec<D> {
    Rec(def)
}

impl<D: Recursive> Decoder for Rec<D> {
    type Output = D::Output;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<D::Output, R> {
        self.0.codec().decode(r, cx)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        named(D::NAME, self.0.codec()).schema(s)
    }
}

impl<D: Recursive> Encoder for Rec<D> {
    type Input = D::Output;
    fn encode<S: Serializer>(&self, v: &D::Output, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        self.0.codec().encode(v, s, cx)
    }
}

/// `Box<T>` codec, needed by recursive enums.
pub struct Boxed<C>(C);
pub fn boxed<C: Codec>(c: C) -> Boxed<C> {
    Boxed(c)
}
impl<C: Decoder> Decoder for Boxed<C> {
    type Output = Box<C::Output>;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Self::Output, R> {
        self.0.decode(r, cx).map(Box::new)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        self.0.schema(s)
    }
}
impl<C: Encoder> Encoder for Boxed<C>
where
    C::Input: Sized,
{
    type Input = Box<C::Input>;
    fn encode<S: Serializer>(&self, v: &Box<C::Input>, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        self.0.encode(v, s, cx)
    }
}
