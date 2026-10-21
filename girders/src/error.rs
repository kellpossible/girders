//! The decode error model: logical path, location, problem and context
//! frames, with room for several errors when accumulating.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// A key as itself, an index as its number.
impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathSegment::Key(k) => f.write_str(k),
            PathSegment::Index(i) => write!(f, "{i}"),
        }
    }
}

/// What went wrong. `F` is the format's own error type (see
/// [`FormatError`]); application errors are [`Problem::Custom`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Problem<F> {
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
    OutOfRange(String),
    /// The format's own error: malformed input (JSON syntax, CBOR
    /// well-formedness) or anything else the format defines. Readers
    /// produce these; combinators never do.
    Format(F),
    /// An error from outside girders: a validator's (`refine`,
    /// `try_convert`), a custom codec's or a third-party format's. The
    /// original value is kept; get it back with
    /// [`ErrorDetail::custom`] or [`DecodeError::custom`].
    Custom(CustomError),
    DepthLimitExceeded(usize),
    /// The input source failed (an I/O error while streaming).
    Io(String),
    /// The format can't do what the codec asked, for example read a map
    /// key as a number from a format whose keys are always strings.
    Unsupported(Cow<'static, str>),
    /// The available input ran out before the value was complete, and
    /// more may still arrive. Streaming drivers catch this and retry.
    /// Combinators must pass it through unchanged.
    Incomplete,
}

impl<F> Problem<F> {
    pub fn mismatch(
        expected: impl Into<Cow<'static, str>>,
        found: impl Into<Cow<'static, str>>,
    ) -> Self {
        Problem::TypeMismatch {
            expected: expected.into(),
            found: found.into(),
        }
    }
}

impl<F: FormatError> Problem<F> {
    /// Errors after which nothing in the input can be trusted: decoding
    /// stops even when accumulating, and `one_of` does not try another
    /// branch.
    pub fn is_fatal(&self) -> bool {
        match self {
            Problem::Format(e) => e.is_fatal(),
            Problem::Custom(e) => e.is_fatal(),
            Problem::Incomplete | Problem::DepthLimitExceeded(_) | Problem::Io(_) => true,
            _ => false,
        }
    }
}

/// Custom errors compare by fatality and message, since boxed errors have
/// no equality of their own.
impl<F: PartialEq> PartialEq for Problem<F> {
    fn eq(&self, other: &Self) -> bool {
        use Problem::*;
        match (self, other) {
            (
                TypeMismatch {
                    expected: a,
                    found: b,
                },
                TypeMismatch {
                    expected: c,
                    found: d,
                },
            ) => a == c && b == d,
            (MissingField(a), MissingField(b)) => a == b,
            (
                UnknownField {
                    name: a,
                    expected: b,
                },
                UnknownField {
                    name: c,
                    expected: d,
                },
            ) => a == c && b == d,
            (DuplicateField(a), DuplicateField(b)) => a == b,
            (
                UnknownVariant {
                    name: a,
                    expected: b,
                },
                UnknownVariant {
                    name: c,
                    expected: d,
                },
            ) => a == c && b == d,
            (UnknownVersion { found: a, known: b }, UnknownVersion { found: c, known: d }) => {
                a == c && b == d
            }
            (OutOfRange(a), OutOfRange(b)) => a == b,
            (Format(a), Format(b)) => a == b,
            (Custom(a), Custom(b)) => a == b,
            (DepthLimitExceeded(a), DepthLimitExceeded(b)) => a == b,
            (Io(a), Io(b)) => a == b,
            (Unsupported(a), Unsupported(b)) => a == b,
            (Incomplete, Incomplete) => true,
            _ => false,
        }
    }
}

/// A format's own errors (JSON syntax, CBOR well-formedness, …), carried
/// in [`Problem::Format`]. Formats without any use `Infallible`.
pub trait FormatError: fmt::Debug + fmt::Display + Send + Sync + 'static {
    /// Whether nothing after this error can be trusted: decoding stops
    /// even when accumulating, and `one_of` doesn't try another option.
    /// Malformed input is fatal, which is the default.
    fn is_fatal(&self) -> bool {
        true
    }
}

impl FormatError for core::convert::Infallible {}

/// A boxed error from outside girders, as `Problem::Custom` carries it.
/// Your error types, `&str` and `String` all convert into it.
pub type BoxedError = Box<dyn core::error::Error + Send + Sync>;

/// An application's or third-party format's error, kept as its own type.
///
/// Anything that converts into a boxed error works: your error types,
/// and plain `&str` or `String` messages. Get the original back with
/// [`CustomError::downcast_ref`].
pub struct CustomError {
    error: BoxedError,
    fatal: bool,
}

impl CustomError {
    /// A recoverable error: decoding carries on when accumulating, and
    /// `one_of` tries its next option.
    pub fn new(error: impl Into<BoxedError>) -> Self {
        CustomError {
            error: error.into(),
            fatal: false,
        }
    }

    /// An error after which the input can't be trusted.
    pub fn fatal(error: impl Into<BoxedError>) -> Self {
        CustomError {
            error: error.into(),
            fatal: true,
        }
    }

    pub fn is_fatal(&self) -> bool {
        self.fatal
    }

    /// The original error, if it is a `T`.
    pub fn downcast_ref<T: core::error::Error + 'static>(&self) -> Option<&T> {
        self.error.downcast_ref::<T>()
    }

    pub fn get(&self) -> &(dyn core::error::Error + Send + Sync + 'static) {
        &*self.error
    }

    pub fn into_inner(self) -> BoxedError {
        self.error
    }
}

impl fmt::Debug for CustomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl fmt::Display for CustomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl PartialEq for CustomError {
    fn eq(&self, other: &Self) -> bool {
        use alloc::string::ToString;
        self.fatal == other.fatal && self.error.to_string() == other.error.to_string()
    }
}

/// A validator's error that points inside the value it checked, for
/// cross-field rules: `Invalid::at("end", RangeError::EndBeforeStart)`
/// reports `RangeError` under the value's `end`. Nest calls for deeper
/// paths: `Invalid::at("items", Invalid::at(3, error))`.
#[derive(Debug)]
pub struct Invalid {
    /// Outermost-first, relative to the validated value.
    path: Vec<PathSegment>,
    error: BoxedError,
}

impl Invalid {
    pub fn at(segment: impl Into<PathSegment>, error: impl Into<BoxedError>) -> Self {
        let mut path = alloc::vec![segment.into()];
        match error.into().downcast::<Invalid>() {
            Ok(inner) => {
                path.extend(inner.path);
                Invalid {
                    path,
                    error: inner.error,
                }
            }
            Err(error) => Invalid { path, error },
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.path.iter().enumerate() {
            match segment {
                PathSegment::Key(key) if i > 0 => write!(f, ".{key}")?,
                PathSegment::Key(key) => f.write_str(key)?,
                PathSegment::Index(index) => write!(f, "[{index}]")?,
            }
        }
        write!(f, ": {}", self.error)
    }
}

impl core::error::Error for Invalid {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&*self.error)
    }
}

impl From<&str> for PathSegment {
    fn from(key: &str) -> Self {
        PathSegment::Key(key.into())
    }
}

impl From<String> for PathSegment {
    fn from(key: String) -> Self {
        PathSegment::Key(key)
    }
}

impl From<usize> for PathSegment {
    fn from(index: usize) -> Self {
        PathSegment::Index(index)
    }
}

impl<F: fmt::Display> fmt::Display for Problem<F> {
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
            Problem::OutOfRange(m) => f.write_str(m),
            Problem::Format(e) => write!(f, "{e}"),
            Problem::Custom(e) => write!(f, "{e}"),
            Problem::Io(m) => write!(f, "input error: {m}"),
            Problem::Unsupported(m) => write!(f, "not supported by this format: {m}"),
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
#[derive(Debug, PartialEq)]
pub struct ErrorDetail<L, F> {
    /// Innermost-first while propagating; use [`ErrorDetail::path`].
    path_rev: Vec<PathSegment>,
    pub location: Option<L>,
    pub problem: Problem<F>,
    /// Enclosing structures, innermost-first.
    pub context: Vec<ContextFrame<L>>,
}

impl<L, F> ErrorDetail<L, F> {
    /// The application error this failure carries, if it is a `T` (see
    /// [`Problem::Custom`]).
    pub fn custom<T: core::error::Error + 'static>(&self) -> Option<&T> {
        match &self.problem {
            Problem::Custom(e) => e.downcast_ref::<T>(),
            _ => None,
        }
    }

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
/// accumulate mode). Boxed so `Result<T, DecodeError<L, F>>` stays small.
///
/// `L` is the reader's location type and `F` its format error type.
#[derive(PartialEq)]
pub struct DecodeError<L, F> {
    inner: Box<FirstAndRest<L, F>>,
}

/// The first failure, which always exists, and any others.
type FirstAndRest<L, F> = (ErrorDetail<L, F>, Vec<ErrorDetail<L, F>>);

impl<L, F> DecodeError<L, F> {
    pub fn new(problem: Problem<F>) -> Self {
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

    pub fn at(problem: Problem<F>, location: Option<L>) -> Self {
        let mut e = Self::new(problem);
        e.inner.0.location = location;
        e
    }

    /// An application error at `location`, as `refine` and `try_convert`
    /// report it. If it is an [`Invalid`], its path is added below the
    /// current position.
    pub fn invalid(error: impl Into<BoxedError>, location: Option<L>) -> Self {
        match error.into().downcast::<Invalid>() {
            Ok(invalid) => {
                let Invalid { path, error } = *invalid;
                let mut e = Self::at(
                    Problem::Custom(CustomError {
                        error,
                        fatal: false,
                    }),
                    location,
                );
                for segment in path.into_iter().rev() {
                    e = e.push_path(segment);
                }
                e
            }
            Err(error) => Self::at(
                Problem::Custom(CustomError {
                    error,
                    fatal: false,
                }),
                location,
            ),
        }
    }

    /// The first (or only) failure.
    pub fn first(&self) -> &ErrorDetail<L, F> {
        &self.inner.0
    }

    /// The first application error of type `T`, among all failures.
    pub fn custom<T: core::error::Error + 'static>(&self) -> Option<&T> {
        self.iter().find_map(|d| d.custom::<T>())
    }

    /// Every application error of type `T`, with the failure it belongs to
    /// (for its path and location).
    pub fn custom_errors<T: core::error::Error + 'static>(
        &self,
    ) -> impl Iterator<Item = (&ErrorDetail<L, F>, &T)> {
        self.iter().filter_map(|d| Some((d, d.custom::<T>()?)))
    }

    pub fn problem(&self) -> &Problem<F> {
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

    pub fn iter(&self) -> impl Iterator<Item = &ErrorDetail<L, F>> {
        core::iter::once(&self.inner.0).chain(self.inner.1.iter())
    }

    pub fn is_fatal(&self) -> bool
    where
        F: FormatError,
    {
        self.iter().any(|d| d.problem.is_fatal())
    }

    pub fn is_incomplete(&self) -> bool {
        matches!(self.inner.0.problem, Problem::Incomplete)
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
    pub fn merge(mut self, other: DecodeError<L, F>) -> Self {
        let (first, rest) = *other.inner;
        self.inner.1.push(first);
        self.inner.1.extend(rest);
        self
    }

    pub fn map_location<M>(self, f: impl Fn(L) -> M) -> DecodeError<M, F> {
        let map = |d: ErrorDetail<L, F>| ErrorDetail {
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
pub struct Collector<L, F> {
    acc: Option<DecodeError<L, F>>,
}

impl<L, F> Default for Collector<L, F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L, F> Collector<L, F> {
    pub fn new() -> Self {
        Collector { acc: None }
    }
    pub fn push(&mut self, e: DecodeError<L, F>) {
        self.acc = Some(match self.acc.take() {
            None => e,
            Some(a) => a.merge(e),
        });
    }
    pub fn has_errors(&self) -> bool {
        self.acc.is_some()
    }
    pub fn finish(self) -> Result<(), DecodeError<L, F>> {
        match self.acc {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}

impl<L: fmt::Debug, F: fmt::Debug> fmt::Debug for DecodeError<L, F> {
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
impl<L: fmt::Display, F: fmt::Display> fmt::Display for DecodeError<L, F> {
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

impl<L: fmt::Debug + fmt::Display, F: fmt::Debug + fmt::Display> core::error::Error
    for DecodeError<L, F>
{
}
