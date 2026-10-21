//! Strict JSON for girders: a streaming reader, a compact/pretty
//! serializer, a `Value` tree, JSON Pointer and diagnostics.
//!
//! ```
//! use girders::prelude::*;
//!
//! #[derive(Debug, PartialEq)]
//! struct User { id: String, age: u32 }
//!
//! fn user() -> impl Codec<Output = User> {
//!     record((
//!         field("id", string(), |u: &User| &u.id),
//!         field("age", u32(), |u: &User| &u.age),
//!     ))
//!     .build(|(id, age)| User { id, age })
//! }
//!
//! let u = girders_json::decode(r#"{"age": 7, "id": "a"}"#, &user()).unwrap();
//! assert_eq!(girders_json::encode(&u, &user()).unwrap(), r#"{"id":"a","age":7}"#);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod diagnostic;
pub mod pointer;
pub mod reader;
pub mod serializer;
pub mod source;
pub mod stream;
pub mod value;

pub use pointer::Pointer;
pub use reader::JsonReader;
pub use serializer::{EncodeError, JsonSerializer, Style};
pub use source::{PushSource, SliceSource, Source};
pub use stream::StreamDecoder;
pub use value::{Entry, Node, Object, Value, value};

use alloc::string::String;
use alloc::vec::Vec;
use girders::{
    DecodeContext, DecodeError, DecodeOptions, Decoder, EncodeContext, Encoder, Problem, Reader,
    Span,
};

pub type Error = DecodeError<Span>;

/// Decode a complete document with default options.
pub fn decode<C: Decoder + ?Sized>(input: &str, codec: &C) -> Result<C::Output, Error> {
    decode_with(input, codec, DecodeOptions::new())
}

/// Text input skips UTF-8 re-validation of strings (it is already `&str`).
pub fn decode_with<C: Decoder + ?Sized>(
    input: &str,
    codec: &C,
    options: DecodeOptions,
) -> Result<C::Output, Error> {
    decode_source(SliceSource::from_text(input), codec, options)
}

pub fn decode_bytes<C: Decoder + ?Sized>(input: &[u8], codec: &C) -> Result<C::Output, Error> {
    decode_bytes_with(input, codec, DecodeOptions::new())
}

pub fn decode_bytes_with<C: Decoder + ?Sized>(
    input: &[u8],
    codec: &C,
    options: DecodeOptions,
) -> Result<C::Output, Error> {
    decode_source(SliceSource::new(input), codec, options)
}

/// Decode one document from any source; trailing non-whitespace is an
/// error.
///
/// The reader is passed to the codec as a trait object,
/// so its code exists once however many codecs use it.
pub fn decode_source<S: Source, C: Decoder + ?Sized>(
    src: S,
    codec: &C,
    options: DecodeOptions,
) -> Result<C::Output, Error> {
    let mut r = JsonReader::new(src);
    r.set_track_locations(options.track_locations);
    let mut cx = DecodeContext::new(options);
    let v = codec.decode::<DynReader<'_>>(&mut r, &mut cx)?;
    if !r.at_end()? {
        let at = r.here();
        return Err(DecodeError::at(
            Problem::Syntax("trailing input after value".into()),
            at,
        ));
    }
    Ok(v)
}

/// Decode from any `std::io::Read`, buffered internally.
#[cfg(feature = "std")]
pub fn from_reader<R: std::io::Read, C: Decoder + ?Sized>(
    input: R,
    codec: &C,
) -> Result<C::Output, Error> {
    decode_source(source::ReadSource::new(input), codec, DecodeOptions::new())
}

#[cfg(feature = "std")]
pub fn from_reader_with<R: std::io::Read, C: Decoder + ?Sized>(
    input: R,
    codec: &C,
    options: DecodeOptions,
    chunk_size: usize,
) -> Result<C::Output, Error> {
    decode_source(
        source::ReadSource::with_chunk_size(input, chunk_size),
        codec,
        options,
    )
}

/// Parse into a `Value`.
pub fn parse(input: &str) -> Result<Value, Error> {
    decode(input, &value())
}

/// How to write JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct EncodeOptions {
    pub style: Style,
}

impl EncodeOptions {
    /// Compact output.
    pub const fn new() -> Self {
        EncodeOptions {
            style: Style::Compact,
        }
    }

    /// Indent nested structure by `indent` per level.
    pub fn pretty(self, indent: impl Into<String>) -> Self {
        EncodeOptions {
            style: Style::Pretty(indent.into()),
        }
    }
}

/// Encode as compact JSON. Fails for values JSON can't represent: NaN and
/// infinities, and map keys other than strings, numbers and bools.
pub fn encode<C: Encoder + ?Sized>(v: &C::Input, codec: &C) -> Result<String, EncodeError> {
    encode_with(v, codec, EncodeOptions::new())
}

pub fn encode_with<C: Encoder + ?Sized>(
    v: &C::Input,
    codec: &C,
    options: EncodeOptions,
) -> Result<String, EncodeError> {
    let mut s = JsonSerializer::new(Vec::new(), options.style);
    codec.encode(v, &mut s, &mut EncodeContext::new())?;
    Ok(String::from_utf8(s.into_inner()).expect("the serializer writes UTF-8"))
}

/// Stream the encoding into any `std::io::Write`.
#[cfg(feature = "std")]
pub fn encode_to_writer<W: std::io::Write, C: Encoder + ?Sized>(
    out: W,
    v: &C::Input,
    codec: &C,
    options: EncodeOptions,
) -> Result<(), EncodeError<std::io::Error>> {
    let mut s = JsonSerializer::new(serializer::IoWrite(out), options.style);
    codec.encode(v, &mut s, &mut EncodeContext::new())
}

/// The JSON Pointer to an error's location in the document.
pub fn to_pointer(err: &Error) -> Pointer {
    Pointer::from_path(err.first().path())
}

/// The JSON reader as a trait object. Decoding goes through this so the
/// reader's code exists once, shared by every codec.
pub(crate) type DynReader<'a> =
    dyn Reader<Location = Span, Checkpoint = reader::JsonCheckpoint> + 'a;
