//! Example 04: validation and newtypes.
//!
//! `convert` wraps a codec in a total isomorphism (a newtype). `try_convert`
//! adds a fallible decode: the error becomes a `ValidationFailed` at the
//! offending value's span, while encoding stays total. `refine` validates
//! without changing the type.

use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct UserId(pub String);

pub fn user_id_codec() -> impl Codec<Output = UserId> {
    convert(string(), UserId, |id: &UserId| id.0.clone())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Email(pub String);

pub fn email_codec() -> impl Codec<Output = Email> {
    try_convert(
        string(),
        |raw: String| {
            if raw.contains('@') {
                Ok(Email(raw))
            } else {
                Err("expected an email address containing `@`")
            }
        },
        |e: &Email| e.0.clone(),
    )
}

/// A TCP port: an integer in 1..=65535.
pub fn port_codec() -> impl Codec<Output = i64> {
    refine(i64(), |n: &i64| {
        if (1..=65_535).contains(n) {
            Ok(())
        } else {
            Err("expected a port in 1..65535")
        }
    })
}

pub fn main() {
    let json = girders_json::encode(&UserId("u_1".into()), &user_id_codec()).unwrap();
    assert_eq!(json, r#""u_1""#);
    assert_eq!(
        girders_json::decode(&json, &user_id_codec()).unwrap(),
        UserId("u_1".into())
    );

    assert_eq!(
        girders_json::decode(r#""ada@example.com""#, &email_codec()).unwrap(),
        Email("ada@example.com".into())
    );
    let e = girders_json::decode(r#""not-an-email""#, &email_codec()).unwrap_err();
    assert_eq!(
        e.problem(),
        &Problem::ValidationFailed("expected an email address containing `@`".into())
    );
    // The failure covers the offending value's whole span.
    assert_eq!(e.location(), Some(&Span::new(0, 14)));

    assert_eq!(girders_json::decode("8080", &port_codec()).unwrap(), 8080);
    let e = girders_json::decode("70000", &port_codec()).unwrap_err();
    assert_eq!(
        e.problem(),
        &Problem::ValidationFailed("expected a port in 1..65535".into())
    );
    assert!(e.location().is_some());
    println!("04_validation ok");
}
