//! Streaming input: `io::Read` sources (including ones that return a byte
//! at a time), inputs larger than the compaction threshold, and CBOR
//! sequences pushed in arbitrary chunks.

use girders::prelude::*;
use girders::{DecodeOptions, Problem};
use girders_cbor::{
    StreamDecoder, Value, decode, decode_with, encode, from_reader, from_reader_with, value,
};
use proptest::prelude::*;
use std::io::{self, Read};

#[derive(Debug, Clone, PartialEq)]
struct Point {
    id: u32,
    name: String,
    weights: Vec<f64>,
}

fn point() -> impl Codec<Output = Point> {
    record((
        field("id", u32(), |p: &Point| &p.id),
        field("name", string(), |p: &Point| &p.name),
        field("weights", list(f64()), |p: &Point| &p.weights),
    ))
    .build(|(id, name, weights)| Point { id, name, weights })
}

/// About 300 KB: several times the 64 KiB compaction threshold.
fn many_points() -> Vec<Point> {
    (0..4000)
        .map(|i| Point {
            id: i,
            name: format!("point number {i} with a longish name"),
            weights: (0..5).map(|j| (i * j) as f64 * 0.1).collect(),
        })
        .collect()
}

/// Returns at most one byte per `read`.
struct Trickle<'a>(&'a [u8]);

impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match (self.0.split_first(), buf.first_mut()) {
            (Some((&b, rest)), Some(slot)) => {
                *slot = b;
                self.0 = rest;
                Ok(1)
            }
            _ => Ok(0),
        }
    }
}

#[test]
fn readers_decode_what_slices_decode() {
    let points = many_points();
    let bytes = encode(&points, &list(point()));
    assert!(bytes.len() > 4 * 64 * 1024, "{}", bytes.len());
    assert_eq!(from_reader(&bytes[..], &list(point())).unwrap(), points);
    let small = &points[..50];
    let small_bytes = encode(&small.to_vec(), &list(point()));
    assert_eq!(
        from_reader(Trickle(&small_bytes), &list(point())).unwrap(),
        small
    );
    assert_eq!(
        from_reader_with(&bytes[..], &value(), DecodeOptions::new(), 1000).unwrap(),
        decode(&bytes, &value()).unwrap()
    );
}

#[test]
fn errors_after_compaction_have_absolute_offsets() {
    let mut points = many_points();
    points.last_mut().unwrap().name = "BAD".into();
    let bytes = encode(&points, &list(point()));
    // A codec that rejects the last name, after hundreds of KB of input.
    let picky = list(refine(point(), |p: &Point| {
        if p.name == "BAD" {
            Err("bad name")
        } else {
            Ok(())
        }
    }));
    let from_slice = decode(&bytes, &picky).unwrap_err();
    let from_stream = from_reader_with(&bytes[..], &picky, DecodeOptions::new(), 4096).unwrap_err();
    assert_eq!(from_stream.location(), from_slice.location());
    assert!(from_slice.location().unwrap().start() > 200_000);
}

#[test]
fn accumulating_errors_pins_and_rewinds_correctly_while_streaming() {
    // Accumulate mode checkpoints every field; a bad field late in a large
    // input must be rewound and skipped like any other.
    let mut tree = decode(&encode(&many_points(), &list(point())), &value()).unwrap();
    let Value::Array(items) = &mut tree else {
        panic!()
    };
    for i in [10, 3000, 3999] {
        let Value::Map(entries) = &mut items[i] else {
            panic!()
        };
        entries[0].1 = Value::Text("not a number".into());
    }
    let bytes = encode(&tree, &value());
    let options = DecodeOptions::strict();
    let from_slice = decode_with(&bytes, &list(point()), options).unwrap_err();
    let from_stream = from_reader_with(Trickle(&bytes), &list(point()), options, 1).unwrap_err();
    assert_eq!(from_slice.len(), 3);
    assert_eq!(
        from_stream.iter().map(|e| e.location).collect::<Vec<_>>(),
        from_slice.iter().map(|e| e.location).collect::<Vec<_>>()
    );
}

#[test]
fn read_errors_are_reported_as_io() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("cable unplugged"))
        }
    }
    let err = from_reader(Broken, &value()).unwrap_err();
    assert!(
        matches!(err.problem(), Problem::Io(m) if m.contains("cable unplugged")),
        "{err}"
    );
}

#[test]
fn truncated_streams_are_syntax_errors() {
    let bytes = encode(&many_points(), &list(point()));
    let err = from_reader(&bytes[..bytes.len() - 1], &list(point())).unwrap_err();
    assert!(matches!(err.problem(), Problem::Syntax(_)), "{err}");
}

/// Items pushed in chunks of `size`, collected as they complete.
fn pushed<C: Decoder>(codec: C, bytes: &[u8], size: usize) -> Vec<C::Output> {
    let mut stream = StreamDecoder::new(codec);
    let mut out = Vec::new();
    for chunk in bytes.chunks(size.max(1)) {
        stream.push(chunk);
        while let Some(item) = stream.next() {
            out.push(item.unwrap());
        }
    }
    stream.finish();
    while let Some(item) = stream.next() {
        out.push(item.unwrap());
    }
    out
}

#[test]
fn sequences_decode_whatever_the_chunking() {
    let points: Vec<Point> = many_points().into_iter().take(20).collect();
    let bytes: Vec<u8> = points.iter().flat_map(|p| encode(p, &point())).collect();
    for size in [1, 2, 3, 7, 64, 1000, bytes.len()] {
        assert_eq!(pushed(point(), &bytes, size), points, "chunks of {size}");
    }
}

#[test]
fn a_sequence_waits_for_more_input_and_reports_a_truncated_end() {
    let bytes = encode(&many_points()[0], &point());
    let mut stream = StreamDecoder::new(point());
    stream.push(&bytes[..bytes.len() - 1]);
    assert!(stream.next().is_none(), "waits for the rest");
    stream.finish();
    let err = stream.next().expect("an error").unwrap_err();
    assert!(matches!(err.problem(), Problem::Syntax(_)), "{err}");
}

#[test]
fn a_malformed_item_stops_the_sequence() {
    let mut stream = StreamDecoder::new(value());
    stream.push(&[0x01, 0x1c, 0x02]); // 1, a reserved value, 2
    assert_eq!(stream.next().unwrap().unwrap(), Value::Integer(1));
    assert!(stream.next().unwrap().is_err());
}

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        any::<i64>().prop_map(|i| Value::Integer(i as i128)),
        any::<Vec<u8>>().prop_map(Value::Bytes),
        any::<String>().prop_map(Value::Text),
        Just(Value::Null),
    ];
    leaf.prop_recursive(3, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((inner.clone(), inner), 0..6).prop_map(Value::Map),
        ]
    })
}

proptest! {
    #[test]
    fn sequences_survive_random_chunking(items in prop::collection::vec(arb_value(), 0..8), size in 1usize..40) {
        let bytes: Vec<u8> = items.iter().flat_map(|v| encode(v, &value())).collect();
        prop_assert_eq!(pushed(value(), &bytes, size), items);
    }
}
