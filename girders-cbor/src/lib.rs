//! CBOR (RFC 8949) for girders.
//!
//! - **Reading** is strict about well-formedness: reserved encodings,
//!   misplaced break codes, malformed indefinite-length strings, invalid
//!   UTF-8 and truncated input are syntax errors, with byte offsets.
//! - **Numbers** keep full precision: integers across CBOR's whole range,
//!   bignums (tags 2 and 3) and decimal fractions (tag 4) reach `i128`,
//!   `u128` and number-text codecs exactly, and half, single and double
//!   floats are all read.
//! - **Writing** produces preferred serialization: shortest integer and
//!   length heads, and the shortest float width that is exact. Containers
//!   whose length the codec doesn't announce are written with indefinite
//!   length.
//! - [`Value`] is the dynamic tree, with byte strings, tags, simple values
//!   and keys of any kind. It prints as diagnostic notation.
//!
//! Records encode as maps with text keys, as in JSON, so the same codec
//! reads and writes both formats.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod bignum;
pub mod malformed;
pub mod reader;
pub mod serializer;
pub mod stream;
pub mod value;

pub use malformed::Malformed;
pub use reader::{CborCheckpoint, CborReader};
pub use serializer::{CborSerializer, EncodeOptions, Floats};
pub use stream::StreamDecoder;
pub use value::{Value, ValueCodec, value};

use alloc::vec::Vec;
#[cfg(feature = "std")]
use girders::source::ReadSource;
use girders::source::{SliceSource, Source};
use girders::{
    DecodeContext, DecodeError, DecodeOptions, Decoder, EncodeContext, Encoder, Problem, Reader,
    Span,
};

/// A decode error from CBOR input.
pub type Error = DecodeError<Span, Malformed>;

/// The reader as a trait object. Decoding goes through this so the
/// reader's code exists once, shared by every codec.
pub(crate) type DynReader<'a> =
    dyn Reader<Location = Span, Error = Malformed, Checkpoint = CborCheckpoint> + 'a;

/// Decode one complete data item; trailing bytes are an error.
pub fn decode<C: Decoder + ?Sized>(input: &[u8], codec: &C) -> Result<C::Output, Error> {
    decode_with(input, codec, DecodeOptions::new())
}

pub fn decode_with<C: Decoder + ?Sized>(
    input: &[u8],
    codec: &C,
    options: DecodeOptions,
) -> Result<C::Output, Error> {
    decode_source(SliceSource::new(input), codec, options)
}

/// Decode one data item from any source; trailing bytes are an error.
///
/// The reader is passed to the codec as a trait object,
/// so its code exists once however many codecs use it.
pub fn decode_source<S: Source, C: Decoder + ?Sized>(
    src: S,
    codec: &C,
    options: DecodeOptions,
) -> Result<C::Output, Error> {
    let mut r = CborReader::new(src);
    r.set_track_locations(options.track_locations);
    let mut cx = DecodeContext::new(options);
    let v = codec.decode::<DynReader<'_>>(&mut r, &mut cx)?;
    if !r.at_end()? {
        return Err(DecodeError::at(
            Problem::Format(Malformed::TrailingBytes),
            Some(Span::point(r.offset())),
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
    decode_source(ReadSource::new(input), codec, DecodeOptions::new())
}

#[cfg(feature = "std")]
pub fn from_reader_with<R: std::io::Read, C: Decoder + ?Sized>(
    input: R,
    codec: &C,
    options: DecodeOptions,
    chunk_size: usize,
) -> Result<C::Output, Error> {
    decode_source(
        ReadSource::with_chunk_size(input, chunk_size),
        codec,
        options,
    )
}

/// Decode into a [`Value`].
pub fn parse(input: &[u8]) -> Result<Value, Error> {
    decode(input, &value())
}

/// Stream the encoding into any `std::io::Write`.
#[cfg(feature = "std")]
pub fn encode_to_writer<W: std::io::Write, C: Encoder + ?Sized>(
    mut out: W,
    v: &C::Input,
    codec: &C,
    options: EncodeOptions,
) -> std::io::Result<()> {
    out.write_all(&encode_with(v, codec, options))
}

/// Encode with preferred serialization. CBOR can represent every value, so
/// this can't fail.
pub fn encode<C: Encoder + ?Sized>(v: &C::Input, codec: &C) -> Vec<u8> {
    encode_with(v, codec, EncodeOptions::default())
}

pub fn encode_with<C: Encoder + ?Sized>(
    v: &C::Input,
    codec: &C,
    options: EncodeOptions,
) -> Vec<u8> {
    let mut s = CborSerializer::with_options(options);
    match codec.encode(v, &mut s, &mut EncodeContext::new()) {
        Ok(()) => s.into_inner(),
        Err(never) => match never {},
    }
}
