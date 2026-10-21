//! Example 05: error reporting with spans and paths.
//!
//! A decode failure carries a logical path (`server.port`) and the byte
//! span of the offending token, even for a type mismatch deep in the
//! document. `diagnostic` turns the span into line/column with a caret.

use girders::prelude::*;
use girders_json::diagnostic;

#[derive(Debug, PartialEq)]
pub struct Server {
    pub host: String,
    pub port: i64,
}

#[derive(Debug, PartialEq)]
pub struct Config {
    pub name: String,
    pub server: Server,
}

fn server_codec() -> impl Codec<Output = Server> {
    record((
        field("host", string(), |s: &Server| &s.host),
        field("port", i64(), |s: &Server| &s.port),
    ))
    .build(|(host, port)| Server { host, port })
}

pub fn config_codec() -> impl Codec<Output = Config> {
    record((
        field("name", string(), |c: &Config| &c.name),
        field("server", server_codec(), |c: &Config| &c.server),
    ))
    .build(|(name, server)| Config { name, server })
}

const BAD_CONFIG: &str = r#"{
  "name": "prod",
  "server": {
    "host": "localhost",
    "port": "eighty"
  }
}"#;

pub fn main() {
    let e = girders_json::decode(BAD_CONFIG, &config_codec()).unwrap_err();
    assert_eq!(
        e.path(),
        vec![
            PathSegment::Key("server".into()),
            PathSegment::Key("port".into())
        ]
    );
    assert_eq!(e.problem(), &Problem::mismatch("int", "string"));
    assert_eq!(
        diagnostic::resolve(BAD_CONFIG, e.location().unwrap().start()),
        (5, 13)
    );
    assert_eq!(girders_json::to_pointer(&e).to_string(), "/server/port");
    assert_eq!(
        e.to_string(),
        "server.port: expected int, found string (at byte 71)"
    );
    let rendered = diagnostic::render(&e, BAD_CONFIG);
    assert_eq!(
        rendered,
        "/server/port: expected int, found string\n  at line 5, column 13\n   |\n 5 |     \"port\": \"eighty\"\n   |             ^"
    );
    println!("{rendered}");

    let e = girders_json::decode(r#"{"name": "x", "#, &config_codec()).unwrap_err();
    assert!(matches!(
        e.problem(),
        Problem::Format(girders_json::Malformed::UnexpectedEnd)
    ));

    // Accumulate mode reports every problem at once.
    let input = r#"{"name": 1, "server": {"host": 2, "port": "x"}, "extra": true}"#;
    let e =
        girders_json::decode_with(input, &config_codec(), DecodeOptions::all_errors()).unwrap_err();
    assert_eq!(e.len(), 4);
    println!("{}", diagnostic::render_short(&e, input));
    println!("05_error_reporting ok");
}
