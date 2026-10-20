//! Records built field by field, with typed keys instead of a positional
//! tuple pattern:
//!
//! ```ignore
//! fn user() -> impl Codec<Output = User> {
//!     let r = keyed_record::<User>();
//!     let (r, id) = r.field("id", u32(), |u| &u.id);
//!     let (r, name) = r.field("name", string(), |u| &u.name);
//!     let (r, email) = r.optional_field("email", string(), |u| &u.email);
//!     r.build(move |v| User {
//!         id: v.take(id),
//!         name: v.take(name),
//!         email: v.take(email),
//!     })
//! }
//! ```
//!
//! There is no limit on the number of fields, no tuple pattern to keep in
//! step with the field list, and getters need no type annotations. The
//! struct literal still has to name every field, and a value can only be
//! obtained with a key, so a forgotten field is a compile error.
//!
//! How it works: fields are kept in flat tuples ("chunks") of up to 16,
//! and chunks in a short right-nested list `(C0, (C1, (…, End<Rec>)))`.
//! Each `.field(…)` returns a [`Key`] whose type records the field's chunk
//! and slot. [`Taker::take`] follows that at compile time (a few chunk
//! steps, then a direct slot access), so values stay in typed storage: no
//! type erasure, no allocation, no `unsafe`. Chunking keeps the number of
//! generic instances, and so compile time, close to tuple records.
//!
//! The per-chunk-size impls at the bottom are generated (see the comment
//! there).

use crate::codec::{Codec, DResult};
use crate::context::{DecodeCx, EncodeCx};
use crate::reader::Reader;
use crate::record::{
    Build, DefaultField, Duplicates, Extras, Field, Fields, FieldsOf, KeyCx, OptField, Record,
    RecordCodec, extras, field, field_default, fields_of, optional_field,
};
use crate::serializer::Serializer;
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

/// Type-level chunk number: the first chunk.
#[derive(Debug, Clone, Copy)]
pub enum Here {}

/// Type-level chunk number: one after `N`.
#[derive(Debug, Clone, Copy)]
pub struct Next<N>(PhantomData<N>);

/// A field's position: chunk `C`, slot `S` within it.
#[derive(Debug, Clone, Copy)]
pub struct At<C, S>(PhantomData<(C, S)>);

/// A typed handle to one declared field: its value type `T` and its
/// position `P`.
pub struct Key<T, P> {
    _p: PhantomData<fn() -> (T, P)>,
}

impl<T, P> Clone for Key<T, P> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, P> Copy for Key<T, P> {}

impl<T, P> Key<T, P> {
    const fn new() -> Self {
        Key { _p: PhantomData }
    }
}

/// The end of a keyed record's chunk list.
pub struct End<Rec>(PhantomData<fn(&Rec)>);

impl<Rec> Clone for End<Rec> {
    fn clone(&self) -> Self {
        End(PhantomData)
    }
}

impl<Rec> Fields for End<Rec> {
    type Rec = Rec;
    type Slots = ();
    type Values = ();
    fn new_slots(&self) {}
    fn names(&self, _out: &mut Vec<&'static str>) {}
    fn name_count(&self) -> usize {
        0
    }
    fn decode_at<R: Reader + ?Sized>(
        &self,
        _i: usize,
        _r: &mut R,
        _cx: &mut DecodeCx,
        _slots: &mut (),
        _kcx: &mut KeyCx<'_, R::Location>,
    ) -> DResult<(), R> {
        Ok(())
    }
    fn check(&self, _slots: &(), _missing: &mut dyn FnMut(&'static str)) -> bool {
        true
    }
    fn take(&self, _slots: ()) {}
    fn count(&self, _rec: &Rec) -> usize {
        0
    }
    fn encode<S: Serializer + ?Sized>(
        &self,
        _rec: &Rec,
        _s: &mut S,
        _cx: &mut EncodeCx,
    ) -> Result<(), S::Error> {
        Ok(())
    }
}

/// Type-level append to the chunk list.
pub trait Append<G> {
    type Out;
    fn append(self, g: G) -> Self::Out;
}

impl<Rec, G> Append<G> for End<Rec> {
    type Out = (G, End<Rec>);
    #[inline]
    fn append(self, g: G) -> Self::Out {
        (g, self)
    }
}

impl<A, B: Append<G>, G> Append<G> for (A, B) {
    type Out = (A, B::Out);
    #[inline]
    fn append(self, g: G) -> Self::Out {
        (self.0, self.1.append(g))
    }
}

/// The number of chunks in a chunk list, as `Here` (0), `Next<Here>` (1), …
pub trait ChunkCount {
    type N;
}
impl<Rec> ChunkCount for End<Rec> {
    type N = Here;
}
impl<A, B: ChunkCount> ChunkCount for (A, B) {
    type N = Next<B::N>;
}

/// Move the value in slot `S` of one chunk (a flat tuple) out of its slots.
pub trait TakeSlot<S>: Fields {
    type Out;
    fn take_slot(&self, slots: &mut Self::Slots) -> Self::Out;
}

/// Move the value at position `P` out of a chunk list's slots.
pub trait TakeAt<P>: Fields {
    type Out;
    fn take_at(&self, slots: &mut Self::Slots) -> Self::Out;
}

impl<S, A: TakeSlot<S>, B: Fields<Rec = A::Rec>> TakeAt<At<Here, S>> for (A, B) {
    type Out = A::Out;
    #[inline]
    fn take_at(&self, slots: &mut Self::Slots) -> A::Out {
        self.0.take_slot(&mut slots.0)
    }
}

impl<N, S, A: Fields, B: Fields<Rec = A::Rec> + TakeAt<At<N, S>>> TakeAt<At<Next<N>, S>> for (A, B) {
    type Out = B::Out;
    #[inline]
    fn take_at(&self, slots: &mut Self::Slots) -> B::Out {
        self.1.take_at(&mut slots.1)
    }
}

/// Hands decoded values to the `build` closure, one key at a time.
pub struct Taker<'a, F: Fields> {
    fields: &'a F,
    slots: F::Slots,
}

impl<F: Fields> Taker<'_, F> {
    /// Move a field's value out. Taking the same key twice yields the
    /// field's "absent" value (a panic for required fields).
    #[inline]
    pub fn take<T, P>(&mut self, _key: Key<T, P>) -> T
    where
        F: TakeAt<P, Out = T>,
    {
        self.fields.take_at(&mut self.slots)
    }
}

/// `keyed_record::<Rec>()…build(|v| Rec { … })`: see the module docs.
#[derive(Clone, Copy)]
pub struct KeyedBuild<B>(B);

impl<F: Fields, B: Fn(&mut Taker<'_, F>) -> F::Rec> Build<F> for KeyedBuild<B> {
    #[inline]
    fn build(&self, fields: &F, slots: F::Slots) -> F::Rec {
        (self.0)(&mut Taker { fields, slots })
    }
}

/// A keyed record under construction: `Done` is the list of full chunks,
/// `Cur` the chunk being filled (a flat tuple of 0–16 fields).
pub struct KeyedRecord<Rec, Done, Cur> {
    done: Done,
    cur: Cur,
    _p: PhantomData<fn(&Rec)>,
}

/// Start a keyed record for `Rec`.
pub fn keyed_record<Rec>() -> KeyedRecord<Rec, End<Rec>, ()> {
    KeyedRecord {
        done: End(PhantomData),
        cur: (),
        _p: PhantomData,
    }
}

/// Add one field; `Pos` says where it landed. Implemented per size of the
/// current chunk.
pub trait Push<G> {
    type Out;
    type Pos;
    fn push(self, g: G) -> Self::Out;
}

/// Close the builder: append the current chunk (if any) to the list.
pub trait Finish {
    type Fields: Fields;
    fn finish(self) -> Self::Fields;
}

impl<Rec, Done, Cur> KeyedRecord<Rec, Done, Cur> {
    /// A required field.
    #[allow(clippy::type_complexity)]
    pub fn field<C, G>(
        self,
        name: &'static str,
        codec: C,
        get: G,
    ) -> (
        <Self as Push<Field<C, G, Rec>>>::Out,
        Key<C::Output, <Self as Push<Field<C, G, Rec>>>::Pos>,
    )
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &C::Output,
        Self: Push<Field<C, G, Rec>>,
    {
        (self.push(field(name, codec, get)), Key::new())
    }

    /// An optional field: absent or `null` decodes to `None`.
    #[allow(clippy::type_complexity)]
    pub fn optional_field<C, G>(
        self,
        name: &'static str,
        codec: C,
        get: G,
    ) -> (
        <Self as Push<OptField<C, G, Rec>>>::Out,
        Key<Option<C::Output>, <Self as Push<OptField<C, G, Rec>>>::Pos>,
    )
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &Option<C::Output>,
        Self: Push<OptField<C, G, Rec>>,
    {
        (self.push(optional_field(name, codec, get)), Key::new())
    }

    /// A field that takes `default()` when absent.
    #[allow(clippy::type_complexity)]
    pub fn field_default<C, G, D>(
        self,
        name: &'static str,
        codec: C,
        get: G,
        default: D,
    ) -> (
        <Self as Push<DefaultField<C, G, D, Rec>>>::Out,
        Key<C::Output, <Self as Push<DefaultField<C, G, D, Rec>>>::Pos>,
    )
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &C::Output,
        D: Fn() -> C::Output,
        Self: Push<DefaultField<C, G, D, Rec>>,
    {
        (self.push(field_default(name, codec, get, default)), Key::new())
    }

    /// Collect keys no other field claims.
    #[allow(clippy::type_complexity)]
    pub fn extras<C, G>(
        self,
        codec: C,
        get: G,
    ) -> (
        <Self as Push<Extras<C, G, Rec>>>::Out,
        Key<Vec<(String, C::Output)>, <Self as Push<Extras<C, G, Rec>>>::Pos>,
    )
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &Vec<(String, C::Output)>,
        Self: Push<Extras<C, G, Rec>>,
    {
        (self.push(extras(codec, get)), Key::new())
    }

    /// Read another record's fields from this object.
    #[allow(clippy::type_complexity)]
    pub fn fields_of<RC, G>(
        self,
        inner: RC,
        get: G,
    ) -> (
        <Self as Push<FieldsOf<RC, G, Rec>>>::Out,
        Key<RC::Output, <Self as Push<FieldsOf<RC, G, Rec>>>::Pos>,
    )
    where
        RC: RecordCodec,
        G: Fn(&Rec) -> &RC::Output,
        Self: Push<FieldsOf<RC, G, Rec>>,
    {
        (self.push(fields_of(inner, get)), Key::new())
    }

    /// Finish: `build` assembles the record from the keys.
    pub fn build<B>(self, build: B) -> Record<<Self as Finish>::Fields, KeyedBuild<B>>
    where
        Self: Finish,
        <Self as Finish>::Fields: Fields<Rec = Rec>,
        B: Fn(&mut Taker<'_, <Self as Finish>::Fields>) -> Rec,
    {
        Record::from_parts(self.finish(), KeyedBuild(build), false, Duplicates::Error)
    }
}

// -- generated: per-chunk-size impls (Push, Finish, TakeSlot) ---------------------------
// Generated by a script for chunk sizes 0..=16; edit the generator, not the output.

/// Type-level slot 0 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S0 {}

/// Type-level slot 1 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S1 {}

/// Type-level slot 2 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S2 {}

/// Type-level slot 3 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S3 {}

/// Type-level slot 4 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S4 {}

/// Type-level slot 5 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S5 {}

/// Type-level slot 6 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S6 {}

/// Type-level slot 7 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S7 {}

/// Type-level slot 8 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S8 {}

/// Type-level slot 9 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S9 {}

/// Type-level slot 10 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S10 {}

/// Type-level slot 11 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S11 {}

/// Type-level slot 12 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S12 {}

/// Type-level slot 13 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S13 {}

/// Type-level slot 14 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S14 {}

/// Type-level slot 15 within a chunk.
#[derive(Debug, Clone, Copy)]
pub enum S15 {}

impl<Rec, Done: ChunkCount, G> Push<G> for KeyedRecord<Rec, Done, ()> {
    type Out = KeyedRecord<Rec, Done, (G,)>;
    type Pos = At<Done::N, S0>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let () = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done: Fields<Rec = Rec>> Finish for KeyedRecord<Rec, Done, ()> {
    type Fields = Done;
    fn finish(self) -> Done {
        self.done
    }
}

impl<Rec, Done: ChunkCount, T0, G> Push<G> for KeyedRecord<Rec, Done, (T0,)> {
    type Out = KeyedRecord<Rec, Done, (T0, G,)>;
    type Pos = At<Done::N, S1>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0> Finish for KeyedRecord<Rec, Done, (T0,)>
where
    Done: Append<(T0,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields> TakeSlot<S0> for (T0,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, G,)>;
    type Pos = At<Done::N, S2>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1> Finish for KeyedRecord<Rec, Done, (T0, T1,)>
where
    Done: Append<(T0, T1,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, G,)>;
    type Pos = At<Done::N, S3>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2> Finish for KeyedRecord<Rec, Done, (T0, T1, T2,)>
where
    Done: Append<(T0, T1, T2,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, G,)>;
    type Pos = At<Done::N, S4>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3,)>
where
    Done: Append<(T0, T1, T2, T3,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, G,)>;
    type Pos = At<Done::N, S5>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4,)>
where
    Done: Append<(T0, T1, T2, T3, T4,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, G,)>;
    type Pos = At<Done::N, S6>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, G,)>;
    type Pos = At<Done::N, S7>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, G,)>;
    type Pos = At<Done::N, S8>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, G,)>;
    type Pos = At<Done::N, S9>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, G,)>;
    type Pos = At<Done::N, S10>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, G,)>;
    type Pos = At<Done::N, S11>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>> TakeSlot<S10> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10,) {
    type Out = T10::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T10::Values {
        let slot = core::mem::replace(&mut slots.10, self.10.new_slots());
        self.10.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, G,)>;
    type Pos = At<Done::N, S12>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S10> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T10::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T10::Values {
        let slot = core::mem::replace(&mut slots.10, self.10.new_slots());
        self.10.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>> TakeSlot<S11> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11,) {
    type Out = T11::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T11::Values {
        let slot = core::mem::replace(&mut slots.11, self.11.new_slots());
        self.11.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, G,)>;
    type Pos = At<Done::N, S13>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S10> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T10::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T10::Values {
        let slot = core::mem::replace(&mut slots.10, self.10.new_slots());
        self.10.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S11> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T11::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T11::Values {
        let slot = core::mem::replace(&mut slots.11, self.11.new_slots());
        self.11.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>> TakeSlot<S12> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12,) {
    type Out = T12::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T12::Values {
        let slot = core::mem::replace(&mut slots.12, self.12.new_slots());
        self.12.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, G,)>;
    type Pos = At<Done::N, S14>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S10> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T10::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T10::Values {
        let slot = core::mem::replace(&mut slots.10, self.10.new_slots());
        self.10.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S11> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T11::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T11::Values {
        let slot = core::mem::replace(&mut slots.11, self.11.new_slots());
        self.11.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S12> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T12::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T12::Values {
        let slot = core::mem::replace(&mut slots.12, self.12.new_slots());
        self.12.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>> TakeSlot<S13> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13,) {
    type Out = T13::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T13::Values {
        let slot = core::mem::replace(&mut slots.13, self.13.new_slots());
        self.13.take(slot)
    }
}

impl<Rec, Done: ChunkCount, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,)> {
    type Out = KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, G,)>;
    type Pos = At<Done::N, S15>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        let (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14,) = self.cur;
        KeyedRecord {
            done: self.done,
            cur: (t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S10> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T10::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T10::Values {
        let slot = core::mem::replace(&mut slots.10, self.10.new_slots());
        self.10.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S11> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T11::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T11::Values {
        let slot = core::mem::replace(&mut slots.11, self.11.new_slots());
        self.11.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S12> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T12::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T12::Values {
        let slot = core::mem::replace(&mut slots.12, self.12.new_slots());
        self.12.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S13> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T13::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T13::Values {
        let slot = core::mem::replace(&mut slots.13, self.13.new_slots());
        self.13.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>> TakeSlot<S14> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14,) {
    type Out = T14::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T14::Values {
        let slot = core::mem::replace(&mut slots.14, self.14.new_slots());
        self.14.take(slot)
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15, G> Push<G> for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,)>,
    Done::Out: ChunkCount,
{
    type Out = KeyedRecord<Rec, Done::Out, (G,)>;
    type Pos = At<<Done::Out as ChunkCount>::N, S0>;
    #[inline]
    fn push(self, g: G) -> Self::Out {
        KeyedRecord {
            done: self.done.append(self.cur),
            cur: (g,),
            _p: PhantomData,
        }
    }
}

impl<Rec, Done, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15> Finish for KeyedRecord<Rec, Done, (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,)>
where
    Done: Append<(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,)>,
    Done::Out: Fields<Rec = Rec>,
{
    type Fields = Done::Out;
    fn finish(self) -> Done::Out {
        self.done.append(self.cur)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S0> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T0::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T0::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S1> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T1::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T1::Values {
        let slot = core::mem::replace(&mut slots.1, self.1.new_slots());
        self.1.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S2> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T2::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T2::Values {
        let slot = core::mem::replace(&mut slots.2, self.2.new_slots());
        self.2.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S3> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T3::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T3::Values {
        let slot = core::mem::replace(&mut slots.3, self.3.new_slots());
        self.3.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S4> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T4::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T4::Values {
        let slot = core::mem::replace(&mut slots.4, self.4.new_slots());
        self.4.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S5> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T5::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T5::Values {
        let slot = core::mem::replace(&mut slots.5, self.5.new_slots());
        self.5.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S6> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T6::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T6::Values {
        let slot = core::mem::replace(&mut slots.6, self.6.new_slots());
        self.6.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S7> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T7::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T7::Values {
        let slot = core::mem::replace(&mut slots.7, self.7.new_slots());
        self.7.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S8> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T8::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T8::Values {
        let slot = core::mem::replace(&mut slots.8, self.8.new_slots());
        self.8.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S9> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T9::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T9::Values {
        let slot = core::mem::replace(&mut slots.9, self.9.new_slots());
        self.9.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S10> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T10::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T10::Values {
        let slot = core::mem::replace(&mut slots.10, self.10.new_slots());
        self.10.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S11> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T11::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T11::Values {
        let slot = core::mem::replace(&mut slots.11, self.11.new_slots());
        self.11.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S12> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T12::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T12::Values {
        let slot = core::mem::replace(&mut slots.12, self.12.new_slots());
        self.12.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S13> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T13::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T13::Values {
        let slot = core::mem::replace(&mut slots.13, self.13.new_slots());
        self.13.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S14> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T14::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T14::Values {
        let slot = core::mem::replace(&mut slots.14, self.14.new_slots());
        self.14.take(slot)
    }
}

impl<T0: Fields, T1: Fields<Rec = T0::Rec>, T2: Fields<Rec = T0::Rec>, T3: Fields<Rec = T0::Rec>, T4: Fields<Rec = T0::Rec>, T5: Fields<Rec = T0::Rec>, T6: Fields<Rec = T0::Rec>, T7: Fields<Rec = T0::Rec>, T8: Fields<Rec = T0::Rec>, T9: Fields<Rec = T0::Rec>, T10: Fields<Rec = T0::Rec>, T11: Fields<Rec = T0::Rec>, T12: Fields<Rec = T0::Rec>, T13: Fields<Rec = T0::Rec>, T14: Fields<Rec = T0::Rec>, T15: Fields<Rec = T0::Rec>> TakeSlot<S15> for (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15,) {
    type Out = T15::Values;
    #[inline]
    fn take_slot(&self, slots: &mut Self::Slots) -> T15::Values {
        let slot = core::mem::replace(&mut slots.15, self.15.new_slots());
        self.15.take(slot)
    }
}

