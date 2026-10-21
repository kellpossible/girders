//! Example 02: a tagged union (sum type).
//!
//! `{"type":"circle","radius":2.0}`: the tag sits next to the variant's
//! fields. Variants are listed for decoding; encoding is an exhaustive
//! `match` returning a positional `Pick3::VariantN(&payload)`, so rustc
//! checks both that every variant is handled and that each payload has the
//! right type. Payloads are borrowed, so a variant with several fields
//! holds them in a struct of its own.

use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Rectangle {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Circle { radius: f64 },
    Rectangle(Rectangle),
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
                    field("width", f64(), |r: &Rectangle| &r.width),
                    field("height", f64(), |r: &Rectangle| &r.height),
                ))
                .build(|(width, height)| Rectangle { width, height }),
                Shape::Rectangle,
            ),
            // A fieldless variant: only the tag.
            variant("point", empty_object(), |()| Shape::Point),
        ),
        |s| match s {
            // A variant with one field can borrow it directly.
            Shape::Circle { radius } => Pick3::Variant0(radius),
            Shape::Rectangle(rectangle) => Pick3::Variant1(rectangle),
            Shape::Point => Pick3::Variant2(&()),
        },
    )
}

/// A one-field object `{name: f64}` as a bare `f64`.
fn f64_field(name: &'static str) -> impl Codec<Output = f64> {
    record((field(name, f64(), |x: &f64| x),)).build(|(x,)| x)
}

pub fn to_json(shape: &Shape) -> Result<String, girders_json::EncodeError> {
    girders_json::encode(shape, &shape_codec())
}

pub fn from_json(input: &str) -> Result<Shape, girders_json::Error> {
    girders_json::decode(input, &shape_codec())
}

pub fn main() {
    assert_eq!(
        to_json(&Shape::Circle { radius: 2.0 }).unwrap(),
        r#"{"type":"circle","radius":2.0}"#
    );
    assert_eq!(
        to_json(&Shape::Rectangle(Rectangle {
            width: 3.0,
            height: 4.0
        }))
        .unwrap(),
        r#"{"type":"rectangle","width":3.0,"height":4.0}"#
    );
    assert_eq!(to_json(&Shape::Point).unwrap(), r#"{"type":"point"}"#);
    for s in [
        Shape::Circle { radius: 2.0 },
        Shape::Rectangle(Rectangle {
            width: 3.0,
            height: 4.0,
        }),
        Shape::Point,
    ] {
        assert_eq!(from_json(&to_json(&s).unwrap()).unwrap(), s);
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
