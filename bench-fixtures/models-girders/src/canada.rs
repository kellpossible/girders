//! `canada.json`: GeoJSON, number-heavy (~111k coordinate pairs).

use girders::prelude::*;
use std::collections::BTreeMap;

pub type Canada = FeatureCollection;

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureCollection {
    pub obj_type: ObjType,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    pub obj_type: ObjType,
    pub properties: BTreeMap<String, String>,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub obj_type: ObjType,
    pub coordinates: Vec<Vec<(f32, f32)>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
        field("coordinates", list(list(tuple2(f32(), f32()))), |g: &Geometry| &g.coordinates),
    ))
    .build(|(obj_type, coordinates)| Geometry { obj_type, coordinates })
}

fn feature() -> impl Codec<Output = Feature> + Clone {
    record((
        field("type", obj_type(), |f: &Feature| &f.obj_type),
        field("properties", btree_map(string()), |f: &Feature| &f.properties),
        field("geometry", geometry(), |f: &Feature| &f.geometry),
    ))
    .build(|(obj_type, properties, geometry)| Feature { obj_type, properties, geometry })
}

pub fn codec() -> impl Codec<Output = Canada> + Clone {
    record((
        field("type", obj_type(), |c: &FeatureCollection| &c.obj_type),
        field("features", list(feature()), |c: &FeatureCollection| &c.features),
    ))
    .build(|(obj_type, features)| FeatureCollection { obj_type, features })
}

