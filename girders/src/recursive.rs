//! Recursive codecs without macros.
//!
//! A combinator-only `recursive(name, tree)` cannot compile: proving
//! `Record<…List<Recursive<tree>>…>: Decoder` depends on itself, and rustc
//! only accepts such cycles for auto traits (E0275). Implementing
//! [`Recursive`] on a small struct breaks the cycle nominally: proving
//! `RecursiveCodec<D>: Decoder` needs only `D: Recursive`.

use crate::codec::{Codec, DecodeResult, Decoder, Encoder, decode_child};
use crate::context::{DecodeContext, EncodeContext};
use crate::reader::Reader;
use crate::serializer::Serializer;

/// Implement on a (usually unit) struct to define a recursive codec:
///
/// ```ignore
/// struct TreeDef;
/// impl Recursive for TreeDef {
///     type Output = Tree;
///     fn codec(&self) -> impl Codec<Output = Tree> {
///         record((
///             field("value", i64(), |t: &Tree| &t.value),
///             field("children", list(recursive(TreeDef)), |t: &Tree| &t.children),
///         ))
///         .build(|(value, children)| Tree { value, children })
///     }
/// }
/// ```
pub trait Recursive {
    type Output;
    fn codec(&self) -> impl Codec<Output = Self::Output>;
}

#[derive(Debug, Clone, Copy)]
pub struct RecursiveCodec<D>(pub D);

/// The codec for a [`Recursive`] definition.
pub const fn recursive<D: Recursive>(def: D) -> RecursiveCodec<D> {
    RecursiveCodec(def)
}

impl<D: Recursive> Decoder for RecursiveCodec<D> {
    type Output = D::Output;
    #[inline]
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<D::Output, R> {
        decode_child(&self.0.codec(), r, cx)
    }
}

impl<D: Recursive> Encoder for RecursiveCodec<D> {
    type Input = D::Output;
    #[inline]
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &D::Output,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        self.0.codec().encode(v, s, cx)
    }
}
