//! Keyed records: typed keys instead of a positional tuple pattern.
use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
struct User {
    id: u32,
    name: String,
    email: Option<String>,
    admin: bool,
}

fn user() -> impl RecordCodec<Output = User> {
    let r = keyed_record::<User>();
    let (r, id) = r.field("id", u32(), |u| &u.id);
    let (r, name) = r.field("name", string(), |u| &u.name);
    let (r, email) = r.optional_field("email", string(), |u| &u.email);
    let (r, admin) = r.field_default("admin", bool(), |u| &u.admin, || false);
    r.build(move |v| User {
        id: v.take(id),
        name: v.take(name),
        email: v.take(email),
        admin: v.take(admin),
    })
}

#[test]
fn round_trip_and_strictness() {
    let u = girders_json::decode(r#"{"name":"ada","id":7}"#, &user()).unwrap();
    assert_eq!(u, User { id: 7, name: "ada".into(), email: None, admin: false });
    assert_eq!(girders_json::encode(&u, &user()), r#"{"id":7,"name":"ada","email":null,"admin":false}"#);
    let e = girders_json::decode(r#"{"id":7}"#, &user()).unwrap_err();
    assert_eq!(e.problem(), &Problem::MissingField("name".into()));
    let e = girders_json::decode(r#"{"id":7,"name":"a","nmae":1}"#, &user()).unwrap_err();
    assert!(matches!(e.problem(), Problem::UnknownField { name, .. } if name == "nmae"));
    let e = girders_json::decode_with(r#"{"id":"x","name":1}"#, &user(), DecodeOptions::strict()).unwrap_err();
    assert_eq!(e.len(), 2);
}

#[derive(Debug, PartialEq)]
struct Page {
    user: User,
    tags: Vec<String>,
}

#[test]
fn fields_of_and_nesting() {
    let page = {
        let r = keyed_record::<Page>();
        let (r, user_k) = r.fields_of(user(), |p| &p.user);
        let (r, tags) = r.field("tags", list(string()), |p| &p.tags);
        r.build(move |v| Page { user: v.take(user_k), tags: v.take(tags) })
    };
    let p = girders_json::decode(r#"{"tags":["x"],"id":1,"name":"n"}"#, &page).unwrap();
    assert_eq!(p.user.id, 1);
    assert_eq!(girders_json::encode(&p, &page), r#"{"id":1,"name":"n","email":null,"admin":false,"tags":["x"]}"#);
}

#[derive(Debug, PartialEq)]
struct Tree {
    value: i64,
    children: Vec<Tree>,
}

struct TreeDef;
impl Recursive for TreeDef {
    type Output = Tree;
    fn codec(&self) -> impl Codec<Output = Tree> {
        let r = keyed_record::<Tree>();
        let (r, value) = r.field("value", i64(), |t| &t.value);
        let (r, children) = r.field("children", list(recursive(TreeDef)), |t| &t.children);
        r.build(move |v| Tree { value: v.take(value), children: v.take(children) })
    }
}

#[test]
fn recursive_keyed() {
    let input = r#"{"value":1,"children":[{"value":2,"children":[]}]}"#;
    let t = girders_json::decode(input, &recursive(TreeDef)).unwrap();
    assert_eq!(girders_json::encode(&t, &recursive(TreeDef)), input);
}

/// 40 fields, flat: no cap, no grouping.
#[derive(Debug, PartialEq, Default)]
struct Wide {
    f00: u8, f01: u8, f02: u8, f03: u8, f04: u8, f05: u8, f06: u8, f07: u8, f08: u8, f09: u8,
    f10: u8, f11: u8, f12: u8, f13: u8, f14: u8, f15: u8, f16: u8, f17: u8, f18: u8, f19: u8,
    f20: u8, f21: u8, f22: u8, f23: u8, f24: u8, f25: u8, f26: u8, f27: u8, f28: u8, f29: u8,
    f30: u8, f31: u8, f32: u8, f33: u8, f34: u8, f35: u8, f36: u8, f37: u8, f38: u8, f39: u8,
}

fn wide() -> impl Codec<Output = Wide> {
    let r = keyed_record::<Wide>();
    let (r, f00) = r.field("f00", u8(), |w| &w.f00);
    let (r, f01) = r.field("f01", u8(), |w| &w.f01);
    let (r, f02) = r.field("f02", u8(), |w| &w.f02);
    let (r, f03) = r.field("f03", u8(), |w| &w.f03);
    let (r, f04) = r.field("f04", u8(), |w| &w.f04);
    let (r, f05) = r.field("f05", u8(), |w| &w.f05);
    let (r, f06) = r.field("f06", u8(), |w| &w.f06);
    let (r, f07) = r.field("f07", u8(), |w| &w.f07);
    let (r, f08) = r.field("f08", u8(), |w| &w.f08);
    let (r, f09) = r.field("f09", u8(), |w| &w.f09);
    let (r, f10) = r.field("f10", u8(), |w| &w.f10);
    let (r, f11) = r.field("f11", u8(), |w| &w.f11);
    let (r, f12) = r.field("f12", u8(), |w| &w.f12);
    let (r, f13) = r.field("f13", u8(), |w| &w.f13);
    let (r, f14) = r.field("f14", u8(), |w| &w.f14);
    let (r, f15) = r.field("f15", u8(), |w| &w.f15);
    let (r, f16) = r.field("f16", u8(), |w| &w.f16);
    let (r, f17) = r.field("f17", u8(), |w| &w.f17);
    let (r, f18) = r.field("f18", u8(), |w| &w.f18);
    let (r, f19) = r.field("f19", u8(), |w| &w.f19);
    let (r, f20) = r.field("f20", u8(), |w| &w.f20);
    let (r, f21) = r.field("f21", u8(), |w| &w.f21);
    let (r, f22) = r.field("f22", u8(), |w| &w.f22);
    let (r, f23) = r.field("f23", u8(), |w| &w.f23);
    let (r, f24) = r.field("f24", u8(), |w| &w.f24);
    let (r, f25) = r.field("f25", u8(), |w| &w.f25);
    let (r, f26) = r.field("f26", u8(), |w| &w.f26);
    let (r, f27) = r.field("f27", u8(), |w| &w.f27);
    let (r, f28) = r.field("f28", u8(), |w| &w.f28);
    let (r, f29) = r.field("f29", u8(), |w| &w.f29);
    let (r, f30) = r.field("f30", u8(), |w| &w.f30);
    let (r, f31) = r.field("f31", u8(), |w| &w.f31);
    let (r, f32) = r.field("f32", u8(), |w| &w.f32);
    let (r, f33) = r.field("f33", u8(), |w| &w.f33);
    let (r, f34) = r.field("f34", u8(), |w| &w.f34);
    let (r, f35) = r.field("f35", u8(), |w| &w.f35);
    let (r, f36) = r.field("f36", u8(), |w| &w.f36);
    let (r, f37) = r.field("f37", u8(), |w| &w.f37);
    let (r, f38) = r.field("f38", u8(), |w| &w.f38);
    let (r, f39) = r.field("f39", u8(), |w| &w.f39);
    r.build(move |v| Wide {
        f00: v.take(f00), f01: v.take(f01), f02: v.take(f02), f03: v.take(f03), f04: v.take(f04),
        f05: v.take(f05), f06: v.take(f06), f07: v.take(f07), f08: v.take(f08), f09: v.take(f09),
        f10: v.take(f10), f11: v.take(f11), f12: v.take(f12), f13: v.take(f13), f14: v.take(f14),
        f15: v.take(f15), f16: v.take(f16), f17: v.take(f17), f18: v.take(f18), f19: v.take(f19),
        f20: v.take(f20), f21: v.take(f21), f22: v.take(f22), f23: v.take(f23), f24: v.take(f24),
        f25: v.take(f25), f26: v.take(f26), f27: v.take(f27), f28: v.take(f28), f29: v.take(f29),
        f30: v.take(f30), f31: v.take(f31), f32: v.take(f32), f33: v.take(f33), f34: v.take(f34),
        f35: v.take(f35), f36: v.take(f36), f37: v.take(f37), f38: v.take(f38), f39: v.take(f39),
    })
}

#[test]
fn forty_fields_flat() {
    let w = Wide { f39: 39, f00: 1, ..Default::default() };
    let json = girders_json::encode(&w, &wide());
    assert_eq!(girders_json::decode(&json, &wide()).unwrap(), w);
}
