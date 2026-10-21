//! JSON's own errors: what is malformed about the input.

use core::fmt;

/// What is malformed about the input. Carried in
/// `girders::Problem::Format`; all are fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Malformed {
    /// A byte that can't start or continue a value here.
    UnexpectedCharacter(u8),
    /// The input ended inside a value.
    UnexpectedEnd,
    /// Input after the end of the value.
    TrailingInput,
    UnterminatedString,
    UnterminatedArray,
    UnterminatedObject,
    InvalidUtf8,
    /// A control character (below U+0020) inside a string, which JSON
    /// requires to be escaped.
    ControlCharacterInString,
    UnterminatedEscape,
    /// `\` followed by a character that isn't an escape.
    InvalidEscape(u8),
    IncompleteUnicodeEscape,
    InvalidHexDigit(u8),
    /// A `\u` escape for a high surrogate with no low surrogate after it.
    LoneHighSurrogate(u16),
    /// A `\u` escape for a low surrogate with no high surrogate before it.
    LoneLowSurrogate(u16),
    /// A high surrogate followed by an escape that isn't a low surrogate.
    ExpectedLowSurrogate(u16),
    InvalidUnicodeEscape,
    ExpectedDigit,
    ExpectedFractionDigit,
    ExpectedExponentDigit,
    /// Something that starts like `true`, `false` or `null` but isn't.
    InvalidLiteral,
    ExpectedKey,
    ExpectedColon,
    ExpectedCommaOrBracket,
    ExpectedCommaOrBrace,
}

fn byte(f: &mut fmt::Formatter<'_>, b: u8) -> fmt::Result {
    if b.is_ascii_graphic() || b == b' ' {
        write!(f, "{}", b as char)
    } else {
        write!(f, "\\x{b:02x}")
    }
}

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Malformed::*;
        match *self {
            UnexpectedCharacter(b) => {
                f.write_str("unexpected character `")?;
                byte(f, b)?;
                f.write_str("`")
            }
            UnexpectedEnd => f.write_str("unexpected end of input"),
            TrailingInput => f.write_str("trailing input after value"),
            UnterminatedString => f.write_str("unterminated string"),
            UnterminatedArray => f.write_str("unterminated array"),
            UnterminatedObject => f.write_str("unterminated object"),
            InvalidUtf8 => f.write_str("invalid UTF-8 in string"),
            ControlCharacterInString => f.write_str("unescaped control character in string"),
            UnterminatedEscape => f.write_str("unterminated escape"),
            InvalidEscape(b) => {
                f.write_str("invalid escape `\\")?;
                byte(f, b)?;
                f.write_str("`")
            }
            IncompleteUnicodeEscape => f.write_str("incomplete unicode escape"),
            InvalidHexDigit(b) => {
                f.write_str("invalid hex digit `")?;
                byte(f, b)?;
                f.write_str("`")
            }
            LoneHighSurrogate(n) => write!(f, "lone high surrogate `\\u{n:04X}`"),
            LoneLowSurrogate(n) => write!(f, "lone low surrogate `\\u{n:04X}`"),
            ExpectedLowSurrogate(n) => write!(f, "expected low surrogate after `\\u{n:04X}`"),
            InvalidUnicodeEscape => f.write_str("invalid unicode escape"),
            ExpectedDigit => f.write_str("expected digit"),
            ExpectedFractionDigit => f.write_str("expected digit after decimal point"),
            ExpectedExponentDigit => f.write_str("expected digit in exponent"),
            InvalidLiteral => f.write_str("invalid literal"),
            ExpectedKey => f.write_str("expected string key"),
            ExpectedColon => f.write_str("expected `:`"),
            ExpectedCommaOrBracket => f.write_str("expected `,` or `]`"),
            ExpectedCommaOrBrace => f.write_str("expected `,` or `}`"),
        }
    }
}

impl core::error::Error for Malformed {}

impl girders::FormatError for Malformed {}
