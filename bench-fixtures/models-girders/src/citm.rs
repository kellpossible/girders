//! `citm_catalog.json`: string-keyed maps of records, many small integers
//! and `null`-only fields.
//!
//! Differences from json-benchmark's model: map keys are `String` instead
//! of `PrimStr<u32>` (girders' `btree_map` keys are always strings, so
//! numeric-ness of the ids is not checked by either side), and
//! `empty::Array` is `[(); 0]` (serde's built-in impl accepts exactly `[]`).

use girders::prelude::*;
use std::collections::BTreeMap;

pub type Id = u32;

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct Price {
    pub amount: u32,
    pub audience_sub_category_id: Id,
    pub seat_category_id: Id,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeatCategory {
    pub areas: Vec<Area>,
    pub seat_category_id: Id,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub area_id: Id,
    pub block_ids: [(); 0],
}

/// Exactly `[]`. There is no fixed-length/empty array codec, so go through
/// a list (whose element codec is never used) and reject non-empty input.
fn empty_array() -> impl Codec<Output = [(); 0]> + Clone {
    try_bimap(
        list(null()),
        |v: Vec<()>| if v.is_empty() { Ok([]) } else { Err("expected an empty array") },
        |_: &[(); 0]| Vec::new(),
    )
}

fn area() -> impl Codec<Output = Area> + Clone {
    record((
        field("areaId", u32(), |a: &Area| &a.area_id),
        field("blockIds", empty_array(), |a: &Area| &a.block_ids),
    ))
    .build(|(area_id, block_ids)| Area { area_id, block_ids })
}

fn seat_category() -> impl Codec<Output = SeatCategory> + Clone {
    record((
        field("areas", list(area()), |s: &SeatCategory| &s.areas),
        field("seatCategoryId", u32(), |s: &SeatCategory| &s.seat_category_id),
    ))
    .build(|(areas, seat_category_id)| SeatCategory { areas, seat_category_id })
}

fn price() -> impl Codec<Output = Price> + Clone {
    record((
        field("amount", u32(), |p: &Price| &p.amount),
        field("audienceSubCategoryId", u32(), |p: &Price| &p.audience_sub_category_id),
        field("seatCategoryId", u32(), |p: &Price| &p.seat_category_id),
    ))
    .build(|(amount, audience_sub_category_id, seat_category_id)| Price {
        amount,
        audience_sub_category_id,
        seat_category_id,
    })
}

fn performance() -> impl Codec<Output = Performance> + Clone {
    record((
        field("eventId", u32(), |p: &Performance| &p.event_id),
        field("id", u32(), |p: &Performance| &p.id),
        optional_field("logo", string(), |p: &Performance| &p.logo),
        field("name", null(), |p: &Performance| &p.name),
        field("prices", list(price()), |p: &Performance| &p.prices),
        field("seatCategories", list(seat_category()), |p: &Performance| &p.seat_categories),
        field("seatMapImage", null(), |p: &Performance| &p.seat_map_image),
        field("start", u64(), |p: &Performance| &p.start),
        field("venueCode", string(), |p: &Performance| &p.venue_code),
    ))
    .build(|(event_id, id, logo, name, prices, seat_categories, seat_map_image, start, venue_code)| Performance {
        event_id,
        id,
        logo,
        name,
        prices,
        seat_categories,
        seat_map_image,
        start,
        venue_code,
    })
}

fn event() -> impl Codec<Output = Event> + Clone {
    record((
        field("description", null(), |e: &Event| &e.description),
        field("id", u32(), |e: &Event| &e.id),
        optional_field("logo", string(), |e: &Event| &e.logo),
        field("name", string(), |e: &Event| &e.name),
        field("subTopicIds", list(u32()), |e: &Event| &e.sub_topic_ids),
        field("subjectCode", null(), |e: &Event| &e.subject_code),
        field("subtitle", null(), |e: &Event| &e.subtitle),
        field("topicIds", list(u32()), |e: &Event| &e.topic_ids),
    ))
    .build(|(description, id, logo, name, sub_topic_ids, subject_code, subtitle, topic_ids)| Event {
        description,
        id,
        logo,
        name,
        sub_topic_ids,
        subject_code,
        subtitle,
        topic_ids,
    })
}

pub fn codec() -> impl Codec<Output = CitmCatalog> + Clone {
    record((
        field("areaNames", btree_map(string(), string()), |c: &CitmCatalog| &c.area_names),
        field("audienceSubCategoryNames", btree_map(string(), string()), |c: &CitmCatalog| &c.audience_sub_category_names),
        field("blockNames", btree_map(string(), string()), |c: &CitmCatalog| &c.block_names),
        field("events", btree_map(string(), event()), |c: &CitmCatalog| &c.events),
        field("performances", list(performance()), |c: &CitmCatalog| &c.performances),
        field("seatCategoryNames", btree_map(string(), string()), |c: &CitmCatalog| &c.seat_category_names),
        field("subTopicNames", btree_map(string(), string()), |c: &CitmCatalog| &c.sub_topic_names),
        field("subjectNames", btree_map(string(), string()), |c: &CitmCatalog| &c.subject_names),
        field("topicNames", btree_map(string(), string()), |c: &CitmCatalog| &c.topic_names),
        field("topicSubTopics", btree_map(string(), list(u32())), |c: &CitmCatalog| &c.topic_sub_topics),
        field("venueNames", btree_map(string(), string()), |c: &CitmCatalog| &c.venue_names),
    ))
    .build(
        |(
            area_names,
            audience_sub_category_names,
            block_names,
            events,
            performances,
            seat_category_names,
            sub_topic_names,
            subject_names,
            topic_names,
            topic_sub_topics,
            venue_names,
        )| CitmCatalog {
            area_names,
            audience_sub_category_names,
            block_names,
            events,
            performances,
            seat_category_names,
            sub_topic_names,
            subject_names,
            topic_names,
            topic_sub_topics,
            venue_names,
        },
    )
}

