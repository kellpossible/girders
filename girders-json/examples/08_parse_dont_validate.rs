//! Example 08: parse, don't validate.
//!
//! Each domain type has one way in, a `parse` function returning its own
//! error type, and its codec is `try_convert(raw_codec, parse, unparse)`.
//! A decoded `Signup` is therefore valid by construction: nothing
//! downstream re-checks it.
//!
//! Errors keep their types. `error.custom_errors::<T>()` finds every `T`
//! with its path, so a form can show each message next to its field.
//! A rule spanning several fields (a date range) reports against the field
//! at fault with `Invalid::at`.

use core::fmt;
use girders::prelude::*;

// -- an email address ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Email(String);

#[derive(Debug, Clone, PartialEq)]
pub enum EmailError {
    MissingAt,
    Empty,
}

impl fmt::Display for EmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmailError::MissingAt => f.write_str("an email address needs an `@`"),
            EmailError::Empty => f.write_str("an email address can't be empty"),
        }
    }
}

impl std::error::Error for EmailError {}

impl Email {
    pub fn parse(raw: String) -> Result<Email, EmailError> {
        if raw.is_empty() {
            Err(EmailError::Empty)
        } else if !raw.contains('@') {
            Err(EmailError::MissingAt)
        } else {
            Ok(Email(raw))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn email_codec() -> impl Codec<Output = Email> {
    try_convert(string(), Email::parse, |email: &Email| email.0.clone())
}

// -- an age ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Age(u8);

#[derive(Debug, Clone, PartialEq)]
pub struct TooYoung {
    pub minimum: u8,
}

impl fmt::Display for TooYoung {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "you must be at least {}", self.minimum)
    }
}

impl std::error::Error for TooYoung {}

impl Age {
    pub const MINIMUM: u8 = 18;

    pub fn parse(years: u8) -> Result<Age, TooYoung> {
        if years >= Self::MINIMUM {
            Ok(Age(years))
        } else {
            Err(TooYoung {
                minimum: Self::MINIMUM,
            })
        }
    }
}

/// `u8()` already rejects negative and huge numbers (`Problem::OutOfRange`),
/// so `Age::parse` only checks the rule of its own.
pub fn age_codec() -> impl Codec<Output = Age> {
    try_convert(u8(), Age::parse, |age: &Age| age.0)
}

// -- a stay: a cross-field rule ----------------------------------------------------

/// Days since some epoch, to keep the example short.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stay {
    first_night: u32,
    last_night: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StayError {
    EndsBeforeItStarts,
}

impl fmt::Display for StayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the last night is before the first")
    }
}

impl std::error::Error for StayError {}

impl Stay {
    /// The error is about `last_night`, so it says so with `Invalid::at`:
    /// it is reported at `stay.last_night` rather than at `stay`.
    pub fn parse((first_night, last_night): (u32, u32)) -> Result<Stay, Invalid> {
        if last_night < first_night {
            Err(Invalid::at("last_night", StayError::EndsBeforeItStarts))
        } else {
            Ok(Stay {
                first_night,
                last_night,
            })
        }
    }

    pub fn nights(&self) -> u32 {
        self.last_night - self.first_night + 1
    }
}

pub fn stay_codec() -> impl Codec<Output = Stay> {
    // The raw shape: a record of two plain numbers.
    let raw = record((
        field("first_night", u32(), |pair: &(u32, u32)| &pair.0),
        field("last_night", u32(), |pair: &(u32, u32)| &pair.1),
    ))
    .build(|pair| pair);
    try_convert(raw, Stay::parse, |stay: &Stay| {
        (stay.first_night, stay.last_night)
    })
}

// -- the form ----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Signup {
    pub email: Email,
    pub age: Age,
    pub stay: Stay,
}

pub fn signup_codec() -> impl Codec<Output = Signup> {
    record((
        field("email", email_codec(), |s: &Signup| &s.email),
        field("age", age_codec(), |s: &Signup| &s.age),
        field("stay", stay_codec(), |s: &Signup| &s.stay),
    ))
    .build(|(email, age, stay)| Signup { email, age, stay })
}

/// Each failure as (field path, message), ready to show beside the form.
pub fn form_errors(error: &girders_json::Error) -> Vec<(String, String)> {
    error
        .iter()
        .map(|detail| (detail.path_string(), detail.problem.to_string()))
        .collect()
}

pub fn main() {
    let good = r#"{
        "email": "ada@example.com",
        "age": 36,
        "stay": {"first_night": 10, "last_night": 12}
    }"#;
    let signup = girders_json::decode(good, &signup_codec()).unwrap();
    assert_eq!(signup.email.as_str(), "ada@example.com");
    assert_eq!(signup.stay.nights(), 3);
    // Encoding goes back through the raw shapes.
    let json = girders_json::encode(&signup, &signup_codec()).unwrap();
    assert_eq!(
        json,
        r#"{"email":"ada@example.com","age":36,"stay":{"first_night":10,"last_night":12}}"#
    );

    // Everything wrong at once: accumulate mode keeps going past each
    // failure.
    let bad = r#"{
        "email": "ada.example.com",
        "age": 12,
        "stay": {"first_night": 10, "last_night": 8}
    }"#;
    let error =
        girders_json::decode_with(bad, &signup_codec(), DecodeOptions::all_errors()).unwrap_err();
    assert_eq!(
        form_errors(&error),
        vec![
            ("email".into(), "an email address needs an `@`".into()),
            ("age".into(), "you must be at least 18".into()),
            (
                "stay.last_night".into(),
                "the last night is before the first".into()
            ),
        ]
    );

    // Typed access: find the errors of one type, with where they happened.
    let (detail, too_young) = error.custom_errors::<TooYoung>().next().unwrap();
    assert_eq!(too_young, &TooYoung { minimum: 18 });
    assert_eq!(detail.path_string(), "age");
    assert_eq!(error.custom::<EmailError>(), Some(&EmailError::MissingAt));
    assert_eq!(
        error.custom::<StayError>(),
        Some(&StayError::EndsBeforeItStarts)
    );

    // Failures girders itself finds sit alongside: here a number that
    // doesn't fit a `u8` and a missing field.
    let error = girders_json::decode_with(
        r#"{"email": "", "age": 300}"#,
        &signup_codec(),
        DecodeOptions::all_errors(),
    )
    .unwrap_err();
    assert_eq!(error.custom::<EmailError>(), Some(&EmailError::Empty));
    assert!(
        error
            .iter()
            .any(|d| matches!(d.problem, Problem::OutOfRange(_)))
    );
    assert!(
        error
            .iter()
            .any(|d| matches!(&d.problem, Problem::MissingField(name) if name == "stay"))
    );
    println!("08_parse_dont_validate ok");
}
