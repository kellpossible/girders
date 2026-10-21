//! Sum types: internally and adjacently tagged unions, and `one_of`.
//!
//! Variants are listed for decoding; encoding projects the value with an
//! exhaustive `match` returning a positional `PickN::VariantK(&payload)`,
//! so rustc checks both exhaustiveness and each payload's type. Payloads
//! are borrowed from the value: a variant with several named fields needs
//! them in a struct of its own (`Rectangle(Rectangle)`).

use crate::codec::{Codec, DecodeResult, Decoder, Encoder, decode_child};
use crate::context::{DecodeContext, EncodeContext, TagValue};
use crate::error::{DecodeError, PathSegment, Problem};
use crate::reader::Reader;
use crate::record::expect_object;
use crate::serializer::Serializer;
use alloc::string::String;
use alloc::vec::Vec;

/// Find `tag` in the object at the cursor and read its value with
/// `read`, without consuming anything: scan under a checkpoint, rewind.
pub(crate) fn scan_tag<R: Reader + ?Sized, T>(
    r: &mut R,
    cx: &DecodeContext,
    tag: &'static str,
    read: impl FnOnce(&mut R) -> DecodeResult<T, R>,
) -> DecodeResult<Option<T>, R> {
    expect_object(r)?;
    let cp = r.checkpoint();
    let result = (|| {
        r.begin_object()?;
        let mut key = String::new();
        let mut first = true;
        while r.object_next_key(first, &mut key)? {
            first = false;
            if key == tag {
                return read(r).map(Some).map_err(|e| {
                    if cx.options.track_path {
                        e.push_path(PathSegment::Key(tag.into()))
                    } else {
                        e
                    }
                });
            }
            r.skip_value()?;
            key.clear();
        }
        Ok(None)
    })();
    r.rewind(&cp);
    r.release(cp);
    result
}

/// One variant: its tag, payload codec, and constructor.
#[derive(Debug, Clone, Copy)]
pub struct Variant<C, K> {
    pub tag: &'static str,
    pub codec: C,
    pub ctor: K,
}

pub fn variant<C, K, T>(tag: &'static str, codec: C, ctor: K) -> Variant<C, K>
where
    C: Codec<Output: Sized>,
    K: Fn(C::Output) -> T,
{
    Variant { tag, codec, ctor }
}

/// How the tag is written when encoding.
#[derive(Debug, Clone, Copy)]
pub enum TagStyle {
    Internal {
        tag: &'static str,
    },
    Adjacent {
        tag: &'static str,
        content: &'static str,
    },
}

/// A list of variants (a tuple of [`Variant`]s).
pub trait Variants {
    type Output;
    type Pick<'a>
    where
        Self: 'a;
    fn decode_variant<R: Reader + ?Sized>(
        &self,
        tag: &str,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> Option<DecodeResult<Self::Output, R>>;
    fn tags(&self) -> Vec<&'static str>;
    fn encode_pick<'a, S: Serializer + ?Sized>(
        &'a self,
        pick: Self::Pick<'a>,
        style: TagStyle,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error>;
}

#[inline]
fn encode_variant<C: Encoder, S: Serializer + ?Sized>(
    tag_value: &'static str,
    codec: &C,
    v: &C::Input,
    style: TagStyle,
    s: &mut S,
    cx: &mut EncodeContext,
) -> Result<(), S::Error> {
    match style {
        TagStyle::Internal { tag } => {
            cx.pending_tag = Some((tag, TagValue::Str(tag_value)));
            let r = codec.encode(v, s, cx);
            cx.pending_tag = None;
            r
        }
        TagStyle::Adjacent { tag, content } => {
            s.begin_object(Some(2))?;
            s.object_key(tag)?;
            s.write_str(tag_value)?;
            s.object_key(content)?;
            codec.encode(v, s, cx)?;
            s.end_object()
        }
    }
}

macro_rules! variants {
    ($(#[$m:meta])* $Enum:ident; $($C:ident $K:ident $V:ident $i:tt),+) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub enum $Enum<$($C),+> {
            $($V($C)),+
        }

        impl<T, $($C: Codec<Output: Sized>, $K: Fn($C::Output) -> T),+> Variants for ($(Variant<$C, $K>,)+) {
            type Output = T;
            type Pick<'a> = $Enum<$(&'a $C::Output),+> where Self: 'a;

            #[inline]
            fn decode_variant<R: Reader + ?Sized>(&self, tag: &str, r: &mut R, cx: &mut DecodeContext) -> Option<DecodeResult<T, R>> {
                $(if tag == self.$i.tag {
                    return Some(decode_child(&self.$i.codec, r, cx).map(&self.$i.ctor));
                })+
                None
            }
            fn tags(&self) -> Vec<&'static str> {
                alloc::vec![$(self.$i.tag),+]
            }
            #[inline]
            fn encode_pick<'a, S: Serializer + ?Sized>(
                &'a self,
                pick: Self::Pick<'a>,
                style: TagStyle,
                s: &mut S,
                cx: &mut EncodeContext,
            ) -> Result<(), S::Error> {
                match pick {
                    $($Enum::$V(v) => encode_variant(self.$i.tag, &self.$i.codec, v, style, s, cx),)+
                }
            }
        }
    };
}

variants!(
    /// Positional pick for a 1-variant union.
    Pick1; C0 K0 Variant0 0);
variants!(
    /// Positional pick for a 2-variant union: `Variant0` is the first listed.
    Pick2; C0 K0 Variant0 0, C1 K1 Variant1 1);
variants!(Pick3; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2);
variants!(Pick4; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3);
variants!(Pick5; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4);
variants!(Pick6; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5);
variants!(Pick7; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5, C6 K6 Variant6 6);
variants!(Pick8; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5, C6 K6 Variant6 6, C7 K7 Variant7 7);
variants!(Pick9; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5, C6 K6 Variant6 6, C7 K7 Variant7 7, C8 K8 Variant8 8);
variants!(Of10; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5, C6 K6 Variant6 6, C7 K7 Variant7 7, C8 K8 Variant8 8, C9 K9 Variant9 9);
variants!(Of11; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5, C6 K6 Variant6 6, C7 K7 Variant7 7, C8 K8 Variant8 8, C9 K9 Variant9 9, C10 K10 V10 10);
variants!(Of12; C0 K0 Variant0 0, C1 K1 Variant1 1, C2 K2 Variant2 2, C3 K3 Variant3 3, C4 K4 Variant4 4, C5 K5 Variant5 5, C6 K6 Variant6 6, C7 K7 Variant7 7, C8 K8 Variant8 8, C9 K9 Variant9 9, C10 K10 V10 10, C11 K11 V11 11);

fn unknown_variant<L>(
    tag_key: &'static str,
    found: String,
    expected: Vec<&'static str>,
    cx: &DecodeContext,
    at: Option<L>,
) -> DecodeError<L> {
    let e = DecodeError::at(
        Problem::UnknownVariant {
            name: found,
            expected,
        },
        at,
    );
    if cx.options.track_path {
        e.push_path(PathSegment::Key(tag_key.into()))
    } else {
        e
    }
}

// -- internally tagged ------------------------------------------------------

/// `{"type": "circle", "radius": 2.0}`. The tag may appear anywhere in
/// the object; the variant's record never sees it.
#[derive(Debug, Clone, Copy)]
pub struct TaggedUnion<Vs, P> {
    tag: &'static str,
    variants: Vs,
    project: P,
}

pub fn tagged_union<Vs, P>(tag: &'static str, variants: Vs, project: P) -> TaggedUnion<Vs, P>
where
    Vs: Variants,
    P: for<'a> Fn(&'a Vs::Output) -> Vs::Pick<'a>,
{
    TaggedUnion {
        tag,
        variants,
        project,
    }
}

fn read_tag<R: Reader + ?Sized>(r: &mut R) -> DecodeResult<(String, Option<R::Location>), R> {
    let at = r.here();
    let mut s = String::new();
    r.read_str(&mut s)?;
    Ok((s, at))
}

impl<Vs: Variants, P: for<'a> Fn(&'a Vs::Output) -> Vs::Pick<'a>> Decoder for TaggedUnion<Vs, P> {
    type Output = Vs::Output;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<Vs::Output, R> {
        let at = r.here();
        let (tag, tag_at) = scan_tag(r, cx, self.tag, read_tag)?
            .ok_or_else(|| DecodeError::at(Problem::MissingField(self.tag.into()), at))?;
        let key = self.tag;
        match cx.with_claim(key, |cx| self.variants.decode_variant(&tag, r, cx)) {
            Some(result) => result,
            None => Err(unknown_variant(
                self.tag,
                tag,
                self.variants.tags(),
                cx,
                tag_at,
            )),
        }
    }
}

impl<Vs: Variants, P: for<'a> Fn(&'a Vs::Output) -> Vs::Pick<'a>> Encoder for TaggedUnion<Vs, P> {
    type Input = Vs::Output;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Vs::Output,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.variants.encode_pick(
            (self.project)(v),
            TagStyle::Internal { tag: self.tag },
            s,
            cx,
        )
    }
}

// -- adjacently tagged ------------------------------------------------------

/// `{"kind": "circle", "data": {"radius": 2.0}}`. Keys in any order.
#[derive(Debug, Clone, Copy)]
pub struct AdjacentUnion<Vs, P> {
    tag: &'static str,
    content: &'static str,
    variants: Vs,
    project: P,
}

pub fn adjacent_union<Vs, P>(
    tag: &'static str,
    content: &'static str,
    variants: Vs,
    project: P,
) -> AdjacentUnion<Vs, P>
where
    Vs: Variants,
    P: for<'a> Fn(&'a Vs::Output) -> Vs::Pick<'a>,
{
    AdjacentUnion {
        tag,
        content,
        variants,
        project,
    }
}

impl<Vs: Variants, P: for<'a> Fn(&'a Vs::Output) -> Vs::Pick<'a>> Decoder for AdjacentUnion<Vs, P> {
    type Output = Vs::Output;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<Vs::Output, R> {
        let at = r.here();
        let (tag, tag_at) = scan_tag(r, cx, self.tag, read_tag)?
            .ok_or_else(|| DecodeError::at(Problem::MissingField(self.tag.into()), at.clone()))?;
        r.begin_object()?;
        cx.nested(|cx| {
            let mut out = None;
            let mut key = String::new();
            let mut first = true;
            while r.object_next_key(first, &mut key)? {
                first = false;
                if key == self.tag {
                    r.skip_value()?;
                } else if key == self.content {
                    if out.is_some() {
                        return Err(DecodeError::at(
                            Problem::DuplicateField(key.clone()),
                            r.here(),
                        ));
                    }
                    let res = match self.variants.decode_variant(&tag, r, cx) {
                        Some(res) => res,
                        None => {
                            return Err(unknown_variant(
                                self.tag,
                                tag.clone(),
                                self.variants.tags(),
                                cx,
                                tag_at.clone(),
                            ));
                        }
                    };
                    out = Some(res.map_err(|e| {
                        if cx.options.track_path {
                            e.push_path(PathSegment::Key(self.content.into()))
                        } else {
                            e
                        }
                    })?);
                } else {
                    return Err(DecodeError::at(
                        Problem::UnknownField {
                            name: key.clone(),
                            expected: alloc::vec![self.tag, self.content],
                        },
                        r.here(),
                    ));
                }
                key.clear();
            }
            r.end_object()?;
            out.ok_or_else(|| DecodeError::at(Problem::MissingField(self.content.into()), at))
        })
    }
}

impl<Vs: Variants, P: for<'a> Fn(&'a Vs::Output) -> Vs::Pick<'a>> Encoder for AdjacentUnion<Vs, P> {
    type Input = Vs::Output;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Vs::Output,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.variants.encode_pick(
            (self.project)(v),
            TagStyle::Adjacent {
                tag: self.tag,
                content: self.content,
            },
            s,
            cx,
        )
    }
}

// -- fieldless variants -----------------------------------------------------

/// An object with no fields (besides a claimed tag): `()`. For fieldless
/// variants: `variant("point", empty_object(), |()| Shape::Point)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyObject;
pub const fn empty_object() -> EmptyObject {
    EmptyObject
}

impl Decoder for EmptyObject {
    type Output = ();
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<(), R> {
        let claimed = cx.take_claimed();
        r.begin_object()?;
        let mut key = String::new();
        let mut first = true;
        while r.object_next_key(first, &mut key)? {
            first = false;
            if claimed == Some(key.as_str()) {
                r.skip_value()?;
            } else {
                return Err(DecodeError::at(
                    Problem::UnknownField {
                        name: key.clone(),
                        expected: Vec::new(),
                    },
                    r.here(),
                ));
            }
            key.clear();
        }
        r.end_object()
    }
}

impl Encoder for EmptyObject {
    type Input = ();
    fn encode<S: Serializer + ?Sized>(
        &self,
        _v: &(),
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        let tag = cx.take_pending_tag();
        s.begin_object(Some(tag.is_some() as usize))?;
        if let Some((key, value)) = tag {
            s.object_key(key)?;
            match value {
                TagValue::Str(t) => s.write_str(t)?,
                TagValue::Int(i) => s.write_i64(i)?,
            }
        }
        s.end_object()
    }
}

// -- one_of -------------------------------------------------------------------

/// Decoder-only: try each decoder in turn (rewinding between attempts)
/// and report the deepest error if all fail. Fatal errors stop at once.
#[derive(Debug, Clone, Copy)]
pub struct OneOf<Ds>(Ds);
pub fn one_of<Ds>(decoders: Ds) -> OneOf<Ds> {
    OneOf(decoders)
}

fn deeper<L>(a: Option<DecodeError<L>>, b: DecodeError<L>) -> DecodeError<L> {
    match a {
        Some(a) if a.first().path_len() >= b.first().path_len() => a,
        _ => b,
    }
}

macro_rules! one_of_impl {
    ($D0:ident 0 $(, $D:ident $i:tt)*) => {
        impl<$D0: Decoder, $($D: Decoder<Output = $D0::Output>),*> Decoder for OneOf<($D0, $($D,)*)> {
            type Output = $D0::Output;
            fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
                let cp = r.checkpoint();
                let mut best: Option<DecodeError<R::Location>> = None;
                match decode_child(&(self.0).0, r, cx) {
                    Ok(v) => { r.release(cp); return Ok(v); }
                    Err(e) if e.is_fatal() => { r.release(cp); return Err(e); }
                    Err(e) => { best = Some(deeper(best, e)); r.rewind(&cp); }
                }
                $(match decode_child(&(self.0).$i, r, cx) {
                    Ok(v) => { r.release(cp); return Ok(v); }
                    Err(e) if e.is_fatal() => { r.release(cp); return Err(e); }
                    Err(e) => { best = Some(deeper(best, e)); r.rewind(&cp); }
                })*
                r.release(cp);
                Err(best.expect("at least one decoder"))
            }
        }
    };
}
one_of_impl!(D0 0, D1 1);
one_of_impl!(D0 0, D1 1, D2 2);
one_of_impl!(D0 0, D1 1, D2 2, D3 3);
one_of_impl!(D0 0, D1 1, D2 2, D3 3, D4 4);
one_of_impl!(D0 0, D1 1, D2 2, D3 3, D4 4, D5 5);
one_of_impl!(D0 0, D1 1, D2 2, D3 3, D4 4, D5 5, D6 6);
one_of_impl!(D0 0, D1 1, D2 2, D3 3, D4 4, D5 5, D6 6, D7 7);
