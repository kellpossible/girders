//! Benchmark models: each corpus type defined twice, with serde derives
//! (the baseline) and as girders codecs, decoding the same documents
//! equally strictly (`deny_unknown_fields` ↔ girders' default).
//!
//! The serde models follow serde-rs/json-benchmark (Apache-2.0),
//! <https://github.com/serde-rs/json-benchmark>, at commit
//! 17b13dd2d7a5e5fdd5594e847077932f955b5e2b.

pub mod canada;
pub mod citm;
pub mod twitter;

use std::path::PathBuf;

/// Corpus files, fetched by `mise run fetch-bench-data`.
pub fn data(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run `mise run fetch-bench-data`)", path.display()))
}

/// `Value` objects backed by `indexmap` (insertion order + hash index),
/// for comparison with girders' own map types. Last duplicate wins, in
/// the first occurrence's position.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Indexed(pub indexmap::IndexMap<String, girders_json::Value<Indexed>>);

impl girders_json::ObjectMap for Indexed {
    fn new() -> Self {
        Indexed(indexmap::IndexMap::new())
    }
    fn insert(
        &mut self,
        key: String,
        _key_span: Option<girders::Span>,
        value: girders_json::Value<Self>,
    ) {
        self.0.insert(key, value);
    }
    fn get(&self, key: &str) -> Option<&girders_json::Value<Self>> {
        self.0.get(key)
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn entries(&self) -> Box<dyn Iterator<Item = (&str, &girders_json::Value<Self>)> + '_> {
        Box::new(self.0.iter().map(|(k, v)| (k.as_str(), v)))
    }
}

/// The previous `Ordered`: a plain `Vec`, linear lookups, no index. Kept
/// for before/after comparison.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Linear(pub Vec<(String, girders_json::Value<Linear>)>);

impl girders_json::ObjectMap for Linear {
    fn new() -> Self {
        Linear(Vec::new())
    }
    fn insert(
        &mut self,
        key: String,
        _key_span: Option<girders::Span>,
        value: girders_json::Value<Self>,
    ) {
        self.0.push((key, value));
    }
    fn get(&self, key: &str) -> Option<&girders_json::Value<Self>> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn entries(&self) -> Box<dyn Iterator<Item = (&str, &girders_json::Value<Self>)> + '_> {
        Box::new(self.0.iter().map(|(k, v)| (k.as_str(), v)))
    }
}

