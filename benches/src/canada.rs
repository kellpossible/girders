//! `canada.json`: GeoJSON, number-heavy (~111k coordinate pairs).

use girders::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Canada = FeatureCollection;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureCollection {
    #[serde(rename = "type")]
    pub obj_type: ObjType,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    #[serde(rename = "type")]
    pub obj_type: ObjType,
    pub properties: BTreeMap<String, String>,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    #[serde(rename = "type")]
    pub obj_type: ObjType,
    pub coordinates: Vec<Vec<(f32, f32)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ObjType {
    FeatureCollection,
    Feature,
    Polygon,
}

fn obj_type() -> impl Codec<Output = ObjType> + Clone {
    string_enum([
        ("FeatureCollection", ObjType::FeatureCollection),
        ("Feature", ObjType::Feature),
        ("Polygon", ObjType::Polygon),
    ])
}

fn geometry() -> impl Codec<Output = Geometry> + Clone {
    record((
        field("type", obj_type(), |g: &Geometry| &g.obj_type),
        field(
            "coordinates",
            list(list(tuple2(f32(), f32()))),
            |g: &Geometry| &g.coordinates,
        ),
    ))
    .build(|(obj_type, coordinates)| Geometry {
        obj_type,
        coordinates,
    })
}

fn feature() -> impl Codec<Output = Feature> + Clone {
    record((
        field("type", obj_type(), |f: &Feature| &f.obj_type),
        field(
            "properties",
            btree_map(string(), string()),
            |f: &Feature| &f.properties,
        ),
        field("geometry", geometry(), |f: &Feature| &f.geometry),
    ))
    .build(|(obj_type, properties, geometry)| Feature {
        obj_type,
        properties,
        geometry,
    })
}

pub fn codec() -> impl Codec<Output = Canada> + Clone {
    record((
        field("type", obj_type(), |c: &FeatureCollection| &c.obj_type),
        field("features", list(feature()), |c: &FeatureCollection| {
            &c.features
        }),
    ))
    .build(|(obj_type, features)| FeatureCollection { obj_type, features })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn girders_matches_serde() {
        let json = crate::data("canada.json");
        let a: Canada = serde_json::from_str(&json).unwrap();
        let b = girders_json::decode(&json, &codec()).unwrap();
        assert_eq!(a, b);
        let back = girders_json::encode(&b, &codec()).unwrap();
        assert_eq!(girders_json::decode(&back, &codec()).unwrap(), b);
        assert_eq!(serde_json::from_str::<Canada>(&back).unwrap(), a);
    }
}
