//! CBOR reader over any byte [`Source`]: a slice, an `io::Read`, or
//! chunks pushed in by the caller.
//!
//! Strict about well-formedness (RFC 8949 Appendix F): reserved values,
//! misplaced break codes, bad indefinite-length strings and truncated
//! input are all syntax errors. Text strings must be valid UTF-8. When a
//! push source runs dry before the end of input, reads fail with
//! `Problem::Incomplete` so a streaming driver can wait and retry.

use crate::bignum;
use crate::malformed::Malformed;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::Range;
use girders::half;
use girders::source::{SliceSource, Source};
use girders::{DecodeError, Kind, Number, Problem, Reader, Span};

type CborError = DecodeError<Span, Malformed>;
type CborResult<T> = Result<T, CborError>;

/// The "break" stop code that ends indefinite-length items.
const BREAK: u8 = 0xff;

/// Discard consumed input once this many bytes are buffered and no
/// checkpoint pins them.
const COMPACT_AT: usize = 64 * 1024;

/// An item's initial byte and argument.
#[derive(Debug, Clone, Copy)]
struct Head {
    major: u8,
    info: u8,
    /// The argument (a length, count, value or tag number); 0 when the
    /// length is indefinite.
    arg: u64,
    /// Bytes taken by the initial byte and argument.
    len: usize,
}

impl Head {
    fn indefinite(&self) -> bool {
        self.info == 31
    }
    fn is_break(&self) -> bool {
        self.major == 7 && self.info == 31
    }
}

/// An open array or map: items (or entries) left, or indefinite.
#[derive(Debug, Clone, Copy)]
enum Frame {
    Definite(u64),
    Indefinite,
}

/// A number in any of CBOR's encodings.
enum Numeric {
    Unsigned(u64),
    /// The value is `-1 - n`.
    Negative(u64),
    /// Tags 2 and 3: the value is `n`, or `-1 - n` if negative.
    Big {
        negative: bool,
        magnitude: Vec<u8>,
    },
    Float(f64),
    /// Tag 4: an exact decimal, as number text.
    Decimal(Box<str>),
}

/// Where the content of a string goes.
enum Sink<'b> {
    /// Validated as UTF-8 and appended.
    Text(&'b mut String),
    Bytes(&'b mut Vec<u8>),
    /// Checked for well-formedness only.
    Skip,
}

pub struct CborReader<S> {
    src: S,
    /// Index of the cursor in `src.buf()`.
    pos: usize,
    /// Absolute offset of `src.buf()[0]`: the input discarded so far.
    base: u64,
    stack: Vec<Frame>,
    /// Outstanding checkpoints. While any exist, nothing is discarded.
    pinned: usize,
    track: bool,
    last_key: Option<Span>,
}

#[derive(Debug, Clone)]
pub struct CborCheckpoint {
    at: u64,
    stack: Vec<Frame>,
    last_key: Option<Span>,
}

impl<'a> CborReader<SliceSource<'a>> {
    pub fn from_slice(input: &'a [u8]) -> Self {
        Self::new(SliceSource::new(input))
    }
}

impl<S: Source> CborReader<S> {
    pub fn new(src: S) -> Self {
        CborReader {
            src,
            pos: 0,
            base: 0,
            stack: Vec::new(),
            pinned: 0,
            track: true,
            last_key: None,
        }
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.src
    }

    /// Absolute byte offset of the cursor.
    pub fn offset(&self) -> u64 {
        self.base + self.pos as u64
    }

    /// Whether all input has been consumed. `Incomplete` if a push source
    /// has no more bytes yet but hasn't finished.
    pub fn at_end(&mut self) -> CborResult<bool> {
        Ok(!self.ensure(1)?)
    }

    // -- byte access ---------------------------------------------------------------

    /// Make `n` bytes available from the cursor; `false` if the input ends
    /// first.
    #[inline]
    fn ensure(&mut self, n: usize) -> CborResult<bool> {
        if self.src.buf().len() - self.pos >= n {
            Ok(true)
        } else {
            self.ensure_slow(n)
        }
    }

    #[cold]
    #[inline(never)]
    fn ensure_slow(&mut self, n: usize) -> CborResult<bool> {
        loop {
            if self.src.buf().len() - self.pos >= n {
                return Ok(true);
            }
            if self.src.is_eof() {
                return Ok(false);
            }
            match self.src.fill() {
                Ok(true) => continue,
                Ok(false) if self.src.is_eof() => return Ok(false),
                Ok(false) => {
                    return Err(DecodeError::at(
                        Problem::Incomplete,
                        Some(Span::point(self.offset())),
                    ));
                }
                Err(message) => {
                    return Err(DecodeError::at(
                        Problem::Io(message),
                        Some(Span::point(self.offset())),
                    ));
                }
            }
        }
    }

    /// Make `n` bytes available, or fail with Malformed::UnexpectedEnd.
    fn need(&mut self, n: usize) -> CborResult<()> {
        if self.ensure(n)? {
            Ok(())
        } else {
            Err(self.end_of_input())
        }
    }

    fn peek_byte(&mut self) -> CborResult<Option<u8>> {
        Ok(if self.ensure(1)? {
            Some(self.src.buf()[self.pos])
        } else {
            None
        })
    }

    /// Drop consumed input, between items only.
    fn compact(&mut self) {
        if S::COMPACTS && self.pos >= COMPACT_AT && self.pinned == 0 {
            self.src.discard(self.pos);
            self.base += self.pos as u64;
            self.pos = 0;
        }
    }

    /// Move the cursor to absolute offset `at` (not before the buffer).
    fn seek(&mut self, at: u64) {
        self.pos = (at - self.base) as usize;
    }

    // -- errors --------------------------------------------------------------------

    fn syntax(&self, at: u64, error: Malformed) -> CborError {
        DecodeError::at(Problem::Format(error), Some(Span::point(at)))
    }

    fn end_of_input(&self) -> CborError {
        let end = self.base + self.src.buf().len() as u64;
        self.syntax(end, Malformed::UnexpectedEnd)
    }

    fn kind_of(&self, head: Head) -> CborResult<Kind> {
        Ok(match head.major {
            0 | 1 => Kind::Number,
            2 => Kind::Bytes,
            3 => Kind::String,
            4 => Kind::Array,
            5 => Kind::Object,
            6 => Kind::Tag,
            _ => match head.info {
                20 | 21 => Kind::Bool,
                22 => Kind::Null,
                25..=27 => Kind::Number,
                31 => return Err(self.syntax(self.offset(), Malformed::UnexpectedBreak)),
                _ => Kind::Simple,
            },
        })
    }

    /// `TypeMismatch` against the item at the cursor (a syntax error if it
    /// is a break code).
    fn mismatch(&self, expected: &'static str, head: Head) -> CborError {
        match self.kind_of(head) {
            Ok(kind) => DecodeError::at(
                Problem::mismatch(expected, kind.name()),
                Some(Span::point(self.offset())),
            ),
            Err(e) => e,
        }
    }

    fn out_of_range(&self, at: u64, target: &str) -> CborError {
        DecodeError::at(
            Problem::OutOfRange(alloc::format!("number does not fit {target}")),
            Some(Span::new(at, self.offset())),
        )
    }

    // -- items ---------------------------------------------------------------------

    /// The head at the cursor, without consuming it.
    #[inline]
    fn peek_head(&mut self) -> CborResult<Head> {
        self.compact();
        // Fast path: the whole head is already buffered and has no
        // reserved values.
        let buf = self.src.buf();
        if let Some(&initial) = buf.get(self.pos) {
            let major = initial >> 5;
            let info = initial & 0x1f;
            if info < 24 {
                return Ok(Head {
                    major,
                    info,
                    arg: info as u64,
                    len: 1,
                });
            }
            if info < 28 && !(major == 7 && info == 24) {
                let n = 1usize << (info - 24);
                if let Some(bytes) = buf.get(self.pos + 1..self.pos + 1 + n) {
                    let arg = bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64);
                    return Ok(Head {
                        major,
                        info,
                        arg,
                        len: 1 + n,
                    });
                }
            }
        }
        self.peek_head_slow()
    }

    #[cold]
    #[inline(never)]
    fn peek_head_slow(&mut self) -> CborResult<Head> {
        let at = self.offset();
        let Some(initial) = self.peek_byte()? else {
            return Err(self.end_of_input());
        };
        let major = initial >> 5;
        let info = initial & 0x1f;
        let (arg, len) = match info {
            0..=23 => (info as u64, 1),
            24..=27 => {
                let n = 1usize << (info - 24);
                self.need(1 + n)?;
                let bytes = &self.src.buf()[self.pos + 1..self.pos + 1 + n];
                (
                    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64),
                    1 + n,
                )
            }
            28..=30 => return Err(self.syntax(at, Malformed::ReservedAdditionalInformation)),
            _ => {
                if matches!(major, 0 | 1 | 6) {
                    return Err(self.syntax(at, Malformed::IndefiniteLengthNotAllowed));
                }
                (0, 1)
            }
        };
        if major == 7 && info == 24 && arg < 32 {
            return Err(self.syntax(at, Malformed::TwoByteSimpleValueBelow32));
        }
        Ok(Head {
            major,
            info,
            arg,
            len,
        })
    }

    /// Consume `n` bytes of string content, returning where they are in
    /// the buffer.
    fn take(&mut self, n: u64) -> CborResult<Range<usize>> {
        let n = usize::try_from(n).map_err(|_| self.end_of_input())?;
        self.need(n)?;
        let range = self.pos..self.pos + n;
        self.pos += n;
        Ok(range)
    }

    fn deliver(&self, range: Range<usize>, sink: &mut Sink<'_>) -> CborResult<()> {
        let bytes = &self.src.buf()[range.clone()];
        match sink {
            Sink::Text(out) => match core::str::from_utf8(bytes) {
                Ok(text) => out.push_str(text),
                Err(e) => {
                    let at = self.base + (range.start + e.valid_up_to()) as u64;
                    return Err(self.syntax(at, Malformed::InvalidUtf8));
                }
            },
            Sink::Bytes(out) => out.extend_from_slice(bytes),
            Sink::Skip => {}
        }
        Ok(())
    }

    /// Read a definite or indefinite string of `major` type (2 bytes, 3
    /// text) into `sink`.
    fn read_chunks(
        &mut self,
        major: u8,
        expected: &'static str,
        mut sink: Sink<'_>,
    ) -> CborResult<()> {
        let head = self.peek_head()?;
        if head.major != major {
            return Err(self.mismatch(expected, head));
        }
        self.pos += head.len;
        if !head.indefinite() {
            let range = self.take(head.arg)?;
            return self.deliver(range, &mut sink);
        }
        loop {
            match self.peek_byte()? {
                None => return Err(self.end_of_input()),
                Some(BREAK) => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(_) => {}
            }
            let part = self.peek_head()?;
            if part.major != major || part.indefinite() {
                return Err(self.syntax(self.offset(), Malformed::InvalidStringChunk));
            }
            self.pos += part.len;
            let range = self.take(part.arg)?;
            self.deliver(range, &mut sink)?;
        }
    }

    fn read_numeric(&mut self, expected: &'static str) -> CborResult<Numeric> {
        let head = self.peek_head()?;
        let numeric = match (head.major, head.info) {
            (0, _) => Numeric::Unsigned(head.arg),
            (1, _) => Numeric::Negative(head.arg),
            (7, 25) => Numeric::Float(half::f16_to_f64(head.arg as u16)),
            (7, 26) => Numeric::Float(f32::from_bits(head.arg as u32) as f64),
            (7, 27) => Numeric::Float(f64::from_bits(head.arg)),
            (6, 2) | (6, 3) => {
                self.pos += head.len;
                let mut magnitude = Vec::new();
                self.read_chunks(2, "bignum byte string", Sink::Bytes(&mut magnitude))?;
                return Ok(Numeric::Big {
                    negative: head.arg == 3,
                    magnitude,
                });
            }
            (6, 4) => {
                let at = self.offset();
                self.pos += head.len;
                let pair = self.peek_head()?;
                if pair.major != 4 || pair.indefinite() || pair.arg != 2 {
                    return Err(self.syntax(at, Malformed::DecimalFractionNotAPair));
                }
                self.pos += pair.len;
                let exponent = match self.read_numeric("integer")? {
                    n @ (Numeric::Unsigned(_) | Numeric::Negative(_)) => Self::text_of(n),
                    _ => {
                        return Err(self.syntax(at, Malformed::DecimalFractionExponentNotAnInteger));
                    }
                };
                let mantissa = match self.read_numeric("integer")? {
                    n @ (Numeric::Unsigned(_) | Numeric::Negative(_) | Numeric::Big { .. }) => {
                        Self::text_of(n)
                    }
                    _ => {
                        return Err(self.syntax(at, Malformed::DecimalFractionMantissaNotAnInteger));
                    }
                };
                let text = if exponent == "0" {
                    mantissa
                } else {
                    alloc::format!("{mantissa}e{exponent}")
                };
                return Ok(Numeric::Decimal(text.into()));
            }
            _ => return Err(self.mismatch(expected, head)),
        };
        self.pos += head.len;
        Ok(numeric)
    }

    /// Exact decimal text (for floats, the shortest round-trip form).
    fn text_of(n: Numeric) -> String {
        match n {
            Numeric::Unsigned(v) => v.to_string(),
            Numeric::Negative(v) => alloc::format!("-{}", v as u128 + 1),
            Numeric::Big {
                negative: false,
                magnitude,
            } => bignum::to_decimal(&magnitude),
            Numeric::Big {
                negative: true,
                magnitude,
            } => alloc::format!("-{}", bignum::to_decimal(&bignum::add_one(&magnitude))),
            Numeric::Float(f) => alloc::format!("{f:?}"),
            Numeric::Decimal(text) => text.into(),
        }
    }

    /// The integer value, if it fits `i128`.
    fn integer(n: &Numeric) -> Option<i128> {
        match n {
            Numeric::Unsigned(v) => Some(*v as i128),
            Numeric::Negative(v) => Some(-1 - *v as i128),
            Numeric::Big {
                negative,
                magnitude,
            } => {
                let m = i128::try_from(bignum::to_u128(magnitude)?).ok()?;
                Some(if *negative { -1 - m } else { m })
            }
            _ => None,
        }
    }

    fn read_int<T: TryFrom<i128>>(&mut self, target: &'static str) -> CborResult<T> {
        let at = self.offset();
        // Pinned, so `at` stays in the buffer for the rewind below.
        self.pinned += 1;
        let n = self.read_numeric("int");
        self.pinned -= 1;
        let n = n?;
        if matches!(n, Numeric::Float(_) | Numeric::Decimal(_)) {
            self.seek(at);
            let found = if let Numeric::Float(_) = n {
                "float"
            } else {
                "decimal"
            };
            return Err(DecodeError::at(
                Problem::mismatch("int", found),
                Some(Span::point(at)),
            ));
        }
        Self::integer(&n)
            .and_then(|v| T::try_from(v).ok())
            .ok_or_else(|| self.out_of_range(at, target))
    }

    fn open(&mut self, major: u8, expected: &'static str) -> CborResult<()> {
        let head = self.peek_head()?;
        if head.major != major {
            return Err(self.mismatch(expected, head));
        }
        self.pos += head.len;
        self.stack.push(if head.indefinite() {
            Frame::Indefinite
        } else {
            Frame::Definite(head.arg)
        });
        Ok(())
    }

    /// Whether the open container has another item (or entry).
    fn has_next(&mut self) -> CborResult<bool> {
        match self.stack.last() {
            Some(Frame::Definite(0)) => Ok(false),
            Some(Frame::Definite(_)) => {
                if let Some(Frame::Definite(n)) = self.stack.last_mut() {
                    *n -= 1;
                }
                Ok(true)
            }
            Some(Frame::Indefinite) => match self.peek_byte()? {
                None => Err(self.end_of_input()),
                Some(BREAK) => Ok(false),
                Some(_) => Ok(true),
            },
            None => Err(self.syntax(self.offset(), Malformed::NoContainerOpen)),
        }
    }

    fn close(&mut self, what: Malformed) -> CborResult<()> {
        match self.stack.last() {
            Some(Frame::Definite(0)) => {}
            Some(Frame::Definite(_)) => {
                return Err(self.syntax(self.offset(), what));
            }
            Some(Frame::Indefinite) => match self.peek_byte()? {
                Some(BREAK) => self.pos += 1,
                None => return Err(self.end_of_input()),
                Some(_) => {
                    return Err(self.syntax(self.offset(), what));
                }
            },
            None => return Err(self.syntax(self.offset(), Malformed::NoContainerOpen)),
        }
        self.stack.pop();
        Ok(())
    }
}

impl<S: Source> Reader for CborReader<S> {
    type Location = Span;
    type Error = Malformed;
    type Checkpoint = CborCheckpoint;

    fn peek_kind(&mut self) -> CborResult<Kind> {
        if !self.ensure(1)? {
            return Err(self.end_of_input());
        }
        let head = self.peek_head()?;
        self.kind_of(head)
    }

    fn here(&mut self) -> Option<Span> {
        self.track.then(|| Span::point(self.offset()))
    }

    fn since(&mut self, start: &Span) -> Span {
        Span::new(start.start(), self.offset())
    }

    fn byte_span(&self, location: &Span) -> Option<Span> {
        Some(*location)
    }

    fn set_track_locations(&mut self, on: bool) {
        self.track = on;
    }

    fn key_location(&self) -> Option<Span> {
        self.last_key
    }

    fn read_bool(&mut self) -> CborResult<bool> {
        match self.peek_byte()? {
            Some(0xf4) => {
                self.pos += 1;
                Ok(false)
            }
            Some(0xf5) => {
                self.pos += 1;
                Ok(true)
            }
            _ => {
                let head = self.peek_head()?;
                Err(self.mismatch("bool", head))
            }
        }
    }

    fn read_null(&mut self) -> CborResult<()> {
        if self.peek_byte()? == Some(0xf6) {
            self.pos += 1;
            Ok(())
        } else {
            let head = self.peek_head()?;
            Err(self.mismatch("null", head))
        }
    }

    fn read_str(&mut self, buf: &mut String) -> CborResult<()> {
        self.read_chunks(3, "string", Sink::Text(buf))
    }

    fn read_bytes(&mut self, buf: &mut Vec<u8>) -> CborResult<()> {
        self.read_chunks(2, "bytes", Sink::Bytes(buf))
    }

    fn read_i64(&mut self) -> CborResult<i64> {
        self.read_int("i64")
    }

    fn read_u64(&mut self) -> CborResult<u64> {
        self.read_int("u64")
    }

    fn read_f64(&mut self) -> CborResult<f64> {
        let at = self.offset();
        match self.read_numeric("float")? {
            Numeric::Float(f) => Ok(f),
            Numeric::Unsigned(v) => Ok(v as f64),
            Numeric::Negative(v) => Ok(-(v as f64) - 1.0),
            n => Self::text_of(n)
                .parse()
                .map_err(|_| self.out_of_range(at, "f64")),
        }
    }

    fn read_number(&mut self) -> CborResult<Number> {
        let n = self.read_numeric("number")?;
        if let Numeric::Float(f) = n {
            return Ok(Number::F64(f));
        }
        if let Some(v) = Self::integer(&n) {
            if let Ok(i) = i64::try_from(v) {
                return Ok(Number::I64(i));
            }
            if let Ok(u) = u64::try_from(v) {
                return Ok(Number::U64(u));
            }
        }
        let decimal = matches!(n, Numeric::Decimal(_));
        let text = Self::text_of(n);
        if decimal {
            Ok(Number::F64(text.parse().unwrap_or(f64::NAN)))
        } else {
            Ok(Number::BigInt(text))
        }
    }

    fn read_number_text(&mut self, buf: &mut String) -> CborResult<()> {
        let at = self.offset();
        let n = self.read_numeric("number")?;
        if let Numeric::Float(f) = n
            && !f.is_finite()
        {
            return Err(self.out_of_range(at, "number text"));
        }
        buf.push_str(&Self::text_of(n));
        Ok(())
    }

    fn supports_tags(&self) -> bool {
        true
    }

    fn read_tag(&mut self) -> CborResult<u64> {
        let head = self.peek_head()?;
        if head.major != 6 {
            return Err(self.mismatch("tag", head));
        }
        self.pos += head.len;
        Ok(head.arg)
    }

    fn read_simple(&mut self) -> CborResult<u8> {
        let head = self.peek_head()?;
        if head.major != 7 || !matches!(head.info, 0..=19 | 23 | 24) {
            return Err(self.mismatch("simple value", head));
        }
        self.pos += head.len;
        Ok(head.arg as u8)
    }

    fn begin_array(&mut self) -> CborResult<()> {
        self.open(4, "array")
    }

    fn size_hint(&self) -> Option<usize> {
        match self.stack.last() {
            Some(Frame::Definite(n)) => usize::try_from(*n).ok(),
            _ => None,
        }
    }

    fn array_has_next(&mut self, _first: bool) -> CborResult<bool> {
        self.has_next()
    }

    fn end_array(&mut self) -> CborResult<()> {
        self.close(Malformed::ArrayHasMoreItems)
    }

    fn begin_object(&mut self) -> CborResult<()> {
        self.open(5, "object")
    }

    fn object_next_key(&mut self, _first: bool, buf: &mut String) -> CborResult<bool> {
        if !self.has_next()? {
            return Ok(false);
        }
        let head = self.peek_head()?;
        if head.major != 3 {
            return Err(self.mismatch("string key", head));
        }
        let start = self.offset();
        self.read_str(buf)?;
        self.last_key = Some(Span::new(start, self.offset()));
        Ok(true)
    }

    fn object_next_entry(&mut self, _first: bool) -> CborResult<bool> {
        self.has_next()
    }

    fn end_object(&mut self) -> CborResult<()> {
        self.close(Malformed::MapHasMoreItems)
    }

    fn skip_value(&mut self) -> CborResult<()> {
        /// An open container while skipping.
        struct Level {
            /// Items left, or `None` for indefinite length.
            remaining: Option<u64>,
            map: bool,
            /// An indefinite map has read a key without its value.
            in_entry: bool,
        }
        let mut levels = alloc::vec![Level {
            remaining: Some(1),
            map: false,
            in_entry: false,
        }];
        while let Some(level) = levels.last_mut() {
            match &mut level.remaining {
                Some(0) => {
                    levels.pop();
                    continue;
                }
                Some(n) => *n -= 1,
                None => {
                    let (map, in_entry) = (level.map, level.in_entry);
                    match self.peek_byte()? {
                        None => return Err(self.end_of_input()),
                        Some(BREAK) => {
                            if map && in_entry {
                                return Err(self.syntax(self.offset(), Malformed::BreakInMapValue));
                            }
                            self.pos += 1;
                            levels.pop();
                            continue;
                        }
                        Some(_) => {}
                    }
                    if let Some(level) = levels.last_mut() {
                        level.in_entry = level.map && !level.in_entry;
                    }
                }
            }
            let head = self.peek_head()?;
            if head.is_break() {
                return Err(self.syntax(self.offset(), Malformed::UnexpectedBreak));
            }
            let opened = match head.major {
                2 | 3 => {
                    self.read_chunks(head.major, "string", Sink::Skip)?;
                    None
                }
                4..=6 => {
                    self.pos += head.len;
                    let remaining = match head.major {
                        _ if head.indefinite() => None,
                        5 => Some(head.arg.saturating_mul(2)),
                        6 => Some(1),
                        _ => Some(head.arg),
                    };
                    Some(Level {
                        remaining,
                        map: head.major == 5,
                        in_entry: false,
                    })
                }
                _ => {
                    self.pos += head.len;
                    None
                }
            };
            if let Some(level) = opened {
                levels.push(level);
            }
        }
        Ok(())
    }

    fn checkpoint(&mut self) -> CborCheckpoint {
        self.pinned += 1;
        CborCheckpoint {
            at: self.offset(),
            stack: self.stack.clone(),
            last_key: self.last_key,
        }
    }

    fn rewind(&mut self, cp: &CborCheckpoint) {
        self.seek(cp.at);
        self.stack.clone_from(&cp.stack);
        self.last_key = cp.last_key;
    }

    fn release(&mut self, _cp: CborCheckpoint) {
        self.pinned -= 1;
    }
}
