//! Strict RFC 8259 reader over any [`Source`].
//!
//! Rejects NaN, Infinity, hex, leading zeros, trailing commas, comments,
//! lone surrogates, unescaped control characters, invalid UTF-8, and any
//! whitespace other than space, tab, LF and CR.
//!
//! When the source's buffer runs out before input has ended, reads return
//! `Problem::Incomplete` (see `stream`). For complete inputs that never
//! happens.

use crate::malformed::Malformed;
use crate::source::Source;
use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;
use girders::codec::NumberText;
use girders::reader::DecodeResult;
use girders::{DecodeError, Kind, Number, Problem, Reader, Span};

type JsonError = DecodeError<Span, Malformed>;
type JsonResult<T> = Result<T, JsonError>;

/// Compact once this many consumed bytes are buffered and nothing pins
/// them.
const COMPACT_AT: usize = 64 * 1024;

pub struct JsonReader<S> {
    src: S,
    /// Index into `src.buf()`.
    pos: usize,
    /// Absolute offset of `src.buf()[0]`.
    base: u64,
    track: bool,
    /// Outstanding checkpoint positions (absolute, non-decreasing).
    cps: Vec<u64>,
    scratch: Vec<u8>,
    /// Span of the last key read (when tracking locations).
    last_key: Option<Span>,
    /// A key read by `object_next_entry`, not yet read as a value.
    pending_key: Option<PendingKey>,
}

/// An object key standing in for a value (see `object_next_entry`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingKey {
    text: String,
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonCheckpoint {
    at: u64,
    pending_key: Option<PendingKey>,
}

struct NumberToken {
    start: usize,
    end: usize,
    start_abs: u64,
    is_float: bool,
}

impl<S: Source> JsonReader<S> {
    pub fn new(src: S) -> Self {
        JsonReader {
            src,
            pos: 0,
            base: 0,
            track: true,
            cps: Vec::new(),
            scratch: Vec::new(),
            last_key: None,
            pending_key: None,
        }
    }

    pub fn source(&self) -> &S {
        &self.src
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.src
    }

    /// Absolute byte offset of the cursor.
    #[inline]
    pub fn offset(&self) -> u64 {
        self.base + self.pos as u64
    }

    /// `true` once only whitespace remains.
    pub fn at_end(&mut self) -> JsonResult<bool> {
        self.skip_ws()?;
        Ok(self.peek()?.is_none())
    }

    // -- byte access -----------------------------------------------------------

    #[inline(always)]
    fn peek(&mut self) -> JsonResult<Option<u8>> {
        match self.src.buf().get(self.pos) {
            Some(&b) => Ok(Some(b)),
            None => self.peek_slow(),
        }
    }

    #[cold]
    #[inline(never)]
    fn peek_slow(&mut self) -> JsonResult<Option<u8>> {
        loop {
            if let Some(&b) = self.src.buf().get(self.pos) {
                return Ok(Some(b));
            }
            if self.src.is_eof() {
                return Ok(None);
            }
            match self.src.fill() {
                Ok(true) => continue,
                Ok(false) if self.src.is_eof() => return Ok(None),
                Ok(false) => {
                    return Err(DecodeError::at(
                        Problem::Incomplete,
                        self.loc(self.offset()),
                    ));
                }
                Err(msg) => return Err(DecodeError::at(Problem::Io(msg), self.loc(self.offset()))),
            }
        }
    }

    #[inline]
    fn loc(&self, at: u64) -> Option<Span> {
        if self.track {
            Some(Span::point(at))
        } else {
            None
        }
    }

    fn syntax(&self, at: u64, error: Malformed) -> JsonError {
        DecodeError::at(Problem::Format(error), self.loc(at))
    }

    #[inline]
    fn skip_ws(&mut self) -> JsonResult<()> {
        loop {
            match self.src.buf().get(self.pos) {
                Some(b' ' | b'\n' | b'\r' | b'\t') => self.pos += 1,
                Some(_) => break,
                None => match self.peek_slow()? {
                    Some(_) => continue,
                    None => break,
                },
            }
        }
        if S::COMPACTS && self.pos >= COMPACT_AT && self.cps.is_empty() {
            self.src.discard(self.pos);
            self.base += self.pos as u64;
            self.pos = 0;
        }
        Ok(())
    }

    fn found(&mut self) -> Cow<'static, str> {
        match self.peek() {
            Ok(None) | Err(_) => "end of input".into(),
            Ok(Some(b)) => match classify(b) {
                Some(k) => k.name().into(),
                None => describe_byte(b).into(),
            },
        }
    }

    fn mismatch(&mut self, expected: &'static str) -> JsonError {
        if let Err(e) = self.skip_ws() {
            return e;
        }
        let at = self.offset();
        match self.peek() {
            Err(e) => e,
            Ok(None) => self.syntax(at, Malformed::UnexpectedEnd),
            Ok(Some(_)) => DecodeError::at(Problem::mismatch(expected, self.found()), self.loc(at)),
        }
    }

    #[inline]
    fn expect_byte(&mut self, want: u8, expected: &'static str) -> JsonResult<()> {
        self.skip_ws()?;
        if self.peek()? == Some(want) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.mismatch(expected))
        }
    }

    // -- strings ---------------------------------------------------------------

    /// After the opening quote. Appends to `out` when given; otherwise
    /// validates and discards (skipping).
    fn string_body(&mut self, mut out: Option<&mut String>, start: u64) -> JsonResult<()> {
        // Fast path: no escapes, closing quote within the buffer.
        {
            let buf = self.src.buf();
            let s = self.pos;
            let mut i = s;
            while i < buf.len() {
                let b = buf[i];
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                i += 1;
            }
            if i < buf.len() && buf[i] == b'"' {
                if let Some(t) = self.src.text(s, i) {
                    if let Some(out) = out.as_deref_mut() {
                        out.push_str(t);
                    }
                    self.pos = i + 1;
                    return Ok(());
                }
                match core::str::from_utf8(&buf[s..i]) {
                    Ok(t) => {
                        if let Some(out) = out.as_deref_mut() {
                            out.push_str(t);
                        }
                        self.pos = i + 1;
                        return Ok(());
                    }
                    Err(_) => return Err(self.syntax(start, Malformed::InvalidUtf8)),
                }
            }
        }
        // Slow path: escapes or a chunk boundary.
        self.scratch.clear();
        loop {
            let b = match self.peek()? {
                None => return Err(self.syntax(self.offset(), Malformed::UnterminatedString)),
                Some(b) => b,
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    let text = core::str::from_utf8(&self.scratch)
                        .map_err(|_| self.syntax(start, Malformed::InvalidUtf8))?;
                    if let Some(out) = out.as_deref_mut() {
                        out.push_str(text);
                    }
                    return Ok(());
                }
                b'\\' => {
                    let at = self.offset();
                    self.pos += 1;
                    self.read_escape(at)?;
                }
                b if b < 0x20 => {
                    return Err(self.syntax(self.offset(), Malformed::ControlCharacterInString));
                }
                _ => {
                    let buf = self.src.buf();
                    let s = self.pos;
                    let mut i = s;
                    while i < buf.len() {
                        let b = buf[i];
                        if b == b'"' || b == b'\\' || b < 0x20 {
                            break;
                        }
                        i += 1;
                    }
                    self.scratch.extend_from_slice(&buf[s..i]);
                    self.pos = i;
                }
            }
        }
    }

    fn next_byte(&mut self, at_end: Malformed) -> JsonResult<u8> {
        match self.peek()? {
            Some(b) => {
                self.pos += 1;
                Ok(b)
            }
            None => Err(self.syntax(self.offset(), at_end)),
        }
    }

    fn read_escape(&mut self, at: u64) -> JsonResult<()> {
        let b = self.next_byte(Malformed::UnterminatedEscape)?;
        let c = match b {
            b'"' => b'"',
            b'\\' => b'\\',
            b'/' => b'/',
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'u' => return self.read_unicode_escape(at),
            other => {
                return Err(self.syntax(at, Malformed::InvalidEscape(other)));
            }
        };
        self.scratch.push(c);
        Ok(())
    }

    fn read_hex4(&mut self, at: u64) -> JsonResult<u32> {
        let mut n = 0u32;
        for _ in 0..4 {
            let b = self.next_byte(Malformed::IncompleteUnicodeEscape)?;
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => {
                    return Err(self.syntax(at, Malformed::InvalidHexDigit(b)));
                }
            };
            n = n * 16 + d as u32;
        }
        Ok(n)
    }

    fn read_unicode_escape(&mut self, at: u64) -> JsonResult<()> {
        let n = self.read_hex4(at)?;
        let cp = if (0xD800..0xDC00).contains(&n) {
            let lone = |me: &Self| me.syntax(at, Malformed::LoneHighSurrogate(n as u16));
            if self.peek()? != Some(b'\\') {
                return Err(lone(self));
            }
            self.pos += 1;
            if self.peek()? != Some(b'u') {
                return Err(lone(self));
            }
            self.pos += 1;
            let low = self.read_hex4(at)?;
            if !(0xDC00..0xE000).contains(&low) {
                return Err(self.syntax(at, Malformed::ExpectedLowSurrogate(n as u16)));
            }
            0x10000 + ((n - 0xD800) << 10) + (low - 0xDC00)
        } else if (0xDC00..0xE000).contains(&n) {
            return Err(self.syntax(at, Malformed::LoneLowSurrogate(n as u16)));
        } else {
            n
        };
        let ch =
            char::from_u32(cp).ok_or_else(|| self.syntax(at, Malformed::InvalidUnicodeEscape))?;
        let mut tmp = [0u8; 4];
        self.scratch
            .extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
        Ok(())
    }

    // -- numbers ---------------------------------------------------------------

    fn digits(&mut self) -> JsonResult<usize> {
        let mut n = 0;
        while let Some(b'0'..=b'9') = self.peek()? {
            self.pos += 1;
            n += 1;
        }
        Ok(n)
    }

    /// Lex a number at the cursor (whitespace already skipped).
    fn lex_number(&mut self) -> JsonResult<NumberToken> {
        let start = self.pos;
        let start_abs = self.offset();
        if self.peek()? == Some(b'-') {
            self.pos += 1;
        }
        match self.peek()? {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                self.digits()?;
            }
            _ => return Err(self.syntax(self.offset(), Malformed::ExpectedDigit)),
        }
        let mut is_float = false;
        if self.peek()? == Some(b'.') {
            self.pos += 1;
            is_float = true;
            if self.digits()? == 0 {
                return Err(self.syntax(self.offset(), Malformed::ExpectedFractionDigit));
            }
        }
        if let Some(b'e' | b'E') = self.peek()? {
            self.pos += 1;
            is_float = true;
            if let Some(b'+' | b'-') = self.peek()? {
                self.pos += 1;
            }
            if self.digits()? == 0 {
                return Err(self.syntax(self.offset(), Malformed::ExpectedExponentDigit));
            }
        }
        Ok(NumberToken {
            start,
            end: self.pos,
            start_abs,
            is_float,
        })
    }

    /// Skip whitespace, check a number starts here, lex it.
    #[inline]
    fn number(&mut self, expected: &'static str) -> JsonResult<NumberToken> {
        self.skip_ws()?;
        match self.peek()? {
            Some(b'-' | b'0'..=b'9') => self.lex_number(),
            _ => Err(self.mismatch(expected)),
        }
    }

    fn tok_text(&self, t: &NumberToken) -> &str {
        // Lexed bytes are ASCII.
        match self.src.text(t.start, t.end) {
            Some(text) => text,
            None => core::str::from_utf8(&self.src.buf()[t.start..t.end]).unwrap_or(""),
        }
    }

    fn tok_span(&self, t: &NumberToken) -> Option<Span> {
        if self.track {
            Some(Span::new(t.start_abs, self.base + t.end as u64))
        } else {
            None
        }
    }

    fn out_of_range(&self, t: &NumberToken, ty: &str) -> JsonError {
        DecodeError::at(
            Problem::OutOfRange(alloc::format!(
                "{} is out of range for {ty}",
                self.tok_text(t)
            )),
            self.tok_span(t),
        )
    }

    fn int_tok(&mut self) -> JsonResult<NumberToken> {
        let t = self.number("int")?;
        if t.is_float {
            return Err(DecodeError::at(
                Problem::mismatch("int", "float"),
                self.tok_span(&t),
            ));
        }
        Ok(t)
    }

    /// Magnitude of an integer token, and whether it was negative.
    fn int_magnitude(&self, t: &NumberToken) -> Option<(bool, u64)> {
        let bytes = &self.src.buf()[t.start..t.end];
        let (neg, digits) = match bytes.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, bytes),
        };
        let mut acc: u64 = 0;
        for &d in digits {
            acc = acc.checked_mul(10)?.checked_add((d - b'0') as u64)?;
        }
        Some((neg, acc))
    }

    fn keyword(&mut self, word: &'static [u8], expected: &'static str) -> JsonResult<()> {
        self.skip_ws()?;
        if self.peek()? != Some(word[0]) {
            return Err(self.mismatch(expected));
        }
        let at = self.offset();
        for &w in word {
            match self.peek()? {
                Some(b) if b == w => self.pos += 1,
                _ => return Err(self.syntax(at, Malformed::InvalidLiteral)),
            }
        }
        Ok(())
    }

    fn key_after_separator(&mut self, buf: &mut String) -> JsonResult<()> {
        self.skip_ws()?;
        match self.peek()? {
            Some(b'"') => {
                let start = self.offset();
                self.pos += 1;
                self.string_body(Some(buf), start)?;
                if self.track {
                    self.last_key = Some(Span::new(start, self.offset()));
                }
            }
            None => return Err(self.syntax(self.offset(), Malformed::UnexpectedEnd)),
            _ => return Err(self.syntax(self.offset(), Malformed::ExpectedKey)),
        }
        self.skip_ws()?;
        match self.peek()? {
            Some(b':') => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(self.syntax(self.offset(), Malformed::ExpectedColon)),
        }
    }

    fn skip_key(&mut self) -> JsonResult<()> {
        self.skip_ws()?;
        match self.peek()? {
            Some(b'"') => {
                let start = self.offset();
                self.pos += 1;
                self.string_body(None, start)?;
            }
            None => return Err(self.syntax(self.offset(), Malformed::UnexpectedEnd)),
            _ => return Err(self.syntax(self.offset(), Malformed::ExpectedKey)),
        }
        self.skip_ws()?;
        match self.peek()? {
            Some(b':') => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(self.syntax(self.offset(), Malformed::ExpectedColon)),
        }
    }
}

fn classify(b: u8) -> Option<Kind> {
    Some(match b {
        b'"' => Kind::String,
        b'-' | b'0'..=b'9' => Kind::Number,
        b't' | b'f' => Kind::Bool,
        b'n' => Kind::Null,
        b'[' => Kind::Array,
        b'{' => Kind::Object,
        _ => return None,
    })
}

fn describe_raw(b: u8) -> String {
    if b.is_ascii_graphic() || b == b' ' {
        alloc::format!("{}", b as char)
    } else {
        alloc::format!("\\x{b:02x}")
    }
}

fn describe_byte(b: u8) -> String {
    alloc::format!("`{}`", describe_raw(b))
}

/// A key's text as a dynamic number.
fn key_as_number(text: &str) -> Result<Number, Problem<Malformed>> {
    match girders::codec::number_grammar(text) {
        None => Err(Problem::mismatch("number", alloc::format!("{text:?}"))),
        Some(true) => girders::codec::f64().parse_text(text).map(Number::F64),
        Some(false) => Ok(if let Ok(i) = text.parse::<i64>() {
            Number::I64(i)
        } else if let Ok(u) = text.parse::<u64>() {
            Number::U64(u)
        } else {
            Number::BigInt(text.into())
        }),
    }
}

// Reading an object key as a value (after `object_next_entry`). JSON keys
// are strings, so a key reads as a string, or as a number or bool parsed
// from its text. Out of line: records never take this path.
impl<S: Source> JsonReader<S> {
    /// The cursor's offset after whitespace.
    fn here_offset(&mut self) -> JsonResult<u64> {
        self.skip_ws()?;
        Ok(self.offset())
    }

    #[cold]
    fn key_str(&mut self, buf: &mut String) -> DecodeResult<(), Self> {
        let key = self.pending_key.take().expect("a pending key");
        if buf.is_empty() {
            *buf = key.text;
        } else {
            buf.push_str(&key.text);
        }
        Ok(())
    }

    #[cold]
    fn key_number<T>(
        &mut self,
        parse: impl FnOnce(&str) -> Result<T, Problem<Malformed>>,
    ) -> DecodeResult<T, Self> {
        let key = self.pending_key.take().expect("a pending key");
        parse(&key.text).map_err(|p| DecodeError::at(p, Some(key.span)))
    }

    #[cold]
    fn key_bool(&mut self) -> DecodeResult<bool, Self> {
        self.key_number(|t| match t {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(Problem::mismatch("bool", alloc::format!("{t:?}"))),
        })
    }

    #[cold]
    fn key_mismatch(&self, expected: &'static str) -> JsonError {
        let span = self.pending_key.as_ref().map(|k| k.span);
        DecodeError::at(Problem::mismatch(expected, "string (an object key)"), span)
    }
}

impl<S: Source> Reader for JsonReader<S> {
    type Location = Span;
    type Error = Malformed;
    type Checkpoint = JsonCheckpoint;

    fn peek_kind(&mut self) -> DecodeResult<Kind, Self> {
        if self.pending_key.is_some() {
            return Ok(Kind::String);
        }
        self.skip_ws()?;
        match self.peek()? {
            None => Err(self.syntax(self.offset(), Malformed::UnexpectedEnd)),
            Some(b) => classify(b)
                .ok_or_else(|| self.syntax(self.offset(), Malformed::UnexpectedCharacter(b))),
        }
    }

    #[inline]
    fn here(&mut self) -> Option<Span> {
        if !self.track {
            return None;
        }
        if let Some(key) = &self.pending_key {
            return Some(Span::point(key.span.start()));
        }
        let _ = self.skip_ws();
        Some(Span::point(self.offset()))
    }

    #[inline]
    fn since(&mut self, start: &Span) -> Span {
        Span::new(start.start(), self.offset())
    }

    fn byte_span(&self, location: &Span) -> Option<Span> {
        Some(*location)
    }

    fn key_location(&self) -> Option<Span> {
        self.last_key
    }

    fn set_track_locations(&mut self, on: bool) {
        self.track = on;
    }

    fn read_bool(&mut self) -> DecodeResult<bool, Self> {
        if self.pending_key.is_some() {
            return self.key_bool();
        }
        self.skip_ws()?;
        match self.peek()? {
            Some(b't') => self.keyword(b"true", "bool").map(|()| true),
            Some(b'f') => self.keyword(b"false", "bool").map(|()| false),
            _ => Err(self.mismatch("bool")),
        }
    }

    fn read_null(&mut self) -> DecodeResult<(), Self> {
        if self.pending_key.is_some() {
            return Err(self.key_mismatch("null"));
        }
        self.keyword(b"null", "null")
    }

    #[inline]
    fn read_str(&mut self, buf: &mut String) -> DecodeResult<(), Self> {
        if self.pending_key.is_some() {
            return self.key_str(buf);
        }
        self.skip_ws()?;
        match self.peek()? {
            Some(b'"') => {
                let start = self.offset();
                self.pos += 1;
                self.string_body(Some(buf), start)
            }
            _ => Err(self.mismatch("string")),
        }
    }

    #[inline]
    fn read_i64(&mut self) -> DecodeResult<i64, Self> {
        if self.pending_key.is_some() {
            return self.key_number(|t| girders::codec::i64().parse_text(t));
        }
        let t = self.int_tok()?;
        match self.int_magnitude(&t) {
            Some((false, m)) if m <= i64::MAX as u64 => Ok(m as i64),
            Some((true, m)) if m <= i64::MAX as u64 + 1 => Ok((m as i64).wrapping_neg()),
            _ => Err(self.out_of_range(&t, "i64")),
        }
    }

    #[inline]
    fn read_u64(&mut self) -> DecodeResult<u64, Self> {
        if self.pending_key.is_some() {
            return self.key_number(|t| girders::codec::u64().parse_text(t));
        }
        let t = self.int_tok()?;
        match self.int_magnitude(&t) {
            Some((false, m)) | Some((true, m @ 0)) => Ok(m),
            _ => Err(self.out_of_range(&t, "u64")),
        }
    }

    fn read_f64(&mut self) -> DecodeResult<f64, Self> {
        if self.pending_key.is_some() {
            // Finite numbers only, as serde_json: no `NaN` or `Infinity` keys.
            return self.key_number(|t| match girders::codec::number_grammar(t) {
                Some(_) => girders::codec::f64().parse_text(t),
                None => Err(Problem::mismatch("number", alloc::format!("{t:?}"))),
            });
        }
        let t = self.number("float")?;
        if !t.is_float
            && t.end - t.start <= 16
            && let Some((neg, m)) = self.int_magnitude(&t)
            && m < (1u64 << 53)
        {
            let f = m as f64;
            return Ok(if neg { -f } else { f });
        }
        match self.tok_text(&t).parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(f),
            _ => Err(self.out_of_range(&t, "f64")),
        }
    }

    fn read_number(&mut self) -> DecodeResult<Number, Self> {
        if self.pending_key.is_some() {
            return self.key_number(key_as_number);
        }
        let t = self.number("number")?;
        if t.is_float {
            return match self.tok_text(&t).parse::<f64>() {
                Ok(f) if f.is_finite() => Ok(Number::F64(f)),
                _ => Err(self.out_of_range(&t, "f64")),
            };
        }
        Ok(match self.int_magnitude(&t) {
            Some((false, m)) if m <= i64::MAX as u64 => Number::I64(m as i64),
            Some((false, m)) => Number::U64(m),
            Some((true, m)) if m <= i64::MAX as u64 + 1 => Number::I64((m as i64).wrapping_neg()),
            _ => Number::BigInt(self.tok_text(&t).into()),
        })
    }

    fn read_number_text(&mut self, buf: &mut String) -> DecodeResult<(), Self> {
        if self.pending_key.is_some() {
            let text = self.key_number(|t| match girders::codec::number_grammar(t) {
                Some(_) => Ok(String::from(t)),
                None => Err(Problem::mismatch("number text", alloc::format!("{t:?}"))),
            })?;
            buf.push_str(&text);
            return Ok(());
        }
        let t = self.number("number")?;
        buf.push_str(self.tok_text(&t));
        Ok(())
    }

    #[inline]
    fn begin_array(&mut self) -> DecodeResult<(), Self> {
        if self.pending_key.is_some() {
            return Err(self.key_mismatch("array"));
        }
        self.expect_byte(b'[', "array")
    }

    #[inline]
    fn array_has_next(&mut self, first: bool) -> DecodeResult<bool, Self> {
        self.skip_ws()?;
        match self.peek()? {
            Some(b']') => Ok(false),
            None => Err(self.syntax(self.offset(), Malformed::UnterminatedArray)),
            Some(_) if first => Ok(true),
            Some(b',') => {
                self.pos += 1;
                Ok(true)
            }
            Some(_) => Err(self.syntax(self.offset(), Malformed::ExpectedCommaOrBracket)),
        }
    }

    #[inline]
    fn end_array(&mut self) -> DecodeResult<(), Self> {
        self.expect_byte(b']', "`]`")
    }

    #[inline]
    fn begin_object(&mut self) -> DecodeResult<(), Self> {
        if self.pending_key.is_some() {
            return Err(self.key_mismatch("object"));
        }
        self.expect_byte(b'{', "object")
    }

    #[inline]
    fn object_next_key(&mut self, first: bool, buf: &mut String) -> DecodeResult<bool, Self> {
        self.skip_ws()?;
        match self.peek()? {
            Some(b'}') => Ok(false),
            None => Err(self.syntax(self.offset(), Malformed::UnterminatedObject)),
            Some(_) if first => self.key_after_separator(buf).map(|()| true),
            Some(b',') => {
                self.pos += 1;
                self.key_after_separator(buf).map(|()| true)
            }
            Some(_) => Err(self.syntax(self.offset(), Malformed::ExpectedCommaOrBrace)),
        }
    }

    fn object_next_entry(&mut self, first: bool) -> DecodeResult<bool, Self> {
        let mut text = String::new();
        let start = self.here_offset()?;
        if !self.object_next_key(first, &mut text)? {
            return Ok(false);
        }
        let span = self.last_key.unwrap_or(Span::point(start));
        self.pending_key = Some(PendingKey { text, span });
        Ok(true)
    }

    #[inline]
    fn end_object(&mut self) -> DecodeResult<(), Self> {
        self.expect_byte(b'}', "`}`")
    }

    /// Iterative, so arbitrarily deep input cannot overflow the stack.
    fn skip_value(&mut self) -> DecodeResult<(), Self> {
        if self.pending_key.take().is_some() {
            return Ok(());
        }
        // `true` = object, `false` = array.
        let mut stack: Vec<bool> = Vec::new();
        'value: loop {
            self.skip_ws()?;
            match self.peek()? {
                Some(b'{') => {
                    self.pos += 1;
                    self.skip_ws()?;
                    if self.peek()? == Some(b'}') {
                        self.pos += 1;
                    } else {
                        stack.push(true);
                        self.skip_key()?;
                        continue 'value;
                    }
                }
                Some(b'[') => {
                    self.pos += 1;
                    self.skip_ws()?;
                    if self.peek()? == Some(b']') {
                        self.pos += 1;
                    } else {
                        stack.push(false);
                        continue 'value;
                    }
                }
                Some(b'"') => {
                    let start = self.offset();
                    self.pos += 1;
                    self.string_body(None, start)?;
                }
                Some(b'-' | b'0'..=b'9') => {
                    self.lex_number()?;
                }
                Some(b't') => self.keyword(b"true", "value")?,
                Some(b'f') => self.keyword(b"false", "value")?,
                Some(b'n') => self.keyword(b"null", "value")?,
                Some(b) => {
                    return Err(self.syntax(self.offset(), Malformed::UnexpectedCharacter(b)));
                }
                None => return Err(self.syntax(self.offset(), Malformed::UnexpectedEnd)),
            }
            // After a complete value: close containers or move to the next
            // element.
            loop {
                let Some(&is_object) = stack.last() else {
                    return Ok(());
                };
                self.skip_ws()?;
                match (is_object, self.peek()?) {
                    (_, Some(b',')) => {
                        self.pos += 1;
                        if is_object {
                            self.skip_key()?;
                        }
                        continue 'value;
                    }
                    (false, Some(b']')) | (true, Some(b'}')) => {
                        self.pos += 1;
                        stack.pop();
                    }
                    (false, None) => {
                        return Err(self.syntax(self.offset(), Malformed::UnterminatedArray));
                    }
                    (true, None) => {
                        return Err(self.syntax(self.offset(), Malformed::UnterminatedObject));
                    }
                    (false, _) => {
                        return Err(self.syntax(self.offset(), Malformed::ExpectedCommaOrBracket));
                    }
                    (true, _) => {
                        return Err(self.syntax(self.offset(), Malformed::ExpectedCommaOrBrace));
                    }
                }
            }
        }
    }

    fn checkpoint(&mut self) -> JsonCheckpoint {
        let at = self.offset();
        self.cps.push(at);
        JsonCheckpoint {
            at,
            pending_key: self.pending_key.clone(),
        }
    }

    fn rewind(&mut self, cp: &JsonCheckpoint) {
        self.pos = (cp.at - self.base) as usize;
        self.pending_key.clone_from(&cp.pending_key);
    }

    fn release(&mut self, _cp: JsonCheckpoint) {
        self.cps.pop();
    }
}
