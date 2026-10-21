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
use siphasher::sip::SipHasher13;

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
/// entries also get an index of positions: a hash table keyed with
/// SipHash-1-3 (the `siphasher` crate), so lookups take one or two probes. The key is
/// `DecodeOptions::hash_key`, else random per process with `std`. Without
/// `std` and without a key it is fixed, so input can be crafted to collide:
/// if any key lands more than [`MAX_PROBES`] slots from home, the object
/// uses a sorted index instead, and lookups stay O(log n) whatever the
/// input.
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
    Hashed(HashTable),
    /// The fallback when keys collide: positions sorted by key, then by
    /// position.
    Sorted(Vec<u32>),
}

/// How far from its home slot a key may land before the hash table is
/// abandoned for the sorted index; it also bounds every lookup. With Robin
/// Hood probing at half load, honest keys measured at most 16 even with
/// 2^22 entries (`longest_probe_for_honest_keys`).
pub const MAX_PROBES: usize = 32;

/// The key for objects decoded without `DecodeOptions::hash_key`: random
/// per process with `std`, fixed without it (the sorted fallback then
/// guards against crafted collisions).
fn default_hash_key() -> [u64; 2] {
    #[cfg(feature = "std")]
    {
        static KEY: std::sync::OnceLock<[u64; 2]> = std::sync::OnceLock::new();
        *KEY.get_or_init(|| {
            use std::hash::BuildHasher;
            let state = std::collections::hash_map::RandomState::new();
            [state.hash_one(0u64), state.hash_one(1u64)]
        })
    }
    #[cfg(not(feature = "std"))]
    {
        [0x0706_0504_0302_0100, 0x0f0e_0d0c_0b0a_0908]
    }
}

/// The 32 hash bits a slot keeps: they pick the home slot and filter key
/// comparisons.
///
/// The hash is SipHash-1-3 (what Rust's `HashMap` uses): keyed, so without
/// the key collisions can't be computed.
///
/// It comes from the `siphasher` crate, girders-json's one dependency: this
/// is the security-relevant part of the index, so it uses a vetted
/// implementation rather than our own.
fn slot_hash(hasher: &SipHasher13, text: &str) -> u32 {
    (hasher.hash(text.as_bytes()) >> 32) as u32
}

/// Positions of an object's entries by key: open addressing with linear,
/// Robin Hood probing, at most half full. A repeated key's slot points at
/// its last occurrence.
#[derive(Debug, Clone)]
struct HashTable {
    key: [u64; 2],
    hasher: SipHasher13,
    /// `(hash bits, position + 1)`; position 0 marks an empty slot.
    slots: Vec<(u32, u32)>,
}

impl HashTable {
    /// The table for `entries`, or `None` if some key lands more than
    /// `limit` slots from home. Also returns the largest distance from
    /// home (for measuring).
    fn build(entries: &[Entry], key: [u64; 2], limit: usize) -> Option<(Self, usize)> {
        let capacity = (entries.len() * 2).next_power_of_two();
        let mut table = HashTable {
            key,
            hasher: SipHasher13::new_with_keys(key[0], key[1]),
            slots: alloc::vec![(0, 0); capacity],
        };
        let mut longest = 0;
        for (pos, entry) in entries.iter().enumerate() {
            longest = longest.max(table.insert(entries, &entry.key, pos, limit)?);
        }
        Some((table, longest))
    }

    fn home(&self, hash: u32) -> usize {
        hash as usize & (self.slots.len() - 1)
    }

    fn distance(&self, slot: usize, hash: u32) -> usize {
        slot.wrapping_sub(self.home(hash)) & (self.slots.len() - 1)
    }

    /// Record `text` at `pos` (overwriting an earlier occurrence of the
    /// same key). Returns the largest distance from home this caused, or
    /// `None` past `limit`.
    fn insert(&mut self, entries: &[Entry], text: &str, pos: usize, limit: usize) -> Option<usize> {
        let mask = self.slots.len() - 1;
        let mut carried = (slot_hash(&self.hasher, text), pos as u32 + 1);
        let mut i = self.home(carried.0);
        let mut distance = 0;
        let mut longest = 0;
        // Until something is displaced, the key itself may already be in
        // the table further along; after that, the carried entry is unique.
        let mut may_be_present = true;
        loop {
            if distance > limit {
                return None;
            }
            longest = longest.max(distance);
            let slot = self.slots[i];
            if slot.1 == 0 {
                self.slots[i] = carried;
                return Some(longest);
            }
            if may_be_present && slot.0 == carried.0 && entries[slot.1 as usize - 1].key == text {
                self.slots[i].1 = carried.1;
                return Some(longest);
            }
            let resident = self.distance(i, slot.0);
            if resident < distance {
                // Robin Hood: the entry nearer its home moves on instead.
                self.slots[i] = carried;
                carried = slot;
                distance = resident;
                may_be_present = false;
            }
            i = (i + 1) & mask;
            distance += 1;
        }
    }

    fn find(&self, entries: &[Entry], text: &str) -> Option<usize> {
        let mask = self.slots.len() - 1;
        let hash = slot_hash(&self.hasher, text);
        let mut i = self.home(hash);
        for distance in 0..=MAX_PROBES {
            let (h, p) = self.slots[i];
            // An empty slot, or an entry nearer its home than this key would
            // be, ends the search: Robin Hood keeps them in that order.
            if p == 0 || self.distance(i, h) < distance {
                return None;
            }
            if h == hash && entries[p as usize - 1].key == text {
                return Some(p as usize - 1);
            }
            i = (i + 1) & mask;
        }
        None
    }
}

/// `n` keys for one object, of which `MAX_PROBES + 2` share a home slot
/// under `key`: what an attacker who knows the key builds to force the
/// sorted fallback. For tests and benchmarks.
#[doc(hidden)]
pub fn keys_forcing_fallback(n: usize, key: [u64; 2]) -> Vec<String> {
    let mask = (n * 2).next_power_of_two() - 1;
    let hasher = SipHasher13::new_with_keys(key[0], key[1]);
    let colliding = (MAX_PROBES + 2).min(n);
    let mut keys: Vec<String> = (0..n - colliding).map(|i| alloc::format!("k{i}")).collect();
    let mut candidate = 0u64;
    while keys.len() < n {
        let text = alloc::format!("c{candidate}");
        candidate += 1;
        if slot_hash(&hasher, &text) as usize & mask == 0 {
            keys.push(text);
        }
    }
    keys
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
            Index::Hashed(table) => (table.slots.len() >= 2 * (pos + 1))
                .then(|| table.insert(&self.entries, key, pos, MAX_PROBES))
                .flatten()
                .map(|_| ()),
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
        let key = match self.extra.as_deref().map(|x| &x.index) {
            Some(Index::Hashed(table)) => table.key,
            _ => default_hash_key(),
        };
        self.reindex_with(key);
    }

    fn reindex_with(&mut self, key: [u64; 2]) {
        if self.entries.len() <= Self::INDEX_THRESHOLD {
            if let Some(x) = &mut self.extra {
                x.index = Index::None;
            }
            self.drop_empty_extra();
            return;
        }
        let index = match HashTable::build(&self.entries, key, MAX_PROBES) {
            Some((table, _)) => Index::Hashed(table),
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

    /// After decoding: build the index, hashing with `key` (else the
    /// default key).
    fn finish(&mut self, key: Option<[u64; 2]>) {
        if self.entries.len() > Self::INDEX_THRESHOLD {
            self.reindex_with(key.unwrap_or_else(default_hash_key));
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
            Some(Index::Hashed(table)) => table.find(&self.entries, key),
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
        m.finish(None);
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
                m.finish(cx.options.hash_key);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest distance from home for honest keys, per table size at
    /// the highest load (exactly half full). Run with `--ignored
    /// --nocapture` to choose `MAX_PROBES`.
    #[test]
    #[ignore]
    fn longest_probe_for_honest_keys() {
        for shift in [6, 8, 10, 12, 14, 16, 18, 20, 22] {
            let n = 1usize << shift;
            let mut worst = 0;
            for trial in 0..5u64 {
                let entries: Vec<Entry> = (0..n)
                    .map(|i| Entry {
                        key: alloc::format!("key{trial}_{i}"),
                        value: Value::null(),
                    })
                    .collect();
                let key = [trial.wrapping_mul(0x9e37_79b9), !trial];
                let (_, longest) = HashTable::build(&entries, key, usize::MAX).unwrap();
                worst = worst.max(longest);
            }
            std::println!("n = 2^{shift}: longest probe {worst}");
        }
    }
}
