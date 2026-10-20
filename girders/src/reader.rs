//! The format-side contract for decoding.

use crate::error::DecodeError;
use crate::number::Number;
use crate::span::Span;
use alloc::string::String;
use core::fmt::Debug;

/// The broad classification of the next value, for dispatch and for
/// `TypeMismatch` messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    String,
    Number,
    Bool,
    Null,
    Array,
    Object,
    Eof,
}

impl Kind {
    pub const fn name(self) -> &'static str {
        match self {
            Kind::String => "string",
            Kind::Number => "number",
            Kind::Bool => "bool",
            Kind::Null => "null",
            Kind::Array => "array",
            Kind::Object => "object",
            Kind::Eof => "end of input",
        }
    }
}

pub type DecodeResult<T, R> = Result<T, DecodeError<<R as Reader>::Location>>;

/// A pull reader over some input. Implemented by a mutable cursor; all
/// primitives skip insignificant whitespace (or the format's equivalent)
/// first.
///
/// Readers report their own errors with locations attached. Combinators
/// add paths and context.
pub trait Reader {
    /// What an error points at: a byte span for JSON, a variable name for
    /// an env reader, `()` for a format with no positional notion.
    type Location: Clone + Debug;
    /// An opaque position to return to. For streaming sources, an
    /// outstanding checkpoint keeps the input from that point buffered.
    /// Released in LIFO order.
    type Checkpoint;

    fn peek_kind(&mut self) -> DecodeResult<Kind, Self>;

    /// The location of the next value. `None` when location tracking is
    /// off (see [`Reader::set_track_locations`]).
    fn here(&mut self) -> Option<Self::Location>;
    /// The location from `start` to the current position.
    fn since(&mut self, start: &Self::Location) -> Self::Location;
    /// Project a location to a byte span, where the format has one.
    fn byte_span(&self, _location: &Self::Location) -> Option<Span> {
        None
    }
    /// Readers may skip location bookkeeping when this is off.
    fn set_track_locations(&mut self, _on: bool) {}
    /// The location of the key most recently read by
    /// [`Reader::object_next_key`], if the format tracks it.
    fn key_location(&self) -> Option<Self::Location> {
        None
    }

    fn read_bool(&mut self) -> DecodeResult<bool, Self>;
    fn read_null(&mut self) -> DecodeResult<(), Self>;
    /// Append the next string to `buf`.
    fn read_str(&mut self, buf: &mut String) -> DecodeResult<(), Self>;
    /// An integer that fits `i64`; `OutOfRange` if it doesn't,
    /// `TypeMismatch` for a non-integer number.
    fn read_i64(&mut self) -> DecodeResult<i64, Self>;
    fn read_u64(&mut self) -> DecodeResult<u64, Self>;
    /// Any number, converted to `f64`.
    fn read_f64(&mut self) -> DecodeResult<f64, Self>;
    /// Any number, classified.
    fn read_number(&mut self) -> DecodeResult<Number, Self>;
    /// The number's source text (for `i128`, decimals, big integers).
    fn read_number_text(&mut self, buf: &mut String) -> DecodeResult<(), Self>;

    fn begin_array(&mut self) -> DecodeResult<(), Self>;
    /// `true` if another element follows (consuming any separator);
    /// `false` at the closing delimiter. `first` says whether no element
    /// has been read yet.
    fn array_has_next(&mut self, first: bool) -> DecodeResult<bool, Self>;
    fn end_array(&mut self) -> DecodeResult<(), Self>;

    fn begin_object(&mut self) -> DecodeResult<(), Self>;
    /// Append the next key to `buf` and return `true`, leaving the reader
    /// at its value; `false` at the closing delimiter.
    fn object_next_key(&mut self, first: bool, buf: &mut String) -> DecodeResult<bool, Self>;
    fn end_object(&mut self) -> DecodeResult<(), Self>;

    /// Skip one whole value of any kind.
    fn skip_value(&mut self) -> DecodeResult<(), Self>;

    fn checkpoint(&mut self) -> Self::Checkpoint;
    fn rewind(&mut self, cp: &Self::Checkpoint);
    fn release(&mut self, cp: Self::Checkpoint);
}

/// Shorthand for `TypeMismatch` at the next value, against what the
/// reader actually sees. Used by readers and combinators.
pub fn mismatch<R: Reader + ?Sized>(r: &mut R, expected: &'static str) -> DecodeError<R::Location> {
    let found = match r.peek_kind() {
        Ok(k) => k.name(),
        Err(e) => return e,
    };
    DecodeError::at(crate::Problem::mismatch(expected, found), r.here())
}
