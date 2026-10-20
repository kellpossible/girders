//! Byte-range spans.
//!
//! `Span` is the location type of byte-oriented formats (JSON). Formats
//! with other notions of location (an env-var name, a record index) use
//! their own `Reader::Location`; [`crate::Reader::byte_span`] projects a
//! location to a `Span` when one exists, which is what [`Spanned`] and
//! `Value` trees record.

use core::num::NonZeroU64;

/// A half-open `[start, end)` byte range into the original input.
///
/// 16 bytes, and so is `Option<Span>`: the end is stored as `end + 1` in a
/// `NonZeroU64`, which leaves the zero bit pattern free for `None`.
/// Offsets go up to `u64::MAX - 1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    start: u64,
    end_plus_one: NonZeroU64,
}

impl Span {
    /// Panics if `end` is `u64::MAX`.
    pub const fn new(start: u64, end: u64) -> Self {
        match NonZeroU64::new(end.wrapping_add(1)) {
            Some(end_plus_one) => Span {
                start,
                end_plus_one,
            },
            None => panic!("span end out of range"),
        }
    }

    pub const fn point(at: u64) -> Self {
        Self::new(at, at)
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end_plus_one.get() - 1
    }

    pub const fn len(self) -> u64 {
        self.end().saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Covering span from the start of `self` to the end of `other`.
    pub const fn to(self, other: Span) -> Span {
        Span {
            start: self.start,
            end_plus_one: other.end_plus_one,
        }
    }
}

impl core::fmt::Debug for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Span({}..{})", self.start, self.end())
    }
}

/// `byte 71`, or `bytes 71..80` for a range.
impl core::fmt::Display for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_empty() {
            write!(f, "byte {}", self.start)
        } else {
            write!(f, "bytes {}..{}", self.start, self.end())
        }
    }
}

/// A value with the source span it was decoded from. `span` is `None`
/// when location tracking is off or the format has no byte offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Option<Span>,
}
