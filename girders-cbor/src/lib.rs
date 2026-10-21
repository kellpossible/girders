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
pub mod reader;
pub mod serializer;
pub mod value;

pub use reader::{CborCheckpoint, CborReader};
pub use serializer::CborSerializer;
pub use value::{Value, ValueCodec, value};

use alloc::vec::Vec;
use girders::{
    DecodeContext, DecodeError, DecodeOptions, Decoder, EncodeContext, Encoder, Problem, Reader,
    Span,
};

pub type Error = DecodeError<Span>;

/// The reader as a trait object. Decoding goes through this so the
/// reader's code exists once, shared by every codec.
pub type DynReader<'a> = dyn Reader<Location = Span, Checkpoint = CborCheckpoint> + 'a;

/// Decode one complete data item; trailing bytes are an error.
pub fn decode<C: Decoder + ?Sized>(input: &[u8], codec: &C) -> Result<C::Output, Error> {
    decode_with(input, codec, DecodeOptions::new())
}

pub fn decode_with<C: Decoder + ?Sized>(
    input: &[u8],
    codec: &C,
    options: DecodeOptions,
) -> Result<C::Output, Error> {
    let mut r = CborReader::new(input);
    r.set_track_locations(options.track_locations);
    let mut cx = DecodeContext::new(options);
    let v = codec.decode::<DynReader<'_>>(&mut r, &mut cx)?;
    if !r.at_end() {
        return Err(DecodeError::at(
            Problem::Syntax("trailing bytes after the data item".into()),
            Some(Span::point(r.position() as u64)),
        ));
    }
    Ok(v)
}

/// Encode with preferred serialization.
pub fn encode<C: Encoder + ?Sized>(v: &C::Input, codec: &C) -> Vec<u8> {
    let mut s = CborSerializer::new();
    match codec.encode(v, &mut s, &mut EncodeContext::new()) {
        Ok(()) => s.into_inner(),
        Err(never) => match never {},
    }
}
