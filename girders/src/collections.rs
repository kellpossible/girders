//! Collection codecs: `list`, `entries`, `btree_map`, `hash_map`.

use crate::codec::{DecodeResult, Decoder, Encoder, decode_child};
use crate::context::{DecodeContext, EncodeContext};
use crate::error::{Collector, DecodeError, PathSegment, Problem};
use crate::reader::Reader;
use crate::serializer::Serializer;
use alloc::collections::BTreeMap;
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
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
        r.begin_array()?;
        cx.nested(|cx| {
            let mut out = Vec::new();
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

/// Read every entry of an object, calling `insert` per decoded entry.
fn decode_entries<
    R: Reader + ?Sized,
    C: Decoder,
    F: FnMut(String, C::Output) -> Result<(), Problem>,
>(
    inner: &C,
    r: &mut R,
    cx: &mut DecodeContext,
    mut insert: F,
) -> DecodeResult<(), R> {
    r.begin_object()?;
    cx.nested(|cx| {
        let mut errs = Collector::new();
        let mut first = true;
        let mut key = String::new();
        while r.object_next_key(first, &mut key)? {
            first = false;
            let k = core::mem::take(&mut key);
            let at = r.here();
            if let Some(v) = child(
                r,
                cx,
                &mut errs,
                || PathSegment::Key(k.clone()),
                |r, cx| decode_child(&inner, r, cx),
            )? && let Err(p) = insert(k, v)
            {
                let e = DecodeError::at(p, at);
                if cx.options.accumulate {
                    errs.push(e);
                } else {
                    return Err(e);
                }
            }
        }
        r.end_object()?;
        errs.finish()
    })
}

/// Object entries in document order, duplicates kept: `Vec<(String, T)>`.
#[derive(Debug, Clone, Copy)]
pub struct Entries<C>(pub C);
pub const fn entries<C>(inner: C) -> Entries<C> {
    Entries(inner)
}

impl<C: Decoder> Decoder for Entries<C> {
    type Output = Vec<(String, C::Output)>;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
        let mut out = Vec::new();
        decode_entries(&self.0, r, cx, |k, v| {
            out.push((k, v));
            Ok(())
        })?;
        Ok(out)
    }
}

impl<C: Encoder<Input: Sized>> Encoder for Entries<C> {
    type Input = Vec<(String, C::Input)>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.begin_object(Some(v.len()))?;
        for (k, x) in v {
            s.object_key(k)?;
            self.0.encode(x, s, cx)?;
        }
        s.end_object()
    }
}

/// `BTreeMap<String, T>`. Duplicate keys are an error.
#[derive(Debug, Clone, Copy)]
pub struct BTreeMapCodec<C>(pub C);
pub const fn btree_map<C>(inner: C) -> BTreeMapCodec<C> {
    BTreeMapCodec(inner)
}

impl<C: Decoder> Decoder for BTreeMapCodec<C> {
    type Output = BTreeMap<String, C::Output>;
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Self::Output, R> {
        let mut out = BTreeMap::new();
        decode_entries(&self.0, r, cx, |k, v| match out.entry(k) {
            alloc::collections::btree_map::Entry::Occupied(o) => {
                Err(Problem::DuplicateField(o.key().clone()))
            }
            alloc::collections::btree_map::Entry::Vacant(e) => {
                e.insert(v);
                Ok(())
            }
        })?;
        Ok(out)
    }
}

impl<C: Encoder<Input: Sized>> Encoder for BTreeMapCodec<C> {
    type Input = BTreeMap<String, C::Input>;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Self::Input,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        s.begin_object(Some(v.len()))?;
        for (k, x) in v {
            s.object_key(k)?;
            self.0.encode(x, s, cx)?;
        }
        s.end_object()
    }
}

#[cfg(feature = "std")]
mod hash {
    use super::*;
    use std::collections::HashMap;

    /// `HashMap<String, T>` (requires `std`). Duplicate keys are an error.
    /// Encoding order follows the map's iteration order.
    #[derive(Debug, Clone, Copy)]
    pub struct HashMapCodec<C>(pub C);
    pub const fn hash_map<C>(inner: C) -> HashMapCodec<C> {
        HashMapCodec(inner)
    }

    impl<C: Decoder> Decoder for HashMapCodec<C> {
        type Output = HashMap<String, C::Output>;
        fn decode<R: Reader + ?Sized>(
            &self,
            r: &mut R,
            cx: &mut DecodeContext,
        ) -> DecodeResult<Self::Output, R> {
            let mut out = HashMap::new();
            decode_entries(&self.0, r, cx, |k, v| match out.entry(k) {
                std::collections::hash_map::Entry::Occupied(o) => {
                    Err(Problem::DuplicateField(o.key().clone()))
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(v);
                    Ok(())
                }
            })?;
            Ok(out)
        }
    }

    impl<C: Encoder<Input: Sized>> Encoder for HashMapCodec<C> {
        type Input = HashMap<String, C::Input>;
        fn encode<S: Serializer + ?Sized>(
            &self,
            v: &Self::Input,
            s: &mut S,
            cx: &mut EncodeContext,
        ) -> Result<(), S::Error> {
            s.begin_object(Some(v.len()))?;
            for (k, x) in v {
                s.object_key(k)?;
                self.0.encode(x, s, cx)?;
            }
            s.end_object()
        }
    }
}
#[cfg(feature = "std")]
pub use hash::{HashMapCodec, hash_map};

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
