//! The error model: typed application errors, `Invalid` paths, fatal
//! custom errors and the JSON reader's `Malformed` kinds.

use core::fmt;
use girders::prelude::*;
use girders::{DecodeContext, DecodeResult, Decoder, Reader};
use girders_json::Malformed;

#[derive(Debug, Clone, PartialEq)]
struct Odd(i64);

impl fmt::Display for Odd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is odd", self.0)
    }
}

impl std::error::Error for Odd {}

fn even() -> impl Codec<Output = i64> {
    refine(
        i64(),
        |n: &i64| if n % 2 == 0 { Ok(()) } else { Err(Odd(*n)) },
    )
}

fn key(k: &str) -> PathSegment {
    PathSegment::Key(k.into())
}

// -- typed custom errors ---------------------------------------------------------

#[test]
fn custom_errors_keep_their_type_and_path() {
    let error =
        girders_json::decode_with("[2, 3, 4, 5]", &list(even()), DecodeOptions::all_errors())
            .unwrap_err();
    let found: Vec<(Vec<PathSegment>, i64)> = error
        .custom_errors::<Odd>()
        .map(|(detail, odd)| (detail.path_vec(), odd.0))
        .collect();
    assert_eq!(
        found,
        vec![
            (vec![PathSegment::Index(1)], 3),
            (vec![PathSegment::Index(3)], 5)
        ]
    );
    assert_eq!(error.custom::<Odd>(), Some(&Odd(3)));
    // Another type isn't there.
    assert_eq!(error.custom::<core::fmt::Error>(), None);
    assert_eq!(error.first().custom::<Odd>(), Some(&Odd(3)));
}

#[test]
fn custom_errors_are_recoverable_by_default() {
    let error = girders_json::decode("3", &even()).unwrap_err();
    let Problem::Custom(custom) = error.problem() else {
        panic!("not custom: {error:?}")
    };
    assert!(!custom.is_fatal());
    assert_eq!(custom.downcast_ref::<Odd>(), Some(&Odd(3)));
    assert_eq!(custom.to_string(), "3 is odd");
}

#[test]
fn message_errors_display_their_text() {
    let error = girders_json::decode("1", &fail::<i64, _>("not supported yet")).unwrap_err();
    assert!(matches!(error.problem(), Problem::Custom(_)));
    assert_eq!(error.to_string(), "not supported yet (at byte 0)");

    let error = girders_json::decode(
        "1",
        &try_map(i64(), |n| Err::<i64, _>(format!("{n} is not allowed"))),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "1 is not allowed (at bytes 0..1)");
}

#[test]
fn fail_with_a_typed_error() {
    let error = girders_json::decode("1", &fail::<i64, _>(Odd(1))).unwrap_err();
    assert_eq!(error.custom::<Odd>(), Some(&Odd(1)));
}

// -- Invalid::at -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Range {
    start: i64,
    end: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct EndBeforeStart;

impl fmt::Display for EndBeforeStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("end is before start")
    }
}

impl std::error::Error for EndBeforeStart {}

fn range() -> impl Codec<Output = Range> {
    let raw = record((
        field("start", i64(), |r: &Range| &r.start),
        field("end", i64(), |r: &Range| &r.end),
    ))
    .build(|(start, end)| Range { start, end });
    refine(raw, |r: &Range| {
        if r.end < r.start {
            Err(Invalid::at("end", EndBeforeStart))
        } else {
            Ok(())
        }
    })
}

#[test]
fn invalid_at_reports_below_the_validated_value() {
    let codec = record((field("window", range(), |w: &Range| w),)).build(|(w,)| w);
    let input = r#"{"window": {"start": 5, "end": 1}}"#;
    let error = girders_json::decode(input, &codec).unwrap_err();
    assert_eq!(error.path(), vec![key("window"), key("end")]);
    assert_eq!(error.custom::<EndBeforeStart>(), Some(&EndBeforeStart));
    // The location is the validated value's: the whole range object.
    assert_eq!(error.location(), Some(&Span::new(11, 33)));
    assert_eq!(
        error.to_string(),
        "window.end: end is before start (at bytes 11..33)"
    );
}

#[test]
fn nested_invalid_builds_a_deeper_path() {
    let codec = refine(list(i64()), |items: &Vec<i64>| {
        match items.windows(2).position(|w| w[1] < w[0]) {
            Some(i) => Err(Invalid::at("items", Invalid::at(i + 1, "out of order"))),
            None => Ok(()),
        }
    });
    let error = girders_json::decode("[1, 2, 0]", &codec).unwrap_err();
    assert_eq!(error.path(), vec![key("items"), PathSegment::Index(2)]);
    assert_eq!(error.to_string(), "items[2]: out of order (at bytes 0..9)");

    let invalid = Invalid::at("items", Invalid::at(2, Invalid::at("name", "empty")));
    assert_eq!(invalid.to_string(), "items[2].name: empty");
}

// -- fatal custom errors -----------------------------------------------------------

/// A custom decoder that reports a fatal error: after it, nothing else
/// is tried.
struct Poisoned;

impl Decoder for Poisoned {
    type Output = i64;
    fn decode<R: Reader + ?Sized>(
        &self,
        r: &mut R,
        _cx: &mut DecodeContext,
    ) -> DecodeResult<i64, R> {
        Err(DecodeError::at(
            Problem::Custom(CustomError::fatal("poisoned input")),
            r.here(),
        ))
    }
}

#[test]
fn fatal_custom_errors_stop_one_of() {
    let error = girders_json::decode("1", &one_of((Poisoned, i64()))).unwrap_err();
    assert!(error.is_fatal());
    assert_eq!(error.to_string(), "poisoned input (at byte 0)");
    // A recoverable error lets the next option run.
    assert_eq!(
        girders_json::decode("1", &one_of((fail::<i64, _>("no"), i64()))).unwrap(),
        1
    );
}

#[test]
fn fatal_custom_errors_stop_accumulating() {
    let error =
        girders_json::decode_with("[1, 2, 3]", &list(Poisoned), DecodeOptions::all_errors())
            .unwrap_err();
    assert_eq!(error.len(), 1);
    let error = girders_json::decode_with(
        "[1, 2, 3]",
        &list(fail::<i64, _>("no")),
        DecodeOptions::all_errors(),
    )
    .unwrap_err();
    assert_eq!(error.len(), 3);
}

// -- Malformed -------------------------------------------------------------------

fn malformed(input: &str) -> Malformed {
    match girders_json::parse(input).unwrap_err().problem() {
        Problem::Format(m) => *m,
        other => panic!("{input:?}: not a format error: {other:?}"),
    }
}

#[test]
fn json_syntax_errors_have_kinds() {
    let cases = [
        ("", Malformed::UnexpectedEnd),
        ("[1,", Malformed::UnexpectedEnd),
        (r#"{"a": 1,"#, Malformed::UnexpectedEnd),
        ("1 2", Malformed::TrailingInput),
        ("@", Malformed::UnexpectedCharacter(b'@')),
        ("tru", Malformed::InvalidLiteral),
        ("nul", Malformed::InvalidLiteral),
        (r#""abc"#, Malformed::UnterminatedString),
        ("\"a\u{1}\"", Malformed::ControlCharacterInString),
        (r#""\q""#, Malformed::InvalidEscape(b'q')),
        (r#""\u12x4""#, Malformed::InvalidHexDigit(b'x')),
        (r#""\ud800""#, Malformed::LoneHighSurrogate(0xd800)),
        (r#""\udc00""#, Malformed::LoneLowSurrogate(0xdc00)),
        ("-", Malformed::ExpectedDigit),
        ("1.", Malformed::ExpectedFractionDigit),
        ("1e", Malformed::ExpectedExponentDigit),
        ("{1: 2}", Malformed::ExpectedKey),
        (r#"{"a" 1}"#, Malformed::ExpectedColon),
        ("[1 2]", Malformed::ExpectedCommaOrBracket),
        (r#"{"a": 1 "b": 2}"#, Malformed::ExpectedCommaOrBrace),
    ];
    let mut wrong = Vec::new();
    for (input, expected) in cases {
        let got = std::panic::catch_unwind(|| malformed(input));
        if got.as_ref().ok() != Some(&expected) {
            wrong.push(format!("{input:?}: expected {expected:?}, got {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn malformed_input_is_fatal() {
    let error =
        girders_json::decode_with("[1, @]", &list(i64()), DecodeOptions::all_errors()).unwrap_err();
    assert!(error.is_fatal());
    assert_eq!(error.len(), 1);
}
