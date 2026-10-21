//! Example 03: collections and nesting.
//!
//! Codecs compose: `list` and `btree_map` wrap an inner codec, and a
//! record can hold another record. A `Team` nests a list of `Member`s and
//! a map of seat counts.

use girders::prelude::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub name: String,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub name: String,
    pub members: Vec<Member>,
    pub seats: BTreeMap<String, i64>,
}

fn member_codec() -> impl Codec<Output = Member> {
    record((
        field("name", string(), |m: &Member| &m.name),
        field("admin", bool(), |m: &Member| &m.admin),
    ))
    .build(|(name, admin)| Member { name, admin })
}

pub fn team_codec() -> impl Codec<Output = Team> {
    record((
        field("name", string(), |t: &Team| &t.name),
        field("members", list(member_codec()), |t: &Team| &t.members),
        field("seats", btree_map(string(), i64()), |t: &Team| &t.seats),
    ))
    .build(|(name, members, seats)| Team {
        name,
        members,
        seats,
    })
}

pub fn main() {
    let team = Team {
        name: "Eng".into(),
        members: vec![
            Member {
                name: "ada".into(),
                admin: true,
            },
            Member {
                name: "bo".into(),
                admin: false,
            },
        ],
        seats: BTreeMap::from([("admin".into(), 1), ("dev".into(), 3)]),
    };
    let json = girders_json::encode(&team, &team_codec()).unwrap();
    assert_eq!(girders_json::decode(&json, &team_codec()).unwrap(), team);

    let t = girders_json::decode(
        r#"{"name":"Eng","members":[{"name":"ada","admin":true}],"seats":{"dev":3}}"#,
        &team_codec(),
    )
    .unwrap();
    assert_eq!(
        t.members,
        vec![Member {
            name: "ada".into(),
            admin: true
        }]
    );
    assert_eq!(t.seats.get("dev"), Some(&3));

    // Second member's `admin` is the wrong type.
    let e = girders_json::decode(
        r#"{"name":"Eng","members":[{"name":"ada","admin":true},{"name":"bo","admin":7}],"seats":{}}"#,
        &team_codec(),
    )
    .unwrap_err();
    assert_eq!(
        e.path(),
        vec![
            PathSegment::Key("members".into()),
            PathSegment::Index(1),
            PathSegment::Key("admin".into())
        ]
    );
    println!("03_collections ok");
}
