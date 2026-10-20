//! Format contract, errors, and the Decoder/Encoder/Codec traits.

#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec, vec::Vec};

use core::fmt::Debug;

#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    TypeMismatch { expected: &'static str, found: Kind },
    MissingField(&'static str),
    UnknownField(String),
    DuplicateField(&'static str),
    UnknownVariant(String),
    UnknownVersion(i64),
    OutOfRange,
    Syntax(String),
}

/// Location type is chosen by the reader.
#[derive(Debug, Clone, PartialEq)]
pub struct Error<L> {
    /// Innermost-first while propagating.
    pub path: Vec<Seg>,
    pub location: Option<L>,
    pub problem: Problem,
}

impl<L> Error<L> {
    pub fn new(problem: Problem) -> Self {
        Error { path: Vec::new(), location: None, problem }
    }
    pub fn at(problem: Problem, location: L) -> Self {
        Error { path: Vec::new(), location: Some(location), problem }
    }
    pub fn push(mut self, seg: Seg) -> Self {
        self.path.push(seg);
        self
    }
    /// Outermost-first path.
    pub fn path(&self) -> Vec<Seg> {
        self.path.iter().rev().cloned().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    String,
    Number,
    Bool,
    Null,
    Array,
    Object,
    Eof,
}

pub type Res<T, R> = Result<T, Error<<R as Reader>::Location>>;

pub trait Reader {
    type Location: Clone + Debug;
    type Checkpoint;

    fn peek_kind(&mut self) -> Res<Kind, Self>;
    fn read_str(&mut self, buf: &mut String) -> Res<(), Self>;
    fn read_i64(&mut self) -> Res<i64, Self>;
    fn read_null(&mut self) -> Res<(), Self>;
    fn begin_array(&mut self) -> Res<(), Self>;
    fn array_has_next(&mut self, first: bool) -> Res<bool, Self>;
    fn end_array(&mut self) -> Res<(), Self>;
    fn begin_object(&mut self) -> Res<(), Self>;
    fn object_next_key(&mut self, first: bool, buf: &mut String) -> Res<bool, Self>;
    fn end_object(&mut self) -> Res<(), Self>;
    fn skip_value(&mut self) -> Res<(), Self>;
    fn checkpoint(&mut self) -> Self::Checkpoint;
    fn rewind(&mut self, cp: &Self::Checkpoint);
    fn release(&mut self, cp: Self::Checkpoint);
}

pub trait Serializer {
    type Error;
    fn write_str(&mut self, v: &str) -> Result<(), Self::Error>;
    fn write_i64(&mut self, v: i64) -> Result<(), Self::Error>;
    fn write_null(&mut self) -> Result<(), Self::Error>;
    fn begin_array(&mut self, len: Option<usize>) -> Result<(), Self::Error>;
    fn array_element(&mut self) -> Result<(), Self::Error>;
    fn end_array(&mut self) -> Result<(), Self::Error>;
    fn begin_object(&mut self, len: Option<usize>) -> Result<(), Self::Error>;
    fn object_key(&mut self, key: &str) -> Result<(), Self::Error>;
    fn end_object(&mut self) -> Result<(), Self::Error>;
}

/// Per-decode state threaded through combinators.
#[derive(Default)]
pub struct Cx {
    /// A key the enclosing union/versioned dispatcher already consumed;
    /// the next record skips it instead of rejecting it as unknown.
    pub claimed: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Schema {
    String,
    Integer { min: i64, max: i64 },
    Null,
    Nullable(Box<Schema>),
    Array(Box<Schema>),
    Object { fields: Vec<(&'static str, Schema, bool)>, additional: bool },
    Union { tag: &'static str, variants: Vec<(String, Schema)> },
    Ref(&'static str),
}

#[derive(Default, Debug)]
pub struct SchemaCx {
    pub defs: Vec<(&'static str, Option<Schema>)>,
}

pub trait Decoder {
    type Output;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Self::Output, R>;
    fn schema(&self, s: &mut SchemaCx) -> Schema;
}

/// Per-encode state threaded through combinators. Never wrap the
/// serializer instead: changing `S` per level makes recursive codecs
/// instantiate `Wrap<Wrap<Wrap<…>>>` forever.
#[derive(Default)]
pub struct Ecx {
    /// A `tag: value` entry the next record must write first.
    pub pending_tag: Option<(&'static str, TagValue)>,
}

#[derive(Debug, Clone, Copy)]
pub enum TagValue {
    Str(&'static str),
    Int(i64),
}

pub trait Encoder {
    type Input: ?Sized;
    fn encode<S: Serializer>(&self, v: &Self::Input, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error>;
}

pub trait Codec: Decoder + Encoder<Input = <Self as Decoder>::Output> {}
impl<T> Codec for T where T: Decoder + Encoder<Input = <T as Decoder>::Output> {}
