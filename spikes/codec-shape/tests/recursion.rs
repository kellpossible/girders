//! Macro-free recursion via the `Recursive` trait (option A).
use spike_codec_shape::prelude::*;
use spike_codec_shape::tok::Tok::{self, *};

fn s(x: &str) -> Tok {
    Tok::Str(x.into())
}
fn k(x: &str) -> Tok {
    Key(x.into())
}

// -- Self-recursive record ----------------------------------------------------

#[derive(Debug, PartialEq)]
struct Tree {
    value: i64,
    children: Vec<Tree>,
}

struct TreeDef;
impl Recursive for TreeDef {
    type Output = Tree;
    const NAME: &'static str = "Tree";
    fn codec(&self) -> impl Codec<Output = Tree> {
        record((
            field("value", i64(), |t: &Tree| &t.value),
            field("children", list(recursive(TreeDef)), |t: &Tree| &t.children),
        ))
        .build(|(value, children)| Tree { value, children })
    }
}

fn tree() -> impl Codec<Output = Tree> {
    recursive(TreeDef)
}

#[test]
fn self_recursive() {
    let t = Tree { value: 1, children: vec![Tree { value: 2, children: vec![Tree { value: 3, children: vec![] }] }] };
    assert_eq!(decode(&tree(), encode(&tree(), &t)).unwrap(), t);

    let mut cx = SchemaCx::default();
    assert_eq!(tree().schema(&mut cx), Schema::Ref("Tree"));
    assert_eq!(cx.defs.len(), 1);
    let Some(Schema::Object { fields, .. }) = &cx.defs[0].1 else { panic!() };
    assert_eq!(fields[1].1, Schema::Array(Box::new(Schema::Ref("Tree"))));
}

// -- Recursive tagged union with Box -------------------------------------------

#[derive(Debug, PartialEq)]
struct Num {
    n: i64,
}
#[derive(Debug, PartialEq)]
struct Add {
    l: Box<Expr>,
    r: Box<Expr>,
}
#[derive(Debug, PartialEq)]
enum Expr {
    Num(Num),
    Add(Add),
}

struct ExprDef;
impl Recursive for ExprDef {
    type Output = Expr;
    const NAME: &'static str = "Expr";
    fn codec(&self) -> impl Codec<Output = Expr> {
        tagged_union(
            "op",
            (
                variant("num", record((field("n", i64(), |x: &Num| &x.n),)).build(|(n,)| Num { n }), Expr::Num),
                variant(
                    "add",
                    record((
                        field("l", boxed(recursive(ExprDef)), |a: &Add| &a.l),
                        field("r", boxed(recursive(ExprDef)), |a: &Add| &a.r),
                    ))
                    .build(|(l, r)| Add { l, r }),
                    Expr::Add,
                ),
            ),
            |e| match e {
                Expr::Num(x) => Of2::V0(x),
                Expr::Add(a) => Of2::V1(a),
            },
        )
    }
}

#[test]
fn recursive_union() {
    let e = Expr::Add(Add {
        l: Box::new(Expr::Num(Num { n: 1 })),
        r: Box::new(Expr::Add(Add { l: Box::new(Expr::Num(Num { n: 2 })), r: Box::new(Expr::Num(Num { n: 3 })) })),
    });
    let c = recursive(ExprDef);
    assert_eq!(decode(&c, encode(&c, &e)).unwrap(), e);
}

// -- Mutual recursion -----------------------------------------------------------

#[derive(Debug, PartialEq)]
struct Dir {
    name: String,
    entries: Vec<Entry>,
}
#[derive(Debug, PartialEq)]
struct Entry {
    label: String,
    dir: Option<Dir>,
}

struct DirDef;
struct EntryDef;
impl Recursive for DirDef {
    type Output = Dir;
    const NAME: &'static str = "Dir";
    fn codec(&self) -> impl Codec<Output = Dir> {
        record((
            field("name", string(), |d: &Dir| &d.name),
            field("entries", list(recursive(EntryDef)), |d: &Dir| &d.entries),
        ))
        .build(|(name, entries)| Dir { name, entries })
    }
}
impl Recursive for EntryDef {
    type Output = Entry;
    const NAME: &'static str = "Entry";
    fn codec(&self) -> impl Codec<Output = Entry> {
        record((
            field("label", string(), |e: &Entry| &e.label),
            optional_field("dir", recursive(DirDef), |e: &Entry| &e.dir),
        ))
        .build(|(label, dir)| Entry { label, dir })
    }
}

#[test]
fn mutual_recursion() {
    let d = Dir {
        name: "root".into(),
        entries: vec![
            Entry { label: "a".into(), dir: None },
            Entry { label: "b".into(), dir: Some(Dir { name: "sub".into(), entries: vec![] }) },
        ],
    };
    let c = recursive(DirDef);
    assert_eq!(decode(&c, encode(&c, &d)).unwrap(), d);
    let mut cx = SchemaCx::default();
    c.schema(&mut cx);
    let names: Vec<_> = cx.defs.iter().map(|d| d.0).collect();
    assert_eq!(names, vec!["Dir", "Entry"]);
}

// -- Generic recursion: the def carries a codec for the element type ------------

#[derive(Debug, PartialEq)]
struct GTree<T> {
    value: T,
    children: Vec<GTree<T>>,
}

#[derive(Clone)]
struct GTreeDef<C>(C);
impl<C: Codec + Clone> Recursive for GTreeDef<C>
where
    C::Output: Sized,
{
    type Output = GTree<C::Output>;
    const NAME: &'static str = "GTree";
    fn codec(&self) -> impl Codec<Output = GTree<C::Output>> {
        record((
            field("value", self.0.clone(), |t: &GTree<C::Output>| &t.value),
            field("children", list(recursive(self.clone())), |t: &GTree<C::Output>| &t.children),
        ))
        .build(|(value, children)| GTree { value, children })
    }
}

#[derive(Clone)]
struct StrC;
impl Decoder for StrC {
    type Output = String;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut Cx) -> Res<String, R> {
        string().decode(r, cx)
    }
    fn schema(&self, s: &mut SchemaCx) -> Schema {
        string().schema(s)
    }
}
impl Encoder for StrC {
    type Input = String;
    fn encode<S: Serializer>(&self, v: &String, s: &mut S, cx: &mut Ecx) -> Result<(), S::Error> {
        string().encode(v, s, cx)
    }
}

#[test]
fn generic_recursion() {
    let t = GTree { value: "a".to_string(), children: vec![GTree { value: "b".into(), children: vec![] }] };
    let c = recursive(GTreeDef(StrC));
    assert_eq!(decode(&c, encode(&c, &t)).unwrap(), t);
    let toks = vec![BeginObj, k("value"), s("x"), k("children"), BeginArr, EndArr, EndObj];
    assert_eq!(decode(&c, toks).unwrap().value, "x");
}
