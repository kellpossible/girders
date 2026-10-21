//! CBOR reader over a byte slice.
//!
//! Strict about well-formedness (RFC 8949 Appendix F): reserved values,
//! misplaced break codes, bad indefinite-length strings and truncated
//! input are all syntax errors. Text strings must be valid UTF-8.

use crate::{bignum, half};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use girders::{DecodeError, Kind, Number, Problem, Reader, Span};

type CborError = DecodeError<Span>;
type CborResult<T> = Result<T, CborError>;

/// The "break" stop code that ends indefinite-length items.
const BREAK: u8 = 0xff;

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

pub struct CborReader<'a> {
    input: &'a [u8],
    pos: usize,
    stack: Vec<Frame>,
    track: bool,
    last_key: Option<Span>,
}

#[derive(Debug, Clone)]
pub struct CborCheckpoint {
    pos: usize,
    stack: Vec<Frame>,
    last_key: Option<Span>,
}

impl<'a> CborReader<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        CborReader {
            input,
            pos: 0,
            stack: Vec::new(),
            track: true,
            last_key: None,
        }
    }

    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Whether all input has been consumed.
    pub fn at_end(&self) -> bool {
        self.pos == self.input.len()
    }

    fn syntax(&self, at: usize, message: &str) -> CborError {
        DecodeError::at(
            Problem::Syntax(message.into()),
            Some(Span::point(at as u64)),
        )
    }

    fn end_of_input(&self) -> CborError {
        self.syntax(self.input.len(), "unexpected end of input")
    }

    fn peek_head(&self) -> CborResult<Head> {
        let at = self.pos;
        let Some(&initial) = self.input.get(at) else {
            return Err(self.end_of_input());
        };
        let major = initial >> 5;
        let info = initial & 0x1f;
        let (arg, len) = match info {
            0..=23 => (info as u64, 1),
            24..=27 => {
                let n = 1usize << (info - 24);
                let bytes = self
                    .input
                    .get(at + 1..at + 1 + n)
                    .ok_or_else(|| self.end_of_input())?;
                (
                    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64),
                    1 + n,
                )
            }
            28..=30 => return Err(self.syntax(at, "reserved additional information value")),
            _ => {
                if matches!(major, 0 | 1 | 6) {
                    return Err(self.syntax(at, "indefinite length is not allowed here"));
                }
                (0, 1)
            }
        };
        if major == 7 && info == 24 && arg < 32 {
            return Err(self.syntax(at, "simple value below 32 in two-byte form"));
        }
        Ok(Head {
            major,
            info,
            arg,
            len,
        })
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
                31 => return Err(self.syntax(self.pos, "unexpected break stop code")),
                _ => Kind::Simple,
            },
        })
    }

    /// `TypeMismatch` against the item at the current position (a syntax
    /// error if it is a break code).
    fn mismatch(&self, expected: &'static str, head: Head) -> CborError {
        match self.kind_of(head) {
            Ok(kind) => DecodeError::at(
                Problem::mismatch(expected, kind.name()),
                Some(Span::point(self.pos as u64)),
            ),
            Err(e) => e,
        }
    }

    fn out_of_range(&self, at: usize, target: &str) -> CborError {
        DecodeError::at(
            Problem::OutOfRange(alloc::format!("number does not fit {target}")),
            Some(Span::new(at as u64, self.pos as u64)),
        )
    }

    /// Consume `n` bytes of string content.
    fn take(&mut self, n: u64) -> CborResult<&'a [u8]> {
        let available = self.input.len() - self.pos;
        let n = usize::try_from(n)
            .ok()
            .filter(|&n| n <= available)
            .ok_or_else(|| self.end_of_input())?;
        let bytes = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(bytes)
    }

    /// Read a definite or indefinite string of `major` type (2 bytes, 3
    /// text), handing each chunk to `chunk`.
    fn read_chunks(
        &mut self,
        major: u8,
        expected: &'static str,
        mut chunk: impl FnMut(&Self, usize, &'a [u8]) -> CborResult<()>,
    ) -> CborResult<()> {
        let head = self.peek_head()?;
        if head.major != major {
            return Err(self.mismatch(expected, head));
        }
        self.pos += head.len;
        if !head.indefinite() {
            let at = self.pos;
            let bytes = self.take(head.arg)?;
            return chunk(self, at, bytes);
        }
        loop {
            match self.input.get(self.pos) {
                None => return Err(self.end_of_input()),
                Some(&BREAK) => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(_) => {}
            }
            let part = self.peek_head()?;
            if part.major != major || part.indefinite() {
                return Err(self.syntax(
                    self.pos,
                    "indefinite-length string chunks must be definite strings of the same type",
                ));
            }
            self.pos += part.len;
            let at = self.pos;
            let bytes = self.take(part.arg)?;
            chunk(self, at, bytes)?;
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
                self.read_chunks(2, "bignum byte string", |_, _, b| {
                    magnitude.extend_from_slice(b);
                    Ok(())
                })?;
                return Ok(Numeric::Big {
                    negative: head.arg == 3,
                    magnitude,
                });
            }
            (6, 4) => {
                let at = self.pos;
                self.pos += head.len;
                let pair = self.peek_head()?;
                if pair.major != 4 || pair.indefinite() || pair.arg != 2 {
                    return Err(self.syntax(at, "a decimal fraction is a two-element array"));
                }
                self.pos += pair.len;
                let exponent = match self.read_numeric("integer")? {
                    n @ (Numeric::Unsigned(_) | Numeric::Negative(_)) => Self::text_of(n)?,
                    _ => return Err(self.syntax(at, "a decimal fraction's exponent is an integer")),
                };
                let mantissa = match self.read_numeric("integer")? {
                    n @ (Numeric::Unsigned(_) | Numeric::Negative(_) | Numeric::Big { .. }) => {
                        Self::text_of(n)?
                    }
                    _ => return Err(self.syntax(at, "a decimal fraction's mantissa is an integer")),
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

    /// Exact decimal text. `None` for NaN and the infinities.
    fn text_of(n: Numeric) -> CborResult<String> {
        Ok(match n {
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
        })
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
        let at = self.pos;
        let n = self.read_numeric("int")?;
        if matches!(n, Numeric::Float(_) | Numeric::Decimal(_)) {
            self.pos = at;
            let found = if let Numeric::Float(_) = n {
                "float"
            } else {
                "decimal"
            };
            return Err(DecodeError::at(
                Problem::mismatch("int", found),
                Some(Span::point(at as u64)),
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
        match self.stack.last_mut() {
            Some(Frame::Definite(0)) => Ok(false),
            Some(Frame::Definite(n)) => {
                *n -= 1;
                Ok(true)
            }
            Some(Frame::Indefinite) => match self.input.get(self.pos) {
                None => Err(self.end_of_input()),
                Some(&BREAK) => Ok(false),
                Some(_) => Ok(true),
            },
            None => Err(self.syntax(self.pos, "no array or map is open")),
        }
    }

    fn close(&mut self, what: &str) -> CborResult<()> {
        match self.stack.pop() {
            Some(Frame::Definite(0)) => Ok(()),
            Some(Frame::Definite(_)) => {
                Err(self.syntax(self.pos, &alloc::format!("{what} has more items")))
            }
            Some(Frame::Indefinite) => match self.input.get(self.pos) {
                Some(&BREAK) => {
                    self.pos += 1;
                    Ok(())
                }
                None => Err(self.end_of_input()),
                Some(_) => Err(self.syntax(self.pos, &alloc::format!("{what} has more items"))),
            },
            None => Err(self.syntax(self.pos, "no array or map is open")),
        }
    }
}

impl Reader for CborReader<'_> {
    type Location = Span;
    type Checkpoint = CborCheckpoint;

    fn peek_kind(&mut self) -> CborResult<Kind> {
        if self.at_end() {
            return Ok(Kind::Eof);
        }
        let head = self.peek_head()?;
        self.kind_of(head)
    }

    fn here(&mut self) -> Option<Span> {
        self.track.then(|| Span::point(self.pos as u64))
    }

    fn since(&mut self, start: &Span) -> Span {
        Span::new(start.start(), self.pos as u64)
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
        match self.input.get(self.pos) {
            Some(0xf4) => {
                self.pos += 1;
                Ok(false)
            }
            Some(0xf5) => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(self.mismatch("bool", self.peek_head()?)),
        }
    }

    fn read_null(&mut self) -> CborResult<()> {
        if self.input.get(self.pos) == Some(&0xf6) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.mismatch("null", self.peek_head()?))
        }
    }

    fn read_str(&mut self, buf: &mut String) -> CborResult<()> {
        self.read_chunks(3, "string", |r, at, bytes| {
            let text = core::str::from_utf8(bytes)
                .map_err(|e| r.syntax(at + e.valid_up_to(), "invalid UTF-8 in a text string"))?;
            buf.push_str(text);
            Ok(())
        })
    }

    fn read_bytes(&mut self, buf: &mut Vec<u8>) -> CborResult<()> {
        self.read_chunks(2, "bytes", |_, _, bytes| {
            buf.extend_from_slice(bytes);
            Ok(())
        })
    }

    fn read_i64(&mut self) -> CborResult<i64> {
        self.read_int("i64")
    }

    fn read_u64(&mut self) -> CborResult<u64> {
        self.read_int("u64")
    }

    fn read_f64(&mut self) -> CborResult<f64> {
        let at = self.pos;
        match self.read_numeric("float")? {
            Numeric::Float(f) => Ok(f),
            Numeric::Unsigned(v) => Ok(v as f64),
            Numeric::Negative(v) => Ok(-(v as f64) - 1.0),
            n => {
                let text = Self::text_of(n)?;
                text.parse().map_err(|_| self.out_of_range(at, "f64"))
            }
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
        let text = Self::text_of(n)?;
        if decimal {
            Ok(Number::F64(text.parse().unwrap_or(f64::NAN)))
        } else {
            Ok(Number::BigInt(text))
        }
    }

    fn read_number_text(&mut self, buf: &mut String) -> CborResult<()> {
        let at = self.pos;
        let n = self.read_numeric("number")?;
        if let Numeric::Float(f) = n
            && !f.is_finite()
        {
            return Err(self.out_of_range(at, "number text"));
        }
        buf.push_str(&Self::text_of(n)?);
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

    fn array_has_next(&mut self, _first: bool) -> CborResult<bool> {
        self.has_next()
    }

    fn end_array(&mut self) -> CborResult<()> {
        self.close("array")
    }

    fn begin_object(&mut self) -> CborResult<()> {
        self.open(5, "object")
    }

    fn object_next_key(&mut self, _first: bool, buf: &mut String) -> CborResult<bool> {
        if !self.has_next()? {
            return Ok(false);
        }
        let start = self.pos as u64;
        let head = self.peek_head()?;
        if head.major != 3 {
            return Err(self.mismatch("string key", head));
        }
        self.read_str(buf)?;
        self.last_key = Some(Span::new(start, self.pos as u64));
        Ok(true)
    }

    fn object_next_entry(&mut self, _first: bool) -> CborResult<bool> {
        self.has_next()
    }

    fn end_object(&mut self) -> CborResult<()> {
        self.close("map")
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
                    match self.input.get(self.pos) {
                        None => return Err(self.end_of_input()),
                        Some(&BREAK) => {
                            if level.map && level.in_entry {
                                return Err(
                                    self.syntax(self.pos, "break in a map's value position")
                                );
                            }
                            self.pos += 1;
                            levels.pop();
                            continue;
                        }
                        Some(_) => {}
                    }
                    level.in_entry = level.map && !level.in_entry;
                }
            }
            let head = self.peek_head()?;
            if head.is_break() {
                return Err(self.syntax(self.pos, "unexpected break stop code"));
            }
            let opened = match head.major {
                2 | 3 => {
                    self.read_chunks(head.major, "string", |_, _, _| Ok(()))?;
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
        CborCheckpoint {
            pos: self.pos,
            stack: self.stack.clone(),
            last_key: self.last_key,
        }
    }

    fn rewind(&mut self, cp: &CborCheckpoint) {
        self.pos = cp.pos;
        self.stack.clone_from(&cp.stack);
        self.last_key = cp.last_key;
    }

    fn release(&mut self, _cp: CborCheckpoint) {}
}
