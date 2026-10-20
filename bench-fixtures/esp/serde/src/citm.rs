#[allow(unused_imports)]
use alloc::{boxed::Box, string::String, vec, vec::Vec};
// `citm_catalog.json`: string-keyed maps of records, many small integers
// and `null`-only fields.
//
// Differences from json-benchmark's model: map keys are `String` instead
// of `PrimStr<u32>` (girders' `btree_map` keys are always strings, so
// numeric-ness of the ids is not checked by either side), and
// `empty::Array` is `[(); 0]` (serde's built-in impl accepts exactly `[]`).

use serde::{Deserialize, Serialize};
use alloc::collections::BTreeMap;

pub type Id = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CitmCatalog {
    pub area_names: BTreeMap<String, String>,
    pub audience_sub_category_names: BTreeMap<String, String>,
    pub block_names: BTreeMap<String, String>,
    pub events: BTreeMap<String, Event>,
    pub performances: Vec<Performance>,
    pub seat_category_names: BTreeMap<String, String>,
    pub sub_topic_names: BTreeMap<String, String>,
    pub subject_names: BTreeMap<String, String>,
    pub topic_names: BTreeMap<String, String>,
    pub topic_sub_topics: BTreeMap<String, Vec<Id>>,
    pub venue_names: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Event {
    pub description: (),
    pub id: Id,
    pub logo: Option<String>,
    pub name: String,
    pub sub_topic_ids: Vec<Id>,
    pub subject_code: (),
    pub subtitle: (),
    pub topic_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Performance {
    pub event_id: Id,
    pub id: Id,
    pub logo: Option<String>,
    pub name: (),
    pub prices: Vec<Price>,
    pub seat_categories: Vec<SeatCategory>,
    pub seat_map_image: (),
    pub start: u64,
    pub venue_code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Price {
    pub amount: u32,
    pub audience_sub_category_id: Id,
    pub seat_category_id: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SeatCategory {
    pub areas: Vec<Area>,
    pub seat_category_id: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Area {
    pub area_id: Id,
    pub block_ids: [(); 0],
}
