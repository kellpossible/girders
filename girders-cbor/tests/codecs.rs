//! Typed codecs over CBOR: exact bytes for records and numbers, the same
//! codec across JSON and CBOR, and round trips of arbitrary values.

use girders::prelude::*;
use girders_cbor::{Value, decode, encode, value};
use proptest::prelude::*;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
struct Reading {
    sensor: u32,
    label: String,
    celsius: f32,
    raw: Vec<u8>,
    note: Option<String>,
}

fn reading() -> impl Codec<Output = Reading> {
    record((
        field("sensor", u32(), |r: &Reading| &r.sensor),
        field("label", string(), |r: &Reading| &r.label),
        field("celsius", f32(), |r: &Reading| &r.celsius),
        field("raw", bytes(), |r: &Reading| &r.raw),
        optional_field("note", string(), |r: &Reading| &r.note).skip_none(),
    ))
    .build(|(sensor, label, celsius, raw, note)| Reading {
        sensor,
        label,
        celsius,
        raw,
        note,
    })
}

fn sample() -> Reading {
    Reading {
        sensor: 7,
        label: "t".into(),
        celsius: 21.5,
        raw: vec![1, 2],
        note: None,
    }
}

#[test]
fn records_are_maps_with_text_keys_and_shortest_numbers() {
    let bytes = encode(&sample(), &reading());
    let expected = concat!(
        "a4", // map of 4 (`note` skipped)
        "6673656e736f72",
        "07", // "sensor": 7
        "656c6162656c",
        "6174", // "label": "t"
        "6763656c73697573",
        "f94d60", // "celsius": 21.5 as a half float
        "63726177",
        "420102", // "raw": h'0102'
    );
    assert_eq!(bytes, hex(&expected.replace(' ', "")));
    assert_eq!(decode(&bytes, &reading()).unwrap(), sample());
}

#[test]
fn the_same_codec_reads_both_formats() {
    let v = Reading {
        note: Some("ok".into()),
        ..sample()
    };
    let json = girders_json::encode(&v, &reading()).unwrap();
    assert_eq!(
        json,
        r#"{"sensor":7,"label":"t","celsius":21.5,"raw":[1,2],"note":"ok"}"#
    );
    let from_json = girders_json::decode(&json, &reading()).unwrap();
    let cbor = encode(&from_json, &reading());
    assert_eq!(decode(&cbor, &reading()).unwrap(), v);
}

#[test]
fn unknown_keys_and_wrong_types_are_reported_with_offsets() {
    let mut map = decode(&encode(&sample(), &reading()), &value()).unwrap();
    let Value::Map(entries) = &mut map else {
        panic!()
    };
    entries.push((Value::Text("extra".into()), Value::Null));
    let err = decode(&encode(&map, &value()), &reading()).unwrap_err();
    assert!(err.to_string().contains("extra"), "{err}");

    let err = decode(
        &hex("a1667365 6e736f72 f93e00".replace(' ', "").as_str()),
        &reading(),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("expected int, found float"),
        "{err}"
    );
}

#[test]
fn integers_keep_full_precision() {
    // u64::MAX + 1 is a bignum (tag 2); the most negative CBOR integer is not.
    let big: u128 = u64::MAX as u128 + 1;
    assert_eq!(encode(&big, &u128()), hex("c249010000000000000000"));
    assert_eq!(
        decode(&hex("c249010000000000000000"), &u128()).unwrap(),
        big
    );
    let low: i128 = -(u64::MAX as i128) - 1;
    assert_eq!(encode(&low, &i128()), hex("3bffffffffffffffff"));
    assert_eq!(decode(&hex("3bffffffffffffffff"), &i128()).unwrap(), low);
    assert_eq!(
        decode(&hex("c349010000000000000000"), &i128()).unwrap(),
        low - 1
    );
    for v in [i128::MIN, i128::MAX, 0, -1] {
        assert_eq!(decode(&encode(&v, &i128()), &i128()).unwrap(), v);
    }
    // Out of range is an error, not a wrap.
    assert!(decode(&hex("3bffffffffffffffff"), &i64()).is_err());
    assert!(decode(&hex("1bffffffffffffffff"), &i64()).is_err());
    assert_eq!(
        decode(&hex("1bffffffffffffffff"), &u64()).unwrap(),
        u64::MAX
    );
}

#[test]
fn decimal_fractions_are_exact_numbers() {
    // 273.15 as tag 4 [-2, 27315], RFC 8949 Section 3.4.4.
    let bytes = hex("c48221196ab3");
    assert_eq!(decode(&bytes, &f64()).unwrap(), 273.15);
    assert_eq!(
        decode(&bytes, &value()).unwrap(),
        Value::Tag(
            4,
            Box::new(Value::Array(vec![
                Value::Integer(-2),
                Value::Integer(27315)
            ]))
        )
    );
}

#[test]
fn floats_use_the_shortest_exact_width() {
    assert_eq!(encode(&1.5f32, &f32()), hex("f93e00"));
    assert_eq!(encode(&100000.0f32, &f32()), hex("fa47c35000"));
    assert_eq!(encode(&1.1f64, &f64()), hex("fb3ff199999999999a"));
    assert_eq!(decode(&hex("f93e00"), &f32()).unwrap(), 1.5);
    assert!(
        decode(&hex("fb3ff199999999999a"), &f32()).is_ok(),
        "f64 -> f32 narrows within range"
    );
    assert!(
        decode(&hex("fb7e37e43c8800759c"), &f32()).is_err(),
        "1e300 does not fit f32"
    );
}

#[test]
fn tags_are_checked_where_the_format_has_them() {
    let epoch = cbor_tag(1, u64());
    let bytes = encode(&1363896240, &epoch);
    assert_eq!(bytes, hex("c11a514b67b0"));
    assert_eq!(decode(&bytes, &epoch).unwrap(), 1363896240);
    let err = decode(&hex("1a514b67b0"), &epoch).unwrap_err();
    assert!(err.to_string().contains("expected tag"), "{err}");
    let err = decode(&hex("c01a514b67b0"), &epoch).unwrap_err();
    assert!(err.to_string().contains("tag 1"), "{err}");
    // JSON has no tags: the tag is dropped and not required.
    assert_eq!(girders_json::encode(&5, &epoch).unwrap(), "5");
    assert_eq!(girders_json::decode("5", &epoch).unwrap(), 5);
}

#[test]
fn values_with_integer_keys_convert_to_json() {
    let v = decode(&hex("a201020304"), &value()).unwrap();
    assert_eq!(v.to_string(), "{1: 2, 3: 4}");
    assert_eq!(
        girders_json::encode(&v, &value()).unwrap(),
        r#"{"1":2,"3":4}"#
    );
    let bad = Value::Map(vec![(Value::Array(vec![]), Value::Null)]);
    assert!(girders_json::encode(&bad, &value()).is_err());
}

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        any::<i128>().prop_map(Value::Integer),
        any::<f64>().prop_map(Value::Float),
        any::<Vec<u8>>().prop_map(Value::Bytes),
        any::<String>().prop_map(Value::Text),
        any::<bool>().prop_map(Value::Bool),
        Just(Value::Null),
        Just(Value::Undefined),
        // 20..=23 are false, true, null and undefined; 24..=31 are reserved.
        prop_oneof![0u8..=19, 32u8..=255].prop_map(Value::Simple),
    ];
    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::vec((inner.clone(), inner.clone()), 0..8).prop_map(Value::Map),
            // Tags 2 and 3 over small byte strings decode as integers.
            (4u64..u64::MAX, inner).prop_map(|(t, v)| Value::Tag(t, Box::new(v))),
        ]
    })
}

proptest! {
    #[test]
    fn values_round_trip(v in arb_value()) {
        let bytes = encode(&v, &value());
        prop_assert_eq!(decode(&bytes, &value()).unwrap(), v);
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in any::<Vec<u8>>()) {
        let _ = decode(&bytes, &value());
    }
}
