//! Property tests: round trips, streaming equivalence, accumulate ⊇
//! fail-fast, pointer escaping.

use girders::Number;
use girders::prelude::*;
use girders_json::{Node, Object, Pointer, StreamDecoder, Value, value};
use proptest::prelude::*;

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::null()),
        any::<bool>().prop_map(Value::bool),
        any::<i64>().prop_map(Value::int),
        any::<u64>().prop_map(|u| Value::new(Node::Number(Number::U64(u))).normalized()),
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Value::float),
        ".*".prop_map(Value::string),
    ];
    leaf.prop_recursive(5, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::array),
            prop::collection::vec((".{0,8}", inner), 0..6).prop_map(Value::object),
        ]
    })
}

trait Normalize {
    fn normalized(self) -> Self;
}
impl Normalize for Value {
    /// Small `U64`s parse back as `I64`.
    fn normalized(self) -> Self {
        match self.node {
            Node::Number(Number::U64(u)) if u <= i64::MAX as u64 => Value::int(u as i64),
            _ => self,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Rec {
    name: String,
    n: i32,
    tags: Vec<String>,
    opt: Option<u8>,
}

fn rec() -> impl Codec<Output = Rec> + Clone {
    record((
        field("name", string(), |r: &Rec| &r.name),
        field("n", i32(), |r: &Rec| &r.n),
        field("tags", list(string()), |r: &Rec| &r.tags),
        optional_field("opt", u8(), |r: &Rec| &r.opt),
    ))
    .build(|(name, n, tags, opt)| Rec { name, n, tags, opt })
}

fn arb_rec() -> impl Strategy<Value = Rec> {
    (
        ".*",
        any::<i32>(),
        prop::collection::vec(".*", 0..4),
        any::<Option<u8>>(),
    )
        .prop_map(|(name, n, tags, opt)| Rec { name, n, tags, opt })
}

proptest! {
    #[test]
    fn value_round_trip(v in arb_value()) {
        let json = girders_json::encode(&v, &value()).unwrap();
        let back: Value = girders_json::parse(&json).unwrap();
        prop_assert_eq!(back.without_spans(), v.clone());
        let pretty = girders_json::encode_with(&v, &value(), girders_json::EncodeOptions::new().pretty("  ")).unwrap();
        let back: Value = girders_json::parse(&pretty).unwrap();
        prop_assert_eq!(back.without_spans(), v);
    }

    #[test]
    fn record_round_trip(r in arb_rec()) {
        let json = girders_json::encode(&r, &rec()).unwrap();
        prop_assert_eq!(girders_json::decode(&json, &rec()).unwrap(), r);
    }

    /// Any chunking of any document gives the slice result, through both
    /// streaming paths.
    #[test]
    fn streaming_equals_slice(v in arb_value(), chunk in 1usize..40) {
        let json = girders_json::encode_with(&v, &value(), girders_json::EncodeOptions::new().pretty(" ")).unwrap();
        let a = girders_json::parse(&json).unwrap();
        let b = girders_json::from_reader_with(json.as_bytes(), &value(), DecodeOptions::new(), chunk).unwrap();
        prop_assert_eq!(&a, &b);
        let mut d = StreamDecoder::new(value());
        let mut out = Vec::new();
        for c in json.as_bytes().chunks(chunk) {
            d.push(c);
            while let Some(r) = d.next() { out.push(r.unwrap()); }
        }
        d.finish();
        while let Some(r) = d.next() { out.push(r.unwrap()); }
        prop_assert_eq!(out, vec![a]);
    }

    /// Accumulate mode reports the fail-fast error among its errors, and
    /// the same success/failure outcome.
    #[test]
    fn accumulate_contains_fail_fast(v in arb_value()) {
        let json = girders_json::encode(&v, &value()).unwrap();
        let c = list(rec());
        let fast = girders_json::decode(&json, &c);
        let all = girders_json::decode_with(&json, &c, DecodeOptions::strict());
        prop_assert_eq!(fast.is_ok(), all.is_ok());
        if let (Err(f), Err(a)) = (fast, all) {
            let first = f.first().clone();
            prop_assert!(a.iter().any(|d| *d == first), "fail-fast {:?} not in {:?}", first, a);
        }
    }

    #[test]
    fn pointer_round_trip(segs in prop::collection::vec(".*", 0..5)) {
        let mut p = Pointer::root();
        for s in &segs { p.push(s.clone()); }
        let parsed = Pointer::parse(&p.to_string()).unwrap();
        prop_assert_eq!(parsed.segments().collect::<Vec<_>>(), segs.iter().map(String::as_str).collect::<Vec<_>>());
    }
}

proptest! {
    /// An indexed `Object` answers lookups exactly like a linear scan
    /// for the last occurrence, around the index threshold and with
    /// duplicate keys, both when parsed and when built with `push`.
    #[test]
    fn ordered_index_matches_linear_scan(
        keys in prop::collection::vec("[a-e]{0,3}", 0..80),
        probes in prop::collection::vec("[a-f]{0,3}", 0..20),
    ) {
        let members: Vec<String> = keys.iter().enumerate().map(|(i, k)| format!("{:?}:{i}", k)).collect();
        let json = format!("{{{}}}", members.join(","));
        let parsed: Value = girders_json::parse(&json).unwrap();
        let mut pushed = Object::default();
        for (i, k) in keys.iter().enumerate() {
            pushed.push(k.clone(), Value::int(i as i64));
        }
        let Node::Object(map) = &parsed.node else { unreachable!() };
        for probe in keys.iter().chain(probes.iter()) {
            let expected = keys.iter().rposition(|k| k == probe).map(|i| i as i64);
            let got = map.get(probe).and_then(|v| match &v.node { Node::Number(n) => n.as_i64(), _ => None });
            prop_assert_eq!(got, expected, "parsed, key {:?}", probe);
            let got = pushed.get(probe).and_then(|v| match &v.node { Node::Number(n) => n.as_i64(), _ => None });
            prop_assert_eq!(got, expected, "pushed, key {:?}", probe);
        }
    }
}
