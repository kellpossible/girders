//! Versioned codecs: each version is its own type, migrations
//! are plain functions, decoding dispatches on a version tag and migrates
//! to the latest type, encoding always writes the latest version.

use crate::codec::{Codec, DecodeResult, Decoder, Encoder, decode_child};
use crate::context::{DecodeContext, EncodeContext, TagValue};
use crate::error::{DecodeError, PathSegment, Problem};
use crate::reader::{Kind, Reader};
use crate::serializer::Serializer;
use crate::union::scan_tag;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A version identifier: a number (`2`) or a label (`"2024-06"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Int(i64),
    Label(&'static str),
}

impl From<i64> for Version {
    fn from(v: i64) -> Self {
        Version::Int(v)
    }
}
impl From<i32> for Version {
    fn from(v: i32) -> Self {
        Version::Int(v as i64)
    }
}
impl From<&'static str> for Version {
    fn from(v: &'static str) -> Self {
        Version::Label(v)
    }
}

impl Version {
    fn tag_value(self) -> TagValue {
        match self {
            Version::Int(i) => TagValue::Int(i),
            Version::Label(s) => TagValue::Str(s),
        }
    }
    fn display(self) -> String {
        match self {
            Version::Int(i) => i.to_string(),
            Version::Label(s) => s.into(),
        }
    }
}

/// A version tag as read from input.
#[derive(Debug, Clone, PartialEq)]
enum FoundVersionTag {
    Int(i64),
    Label(String),
}

impl FoundVersionTag {
    fn matches(&self, v: Version) -> bool {
        match (self, v) {
            (FoundVersionTag::Int(a), Version::Int(b)) => *a == b,
            (FoundVersionTag::Label(a), Version::Label(b)) => a == b,
            _ => false,
        }
    }
    fn display(&self) -> String {
        match self {
            FoundVersionTag::Int(i) => i.to_string(),
            FoundVersionTag::Label(s) => s.clone(),
        }
    }
}

fn read_version<R: Reader + ?Sized>(r: &mut R) -> DecodeResult<(FoundVersionTag, Option<R::Location>), R> {
    let at = r.here();
    match r.peek_kind()? {
        Kind::Number => Ok((FoundVersionTag::Int(r.read_i64()?), at)),
        Kind::String => {
            let mut s = String::new();
            r.read_str(&mut s)?;
            Ok((FoundVersionTag::Label(s), at))
        }
        _ => Err(crate::reader::mismatch(r, "version number or label")),
    }
}

/// Type-level list of legacy decoders: `()` or `(Legacy<D>, Rest)`. No
/// boxing and no fixed reader type.
pub trait LegacyList<Output> {
    fn decode_version<R: Reader + ?Sized>(
        &self,
        v: &FoundVersion,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> Option<DecodeResult<Output, R>>;
    fn versions(&self, out: &mut Vec<Version>);
}

/// Opaque wrapper so `FoundVersionTag` stays private.
pub struct FoundVersion(FoundVersionTag);

impl<Output> LegacyList<Output> for () {
    fn decode_version<R: Reader + ?Sized>(
        &self,
        _: &FoundVersion,
        _: &mut R,
        _: &mut DecodeContext,
    ) -> Option<DecodeResult<Output, R>> {
        None
    }
    fn versions(&self, _: &mut Vec<Version>) {}
}

#[derive(Debug, Clone, Copy)]
pub struct Legacy<D> {
    version: Version,
    decoder: D,
}

impl<Output, D: Decoder<Output = Output>, Rest: LegacyList<Output>> LegacyList<Output> for (Legacy<D>, Rest) {
    fn decode_version<R: Reader + ?Sized>(
        &self,
        v: &FoundVersion,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> Option<DecodeResult<Output, R>> {
        if v.0.matches(self.0.version) {
            Some(decode_child(&self.0.decoder, r, cx))
        } else {
            self.1.decode_version(v, r, cx)
        }
    }
    fn versions(&self, out: &mut Vec<Version>) {
        self.1.versions(out);
        out.push(self.0.version);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Versioned<C, L> {
    tag: &'static str,
    envelope: Option<&'static str>,
    latest_version: Version,
    latest: C,
    default_version: Option<Version>,
    legacy: L,
}

/// `versioned("schema_version", 3, user_v3())`: the latest version's
/// full codec. Add older versions with [`Versioned::legacy`].
pub fn versioned<C: Codec<Output: Sized>>(
    tag: &'static str,
    latest_version: impl Into<Version>,
    latest: C,
) -> Versioned<C, ()> {
    Versioned {
        tag,
        envelope: None,
        latest_version: latest_version.into(),
        latest,
        default_version: None,
        legacy: (),
    }
}

impl<C: Codec<Output: Sized>, L: LegacyList<C::Output>> Versioned<C, L> {
    /// The version assumed when the tag is absent (legacy unversioned data).
    pub fn default_version(mut self, v: impl Into<Version>) -> Self {
        self.default_version = Some(v.into());
        self
    }

    /// Envelope form: `{"<tag>": 2, "<content>": {...}}` instead of the tag
    /// inside the object.
    pub fn envelope(mut self, content: &'static str) -> Self {
        self.envelope = Some(content);
        self
    }

    /// An older version: a decoder already mapped to the latest type.
    pub fn legacy<D>(self, version: impl Into<Version>, decoder: D) -> Versioned<C, (Legacy<D>, L)>
    where
        D: Decoder<Output = C::Output>,
    {
        Versioned {
            tag: self.tag,
            envelope: self.envelope,
            latest_version: self.latest_version,
            latest: self.latest,
            default_version: self.default_version,
            legacy: (
                Legacy {
                    version: version.into(),
                    decoder,
                },
                self.legacy,
            ),
        }
    }

    fn known(&self) -> Vec<String> {
        let mut vs = Vec::new();
        self.legacy.versions(&mut vs);
        vs.push(self.latest_version);
        vs.into_iter().map(Version::display).collect()
    }

    fn dispatch<R: Reader + ?Sized>(
        &self,
        found: FoundVersionTag,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> Option<DecodeResult<C::Output, R>> {
        if found.matches(self.latest_version) {
            return Some(decode_child(&self.latest, r, cx));
        }
        self.legacy.decode_version(&FoundVersion(found), r, cx)
    }
}

impl<C: Codec<Output: Sized>, L: LegacyList<C::Output>> Decoder for Versioned<C, L> {
    type Output = C::Output;

    fn decode<R: Reader + ?Sized>(&self, r: &mut R, cx: &mut DecodeContext) -> DecodeResult<C::Output, R> {
        let at = r.here();
        let (found, tag_at) = match scan_tag(r, cx, self.tag, read_version)? {
            Some(f) => f,
            None => match self.default_version {
                Some(Version::Int(i)) => (FoundVersionTag::Int(i), None),
                Some(Version::Label(s)) => (FoundVersionTag::Label(s.into()), None),
                None => return Err(DecodeError::at(Problem::MissingField(self.tag.into()), at)),
            },
        };
        let unknown = |found: &FoundVersionTag, cx: &DecodeContext| {
            let e = DecodeError::at(
                Problem::UnknownVersion {
                    found: found.display(),
                    known: self.known(),
                },
                tag_at.clone(),
            );
            if cx.options.track_path {
                e.push_path(PathSegment::Key(self.tag.into()))
            } else {
                e
            }
        };
        match self.envelope {
            None => match cx.with_claim(self.tag, |cx| self.dispatch(found.clone(), r, cx)) {
                Some(res) => res,
                None => Err(unknown(&found, cx)),
            },
            Some(content) => {
                r.begin_object()?;
                cx.nested(|cx| {
                    let mut out = None;
                    let mut key = String::new();
                    let mut first = true;
                    while r.object_next_key(first, &mut key)? {
                        first = false;
                        if key == self.tag {
                            r.skip_value()?;
                        } else if key == content && out.is_none() {
                            let res = self
                                .dispatch(found.clone(), r, cx)
                                .ok_or_else(|| unknown(&found, cx))?;
                            out = Some(res.map_err(|e| {
                                if cx.options.track_path {
                                    e.push_path(PathSegment::Key(content.into()))
                                } else {
                                    e
                                }
                            })?);
                        } else {
                            return Err(DecodeError::at(
                                Problem::UnknownField {
                                    name: key.clone(),
                                    expected: alloc::vec![self.tag, content],
                                },
                                r.here(),
                            ));
                        }
                        key.clear();
                    }
                    r.end_object()?;
                    out.ok_or_else(|| DecodeError::at(Problem::MissingField(content.into()), at))
                })
            }
        }
    }
}

impl<C: Codec<Output: Sized>, L: LegacyList<C::Output>> Encoder for Versioned<C, L> {
    type Input = C::Output;

    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &C::Output,
        s: &mut S,
        cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        match self.envelope {
            None => {
                cx.pending_tag = Some((self.tag, self.latest_version.tag_value()));
                let r = self.latest.encode(v, s, cx);
                cx.pending_tag = None;
                r
            }
            Some(content) => {
                s.begin_object(Some(2))?;
                s.object_key(self.tag)?;
                match self.latest_version {
                    Version::Int(i) => s.write_i64(i)?,
                    Version::Label(l) => s.write_str(l)?,
                }
                s.object_key(content)?;
                self.latest.encode(v, s, cx)?;
                s.end_object()
            }
        }
    }
}
