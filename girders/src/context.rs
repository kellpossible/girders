//! Per-call state threaded through combinators.
//!
//! Rule: combinators never wrap the reader or serializer to pass state
//! down. Changing `R`/`S` per level makes recursive codecs instantiate
//! `Wrap<Wrap<…>>` forever. State that must reach an inner codec travels
//! here instead.

/// Runtime switches for the cost of diagnostics. Locations and paths are
/// only built when something fails, so on the success path they cost
/// almost nothing; turning them off mainly shrinks errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Attach locations to errors and record spans (`spanned`, `Value`).
    pub track_locations: bool,
    /// Build logical paths (`users[0].age`) as errors propagate.
    pub track_path: bool,
    /// Keep going after non-fatal errors and report all of them.
    pub accumulate: bool,
    /// Maximum nesting of arrays/objects; `None` is unlimited (only for
    /// trusted input: recursion uses the native stack).
    pub max_depth: Option<usize>,
    /// Record where each object key appears, in dynamic value trees that
    /// support it. Off by default: it costs memory per key. Needs
    /// `track_locations`.
    pub key_spans: bool,
}

impl DecodeOptions {
    /// Locations and paths on, fail fast, depth 128.
    pub const fn new() -> Self {
        DecodeOptions {
            track_locations: true,
            track_path: true,
            accumulate: false,
            max_depth: Some(128),
            key_spans: false,
        }
    }

    /// Everything on, accumulate all errors.
    pub const fn strict() -> Self {
        DecodeOptions {
            accumulate: true,
            ..Self::new()
        }
    }

    /// Everything off except the depth limit.
    pub const fn fast() -> Self {
        DecodeOptions {
            track_locations: false,
            track_path: false,
            accumulate: false,
            max_depth: Some(128),
            key_spans: false,
        }
    }

    pub const fn accumulate(self, on: bool) -> Self {
        DecodeOptions {
            accumulate: on,
            ..self
        }
    }

    pub const fn locations(self, on: bool) -> Self {
        DecodeOptions {
            track_locations: on,
            ..self
        }
    }

    pub const fn paths(self, on: bool) -> Self {
        DecodeOptions {
            track_path: on,
            ..self
        }
    }

    pub const fn key_spans(self, on: bool) -> Self {
        DecodeOptions {
            key_spans: on,
            ..self
        }
    }

    pub const fn max_depth(self, max: Option<usize>) -> Self {
        DecodeOptions {
            max_depth: max,
            ..self
        }
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode-side context.
#[derive(Debug, Default)]
pub struct DecodeContext {
    pub options: DecodeOptions,
    /// Current container nesting.
    pub depth: usize,
    /// A key the enclosing tag dispatcher already consumed. The next
    /// record decoder takes it and skips that key instead of rejecting
    /// it; every other decoder leaves it alone, and dispatchers clear it
    /// after the variant returns so it never leaks deeper.
    /// Scoped to the depth it was claimed at, so a list of records inside
    /// a variant doesn't take it.
    pub(crate) claimed: Option<(&'static str, usize)>,
    /// Reusable key buffers, so objects don't allocate a key `String` each.
    key_bufs: alloc::vec::Vec<alloc::string::String>,
    /// Reusable field-name tables for compact record decoding.
    name_bufs: alloc::vec::Vec<alloc::vec::Vec<&'static str>>,
}

impl DecodeContext {
    pub fn new(options: DecodeOptions) -> Self {
        DecodeContext {
            options,
            depth: 0,
            claimed: None,
            key_bufs: alloc::vec::Vec::new(),
            name_bufs: alloc::vec::Vec::new(),
        }
    }

    /// Run `f` one container level deeper, enforcing `max_depth`.
    #[inline]
    pub fn nested<L, T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, crate::DecodeError<L>>,
    ) -> Result<T, crate::DecodeError<L>> {
        if let Some(max) = self.options.max_depth
            && self.depth >= max
        {
            return Err(crate::DecodeError::new(crate::Problem::DepthLimitExceeded(
                max,
            )));
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    /// A cleared key buffer; hand it back with [`DecodeContext::put_key_buf`].
    #[inline]
    pub fn take_key_buf(&mut self) -> alloc::string::String {
        self.key_bufs.pop().unwrap_or_default()
    }

    #[inline]
    pub fn put_key_buf(&mut self, mut buf: alloc::string::String) {
        buf.clear();
        self.key_bufs.push(buf);
    }

    /// An empty name table; hand it back with [`DecodeContext::put_name_buf`].
    #[inline]
    pub fn take_name_buf(&mut self) -> alloc::vec::Vec<&'static str> {
        self.name_bufs.pop().unwrap_or_default()
    }

    #[inline]
    pub fn put_name_buf(&mut self, mut buf: alloc::vec::Vec<&'static str>) {
        buf.clear();
        self.name_bufs.push(buf);
    }

    /// Take the claimed key, if it was claimed for an object at this
    /// depth (records call this as they open the object).
    pub fn take_claimed(&mut self) -> Option<&'static str> {
        match self.claimed {
            Some((key, depth)) if depth == self.depth => {
                self.claimed = None;
                Some(key)
            }
            _ => None,
        }
    }

    /// Run `f` with `key` claimed for the object about to be decoded at
    /// the current depth. The claim never outlives `f`.
    pub fn with_claim<T>(&mut self, key: &'static str, f: impl FnOnce(&mut Self) -> T) -> T {
        let prev = self.claimed.replace((key, self.depth));
        let out = f(self);
        self.claimed = prev;
        out
    }
}

/// A tag value written by unions and versioned codecs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TagValue {
    Str(&'static str),
    Int(i64),
}

/// Encode-side context.
#[derive(Debug, Default)]
pub struct EncodeContext {
    /// A `key: value` entry the next record writes before its own fields
    /// (internally-tagged unions, versioned codecs).
    pub(crate) pending_tag: Option<(&'static str, TagValue)>,
}

impl EncodeContext {
    pub fn new() -> Self {
        EncodeContext::default()
    }

    pub fn take_pending_tag(&mut self) -> Option<(&'static str, TagValue)> {
        self.pending_tag.take()
    }
}
