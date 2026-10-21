//! JSON Pointer (RFC 6901).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use girders::PathSegment;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pointer {
    segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PointerError {
    MissingLeadingSlash,
    /// A `~` not followed by `0` or `1`, at this byte offset.
    InvalidEscape(usize),
}

impl Pointer {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn push(&mut self, segment: impl Into<String>) {
        self.segments.push(segment.into());
    }

    pub fn push_index(&mut self, index: usize) {
        self.segments.push(index.to_string());
    }

    pub fn from_path<'a>(path: impl IntoIterator<Item = &'a PathSegment>) -> Self {
        let mut p = Self::root();
        for seg in path {
            match seg {
                PathSegment::Key(k) => p.push(k.clone()),
                PathSegment::Index(i) => p.push_index(*i),
                // Kinds of segment a future version may add: their text.
                other => p.push(alloc::format!("{other}")),
            }
        }
        p
    }

    pub fn parent(&self) -> Option<Pointer> {
        let mut p = self.clone();
        p.segments.pop()?;
        Some(p)
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().map(String::as_str)
    }

    pub fn parse(input: &str) -> Result<Pointer, PointerError> {
        if input.is_empty() {
            return Ok(Self::root());
        }
        let Some(rest) = input.strip_prefix('/') else {
            return Err(PointerError::MissingLeadingSlash);
        };
        let mut segments = Vec::new();
        let mut offset = 1;
        for raw in rest.split('/') {
            let mut seg = String::with_capacity(raw.len());
            let mut chars = raw.char_indices();
            while let Some((i, c)) = chars.next() {
                if c == '~' {
                    match chars.next() {
                        Some((_, '0')) => seg.push('~'),
                        Some((_, '1')) => seg.push('/'),
                        _ => return Err(PointerError::InvalidEscape(offset + i)),
                    }
                } else {
                    seg.push(c);
                }
            }
            offset += raw.len() + 1;
            segments.push(seg);
        }
        Ok(Pointer { segments })
    }

    /// `#/a/b`, percent-encoding what a URI fragment can't hold.
    pub fn to_uri_fragment(&self) -> String {
        let mut out = String::from("#");
        for b in self.to_string().bytes() {
            if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&alloc::format!("%{b:02X}"));
            }
        }
        out
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for seg in &self.segments {
            f.write_str("/")?;
            for c in seg.chars() {
                match c {
                    '~' => f.write_str("~0")?,
                    '/' => f.write_str("~1")?,
                    c => write!(f, "{c}")?,
                }
            }
        }
        Ok(())
    }
}
