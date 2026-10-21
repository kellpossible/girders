//! Collection codecs: `list`, `array`, `btree_map` and tuples.

use crate::codec::{Codec, DecodeResult, Decoder, Encoder, decode_child};
use crate::context::{DecodeContext, EncodeContext};
use crate::error::{Collector, DecodeError, PathSegment, Problem};
use crate::reader::Reader;
use crate::record::Duplicates;
use crate::serializer::Serializer;
use alloc::collections::{BTreeMap, btree_map};
use alloc::string::String;
use alloc::vec::Vec;

/// Add a path segment to an error if path tracking is on.
#[inline]
pub(crate) fn at_seg<L>(
    e: DecodeError<L>,
    cx: &DecodeContext,
    seg: impl FnOnce() -> PathSegment,
) -> DecodeError<L> {
    if cx.options.track_path {
        e.push_path(seg())
    } else {
        e
    }
}

/// Decode one child value. In accumulate mode a non-fatal failure is
/// recorded in `errs`, the reader is rewound and the value skipped, and
/// `Ok(None)` is returned so the caller carries on.
#[inline]
pub(crate) fn child<R: Reader + ?Sized, T>(
    r: &mut R,
    cx: &mut DecodeContext,
    errs: &mut Collector<R::Location>,
    seg: impl FnOnce() -> PathSegment,
    decode: impl FnOnce(&mut R, &mut DecodeContext) -> DecodeResult<T, R>,
) -> DecodeResult<Option<T>, R> {
    // One call site for `decode`, so its (inlined) body isn't duplicated
    // between the fail-fast and accumulate paths; the accumulate
    // bookkeeping is out of line and generic only over the reader.
    let cp = if cx.options.accumulate {
        Some(r.checkpoint())
    } else {
        None
    };
    match (decode(r, cx), cp) {
        (Ok(v), None) => Ok(Some(v)),
        (Ok(v), Some(cp)) => {
            r.release(cp);
            Ok(Some(v))
        }
        (Err(e), None) => Err(at_seg(e, cx, seg)),
        (Err(e), Some(cp)) => {
            let e = at_seg(e, cx, seg);
            recover(r, errs, cp, e).map(|()| None)
        }
    }
}

/// A failed child in accumulate mode: record the error and skip the
/// value, unless it's fatal. Monomorphized once per reader type.
#[cold]
#[inline(never)]
fn recover<R: Reader + ?Sized>(
    r: &mut R,
    errs: &mut Collector<R::Location>,
    cp: R::Checkpoint,
    e: DecodeError<R::Location>,
) -> DecodeResult<(), R> {
    if e.is_fatal() {
        r.release(cp);
        return Err(e);
    }
    r.rewind(&cp);
    r.release(cp);
    r.skip_value()?;
    errs.push(e);
    Ok(())
}

// -- list ---------------------------------------------------------------------

/// `Vec<T>` as an array.
#[derive(Debug, Clone, Copy)]
pub struct List<C>(pub C);
pub const fn list<C>(inner: C) -> List<C> {
    List(inner)
}

impl<C: Decoder> Decoder for List<C> {
    type Output = Vec<C::Output>;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<Self::Output, R> {
        r.begin_array()?;
        let capacity = crate::reader::cautious_capacity::<C::Output>(r.size_hint());
        cx.nested(|cx| {
            let mut out = Vec::with_capacity(capacity);
            let mut errs = Collector::new();
            let mut first = true;
            let mut i = 0;
            while r.array_has_next(first)? {
                first = false;
                if let Some(v) = child(
                    r,
                    cx,
                    &mut errs,
                    || PathSegment::Index(i),
                    |r, cx| decode_child(&self.0, r, cx),
                )? {
                    out.push(v);
                }
                i += 1;
            }
            r.end_array()?;
            errs.finish()?;
            Ok(out)
        })
    }
}

impl<C: Encoder<Input: Sized>> Encoder for List<C> {
    type Input = Vec<C::Input>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.begin_array(Some(v.len()))?;
        for x in v {
            s.array_element()?;
            self.0.encode(x, s, cx)?;
        }
        s.end_array()
    }
}

// -- objects as maps ---------------------------------------------------------

// -- maps --------------------------------------------------------------------

/// A `BTreeMap<K, V>` as an object (JSON) or map (CBOR), with keys and
/// values through their own codecs: `btree_map(string(), user())`.
///
/// The format decides how a key looks: JSON keys are strings, so a `u32()`
/// key reads and writes `"42"`; CBOR keys are values, so the same codec
/// reads and writes the integer `42`. Duplicate keys are an error unless
/// [`MapCodec::duplicates`] says otherwise. Encoding is in key order.
#[derive(Debug, Clone, Copy)]
pub struct MapCodec<KC, VC> {
    key: KC,
    value: VC,
    duplicates: Duplicates,
}

impl<KC, VC> MapCodec<KC, VC> {
    /// What to do when a key repeats.
    pub fn duplicates(mut self, duplicates: Duplicates) -> Self {
        self.duplicates = duplicates;
        self
    }
}

pub const fn btree_map<KC, VC>(key: KC, value: VC) -> MapCodec<KC, VC> {
    MapCodec {
        key,
        value,
        duplicates: Duplicates::Error,
    }
}

impl<KC, VC> Decoder for MapCodec<KC, VC>
where
    KC: Codec<Output: Ord>,
    VC: Decoder,
{
    type Output = BTreeMap<KC::Output, VC::Output>;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<Self::Output, R> {
        r.begin_object()?;
        let out = cx.nested(|cx| {
            let mut out = BTreeMap::new();
            let mut errs = Collector::new();
            let mut first = true;
            while r.object_next_entry(first)? {
                first = false;
                let at = r.here();
                let Some(key) = self.decode_key(r, cx, &mut errs)? else {
                    continue;
                };
                // One lookup serves both the duplicate check and the insert.
                let slot = match out.entry(key) {
                    btree_map::Entry::Vacant(slot) => MapSlot::Vacant(slot),
                    btree_map::Entry::Occupied(slot) => match self.duplicates {
                        Duplicates::Error => {
                            let e = DecodeError::at(
                                Problem::DuplicateField(key_text(&self.key, slot.key())),
                                at,
                            );
                            if !cx.options.accumulate {
                                return Err(e);
                            }
                            errs.push(e);
                            r.skip_value()?;
                            continue;
                        }
                        Duplicates::FirstWins => {
                            r.skip_value()?;
                            continue;
                        }
                        Duplicates::LastWins => MapSlot::Occupied(slot),
                    },
                };
                let key = match &slot {
                    MapSlot::Vacant(s) => s.key(),
                    MapSlot::Occupied(s) => s.key(),
                };
                if let Some(v) = child(
                    r,
                    cx,
                    &mut errs,
                    || PathSegment::Key(key_text(&self.key, key)),
                    |r, cx| decode_child(&self.value, r, cx),
                )? {
                    match slot {
                        MapSlot::Vacant(s) => {
                            s.insert(v);
                        }
                        MapSlot::Occupied(mut s) => {
                            s.insert(v);
                        }
                    }
                }
            }
            errs.finish()?;
            Ok(out)
        })?;
        r.end_object()?;
        Ok(out)
    }
}

impl<KC: Codec<Output: Sized>, VC> MapCodec<KC, VC> {
    /// The entry's key; `None` when it failed in accumulate mode (the
    /// error is recorded and the whole entry skipped).
    fn decode_key<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
        errs: &mut Collector<R::Location>,
    ) -> DecodeResult<Option<KC::Output>, R> {
        if !cx.options.accumulate {
            return decode_child(&self.key, r, cx).map(Some);
        }
        let cp = r.checkpoint();
        match decode_child(&self.key, r, cx) {
            Ok(k) => {
                r.release(cp);
                Ok(Some(k))
            }
            Err(e) if e.is_fatal() => {
                r.release(cp);
                Err(e)
            }
            Err(e) => {
                r.rewind(&cp);
                r.release(cp);
                r.skip_value()?;
                r.skip_value()?;
                errs.push(e);
                Ok(None)
            }
        }
    }
}

impl<KC, VC> Encoder for MapCodec<KC, VC>
where
    KC: Codec<Output: Ord>,
    VC: Encoder<Input: Sized>,
{
    type Input = BTreeMap<KC::Output, VC::Input>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.begin_object(Some(v.len()))?;
        for (k, x) in v {
            s.object_key_value()?;
            self.key.encode(k, s, cx)?;
            self.value.encode(x, s, cx)?;
        }
        s.end_object()
    }
}

/// Where a decoded entry goes: a new key, or (under `LastWins`) an
/// existing one.
enum MapSlot<'a, K, V> {
    Vacant(btree_map::VacantEntry<'a, K, V>),
    Occupied(btree_map::OccupiedEntry<'a, K, V>),
}

/// A key as text, for error paths: encoded with its own codec into a
/// serializer that only collects text.
#[cold]
#[inline(never)]
fn key_text<KC: Encoder + ?Sized>(codec: &KC, key: &KC::Input) -> String {
    let mut text = KeyText(String::new());
    let _ = codec.encode(key, &mut text, &mut EncodeContext::new());
    text.0
}

/// Collects a key's text (see [`key_text`]).
struct KeyText(String);

impl Serializer for KeyText {
    type Error = core::convert::Infallible;
    fn write_bool(&mut self, v: bool) -> Result<(), Self::Error> {
        self.0.push_str(if v { "true" } else { "false" });
        Ok(())
    }
    fn write_null(&mut self) -> Result<(), Self::Error> {
        self.0.push_str("null");
        Ok(())
    }
    fn write_i64(&mut self, v: i64) -> Result<(), Self::Error> {
        self.0.push_str(&alloc::format!("{v}"));
        Ok(())
    }
    fn write_u64(&mut self, v: u64) -> Result<(), Self::Error> {
        self.0.push_str(&alloc::format!("{v}"));
        Ok(())
    }
    fn write_f64(&mut self, v: f64) -> Result<(), Self::Error> {
        self.0.push_str(&alloc::format!("{v:?}"));
        Ok(())
    }
    fn write_f32(&mut self, v: f32) -> Result<(), Self::Error> {
        self.0.push_str(&alloc::format!("{v:?}"));
        Ok(())
    }
    fn write_number_text(&mut self, v: &str) -> Result<(), Self::Error> {
        self.0.push_str(v);
        Ok(())
    }
    fn write_str(&mut self, v: &str) -> Result<(), Self::Error> {
        self.0.push_str(v);
        Ok(())
    }
    fn begin_array(&mut self, _len: Option<usize>) -> Result<(), Self::Error> {
        self.0.push('[');
        Ok(())
    }
    fn array_element(&mut self) -> Result<(), Self::Error> {
        if !self.0.ends_with('[') {
            self.0.push_str(", ");
        }
        Ok(())
    }
    fn end_array(&mut self) -> Result<(), Self::Error> {
        self.0.push(']');
        Ok(())
    }
    fn begin_object(&mut self, _len: Option<usize>) -> Result<(), Self::Error> {
        self.0.push_str("{…}");
        Ok(())
    }
    fn object_key(&mut self, _key: &str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn object_key_value(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn end_object(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

// -- fixed-length arrays -------------------------------------------------------

/// `[T; N]`: an array of exactly `N` elements. `N` is usually inferred
/// from the field's type: `field("rgb", array(u8()), |c: &Color| &c.rgb)`.
#[derive(Debug, Clone, Copy)]
pub struct ArrayCodec<C, const N: usize>(C);

pub const fn array<C, const N: usize>(element: C) -> ArrayCodec<C, N> {
    ArrayCodec(element)
}

impl<C: Decoder, const N: usize> Decoder for ArrayCodec<C, N> {
    type Output = [C::Output; N];
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<[C::Output; N], R> {
        let at = r.here();
        r.begin_array()?;
        // Filled in place: no allocation, and no `Default` needed for `T`.
        let slots = cx.nested(|cx| {
            let mut slots: [Option<C::Output>; N] = [const { None }; N];
            let mut errs = Collector::new();
            let mut count = 0;
            while r.array_has_next(count == 0)? {
                if count == N {
                    return Err(DecodeError::at(
                        Problem::mismatch(
                            alloc::format!("{N} elements"),
                            alloc::format!("more than {N}"),
                        ),
                        r.here(),
                    ));
                }
                let i = count;
                if let Some(v) = child(
                    r,
                    cx,
                    &mut errs,
                    || PathSegment::Index(i),
                    |r, cx| decode_child(&self.0, r, cx),
                )? {
                    slots[i] = Some(v);
                }
                count += 1;
            }
            if count < N {
                return Err(DecodeError::at(
                    Problem::mismatch(alloc::format!("{N} elements"), alloc::format!("{count}")),
                    at.clone(),
                ));
            }
            errs.finish()?;
            Ok(slots)
        })?;
        r.end_array()?;
        Ok(slots.map(|slot| slot.expect("every element was decoded")))
    }
}

impl<C: Encoder<Input: Sized>, const N: usize> Encoder for ArrayCodec<C, N> {
    type Input = [C::Input; N];
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &[C::Input; N],
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.begin_array(Some(N))?;
        for x in v {
            s.array_element()?;
            self.0.encode(x, s, cx)?;
        }
        s.end_array()
    }
}

// -- fixed-size tuples as arrays -------------------------------------------------

macro_rules! tuple_codec {
    ($name:ident, $fn:ident, $n:expr; $($C:ident $i:tt),+) => {
        /// A fixed-length array as a tuple: `[a, b]` ↔ `(A, B)`.
        #[derive(Debug, Clone, Copy)]
        pub struct $name<$($C),+>($(pub $C),+);
        /// A fixed-length array as a tuple.
        #[allow(non_snake_case)]
        pub const fn $fn<$($C),+>($($C: $C),+) -> $name<$($C),+> {
            $name($($C),+)
        }
        #[allow(non_snake_case)]
        impl<$($C: Decoder),+> Decoder for $name<$($C),+> {
            type Output = ($($C::Output,)+);
            fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
                let at = r.here();
                r.begin_array()?;
                let out = cx.nested(|cx| {
                    Ok(($({
                        if !r.array_has_next($i == 0)? {
                            return Err(DecodeError::at(
                                Problem::mismatch(concat!("array of ", stringify!($n)), alloc::format!("array of {}", $i)),
                                at.clone(),
                            ));
                        }
                        decode_child(&self.$i, r, cx).map_err(|e| at_seg(e, cx, || PathSegment::Index($i)))?
                    },)+))
                })?;
                if r.array_has_next(false)? {
                    return Err(DecodeError::at(
                        Problem::mismatch(concat!("array of ", stringify!($n)), "longer array"),
                        at,
                    ));
                }
                r.end_array()?;
                Ok(out)
            }
        }
        impl<$($C: Encoder<Input: Sized>),+> Encoder for $name<$($C),+> {
            type Input = ($($C::Input,)+);
            fn encode<S: Serializer + ?Sized>(&self, v: &Self::Input, s: &mut S, cx: &mut EncodeContext) -> Result<(), S::Error> {
                s.begin_array(Some($n))?;
                $(s.array_element()?; self.$i.encode(&v.$i, s, cx)?;)+
                s.end_array()
            }
        }
    };
}
tuple_codec!(Tuple2, tuple2, 2; A 0, B 1);
tuple_codec!(Tuple3, tuple3, 3; A 0, B 1, C 2);
tuple_codec!(Tuple4, tuple4, 4; A 0, B 1, C 2, D 3);
