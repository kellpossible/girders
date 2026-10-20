//! Runtime benchmarks (gungraun: Callgrind instruction counts, plus DHAT
//! heap profiles). girders vs serde_json on the json-benchmark corpus,
//! and girders' own cost knobs.
//!
//! Run: `mise run bench` (needs valgrind and gungraun-runner 0.20.0).

use girders::DecodeOptions;
use girders_benches::{Indexed, Linear, canada, citm, data, twitter};
use girders_json::{Hashed, Ordered, Sorted, StreamDecoder, Value, value};
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
    black_box(girders_json::encode(black_box(&v), &canada::codec()))
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_serde_citm(v: citm::CitmCatalog) -> String {
    black_box(serde_json::to_string(black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_girders_citm(v: citm::CitmCatalog) -> String {
    black_box(girders_json::encode(black_box(&v), &citm::codec()))
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_serde_twitter(v: twitter::Twitter) -> String {
    black_box(serde_json::to_string(black_box(&v)).unwrap())
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_girders_twitter(v: twitter::Twitter) -> String {
    black_box(girders_json::encode(black_box(&v), &twitter::codec()))
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
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json"])]
fn value_serde(json: String) -> serde_json::Value {
    black_box(serde_json::from_str(black_box(&json)).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json"])]
fn value_ordered(json: String) -> Value<Ordered> {
    black_box(girders_json::parse::<Ordered>(black_box(&json)).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json"])]
fn value_ordered_fast(json: String) -> Value<Ordered> {
    black_box(
        girders_json::decode_with(black_box(&json), &value::<Ordered>(), DecodeOptions::fast())
            .unwrap(),
    )
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json"])]
fn value_sorted(json: String) -> Value<Sorted> {
    black_box(girders_json::parse::<Sorted>(black_box(&json)).unwrap())
}

#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json"])]
fn value_hashed(json: String) -> Value<Hashed> {
    black_box(girders_json::parse::<Hashed>(black_box(&json)).unwrap())
}

library_benchmark_group!(
    name = dynamic_value,
    benchmarks = [
        value_serde,
        value_ordered,
        value_ordered_fast,
        value_sorted,
        value_hashed
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

// -- dyn serializer (code-size experiment) ---------------------------------------------

#[library_benchmark]
#[bench::canada(setup = canada_value, args = ["canada.json"])]
fn encode_dyn_canada(v: canada::Canada) -> String {
    black_box(girders_json::encode_dyn(black_box(&v), &canada::codec()))
}

#[library_benchmark]
#[bench::citm(setup = citm_value, args = ["citm_catalog.json"])]
fn encode_dyn_citm(v: citm::CitmCatalog) -> String {
    black_box(girders_json::encode_dyn(black_box(&v), &citm::codec()))
}

#[library_benchmark]
#[bench::twitter(setup = twitter_value, args = ["twitter.json"])]
fn encode_dyn_twitter(v: twitter::Twitter) -> String {
    black_box(girders_json::encode_dyn(black_box(&v), &twitter::codec()))
}

library_benchmark_group!(
    name = dyn_dispatch,
    benchmarks = [encode_dyn_canada, encode_dyn_citm, encode_dyn_twitter]
);

// -- Value representations: lookups and encoding -------------------------------------

use girders_json::{Node, ObjectMap};

/// Every status's `user.screen_name`, `id` and `text`, and
/// `search_metadata.count`: typical shallow lookups in small objects.
fn twitter_lookups<M: ObjectMap>(v: &Value<M>) -> usize {
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
/// large object.
fn citm_lookups<M: ObjectMap>(input: &(Value<M>, Vec<String>)) -> usize {
    let (v, keys) = input;
    let events = v.get("events").unwrap();
    keys.iter().filter(|k| events.get(k).is_some()).count()
}

fn parse_twitter<M: ObjectMap>(name: &str) -> Value<M> {
    girders_json::parse::<M>(&data(name)).unwrap()
}

fn parse_citm_keys<M: ObjectMap>(name: &str) -> (Value<M>, Vec<String>) {
    let v = girders_json::parse::<M>(&data(name)).unwrap();
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
#[bench::twitter(setup = parse_twitter::<Ordered>, args = ["twitter.json"])]
fn lookup_twitter_ordered(v: Value<Ordered>) -> (usize, Value<Ordered>) {
    (black_box(twitter_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Sorted>, args = ["twitter.json"])]
fn lookup_twitter_sorted(v: Value<Sorted>) -> (usize, Value<Sorted>) {
    (black_box(twitter_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Hashed>, args = ["twitter.json"])]
fn lookup_twitter_hashed(v: Value<Hashed>) -> (usize, Value<Hashed>) {
    (black_box(twitter_lookups(black_box(&v))), v)
}

#[library_benchmark]
#[bench::citm(setup = parse_citm_keys::<Ordered>, args = ["citm_catalog.json"])]
fn lookup_citm_ordered(v: (Value<Ordered>, Vec<String>)) -> (usize, (Value<Ordered>, Vec<String>)) {
    (black_box(citm_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::citm(setup = parse_citm_keys::<Sorted>, args = ["citm_catalog.json"])]
fn lookup_citm_sorted(v: (Value<Sorted>, Vec<String>)) -> (usize, (Value<Sorted>, Vec<String>)) {
    (black_box(citm_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::citm(setup = parse_citm_keys::<Hashed>, args = ["citm_catalog.json"])]
fn lookup_citm_hashed(v: (Value<Hashed>, Vec<String>)) -> (usize, (Value<Hashed>, Vec<String>)) {
    (black_box(citm_lookups(black_box(&v))), v)
}

#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Ordered>, args = ["twitter.json"])]
fn encode_value_ordered(v: Value<Ordered>) -> (String, Value<Ordered>) {
    (
        black_box(girders_json::encode(black_box(&v), &value::<Ordered>())),
        v,
    )
}
#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Sorted>, args = ["twitter.json"])]
fn encode_value_sorted(v: Value<Sorted>) -> (String, Value<Sorted>) {
    (
        black_box(girders_json::encode(black_box(&v), &value::<Sorted>())),
        v,
    )
}
#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Hashed>, args = ["twitter.json"])]
fn encode_value_hashed(v: Value<Hashed>) -> (String, Value<Hashed>) {
    (
        black_box(girders_json::encode(black_box(&v), &value::<Hashed>())),
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

#[library_benchmark]
#[bench::citm(setup = load, args = ["citm_catalog.json"])]
fn value_citm_serde(json: String) -> serde_json::Value {
    black_box(serde_json::from_str(black_box(&json)).unwrap())
}
#[library_benchmark]
#[bench::citm(setup = load, args = ["citm_catalog.json"])]
fn value_citm_ordered(json: String) -> Value<Ordered> {
    black_box(girders_json::parse::<Ordered>(black_box(&json)).unwrap())
}
#[library_benchmark]
#[bench::citm(setup = load, args = ["citm_catalog.json"])]
fn value_citm_sorted(json: String) -> Value<Sorted> {
    black_box(girders_json::parse::<Sorted>(black_box(&json)).unwrap())
}
#[library_benchmark]
#[bench::citm(setup = load, args = ["citm_catalog.json"])]
fn value_citm_hashed(json: String) -> Value<Hashed> {
    black_box(girders_json::parse::<Hashed>(black_box(&json)).unwrap())
}

library_benchmark_group!(
    name = value_ops,
    benchmarks = [
        lookup_twitter_ordered,
        lookup_twitter_sorted,
        lookup_twitter_hashed,
        lookup_citm_ordered,
        lookup_citm_sorted,
        lookup_citm_hashed,
        encode_value_ordered,
        encode_value_sorted,
        encode_value_hashed,
        encode_value_serde,
        value_citm_serde,
        value_citm_ordered,
        value_citm_sorted,
        value_citm_hashed
    ]
);

#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Indexed>, args = ["twitter.json"])]
fn lookup_twitter_indexed(v: Value<Indexed>) -> (usize, Value<Indexed>) {
    (black_box(twitter_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::citm(setup = parse_citm_keys::<Indexed>, args = ["citm_catalog.json"])]
fn lookup_citm_indexed(v: (Value<Indexed>, Vec<String>)) -> (usize, (Value<Indexed>, Vec<String>)) {
    (black_box(citm_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Indexed>, args = ["twitter.json"])]
fn encode_value_indexed(v: Value<Indexed>) -> (String, Value<Indexed>) {
    (
        black_box(girders_json::encode(black_box(&v), &value::<Indexed>())),
        v,
    )
}
#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_indexed(json: String) -> Value<Indexed> {
    black_box(girders_json::parse::<Indexed>(black_box(&json)).unwrap())
}

library_benchmark_group!(
    name = value_indexed_group,
    benchmarks = [
        lookup_twitter_indexed,
        lookup_citm_indexed,
        encode_value_indexed,
        value_indexed
    ]
);

#[library_benchmark]
#[bench::twitter(setup = parse_twitter::<Linear>, args = ["twitter.json"])]
fn lookup_twitter_linear(v: Value<Linear>) -> (usize, Value<Linear>) {
    (black_box(twitter_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[bench::citm(setup = parse_citm_keys::<Linear>, args = ["citm_catalog.json"])]
fn lookup_citm_linear(v: (Value<Linear>, Vec<String>)) -> (usize, (Value<Linear>, Vec<String>)) {
    (black_box(citm_lookups(black_box(&v))), v)
}
#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_linear(json: String) -> Value<Linear> {
    black_box(girders_json::parse::<Linear>(black_box(&json)).unwrap())
}
#[library_benchmark]
#[benches::corpus(setup = load, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_ordered_all(json: String) -> Value<Ordered> {
    black_box(girders_json::parse::<Ordered>(black_box(&json)).unwrap())
}

library_benchmark_group!(
    name = value_linear_group,
    benchmarks = [
        lookup_twitter_linear,
        lookup_citm_linear,
        value_linear,
        value_ordered_all
    ]
);

fn parse_key_spans(name: &str) -> String {
    data(name)
}

#[library_benchmark]
#[benches::corpus(setup = parse_key_spans, args = ["canada.json", "twitter.json", "citm_catalog.json"])]
fn value_ordered_key_spans(json: String) -> Value<Ordered> {
    black_box(
        girders_json::decode_with(
            black_box(&json),
            &value::<Ordered>(),
            DecodeOptions::new().key_spans(true),
        )
        .unwrap(),
    )
}

library_benchmark_group!(
    name = value_key_spans_group,
    benchmarks = [value_ordered_key_spans]
);

main!(
    config = LibraryBenchmarkConfig::default().tool(Dhat::default()),
    library_benchmark_groups = [
        decode_typed,
        options,
        encode_typed,
        dynamic_value,
        streaming,
        dyn_dispatch,
        value_ops,
        value_indexed_group,
        value_linear_group,
        value_key_spans_group
    ]
);
