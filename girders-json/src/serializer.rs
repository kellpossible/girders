//! JSON writer, compact or pretty, over any [`Write`] sink.

use alloc::string::String;
use alloc::vec::Vec;
use core::convert::Infallible;
use core::fmt;
use girders::Serializer;

/// A byte sink. Implemented for `Vec<u8>`, and for any `std::io::Write`
/// through [`IoWrite`].
pub trait Write {
    type Error;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
}

impl Write for Vec<u8> {
    type Error = Infallible;
    #[inline]
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Infallible> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

impl<W: Write + ?Sized> Write for &mut W {
    type Error = W::Error;
    #[inline]
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), W::Error> {
        (**self).write_all(bytes)
    }
}

/// Adapts a `std::io::Write`. Wrap it in a `BufWriter` for unbuffered sinks.
#[cfg(feature = "std")]
pub struct IoWrite<W>(pub W);

#[cfg(feature = "std")]
impl<W: std::io::Write> Write for IoWrite<W> {
    type Error = std::io::Error;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), std::io::Error> {
        self.0.write_all(bytes)
    }
}

#[derive(Debug)]
pub enum EncodeError<E> {
    /// JSON has no NaN or infinities.
    NonFinite(f64),
    /// A map key that JSON can't represent: only strings and integers
    /// (written as their text) can be keys.
    UnsupportedKey,
    Write(E),
}

impl<E: fmt::Debug> fmt::Display for EncodeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::NonFinite(x) => write!(f, "cannot encode non-finite number {x} as JSON"),
            EncodeError::UnsupportedKey => f.write_str("JSON object keys must be strings or integers"),
            EncodeError::Write(e) => write!(f, "write failed: {e:?}"),
        }
    }
}

impl<E: fmt::Debug> core::error::Error for EncodeError<E> {}

/// Output style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Style {
    Compact,
    /// Indent nested structure by this string per level.
    Pretty(String),
}

pub struct JsonSerializer<W> {
    out: W,
    indent: Option<String>,
    depth: usize,
    /// True while the current container has had no element yet.
    fresh: bool,
    /// The next value is an object key (see `object_key_value`).
    key_next: bool,
}

impl<W: Write> JsonSerializer<W> {
    pub fn new(out: W, style: Style) -> Self {
        let indent = match style {
            Style::Compact => None,
            Style::Pretty(unit) => Some(unit),
        };
        JsonSerializer {
            out,
            indent,
            depth: 0,
            fresh: false,
            key_next: false,
        }
    }

    pub fn compact(out: W) -> Self {
        Self::new(out, Style::Compact)
    }

    pub fn into_inner(self) -> W {
        self.out
    }

    #[inline]
    fn raw(&mut self, bytes: &[u8]) -> Result<(), EncodeError<W::Error>> {
        self.out.write_all(bytes).map_err(EncodeError::Write)
    }

    #[inline]
    fn newline(&mut self) -> Result<(), EncodeError<W::Error>> {
        if let Some(unit) = &self.indent {
            self.out.write_all(b"\n").map_err(EncodeError::Write)?;
            for _ in 0..self.depth {
                self.out
                    .write_all(unit.as_bytes())
                    .map_err(EncodeError::Write)?;
            }
        }
        Ok(())
    }

    #[inline]
    fn separator(&mut self) -> Result<(), EncodeError<W::Error>> {
        if !self.fresh {
            self.raw(b",")?;
        }
        self.fresh = false;
        self.newline()
    }

    fn close(&mut self, delim: &[u8]) -> Result<(), EncodeError<W::Error>> {
        let empty = self.fresh;
        self.depth -= 1;
        if !empty {
            self.newline()?;
        }
        self.fresh = false;
        self.raw(delim)
    }

    /// Fail if a key is expected: only strings and integers can be keys.
    #[inline]
    fn not_a_key(&self) -> Result<(), EncodeError<W::Error>> {
        if self.key_next {
            Err(EncodeError::UnsupportedKey)
        } else {
            Ok(())
        }
    }

    /// After a string key written as a value: the `:`.
    #[cold]
    fn finish_key(&mut self) -> Result<(), EncodeError<W::Error>> {
        self.key_next = false;
        self.raw(if self.indent.is_some() { b": " } else { b":" })
    }

    /// An integer key written as a value: quoted, since JSON keys are
    /// strings.
    #[cold]
    fn integer_key(&mut self, negative: bool, magnitude: u64) -> Result<(), EncodeError<W::Error>> {
        self.raw(if negative { b"\"-" } else { b"\"" })?;
        let mut buf = [0u8; 20];
        let digits = format_u64(magnitude, &mut buf);
        self.out.write_all(digits).map_err(EncodeError::Write)?;
        self.raw(b"\"")?;
        self.finish_key()
    }

    fn write_string(&mut self, s: &str) -> Result<(), EncodeError<W::Error>> {
        self.raw(b"\"")?;
        let bytes = s.as_bytes();
        let mut run = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let esc: &[u8] = match b {
                b'"' => b"\\\"",
                b'\\' => b"\\\\",
                b'\n' => b"\\n",
                b'\r' => b"\\r",
                b'\t' => b"\\t",
                0x08 => b"\\b",
                0x0c => b"\\f",
                0..=0x1f => b"",
                _ => continue,
            };
            self.raw(&bytes[run..i])?;
            if esc.is_empty() {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                self.raw(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[(b >> 4) as usize],
                    HEX[(b & 15) as usize],
                ])?;
            } else {
                self.raw(esc)?;
            }
            run = i + 1;
        }
        self.raw(&bytes[run..])?;
        self.raw(b"\"")
    }
}

/// Formats into a fixed stack buffer (numbers are short).
struct StackBuffer {
    buf: [u8; 40],
    len: usize,
}

impl fmt::Write for StackBuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        if end > self.buf.len() {
            return Err(fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn format_u64(mut n: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    &buf[i..]
}

impl<W: Write> Serializer for JsonSerializer<W> {
    type Error = EncodeError<W::Error>;

    fn write_bool(&mut self, v: bool) -> Result<(), Self::Error> {
        self.not_a_key()?;
        self.raw(if v { b"true" } else { b"false" })
    }

    fn write_null(&mut self) -> Result<(), Self::Error> {
        self.not_a_key()?;
        self.raw(b"null")
    }

    #[inline]
    fn write_i64(&mut self, v: i64) -> Result<(), Self::Error> {
        if self.key_next {
            return self.integer_key(v < 0, v.unsigned_abs());
        }
        if v < 0 {
            self.raw(b"-")?;
        }
        let mut buf = [0u8; 20];
        let digits = format_u64(v.unsigned_abs(), &mut buf);
        self.out.write_all(digits).map_err(EncodeError::Write)
    }

    #[inline]
    fn write_u64(&mut self, v: u64) -> Result<(), Self::Error> {
        if self.key_next {
            return self.integer_key(false, v);
        }
        let mut buf = [0u8; 20];
        let digits = format_u64(v, &mut buf);
        self.out.write_all(digits).map_err(EncodeError::Write)
    }

    fn write_f64(&mut self, v: f64) -> Result<(), Self::Error> {
        self.not_a_key()?;
        if !v.is_finite() {
            return Err(EncodeError::NonFinite(v));
        }
        // `Debug` is shortest round-trip, keeps `.0` on whole numbers and
        // uses exponents for very large/small magnitudes: all valid JSON.
        let mut b = StackBuffer {
            buf: [0; 40],
            len: 0,
        };
        let _ = fmt::write(&mut b, format_args!("{v:?}"));
        self.raw(&b.buf[..b.len])
    }

    fn write_f32(&mut self, v: f32) -> Result<(), Self::Error> {
        self.not_a_key()?;
        if !v.is_finite() {
            return Err(EncodeError::NonFinite(v as f64));
        }
        let mut b = StackBuffer {
            buf: [0; 40],
            len: 0,
        };
        let _ = fmt::write(&mut b, format_args!("{v:?}"));
        self.raw(&b.buf[..b.len])
    }

    fn write_number_text(&mut self, v: &str) -> Result<(), Self::Error> {
        self.not_a_key()?;
        self.raw(v.as_bytes())
    }

    #[inline]
    fn write_str(&mut self, v: &str) -> Result<(), Self::Error> {
        self.write_string(v)?;
        if self.key_next {
            self.finish_key()?;
        }
        Ok(())
    }

    fn begin_array(&mut self, _len: Option<usize>) -> Result<(), Self::Error> {
        self.not_a_key()?;
        self.depth += 1;
        self.fresh = true;
        self.raw(b"[")
    }

    #[inline]
    fn array_element(&mut self) -> Result<(), Self::Error> {
        self.separator()
    }

    fn end_array(&mut self) -> Result<(), Self::Error> {
        self.close(b"]")
    }

    fn begin_object(&mut self, _len: Option<usize>) -> Result<(), Self::Error> {
        self.not_a_key()?;
        self.depth += 1;
        self.fresh = true;
        self.raw(b"{")
    }

    #[inline]
    fn object_key(&mut self, key: &str) -> Result<(), Self::Error> {
        self.separator()?;
        self.write_string(key)?;
        self.raw(if self.indent.is_some() { b": " } else { b":" })
    }

    fn object_key_value(&mut self) -> Result<(), Self::Error> {
        self.separator()?;
        self.key_next = true;
        Ok(())
    }

    fn end_object(&mut self) -> Result<(), Self::Error> {
        self.close(b"}")
    }
}
