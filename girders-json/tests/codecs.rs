//! Behaviour tests for the core combinators, through the JSON format.

use girders::prelude::*;
use girders_json::{StreamDecoder, Value, diagnostic, value};

fn dec<C: Decoder>(c: &C, s: &str) -> Result<C::Output, girders_json::Error> {
    girders_json::decode(s, c)
}

fn acc<C: Decoder>(c: &C, s: &str) -> girders_json::Error {
    girders_json::decode_with(s, c, DecodeOptions::strict())
        .err()
        .expect("expected errors")
}

fn key(k: &str) -> PathSegment {
    PathSegment::Key(k.into())
}

// -- the examples are the acceptance suite --------------------------------------

#[path = "../examples/01_record.rs"]
mod e01;
#[path = "../examples/02_tagged_union.rs"]
mod e02;
#[path = "../examples/03_collections.rs"]
mod e03;
#[path = "../examples/04_validation.rs"]
mod e04;
#[path = "../examples/05_error_reporting.rs"]
mod e05;
#[path = "../examples/06_dynamic_value.rs"]
mod e06;
#[path = "../examples/07_pretty_printing.rs"]
mod e07;

#[test]
fn examples() {
    e01::main();
    e02::main();
    e03::main();
    e04::main();
    e05::main();
    e06::main();
    e07::main();
}

// -- records -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct User {
    id: String,
    age: u32,
    email: Option<String>,
}

fn user() -> impl RecordCodec<Output = User> + Clone {
    record((
        field("id", string(), |u: &User| &u.id),
        field("age", u32(), |u: &User| &u.age),
        optional_field("email", string(), |u: &User| &u.email).skip_none(),
    ))
    .build(|(id, age, email)| User { id, age, email })
}

#[test]
fn strict_unknown_field() {
    let e = dec(&user(), r#"{"id":"a","age":1,"nmae":"x"}"#).unwrap_err();
    assert!(
        matches!(e.problem(), Problem::UnknownField { name, expected } if name == "nmae" && expected == &["id", "age", "email"])
    );
    assert_eq!(e.path(), vec![key("nmae")]);
}

#[test]
fn duplicate_policies() {
    let input = r#"{"id":"a","id":"b","age":1}"#;
    let e = dec(&user(), input).unwrap_err();
    assert_eq!(e.problem(), &Problem::DuplicateField("id".into()));
    let first = user_with(Duplicates::FirstWins);
    assert_eq!(dec(&first, input).unwrap().id, "a");
    let last = user_with(Duplicates::LastWins);
    assert_eq!(dec(&last, input).unwrap().id, "b");
}

fn user_with(d: Duplicates) -> impl Codec<Output = User> {
    record((
        field("id", string(), |u: &User| &u.id),
        field("age", u32(), |u: &User| &u.age),
        optional_field("email", string(), |u: &User| &u.email),
    ))
    .build(|(id, age, email)| User { id, age, email })
    .duplicates(d)
}

#[test]
fn allow_unknown_skips_any_value() {
    let lenient = record((field("id", string(), |u: &User| &u.id),))
        .build(|(id,)| User {
            id,
            age: 0,
            email: None,
        })
        .allow_unknown();
    let deep = "[".repeat(10_000) + &"]".repeat(10_000);
    let input = format!(r#"{{"x":{deep},"y":{{"a":[1,"s",null,{{}}]}},"id":"z"}}"#);
    assert_eq!(dec(&lenient, &input).unwrap().id, "z");
}

#[derive(Debug, PartialEq)]
struct Open {
    name: String,
    rest: Vec<(String, Value)>,
}

#[test]
fn extras_collects_unknown_keys() {
    let c = record((
        field("name", string(), |o: &Open| &o.name),
        extras(value(), |o: &Open| &o.rest),
    ))
    .build(|(name, rest)| Open { name, rest });
    let input = r#"{"a":1,"name":"n","b":[true]}"#;
    let o = dec(&c, input).unwrap();
    assert_eq!(
        o.rest.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        girders_json::encode(&o, &c).unwrap(),
        r#"{"name":"n","a":1,"b":[true]}"#
    );
}

#[test]
fn field_default_and_skip_none() {
    #[derive(Debug, PartialEq)]
    struct P {
        port: u16,
        tls: bool,
    }
    let c = record((
        field("port", u16(), |p: &P| &p.port),
        field_default("tls", bool(), |p: &P| &p.tls, || true),
    ))
    .build(|(port, tls)| P { port, tls });
    assert_eq!(
        dec(&c, r#"{"port":80}"#).unwrap(),
        P {
            port: 80,
            tls: true
        }
    );
    assert_eq!(
        girders_json::encode(
            &User {
                id: "a".into(),
                age: 1,
                email: None
            },
            &user()
        )
        .unwrap(),
        r#"{"id":"a","age":1}"#
    );
}

#[derive(Debug, PartialEq, Clone)]
struct Pagination {
    page: u32,
    per_page: u32,
}

#[derive(Debug, PartialEq)]
struct Page {
    pagination: Pagination,
    items: Vec<String>,
}

fn pagination() -> impl RecordCodec<Output = Pagination> {
    record((
        field("page", u32(), |p: &Pagination| &p.page),
        field("per_page", u32(), |p: &Pagination| &p.per_page),
    ))
    .build(|(page, per_page)| Pagination { page, per_page })
}

#[test]
fn fields_of_flattens_explicitly() {
    let c = record((
        fields_of(pagination(), |p: &Page| &p.pagination),
        field("items", list(string()), |p: &Page| &p.items),
    ))
    .build(|(pagination, items)| Page { pagination, items });
    let p = dec(&c, r#"{"items":["a"],"per_page":10,"page":2}"#).unwrap();
    assert_eq!(
        p.pagination,
        Pagination {
            page: 2,
            per_page: 10
        }
    );
    assert_eq!(
        girders_json::encode(&p, &c).unwrap(),
        r#"{"page":2,"per_page":10,"items":["a"]}"#
    );
    // Strictness sees the merged field set.
    let e = dec(&c, r#"{"items":[],"per_page":1,"page":1,"pages":3}"#).unwrap_err();
    assert!(
        matches!(e.problem(), Problem::UnknownField { expected, .. } if expected == &["page", "per_page", "items"])
    );
}

// -- accumulate mode -------------------------------------------------------------

#[test]
fn accumulate_reports_everything_once() {
    let e = acc(
        &list(user()),
        r#"[{"id":1,"age":-1,"zz":0},{"age":"x"},{"id":"ok","age":3}]"#,
    );
    let got: Vec<_> = e
        .iter()
        .map(|d| (d.path_string(), d.problem.to_string()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "[0].id".to_string(),
                "expected string, found number".to_string()
            ),
            ("[0].age".into(), "-1 is out of range for u64".into()),
            ("[0].zz".into(), "unknown field `zz`".into()),
            ("[1].age".into(), "expected int, found string".into()),
            ("[1]".into(), "missing required field `id`".into()),
        ]
    );
}

#[test]
fn accumulate_stops_at_syntax_errors() {
    let e = acc(&list(user()), r#"[{"id":1,"age":2}, {"id": tru}]"#);
    assert!(matches!(
        e.iter().last().unwrap().problem,
        Problem::Syntax(_)
    ));
}

#[test]
fn depth_limit() {
    let deep = "[".repeat(200) + &"]".repeat(200);
    let e = girders_json::parse(&deep).unwrap_err();
    assert_eq!(e.problem(), &Problem::DepthLimitExceeded(128));
    let ok = girders_json::decode_with(&deep, &value(), DecodeOptions::new().max_depth(Some(300)));
    assert!(ok.is_ok());
}

// -- numbers and strings -------------------------------------------------------------

#[test]
fn numbers() {
    assert_eq!(
        dec(&i128(), "-170141183460469231731687303715884105728").unwrap(),
        i128::MIN
    );
    assert_eq!(
        dec(&u128(), "340282366920938463463374607431768211455").unwrap(),
        u128::MAX
    );
    assert_eq!(dec(&u64(), "18446744073709551615").unwrap(), u64::MAX);
    assert!(matches!(
        dec(&u64(), "18446744073709551616").unwrap_err().problem(),
        Problem::OutOfRange(_)
    ));
    assert_eq!(dec(&i8(), "-128").unwrap(), -128);
    assert!(matches!(
        dec(&i8(), "128").unwrap_err().problem(),
        Problem::OutOfRange(_)
    ));
    assert_eq!(
        dec(&i64(), "1.5").unwrap_err().problem(),
        &Problem::mismatch("int", "float")
    );
    assert_eq!(dec(&f64(), "1e3").unwrap(), 1000.0);
    assert_eq!(dec(&f64(), "-0.0").unwrap().to_bits(), (-0.0f64).to_bits());
    assert!(matches!(
        dec(&f64(), "1e400").unwrap_err().problem(),
        Problem::OutOfRange(_)
    ));
    assert_eq!(girders_json::encode(&2.0, &f64()).unwrap(), "2.0");
    assert_eq!(girders_json::encode(&1e300, &f64()).unwrap(), "1e300");
    assert_eq!(girders_json::encode(&0.1f32, &f32()).unwrap(), "0.1");
    assert!(girders_json::encode(&f64::NAN, &f64()).is_err());
}

#[test]
fn string_escapes_round_trip() {
    let s = "quote\" back\\ nl\n tab\t ctl\u{1} emoji😀 é".to_string();
    let json = girders_json::encode(&s, &string()).unwrap();
    assert_eq!(json, r#""quote\" back\\ nl\n tab\t ctl\u0001 emoji😀 é""#);
    assert_eq!(dec(&string(), &json).unwrap(), s);
    assert_eq!(dec(&string(), r#""😀é""#).unwrap(), "😀é");
}

#[test]
fn spanned_values() {
    let c = list(spanned(string()));
    let v = dec(&c, r#"[ "ab", "c" ]"#).unwrap();
    assert_eq!(v[0].span, Some(Span::new(2, 6)));
    assert_eq!(v[1].span, Some(Span::new(8, 11)));
    let fast = girders_json::decode_with(r#"["ab"]"#, &c, DecodeOptions::fast()).unwrap();
    assert_eq!(fast[0].span, None);
}

#[test]
fn fast_mode_drops_locations_and_paths() {
    let e = girders_json::decode_with(
        r#"[{"id":"a","age":"x"}]"#,
        &list(user()),
        DecodeOptions::fast(),
    )
    .unwrap_err();
    assert_eq!(e.location(), None);
    assert!(e.path().is_empty());
}

// -- unions ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Item {
    kind: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Thing {
    Bag(Vec<Item>),
    Note(String),
}

fn item() -> impl Codec<Output = Item> {
    // Has its own field called "type": the union's claim must not leak to it.
    record((field("type", string(), |i: &Item| &i.kind),)).build(|(kind,)| Item { kind })
}

fn thing() -> impl Codec<Output = Thing> {
    tagged_union(
        "type",
        (
            variant(
                "bag",
                record((field("items", list(item()), |v: &Vec<Item>| v),)).build(|(items,)| items),
                Thing::Bag,
            ),
            variant(
                "note",
                record((field("text", string(), |s: &String| s),)).build(|(t,)| t),
                Thing::Note,
            ),
        ),
        |t| match t {
            Thing::Bag(items) => Pick2::Variant0(items),
            Thing::Note(t) => Pick2::Variant1(t),
        },
    )
}

#[test]
fn union_claim_is_scoped() {
    let v = dec(&thing(), r#"{"items":[{"type":"x"}],"type":"bag"}"#).unwrap();
    assert_eq!(v, Thing::Bag(vec![Item { kind: "x".into() }]));
    assert_eq!(
        girders_json::encode(&v, &thing()).unwrap(),
        r#"{"type":"bag","items":[{"type":"x"}]}"#
    );
    let e = dec(&thing(), r#"{"text":"t"}"#).unwrap_err();
    assert_eq!(e.problem(), &Problem::MissingField("type".into()));
}

fn thing_adjacent() -> impl Codec<Output = Thing> {
    adjacent_union(
        "kind",
        "data",
        (
            variant("bag", list(item()), Thing::Bag),
            variant("note", string(), Thing::Note),
        ),
        |t| match t {
            Thing::Bag(items) => Pick2::Variant0(items),
            Thing::Note(t) => Pick2::Variant1(t),
        },
    )
}

#[test]
fn adjacent_union_round_trip() {
    let v = dec(&thing_adjacent(), r#"{"data":"hi","kind":"note"}"#).unwrap();
    assert_eq!(v, Thing::Note("hi".into()));
    assert_eq!(
        girders_json::encode(&v, &thing_adjacent()).unwrap(),
        r#"{"kind":"note","data":"hi"}"#
    );
    let e = dec(&thing_adjacent(), r#"{"kind":"note","data":3}"#).unwrap_err();
    assert_eq!(e.path(), vec![key("data")]);
}

#[test]
fn one_of_reports_deepest_error() {
    let c = one_of((map(user(), |u| u.id), map(string(), |s| s)));
    assert_eq!(dec(&c, r#""plain""#).unwrap(), "plain");
    assert_eq!(dec(&c, r#"{"id":"x","age":1}"#).unwrap(), "x");
    let e = dec(&c, r#"{"id":"x","age":"old"}"#).unwrap_err();
    assert_eq!(e.path(), vec![key("age")]);
}

// -- versioning -----------------------------------------------------------------------

#[derive(Debug, PartialEq)]
struct V1 {
    name: String,
}
#[derive(Debug, PartialEq)]
struct V3 {
    full_name: String,
    age: u32,
}

fn v3_codec() -> impl Codec<Output = V3> {
    record((
        field("full_name", string(), |u: &V3| &u.full_name),
        field("age", u32(), |u: &V3| &u.age),
    ))
    .build(|(full_name, age)| V3 { full_name, age })
}

fn versioned_user() -> impl Codec<Output = V3> {
    versioned("schema_version", 3, v3_codec())
        .default_version(1)
        .legacy(
            1,
            map(
                record((field("name", string(), |u: &V1| &u.name),)).build(|(name,)| V1 { name }),
                |v1| V3 {
                    full_name: v1.name,
                    age: 0,
                },
            ),
        )
        .legacy("2-beta", map(v3_codec(), |v| v))
}

#[test]
fn versioned_dispatch() {
    let c = versioned_user();
    assert_eq!(
        dec(&c, r#"{"name":"Ada"}"#).unwrap(),
        V3 {
            full_name: "Ada".into(),
            age: 0
        }
    );
    assert_eq!(
        dec(&c, r#"{"name":"Ada","schema_version":1}"#)
            .unwrap()
            .full_name,
        "Ada"
    );
    assert_eq!(
        dec(
            &c,
            r#"{"full_name":"Bo","age":2,"schema_version":"2-beta"}"#
        )
        .unwrap()
        .age,
        2
    );
    let e = dec(&c, r#"{"schema_version":9}"#).unwrap_err();
    assert_eq!(
        e.problem(),
        &Problem::UnknownVersion {
            found: "9".into(),
            known: vec!["1".into(), "2-beta".into(), "3".into()]
        }
    );
    assert_eq!(e.path(), vec![key("schema_version")]);
    let v = V3 {
        full_name: "Cy".into(),
        age: 4,
    };
    let json = girders_json::encode(&v, &c).unwrap();
    assert_eq!(json, r#"{"schema_version":3,"full_name":"Cy","age":4}"#);
    assert_eq!(dec(&c, &json).unwrap(), v);
}

#[test]
fn versioned_envelope() {
    let c = versioned("version", 2, v3_codec()).envelope("data");
    let v = V3 {
        full_name: "E".into(),
        age: 1,
    };
    let json = girders_json::encode(&v, &c).unwrap();
    assert_eq!(json, r#"{"version":2,"data":{"full_name":"E","age":1}}"#);
    assert_eq!(
        dec(&c, r#"{"data":{"full_name":"E","age":1},"version":2}"#).unwrap(),
        v
    );
}

// -- recursion -------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
struct Tree {
    value: i64,
    children: Vec<Tree>,
}

struct TreeDef;
impl Recursive for TreeDef {
    type Output = Tree;
    fn codec(&self) -> impl Codec<Output = Tree> {
        record((
            field("value", i64(), |t: &Tree| &t.value),
            field("children", list(recursive(TreeDef)), |t: &Tree| &t.children),
        ))
        .build(|(value, children)| Tree { value, children })
    }
}

#[test]
fn recursive_tree() {
    let c = recursive(TreeDef);
    let input = r#"{"value":1,"children":[{"value":2,"children":[]},{"value":3,"children":[{"value":4,"children":[]}]}]}"#;
    let t = dec(&c, input).unwrap();
    assert_eq!(girders_json::encode(&t, &c).unwrap(), input);
    let e = dec(
        &c,
        r#"{"value":1,"children":[{"value":2,"children":[{"value":"x","children":[]}]}]}"#,
    )
    .unwrap_err();
    assert_eq!(
        girders_json::to_pointer(&e).to_string(),
        "/children/0/children/0/value"
    );
}

// -- streaming ---------------------------------------------------------------------------

#[test]
fn ndjson_push_stream() {
    let input = b"{\"id\":\"a\",\"age\":1}\n{\"id\":\"b\",\"age\":2}\n{\"id\":\"c\",\"age\":3}\n";
    for chunk in [1, 3, 10, 1000] {
        let mut d = StreamDecoder::new(user());
        let mut got = Vec::new();
        for c in input.chunks(chunk) {
            d.push(c);
            while let Some(u) = d.next() {
                got.push(u.unwrap().id);
            }
        }
        d.finish();
        while let Some(u) = d.next() {
            got.push(u.unwrap().id);
        }
        assert_eq!(got, ["a", "b", "c"], "chunk {chunk}");
    }
}

#[test]
fn from_reader_any_chunk_size() {
    let input = r#"{"id":"a","age":1,"email":"x@y"}"#;
    for chunk in [1, 2, 5, 64] {
        let u =
            girders_json::from_reader_with(input.as_bytes(), &user(), DecodeOptions::new(), chunk)
                .unwrap();
        assert_eq!(u.email.as_deref(), Some("x@y"));
    }
    let e = girders_json::from_reader_with(
        r#"{"id":"a","age":"x"}"#.as_bytes(),
        &user(),
        DecodeOptions::new(),
        3,
    )
    .unwrap_err();
    assert_eq!(e.location(), Some(&Span::point(16)));
}

#[test]
fn trailing_input_is_an_error() {
    assert!(
        matches!(dec(&u32(), "1 2").unwrap_err().problem(), Problem::Syntax(m) if m.contains("trailing"))
    );
}

// -- diagnostics (snapshots ported from the Gleam birdie suite) -------------------------

fn render<C: Decoder>(c: &C, src: &str) -> String {
    diagnostic::render(&dec(c, src).err().unwrap(), src)
}

#[derive(Debug)]
struct Server {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: i64,
}

fn server() -> impl Codec<Output = Server> {
    record((
        field("host", string(), |s: &Server| &s.host),
        field("port", i64(), |s: &Server| &s.port),
    ))
    .build(|(host, port)| Server { host, port })
}

#[derive(Debug)]
struct Cfg {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    server: Server,
}

fn cfg() -> impl Codec<Output = Cfg> {
    record((
        field("name", string(), |c: &Cfg| &c.name),
        field("server", server(), |c: &Cfg| &c.server),
    ))
    .build(|(name, server)| Cfg { name, server })
}

#[test]
fn diagnostics() {
    let nested = "{\n  \"server\": {\n    \"port\": \"eighty\",\n    \"host\": \"h\"\n  },\n  \"name\": \"n\"\n}";
    insta::assert_snapshot!("render_nested_type_mismatch", render(&cfg(), nested));

    let missing = "{\n  \"server\": {\n    \"host\": \"h\"\n  },\n  \"name\": \"n\"\n}";
    insta::assert_snapshot!(
        "render_missing_field_in_nested_object",
        render(&cfg(), missing)
    );

    let matrix = "{\n  \"matrix\": [\n    [1, 2, 3],\n    [4, 5, 6],\n    [4, 5, \"x\"]\n  ]\n}";
    let m = record((field("matrix", list(list(i64())), |m: &Vec<Vec<i64>>| m),)).build(|(m,)| m);
    insta::assert_snapshot!("render_array_within_object", render(&m, matrix));

    let servers = "[\n  { \"host\": \"a\", \"port\": 1 },\n  { \"host\": \"b\", \"port\": 2 },\n  { \"host\": \"c\", \"port\": \"oops\" }\n]";
    insta::assert_snapshot!(
        "render_object_within_array",
        render(&list(server()), servers)
    );

    insta::assert_snapshot!(
        "render_trailing_comma_syntax_error",
        render(&list(i64()), "[1, 2, ]")
    );

    let with_context = context(
        "in the configuration document",
        record((
            field(
                "server",
                context("while decoding the server entry", server()),
                |c: &Cfg| &c.server,
            ),
            field("name", string(), |c: &Cfg| &c.name),
        ))
        .build(|(server, name)| Cfg { name, server }),
    );
    let e = dec(&with_context, nested).unwrap_err();
    insta::assert_snapshot!(
        "render_detailed_with_context",
        diagnostic::render_detailed(&e, nested)
    );
    insta::assert_snapshot!(
        "render_short_one_liner",
        diagnostic::render_short(&e, nested)
    );
    insta::assert_snapshot!("display_one_liner", e.to_string());
}

#[test]
fn pretty_snapshots() {
    let v: Value =
        girders_json::parse(r#"{"name":"prod","ports":[8080,9090],"meta":{"tls":true},"tags":[]}"#)
            .unwrap();
    insta::assert_snapshot!(
        "pretty_nested_object",
        girders_json::encode_with(
            &v,
            &value(),
            girders_json::EncodeOptions::new().pretty("  ")
        )
        .unwrap()
    );
    let v: Value = girders_json::parse(r#"[1,[2,3],{"k":4},{},[]]"#).unwrap();
    insta::assert_snapshot!(
        "pretty_top_level_array",
        girders_json::encode_with(
            &v,
            &value(),
            girders_json::EncodeOptions::new().pretty("  ")
        )
        .unwrap()
    );
}

#[test]
fn key_spans_only_when_requested() {
    let src = r#"{"a": 1, "bee": {"c": [true]}}"#;
    let v: Value =
        girders_json::decode_with(src, &value(), DecodeOptions::new().key_spans(true)).unwrap();
    let girders_json::Node::Object(m) = &v.node else {
        panic!()
    };
    let span_text = |s: Span| &src[s.start() as usize..s.end() as usize];
    assert_eq!(span_text(m.key_span(0).unwrap()), "\"a\"");
    assert_eq!(span_text(m.key_span(1).unwrap()), "\"bee\"");
    let (inner, bee) = m.get_with_key_span("bee").unwrap();
    assert_eq!(span_text(bee.unwrap()), "\"bee\"");
    let girders_json::Node::Object(inner) = &inner.node else {
        panic!()
    };
    assert_eq!(span_text(inner.key_span(0).unwrap()), "\"c\"");

    let off: Value = girders_json::parse(src).unwrap();
    let girders_json::Node::Object(m) = &off.node else {
        panic!()
    };
    assert_eq!(m.key_span(0), None);
}

#[test]
fn large_objects_use_a_hash_index_and_fall_back_when_keys_collide() {
    use girders_json::{Node, value::keys_forcing_fallback};
    let object = |keys: &[String], options: DecodeOptions| {
        let json = format!(
            "{{{}}}",
            keys.iter()
                .enumerate()
                .map(|(i, k)| format!("{}:{i}", girders_json::encode(k, &string()).unwrap()))
                .collect::<Vec<_>>()
                .join(",")
        );
        girders_json::decode_with(&json, &value(), options).unwrap()
    };
    let check = |v: &Value, keys: &[String], hashed: bool| {
        let Node::Object(m) = &v.node else { panic!() };
        assert_eq!(m.is_hashed(), hashed);
        for (i, k) in keys.iter().enumerate() {
            let found = v.get(k).and_then(|x| match &x.node {
                Node::Number(n) => n.as_i64(),
                _ => None,
            });
            assert_eq!(found, Some(i as i64), "{k:?}");
        }
        assert!(v.get("absent").is_none());
    };
    let normal: Vec<String> = (0..1000).map(|i| format!("key{i}")).collect();
    check(&object(&normal, DecodeOptions::new()), &normal, true);

    // An attacker who knows the hash key forces the sorted fallback, and
    // lookups stay correct. The same keys under any other key don't collide.
    let known = [1, 2];
    let attack = keys_forcing_fallback(1000, known);
    check(
        &object(&attack, DecodeOptions::new().hash_key(known)),
        &attack,
        false,
    );
    check(
        &object(&attack, DecodeOptions::new().hash_key([3, 4])),
        &attack,
        true,
    );
    check(&object(&attack, DecodeOptions::new()), &attack, true);
}
