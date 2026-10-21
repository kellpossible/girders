//! Example 04: validation and newtypes.
//!
//! `convert` wraps a codec in a total isomorphism (a newtype). `try_convert`
//! adds a fallible decode, and `refine` validates without changing the type.
//! Either reports your error as `Problem::Custom` at the offending value's
//! span, kept as its own type: get it back with `error.custom::<T>()`. A
//! plain message works too.

use core::fmt;
use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct UserId(pub String);

pub fn user_id_codec() -> impl Codec<Output = UserId> {
    convert(string(), UserId, |id: &UserId| id.0.clone())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Email(String);

/// Why a string isn't an email address: an error type of your own.
#[derive(Debug, Clone, PartialEq)]
pub enum EmailError {
    MissingAt,
}

impl fmt::Display for EmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected an email address containing `@`")
    }
}

impl std::error::Error for EmailError {}

impl Email {
    /// The only way to make an `Email`: parse, don't validate.
    pub fn parse(raw: String) -> Result<Email, EmailError> {
        if raw.contains('@') {
            Ok(Email(raw))
        } else {
            Err(EmailError::MissingAt)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn email_codec() -> impl Codec<Output = Email> {
    try_convert(string(), Email::parse, |e: &Email| e.0.clone())
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

    let email = girders_json::decode(r#""ada@example.com""#, &email_codec()).unwrap();
    assert_eq!(email.as_str(), "ada@example.com");
    let e = girders_json::decode(r#""not-an-email""#, &email_codec()).unwrap_err();
    // Your error comes back as its own type.
    assert_eq!(e.custom::<EmailError>(), Some(&EmailError::MissingAt));
    // The failure covers the offending value's whole span.
    assert_eq!(e.location(), Some(&Span::new(0, 14)));

    assert_eq!(girders_json::decode("8080", &port_codec()).unwrap(), 8080);
    let e = girders_json::decode("70000", &port_codec()).unwrap_err();
    // A plain message: a custom error whose text is the message.
    assert!(matches!(e.problem(), Problem::Custom(_)));
    assert_eq!(e.to_string(), "expected a port in 1..65535 (at bytes 0..5)");
    assert!(e.location().is_some());
    println!("04_validation ok");
}
