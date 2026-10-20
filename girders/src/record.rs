//! Records: a static set of fields, decoded in one pass.
//!
//! A record is a static tuple of field codecs. Decoding is one pass: each
//! key is dispatched to its field, which decodes straight into a slot, so
//! fields may come in any order and nothing is buffered. The field list is
//! static, which is what makes strict unknown-field checks and `fields_of`
//! possible.

use crate::codec::{Codec, DecodeResult, Decoder, Encoder, decode_child};
use crate::collections::{at_seg, child};
use crate::context::{DecodeContext, EncodeContext, TagValue};
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

// The helpers below hold the bodies of per-field methods. Every field has
// its own type (its getter closure differs), so a method on
// the field is compiled once per field; these are compiled once per value
// type and shared.
impl<T> Slot<T> {
    /// `check` for a required field: report it if absent.
    fn check_required(&self, name: &'static str, missing: &mut dyn FnMut(&'static str)) -> bool {
        match self {
            Slot::Full(_) => true,
            Slot::Failed => false,
            Slot::Empty => {
                missing(name);
                false
            }
        }
    }

    /// The value after `check` passed; `absent` if the key never came.
    fn into_checked_or(self, absent: T) -> T {
        match self {
            Slot::Full(v) => v,
            Slot::Empty => absent,
            Slot::Failed => unreachable!("checked"),
        }
    }

    /// The value after `check` passed for a required field.
    fn into_checked(self) -> T {
        match self {
            Slot::Full(v) => v,
            _ => unreachable!("checked"),
        }
    }
}

fn encode_field<C: Encoder + ?Sized, S: Serializer + ?Sized>(
    name: &'static str,
    codec: &C,
    value: &C::Input,
    s: &mut S,
    cx: &mut EncodeContext,
) -> Result<(), S::Error> {
    s.object_key(name)?;
    codec.encode(value, s, cx)
}

fn encode_optional_field<C: Encoder<Input: Sized> + ?Sized, S: Serializer + ?Sized>(
    name: &'static str,
    codec: &C,
    value: &Option<C::Input>,
    skip_none: bool,
    s: &mut S,
    cx: &mut EncodeContext,
) -> Result<(), S::Error> {
    match value {
        None if skip_none => Ok(()),
        None => {
            s.object_key(name)?;
            s.write_null()
        }
        Some(v) => encode_field(name, codec, v, s, cx),
    }
}

/// Shared state for one record decode.
pub struct RecordDecodeState<'a, L> {
    pub duplicates: Duplicates,
    pub errs: &'a mut Collector<L>,
}

/// A set of fields reading from (and building) `Self::RecordType`.
pub trait Fields {
    type RecordType;
    type Slots;
    type Values;

    fn new_slots(&self) -> Self::Slots;
    fn names(&self, out: &mut Vec<&'static str>);
    /// How many names [`Fields::names`] reports.
    const NAME_COUNT: usize;
    /// Whether [`Fields::decode_extra`] can claim keys no field names
    /// (only `extras` does). Records skip the call entirely otherwise.
    const CLAIMS_UNKNOWN_KEYS: bool = false;
    /// Decode the value for the `i`th name (compact decoding: the key was
    /// already matched against the name table).
    fn decode_at<R: Reader + ?Sized>(
        &self,
        i: usize,
        r: &mut R,
        cx: &mut DecodeContext,
        slots: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<(), R>;
    /// Decode a key no field claimed (only `extras` does this).
    fn decode_extra<R: Reader + ?Sized>(
        &self,
        _key: &str,
        _r: &mut R,
        _cx: &mut DecodeContext,
        _slots: &mut Self::Slots,
        _record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<bool, R> {
        Ok(false)
    }
    /// Report required fields that are absent, without moving anything.
    /// `false` if any value can't be taken (missing, or failed in
    /// accumulate mode). Checking before moving keeps `take` free of
    /// early exits, whose drop glue would otherwise grow with the square
    /// of the field count.
    fn check(&self, slots: &Self::Slots, missing: &mut dyn FnMut(&'static str)) -> bool;
    /// Move the values out; only called after `check` returned `true`.
    fn take(&self, slots: Self::Slots) -> Self::Values;
    fn count(&self, rec: &Self::RecordType) -> usize;
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &Self::RecordType,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error>;
}

#[cold]
#[inline(never)]
fn missing<L: Clone>(name: &'static str, cx: &DecodeContext, errs: &mut Collector<L>, at: &Option<L>) {
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
    cx: &DecodeContext,
    record_state: &mut RecordDecodeState<'_, R::Location>,
) -> DecodeResult<bool, R> {
    match record_state.duplicates {
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
                record_state.errs.push(e);
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
    cx: &mut DecodeContext,
    slot: &mut Slot<C::Output>,
    record_state: &mut RecordDecodeState<'_, R::Location>,
) -> DecodeResult<(), R> {
    if !matches!(slot, Slot::Empty) && !on_duplicate(name, r, cx, record_state)? {
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
            field_failed(name, r, cx, record_state, cp, e)?;
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
    cx: &DecodeContext,
    record_state: &mut RecordDecodeState<'_, R::Location>,
    cp: Option<R::Checkpoint>,
    e: DecodeError<R::Location>,
) -> DecodeResult<(), R> {
    let e = at_seg(e, cx, || PathSegment::Key(name.into()));
    let Some(cp) = cp else { return Err(e) };
    if e.is_fatal() {
        r.release(cp);
        return Err(e);
    }
    r.rewind(&cp);
    r.release(cp);
    r.skip_value()?;
    record_state.errs.push(e);
    Ok(())
}

// -- field ---------------------------------------------------------------------

/// A required field.
pub struct FieldCodec<C, G, RecordType> {
    name: &'static str,
    codec: C,
    get: G,
    _record: PhantomData<fn(&RecordType)>,
}

impl<C: Copy, G: Copy, RecordType> Copy for FieldCodec<C, G, RecordType> {}
impl<C: Clone, G: Clone, RecordType> Clone for FieldCodec<C, G, RecordType> {
    fn clone(&self) -> Self {
        FieldCodec {
            name: self.name,
            codec: self.codec.clone(),
            get: self.get.clone(),
            _record: PhantomData,
        }
    }
}

/// A required field: name, codec, and a getter the encoder reads it with.
///
pub fn field<RecordType, C, G>(name: &'static str, codec: C, get: G) -> FieldCodec<C, G, RecordType>
where
    C: Codec<Output: Sized>,
    G: Fn(&RecordType) -> &C::Output,
{
    FieldCodec {
            name,
            codec,
            get,
            _record: PhantomData,
        }
}

impl<RecordType, C: Codec<Output: Sized>, G: Fn(&RecordType) -> &C::Output> Fields for FieldCodec<C, G, RecordType> {
    type RecordType = RecordType;
    type Slots = Slot<C::Output>;
    type Values = C::Output;
    const NAME_COUNT: usize = 1;

    fn new_slots(&self) -> Self::Slots {
        Slot::Empty
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        out.push(self.name);
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeContext,
        slot: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<(), R> {
        decode_into(self.name, &self.codec, r, cx, slot, record_state)
    }
    fn check(&self, slot: &Self::Slots, missing: &mut dyn FnMut(&'static str)) -> bool {
        slot.check_required(self.name, missing)
    }
    fn take(&self, slot: Self::Slots) -> Self::Values {
        slot.into_checked()
    }
    fn count(&self, _rec: &RecordType) -> usize {
        1
    }
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &RecordType,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        encode_field(self.name, &self.codec, (self.get)(rec), s, cx)
    }
}

// -- field with a default -------------------------------------------------------

/// A field that takes `default()` when absent. Always encoded.
pub struct DefaultFieldCodec<C, G, D, RecordType> {
    inner: FieldCodec<C, G, RecordType>,
    default: D,
    _record: PhantomData<fn(&RecordType)>,
}

impl<C: Copy, G: Copy, D: Copy, RecordType> Copy for DefaultFieldCodec<C, G, D, RecordType> {}
impl<C: Clone, G: Clone, D: Clone, RecordType> Clone for DefaultFieldCodec<C, G, D, RecordType> {
    fn clone(&self) -> Self {
        DefaultFieldCodec {
            inner: self.inner.clone(),
            default: self.default.clone(),
            _record: PhantomData,
        }
    }
}

pub fn field_default<RecordType, C, G, D>(
    name: &'static str,
    codec: C,
    get: G,
    default: D,
) -> DefaultFieldCodec<C, G, D, RecordType>
where
    C: Codec<Output: Sized>,
    G: Fn(&RecordType) -> &C::Output,
    D: Fn() -> C::Output,
{
    DefaultFieldCodec {
            inner: field(name, codec, get),
            default,
            _record: PhantomData,
        }
}

impl<RecordType, C, G, D> Fields for DefaultFieldCodec<C, G, D, RecordType>
where
    C: Codec<Output: Sized>,
    G: Fn(&RecordType) -> &C::Output,
    D: Fn() -> C::Output,
{
    type RecordType = RecordType;
    type Slots = Slot<C::Output>;
    type Values = C::Output;
    const NAME_COUNT: usize = 1;

    fn new_slots(&self) -> Self::Slots {
        Slot::Empty
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        self.inner.names(out)
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeContext,
        slot: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<(), R> {
        decode_into(self.inner.name, &self.inner.codec, r, cx, slot, record_state)
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
    fn count(&self, rec: &RecordType) -> usize {
        self.inner.count(rec)
    }
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &RecordType,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.inner.encode(rec, s, cx)
    }
}

// -- optional field ------------------------------------------------------------

/// An optional field: absent or `null` decodes to `None`. `None` encodes
/// as `null` unless [`OptionalFieldCodec::skip_none`] is set.
pub struct OptionalFieldCodec<C, G, RecordType> {
    name: &'static str,
    codec: C,
    get: G,
    skip_none: bool,
    _record: PhantomData<fn(&RecordType)>,
}

impl<C: Copy, G: Copy, RecordType> Copy for OptionalFieldCodec<C, G, RecordType> {}
impl<C: Clone, G: Clone, RecordType> Clone for OptionalFieldCodec<C, G, RecordType> {
    fn clone(&self) -> Self {
        OptionalFieldCodec {
            name: self.name,
            codec: self.codec.clone(),
            get: self.get.clone(),
            skip_none: self.skip_none,
            _record: PhantomData,
        }
    }
}

pub fn optional_field<RecordType, C, G>(name: &'static str, codec: C, get: G) -> OptionalFieldCodec<C, G, RecordType>
where
    C: Codec<Output: Sized>,
    G: Fn(&RecordType) -> &Option<C::Output>,
{
    OptionalFieldCodec {
        name,
        codec,
        get,
        skip_none: false,
        _record: PhantomData,
    }
}

impl<C, G, RecordType> OptionalFieldCodec<C, G, RecordType> {
    /// Omit the key entirely for `None` instead of writing `null`.
    pub fn skip_none(mut self) -> Self {
        self.skip_none = true;
        self
    }
}

impl<RecordType, C: Codec<Output: Sized>, G: Fn(&RecordType) -> &Option<C::Output>> Fields
    for OptionalFieldCodec<C, G, RecordType>
{
    type RecordType = RecordType;
    type Slots = Slot<Option<C::Output>>;
    type Values = Option<C::Output>;
    const NAME_COUNT: usize = 1;

    fn new_slots(&self) -> Self::Slots {
        Slot::Empty
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        out.push(self.name);
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeContext,
        slot: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<(), R> {
        decode_into(
            self.name,
            &crate::codec::optional(&self.codec),
            r,
            cx,
            slot,
            record_state,
        )
    }
    fn check(&self, slot: &Self::Slots, _missing: &mut dyn FnMut(&'static str)) -> bool {
        !matches!(slot, Slot::Failed)
    }
    fn take(&self, slot: Self::Slots) -> Self::Values {
        slot.into_checked_or(None)
    }
    fn count(&self, rec: &RecordType) -> usize {
        (!self.skip_none || (self.get)(rec).is_some()) as usize
    }
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &RecordType,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        encode_optional_field(self.name, &self.codec, (self.get)(rec), self.skip_none, s, cx)
    }
}

// -- extras: collect unknown keys ------------------------------------------------

/// Decode state for [`extras`]: the entries so far, and whether one of
/// them failed (accumulate mode).
#[derive(Debug)]
pub struct ExtrasSlot<V> {
    entries: Vec<(String, V)>,
    failed: bool,
}

/// Collects every key no other field claims into `Vec<(String, V)>`,
/// decoded with `codec` (e.g. a dynamic `Value` codec).
pub struct ExtrasCodec<C, G, RecordType> {
    codec: C,
    get: G,
    _record: PhantomData<fn(&RecordType)>,
}

impl<C: Copy, G: Copy, RecordType> Copy for ExtrasCodec<C, G, RecordType> {}
impl<C: Clone, G: Clone, RecordType> Clone for ExtrasCodec<C, G, RecordType> {
    fn clone(&self) -> Self {
        ExtrasCodec {
            codec: self.codec.clone(),
            get: self.get.clone(),
            _record: PhantomData,
        }
    }
}

pub fn extras<RecordType, C, G>(codec: C, get: G) -> ExtrasCodec<C, G, RecordType>
where
    C: Codec<Output: Sized>,
    G: Fn(&RecordType) -> &Vec<(String, C::Output)>,
{
    ExtrasCodec {
        codec,
        get,
        _record: PhantomData,
    }
}

impl<RecordType, C: Codec<Output: Sized>, G: Fn(&RecordType) -> &Vec<(String, C::Output)>> Fields
    for ExtrasCodec<C, G, RecordType>
{
    type RecordType = RecordType;
    type Slots = ExtrasSlot<C::Output>;
    type Values = Vec<(String, C::Output)>;
    const NAME_COUNT: usize = 0;
    const CLAIMS_UNKNOWN_KEYS: bool = true;

    fn new_slots(&self) -> Self::Slots {
        ExtrasSlot { entries: Vec::new(), failed: false }
    }
    fn names(&self, _out: &mut Vec<&'static str>) {}
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        r: &mut R,
        cx: &mut DecodeContext,
        slot: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<(), R> {
        let _ = (r, cx, slot, record_state);
        Ok(())
    }
    fn decode_extra<R: Reader + ?Sized>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut DecodeContext,
        slots: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<bool, R> {
        let k = String::from(key);
        match child(
            r,
            cx,
            record_state.errs,
            || PathSegment::Key(k.clone()),
            |r, cx| decode_child(&self.codec, r, cx),
        )? {
            Some(v) => slots.entries.push((k, v)),
            None => slots.failed = true,
        }
        Ok(true)
    }
    fn check(&self, slots: &Self::Slots, _missing: &mut dyn FnMut(&'static str)) -> bool {
        !slots.failed
    }
    fn take(&self, slots: Self::Slots) -> Self::Values {
        slots.entries
    }
    fn count(&self, rec: &RecordType) -> usize {
        (self.get)(rec).len()
    }
    fn encode<S: Serializer + ?Sized>(
        &self,
        rec: &RecordType,
        s: &mut S,
        cx: &mut EncodeContext,
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
    type Fields: Fields<RecordType = Self::Output>;
    fn fields(&self) -> &Self::Fields;
    /// Build the record from slots that passed [`Fields::check`].
    fn build_slots(&self, slots: <Self::Fields as Fields>::Slots) -> Self::Output;
}

pub struct FieldsOfCodec<InnerRecord, G, Outer> {
    inner: InnerRecord,
    get: G,
    _record: PhantomData<fn(&Outer)>,
}

impl<InnerRecord: Copy, G: Copy, Outer> Copy for FieldsOfCodec<InnerRecord, G, Outer> {}
impl<InnerRecord: Clone, G: Clone, Outer> Clone for FieldsOfCodec<InnerRecord, G, Outer> {
    fn clone(&self) -> Self {
        FieldsOfCodec {
            inner: self.inner.clone(),
            get: self.get.clone(),
            _record: PhantomData,
        }
    }
}

/// Read another record's fields from the *current* object: explicit
/// flattening. Both field sets are static, so unknown-field checks see
/// the merged set.
pub fn fields_of<Outer, InnerRecord, G>(inner: InnerRecord, get: G) -> FieldsOfCodec<InnerRecord, G, Outer>
where
    InnerRecord: RecordCodec,
    G: Fn(&Outer) -> &InnerRecord::Output,
{
    FieldsOfCodec {
        inner,
        get,
        _record: PhantomData,
    }
}

impl<Outer, InnerRecord: RecordCodec, G: Fn(&Outer) -> &InnerRecord::Output> Fields for FieldsOfCodec<InnerRecord, G, Outer> {
    type RecordType = Outer;
    type Slots = <InnerRecord::Fields as Fields>::Slots;
    type Values = InnerRecord::Output;
    const NAME_COUNT: usize = <InnerRecord::Fields as Fields>::NAME_COUNT;
    const CLAIMS_UNKNOWN_KEYS: bool = <InnerRecord::Fields as Fields>::CLAIMS_UNKNOWN_KEYS;

    fn new_slots(&self) -> Self::Slots {
        self.inner.fields().new_slots()
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        self.inner.fields().names(out)
    }
    #[inline]
    fn decode_at<R: Reader + ?Sized>(
        &self,
        i: usize,
        r: &mut R,
        cx: &mut DecodeContext,
        slots: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<(), R> {
        self.inner.fields().decode_at(i, r, cx, slots, record_state)
    }
    fn decode_extra<R: Reader + ?Sized>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut DecodeContext,
        slots: &mut Self::Slots,
        record_state: &mut RecordDecodeState<'_, R::Location>,
    ) -> DecodeResult<bool, R> {
        self.inner.fields().decode_extra(key, r, cx, slots, record_state)
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
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.inner.fields().encode((self.get)(rec), s, cx)
    }
}

// -- tuples of field codecs ----------------------------------------------------

macro_rules! tuple_fields {
    ($T0:ident 0 $(, $T:ident $i:tt)*) => {
        impl<$T0: Fields, $($T: Fields<RecordType = $T0::RecordType>),*> Fields for ($T0, $($T,)*) {
            type RecordType = $T0::RecordType;
            type Slots = ($T0::Slots, $($T::Slots,)*);
            type Values = ($T0::Values, $($T::Values,)*);
            const NAME_COUNT: usize = $T0::NAME_COUNT $(+ $T::NAME_COUNT)*;
            const CLAIMS_UNKNOWN_KEYS: bool = $T0::CLAIMS_UNKNOWN_KEYS $(|| $T::CLAIMS_UNKNOWN_KEYS)*;

            fn new_slots(&self) -> Self::Slots {
                (self.0.new_slots(), $(self.$i.new_slots(),)*)
            }
            fn names(&self, out: &mut Vec<&'static str>) {
                self.0.names(out);
                $(self.$i.names(out);)*
            }
            #[inline]
            fn decode_at<R: Reader + ?Sized>(
                &self,
                i: usize,
                r: &mut R,
                cx: &mut DecodeContext,
                slots: &mut Self::Slots,
                record_state: &mut RecordDecodeState<'_, R::Location>,
            ) -> DecodeResult<(), R> {
                #[allow(unused_mut)]
                let mut i = i;
                if i < $T0::NAME_COUNT {
                    return self.0.decode_at(i, r, cx, &mut slots.0, record_state);
                }
                i -= $T0::NAME_COUNT;
                $(
                    if i < $T::NAME_COUNT {
                        return self.$i.decode_at(i, r, cx, &mut slots.$i, record_state);
                    }
                    i -= $T::NAME_COUNT;
                )*
                let _ = i;
                Ok(())
            }
            fn decode_extra<R: Reader + ?Sized>(
                &self,
                key: &str,
                r: &mut R,
                cx: &mut DecodeContext,
                slots: &mut Self::Slots,
                record_state: &mut RecordDecodeState<'_, R::Location>,
            ) -> DecodeResult<bool, R> {
                if $T0::CLAIMS_UNKNOWN_KEYS {
                    if self.0.decode_extra(key, r, cx, &mut slots.0, record_state)? {
                        return Ok(true);
                    }
                }
                $(if $T::CLAIMS_UNKNOWN_KEYS {
                    if self.$i.decode_extra(key, r, cx, &mut slots.$i, record_state)? {
                        return Ok(true);
                    }
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
            fn count(&self, rec: &Self::RecordType) -> usize {
                self.0.count(rec) $(+ self.$i.count(rec))*
            }
            #[inline]
            fn encode<S: Serializer + ?Sized>(&self, rec: &Self::RecordType, s: &mut S, cx: &mut EncodeContext) -> Result<(), S::Error> {
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

/// Start a record from a tuple of field codecs; finish with
/// `.build(|(a, b, …)| …)`, which receives the values as a tuple of the
/// same shape.
pub fn record<F: Fields>(fields: F) -> RecordBuilder<F> {
    RecordBuilder { fields }
}

impl<F: Fields> RecordBuilder<F> {
    pub fn build<B>(self, build: B) -> Record<F, TupleBuild<B>>
    where
        B: Fn(F::Values) -> F::RecordType,
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
    fn build(&self, fields: &F, slots: F::Slots) -> F::RecordType;
}

/// `record((...)).build(|(a, b, ...)| ...)`: the values as a tuple.
#[derive(Debug, Clone, Copy)]
pub struct TupleBuild<B>(B);

impl<F: Fields, B: Fn(F::Values) -> F::RecordType> Build<F> for TupleBuild<B> {
    #[inline]
    fn build(&self, fields: &F, slots: F::Slots) -> F::RecordType {
        (self.0)(fields.take(slots))
    }
}

#[derive(Clone, Copy)]
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
        cx: &DecodeContext,
        mut errs: Collector<L>,
        at: &Option<L>,
    ) -> Result<F::RecordType, DecodeError<L>> {
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
        &mut DecodeContext,
        &mut Collector<<R as Reader>::Location>,
    ) -> DecodeResult<bool, R>
    + 'a;

/// The record key loop shared by every compact record decode.
#[inline(never)]
fn record_loop<R: Reader + ?Sized>(
    r: &mut R,
    cx: &mut DecodeContext,
    names: &[&'static str],
    allow_unknown: bool,
    errs: &mut Collector<R::Location>,
    on_key: &mut OnKey<'_, R>,
) -> DecodeResult<(), R> {
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
    type Output = F::RecordType;

    /// The key loop is shared by every record (generic only over the
    /// reader); per record there is just the index dispatch, the
    /// missing-field check and `build`.
    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<F::RecordType, R> {
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
                let mut record_state = RecordDecodeState { duplicates, errs };
                match found {
                    Some(i) => self
                        .fields
                        .decode_at(i, r, cx, &mut slots, &mut record_state)
                        .map(|()| true),
                    None if F::CLAIMS_UNKNOWN_KEYS => {
                        self.fields.decode_extra(key, r, cx, &mut slots, &mut record_state)
                    }
                    None => Ok(false),
                }
            },
        );
        cx.put_name_buf(names);
        looped?;
        self.complete(slots, cx, errs, &at)
    }
}

impl<F: Fields, B: Build<F>> Encoder for Record<F, B> {
    type Input = F::RecordType;

    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &F::RecordType,
        s: &mut S,
        cx: &mut EncodeContext,
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

impl<F: Fields<RecordType: Sized>, B: Build<F>> RecordCodec for Record<F, B> {
    type Fields = F;
    fn fields(&self) -> &F {
        &self.fields
    }
    fn build_slots(&self, slots: F::Slots) -> F::RecordType {
        self.build.build(&self.fields, slots)
    }
}

/// Whether the next value is an object (used by dispatchers).
pub(crate) fn expect_object<R: Reader + ?Sized>(r: &mut R) -> DecodeResult<(), R> {
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
pub struct FieldsFor<RecordType>(PhantomData<fn(&RecordType)>);

impl<RecordType> Clone for FieldsFor<RecordType> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<RecordType> Copy for FieldsFor<RecordType> {}

pub const fn fields_for<RecordType>() -> FieldsFor<RecordType> {
    FieldsFor(PhantomData)
}

impl<RecordType> FieldsFor<RecordType> {
    #[allow(clippy::type_complexity)]
    pub fn field<C, G>(self, name: &'static str, codec: C, get: G) -> FieldCodec<C, G, RecordType>
    where
        C: Codec<Output: Sized>,
        G: Fn(&RecordType) -> &C::Output,
    {
        field(name, codec, get)
    }

    #[allow(clippy::type_complexity)]
    pub fn optional<C, G>(self, name: &'static str, codec: C, get: G) -> OptionalFieldCodec<C, G, RecordType>
    where
        C: Codec<Output: Sized>,
        G: Fn(&RecordType) -> &Option<C::Output>,
    {
        optional_field(name, codec, get)
    }

    #[allow(clippy::type_complexity)]
    pub fn default<C, G, D>(
        self,
        name: &'static str,
        codec: C,
        get: G,
        default: D,
    ) -> DefaultFieldCodec<C, G, D, RecordType>
    where
        C: Codec<Output: Sized>,
        G: Fn(&RecordType) -> &C::Output,
        D: Fn() -> C::Output,
    {
        field_default(name, codec, get, default)
    }

    #[allow(clippy::type_complexity)]
    pub fn extras<C, G>(self, codec: C, get: G) -> ExtrasCodec<C, G, RecordType>
    where
        C: Codec<Output: Sized>,
        G: Fn(&RecordType) -> &Vec<(String, C::Output)>,
    {
        extras(codec, get)
    }

    #[allow(clippy::type_complexity)]
    pub fn flatten<InnerRecord, G>(self, inner: InnerRecord, get: G) -> FieldsOfCodec<InnerRecord, G, RecordType>
    where
        InnerRecord: RecordCodec,
        G: Fn(&RecordType) -> &InnerRecord::Output,
    {
        fields_of(inner, get)
    }
}
