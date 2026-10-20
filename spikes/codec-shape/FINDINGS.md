# Spike: codec shape — findings

Throwaway crate testing the risky type-level parts of DESIGN.md with
real rustc (1.96, edition 2024). It uses a token-stream "format"
(`src/tok.rs`) instead of JSON. `cargo test` runs 14 tests, all
passing.

| # | Question | Result |
|---|---|---|
| 1 | Applicative tuple record builder: inference, one-pass slot decoding | **Works.** Fields can come in any order, with no snapshots. Unknown and duplicate fields are errors, missing fields are reported, and paths are kept. |
| 2 | Format-agnostic `impl Codec<Output = T>` values | **Works.** The same `user()` decodes from `TokReader` (Location = token index) and from a test `EnvReader` (Location = variable name, e.g. `APP_AGE`). |
| 3 | Arity 16 | **Works.** Needs a `new_slots()` method, because std implements `Default` for tuples only up to 12. Compiling a 16-field record costs about 30–50 ms more than an empty test crate (debug). |
| 4 | Recursive `impl Codec` through a fn item | **Fails** with E0275: the trait solver overflows. **Solved without macros** by a `Recursive` trait (option A, below). |
| 5 | Versioned legacy decoders without fixing the reader type | **Works.** A type-level cons list `(Legacy<D>, Rest)`, with no boxing and no dyn. |
| 6 | Tagged union encode with an exhaustive `match` | **Works.** The projection closure returns a positional `Of2::V0(c)` / `Of2::V1(q)`. rustc checks exhaustiveness and the payload types, and the closure doesn't need a type annotation. |
| 7 | Tag key anywhere in the object, while streaming-shaped | **Works.** Scan under a checkpoint, rewind, and "claim" the key so the strict variant record skips it. |
| 8 | Tagging an arbitrary codec when encoding | First tried an `InjectTag<S>` serializer wrapper. That **broke recursive unions** with infinite monomorphization (see below). Replaced by an `Ecx` encode context carrying `pending_tag`, which the next record writes first. |
| 9 | `fields_of` | **Works**, but the reused record's return type is verbose (see below). |

## Recursion (question 4)

`Record<…List<Recursive<tree>>…>: Decoder` needs
`Recursive<tree>: Decoder`, which needs `tree`'s output type to be a
Decoder, and that is the same `Record<…>` again. That is an inductive
cycle. rustc only treats cycles coinductively for auto traits, so it
reports overflow. Splitting the definition across two functions
doesn't help, because the cycle is in trait solving, not in revealing
the opaque type.

**What works:** a *nominal* type whose impls have no structural
where-clauses:

```rust
struct TreeCodec;
fn tree_body() -> impl Codec<Output = Tree> { /* … list(named("Tree", TreeCodec)) … */ }
impl Decoder for TreeCodec { type Output = Tree; fn decode<R: Reader>(…) { tree_body().decode(r, cx) } fn schema(…) { … } }
impl Encoder for TreeCodec { type Input = Tree; fn encode<S: Serializer>(…) { tree_body().encode(v, s) } }
```

That's about 15 lines of boilerplate per recursive type. Options:
- (a) Accept it.
- (b) Ship a small library `macro_rules!` (`recursive_codec!(TreeCodec: Tree = tree_body)`).
  Note this is a declarative macro, not a proc-macro or derive.
- (c) A boxed codec tied to one format:
  `recursive::<JsonReader<_>>(…)`. `dyn` is nominal, so it breaks the
  cycle too.

`named(name, codec)` covers the schema side: it emits `Ref(name)` and
registers the body once. That part works (test `recursive_roundtrip_and_schema`).

## Recursion, option A: a `Recursive` trait (no macros)

```rust
pub trait Recursive {
    type Output;
    const NAME: &'static str;               // schema Ref name
    fn codec(&self) -> impl Codec<Output = Self::Output>;   // return-position impl Trait in a trait (stable since 1.75)
}
pub fn recursive<D: Recursive>(def: D) -> Rec<D>;
```

Proving `Rec<D>: Decoder` needs only `D: Recursive`. That's a nominal
fact, so the trait solver never walks the body and there's no cycle.
User cost is 5 lines on top of the codec body. It stays format-agnostic,
there's no `dyn`, and nothing is allocated. Its schema emits `Ref(NAME)`
and registers the body once.

Tests in `tests/recursion.rs`, all passing:
- a self-recursive record (`Tree`);
- a recursive tagged union using `Box` (`Expr`);
- mutual recursion (`Dir` ↔ `Entry`);
- generic recursion where the def carries a codec (`GTreeDef<C>`).
  This needs `C: Clone`, so primitive codecs should derive `Clone`
  (`Str` didn't in the spike, which is why a `StrC` wrapper exists).

Other options considered:
- **B: hand-written unit struct.** Works, about 15 lines. Option A
  supersedes it.
- **C: boxed, tied to one format.** Not needed.
- **D: fixpoint with a type-erased back-reference.** Ruled out: it needs
  `unsafe`, and combinators that change `R` or `S` would make the erased
  pointer unsound.

## Rule: combinators must not change `R` or `S` for inner codecs

The first union encoder wrapped the serializer (`InjectTag<S>`).
Through a recursive union, each level instantiates
`InjectTag<InjectTag<…<S>>>`, and rustc stops with "reached the
recursion limit while instantiating …". Per-call state has to travel in
context parameters instead: `Cx` for decoding (`claimed`) and `Ecx` for
encoding (`pending_tag`). The same applies to readers. No reader
adapters inside combinators.

## Ergonomics notes

- **Getters need a type annotation** on the closure (`|u: &User| &u.id`).
  Without it rustc says "type annotations needed for `&_`", with a clear
  hint. Possible fix: `record::<User>(|f| (f.field("id", string(), |u| &u.id), …))`,
  where a typed field factory supplies `Rec`. Not yet tried.
- **Reusing a record in `fields_of`** needs its concrete `Record` type,
  because `impl Codec` hides `.fields`:
  `fn pagination() -> Record<impl Fields<Rec = Pagination, Values = (u32, u32)>, impl Fn((u32, u32)) -> Pagination>`.
  Fix: a `RecordCodec: Codec` trait that exposes the field set, so the
  signature becomes `-> impl RecordCodec<Output = Pagination>`.
- **`Cx::claimed` leaks.** If the selected variant's codec isn't a
  record, the claim stays set and the next nested record would skip
  that key. In the real design the claim has to be scoped: cleared by
  every non-record decoder, or passed explicitly to the object-level
  decode.

## Compile errors for common mistakes (rustc 1.96)

| Mistake | Error |
|---|---|
| Getter reads the wrong field (different type) | E0308 `expected &u32, found &String`, pointing at the getter. |
| Field left out of the tuple | E0308 at `build`: `expected a tuple with 2 elements, found one with 3`. |
| Getter missing its annotation | E0282 `type annotations needed for &_`, with a suggested fix. |
| Union projection not exhaustive | E0004 `` `&Shape::Square(_)` not covered ``. |
| Union projection positions swapped | E0308 `expected &Square, found &Circle`. |

All of these point at the user's own code. None of them dump long
combinator types. The only long-type error seen was the E0275 recursion
overflow.

## Compile time (16-core box, debug unless noted)

| Build | Time |
|---|---|
| Rebuild spike test crate (5 records, union, versioned, recursion, env reader) | ~0.21 s |
| Same, release | ~0.83 s |
| Rebuild test crate with only the 16-field record | ~0.15–0.18 s |
| Rebuild empty test crate (baseline) | ~0.12–0.14 s |
