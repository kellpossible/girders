//! Byte sources for readers of byte formats (JSON, CBOR).
//!
//! A source exposes a buffer of available bytes. The reader indexes into
//! it; `fill` appends more (never moving existing indices), and `discard`
//! drops a consumed prefix when the reader knows nothing earlier is
//! needed (no checkpoint pins it).

use alloc::string::String;
use alloc::vec::Vec;

pub trait Source {
    /// Bytes currently available.
    fn buf(&self) -> &[u8];
    /// `true` when `buf` holds all remaining input.
    fn is_eof(&self) -> bool;
    /// Try to append more bytes. `Ok(true)` if any were added, `Ok(false)`
    /// if none are available right now (or ever, at EOF).
    fn fill(&mut self) -> Result<bool, String>;
    /// Drop the first `n` bytes.
    fn discard(&mut self, n: usize);
    /// Whether discarding is worth doing (growing buffers only).
    const COMPACTS: bool;
    /// `buf()[start..end]` as text, if the source knows it is valid UTF-8
    /// (input given as `&str`), so the reader can skip validating it.
    #[inline(always)]
    fn text(&self, _start: usize, _end: usize) -> Option<&str> {
        None
    }
}

/// A complete in-memory document.
#[derive(Debug, Clone, Copy)]
pub struct SliceSource<'a> {
    bytes: &'a [u8],
    /// The same input as `&str`, when it came as text.
    text: Option<&'a str>,
}

impl<'a> SliceSource<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        SliceSource { bytes, text: None }
    }

    /// Text input: already-valid UTF-8, so strings needn't be re-validated.
    pub fn from_text(text: &'a str) -> Self {
        SliceSource {
            bytes: text.as_bytes(),
            text: Some(text),
        }
    }
}

impl Source for SliceSource<'_> {
    #[inline(always)]
    fn buf(&self) -> &[u8] {
        self.bytes
    }
    #[inline(always)]
    fn is_eof(&self) -> bool {
        true
    }
    #[inline(always)]
    fn fill(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn discard(&mut self, n: usize) {
        self.bytes = &self.bytes[n..];
        self.text = self.text.map(|t| &t[n..]);
    }
    const COMPACTS: bool = false;
    #[inline(always)]
    fn text(&self, start: usize, end: usize) -> Option<&str> {
        // Slicing checks only that both ends are char boundaries.
        self.text.and_then(|t| t.get(start..end))
    }
}

/// Bytes pushed in by the caller (sans-IO streaming). `fill` never adds
/// anything: when the buffer runs dry before `finish`, the reader reports
/// `Incomplete` and the driver waits for the next `push`.
#[derive(Debug, Default, Clone)]
pub struct PushSource {
    bytes: Vec<u8>,
    eof: bool,
}

impl PushSource {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }
    pub fn finish(&mut self) {
        self.eof = true;
    }
}

impl Source for PushSource {
    fn buf(&self) -> &[u8] {
        &self.bytes
    }
    fn is_eof(&self) -> bool {
        self.eof
    }
    fn fill(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn discard(&mut self, n: usize) {
        self.bytes.drain(..n);
    }
    const COMPACTS: bool = true;
}

/// Any `std::io::Read`, buffered internally (no `BufReader` needed).
#[cfg(feature = "std")]
pub struct ReadSource<R> {
    inner: R,
    bytes: Vec<u8>,
    eof: bool,
    chunk: usize,
}

#[cfg(feature = "std")]
impl<R: std::io::Read> ReadSource<R> {
    pub fn new(inner: R) -> Self {
        Self::with_chunk_size(inner, 64 * 1024)
    }
    pub fn with_chunk_size(inner: R, chunk: usize) -> Self {
        ReadSource {
            inner,
            bytes: Vec::new(),
            eof: false,
            chunk: chunk.max(1),
        }
    }
}

#[cfg(feature = "std")]
impl<R: std::io::Read> Source for ReadSource<R> {
    fn buf(&self) -> &[u8] {
        &self.bytes
    }
    fn is_eof(&self) -> bool {
        self.eof
    }
    fn fill(&mut self) -> Result<bool, String> {
        if self.eof {
            return Ok(false);
        }
        let len = self.bytes.len();
        self.bytes.resize(len + self.chunk, 0);
        loop {
            match self.inner.read(&mut self.bytes[len..]) {
                Ok(0) => {
                    self.bytes.truncate(len);
                    self.eof = true;
                    return Ok(false);
                }
                Ok(n) => {
                    self.bytes.truncate(len + n);
                    return Ok(true);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.bytes.truncate(len);
                    return Err(alloc::format!("{e}"));
                }
            }
        }
    }
    fn discard(&mut self, n: usize) {
        self.bytes.drain(..n);
    }
    const COMPACTS: bool = true;
}
