//! Half-precision floats and RFC 8746 typed arrays.

use girders::Problem;
use girders::prelude::*;
use girders_cbor::{Value, decode, encode, value};
use proptest::prelude::*;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

// -- f16 -----------------------------------------------------------------------------

#[test]
fn f16_quantizes_to_the_nearest_half() {
    // 1.1 is not a half; the nearest is 1.099609375 (0x3c66).
    assert_eq!(encode(&1.1, &f16()), hex("f9 3c66"));
    assert_eq!(
        decode(&hex("fb 3ff199999999999a"), &f16()).unwrap(),
        1.099_609_4
    );
    assert_eq!(girders_json::decode("1.1", &f16()).unwrap(), 1.099_609_4);
    assert_eq!(girders_json::encode(&1.1, &f16()).unwrap(), "1.0996094");
    assert_eq!(decode(&hex("f9 7bff"), &f16()).unwrap(), 65504.0);
}

#[test]
fn f16_rejects_values_beyond_its_range() {
    let err = decode(&encode(&70000.0, &f64()), &f16()).unwrap_err();
    assert!(matches!(err.problem(), Problem::OutOfRange(_)), "{err}");
    // Infinities and NaN are halves too.
    assert_eq!(decode(&hex("f9 7c00"), &f16()).unwrap(), f32::INFINITY);
    assert!(decode(&hex("f9 7e00"), &f16()).unwrap().is_nan());
}

// -- packed --------------------------------------------------------------------------

#[test]
fn packed_arrays_are_tagged_little_endian_byte_strings() {
    // Tag 69 (uint16, little endian) over 6 bytes.
    let bytes = encode(&vec![1u16, 2, 0x0304], &packed(u16()));
    assert_eq!(bytes, hex("d8 45 46 0100 0200 0403"));
    assert_eq!(decode(&bytes, &packed(u16())).unwrap(), [1, 2, 0x0304]);
    // Tag 64: uint8.
    assert_eq!(encode(&vec![7u8, 255], &packed(u8())), hex("d8 40 42 07ff"));
    // Tag 79: sint64, little endian.
    assert_eq!(
        encode(&vec![-2i64], &packed(i64())),
        hex("d8 4f 48 feffffffffffffff")
    );
    // Tag 84: binary16, little endian.
    assert_eq!(
        encode(&vec![1.5f32, -2.0], &packed(f16())),
        hex("d8 54 44 003e 00c0")
    );
}

#[test]
fn packed_arrays_read_either_byte_order_or_a_plain_array() {
    // Tag 65: uint16, big endian.
    assert_eq!(
        decode(&hex("d8 41 44 0001 0203"), &packed(u16())).unwrap(),
        [1, 0x0203]
    );
    // Tag 82: binary64, big endian.
    assert_eq!(
        decode(&hex("d8 52 48 3ff8000000000000"), &packed(f64())).unwrap(),
        [1.5]
    );
    // Tag 68: uint8, clamped.
    assert_eq!(
        decode(&hex("d8 44 42 0102"), &packed(u8())).unwrap(),
        [1, 2]
    );
    // A plain array, as an encoder without typed arrays writes it.
    assert_eq!(
        decode(&hex("83 01 02 19 0304"), &packed(u16())).unwrap(),
        [1, 2, 0x0304]
    );
}

#[test]
fn packed_arrays_check_the_tag_and_the_length() {
    // A float32 array where uint16 is expected.
    let err = decode(&hex("d8 55 44 0000c03f"), &packed(u16())).unwrap_err();
    assert!(err.to_string().contains("tag 85"), "{err}");
    // Five bytes cannot hold uint16s.
    let err = decode(&hex("d8 45 45 0100020003"), &packed(u16())).unwrap_err();
    assert_eq!(
        err.custom::<girders::packed::TypedArrayLengthError>(),
        Some(&girders::packed::TypedArrayLengthError {
            element_size: 2,
            length: 5
        })
    );
    // Elements of a plain array are range-checked as usual.
    assert!(decode(&hex("81 1a 00010000"), &packed(u16())).is_err());
}

#[test]
fn json_has_plain_arrays() {
    let v = vec![1u16, 2, 3];
    assert_eq!(girders_json::encode(&v, &packed(u16())).unwrap(), "[1,2,3]");
    assert_eq!(girders_json::decode("[1,2,3]", &packed(u16())).unwrap(), v);
}

#[test]
fn packed_arrays_are_kept_as_tags_by_value() {
    let tree = decode(&encode(&vec![1u16, 2], &packed(u16())), &value()).unwrap();
    assert_eq!(
        tree,
        Value::Tag(69, Box::new(Value::Bytes(vec![1, 0, 2, 0])))
    );
}

#[test]
fn sizes_for_a_thousand_half_floats_and_small_integers() {
    let halves: Vec<f32> = (0..1000).map(|i| i as f32 / 8.0).collect();
    // A tag (2 bytes), a byte string head (3) and 2 bytes per value.
    assert_eq!(encode(&halves, &packed(f16())).len(), 2 + 3 + 2000);
    // As an array: a 3-byte head, and 3 bytes per half float.
    assert_eq!(encode(&halves, &list(f16())).len(), 3 + 3 * 1000);
    // f32 values that are exact halves are written as halves anyway.
    assert_eq!(encode(&halves, &list(f32())).len(), 3 + 3 * 1000);

    let small: Vec<u8> = (0..1000).map(|i| (i % 100) as u8).collect();
    assert_eq!(encode(&small, &packed(u8())).len(), 2 + 3 + 1000);
    // 0..=23 take 1 byte as integers, 24..=99 take 2.
    assert_eq!(encode(&small, &list(u8())).len(), 3 + 240 + 760 * 2);
}

proptest! {
    #[test]
    fn packed_round_trips(a in any::<Vec<u16>>(), b in any::<Vec<i32>>(), c in any::<Vec<u64>>(), d in any::<Vec<f64>>()) {
        prop_assert_eq!(decode(&encode(&a, &packed(u16())), &packed(u16())).unwrap(), a.clone());
        prop_assert_eq!(decode(&encode(&b, &packed(i32())), &packed(i32())).unwrap(), b);
        prop_assert_eq!(decode(&encode(&c, &packed(u64())), &packed(u64())).unwrap(), c);
        let back = decode(&encode(&d, &packed(f64())), &packed(f64())).unwrap();
        prop_assert_eq!(back.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), d.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
        // And through JSON, as plain arrays.
        prop_assert_eq!(girders_json::decode(&girders_json::encode(&a, &packed(u16())).unwrap(), &packed(u16())).unwrap(), a);
    }
}
