//! Records: a static set of fields, decoded in one pass.
//!
//! A record is a static tuple of field specs. Decoding is one pass: each
//! key is dispatched to its field, which decodes straight into a slot, so
//! fields may come in any order and nothing is buffered. The field list is
//! static, which is what makes strict unknown-field checks and `fields_of`
//! possible.

use crate::codec::{Codec, DResult, Decoder, Encoder, decode_child};
use crate::collections::{at_seg, child};
use crate::context::{DecodeCx, EncodeCx, TagValue};
use crate::error::{Collector, DecodeError, PathSegment, Problem};
use crate::reader::{Kind, Reader};
use crate::serializer::Serializer;
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

/// What to do when an object repeats a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Duplicates {
    /// `DuplicateField` error (the default: strict).
    #[default]
    Error,
    /// Keep the first value, skip later ones.
    FirstWins,
    /// Decode every occurrence, keep the last.
    LastWins,
}

/// Per-field decode state.
#[derive(Debug)]
pub enum Slot<T> {
    Empty,
    /// The value was present but failed to decode (accumulate mode), so a
    /// "missing field" error must not be reported on top.
    Failed,
    Full(T),
}

/// Shared state for one record decode.
pub struct KeyCx<'a, L> {
    pub duplicates: Duplicates,
    pub errs: &'a mut Collector<L>,
}

/// A set of fields reading from (and building) `Self::Rec`.
pub trait Fields {
    type Rec;
    type Slots;
    type Values;

    fn new_slots(&self) -> Self::Slots;
    fn names(&self, out: &mut Vec<&'static str>);
    /// How many names [`Fields::names`] reports.
    fn name_count(&self) -> usize;
    /// Decode the value for the `i`th name (compact decoding: the key was
    /// already matched against the name table).
    fn decode_at<R: Reader + ?Sized>(
        &self,
        i: usize,
        r: &mut R,
        cx: &mut DecodeCx,
        slots: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R>;
    /// Decode a key no field claimed (only `extras` does this).
    fn decode_extra<R: Reader + ?Sized>(
        &self,
        _key: &str,
        _r: &mut R,
        _cx: &mut DecodeCx,
        _slots: &mut Self::Slots,
        _kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<bool, R> {
        Ok(false)
    }
    /// Turn slots into values. Fail fast: `Err` on the first missing
    /// field. Accumulating: record every problem in `kcx.errs` and return
    /// `Ok(None)`.
    /// Report required fields that are absent, without moving anything.
    /// `false` if any value can't be taken (missing, or failed in
    /// accumulate mode). Checking before moving keeps `take` free of
    /// early exits, whose drop glue would otherwise grow with the square
    /// of the field count.
    fn check(&self, slots: &Self::Slots, missing: &mut dyn FnMut(&'static str)) -> bool;
    /// Move the values out; only called after `check` returned `true`.
    fn take(&self, slots: Self::Slots) -> Self::Values;
    fn count(&self, rec: &Self::Rec) -> usize;
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Self::Rec,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error>;
}

#[cold]
#[inline(never)]
fn missing<L: Clone>(name: &'static str, cx: &DecodeCx, errs: &mut Collector<L>, at: &Option<L>) {
    // Fail-fast reports only the first problem.
    if cx.options.accumulate || !errs.has_errors() {
        errs.push(DecodeError::at(
            Problem::MissingField(name.into()),
            at.clone(),
        ));
    }
}

/// Handle a repeated key. `Ok(true)` means "decode it (again)".
#[cold]
#[inline(never)]
fn on_duplicate<R: Reader + ?Sized>(
    name: &'static str,
    r: &mut R,
    cx: &DecodeCx,
    kcx: &mut KeyCx<'_, R::Location>,
) -> DResult<bool, R> {
    match kcx.duplicates {
        Duplicates::LastWins => Ok(true),
        Duplicates::FirstWins => {
            r.skip_value()?;
            Ok(false)
        }
        Duplicates::Error => {
            let e = at_seg(
                DecodeError::at(Problem::DuplicateField(name.into()), r.here()),
                cx,
                || PathSegment::Key(name.into()),
            );
            if cx.options.accumulate {
                kcx.errs.push(e);
                r.skip_value()?;
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

fn decode_into<R: Reader + ?Sized, C: Decoder>(
    name: &'static str,
    codec: &C,
    r: &mut R,
    cx: &mut DecodeCx,
    slot: &mut Slot<C::Output>,
    kcx: &mut KeyCx<'_, R::Location>,
) -> DResult<(), R> {
    if !matches!(slot, Slot::Empty) && !on_duplicate(name, r, cx, kcx)? {
        return Ok(());
    }
    let cp = if cx.options.accumulate {
        Some(r.checkpoint())
    } else {
        None
    };
    match decode_child(codec, r, cx) {
        Ok(v) => {
            if let Some(cp) = cp {
                r.release(cp);
            }
            *slot = Slot::Full(v);
        }
        Err(e) => {
            field_failed(name, r, cx, kcx, cp, e)?;
            *slot = Slot::Failed;
        }
    }
    Ok(())
}

/// A field's value failed to decode: add the key to the path, then fail
/// (fail-fast or fatal) or record it and skip the value (accumulate).
/// Generic only over the reader, so it isn't duplicated per field.
#[cold]
#[inline(never)]
fn field_failed<R: Reader + ?Sized>(
    name: &'static str,
    r: &mut R,
    cx: &DecodeCx,
    kcx: &mut KeyCx<'_, R::Location>,
    cp: Option<R::Checkpoint>,
    e: DecodeError<R::Location>,
) -> DResult<(), R> {
    let e = at_seg(e, cx, || PathSegment::Key(name.into()));
    let Some(cp) = cp else { return Err(e) };
    if e.is_fatal() {
        r.release(cp);
        return Err(e);
    }
    r.rewind(&cp);
    r.release(cp);
    r.skip_value()?;
    kcx.errs.push(e);
    Ok(())
}

// -- field ---------------------------------------------------------------------

/// A required field.
pub struct Field<C, G, Rec> {
    name: &'static str,
    codec: C,
    get: G,
    _rec: PhantomData<fn(&Rec)>,
}

impl<C: Clone, G: Clone, Rec> Clone for Field<C, G, Rec> {
    fn clone(&self) -> Self {
        Field {
            name: self.name,
            codec: self.codec.clone(),
            get: self.get.clone(),
            _rec: PhantomData,
        }
    }
}

/// A required field: name, codec, and a getter the encoder reads it with.
pub fn field<Rec, C, G>(name: &'static str, codec: C, get: G) -> Field<C, G, Rec>
where
    C: Codec<Output: Sized>,
    G: Fn(&Rec) -> &C::Output,
{
    Field {
        name,
        codec,
        get,
        _rec: PhantomData,
    }
}

impl<Rec, C: Codec<Output: Sized>, G: Fn(&Rec) -> &C::Output> Fields for Field<C, G, Rec> {
    type Rec = Rec;
    type Slots = Slot<C::Output>;
    type Values = C::Output;

    fn new_slots(&self) -> Self::Slots {
        Slot::Empty
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        out.push(self.name);
    }
    fn name_count(&self) -> usize {
        1
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeCx,
        slot: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R> {
        decode_into(self.name, &self.codec, r, cx, slot, kcx)
    }
    fn check(&self, slot: &Self::Slots, missing: &mut dyn FnMut(&'static str)) -> bool {
        match slot {
            Slot::Full(_) => true,
            Slot::Failed => false,
            Slot::Empty => {
                missing(self.name);
                false
            }
        }
    }
    fn take(&self, slot: Self::Slots) -> Self::Values {
        match slot {
            Slot::Full(v) => v,
            _ => unreachable!("checked"),
        }
    }
    fn count(&self, _rec: &Rec) -> usize {
        1
    }
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Rec,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        s.object_key(self.name)?;
        self.codec.encode((self.get)(rec), s, cx)
    }
}

// -- field with a default -------------------------------------------------------

/// A field that takes `default()` when absent. Always encoded.
pub struct DefaultField<C, G, D, Rec> {
    inner: Field<C, G, Rec>,
    default: D,
}

impl<C: Clone, G: Clone, D: Clone, Rec> Clone for DefaultField<C, G, D, Rec> {
    fn clone(&self) -> Self {
        DefaultField {
            inner: self.inner.clone(),
            default: self.default.clone(),
        }
    }
}

pub fn field_default<Rec, C, G, D>(
    name: &'static str,
    codec: C,
    get: G,
    default: D,
) -> DefaultField<C, G, D, Rec>
where
    C: Codec<Output: Sized>,
    G: Fn(&Rec) -> &C::Output,
    D: Fn() -> C::Output,
{
    DefaultField {
        inner: field(name, codec, get),
        default,
    }
}

impl<Rec, C, G, D> Fields for DefaultField<C, G, D, Rec>
where
    C: Codec<Output: Sized>,
    G: Fn(&Rec) -> &C::Output,
    D: Fn() -> C::Output,
{
    type Rec = Rec;
    type Slots = Slot<C::Output>;
    type Values = C::Output;

    fn new_slots(&self) -> Self::Slots {
        Slot::Empty
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        self.inner.names(out)
    }
    fn name_count(&self) -> usize {
        1
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeCx,
        slot: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R> {
        decode_into(self.inner.name, &self.inner.codec, r, cx, slot, kcx)
    }
    fn check(&self, slot: &Self::Slots, _missing: &mut dyn FnMut(&'static str)) -> bool {
        !matches!(slot, Slot::Failed)
    }
    fn take(&self, slot: Self::Slots) -> Self::Values {
        match slot {
            Slot::Full(v) => v,
            Slot::Empty => (self.default)(),
            Slot::Failed => unreachable!("checked"),
        }
    }
    fn count(&self, rec: &Rec) -> usize {
        self.inner.count(rec)
    }
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Rec,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        self.inner.encode(rec, s, cx)
    }
}

// -- optional field ------------------------------------------------------------

/// An optional field: absent or `null` decodes to `None`. `None` encodes
/// as `null` unless [`OptField::skip_none`] is set.
pub struct OptField<C, G, Rec> {
    name: &'static str,
    codec: C,
    get: G,
    skip_none: bool,
    _rec: PhantomData<fn(&Rec)>,
}

impl<C: Clone, G: Clone, Rec> Clone for OptField<C, G, Rec> {
    fn clone(&self) -> Self {
        OptField {
            name: self.name,
            codec: self.codec.clone(),
            get: self.get.clone(),
            skip_none: self.skip_none,
            _rec: PhantomData,
        }
    }
}

pub fn optional_field<Rec, C, G>(name: &'static str, codec: C, get: G) -> OptField<C, G, Rec>
where
    C: Codec<Output: Sized>,
    G: Fn(&Rec) -> &Option<C::Output>,
{
    OptField {
        name,
        codec,
        get,
        skip_none: false,
        _rec: PhantomData,
    }
}

impl<C, G, Rec> OptField<C, G, Rec> {
    /// Omit the key entirely for `None` instead of writing `null`.
    pub fn skip_none(mut self) -> Self {
        self.skip_none = true;
        self
    }
}

impl<Rec, C: Codec<Output: Sized>, G: Fn(&Rec) -> &Option<C::Output>> Fields
    for OptField<C, G, Rec>
{
    type Rec = Rec;
    type Slots = Slot<Option<C::Output>>;
    type Values = Option<C::Output>;

    fn new_slots(&self) -> Self::Slots {
        Slot::Empty
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        out.push(self.name);
    }
    fn name_count(&self) -> usize {
        1
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeCx,
        slot: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R> {
        decode_into(
            self.name,
            &crate::codec::optional(&self.codec),
            r,
            cx,
            slot,
            kcx,
        )
    }
    fn check(&self, slot: &Self::Slots, _missing: &mut dyn FnMut(&'static str)) -> bool {
        !matches!(slot, Slot::Failed)
    }
    fn take(&self, slot: Self::Slots) -> Self::Values {
        match slot {
            Slot::Full(v) => v,
            Slot::Empty => None,
            Slot::Failed => unreachable!("checked"),
        }
    }
    fn count(&self, rec: &Rec) -> usize {
        (!self.skip_none || (self.get)(rec).is_some()) as usize
    }
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Rec,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        match (self.get)(rec) {
            None if self.skip_none => Ok(()),
            None => {
                s.object_key(self.name)?;
                s.write_null()
            }
            Some(v) => {
                s.object_key(self.name)?;
                self.codec.encode(v, s, cx)
            }
        }
    }
}

// -- extras: collect unknown keys ------------------------------------------------

/// Collects every key no other field claims into `Vec<(String, V)>`,
/// decoded with `codec` (e.g. a dynamic `Value` codec).
pub struct Extras<C, G, Rec> {
    codec: C,
    get: G,
    _rec: PhantomData<fn(&Rec)>,
}

impl<C: Clone, G: Clone, Rec> Clone for Extras<C, G, Rec> {
    fn clone(&self) -> Self {
        Extras {
            codec: self.codec.clone(),
            get: self.get.clone(),
            _rec: PhantomData,
        }
    }
}

pub fn extras<Rec, C, G>(codec: C, get: G) -> Extras<C, G, Rec>
where
    C: Codec<Output: Sized>,
    G: Fn(&Rec) -> &Vec<(String, C::Output)>,
{
    Extras {
        codec,
        get,
        _rec: PhantomData,
    }
}

impl<Rec, C: Codec<Output: Sized>, G: Fn(&Rec) -> &Vec<(String, C::Output)>> Fields
    for Extras<C, G, Rec>
{
    type Rec = Rec;
    type Slots = (Vec<(String, C::Output)>, bool);
    type Values = Vec<(String, C::Output)>;

    fn new_slots(&self) -> Self::Slots {
        (Vec::new(), false)
    }
    fn names(&self, _out: &mut Vec<&'static str>) {}
    fn name_count(&self) -> usize {
        0
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeCx,
        slot: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R> {
        let _ = (r, cx, slot, kcx);
        Ok(())
    }
    fn decode_extra<R: Reader + ?Sized>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut DecodeCx,
        slots: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<bool, R> {
        let k = String::from(key);
        match child(
            r,
            cx,
            kcx.errs,
            || PathSegment::Key(k.clone()),
            |r, cx| decode_child(&self.codec, r, cx),
        )? {
            Some(v) => slots.0.push((k, v)),
            None => slots.1 = true,
        }
        Ok(true)
    }
    fn check(&self, slots: &Self::Slots, _missing: &mut dyn FnMut(&'static str)) -> bool {
        !slots.1
    }
    fn take(&self, slots: Self::Slots) -> Self::Values {
        slots.0
    }
    fn count(&self, rec: &Rec) -> usize {
        (self.get)(rec).len()
    }
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Rec,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        for (k, v) in (self.get)(rec) {
            s.object_key(k)?;
            self.codec.encode(v, s, cx)?;
        }
        Ok(())
    }
}

// -- fields_of: splice another record's fields into this object -----------------

/// A record whose field set can be reused by [`fields_of`].
pub trait RecordCodec: Codec<Output: Sized> {
    type Fields: Fields<Rec = Self::Output>;
    fn fields(&self) -> &Self::Fields;
    /// Build the record from slots that passed [`Fields::check`].
    fn build_slots(&self, slots: <Self::Fields as Fields>::Slots) -> Self::Output;
}

pub struct FieldsOf<RC, G, Outer> {
    inner: RC,
    get: G,
    _outer: PhantomData<fn(&Outer)>,
}

impl<RC: Clone, G: Clone, Outer> Clone for FieldsOf<RC, G, Outer> {
    fn clone(&self) -> Self {
        FieldsOf {
            inner: self.inner.clone(),
            get: self.get.clone(),
            _outer: PhantomData,
        }
    }
}

/// Read another record's fields from the *current* object: explicit
/// flattening. Both field sets are static, so unknown-field checks see
/// the merged set.
pub fn fields_of<Outer, RC, G>(inner: RC, get: G) -> FieldsOf<RC, G, Outer>
where
    RC: RecordCodec,
    G: Fn(&Outer) -> &RC::Output,
{
    FieldsOf {
        inner,
        get,
        _outer: PhantomData,
    }
}

impl<Outer, RC: RecordCodec, G: Fn(&Outer) -> &RC::Output> Fields for FieldsOf<RC, G, Outer> {
    type Rec = Outer;
    type Slots = <RC::Fields as Fields>::Slots;
    type Values = RC::Output;

    fn new_slots(&self) -> Self::Slots {
        self.inner.fields().new_slots()
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        self.inner.fields().names(out)
    }
    fn name_count(&self) -> usize {
        self.inner.fields().name_count()
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        i: usize,
        r: &mut R,
        cx: &mut DecodeCx,
        slots: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R> {
        self.inner.fields().decode_at(i, r, cx, slots, kcx)
    }
    fn decode_extra<R: Reader + ?Sized>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut DecodeCx,
        slots: &mut Self::Slots,
        kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<bool, R> {
        self.inner.fields().decode_extra(key, r, cx, slots, kcx)
    }
    fn check(&self, slots: &Self::Slots, missing: &mut dyn FnMut(&'static str)) -> bool {
        self.inner.fields().check(slots, missing)
    }
    fn take(&self, slots: Self::Slots) -> Self::Values {
        self.inner.build_slots(slots)
    }
    fn count(&self, rec: &Outer) -> usize {
        self.inner.fields().count((self.get)(rec))
    }
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Outer,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        self.inner.fields().encode((self.get)(rec), s, cx)
    }
}

// -- tuples of field specs -----------------------------------------------------

macro_rules! tuple_fields {
    ($T0:ident 0 $(, $T:ident $i:tt)*) => {
        impl<$T0: Fields, $($T: Fields<Rec = $T0::Rec>),*> Fields for ($T0, $($T,)*) {
            type Rec = $T0::Rec;
            type Slots = ($T0::Slots, $($T::Slots,)*);
            type Values = ($T0::Values, $($T::Values,)*);

            fn new_slots(&self) -> Self::Slots {
                (self.0.new_slots(), $(self.$i.new_slots(),)*)
            }
            fn names(&self, out: &mut Vec<&'static str>) {
                self.0.names(out);
                $(self.$i.names(out);)*
            }
            fn name_count(&self) -> usize {
                self.0.name_count() $(+ self.$i.name_count())*
            }
            #[inline]
            fn decode_at<R: Reader + ?Sized>(
                &self,
                i: usize,
                r: &mut R,
                cx: &mut DecodeCx,
                slots: &mut Self::Slots,
                kcx: &mut KeyCx<'_, R::Location>,
            ) -> DResult<(), R> {
                #[allow(unused_mut)]
                let mut i = i;
                let n = self.0.name_count();
                if i < n {
                    return self.0.decode_at(i, r, cx, &mut slots.0, kcx);
                }
                i -= n;
                $(
                    let n = self.$i.name_count();
                    if i < n {
                        return self.$i.decode_at(i, r, cx, &mut slots.$i, kcx);
                    }
                    i -= n;
                )*
                let _ = i;
                Ok(())
            }
            fn decode_extra<R: Reader + ?Sized>(
                &self,
                key: &str,
                r: &mut R,
                cx: &mut DecodeCx,
                slots: &mut Self::Slots,
                kcx: &mut KeyCx<'_, R::Location>,
            ) -> DResult<bool, R> {
                if self.0.decode_extra(key, r, cx, &mut slots.0, kcx)? {
                    return Ok(true);
                }
                $(if self.$i.decode_extra(key, r, cx, &mut slots.$i, kcx)? {
                    return Ok(true);
                })*
                Ok(false)
            }
            fn check(&self, slots: &Self::Slots, missing: &mut dyn FnMut(&'static str)) -> bool {
                // Check every field (no short-circuit) so all missing
                // fields are reported.
                #[allow(unused_mut)]
                let mut ok = self.0.check(&slots.0, missing);
                $(ok &= self.$i.check(&slots.$i, missing);)*
                ok
            }
            fn take(&self, slots: Self::Slots) -> Self::Values {
                (self.0.take(slots.0), $(self.$i.take(slots.$i),)*)
            }
            fn count(&self, rec: &Self::Rec) -> usize {
                self.0.count(rec) $(+ self.$i.count(rec))*
            }
            #[inline]
            fn encode<S: Serializer + ?Sized>(&self, rec: &Self::Rec, s: &mut S, cx: &mut EncodeCx) -> Result<(), S::Error> {
                self.0.encode(rec, s, cx)?;
                $(self.$i.encode(rec, s, cx)?;)*
                Ok(())
            }
        }
    };
}

tuple_fields!(T0 0);
tuple_fields!(T0 0, T1 1);
tuple_fields!(T0 0, T1 1, T2 2);
tuple_fields!(T0 0, T1 1, T2 2, T3 3);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9, T10 10);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9, T10 10, T11 11);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9, T10 10, T11 11, T12 12);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9, T10 10, T11 11, T12 12, T13 13);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9, T10 10, T11 11, T12 12, T13 13, T14 14);
tuple_fields!(T0 0, T1 1, T2 2, T3 3, T4 4, T5 5, T6 6, T7 7, T8 8, T9 9, T10 10, T11 11, T12 12, T13 13, T14 14, T15 15);

// -- record --------------------------------------------------------------------

pub struct RecordBuilder<F> {
    fields: F,
}

/// Start a record from a tuple of field specs; finish with `.build(...)`.
pub fn record<F: Fields>(fields: F) -> RecordBuilder<F> {
    RecordBuilder { fields }
}

impl<F: Fields> RecordBuilder<F> {
    pub fn build<B>(self, build: B) -> Record<F, TupleBuild<B>>
    where
        B: Fn(F::Values) -> F::Rec,
    {
        Record {
            fields: self.fields,
            build: TupleBuild(build),
            allow_unknown: false,
            duplicates: Duplicates::Error,
        }
    }
}

/// How a record turns its decoded slots into the record value.
pub trait Build<F: Fields> {
    fn build(&self, fields: &F, slots: F::Slots) -> F::Rec;
}

/// `record((...)).build(|(a, b, ...)| ...)`: the values as a tuple.
#[derive(Debug, Clone, Copy)]
pub struct TupleBuild<B>(B);

impl<F: Fields, B: Fn(F::Values) -> F::Rec> Build<F> for TupleBuild<B> {
    #[inline]
    fn build(&self, fields: &F, slots: F::Slots) -> F::Rec {
        (self.0)(fields.take(slots))
    }
}

#[derive(Clone)]
pub struct Record<F, B> {
    fields: F,
    build: B,
    allow_unknown: bool,
    duplicates: Duplicates,
}

impl<F: Fields, B: Build<F>> Record<F, B> {
    /// Check for missing fields, then build the record.
    #[inline]
    fn complete<L: Clone>(
        &self,
        slots: F::Slots,
        cx: &DecodeCx,
        mut errs: Collector<L>,
        at: &Option<L>,
    ) -> Result<F::Rec, DecodeError<L>> {
        let ok = self
            .fields
            .check(&slots, &mut |name| missing(name, cx, &mut errs, at));
        errs.finish()?;
        debug_assert!(ok, "a field can only fail with an error recorded");
        Ok(self.build.build(&self.fields, slots))
    }
}

/// Called per key with the matched field index (or `None`) and the key.
/// Returns whether the key was consumed.
type OnKey<'a, R> = dyn FnMut(
        Option<usize>,
        &str,
        &mut R,
        &mut DecodeCx,
        &mut Collector<<R as Reader>::Location>,
    ) -> DResult<bool, R>
    + 'a;

/// The record key loop shared by every compact record decode.
#[inline(never)]
fn record_loop<R: Reader + ?Sized>(
    r: &mut R,
    cx: &mut DecodeCx,
    names: &[&'static str],
    allow_unknown: bool,
    errs: &mut Collector<R::Location>,
    on_key: &mut OnKey<'_, R>,
) -> DResult<(), R> {
    let claimed = cx.take_claimed();
    r.begin_object()?;
    let mut key = cx.take_key_buf();
    let out = cx.nested(|cx| {
        let mut first = true;
        while r.object_next_key(first, &mut key)? {
            first = false;
            if claimed == Some(key.as_str()) {
                r.skip_value()?;
            } else {
                let found = names.iter().position(|n| *n == key);
                if !on_key(found, &key, r, cx, errs)? {
                    if allow_unknown {
                        r.skip_value()?;
                    } else {
                        let e = DecodeError::at(
                            Problem::UnknownField {
                                name: key.clone(),
                                expected: names.to_vec(),
                            },
                            r.here(),
                        );
                        let e = at_seg(e, cx, || PathSegment::Key(key.clone()));
                        if !cx.options.accumulate {
                            return Err(e);
                        }
                        errs.push(e);
                        r.skip_value()?;
                    }
                }
            }
            key.clear();
        }
        r.end_object()
    });
    cx.put_key_buf(key);
    out
}

impl<F, B> Record<F, B> {
    pub(crate) fn from_parts(fields: F, build: B, allow_unknown: bool, duplicates: Duplicates) -> Self {
        Record { fields, build, allow_unknown, duplicates }
    }

    /// Skip unknown keys instead of failing.
    pub fn allow_unknown(mut self) -> Self {
        self.allow_unknown = true;
        self
    }

    pub fn duplicates(mut self, policy: Duplicates) -> Self {
        self.duplicates = policy;
        self
    }
}

impl<F: Fields, B: Build<F>> Decoder for Record<F, B> {
    type Output = F::Rec;

    /// The key loop is shared by every record (generic only over the
    /// reader); per record there is just the index dispatch, the
    /// missing-field check and `build`.
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeCx) -> DResult<F::Rec, R> {
        let at = r.here();
        let mut names = cx.take_name_buf();
        self.fields.names(&mut names);
        let mut slots = self.fields.new_slots();
        let mut errs = Collector::new();
        let duplicates = self.duplicates;
        let looped = record_loop(
            r,
            cx,
            &names,
            self.allow_unknown,
            &mut errs,
            &mut |found, key, r, cx, errs| {
                let mut kcx = KeyCx { duplicates, errs };
                match found {
                    Some(i) => self
                        .fields
                        .decode_at(i, r, cx, &mut slots, &mut kcx)
                        .map(|()| true),
                    None => self.fields.decode_extra(key, r, cx, &mut slots, &mut kcx),
                }
            },
        );
        cx.put_name_buf(names);
        looped?;
        self.complete(slots, cx, errs, &at)
    }
}

impl<F: Fields, B: Build<F>> Encoder for Record<F, B> {
    type Input = F::Rec;

    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &F::Rec,
        s: &mut S,
        cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        let tag = cx.take_pending_tag();
        s.begin_object(Some(self.fields.count(v) + tag.is_some() as usize))?;
        if let Some((key, value)) = tag {
            s.object_key(key)?;
            match value {
                TagValue::Str(t) => s.write_str(t)?,
                TagValue::Int(i) => s.write_i64(i)?,
            }
        }
        self.fields.encode(v, s, cx)?;
        s.end_object()
    }
}

impl<F: Fields<Rec: Sized>, B: Build<F>> RecordCodec for Record<F, B> {
    type Fields = F;
    fn fields(&self) -> &F {
        &self.fields
    }
    fn build_slots(&self, slots: F::Slots) -> F::Rec {
        self.build.build(&self.fields, slots)
    }
}

/// Whether the next value is an object (used by dispatchers).
pub(crate) fn expect_object<R: Reader + ?Sized>(r: &mut R) -> DResult<(), R> {
    match r.peek_kind()? {
        Kind::Object => Ok(()),
        _ => Err(crate::reader::mismatch(r, "object")),
    }
}

// -- typed field factory -------------------------------------------------------

/// Field constructors with the record type fixed once, so getter closures
/// need no type annotation:
///
/// ```ignore
/// let f = fields_for::<User>();
/// record((f.field("id", string(), |u| &u.id), f.field("age", u32(), |u| &u.age)))
/// ```
pub struct FieldsFor<Rec>(PhantomData<fn(&Rec)>);

impl<Rec> Clone for FieldsFor<Rec> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<Rec> Copy for FieldsFor<Rec> {}

pub const fn fields_for<Rec>() -> FieldsFor<Rec> {
    FieldsFor(PhantomData)
}

impl<Rec> FieldsFor<Rec> {
    pub fn field<C, G>(self, name: &'static str, codec: C, get: G) -> Field<C, G, Rec>
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &C::Output,
    {
        field(name, codec, get)
    }

    pub fn optional<C, G>(self, name: &'static str, codec: C, get: G) -> OptField<C, G, Rec>
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &Option<C::Output>,
    {
        optional_field(name, codec, get)
    }

    pub fn default<C, G, D>(
        self,
        name: &'static str,
        codec: C,
        get: G,
        default: D,
    ) -> DefaultField<C, G, D, Rec>
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &C::Output,
        D: Fn() -> C::Output,
    {
        field_default(name, codec, get, default)
    }

    pub fn extras<C, G>(self, codec: C, get: G) -> Extras<C, G, Rec>
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &Vec<(String, C::Output)>,
    {
        extras(codec, get)
    }

    pub fn flatten<RC, G>(self, inner: RC, get: G) -> FieldsOf<RC, G, Rec>
    where
        RC: RecordCodec,
        G: Fn(&Rec) -> &RC::Output,
    {
        fields_of(inner, get)
    }
}
