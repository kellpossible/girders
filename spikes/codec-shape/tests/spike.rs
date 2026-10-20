use spike_codec_shape::prelude::*;
use Tok::*;

fn s(x: &str) -> Tok {
    Tok::Str(x.into())
}
fn k(x: &str) -> Tok {
    Key(x.into())
}

// -- Q: does the applicative record builder infer cleanly? --------------------

#[derive(Debug, Clone, PartialEq)]
struct User {
    id: String,
    age: u32,
    email: Option<String>,
}

fn user() -> impl Codec<Output = User> {
    record((
        field("id", string(), |u: &User| &u.id),
        field("age", u32(), |u: &User| &u.age),
        optional_field("email", string(), |u: &User| &u.email),
    ))
    .build(|(id, age, email)| User { id, age, email })
}

#[test]
fn record_roundtrip_any_field_order() {
    let toks = vec![BeginObj, k("age"), Int(30), k("id"), s("u1"), EndObj];
    let u = decode(&user(), toks).unwrap();
    assert_eq!(u, User { id: "u1".into(), age: 30, email: None });
    let enc = encode(&user(), &u);
    assert_eq!(enc, vec![BeginObj, k("id"), s("u1"), k("age"), Int(30), EndObj]);
    assert_eq!(decode(&user(), enc).unwrap(), u);
}

#[test]
fn record_strict_errors() {
    let unknown = vec![BeginObj, k("id"), s("u1"), k("age"), Int(1), k("nmae"), s("x"), EndObj];
    assert_eq!(decode(&user(), unknown).unwrap_err().problem, Problem::UnknownField("nmae".into()));

    let dup = vec![BeginObj, k("id"), s("a"), k("id"), s("b"), k("age"), Int(1), EndObj];
    assert_eq!(decode(&user(), dup).unwrap_err().problem, Problem::DuplicateField("id"));

    let missing = vec![BeginObj, k("id"), s("a"), EndObj];
    assert_eq!(decode(&user(), missing).unwrap_err().problem, Problem::MissingField("age"));

    let range = vec![BeginObj, k("id"), s("a"), k("age"), Int(-1), EndObj];
    let e = decode(&user(), range).unwrap_err();
    assert_eq!((e.problem.clone(), e.path()), (Problem::OutOfRange, vec![Seg::Key("age".into())]));
}

#[test]
fn record_allow_unknown() {
    let lenient = record((field("id", string(), |u: &User| &u.id),))
        .build(|(id,)| User { id, age: 0, email: None })
        .allow_unknown();
    let toks = vec![BeginObj, k("x"), BeginArr, Int(1), EndArr, k("id"), s("a"), EndObj];
    assert_eq!(decode(&lenient, toks).unwrap().id, "a");
}

#[test]
fn nested_error_path() {
    let users = list(user());
    let toks = vec![
        BeginArr,
        BeginObj, k("id"), s("a"), k("age"), Int(1), EndObj,
        BeginObj, k("id"), s("b"), k("age"), s("old"), EndObj,
        EndArr,
    ];
    let e = decode(&users, toks).unwrap_err();
    assert_eq!(e.path(), vec![Seg::Index(1), Seg::Key("age".into())]);
    assert_eq!(e.location, Some(11));
}

// -- Q: arity 16 -----------------------------------------------------------

#[derive(Debug, PartialEq)]
struct Wide {
    f0: u8, f1: u8, f2: u8, f3: u8, f4: u8, f5: u8, f6: u8, f7: u8,
    f8: u8, f9: u8, f10: u8, f11: u8, f12: u8, f13: u8, f14: u8, f15: u8,
}

fn wide() -> impl Codec<Output = Wide> {
    record((
        field("f0", u8(), |w: &Wide| &w.f0),
        field("f1", u8(), |w: &Wide| &w.f1),
        field("f2", u8(), |w: &Wide| &w.f2),
        field("f3", u8(), |w: &Wide| &w.f3),
        field("f4", u8(), |w: &Wide| &w.f4),
        field("f5", u8(), |w: &Wide| &w.f5),
        field("f6", u8(), |w: &Wide| &w.f6),
        field("f7", u8(), |w: &Wide| &w.f7),
        field("f8", u8(), |w: &Wide| &w.f8),
        field("f9", u8(), |w: &Wide| &w.f9),
        field("f10", u8(), |w: &Wide| &w.f10),
        field("f11", u8(), |w: &Wide| &w.f11),
        field("f12", u8(), |w: &Wide| &w.f12),
        field("f13", u8(), |w: &Wide| &w.f13),
        field("f14", u8(), |w: &Wide| &w.f14),
        field("f15", u8(), |w: &Wide| &w.f15),
    ))
    .build(|(f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15)| Wide {
        f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15,
    })
}

#[test]
fn arity_16_roundtrip() {
    let w = Wide {
        f0: 0, f1: 1, f2: 2, f3: 3, f4: 4, f5: 5, f6: 6, f7: 7,
        f8: 8, f9: 9, f10: 10, f11: 11, f12: 12, f13: 13, f14: 14, f15: 15,
    };
    assert_eq!(decode(&wide(), encode(&wide(), &w)).unwrap(), w);
}

// -- Q: recursive impl Codec through a fn item ---------------------------------

#[derive(Debug, PartialEq)]
struct Tree {
    value: i64,
    children: Vec<Tree>,
}

/// The recursion is broken by a *nominal* type: `TreeCodec`'s impls have
/// no structural where-clauses, so proving `TreeCodec: Decoder` doesn't
/// require proving the body's type, which contains `TreeCodec`. A purely
/// structural `recursive(name, tree)` overflows the trait solver (E0275),
/// because `Record<..List<Recursive<tree>>..>: Decoder` depends on itself.
struct TreeCodec;

fn tree() -> TreeCodec {
    TreeCodec
}

fn tree_body() -> impl Codec<Output = Tree> {
    record((
        field("value", i64(), |t: &Tree| &t.value),
        field("children", list(named("Tree", tree())), |t: &Tree| &t.children),
    ))
    .build(|(value, children)| Tree { value, children })
}

impl Decoder for TreeCodec {
    type Output = Tree;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<Tree, R> {
        tree_body().decode(r, cx)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        tree_body().schema(s)
    }
}
impl Encoder for TreeCodec {
    type Input = Tree;
    fn encode<S: Serializer>(&self, v: &Tree, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        tree_body().encode(v, s, cx)
    }
}

#[test]
fn recursive_roundtrip_and_schema() {
    let t = Tree {
        value: 1,
        children: vec![Tree { value: 2, children: vec![] }, Tree {
            value: 3,
            children: vec![Tree { value: 4, children: vec![] }],
        }],
    };
    assert_eq!(decode(&tree(), encode(&tree(), &t)).unwrap(), t);

    let mut cx = SchemaCx::default();
    let schema = tree().schema(&mut cx);
    // Terminates; the recursive position is a Ref with the body registered once.
    let Schema::Object { fields, .. } = schema else { panic!() };
    assert_eq!(fields[1].1, Schema::Array(Box::new(Schema::Ref("Tree"))));
    assert_eq!(cx.defs.len(), 1);
    assert!(cx.defs[0].1.is_some());
}

// -- Q: tagged union with exhaustive-match encoding ----------------------------

#[derive(Debug, PartialEq)]
struct Circle {
    r: i64,
}
#[derive(Debug, PartialEq)]
struct Square {
    side: i64,
}
#[derive(Debug, PartialEq)]
enum Shape {
    Circle(Circle),
    Square(Square),
}

fn shape() -> impl Codec<Output = Shape> {
    tagged_union(
        "type",
        (
            variant("circle", record((field("r", i64(), |c: &Circle| &c.r),)).build(|(r,)| Circle { r }), Shape::Circle),
            variant("square", record((field("side", i64(), |q: &Square| &q.side),)).build(|(side,)| Square { side }), Shape::Square),
        ),
        |s: &Shape| match s {
            Shape::Circle(c) => Of2::V0(c),
            Shape::Square(q) => Of2::V1(q),
        },
    )
}

#[test]
fn union_tag_anywhere_and_claimed() {
    // Tag after the body field: needs the checkpoint scan; the strict
    // variant record must not reject the claimed "type" key.
    let toks = vec![BeginObj, k("side"), Int(2), k("type"), s("square"), EndObj];
    assert_eq!(decode(&shape(), toks).unwrap(), Shape::Square(Square { side: 2 }));

    let enc = encode(&shape(), &Shape::Circle(Circle { r: 5 }));
    assert_eq!(enc, vec![BeginObj, k("type"), s("circle"), k("r"), Int(5), EndObj]);

    let bad = vec![BeginObj, k("type"), s("hexagon"), EndObj];
    let e = decode(&shape(), bad).unwrap_err();
    assert_eq!(e.problem, Problem::UnknownVariant("hexagon".into()));
}

// -- Q: versioned legacy decoders without fixing the reader type ---------------

#[derive(Debug, PartialEq)]
struct UserV1 {
    name: String,
}
#[derive(Debug, PartialEq)]
struct UserV2 {
    full_name: String,
}
#[derive(Debug, PartialEq)]
struct UserV3 {
    full_name: String,
    age: u32,
}
impl From<UserV1> for UserV2 {
    fn from(v: UserV1) -> Self {
        UserV2 { full_name: v.name }
    }
}
impl From<UserV2> for UserV3 {
    fn from(v: UserV2) -> Self {
        UserV3 { full_name: v.full_name, age: 0 }
    }
}

fn user_v1() -> impl Codec<Output = UserV1> {
    record((field("name", string(), |u: &UserV1| &u.name),)).build(|(name,)| UserV1 { name })
}
fn user_v2() -> impl Codec<Output = UserV2> {
    record((field("full_name", string(), |u: &UserV2| &u.full_name),))
        .build(|(full_name,)| UserV2 { full_name })
}
fn user_v3() -> impl Codec<Output = UserV3> {
    record((
        field("full_name", string(), |u: &UserV3| &u.full_name),
        field("age", u32(), |u: &UserV3| &u.age),
    ))
    .build(|(full_name, age)| UserV3 { full_name, age })
}

fn user_versioned() -> impl Codec<Output = UserV3> {
    versioned("schema_version", 3, user_v3())
        .default_version(1)
        .legacy(1, map(map(user_v1(), UserV2::from), UserV3::from))
        .legacy(2, map(user_v2(), UserV3::from))
}

#[test]
fn versioned_dispatch_and_migrate() {
    let unversioned = vec![BeginObj, k("name"), s("Ada"), EndObj];
    assert_eq!(decode(&user_versioned(), unversioned).unwrap(), UserV3 { full_name: "Ada".into(), age: 0 });

    let v2_tag_last = vec![BeginObj, k("full_name"), s("Bo"), k("schema_version"), Int(2), EndObj];
    assert_eq!(decode(&user_versioned(), v2_tag_last).unwrap().full_name, "Bo");

    let v9 = vec![BeginObj, k("schema_version"), Int(9), EndObj];
    assert_eq!(decode(&user_versioned(), v9).unwrap_err().problem, Problem::UnknownVersion(9));

    let latest = UserV3 { full_name: "Cy".into(), age: 4 };
    let enc = encode(&user_versioned(), &latest);
    assert_eq!(enc, vec![BeginObj, k("schema_version"), Int(3), k("full_name"), s("Cy"), k("age"), Int(4), EndObj]);
    assert_eq!(decode(&user_versioned(), enc).unwrap(), latest);
}

// -- Q: fields_of (explicit flatten) ------------------------------------------

#[derive(Debug, PartialEq)]
struct Pagination {
    page: u32,
    per_page: u32,
}
#[derive(Debug, PartialEq)]
struct Page {
    pagination: Pagination,
    items: Vec<String>,
}

fn pagination() -> Record<impl Fields<Rec = Pagination, Values = (u32, u32)>, impl Fn((u32, u32)) -> Pagination> {
    record((
        field("page", u32(), |p: &Pagination| &p.page),
        field("per_page", u32(), |p: &Pagination| &p.per_page),
    ))
    .build(|(page, per_page)| Pagination { page, per_page })
}

fn page() -> impl Codec<Output = Page> {
    record((
        fields_of(pagination(), |p: &Page| &p.pagination),
        field("items", list(string()), |p: &Page| &p.items),
    ))
    .build(|(pagination, items)| Page { pagination, items })
}

#[test]
fn fields_of_flat_object() {
    let toks = vec![BeginObj, k("items"), BeginArr, s("a"), EndArr, k("per_page"), Int(10), k("page"), Int(2), EndObj];
    let p = decode(&page(), toks).unwrap();
    assert_eq!(p, Page { pagination: Pagination { page: 2, per_page: 10 }, items: vec!["a".into()] });
    assert_eq!(decode(&page(), encode(&page(), &p)).unwrap(), p);
}

// -- Q: one codec value, two formats with different Location types -------------

mod env {
    //! Flat env-var "format": an object of string values; Location = var name.
    use spike_codec_shape::prelude::*;

    pub struct EnvReader {
        vars: Vec<(String, String)>,
        i: usize,
        in_value: bool,
    }
    impl EnvReader {
        pub fn new(vars: &[(&str, &str)]) -> Self {
            EnvReader { vars: vars.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), i: 0, in_value: false }
        }
        fn here(&self) -> String {
            self.vars.get(self.i).map(|v| v.0.clone()).unwrap_or_default()
        }
        fn take(&mut self) -> String {
            self.in_value = false;
            let v = self.vars[self.i].1.clone();
            self.i += 1;
            v
        }
    }
    impl Reader for EnvReader {
        type Location = String;
        type Checkpoint = (usize, bool);
        fn peek_kind(&mut self) -> Res<Kind, Self> {
            Ok(if self.in_value { Kind::String } else { Kind::Object })
        }
        fn read_str(&mut self, buf: &mut String) -> Res<(), Self> {
            buf.push_str(&self.take());
            Ok(())
        }
        fn read_i64(&mut self) -> Res<i64, Self> {
            let at = self.here();
            let v = self.take();
            v.parse().map_err(|_| Error::at(Problem::TypeMismatch { expected: "integer", found: Kind::String }, at))
        }
        fn read_null(&mut self) -> Res<(), Self> {
            unimplemented!()
        }
        fn begin_array(&mut self) -> Res<(), Self> {
            unimplemented!()
        }
        fn array_has_next(&mut self, _: bool) -> Res<bool, Self> {
            unimplemented!()
        }
        fn end_array(&mut self) -> Res<(), Self> {
            unimplemented!()
        }
        fn begin_object(&mut self) -> Res<(), Self> {
            Ok(())
        }
        fn object_next_key(&mut self, _: bool, buf: &mut String) -> Res<bool, Self> {
            match self.vars.get(self.i) {
                None => Ok(false),
                Some((name, _)) => {
                    buf.push_str(&name.strip_prefix("APP_").unwrap_or(name).to_lowercase());
                    self.in_value = true;
                    Ok(true)
                }
            }
        }
        fn end_object(&mut self) -> Res<(), Self> {
            Ok(())
        }
        fn skip_value(&mut self) -> Res<(), Self> {
            self.take();
            Ok(())
        }
        fn checkpoint(&mut self) -> (usize, bool) {
            (self.i, self.in_value)
        }
        fn rewind(&mut self, cp: &(usize, bool)) {
            (self.i, self.in_value) = *cp;
        }
        fn release(&mut self, _: (usize, bool)) {}
    }
}

#[test]
fn same_codec_env_format_location_is_var_name() {
    let mut r = env::EnvReader::new(&[("APP_ID", "u9"), ("APP_AGE", "forty")]);
    let e = user().decode(&mut r, &mut Cx::default()).unwrap_err();
    assert_eq!(e.location.as_deref(), Some("APP_AGE"));
    assert_eq!(e.path(), vec![Seg::Key("age".into())]);

    let mut r = env::EnvReader::new(&[("APP_AGE", "40"), ("APP_ID", "u9")]);
    assert_eq!(user().decode(&mut r, &mut Cx::default()).unwrap().age, 40);
}
