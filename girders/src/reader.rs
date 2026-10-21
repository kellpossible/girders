//! The format-side contract for decoding.

use crate::error::{DecodeError, Problem};
use crate::number::Number;
use crate::span::Span;
use alloc::string::String;
use alloc::vec::Vec;
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
    /// A byte string (binary formats).
    Bytes,
    /// A tag: a number annotating the value that follows (CBOR).
    Tag,
    /// A simple value other than `true`, `false` and `null`, such as
    /// CBOR's `undefined`.
    Simple,
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
            Kind::Bytes => "bytes",
            Kind::Tag => "tag",
            Kind::Simple => "simple value",
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

    /// Append the next byte string to `buf`. Formats without byte strings
    /// use the default: an array of numbers from 0 to 255.
    fn read_bytes(&mut self, buf: &mut Vec<u8>) -> DecodeResult<(), Self> {
        self.begin_array()?;
        let mut first = true;
        while self.array_has_next(first)? {
            first = false;
            let at = self.here();
            let b = self.read_u64()?;
            let b = u8::try_from(b)
                .map_err(|_| DecodeError::at(Problem::OutOfRange("byte above 255".into()), at))?;
            buf.push(b);
        }
        self.end_array()
    }
    /// Whether the format has tags (see [`Kind::Tag`]).
    fn supports_tags(&self) -> bool {
        false
    }
    /// Read a tag number, leaving the reader at the tagged value.
    fn read_tag(&mut self) -> DecodeResult<u64, Self> {
        Err(mismatch(self, "tag"))
    }
    /// Read a simple value (see [`Kind::Simple`]) as its number.
    fn read_simple(&mut self) -> DecodeResult<u8, Self> {
        Err(mismatch(self, "simple value"))
    }

    fn begin_array(&mut self) -> DecodeResult<(), Self>;
    /// How many items (arrays) or entries (objects) the container just
    /// opened has, if the format says up front: CBOR's definite lengths,
    /// never JSON. Only a hint for preallocation, and untrusted: size
    /// allocations with [`cautious_capacity`].
    fn size_hint(&self) -> Option<usize> {
        None
    }
    /// `true` if another element follows (consuming any separator);
    /// `false` at the closing delimiter. `first` says whether no element
    /// has been read yet.
    fn array_has_next(&mut self, first: bool) -> DecodeResult<bool, Self>;
    fn end_array(&mut self) -> DecodeResult<(), Self>;

    fn begin_object(&mut self) -> DecodeResult<(), Self>;
    /// Append the next key to `buf` and return `true`, leaving the reader
    /// at its value; `false` at the closing delimiter.
    fn object_next_key(&mut self, first: bool, buf: &mut String) -> DecodeResult<bool, Self>;
    /// Like [`Reader::object_next_key`], but leave the reader at the key so
    /// it can be read as a value of any kind, followed by the entry's
    /// value. Only formats with non-string keys (CBOR) support it.
    fn object_next_entry(&mut self, _first: bool) -> DecodeResult<bool, Self> {
        Err(DecodeError::at(
            Problem::Unsupported("map keys other than strings".into()),
            self.here(),
        ))
    }
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

/// The most a decoder preallocates from a [`Reader::size_hint`]: 1 MiB of
/// elements, as serde does. A length is only a claim until the items
/// arrive, so a forged one can't force a huge allocation.
const MAX_PREALLOCATION: usize = 1024 * 1024;

/// A capacity for `T`s from a size hint, capped at 1 MiB of elements.
pub fn cautious_capacity<T>(hint: Option<usize>) -> usize {
    let per_element = core::mem::size_of::<T>().max(1);
    hint.map_or(0, |n| n.min(MAX_PREALLOCATION / per_element))
}
