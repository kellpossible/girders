//! The decode error model: logical path, location, problem and context
//! frames, with room for several errors when accumulating.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    TypeMismatch {
        expected: Cow<'static, str>,
        found: Cow<'static, str>,
    },
    MissingField(Cow<'static, str>),
    UnknownField {
        name: String,
        expected: Vec<&'static str>,
    },
    DuplicateField(String),
    UnknownVariant {
        name: String,
        expected: Vec<&'static str>,
    },
    UnknownVersion {
        found: String,
        known: Vec<String>,
    },
    ValidationFailed(String),
    OutOfRange(String),
    /// Malformed input for the format. Readers produce these; combinators
    /// never do.
    Syntax(String),
    DepthLimitExceeded(usize),
    /// The input source failed (an I/O error while streaming).
    Io(String),
    /// The available input ran out before the value was complete, and
    /// more may still arrive. Streaming drivers catch this and retry.
    /// Combinators must pass it through unchanged.
    Incomplete,
}

impl Problem {
    pub fn mismatch(
        expected: impl Into<Cow<'static, str>>,
        found: impl Into<Cow<'static, str>>,
    ) -> Self {
        Problem::TypeMismatch {
            expected: expected.into(),
            found: found.into(),
        }
    }

    /// Errors after which nothing in the input can be trusted: decoding
    /// stops even when accumulating, and `one_of` does not try another
    /// branch.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Problem::Syntax(_)
                | Problem::Incomplete
                | Problem::DepthLimitExceeded(_)
                | Problem::Io(_)
        )
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::TypeMismatch { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            Problem::MissingField(name) => write!(f, "missing required field `{name}`"),
            Problem::UnknownField { name, .. } => write!(f, "unknown field `{name}`"),
            Problem::DuplicateField(name) => write!(f, "duplicate field `{name}`"),
            Problem::UnknownVariant { name, expected } => {
                write!(f, "unknown variant `{name}`, expected one of: ")?;
                join(f, expected.iter())
            }
            Problem::UnknownVersion { found, known } => {
                write!(f, "unknown version `{found}`, known versions: ")?;
                join(f, known.iter())
            }
            Problem::ValidationFailed(m) | Problem::OutOfRange(m) | Problem::Syntax(m) => {
                f.write_str(m)
            }
            Problem::Io(m) => write!(f, "input error: {m}"),
            Problem::DepthLimitExceeded(max) => write!(f, "nesting deeper than the limit of {max}"),
            Problem::Incomplete => f.write_str("unexpected end of available input"),
        }
    }
}

fn join<T: fmt::Display>(
    f: &mut fmt::Formatter<'_>,
    items: impl Iterator<Item = T>,
) -> fmt::Result {
    for (i, item) in items.enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

/// An enclosing structure the failure happened inside, for "while
/// decoding the server entry" style context.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextFrame<L> {
    pub location: Option<L>,
    pub description: Cow<'static, str>,
}

/// One failure.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorDetail<L> {
    /// Innermost-first while propagating; use [`ErrorDetail::path`].
    path_rev: Vec<PathSegment>,
    pub location: Option<L>,
    pub problem: Problem,
    /// Enclosing structures, innermost-first.
    pub context: Vec<ContextFrame<L>>,
}

impl<L> ErrorDetail<L> {
    /// Logical path from the document root, outermost-first.
    pub fn path(&self) -> impl DoubleEndedIterator<Item = &PathSegment> + ExactSizeIterator {
        self.path_rev.iter().rev()
    }

    pub fn path_vec(&self) -> Vec<PathSegment> {
        self.path().cloned().collect()
    }

    pub fn path_len(&self) -> usize {
        self.path_rev.len()
    }

    /// `users[0].age`; empty for the document root.
    pub fn path_string(&self) -> String {
        let mut out = String::new();
        for seg in self.path() {
            match seg {
                PathSegment::Key(k) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(k);
                }
                PathSegment::Index(i) => {
                    use core::fmt::Write;
                    let _ = write!(out, "[{i}]");
                }
            }
        }
        out
    }
}

/// A decode failure: one or more [`ErrorDetail`]s (several only in
/// accumulate mode). Boxed so `Result<T, DecodeError<L>>` stays small.
#[derive(Clone, PartialEq)]
pub struct DecodeError<L> {
    inner: Box<(ErrorDetail<L>, Vec<ErrorDetail<L>>)>,
}

impl<L> DecodeError<L> {
    pub fn new(problem: Problem) -> Self {
        DecodeError {
            inner: Box::new((
                ErrorDetail {
                    path_rev: Vec::new(),
                    location: None,
                    problem,
                    context: Vec::new(),
                },
                Vec::new(),
            )),
        }
    }

    pub fn at(problem: Problem, location: Option<L>) -> Self {
        let mut e = Self::new(problem);
        e.inner.0.location = location;
        e
    }

    /// The first (or only) failure.
    pub fn first(&self) -> &ErrorDetail<L> {
        &self.inner.0
    }

    pub fn problem(&self) -> &Problem {
        &self.inner.0.problem
    }

    pub fn location(&self) -> Option<&L> {
        self.inner.0.location.as_ref()
    }

    pub fn path(&self) -> Vec<PathSegment> {
        self.inner.0.path_vec()
    }

    pub fn len(&self) -> usize {
        1 + self.inner.1.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn iter(&self) -> impl Iterator<Item = &ErrorDetail<L>> {
        core::iter::once(&self.inner.0).chain(self.inner.1.iter())
    }

    pub fn is_fatal(&self) -> bool {
        self.iter().any(|d| d.problem.is_fatal())
    }

    pub fn is_incomplete(&self) -> bool {
        self.inner.0.problem == Problem::Incomplete
    }

    /// Prefix every detail's path with `segment` as the error propagates
    /// out of a containing structure.
    pub fn push_path(mut self, segment: PathSegment) -> Self {
        if self.inner.1.is_empty() {
            self.inner.0.path_rev.push(segment);
        } else {
            for d in core::iter::once(&mut self.inner.0).chain(self.inner.1.iter_mut()) {
                d.path_rev.push(segment.clone());
            }
        }
        self
    }

    /// Attach a location if the error has none yet.
    pub fn or_at(mut self, location: impl FnOnce() -> Option<L>) -> Self {
        if self.inner.0.location.is_none() {
            self.inner.0.location = location();
        }
        self
    }

    pub fn push_context(mut self, frame: ContextFrame<L>) -> Self
    where
        L: Clone,
    {
        for d in core::iter::once(&mut self.inner.0).chain(self.inner.1.iter_mut()) {
            d.context.push(frame.clone());
        }
        self
    }

    /// Merge `other`'s details after this error's (accumulate mode).
    pub fn merge(mut self, other: DecodeError<L>) -> Self {
        let (first, rest) = *other.inner;
        self.inner.1.push(first);
        self.inner.1.extend(rest);
        self
    }

    pub fn map_location<M>(self, f: impl Fn(L) -> M) -> DecodeError<M> {
        let map = |d: ErrorDetail<L>| ErrorDetail {
            path_rev: d.path_rev,
            location: d.location.map(&f),
            problem: d.problem,
            context: d
                .context
                .into_iter()
                .map(|c| ContextFrame {
                    location: c.location.map(&f),
                    description: c.description,
                })
                .collect(),
        };
        let (first, rest) = *self.inner;
        DecodeError {
            inner: Box::new((map(first), rest.into_iter().map(map).collect())),
        }
    }
}

/// Accumulates errors across siblings (record fields, list elements).
/// Used by `Fields` implementations.
pub struct Collector<L> {
    acc: Option<DecodeError<L>>,
}

impl<L> Default for Collector<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L> Collector<L> {
    pub fn new() -> Self {
        Collector { acc: None }
    }
    pub fn push(&mut self, e: DecodeError<L>) {
        self.acc = Some(match self.acc.take() {
            None => e,
            Some(a) => a.merge(e),
        });
    }
    pub fn has_errors(&self) -> bool {
        self.acc.is_some()
    }
    pub fn finish(self) -> Result<(), DecodeError<L>> {
        match self.acc {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}

impl<L: fmt::Debug> fmt::Debug for DecodeError<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inner.1.is_empty() {
            self.inner.0.fmt(f)
        } else {
            f.debug_list().entries(self.iter()).finish()
        }
    }
}

/// `users[0].age: expected int, found string (at byte 71)`, one line
/// per detail.
impl<L: fmt::Display> fmt::Display for DecodeError<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            let path = d.path_string();
            if !path.is_empty() {
                write!(f, "{path}: ")?;
            }
            write!(f, "{}", d.problem)?;
            if let Some(loc) = &d.location {
                write!(f, " (at {loc})")?;
            }
        }
        Ok(())
    }
}

impl<L: fmt::Debug + fmt::Display> core::error::Error for DecodeError<L> {}
