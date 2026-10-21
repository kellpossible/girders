//! Behaviour that differs between encoders, or between JSON and CBOR, and
//! that a girders model must handle consistently: absent versus `null`
//! optionals, float narrowing, input written by other encoders
//! (indefinite lengths, non-shortest encodings), unions and errors.

use girders::prelude::*;
use girders::{DecodeOptions, Problem};
use girders_cbor::{Value, decode, decode_with, encode, value};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
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
        optional_field("note", string(), |r: &Reading| &r.note),
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

// -- absent versus null -------------------------------------------------------------

#[test]
fn none_is_written_as_null_and_read_from_null_or_absence() {
    // The same choice as JSON (and serde): `None` is `null` unless the
    // field says `skip_none`.
    let bytes = encode(&sample(), &reading());
    let tree = decode(&bytes, &value()).unwrap();
    assert!(tree.to_string().ends_with(r#""note": null}"#), "{tree}");

    let Value::Map(mut entries) = tree else {
        panic!()
    };
    entries.pop();
    let absent = encode(&Value::Map(entries.clone()), &value());
    assert_eq!(decode(&absent, &reading()).unwrap(), sample());

    entries.push((Value::Text("note".into()), Value::Text("hi".into())));
    let present = encode(&Value::Map(entries), &value());
    assert_eq!(
        decode(&present, &reading()).unwrap().note.as_deref(),
        Some("hi")
    );
}

#[test]
fn undefined_is_not_null() {
    // CBOR's `undefined` is its own simple value; an optional field
    // rejects it rather than reading it as absent.
    let mut tree = decode(&encode(&sample(), &reading()), &value()).unwrap();
    let Value::Map(entries) = &mut tree else {
        panic!()
    };
    entries.last_mut().unwrap().1 = Value::Undefined;
    let err = decode(&encode(&tree, &value()), &reading()).unwrap_err();
    assert!(err.to_string().contains("simple value"), "{err}");
}

// -- float widths -------------------------------------------------------------------

#[test]
fn f32_fields_narrow_the_same_way_from_json_and_cbor() {
    // A double that isn't exact in f32 rounds to the nearest f32 in both
    // formats, and is then written as a single-precision float.
    let from_cbor = decode(&hex("fb 3ff199999999999a"), &f32()).unwrap();
    let from_json = girders_json::decode("1.1", &f32()).unwrap();
    assert_eq!(from_cbor, 1.1f32);
    assert_eq!(from_cbor.to_bits(), from_json.to_bits());
    assert_eq!(encode(&from_cbor, &f32()), hex("fa 3f8ccccd"));
    // Half and single precision widen exactly.
    assert_eq!(decode(&hex("f9 3e00"), &f64()).unwrap(), 1.5);
    assert_eq!(decode(&hex("fa 3f8ccccd"), &f64()).unwrap(), 1.1f32 as f64);
}

#[test]
fn integers_read_as_floats_and_floats_never_as_integers() {
    assert_eq!(decode(&hex("18 64"), &f64()).unwrap(), 100.0);
    assert_eq!(decode(&hex("38 63"), &f64()).unwrap(), -100.0);
    let err = decode(&hex("f9 3c00"), &u8()).unwrap_err();
    assert!(
        err.to_string().contains("expected int, found float"),
        "{err}"
    );
}

// -- input from other encoders --------------------------------------------------------

#[test]
fn indefinite_lengths_and_long_heads_decode_like_preferred_ones() {
    // What a streaming encoder might write for `sample()`: an indefinite
    // map, the integer 7 in a two-byte head, an indefinite text string,
    // 21.5 as a double, and the bytes in two chunks.
    let streamed = hex("bf
         66 73656e736f72 1807
         65 6c6162656c 7f 6174 ff
         67 63656c73697573 fb 4035800000000000
         63 726177 5f 4101 4102 ff
         ff");
    assert_eq!(
        decode(&streamed, &reading()).unwrap(),
        Reading {
            note: None,
            ..sample()
        }
    );
    // Re-encoding gives the preferred form.
    let preferred = encode(&sample(), &reading());
    assert!(preferred.len() < streamed.len());
    assert_eq!(decode(&preferred, &reading()).unwrap(), sample());
}

#[test]
fn entries_may_come_in_any_order_but_not_twice() {
    let tree = decode(&encode(&sample(), &reading()), &value()).unwrap();
    let Value::Map(mut entries) = tree else {
        panic!()
    };
    entries.reverse();
    assert_eq!(
        decode(&encode(&Value::Map(entries.clone()), &value()), &reading()).unwrap(),
        sample()
    );

    entries.push(entries[0].clone());
    let err = decode(&encode(&Value::Map(entries), &value()), &reading()).unwrap_err();
    assert!(matches!(err.problem(), Problem::DuplicateField(_)), "{err}");
}

#[test]
fn non_text_keys_are_rejected_by_records() {
    // {1: 2}: records only have text keys.
    let err = decode(&hex("a1 01 02"), &reading()).unwrap_err();
    assert!(
        err.to_string()
            .contains("expected string key, found number"),
        "{err}"
    );
}

// -- unions ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Reading(Reading),
    Reset(u32),
}

fn event() -> impl Codec<Output = Event> {
    tagged_union(
        "type",
        (
            variant("reading", reading(), Event::Reading),
            variant(
                "reset",
                record((field("sensor", u32(), |s: &u32| s),)).build(|(s,)| s),
                Event::Reset,
            ),
        ),
        |e| match e {
            Event::Reading(r) => Pick2::Variant0(r),
            Event::Reset(s) => Pick2::Variant1(s),
        },
    )
}

#[test]
fn tagged_unions_count_the_tag_in_the_map_length() {
    let bytes = encode(&Event::Reset(3), &event());
    // {"type": "reset", "sensor": 3}: a definite map of 2 entries.
    assert_eq!(bytes, hex("a2 6474797065 657265736574 6673656e736f72 03"));
    assert_eq!(decode(&bytes, &event()).unwrap(), Event::Reset(3));
    let r = Event::Reading(sample());
    assert_eq!(decode(&encode(&r, &event()), &event()).unwrap(), r);
}

// -- errors ------------------------------------------------------------------------

#[test]
fn accumulated_errors_point_at_byte_offsets() {
    // {"sensor": -1, "label": 5, ...}: two bad fields.
    let tree = Value::Map(vec![
        (Value::Text("sensor".into()), Value::Integer(-1)),
        (Value::Text("label".into()), Value::Integer(5)),
        (Value::Text("celsius".into()), Value::Float(1.0)),
        (Value::Text("raw".into()), Value::Bytes(vec![])),
    ]);
    let bytes = encode(&tree, &value());
    let err = decode_with(&bytes, &reading(), DecodeOptions::all_errors()).unwrap_err();
    assert_eq!(err.len(), 2, "{err}");
    let offsets: Vec<_> = err.iter().map(|e| e.location.unwrap().start()).collect();
    // After the map head (1 byte) and "sensor" (7 bytes); then after -1
    // (1 byte) and "label" (6 bytes).
    assert_eq!(offsets, [8, 15]);
}

#[test]
fn truncated_records_are_syntax_errors() {
    let bytes = encode(&sample(), &reading());
    for cut in [1, bytes.len() / 2, bytes.len() - 1] {
        let err = decode(&bytes[..cut], &reading()).unwrap_err();
        assert!(
            matches!(err.problem(), Problem::Format(_)),
            "cut at {cut}: {err}"
        );
    }
}

#[test]
fn deep_nesting_hits_the_depth_limit_not_the_stack() {
    let mut deep = vec![0x81; 10_000];
    deep.push(0x00);
    let err = decode(&deep, &value()).unwrap_err();
    assert!(
        matches!(err.problem(), Problem::DepthLimitExceeded(_)),
        "{err}"
    );
    // Skipping (an unknown field) is iterative and has no limit to hit.
    let mut entries = vec![(Value::Text("x".into()), Value::Null)];
    entries.extend(
        decode(&encode(&sample(), &reading()), &value())
            .map(|v| match v {
                Value::Map(e) => e,
                _ => unreachable!(),
            })
            .unwrap(),
    );
    let mut input = encode(&Value::Map(entries), &value());
    // Replace the `null` value of "x" (right after the key) with the deep array.
    let at = input.iter().position(|&b| b == 0xf6).unwrap();
    input.splice(at..=at, deep);
    let allow = record((
        field("sensor", u32(), |r: &Reading| &r.sensor),
        field("label", string(), |r: &Reading| &r.label),
        field("celsius", f32(), |r: &Reading| &r.celsius),
        field("raw", bytes(), |r: &Reading| &r.raw),
        optional_field("note", string(), |r: &Reading| &r.note),
    ))
    .build(|(sensor, label, celsius, raw, note)| Reading {
        sensor,
        label,
        celsius,
        raw,
        note,
    })
    .allow_unknown();
    assert_eq!(decode(&input, &allow).unwrap(), sample());
}

// -- dynamic values across formats -------------------------------------------------------

#[test]
fn bignums_beyond_i128_stay_tagged_and_round_trip() {
    // 2^128 is 17 bytes: past i128, so it stays tag 2 over its bytes.
    let mut magnitude = vec![1];
    magnitude.extend([0; 16]);
    let big = Value::Tag(2, Box::new(Value::Bytes(magnitude)));
    let bytes = encode(&big, &value());
    assert_eq!(decode(&bytes, &value()).unwrap(), big);
    // Typed codecs reject it as out of range rather than truncating.
    assert!(matches!(
        decode(&bytes, &u128()).unwrap_err().problem(),
        Problem::OutOfRange(_)
    ));
}

#[test]
fn json_values_round_trip_through_cbor() {
    let json = r#"{"name":"水","n":[1,-2,18446744073709551615,1.5,1e300],"ok":true,"none":null,"nested":{"a":[]}}"#;
    let v = girders_json::parse(json).unwrap();
    let codec = girders_json::value();
    let bytes = encode(&v, &codec);
    let back = decode(&bytes, &codec).unwrap();
    assert_eq!(girders_json::encode(&back, &codec).unwrap(), json);
    // And the CBOR tree sees the same data.
    assert_eq!(
        decode(&bytes, &value()).unwrap().to_string(),
        r#"{"name": "水", "n": [1, -2, 18446744073709551615, 1.5, 1e300], "ok": true, "none": null, "nested": {"a": []}}"#
    );
}
