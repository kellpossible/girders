//! Typed map keys, fixed-length arrays and numbers in strings, in both
//! JSON and CBOR.

use girders::prelude::*;
use girders::{DecodeOptions, Problem};
use std::collections::BTreeMap;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn json<C: Decoder>(codec: &C, text: &str) -> Result<C::Output, girders_json::Error> {
    girders_json::decode(text, codec)
}

// -- map keys ------------------------------------------------------------------------

fn names() -> BTreeMap<u32, String> {
    BTreeMap::from([(1, "a".into()), (2, "b".into())])
}

#[test]
fn integer_keys_are_quoted_in_json_and_native_in_cbor() {
    let codec = btree_map(u32(), string());
    assert_eq!(json(&codec, r#"{"2":"b","1":"a"}"#).unwrap(), names());
    assert_eq!(
        girders_json::encode(&names(), &codec).unwrap(),
        r#"{"1":"a","2":"b"}"#
    );

    let cbor = girders_cbor::encode(&names(), &codec);
    assert_eq!(cbor, hex("a2 01 6161 02 6162"));
    assert_eq!(girders_cbor::decode(&cbor, &codec).unwrap(), names());
}

#[test]
fn json_keys_follow_the_number_grammar_like_serde_json() {
    let codec = btree_map(i64(), u8());
    assert_eq!(
        json(&codec, r#"{"-7":1}"#).unwrap(),
        BTreeMap::from([(-7, 1)])
    );
    for bad in [
        r#"{"007":1}"#,
        r#"{"+1":1}"#,
        r#"{" 1":1}"#,
        r#"{"1.0":1}"#,
        r#"{"one":1}"#,
    ] {
        assert!(json(&codec, bad).is_err(), "{bad}");
    }
    // The error points at the key.
    let err = json(&codec, r#"{"1":1, "x":2}"#).unwrap_err();
    assert_eq!(err.location().unwrap().start(), 8, "{err}");

    let float_keys = btree_map(f64_key(), u8());
    assert_eq!(
        girders_json::encode(&BTreeMap::from([(Ordered(1.5), 1)]), &float_keys).unwrap(),
        r#"{"1.5":1}"#
    );
    assert!(
        json(&float_keys, r#"{"NaN":1}"#).is_err(),
        "no NaN keys, as serde_json"
    );
}

/// `f64` isn't `Ord`; a wrapper that is, for float keys.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ordered(f64);
impl Eq for Ordered {}
impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

fn f64_key() -> impl Codec<Output = Ordered> {
    bimap(f64(), Ordered, |o: &Ordered| o.0)
}

#[test]
fn bool_and_enum_keys() {
    let flags = btree_map(bool(), u8());
    let v = BTreeMap::from([(false, 0), (true, 1)]);
    assert_eq!(
        girders_json::encode(&v, &flags).unwrap(),
        r#"{"false":0,"true":1}"#
    );
    assert_eq!(json(&flags, r#"{"true":1,"false":0}"#).unwrap(), v);
    assert_eq!(
        girders_cbor::decode(&girders_cbor::encode(&v, &flags), &flags).unwrap(),
        v
    );

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Color {
        Red,
        Green,
    }
    let colors = btree_map(
        string_enum([("red", Color::Red), ("green", Color::Green)]),
        u8(),
    );
    let v = BTreeMap::from([(Color::Red, 1), (Color::Green, 2)]);
    assert_eq!(
        girders_json::encode(&v, &colors).unwrap(),
        r#"{"red":1,"green":2}"#
    );
    assert_eq!(json(&colors, r#"{"green":2,"red":1}"#).unwrap(), v);
    assert!(json(&colors, r#"{"blue":3}"#).is_err());
}

#[test]
fn composite_keys_work_in_cbor_only() {
    let codec = btree_map(tuple2(u8(), u8()), string());
    let v = BTreeMap::from([((1, 2), "x".to_string())]);
    let cbor = girders_cbor::encode(&v, &codec);
    assert_eq!(cbor, hex("a1 82 01 02 6178"));
    assert_eq!(girders_cbor::decode(&cbor, &codec).unwrap(), v);
    assert!(girders_json::encode(&v, &codec).is_err());
    let err = json(&codec, r#"{"1,2":"x"}"#).unwrap_err();
    assert!(err.to_string().contains("expected array"), "{err}");
}

#[test]
fn duplicate_keys_follow_the_policy() {
    let text = r#"{"1":"a","1":"b"}"#;
    let err = json(&btree_map(u32(), string()), text).unwrap_err();
    assert_eq!(err.problem(), &Problem::DuplicateField("1".into()));
    let last = btree_map(u32(), string()).duplicates(Duplicates::LastWins);
    assert_eq!(json(&last, text).unwrap()[&1], "b");
    let first = btree_map(u32(), string()).duplicates(Duplicates::FirstWins);
    assert_eq!(json(&first, text).unwrap()[&1], "a");
    // {1: "a", 1: "b"} in CBOR.
    let err =
        girders_cbor::decode(&hex("a2 01 6161 01 6162"), &btree_map(u32(), string())).unwrap_err();
    assert_eq!(err.problem(), &Problem::DuplicateField("1".into()));
}

#[test]
fn value_errors_name_their_key_in_the_path() {
    let err = json(&btree_map(u32(), u8()), r#"{"7":300}"#).unwrap_err();
    assert_eq!(girders_json::to_pointer(&err).to_string(), "/7");
    let err = girders_cbor::decode(&hex("a1 07 19 012c"), &btree_map(u32(), u8())).unwrap_err();
    assert_eq!(err.path(), [PathSegment::Key("7".into())]);
}

#[test]
fn accumulate_mode_reports_bad_keys_and_values_and_keeps_the_rest() {
    let codec = btree_map(u32(), u8());
    let err =
        girders_json::decode_with(r#"{"x":1,"2":300,"3":3}"#, &codec, DecodeOptions::strict())
            .unwrap_err();
    assert_eq!(err.len(), 2, "{err}");
}

// -- arrays --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Pixel {
    rgb: [u8; 3],
    position: [f64; 2],
}

fn pixel() -> impl Codec<Output = Pixel> {
    record((
        field("rgb", array(u8()), |p: &Pixel| &p.rgb),
        field("position", array(f64()), |p: &Pixel| &p.position),
    ))
    .build(|(rgb, position)| Pixel { rgb, position })
}

#[test]
fn array_lengths_are_inferred_and_exact() {
    let p = Pixel {
        rgb: [1, 2, 3],
        position: [0.5, -1.0],
    };
    let text = r#"{"rgb":[1,2,3],"position":[0.5,-1.0]}"#;
    assert_eq!(json(&pixel(), text).unwrap(), p);
    assert_eq!(girders_json::encode(&p, &pixel()).unwrap(), text);
    assert_eq!(
        girders_cbor::decode(&girders_cbor::encode(&p, &pixel()), &pixel()).unwrap(),
        p
    );

    let err = json(&pixel(), r#"{"rgb":[1,2],"position":[0,0]}"#).unwrap_err();
    assert!(
        err.to_string().contains("expected 3 elements, found 2"),
        "{err}"
    );
    let err = json(&pixel(), r#"{"rgb":[1,2,3,4],"position":[0,0]}"#).unwrap_err();
    assert!(err.to_string().contains("more than 3"), "{err}");
    assert_eq!(girders_json::to_pointer(&err).to_string(), "/rgb");
}

#[test]
fn arrays_need_no_default_for_their_elements() {
    // `Pixel` has no `Default`.
    let codec = array::<_, 2>(pixel());
    let text = r#"[{"rgb":[0,0,0],"position":[0,0]},{"rgb":[9,9,9],"position":[1,1]}]"#;
    let pixels = json(&codec, text).unwrap();
    assert_eq!(pixels[1].rgb, [9, 9, 9]);
    // And an empty array (the models' old `empty_array()` workaround).
    assert_eq!(json(&array::<_, 0>(null()), "[]").unwrap(), []);
    assert!(json(&array::<_, 0>(null()), "[null]").is_err());
}

#[test]
fn array_element_errors_accumulate_with_their_index() {
    let err = girders_json::decode_with(
        "[1,300,3,400]",
        &array::<_, 4>(u8()),
        DecodeOptions::strict(),
    )
    .unwrap_err();
    let paths: Vec<Vec<PathSegment>> = err.iter().map(|e| e.path().cloned().collect()).collect();
    assert_eq!(
        paths,
        [vec![PathSegment::Index(1)], vec![PathSegment::Index(3)]]
    );
}

// -- numbers in strings ---------------------------------------------------------------

#[test]
fn stringified_numbers() {
    let id = stringified(u64());
    assert_eq!(
        girders_json::encode(&u64::MAX, &id).unwrap(),
        r#""18446744073709551615""#
    );
    assert_eq!(json(&id, r#""18446744073709551615""#).unwrap(), u64::MAX);
    assert_eq!(girders_cbor::encode(&7, &id), hex("61 37"));
    for bad in [r#""007""#, r#""+5""#, r#""1.0""#, r#"" 5""#, r#""""#, "5"] {
        assert!(json(&id, bad).is_err(), "{bad}");
    }
    let err = json(&id, r#""18446744073709551616""#).unwrap_err();
    assert!(matches!(err.problem(), Problem::OutOfRange(_)), "{err}");

    let big = stringified(i128());
    assert_eq!(
        json(&big, &format!("\"{}\"", i128::MIN)).unwrap(),
        i128::MIN
    );
    let ratio = stringified(f64());
    assert_eq!(girders_json::encode(&f64::NAN, &ratio).unwrap(), r#""NaN""#);
    assert!(json(&ratio, r#""Infinity""#).unwrap().is_infinite());
    assert_eq!(json(&ratio, r#""1e3""#).unwrap(), 1000.0);
}

#[test]
fn a_number_or_its_string() {
    let lenient = one_of((u64(), stringified(u64())));
    assert_eq!(json(&lenient, "5").unwrap(), 5);
    assert_eq!(json(&lenient, r#""5""#).unwrap(), 5);
}
