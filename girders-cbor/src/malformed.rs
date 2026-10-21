//! CBOR's own errors: what is malformed about the input (RFC 8949
//! well-formedness, plus the validity rules the reader checks).

use core::fmt;

/// What is malformed about the input. Carried in
/// `girders::Problem::Format`; all are fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Malformed {
    /// The input ended inside a data item.
    UnexpectedEnd,
    /// Bytes after the end of the data item.
    TrailingBytes,
    /// A "break" stop code outside an indefinite-length item, or in a
    /// definite-length array, map or tag.
    UnexpectedBreak,
    /// A break where a map's value should be (an odd number of items).
    BreakInMapValue,
    /// Additional information 28, 29 or 30, which RFC 8949 reserves.
    ReservedAdditionalInformation,
    /// Indefinite length for an integer or a tag.
    IndefiniteLengthNotAllowed,
    /// A simple value below 32 in the two-byte form (`f8 00` to `f8 1f`).
    TwoByteSimpleValueBelow32,
    /// A chunk of an indefinite-length string that isn't a definite string
    /// of the same type.
    InvalidStringChunk,
    InvalidUtf8,
    /// A decimal fraction (tag 4) that isn't a two-element array.
    DecimalFractionNotAPair,
    DecimalFractionExponentNotAnInteger,
    DecimalFractionMantissaNotAnInteger,
    /// A codec finished an array before reading all of its items.
    ArrayHasMoreItems,
    /// A codec finished a map before reading all of its entries.
    MapHasMoreItems,
    /// A codec asked for the next item with no array or map open.
    NoContainerOpen,
}

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Malformed::*;
        f.write_str(match self {
            UnexpectedEnd => "unexpected end of input",
            TrailingBytes => "trailing bytes after the data item",
            UnexpectedBreak => "unexpected break stop code",
            BreakInMapValue => "break in a map's value position",
            ReservedAdditionalInformation => "reserved additional information value",
            IndefiniteLengthNotAllowed => "indefinite length is not allowed here",
            TwoByteSimpleValueBelow32 => "simple value below 32 in two-byte form",
            InvalidStringChunk => {
                "indefinite-length string chunks must be definite strings of the same type"
            }
            InvalidUtf8 => "invalid UTF-8 in a text string",
            DecimalFractionNotAPair => "a decimal fraction is a two-element array",
            DecimalFractionExponentNotAnInteger => "a decimal fraction's exponent is an integer",
            DecimalFractionMantissaNotAnInteger => "a decimal fraction's mantissa is an integer",
            ArrayHasMoreItems => "array has more items",
            MapHasMoreItems => "map has more items",
            NoContainerOpen => "no array or map is open",
        })
    }
}

impl core::error::Error for Malformed {}

impl girders::FormatError for Malformed {}
