//! Runtime benchmarks (gungraun: Callgrind instruction counts, plus DHAT
//! heap profiles). girders vs serde_json on the json-benchmark corpus,
//! and girders' own cost knobs.
//!
//! Run: `mise run bench` (needs valgrind and gungraun-runner 0.20.0).

use girders::DecodeOptions;
use girders::collections::list;
use girders_benches::{canada, citm, data, events, twitter};
use girders_json::{StreamDecoder, Value, value};
use gungraun::{Dhat, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

fn load(name: &str) -> String {
    data(name)
}

// -- typed decode ---------------------------------------------------------------------

#[library_benchmark]
#[bench::canada(setup = load, args = ["canada.json"])]
fn decode_serde_canada(json: String) -> canada::Canada {
    black_box(serde_json::from_str(black_box(&json)).unwrap())
}

#[library_benchmark]
#[bench::canada(setup = load, args = ["canada.json"])]
fn decode_girders_canada(json: String) -> canada::Canada {
    black_box(girders_json::decode(black_box(&json), &canada::codec()).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = load, args = ["citm_catalog.json"])]
fn decode_serde_citm(json: String) -> citm::CitmCatalog {
    black_box(serde_json::from_str(black_box(&json)).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = load, args = ["citm_catalog.json"])]
fn decode_girders_citm(json: String) -> citm::CitmCatalog {
    black_box(girders_json::decode(black_box(&json), &citm::codec()).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = load, args = ["twitter.json"])]
fn decode_serde_twitter(json: String) -> twitter::Twitter {
    black_box(serde_json::from_str(black_box(&json)).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = load, args = ["twitter.json"])]
fn decode_girders_twitter(json: String) -> twitter::Twitter {
    black_box(girders_json::decode(black_box(&json), &twitter::codec()).unwrap())
}

library_benchmark_group!(
    name = decode_typed,
    benchmarks = [
        decode_serde_canada,
        decode_girders_canada,
        decode_serde_citm,
        decode_girders_citm,
        decode_serde_twitter,
        decode_girders_twitter
    ]
);

// -- girders' cost knobs (twitter, the string/key-heavy corpus) --------------------------

fn opts(kind: &str) -> (String, DecodeOptions) {
    let o = match kind {
        "default" => DecodeOptions::new(),
        "no_locations" => DecodeOptions::new().locations(false),
        "no_paths" => DecodeOptions::new().paths(false),
        "fast" => DecodeOptions::fast(),
        "accumulate" => DecodeOptions::strict(),
        _ => unreachable!(),
    };
    (data("twitter.json"), o)
}

#[library_benchmark]
#[benches::twitter(setup = opts, args = ["default", "no_locations", "no_paths", "fast", "accumulate"])]
fn decode_options(input: (String, DecodeOptions)) -> twitter::Twitter {
    black_box(girders_json::decode_with(black_box(&input.0), &twitter::codec(), input.1).unwrap())
}

library_benchmark_group!(name = options, benchmarks = [decode_options]);

// -- typed encode ---------------------------------------------------------------------

fn canada_value(name: &str) -> canada::Canada {
    serde_json::from_str(&data(name)).unwrap()
}
fn citm_value(name: &str) -> citm::CitmCatalog {
    serde_json::from_str(&data(name)).unwrap()
}
fn twitter_value(name: &str) -> twitter::Twitter {
    serde_json::from_str(&data(name)).unwrap()
}

#[library_benchmark]
#[bench::canada(setup = canada_value, args = ["canada.json"])]
fn encode_serde_canada(v: canada::Canada) -> String {
    black_box(serde_json::to_string(black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::canada(setup = canada_value, args = ["canada.json"])]
fn encode_girders_canada(v: canada::Canada) -> String {
    black_box(girders_json::encode(black_box(&v), &canada::codec()).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_serde_citm(v: citm::CitmCatalog) -> String {
    black_box(serde_json::to_string(black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_girders_citm(v: citm::CitmCatalog) -> String {
    black_box(girders_json::encode(black_box(&v), &citm::codec()).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_serde_twitter(v: twitter::Twitter) -> String {
    black_box(serde_json::to_string(black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_girders_twitter(v: twitter::Twitter) -> String {
    black_box(girders_json::encode(black_box(&v), &twitter::codec()).unwrap())
}

library_benchmark_group!(
    name = encode_typed,
    benchmarks = [
        encode_serde_canada,
        encode_girders_canada,
        encode_serde_citm,
        encode_girders_citm,
        encode_serde_twitter,
        encode_girders_twitter
    ]
);

// -- dynamic values -----------------------------------------------------------------

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_serde(json: String) -> serde_json::Value {
    black_box(serde_json::from_str(black_box(&json)).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_girders(json: String) -> Value {
    black_box(girders_json::parse(black_box(&json)).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json"])]
fn value_girders_fast(json: String) -> Value {
    black_box(girders_json::decode_with(black_box(&json), &value(), DecodeOptions::fast()).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_girders_key_spans(json: String) -> Value {
    black_box(
        girders_json::decode_with(
            black_box(&json),
            &value(),
            DecodeOptions::new().key_spans(true),
        )
        .unwrap(),
    )
}

library_benchmark_group!(
    name = dynamic_value,
    benchmarks = [
        value_serde,
        value_girders,
        value_girders_fast,
        value_girders_key_spans
    ]
);

// -- streaming ----------------------------------------------------------------------

fn chunked(name: &str, chunk: usize) -> (String, usize) {
    (data(name), chunk)
}

// `io::Read` source with an internal buffer of `chunk` bytes.
#[library_benchmark]
#[benches::twitter(setup = chunked, args = [("twitter.json", 4096), ("twitter.json", 65536)])]
fn stream_reader(input: (String, usize)) -> twitter::Twitter {
    let (json, chunk) = input;
    black_box(
        girders_json::from_reader_with(
            black_box(json.as_bytes()),
            &twitter::codec(),
            DecodeOptions::new(),
            chunk,
        )
        .unwrap(),
    )
}

#[library_benchmark]
#[bench::twitter(setup = load, args = ["twitter.json"])]
fn stream_serde_reader(json: String) -> twitter::Twitter {
    black_box(serde_json::from_reader(std::io::BufReader::new(black_box(json.as_bytes()))).unwrap())
}

// Push decoder fed `chunk` bytes at a time: restarts the document on
// every chunk boundary (O(k·n) for one big value).
#[library_benchmark]
#[benches::twitter(setup = chunked, args = [("twitter.json", 65536), ("twitter.json", 1 << 30)])]
fn stream_push_one_document(input: (String, usize)) -> (twitter::Twitter, u64) {
    let (json, chunk) = input;
    let mut d = StreamDecoder::new(twitter::codec());
    let mut out = None;
    for c in json.as_bytes().chunks(chunk) {
        d.push(c);
        if let Some(r) = d.next() {
            out = Some(r.unwrap());
        }
    }
    d.finish();
    if let Some(r) = d.next() {
        out = Some(r.unwrap());
    }
    black_box((out.unwrap(), d.restarts))
}

library_benchmark_group!(
    name = streaming,
    benchmarks = [stream_reader, stream_serde_reader, stream_push_one_document]
);

// -- Value lookups and encoding -----------------------------------------------------

use girders_json::Node;

/// Every status's `user.screen_name`, `id` and `text`, and
/// `search_metadata.count`: typical shallow lookups in small objects.
fn twitter_lookups(v: &Value) -> usize {
    let mut n = 0;
    if let Some(Node::Array(statuses)) = v.get("statuses").map(|s| &s.node) {
        for st in statuses {
            n += st.get("id").is_some() as usize;
            n += st.get("text").is_some() as usize;
            if let Some(u) = st.get("user") {
                n += u.get("screen_name").is_some() as usize;
            }
        }
    }
    n + v
        .get("search_metadata")
        .and_then(|m| m.get("count"))
        .is_some() as usize
}

/// Look up every key of citm's 184-entry `events` object: lookups in a
/// large (indexed) object.
fn citm_lookups(input: &(Value, Vec<String>)) -> usize {
    let (v, keys) = input;
    let events = v.get("events").unwrap();
    keys.iter().filter(|k| events.get(k).is_some()).count()
}

fn parse_value(name: &str) -> Value {
    girders_json::parse(&data(name)).unwrap()
}

fn parse_citm_keys(name: &str) -> (Value, Vec<String>) {
    let v = parse_value(name);
    let keys = v
        .get("events")
        .unwrap()
        .keys()
        .into_iter()
        .map(String::from)
        .collect();
    (v, keys)
}

#[library_benchmark]
#[bench::twitter(setup = parse_value, args = ["twitter.json"])]
fn lookup_twitter(v: Value) -> (usize, Value) {
    (black_box(twitter_lookups(black_box(&v))), v)
}

#[library_benchmark]
#[bench::citm(setup = parse_citm_keys, args = ["citm_catalog.json"])]
fn lookup_citm(v: (Value, Vec<String>)) -> (usize, (Value, Vec<String>)) {
    (black_box(citm_lookups(black_box(&v))), v)
}

#[library_benchmark]
#[bench::twitter(setup = parse_value, args = ["twitter.json"])]
fn encode_value_girders(v: Value) -> (String, Value) {
    (
        black_box(girders_json::encode(black_box(&v), &value()).unwrap()),
        v,
    )
}

fn parse_serde_value(name: &str) -> serde_json::Value {
    serde_json::from_str(&data(name)).unwrap()
}

#[library_benchmark]
#[bench::twitter(setup = parse_serde_value, args = ["twitter.json"])]
fn encode_value_serde(v: serde_json::Value) -> (String, serde_json::Value) {
    (black_box(serde_json::to_string(black_box(&v)).unwrap()), v)
}

library_benchmark_group!(
    name = value_ops,
    benchmarks = [
        lookup_twitter,
        lookup_citm,
        encode_value_girders,
        encode_value_serde
    ]
);

// -- CBOR: the same typed models in a binary format -----------------------------------

fn canada_cbor(name: &str) -> Vec<u8> {
    girders_cbor::encode(&canada_value(name), &canada::codec())
}
fn citm_cbor(name: &str) -> Vec<u8> {
    girders_cbor::encode(&citm_value(name), &citm::codec())
}
fn twitter_cbor(name: &str) -> Vec<u8> {
    girders_cbor::encode(&twitter_value(name), &twitter::codec())
}

#[library_benchmark]
#[bench::canada(setup = canada_cbor, args = ["canada.json"])]
fn decode_cbor_canada(bytes: Vec<u8>) -> canada::Canada {
    black_box(girders_cbor::decode(black_box(&bytes), &canada::codec()).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_cbor, args = ["citm_catalog.json"])]
fn decode_cbor_citm(bytes: Vec<u8>) -> citm::CitmCatalog {
    black_box(girders_cbor::decode(black_box(&bytes), &citm::codec()).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_cbor, args = ["twitter.json"])]
fn decode_cbor_twitter(bytes: Vec<u8>) -> twitter::Twitter {
    black_box(girders_cbor::decode(black_box(&bytes), &twitter::codec()).unwrap())
}

#[library_benchmark]
#[bench::canada(setup = canada_value, args = ["canada.json"])]
fn encode_cbor_canada(v: canada::Canada) -> Vec<u8> {
    black_box(girders_cbor::encode(black_box(&v), &canada::codec()))
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_cbor_citm(v: citm::CitmCatalog) -> Vec<u8> {
    black_box(girders_cbor::encode(black_box(&v), &citm::codec()))
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_cbor_twitter(v: twitter::Twitter) -> Vec<u8> {
    black_box(girders_cbor::encode(black_box(&v), &twitter::codec()))
}

/// cbor4ii's own encoding: it reads floats only at their declared width.
fn canada_cbor4ii(name: &str) -> Vec<u8> {
    cbor4ii::serde::to_vec(Vec::new(), &canada_value(name)).unwrap()
}
fn citm_cbor4ii(name: &str) -> Vec<u8> {
    cbor4ii::serde::to_vec(Vec::new(), &citm_value(name)).unwrap()
}
fn twitter_cbor4ii(name: &str) -> Vec<u8> {
    cbor4ii::serde::to_vec(Vec::new(), &twitter_value(name)).unwrap()
}

#[library_benchmark]
#[bench::canada(setup = canada_cbor, args = ["canada.json"])]
fn decode_ciborium_canada(bytes: Vec<u8>) -> canada::Canada {
    black_box(ciborium::from_reader(black_box(&bytes[..])).unwrap())
}

#[library_benchmark]
#[bench::canada(setup = canada_cbor4ii, args = ["canada.json"])]
fn decode_cbor4ii_canada(bytes: Vec<u8>) -> canada::Canada {
    black_box(cbor4ii::serde::from_slice(black_box(&bytes)).unwrap())
}

#[library_benchmark]
#[bench::canada(setup = canada_value, args = ["canada.json"])]
fn encode_ciborium_canada(v: canada::Canada) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(black_box(&v), &mut out).unwrap();
    black_box(out)
}

#[library_benchmark]
#[bench::canada(setup = canada_value, args = ["canada.json"])]
fn encode_cbor4ii_canada(v: canada::Canada) -> Vec<u8> {
    black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_cbor, args = ["citm_catalog.json"])]
fn decode_ciborium_citm(bytes: Vec<u8>) -> citm::CitmCatalog {
    black_box(ciborium::from_reader(black_box(&bytes[..])).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_cbor4ii, args = ["citm_catalog.json"])]
fn decode_cbor4ii_citm(bytes: Vec<u8>) -> citm::CitmCatalog {
    black_box(cbor4ii::serde::from_slice(black_box(&bytes)).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_ciborium_citm(v: citm::CitmCatalog) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(black_box(&v), &mut out).unwrap();
    black_box(out)
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_cbor4ii_citm(v: citm::CitmCatalog) -> Vec<u8> {
    black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_cbor, args = ["twitter.json"])]
fn decode_ciborium_twitter(bytes: Vec<u8>) -> twitter::Twitter {
    black_box(ciborium::from_reader(black_box(&bytes[..])).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_cbor4ii, args = ["twitter.json"])]
fn decode_cbor4ii_twitter(bytes: Vec<u8>) -> twitter::Twitter {
    black_box(cbor4ii::serde::from_slice(black_box(&bytes)).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_ciborium_twitter(v: twitter::Twitter) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(black_box(&v), &mut out).unwrap();
    black_box(out)
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_cbor4ii_twitter(v: twitter::Twitter) -> Vec<u8> {
    black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&v)).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = cbor_of, args = ["canada.json", "citm_catalog.json", "twitter.json"])]
fn decode_cbor_value(bytes: Vec<u8>) -> girders_cbor::Value {
    black_box(girders_cbor::decode(black_box(&bytes), &girders_cbor::value()).unwrap())
}

/// A corpus as CBOR, through the dynamic tree.
fn cbor_of(name: &str) -> Vec<u8> {
    let json = girders_json::parse(&data(name)).unwrap();
    girders_cbor::encode(&json, &value())
}

library_benchmark_group!(
    name = cbor,
    benchmarks = [
        decode_cbor_canada,
        decode_cbor_citm,
        decode_cbor_twitter,
        decode_cbor_value,
        encode_cbor_canada,
        encode_cbor_citm,
        encode_cbor_twitter,
        decode_ciborium_canada,
        decode_ciborium_citm,
        decode_ciborium_twitter,
        decode_cbor4ii_canada,
        decode_cbor4ii_citm,
        decode_cbor4ii_twitter,
        encode_ciborium_canada,
        encode_ciborium_citm,
        encode_ciborium_twitter,
        encode_cbor4ii_canada,
        encode_cbor4ii_citm,
        encode_cbor4ii_twitter
    ]
);

// -- unions: 10,000 mixed events ---------------------------------------------------

fn events_value(_: ()) -> Vec<events::Event> {
    events::sample()
}

fn events_json(_: ()) -> String {
    girders_json::encode(&events::sample(), &list(events::event())).unwrap()
}

#[library_benchmark]
#[bench::events(setup = events_value, args = [()])]
fn encode_events(v: Vec<events::Event>) -> String {
    black_box(girders_json::encode(black_box(&v), &list(events::event())).unwrap())
}

#[library_benchmark]
#[bench::events(setup = events_json, args = [()])]
fn decode_events(json: String) -> Vec<events::Event> {
    black_box(girders_json::decode(black_box(&json), &list(events::event())).unwrap())
}

library_benchmark_group!(name = unions, benchmarks = [encode_events, decode_events]);

main!(
    config = LibraryBenchmarkConfig::default().tool(Dhat::default()),
    library_benchmark_groups = [
        decode_typed,
        options,
        encode_typed,
        dynamic_value,
        streaming,
        value_ops,
        cbor,
        unions
    ]
);
