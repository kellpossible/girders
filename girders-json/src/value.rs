//! A self-describing value tree. Objects keep their entries in document
//! order, duplicates included.
//!
//! `Value` decodes through the generic [`Reader`] trait, so it works over
//! any girders format; spans are recorded when the format has byte spans
//! and location tracking is on.

use crate::pointer::Pointer;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use girders::codec::DecodeResult;
use girders::prelude::*;
use girders::{Number, Span};

#[derive(Debug, Clone, PartialEq)]
pub struct Value {
    pub node: Node,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Object),
}

/// An object's entries in document order, duplicate keys kept (lossless);
/// lookups return the last occurrence.
///
/// Small objects are a plain `Vec` scanned linearly, the fastest option
/// at that size. Objects with more than [`Object::INDEX_THRESHOLD`]
/// entries also get a sorted index of positions, so lookups are a binary
/// search: O(log n) whatever the input, with no hashing (so nothing for
/// an attacker to collide) and 4 bytes per entry.
///
/// Key spans (where each key appears in the source) are recorded only when
/// decoding with `DecodeOptions::key_spans`; otherwise they cost nothing.
#[derive(Debug, Clone, Default)]
pub struct Object {
    entries: Vec<Entry>,
    /// Optional per-object extras: the lookup index and key spans. One
    /// thin pointer in every object, allocated only when needed.
    extra: Option<Box<Extra>>,
}

#[derive(Debug, Clone, Default)]
struct Extra {
    /// Positions into `entries`, sorted by key, then by position. Empty
    /// for objects at or below the threshold.
    index: Vec<u32>,
    /// Parallel to `entries` when key spans are recorded; otherwise empty.
    key_spans: Vec<Option<Span>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: Value,
}

/// Equality compares entries only (not the index or key spans).
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Object {
    /// Objects with more entries than this get a lookup index.
    pub const INDEX_THRESHOLD: usize = 64;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in document order.
    pub fn as_slice(&self) -> &[Entry] {
        &self.entries
    }

    /// Keys and values in document order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|e| (e.key.as_str(), &e.value))
    }

    pub fn iter_mut_values(&mut self) -> impl Iterator<Item = &mut Value> {
        self.entries.iter_mut().map(|e| &mut e.value)
    }

    /// The value for `key` (last occurrence).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.position(key).map(|i| &self.entries[i].value)
    }

    /// The value for `key` (last occurrence), mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        let i = self.position(key)?;
        Some(&mut self.entries[i].value)
    }

    /// Where the `i`th key appeared in the source, if key spans were
    /// recorded.
    pub fn key_span(&self, i: usize) -> Option<Span> {
        self.extra.as_ref()?.key_spans.get(i).copied().flatten()
    }

    /// The value for `key` (last occurrence) and the key's span.
    pub fn get_with_key_span(&self, key: &str) -> Option<(&Value, Option<Span>)> {
        let i = self.position(key)?;
        Some((&self.entries[i].value, self.key_span(i)))
    }

    /// Append an entry, keeping the index up to date.
    pub fn push(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        let pos = self.entries.len() as u32;
        let slot = self
            .index()
            .map(|ix| ix.partition_point(|&p| self.entries[p as usize].key <= key));
        self.entries.push(Entry { key, value });
        if let Some(x) = &mut self.extra
            && !x.key_spans.is_empty()
        {
            x.key_spans.push(None);
        }
        match slot {
            Some(slot) => self
                .extra
                .as_mut()
                .expect("index exists")
                .index
                .insert(slot, pos),
            None => self.reindex(),
        }
    }

    /// Insert an entry at `at` (positions after it shift), rebuilding the
    /// index if there is one.
    pub fn insert_at(&mut self, at: usize, key: impl Into<String>, value: Value) {
        self.entries.insert(
            at,
            Entry {
                key: key.into(),
                value,
            },
        );
        if let Some(x) = &mut self.extra
            && !x.key_spans.is_empty()
        {
            x.key_spans.insert(at, None);
        }
        self.reindex();
    }

    /// Build (or drop) the index to match the current size.
    pub fn reindex(&mut self) {
        if self.entries.len() <= Self::INDEX_THRESHOLD {
            if let Some(x) = &mut self.extra {
                x.index = Vec::new();
            }
            self.drop_empty_extra();
            return;
        }
        let mut ix: Vec<u32> = (0..self.entries.len() as u32).collect();
        // Stable: equal keys stay in document order, so the last one is
        // the last occurrence.
        ix.sort_by(|&a, &b| {
            self.entries[a as usize]
                .key
                .cmp(&self.entries[b as usize].key)
        });
        self.extra.get_or_insert_with(Default::default).index = ix;
    }

    /// Append while decoding, without touching the index; [`Object::finish`]
    /// builds it once at the end.
    #[inline]
    fn push_decoded(&mut self, key: String, key_span: Option<Span>, value: Value) {
        if key_span.is_some() {
            let n = self.entries.len();
            let spans = &mut self.extra.get_or_insert_with(Default::default).key_spans;
            spans.resize(n, None);
            spans.push(key_span);
        } else if let Some(x) = &mut self.extra
            && !x.key_spans.is_empty()
        {
            x.key_spans.push(None);
        }
        self.entries.push(Entry { key, value });
    }

    fn finish(&mut self) {
        if self.entries.len() > Self::INDEX_THRESHOLD {
            self.reindex();
        }
    }

    fn index(&self) -> Option<&[u32]> {
        self.extra
            .as_deref()
            .map(|x| x.index.as_slice())
            .filter(|ix| !ix.is_empty())
    }

    fn drop_empty_extra(&mut self) {
        if self
            .extra
            .as_ref()
            .is_some_and(|x| x.index.is_empty() && x.key_spans.is_empty())
        {
            self.extra = None;
        }
    }

    fn position(&self, key: &str) -> Option<usize> {
        match self.index() {
            None => self.entries.iter().rposition(|e| e.key == key),
            Some(ix) => {
                let end = ix.partition_point(|&p| self.entries[p as usize].key.as_str() <= key);
                let p = *ix.get(end.checked_sub(1)?)? as usize;
                (self.entries[p].key == key).then_some(p)
            }
        }
    }
}

impl Value {
    pub fn new(node: Node) -> Self {
        Value { node, span: None }
    }
    pub fn null() -> Self {
        Self::new(Node::Null)
    }
    pub fn bool(b: bool) -> Self {
        Self::new(Node::Bool(b))
    }
    pub fn int(i: i64) -> Self {
        Self::new(Node::Number(Number::I64(i)))
    }
    pub fn float(f: f64) -> Self {
        Self::new(Node::Number(Number::F64(f)))
    }
    pub fn string(s: impl Into<String>) -> Self {
        Self::new(Node::String(s.into()))
    }
    pub fn array(items: Vec<Value>) -> Self {
        Self::new(Node::Array(items))
    }
    pub fn object(entries: impl IntoIterator<Item = (String, Value)>) -> Self {
        let mut m = Object::new();
        for (k, v) in entries {
            m.push_decoded(k, None, v);
        }
        m.finish();
        Self::new(Node::Object(m))
    }

    /// Field of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match &self.node {
            Node::Object(m) => m.get(key),
            _ => None,
        }
    }

    /// Object keys in document order.
    pub fn keys(&self) -> Vec<&str> {
        match &self.node {
            Node::Object(m) => m.iter().map(|(k, _)| k).collect(),
            _ => Vec::new(),
        }
    }

    /// Resolve an RFC 6901 pointer.
    pub fn pointer(&self, pointer: &Pointer) -> Option<&Value> {
        let mut cur = self;
        for seg in pointer.segments() {
            cur = match &cur.node {
                Node::Object(m) => m.get(seg)?,
                Node::Array(items) => {
                    if seg.len() > 1 && seg.starts_with('0') {
                        return None;
                    }
                    items.get(seg.parse::<usize>().ok()?)?
                }
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Resolve a pointer given as a string; `None` if it doesn't parse or
    /// resolve.
    pub fn pointer_str(&self, pointer: &str) -> Option<&Value> {
        self.pointer(&Pointer::parse(pointer).ok()?)
    }

    /// Drop all spans, including key spans (for comparing trees
    /// structurally).
    pub fn without_spans(mut self) -> Self {
        self.strip();
        self
    }

    fn strip(&mut self) {
        self.span = None;
        match &mut self.node {
            Node::Array(items) => items.iter_mut().for_each(Value::strip),
            Node::Object(m) => {
                if let Some(x) = &mut m.extra {
                    x.key_spans = Vec::new();
                }
                m.drop_empty_extra();
                m.iter_mut_values().for_each(Value::strip);
            }
            _ => {}
        }
    }
}

// -- codec -------------------------------------------------------------------

/// The [`Value`] codec, over any format.
#[derive(Debug, Clone, Copy, Default)]
pub struct ValueCodec;

pub const fn value() -> ValueCodec {
    ValueCodec
}

fn decode_value<R: Reader + ?Sized>(r: &mut R, cx: &mut DecodeContext) -> DecodeResult<Value, R> {
    let start = r.here();
    let node = match r.peek_kind()? {
        Kind::Null => {
            r.read_null()?;
            Node::Null
        }
        Kind::Bool => Node::Bool(r.read_bool()?),
        Kind::Number => Node::Number(r.read_number()?),
        Kind::String => {
            let mut s = String::new();
            r.read_str(&mut s)?;
            Node::String(s)
        }
        Kind::Array => {
            r.begin_array()?;
            let items = cx.nested(|cx| {
                let mut items = Vec::new();
                let mut first = true;
                while r.array_has_next(first)? {
                    first = false;
                    let i = items.len();
                    items.push(decode_value(r, cx).map_err(|e| {
                        if cx.options.track_path {
                            e.push_path(PathSegment::Index(i))
                        } else {
                            e
                        }
                    })?);
                }
                Ok(items)
            })?;
            r.end_array()?;
            Node::Array(items)
        }
        Kind::Object => {
            r.begin_object()?;
            let m = cx.nested(|cx| {
                let mut m = Object::new();
                let mut first = true;
                let mut key = String::new();
                while r.object_next_key(first, &mut key)? {
                    first = false;
                    let key_span = if cx.options.key_spans {
                        r.key_location().and_then(|l| r.byte_span(&l))
                    } else {
                        None
                    };
                    let v = decode_value(r, cx).map_err(|e| {
                        if cx.options.track_path {
                            e.push_path(PathSegment::Key(key.clone()))
                        } else {
                            e
                        }
                    })?;
                    m.push_decoded(core::mem::take(&mut key), key_span, v);
                }
                m.finish();
                Ok(m)
            })?;
            r.end_object()?;
            Node::Object(m)
        }
        Kind::Eof | Kind::Bytes | Kind::Tag | Kind::Simple => {
            return Err(girders::reader::mismatch(r, "value"));
        }
    };
    let span = start.and_then(|s| {
        let loc = r.since(&s);
        r.byte_span(&loc)
    });
    Ok(Value { node, span })
}

impl Decoder for ValueCodec {
    type Output = Value;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        cx: &mut DecodeContext,
    ) -> DecodeResult<Value, R> {
        decode_value(r, cx)
    }
}

fn encode_value<S: Serializer + ?Sized>(v: &Value, s: &mut S) -> Result<(), S::Error> {
    match &v.node {
        Node::Null => s.write_null(),
        Node::Bool(b) => s.write_bool(*b),
        Node::Number(Number::I64(i)) => s.write_i64(*i),
        Node::Number(Number::U64(u)) => s.write_u64(*u),
        Node::Number(Number::F64(f)) => s.write_f64(*f),
        Node::Number(Number::BigInt(t)) => s.write_number_text(t),
        Node::String(t) => s.write_str(t),
        Node::Array(items) => {
            s.begin_array(Some(items.len()))?;
            for x in items {
                s.array_element()?;
                encode_value(x, s)?;
            }
            s.end_array()
        }
        Node::Object(m) => {
            s.begin_object(Some(m.len()))?;
            for (k, x) in m.iter() {
                s.object_key(k)?;
                encode_value(x, s)?;
            }
            s.end_object()
        }
    }
}

impl Encoder for ValueCodec {
    type Input = Value;
    fn encode<S: Serializer + ?Sized>(
        &self,
        v: &Value,
        s: &mut S,
        _cx: &mut EncodeContext,
    ) -> Result<(), S::Error> {
        encode_value(v, s)
    }
}
