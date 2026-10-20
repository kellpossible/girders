//! Example 02: a tagged union (sum type).
//!
//! `{"type":"circle","radius":2.0}`: the tag sits next to the variant's
//! fields. Variants are listed for decoding; encoding is an exhaustive
//! `match` returning a positional `Of3::Vn(payload)`, so rustc checks both that
//! every variant is handled and that each payload has the right type.

use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
    Point,
}

pub fn shape_codec() -> impl Codec<Output = Shape> {
    tagged_union(
        "type",
        (
            variant("circle", f64_field("radius"), |radius| Shape::Circle {
                radius,
            }),
            variant(
                "rectangle",
                record((
                    field("width", f64(), |r: &(f64, f64)| &r.0),
                    field("height", f64(), |r: &(f64, f64)| &r.1),
                ))
                .build(|(w, h)| (w, h)),
                |(width, height)| Shape::Rectangle { width, height },
            ),
            // A fieldless variant: only the tag.
            variant("point", empty_object(), |()| Shape::Point),
        ),
        |s| match s {
            Shape::Circle { radius } => Pick3::Variant0(radius.into()),
            // Struct-like variants have no single value to borrow, so the
            // payload is built on the spot.
            Shape::Rectangle { width, height } => Pick3::Variant1(Payload::Owned((*width, *height))),
            Shape::Point => Pick3::Variant2(Payload::Owned(())),
        },
    )
}

/// A one-field object `{name: f64}` as a bare `f64`.
fn f64_field(name: &'static str) -> impl Codec<Output = f64> {
    record((field(name, f64(), |x: &f64| x),)).build(|(x,)| x)
}

pub fn to_json(shape: &Shape) -> String {
    girders_json::encode(shape, &shape_codec())
}

pub fn from_json(input: &str) -> Result<Shape, girders_json::Error> {
    girders_json::decode(input, &shape_codec())
}

pub fn main() {
    assert_eq!(
        to_json(&Shape::Circle { radius: 2.0 }),
        r#"{"type":"circle","radius":2.0}"#
    );
    assert_eq!(
        to_json(&Shape::Rectangle {
            width: 3.0,
            height: 4.0
        }),
        r#"{"type":"rectangle","width":3.0,"height":4.0}"#
    );
    assert_eq!(to_json(&Shape::Point), r#"{"type":"point"}"#);
    for s in [
        Shape::Circle { radius: 2.0 },
        Shape::Rectangle {
            width: 3.0,
            height: 4.0,
        },
        Shape::Point,
    ] {
        assert_eq!(from_json(&to_json(&s)).unwrap(), s);
    }
    // Tag after the fields works too.
    assert_eq!(
        from_json(r#"{"radius":1.5,"type":"circle"}"#).unwrap(),
        Shape::Circle { radius: 1.5 }
    );
    let e = from_json(r#"{"type":"hexagon"}"#).unwrap_err();
    assert_eq!(
        e.problem(),
        &Problem::UnknownVariant {
            name: "hexagon".into(),
            expected: vec!["circle", "rectangle", "point"]
        }
    );
    println!("02_tagged_union ok");
}
