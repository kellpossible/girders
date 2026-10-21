//! girders: bidirectional, format-agnostic serialization codecs.
//!
//! A [`Codec`] decodes and encodes a type from one definition. Formats implement [`Reader`] and [`Serializer`]; codecs are
//! generic over both, so one codec value works with every format.
//!
//! - Errors carry a logical path (`users[0].age`), a format-chosen
//!   location (a byte span for JSON) and context frames.
//! - Decoding is strict by default: unknown and duplicate fields are
//!   errors unless a codec opts out.
//! - Records, tagged unions, versioned codecs and recursive types are
//!   all built from plain combinators; there are no derive macros.
//! - `no_std` + `alloc`; the `std` feature only adds `io::Read` input.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod codec;
pub mod collections;
pub mod context;
pub mod error;
pub mod half;
pub mod number;
pub mod packed;
pub mod reader;
pub mod record;
pub mod recursive;
pub mod serializer;
pub mod source;
pub mod span;
pub mod union;
pub mod versioned;

pub use codec::{Codec, Decoder, Encoder, decode_child};
pub use context::{DecodeContext, DecodeOptions, EncodeContext, TagValue};
pub use error::{DecodeError, ErrorDetail, PathSegment, Problem};
pub use number::Number;
pub use reader::{Kind, Reader};
pub use serializer::Serializer;
pub use span::{Span, Spanned};

/// Everything needed to write codecs: `use girders::prelude::*;`.
pub mod prelude {
    pub use crate::codec::*;
    pub use crate::collections::*;
    pub use crate::context::{DecodeContext, DecodeOptions, EncodeContext};
    pub use crate::error::{DecodeError, PathSegment, Problem};
    pub use crate::packed::packed;
    pub use crate::reader::{Kind, Reader};
    pub use crate::record::{
        Duplicates, Fields, FieldsFor, RecordCodec, field, field_with_default, fields_for, flatten,
        optional_field, record, unknown_fields,
    };
    pub use crate::recursive::{Recursive, recursive};
    pub use crate::serializer::Serializer;
    pub use crate::span::{Span, Spanned};
    pub use crate::union::*;
    pub use crate::versioned::versioned;
}
