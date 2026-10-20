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
//! How it works: each `.field(…)` appends to a right-nested list
//! `(F0, (F1, (…, End<Rec>)))` whose type the compiler infers, and returns
//! a [`Key`] whose type records the field's position (`Here`,
//! `Next<Here>`, …). [`Taker::take`] follows that position at compile time,
//! so values stay in typed storage: no type erasure, no allocation, no
//! `unsafe`. Decoding uses the same machinery as tuple records.

use crate::codec::{Codec, DResult};
use crate::context::{DecodeCx, EncodeCx};
use crate::reader::Reader;
use crate::record::{
    Build, DefaultField, Duplicates, Extras, Field, Fields, FieldsOf, KeyCx, OptField, Record,
    RecordCodec, field, field_default, fields_of, optional_field, extras,
};
use crate::serializer::Serializer;
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

/// Type-level position: the first element.
#[derive(Debug, Clone, Copy)]
pub enum Here {}

/// Type-level position: one after `N`.
#[derive(Debug, Clone, Copy)]
pub struct Next<N>(PhantomData<N>);

/// A typed handle to one declared field: its value type `T` and its
/// position `N` in the record's field list.
pub struct Key<T, N> {
    _p: PhantomData<fn() -> (T, N)>,
}

impl<T, N> Clone for Key<T, N> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, N> Copy for Key<T, N> {}

impl<T, N> Key<T, N> {
    const fn new() -> Self {
        Key { _p: PhantomData }
    }
}

/// The end of a keyed record's field list.
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
    fn encode<S: Serializer + ?Sized>(&self, _rec: &Rec, _s: &mut S, _cx: &mut EncodeCx) -> Result<(), S::Error> {
        Ok(())
    }
}

/// Type-level append to a right-nested field list.
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

/// Move the value at position `N` out of a field list's slots.
pub trait TakeAt<N>: Fields {
    type Out;
    fn take_at(&self, slots: &mut Self::Slots) -> Self::Out;
}

impl<A: Fields, B: Fields<Rec = A::Rec>> TakeAt<Here> for (A, B) {
    type Out = A::Values;
    #[inline]
    fn take_at(&self, slots: &mut Self::Slots) -> A::Values {
        let slot = core::mem::replace(&mut slots.0, self.0.new_slots());
        self.0.take(slot)
    }
}

impl<N, A: Fields, B: Fields<Rec = A::Rec> + TakeAt<N>> TakeAt<Next<N>> for (A, B) {
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
    pub fn take<T, N>(&mut self, _key: Key<T, N>) -> T
    where
        F: TakeAt<N, Out = T>,
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

/// A keyed record under construction: `F` is the field list so far, `N`
/// the position the next field will get.
pub struct KeyedRecord<Rec, F, N> {
    fields: F,
    _p: PhantomData<fn(&Rec) -> N>,
}

/// Start a keyed record for `Rec`.
pub fn keyed_record<Rec>() -> KeyedRecord<Rec, End<Rec>, Here> {
    KeyedRecord { fields: End(PhantomData), _p: PhantomData }
}

impl<Rec, F, N> KeyedRecord<Rec, F, N> {
    fn push<G>(self, g: G) -> KeyedRecord<Rec, F::Out, Next<N>>
    where
        F: Append<G>,
    {
        KeyedRecord { fields: self.fields.append(g), _p: PhantomData }
    }

    /// A required field.
    #[allow(clippy::type_complexity)]
    pub fn field<C, G>(
        self,
        name: &'static str,
        codec: C,
        get: G,
    ) -> (KeyedRecord<Rec, F::Out, Next<N>>, Key<C::Output, N>)
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &C::Output,
        F: Append<Field<C, G, Rec>>,
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
    ) -> (KeyedRecord<Rec, F::Out, Next<N>>, Key<Option<C::Output>, N>)
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &Option<C::Output>,
        F: Append<OptField<C, G, Rec>>,
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
    ) -> (KeyedRecord<Rec, F::Out, Next<N>>, Key<C::Output, N>)
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &C::Output,
        D: Fn() -> C::Output,
        F: Append<DefaultField<C, G, D, Rec>>,
    {
        (self.push(field_default(name, codec, get, default)), Key::new())
    }

    /// Collect keys no other field claims.
    #[allow(clippy::type_complexity)]
    pub fn extras<C, G>(self, codec: C, get: G) -> (KeyedRecord<Rec, F::Out, Next<N>>, Key<Vec<(String, C::Output)>, N>)
    where
        C: Codec<Output: Sized>,
        G: Fn(&Rec) -> &Vec<(String, C::Output)>,
        F: Append<Extras<C, G, Rec>>,
    {
        (self.push(extras(codec, get)), Key::new())
    }

    /// Read another record's fields from this object.
    #[allow(clippy::type_complexity)]
    pub fn fields_of<RC, G>(self, inner: RC, get: G) -> (KeyedRecord<Rec, F::Out, Next<N>>, Key<RC::Output, N>)
    where
        RC: RecordCodec,
        G: Fn(&Rec) -> &RC::Output,
        F: Append<FieldsOf<RC, G, Rec>>,
    {
        (self.push(fields_of(inner, get)), Key::new())
    }

    /// Finish: `build` assembles the record from the keys.
    pub fn build<B>(self, build: B) -> Record<F, KeyedBuild<B>>
    where
        F: Fields<Rec = Rec>,
        B: Fn(&mut Taker<'_, F>) -> Rec,
    {
        Record::from_parts(self.fields, KeyedBuild(build), false, Duplicates::Error)
    }
}
