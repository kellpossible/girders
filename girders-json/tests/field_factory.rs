//! The typed field factory removes the getter annotations.
use girders::prelude::*;

#[derive(Debug, PartialEq)]
struct User {
    id: String,
    age: u32,
    email: Option<String>,
}

fn user() -> impl Codec<Output = User> {
    let f = fields_for::<User>();
    record((
        f.field("id", string(), |u| &u.id),
        f.field("age", u32(), |u| &u.age),
        f.optional("email", string(), |u| &u.email),
    ))
    .build(|(id, age, email)| User { id, age, email })
}

#[test]
fn no_annotations_needed() {
    let u = girders_json::decode(r#"{"id":"a","age":3}"#, &user()).unwrap();
    assert_eq!(
        u,
        User {
            id: "a".into(),
            age: 3,
            email: None
        }
    );
    assert_eq!(
        girders_json::encode(&u, &user()).unwrap(),
        r#"{"id":"a","age":3,"email":null}"#
    );
}
