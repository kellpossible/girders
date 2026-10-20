//! `canada.json`: GeoJSON, number-heavy (~111k coordinate pairs).

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
