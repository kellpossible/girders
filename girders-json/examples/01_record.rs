//! Example 01: a record codec.
//!
//! One `record` gives both directions. Each field is declared once (name,
//! inner codec, getter); forgetting a field is a compile error at `build`.

use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: String,
    pub age: i64,
    pub nickname: Option<String>,
}

pub fn user_codec() -> impl Codec<Output = User> {
    record((
        field("id", string(), |u: &User| &u.id),
        field("age", i64(), |u: &User| &u.age),
        // A missing key or an explicit null decodes to None; None encodes
        // to null.
        optional_field("nickname", string(), |u: &User| &u.nickname),
    ))
    .build(|(id, age, nickname)| User { id, age, nickname })
}

pub fn to_json(user: &User) -> String {
    girders_json::encode(user, &user_codec())
}

pub fn from_json(input: &str) -> Result<User, girders_json::Error> {
    girders_json::decode(input, &user_codec())
}

pub fn main() {
    for user in [
        User {
            id: "u_1".into(),
            age: 30,
            nickname: Some("ada".into()),
        },
        User {
            id: "u_2".into(),
            age: 41,
            nickname: None,
        },
    ] {
        assert_eq!(from_json(&to_json(&user)).unwrap(), user);
    }
    assert_eq!(
        to_json(&User {
            id: "u_3".into(),
            age: 7,
            nickname: Some("bo".into())
        }),
        r#"{"id":"u_3","age":7,"nickname":"bo"}"#
    );
    assert_eq!(
        to_json(&User {
            id: "u_4".into(),
            age: 8,
            nickname: None
        }),
        r#"{"id":"u_4","age":8,"nickname":null}"#
    );
    assert_eq!(
        from_json(r#"{"id":"u_5","age":9}"#).unwrap(),
        User {
            id: "u_5".into(),
            age: 9,
            nickname: None
        }
    );
    assert_eq!(
        from_json(r#"{"id":"u_6","age":9,"nickname":null}"#)
            .unwrap()
            .nickname,
        None
    );
    assert!(from_json(r#"{"id":"u_7"}"#).is_err());
    println!("01_record ok");
}
