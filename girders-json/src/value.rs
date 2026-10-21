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
/// entries also get an index of positions: a hash table, so lookups take
/// one or two probes. The hash is fixed (there is no randomness without
/// `std`), so input can be crafted to collide; if any key lands more than
/// [`MAX_PROBES`] slots from home, the table is dropped for a sorted index
/// instead, so lookups stay O(log n) whatever the input.
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
    /// Lookup index; `None` for objects at or below the threshold.
    index: Index,
    /// Parallel to `entries` when key spans are recorded; otherwise empty.
    key_spans: Vec<Option<Span>>,
}

#[derive(Debug, Clone, Default)]
enum Index {
    #[default]
    None,
    /// Open addressing with linear probing, at most half full. Each slot
    /// holds the key's hash fragment and its position + 1 (0 = empty).
    /// A repeated key's slot points at its last occurrence.
    Hashed(Vec<(u32, u32)>),
    /// The fallback for colliding input: positions sorted by key, then by
    /// position.
    Sorted(Vec<u32>),
}

/// How far from its home slot a key may land before the hash table is
/// abandoned for the sorted index. Random keys stay well within this at
/// half load; it also bounds every lookup.
pub const MAX_PROBES: usize = 64;

const HASH_MULTIPLIER: u64 = 0xf135_7aea_2e62_a9c5;

/// A fast, fixed hash of a key's bytes (FxHash-style).
fn hash_key(key: &str) -> u64 {
    let bytes = key.as_bytes();
    let mut h = bytes.len() as u64;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let word = u64::from_le_bytes(chunk.try_into().expect("8 bytes"));
        h = (h.rotate_left(5) ^ word).wrapping_mul(HASH_MULTIPLIER);
    }
    let mut tail = [0u8; 8];
    tail[..chunks.remainder().len()].copy_from_slice(chunks.remainder());
    h = (h.rotate_left(5) ^ u64::from_le_bytes(tail)).wrapping_mul(HASH_MULTIPLIER);
    h ^ (h >> 29)
}

/// Keys of 16 ASCII bytes that all have the same hash: what an attacker
/// can build against a fixed hash. For tests and benchmarks.
#[doc(hidden)]
pub fn colliding_keys(n: usize) -> Vec<String> {
    let target = hash_key("0123456789abcdef");
    // The second word, given the first, is fixed by the target; retry the
    // first until the second is ASCII.
    let first_rounds = |w1: u64| {
        let h = (16u64.rotate_left(5) ^ w1).wrapping_mul(HASH_MULTIPLIER);
        h.rotate_left(5)
    };
    let pre_tail = {
        // Undo `h ^ (h >> 29)` and the last two rounds to find the state
        // before the second word that every key must reach.
        let mut h = target;
        h ^= h >> 29;
        h ^= h >> 58;
        let inverse = modular_inverse(HASH_MULTIPLIER);
        let after_tail = h.wrapping_mul(inverse).rotate_right(5);
        // The tail round mixed in an empty (zero) tail word.
        after_tail.wrapping_mul(inverse)
    };
    let mut keys = Vec::with_capacity(n);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    while keys.len() < n {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let w1 = seed & 0x7f7f_7f7f_7f7f_7f7f;
        let w2 = pre_tail ^ first_rounds(w1);
        if w2 & 0x8080_8080_8080_8080 != 0 {
            continue;
        }
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&w1.to_le_bytes());
        bytes.extend_from_slice(&w2.to_le_bytes());
        let key = String::from_utf8(bytes).expect("ASCII");
        debug_assert_eq!(hash_key(&key), target);
        keys.push(key);
    }
    keys
}

/// The inverse of an odd number modulo 2^64 (Newton's iteration).
fn modular_inverse(a: u64) -> u64 {
    let mut x = a;
    for _ in 0..6 {
        x = x.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(x)));
    }
    x
}

/// A hash table of positions for `entries`, or `None` if some key lands
/// more than [`MAX_PROBES`] from home.
fn build_hashed(entries: &[Entry]) -> Option<Vec<(u32, u32)>> {
    let capacity = (entries.len() * 2).next_power_of_two();
    let mut slots = alloc::vec![(0u32, 0u32); capacity];
    for (pos, entry) in entries.iter().enumerate() {
        insert_hashed(&mut slots, entries, &entry.key, pos)?;
    }
    Some(slots)
}

/// Record `key` at `pos` (overwriting an earlier occurrence); `None` if
/// it lands too far from home.
fn insert_hashed(slots: &mut [(u32, u32)], entries: &[Entry], key: &str, pos: usize) -> Option<()> {
    let mask = slots.len() - 1;
    let h = hash_key(key);
    let tag = h as u32;
    let mut i = (h >> 32) as usize & mask;
    for _ in 0..=MAX_PROBES {
        let (t, p) = slots[i];
        if p == 0 {
            slots[i] = (tag, pos as u32 + 1);
            return Some(());
        }
        if t == tag && entries[p as usize - 1].key == key {
            slots[i].1 = pos as u32 + 1;
            return Some(());
        }
        i = (i + 1) & mask;
    }
    None
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

    pub fn with_capacity(capacity: usize) -> Self {
        Object {
            entries: Vec::with_capacity(capacity),
            extra: None,
        }
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
        let pos = self.entries.len();
        self.entries.push(Entry {
            key: key.into(),
            value,
        });
        let Some(x) = &mut self.extra else {
            return self.reindex();
        };
        if !x.key_spans.is_empty() {
            x.key_spans.push(None);
        }
        let key = self.entries[pos].key.as_str();
        let updated = match &mut x.index {
            Index::None => None,
            Index::Hashed(slots) => (slots.len() >= 2 * (pos + 1))
                .then(|| insert_hashed(slots, &self.entries, key, pos))
                .flatten(),
            Index::Sorted(ix) => {
                let slot = ix.partition_point(|&p| self.entries[p as usize].key.as_str() <= key);
                ix.insert(slot, pos as u32);
                Some(())
            }
        };
        if updated.is_none() {
            self.reindex();
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

    /// Build (or drop) the index to match the current size: a hash table,
    /// or the sorted index if the keys collide too much.
    pub fn reindex(&mut self) {
        if self.entries.len() <= Self::INDEX_THRESHOLD {
            if let Some(x) = &mut self.extra {
                x.index = Index::None;
            }
            self.drop_empty_extra();
            return;
        }
        let index = match build_hashed(&self.entries) {
            Some(slots) => Index::Hashed(slots),
            None => {
                let mut ix: Vec<u32> = (0..self.entries.len() as u32).collect();
                // Stable: equal keys stay in document order, so the last one
                // is the last occurrence.
                ix.sort_by(|&a, &b| {
                    self.entries[a as usize]
                        .key
                        .cmp(&self.entries[b as usize].key)
                });
                Index::Sorted(ix)
            }
        };
        self.extra.get_or_insert_with(Default::default).index = index;
    }

    /// Whether lookups use the hash table (rather than a scan or the sorted
    /// fallback).
    #[doc(hidden)]
    pub fn is_hashed(&self) -> bool {
        matches!(
            self.extra.as_deref().map(|x| &x.index),
            Some(Index::Hashed(_))
        )
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

    fn drop_empty_extra(&mut self) {
        if self
            .extra
            .as_ref()
            .is_some_and(|x| matches!(x.index, Index::None) && x.key_spans.is_empty())
        {
            self.extra = None;
        }
    }

    fn position(&self, key: &str) -> Option<usize> {
        match self.extra.as_deref().map(|x| &x.index) {
            None | Some(Index::None) => self.entries.iter().rposition(|e| e.key == key),
            Some(Index::Hashed(slots)) => {
                let mask = slots.len() - 1;
                let h = hash_key(key);
                let tag = h as u32;
                let mut i = (h >> 32) as usize & mask;
                // Every key was placed within MAX_PROBES of home.
                for _ in 0..=MAX_PROBES {
                    let (t, p) = slots[i];
                    if p == 0 {
                        return None;
                    }
                    if t == tag && self.entries[p as usize - 1].key == key {
                        return Some(p as usize - 1);
                    }
                    i = (i + 1) & mask;
                }
                None
            }
            Some(Index::Sorted(ix)) => {
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
            let capacity = girders::reader::cautious_capacity::<Value>(r.size_hint());
            let items = cx.nested(|cx| {
                let mut items = Vec::with_capacity(capacity);
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
            let capacity = girders::reader::cautious_capacity::<Entry>(r.size_hint());
            let m = cx.nested(|cx| {
                let mut m = Object::with_capacity(capacity);
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
