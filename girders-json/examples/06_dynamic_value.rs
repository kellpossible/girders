//! Example 06: the self-describing `Value` tree.
//!
//! For data without a static type, decode into `Value`. Objects keep
//! document order and duplicate keys, so re-encoding is lossless. Every
//! node carries its span.

use girders_json::{Value, value};

pub fn top_level_keys(input: &str) -> Result<Vec<String>, girders_json::Error> {
    let v: Value = girders_json::parse(input)?;
    Ok(v.keys().into_iter().map(String::from).collect())
}

pub fn reencode(input: &str) -> Result<String, girders_json::Error> {
    let v: Value = girders_json::parse(input)?;
    Ok(girders_json::encode(&v, &value()))
}

pub fn pluck(input: &str, key: &str) -> Option<String> {
    let v: Value = girders_json::parse(input).ok()?;
    Some(girders_json::encode(v.get(key)?, &value()))
}

pub fn at(input: &str, pointer: &str) -> Option<String> {
    let v: Value = girders_json::parse(input).ok()?;
    Some(girders_json::encode(v.pointer_str(pointer)?, &value()))
}

pub fn main() {
    assert_eq!(
        top_level_keys(r#"{"z":1,"a":2,"m":3}"#).unwrap(),
        ["z", "a", "m"]
    );
    let input = r#"{"z":1,"a":[true,null,"x"],"m":{"k":2}}"#;
    assert_eq!(reencode(input).unwrap(), input);
    assert_eq!(
        pluck(
            r#"{"name":"prod","server":{"host":"localhost","port":8080}}"#,
            "server"
        )
        .unwrap(),
        r#"{"host":"localhost","port":8080}"#
    );
    assert_eq!(pluck(r#"{"a":1}"#, "b"), None);
    assert!(reencode("{not json").is_err());
    let doc = r#"{"name":"prod","servers":[{"port":8080},{"port":9090}]}"#;
    assert_eq!(at(doc, "/servers/1/port").unwrap(), "9090");
    assert_eq!(at(doc, "/name").unwrap(), r#""prod""#);
    assert_eq!(at(r#"{"a":1}"#, "/a/b"), None);

    // Duplicate keys survive a round trip; lookups see the last one.
    let dup: Value = girders_json::parse(r#"{"a":1,"a":2}"#).unwrap();
    assert_eq!(girders_json::encode(&dup, &value()), r#"{"a":1,"a":2}"#);
    assert_eq!(girders_json::encode(dup.get("a").unwrap(), &value()), "2");

    // Spans point back into the source.
    let v: Value = girders_json::parse(r#"{"k": [1, 2]}"#).unwrap();
    assert_eq!(
        v.get("k").unwrap().span.map(|s| (s.start(), s.end())),
        Some((6, 12))
    );
    println!("06_dynamic_value ok");
}
