//! Applicative record builder: a static tuple of field specs, decoded in
//! one pass into per-field slots.

use crate::core::*;
#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec, vec::Vec};

use core::marker::PhantomData;

pub trait Fields {
    /// The record type the getters read from.
    type Rec;
    /// Partially-decoded state (`Option<T>` per field).
    type Slots;
    /// What `build` receives.
    type Values;

    fn new_slots(&self) -> Self::Slots;
    fn names(&self, out: &mut Vec<&'static str>);
    /// `Ok(true)` if `key` belongs to this field set (and was decoded).
    fn decode_key<R: Reader>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut Cx,
        slots: &mut Self::Slots,
    ) -> Res<bool, R>;
    fn finish<Loc>(&self, slots: Self::Slots) -> Result<Self::Values, Error<Loc>>;
    fn count(&self, rec: &Self::Rec) -> usize;
    fn encode<S: Serializer>(&self, rec: &Self::Rec, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error>;
    fn schema(&self, s: &mut SchemaCx, out: &mut Vec<(&'static str, Schema, bool)>);
}

// -- Required field -----------------------------------------------------------

pub struct Field<C, G, Rec> {
    name: &'static str,
    codec: C,
    get: G,
    _rec: PhantomData<fn(&Rec)>,
}

pub fn field<Rec, C, G>(name: &'static str, codec: C, get: G) -> Field<C, G, Rec>
where
    C: Codec,
    G: Fn(&Rec) -> &C::Output,
{
    Field { name, codec, get, _rec: PhantomData }
}

impl<Rec, C, G> Fields for Field<C, G, Rec>
where
    C: Codec,
    G: Fn(&Rec) -> &C::Output,
{
    type Rec = Rec;
    type Slots = Option<C::Output>;
    type Values = C::Output;

    fn new_slots(&self) -> Self::Slots {
        None
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        out.push(self.name);
    }
    fn decode_key<R: Reader>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut Cx,
        slot: &mut Self::Slots,
    ) -> Res<bool, R> {
        if key != self.name {
            return Ok(false);
        }
        if slot.is_some() {
            return Err(Error::new(Problem::DuplicateField(self.name)));
        }
        let v = self
            .codec
            .decode(r, cx)
            .map_err(|e| e.push(Seg::Key(self.name.into())))?;
        *slot = Some(v);
        Ok(true)
    }
    fn finish<L>(&self, slot: Self::Slots) -> Result<Self::Values, Error<L>> {
        slot.ok_or_else(|| Error::new(Problem::MissingField(self.name)))
    }
    fn count(&self, _rec: &Rec) -> usize {
        1
    }
    fn encode<S: Serializer>(&self, rec: &Rec, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        s.object_key(self.name)?;
        self.codec.encode((self.get)(rec), s, cx)
    }
    fn schema(&self, s: &mut SchemaCx, out: &mut Vec<(&'static str, Schema, bool)>) {
        out.push((self.name, self.codec.schema(s), true));
    }
}

// -- Optional field (absent key or null => None; None => omitted) ------------

pub struct OptField<C, G, Rec> {
    name: &'static str,
    codec: C,
    get: G,
    _rec: PhantomData<fn(&Rec)>,
}

pub fn optional_field<Rec, C, G>(name: &'static str, codec: C, get: G) -> OptField<C, G, Rec>
where
    C: Codec,
    G: Fn(&Rec) -> &Option<C::Output>,
{
    OptField { name, codec, get, _rec: PhantomData }
}

impl<Rec, C, G> Fields for OptField<C, G, Rec>
where
    C: Codec,
    G: Fn(&Rec) -> &Option<C::Output>,
{
    type Rec = Rec;
    // Outer Option = "seen", inner = value-or-null.
    type Slots = Option<Option<C::Output>>;
    type Values = Option<C::Output>;

    fn new_slots(&self) -> Self::Slots {
        None
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        out.push(self.name);
    }
    fn decode_key<R: Reader>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut Cx,
        slot: &mut Self::Slots,
    ) -> Res<bool, R> {
        if key != self.name {
            return Ok(false);
        }
        if slot.is_some() {
            return Err(Error::new(Problem::DuplicateField(self.name)));
        }
        let v = if r.peek_kind()? == Kind::Null {
            r.read_null()?;
            None
        } else {
            Some(
                self.codec
                    .decode(r, cx)
                    .map_err(|e| e.push(Seg::Key(self.name.into())))?,
            )
        };
        *slot = Some(v);
        Ok(true)
    }
    fn finish<L>(&self, slot: Self::Slots) -> Result<Self::Values, Error<L>> {
        Ok(slot.flatten())
    }
    fn count(&self, rec: &Rec) -> usize {
        (self.get)(rec).is_some() as usize
    }
    fn encode<S: Serializer>(&self, rec: &Rec, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        match (self.get)(rec) {
            None => Ok(()),
            Some(v) => {
                s.object_key(self.name)?;
                self.codec.encode(v, s, cx)
            }
        }
    }
    fn schema(&self, s: &mut SchemaCx, out: &mut Vec<(&'static str, Schema, bool)>) {
        out.push((self.name, self.codec.schema(s), false));
    }
}

// -- fields_of: splice another record's fields into this object ---------------

pub struct FieldsOf<F, B, G, Outer> {
    inner: Record<F, B>,
    get: G,
    _outer: PhantomData<fn(&Outer)>,
}

pub fn fields_of<Outer, F, B, G>(inner: Record<F, B>, get: G) -> FieldsOf<F, B, G, Outer>
where
    F: Fields,
    B: Fn(F::Values) -> F::Rec,
    G: Fn(&Outer) -> &F::Rec,
{
    FieldsOf { inner, get, _outer: PhantomData }
}

impl<Outer, F, B, G> Fields for FieldsOf<F, B, G, Outer>
where
    F: Fields,
    B: Fn(F::Values) -> F::Rec,
    G: Fn(&Outer) -> &F::Rec,
{
    type Rec = Outer;
    type Slots = F::Slots;
    type Values = F::Rec;

    fn new_slots(&self) -> Self::Slots {
        self.inner.fields.new_slots()
    }
    fn names(&self, out: &mut Vec<&'static str>) {
        self.inner.fields.names(out)
    }
    fn decode_key<R: Reader>(
        &self,
        key: &str,
        r: &mut R,
        cx: &mut Cx,
        slots: &mut Self::Slots,
    ) -> Res<bool, R> {
        self.inner.fields.decode_key(key, r, cx, slots)
    }
    fn finish<Loc>(&self, slots: Self::Slots) -> Result<Self::Values, Error<Loc>> {
        Ok((self.inner.build)(self.inner.fields.finish(slots)?))
    }
    fn count(&self, rec: &Outer) -> usize {
        self.inner.fields.count((self.get)(rec))
    }
    fn encode<S: Serializer>(&self, rec: &Outer, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        self.inner.fields.encode((self.get)(rec), s, cx)
    }
    fn schema(&self, s: &mut SchemaCx, out: &mut Vec<(&'static str, Schema, bool)>) {
        self.inner.fields.schema(s, out)
    }
}

// -- Tuples of field specs ----------------------------------------------------

macro_rules! tuple_fields {
    ($A:ident 0 $(, $T:ident $i:tt)*) => {
        impl<$A: Fields, $($T: Fields<Rec = $A::Rec>),*> Fields for ($A, $($T,)*) {
            type Rec = $A::Rec;
            type Slots = ($A::Slots, $($T::Slots,)*);
            type Values = ($A::Values, $($T::Values,)*);

            fn new_slots(&self) -> Self::Slots {
                (self.0.new_slots(), $(self.$i.new_slots(),)*)
            }
            fn names(&self, out: &mut Vec<&'static str>) {
                self.0.names(out);
                $(self.$i.names(out);)*
            }
            fn decode_key<R: Reader>(
                &self,
                key: &str,
                r: &mut R,
                cx: &mut Cx,
                slots: &mut Self::Slots,
            ) -> Res<bool, R> {
                if self.0.decode_key(key, r, cx, &mut slots.0)? {
                    return Ok(true);
                }
                $(if self.$i.decode_key(key, r, cx, &mut slots.$i)? {
                    return Ok(true);
                })*
                Ok(false)
            }
            fn finish<Loc>(&self, slots: Self::Slots) -> Result<Self::Values, Error<Loc>> {
                Ok((self.0.finish(slots.0)?, $(self.$i.finish(slots.$i)?,)*))
            }
            fn count(&self, rec: &Self::Rec) -> usize {
                self.0.count(rec) $(+ self.$i.count(rec))*
            }
            fn encode<S: Serializer>(&self, rec: &Self::Rec, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
                self.0.encode(rec, s, cx)?;
                $(self.$i.encode(rec, s, cx)?;)*
                Ok(())
            }
            fn schema(&self, s: &mut SchemaCx, out: &mut Vec<(&'static str, Schema, bool)>) {
                self.0.schema(s, out);
                $(self.$i.schema(s, out);)*
            }
        }
    };
}

tuple_fields!(A 0);
tuple_fields!(A 0, B 1);
tuple_fields!(A 0, B 1, C 2);
tuple_fields!(A 0, B 1, C 2, D 3);
tuple_fields!(A 0, B 1, C 2, D 3, E 4);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11, M 12);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11, M 12, N 13);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11, M 12, N 13, O 14);
tuple_fields!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11, M 12, N 13, O 14, P 15);

// -- Record -------------------------------------------------------------------

pub struct RecordBuilder<F> {
    fields: F,
}

pub fn record<F: Fields>(fields: F) -> RecordBuilder<F> {
    RecordBuilder { fields }
}

impl<F: Fields> RecordBuilder<F> {
    pub fn build<B>(self, build: B) -> Record<F, B>
    where
        B: Fn(F::Values) -> F::Rec,
    {
        Record { fields: self.fields, build, allow_unknown: false }
    }
}

pub struct Record<F, B> {
    fields: F,
    build: B,
    allow_unknown: bool,
}

impl<F, B> Record<F, B> {
    pub fn allow_unknown(mut self) -> Self {
        self.allow_unknown = true;
        self
    }
}

impl<F, B> Decoder for Record<F, B>
where
    F: Fields,
    B: Fn(F::Values) -> F::Rec,
{
    type Output = F::Rec;

    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<F::Rec, R> {
        let claimed = cx.claimed.take();
        r.begin_object()?;
        let mut slots = self.fields.new_slots();
        let mut key = String::new();
        let mut first = true;
        loop {
            key.clear();
            if !r.object_next_key(first, &mut key)? {
                break;
            }
            first = false;
            if claimed == Some(key.as_str()) {
                r.skip_value()?;
                continue;
            }
            if !self.fields.decode_key(&key, r, cx, &mut slots)? {
                if self.allow_unknown {
                    r.skip_value()?;
                } else {
                    return Err(Error::new(Problem::UnknownField(key)));
                }
            }
        }
        r.end_object()?;
        Ok((self.build)(self.fields.finish(slots)?))
    }

    fn schema(&self, s: &mut SchemaCx) -> Schema {
        let mut fields = Vec::new();
        self.fields.schema(s, &mut fields);
        Schema::Object { fields, additional: self.allow_unknown }
    }
}

impl<F, B> Encoder for Record<F, B>
where
    F: Fields,
    B: Fn(F::Values) -> F::Rec,
{
    type Input = F::Rec;

    fn encode<S: Serializer>(&self, v: &F::Rec, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        let tag = cx.pending_tag.take();
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
