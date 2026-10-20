//! Tag-dispatched codecs: tagged unions and versioning. Both scan the
//! object for the tag under a checkpoint, rewind, then decode the body
//! with the tag key "claimed" so a strict record skips it.

use crate::core::*;
#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec, vec::Vec};


/// Find `tag` in the object at the cursor without consuming anything.
/// `read` reads the tag's value.
fn scan_tag<R: Reader, T>(
    r: &mut R,
    tag: &str,
    read: impl Fn(&mut R) -> Res<T, R>,
) -> Res<Option<T>, R> {
    let cp = r.checkpoint();
    let result = (|| {
        r.begin_object()?;
        let mut key = String::new();
        let mut first = true;
        loop {
            key.clear();
            if !r.object_next_key(first, &mut key)? {
                return Ok(None);
            }
            first = false;
            if key == tag {
                return read(r).map(Some).map_err(|e| e.push(Seg::Key(key.clone())));
            }
            r.skip_value()?;
        }
    })();
    r.rewind(&cp);
    r.release(cp);
    result
}

// -- Tagged union -------------------------------------------------------------

pub struct Variant<C, K> {
    tag: &'static str,
    codec: C,
    ctor: K,
}

pub fn variant<C, K, T>(tag: &'static str, codec: C, ctor: K) -> Variant<C, K>
where
    C: Codec,
    K: Fn(C::Output) -> T,
{
    Variant { tag, codec, ctor }
}

/// Positional sum used by the encode projection: `V0` is the first
/// variant in the list, and so on. The projection's `match` is checked
/// for exhaustiveness by rustc, and each arm's payload type must match
/// that variant's codec.
pub enum Of2<A, B> {
    V0(A),
    V1(B),
}
pub enum Of3<A, B, C> {
    V0(A),
    V1(B),
    V2(C),
}

pub trait Variants {
    type Out;
    type Pick<'a>
    where
        Self: 'a;
    fn decode_tag<R: Reader>(&self, tag: &str, r: &mut R, cx: &mut Cx) -> Option<Res<Self::Out, R>>;
    fn tags(&self) -> Vec<&'static str>;
    fn encode_pick<'a, S: Serializer>(
        &'a self,
        pick: Self::Pick<'a>,
        tag_key: &'static str,
        s: &mut S,
        cx: &mut Ecx,
    ) -> Result<(), S::Error>;
    fn schemas(&self, s: &mut SchemaCx) -> Vec<(String, Schema)>;
}

macro_rules! variants_impl {
    ($Of:ident; $($C:ident $K:ident $V:ident $i:tt),+) => {
        impl<T, $($C: Codec, $K: Fn($C::Output) -> T),+> Variants for ($(Variant<$C, $K>,)+)
        where $($C::Output: Sized),+
        {
            type Out = T;
            type Pick<'a> = $Of<$(&'a $C::Output),+> where Self: 'a;

            fn decode_tag<R: Reader>(&self, tag: &str, r: &mut R, cx: &mut Cx) -> Option<Res<T, R>> {
                $(if tag == self.$i.tag {
                    return Some(self.$i.codec.decode(r, cx).map(&self.$i.ctor));
                })+
                None
            }
            fn tags(&self) -> Vec<&'static str> {
                vec![$(self.$i.tag),+]
            }
            fn encode_pick<'p, S: Serializer>(
                &'p self,
                pick: Self::Pick<'p>,
                tag_key: &'static str,
                s: &mut S,
                cx: &mut Ecx,
            ) -> Result<(), S::Error> {
                match pick {
                    $($Of::$V(v) => {
                        cx.pending_tag = Some((tag_key, TagValue::Str(self.$i.tag)));
                        self.$i.codec.encode(v, s, cx)
                    })+
                }
            }
            fn schemas(&self, s: &mut SchemaCx) -> Vec<(String, Schema)> {
                vec![$((self.$i.tag.to_string(), self.$i.codec.schema(s))),+]
            }
        }
    };
}
variants_impl!(Of2; C0 K0 V0 0, C1 K1 V1 1);
variants_impl!(Of3; C0 K0 V0 0, C1 K1 V1 1, C2 K2 V2 2);

pub struct TaggedUnion<Vs, P> {
    tag: &'static str,
    variants: Vs,
    project: P,
}

pub fn tagged_union<Vs, P>(tag: &'static str, variants: Vs, project: P) -> TaggedUnion<Vs, P>
where
    Vs: Variants,
    P: for<'a> Fn(&'a Vs::Out) -> Vs::Pick<'a>,
{
    TaggedUnion { tag, variants, project }
}

impl<Vs, P> Decoder for TaggedUnion<Vs, P>
where
    Vs: Variants,
    P: for<'a> Fn(&'a Vs::Out) -> Vs::Pick<'a>,
{
    type Output = Vs::Out;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Vs::Out, R> {
        let tag = scan_tag(r, self.tag, |r| {
            let mut s = String::new();
            r.read_str(&mut s)?;
            Ok(s)
        })?
        .ok_or_else(|| Error::new(Problem::MissingField(self.tag)))?;
        cx.claimed = Some(self.tag);
        match self.variants.decode_tag(&tag, r, cx) {
            Some(result) => result,
            None => Err(Error::new(Problem::UnknownVariant(tag)).push(Seg::Key(self.tag.into()))),
        }
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        Schema::Union { tag: self.tag, variants: self.variants.schemas(s) }
    }
}

impl<Vs, P> Encoder for TaggedUnion<Vs, P>
where
    Vs: Variants,
    P: for<'a> Fn(&'a Vs::Out) -> Vs::Pick<'a>,
{
    type Input = Vs::Out;
    fn encode<S: Serializer>(&self, v: &Vs::Out, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        self.variants.encode_pick((self.project)(v), self.tag, s, cx)
    }
}

// -- Versioned ----------------------------------------------------------------

/// Type-level list of legacy decoders: `()` or `(Legacy<D>, Rest)`. Each
/// entry keeps its own concrete decoder type, so no boxing and no fixed
/// reader type are needed.
pub trait LegacyList<Out> {
    fn decode_version<R: Reader>(&self, v: i64, r: &mut R, cx: &mut Cx) -> Option<Res<Out, R>>;
    fn versions(&self, out: &mut Vec<i64>);
}

impl<Out> LegacyList<Out> for () {
    fn decode_version<R: Reader>(&self, _: i64, _: &mut R, _: &mut Cx) -> Option<Res<Out, R>> {
        None
    }
    fn versions(&self, _: &mut Vec<i64>) {}
}

pub struct Legacy<D> {
    version: i64,
    decoder: D,
}

impl<Out, D, Rest> LegacyList<Out> for (Legacy<D>, Rest)
where
    D: Decoder<Output = Out>,
    Rest: LegacyList<Out>,
{
    fn decode_version<R: Reader>(&self, v: i64, r: &mut R, cx: &mut Cx) -> Option<Res<Out, R>> {
        if v == self.0.version {
            Some(self.0.decoder.decode(r, cx))
        } else {
            self.1.decode_version(v, r, cx)
        }
    }
    fn versions(&self, out: &mut Vec<i64>) {
        out.push(self.0.version);
        self.1.versions(out);
    }
}

pub struct Versioned<C, L> {
    tag: &'static str,
    latest_version: i64,
    latest: C,
    default_version: Option<i64>,
    legacy: L,
}

pub fn versioned<C: Codec>(tag: &'static str, latest_version: i64, latest: C) -> Versioned<C, ()> {
    Versioned { tag, latest_version, latest, default_version: None, legacy: () }
}

impl<C: Codec, L: LegacyList<C::Output>> Versioned<C, L> {
    pub fn default_version(mut self, v: i64) -> Self {
        self.default_version = Some(v);
        self
    }
    pub fn legacy<D>(self, version: i64, decoder: D) -> Versioned<C, (Legacy<D>, L)>
    where
        D: Decoder<Output = C::Output>,
    {
        Versioned {
            tag: self.tag,
            latest_version: self.latest_version,
            latest: self.latest,
            default_version: self.default_version,
            legacy: (Legacy { version, decoder }, self.legacy),
        }
    }
}

impl<C: Codec, L: LegacyList<C::Output>> Decoder for Versioned<C, L> {
    type Output = C::Output;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<C::Output, R> {
        let v = scan_tag(r, self.tag, |r| r.read_i64())?
            .or(self.default_version)
            .ok_or_else(|| Error::new(Problem::MissingField(self.tag)))?;
        cx.claimed = Some(self.tag);
        if v == self.latest_version {
            return self.latest.decode(r, cx);
        }
        match self.legacy.decode_version(v, r, cx) {
            Some(result) => result,
            None => Err(Error::new(Problem::UnknownVersion(v)).push(Seg::Key(self.tag.into()))),
        }
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        self.latest.schema(s)
    }
}

impl<C: Codec, L: LegacyList<C::Output>> Encoder for Versioned<C, L> {
    type Input = C::Output;
    fn encode<S: Serializer>(&self, v: &C::Output, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        cx.pending_tag = Some((self.tag, TagValue::Int(self.latest_version)));
        self.latest.encode(v, s, cx)
    }
}
