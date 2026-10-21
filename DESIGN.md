# girders: Design

Status: **draft for review, with a throwaway implementation.**

- `girders` and `girders-json` implement this design, including the
  recommended answers to the open questions, so those answers could be
  tested.
- Measurements are in `RESULTS.md`.
- §14 lists what the evidence settled and what still needs a decision.
- §15 lists where the implementation departs from the text above.
decision. Everything else reflects decisions already made or a
recommendation that is accepted unless challenged.

girders is a Rust port of the Gleam library `girder` (`../girder`): a
bidirectional, format-agnostic serialization library built from
combinators, with first-class spans and paths, strict defaults, and
streaming input. The design rationale lives in
`../girder/serde-for-gleam-design.md` (cited below as "design report §n").
This document covers what changes in Rust and why.

---

## 1. Purpose and goals

This is an **experiment and comparison**. We want to see how girder's
design carries over to Rust, then measure ergonomics, runtime
performance and compile time against the Gleam version and against
`serde` + `serde_json`. Polished documentation, MSRV policy and API
stability are lower priorities than a clear design and honest
measurements.

### 1.1 Goals (kept from girder)

- **Bidirectional codecs are the main surface.** A codec declared once
  decodes, encodes and describes its schema, so the three cannot drift
  (design report §8.5, A).
- **Format-agnostic core.** `girders` knows nothing about JSON. A format
  implements the `Reader` and `Serializer` traits.
- **Errors carry a logical path, a location and context.** What a
  location means is configurable per format: byte spans for JSON, a
  variable name for an env-var reader, and so on (notes1).
- **Strict by default.** Unknown fields, duplicate keys and invalid
  input are errors unless the codec opts out.
- **Streaming designed in from the start**, not bolted on later.
- **The cost of diagnostics is optional.** Span tracking, path tracking
  and error accumulation can each be switched off.
- **Codecs are plain values.** Combinators only: no proc-macros, no
  derive, no `build.rs`.

### 1.2 Hard constraints

- **Zero runtime or build dependencies.** `girders` and `girders-json`
  use std only: empty `[dependencies]`, no build scripts. This affects
  several choices below (own ordered map, hand-written diagnostics, std
  float parsing and formatting).
- **No serde.** girders has its own data model. Optional serde interop
  is possible later, behind a feature flag.
- Dev-dependencies (tests and benchmarks) are allowed, but kept small.
  They are listed in §11.

### 1.3 Non-goals for v1

- Async decoding (notes1: keep validation synchronous).
- Zero-copy and borrowed output (§4.5).
- Schema-first code generation (§7.6).
- The "editable document" entry point (design report §9.9, path 3).
- Writing old versions of a versioned codec (§8.5).

---

## 2. Scope of v1

Parity with the Gleam implementation, **plus** the parts that were
designed but never built:

| Area | Gleam status | girders v1 |
|---|---|---|
| Spans, `Spanned<T>` | built | port |
| Error model (path, location, problem, context) | built | port, generic location (§5) |
| `Reader` / `Serializer` contracts | built | traits (§4) |
| Decode/encode/codec combinators | built | port (§6) |
| Record builder | built (monadic `use`-chain) | **redesigned**, applicative (§6.3) |
| `one_of`, `tagged_union` | built | port, plus checkpoints (§4.3) |
| JSON cursor / reader | built | port (§9.1) |
| JSON compact and pretty serializer | built | port |
| `Value` tree with spans | built | port, map type chosen by the caller (§9.3) |
| JSON Pointer (RFC 6901) | built | port |
| Diagnostics (line/col, caret snippet) | built | port, hand-written (§9.5) |
| 7 example programs | built | port (§11.3) |
| JSONTestSuite conformance | built | port |
| **Streaming driver** | not built | new (§4.4) |
| **Accumulating errors** | not built | new (§5.3) |
| **Strict unknown fields**, `allow_unknown`, extras | not built | new (§6.4) |
| **`fields_of`** (explicit flatten) | not built | new (§6.5) |
| **Schema** (`SchemaNode`, JSON Schema emitter) | not built | new (§7) |
| **Versioning** (`versioned`) | not built | new (§8) |
| **Decode options** (spans/paths/accumulate/depth) | not built | new (§5.4) |
| **Benchmarks**, runtime and compile time vs serde_json | not built | new (§12) |

---

## 3. Workspace layout

```
girder-rs/                    (directory name unchanged)
  Cargo.toml                  [workspace], resolver 3, edition 2024
  mise.toml                   tools + tasks (§13)
  DESIGN.md
  girders/                    core crate, published as `girders`
    src/
      lib.rs
      span.rs                 Span, Spanned<T>
      error.rs                DecodeError<L>, Problem, PathSegment, Errors
      options.rs              DecodeOptions
      reader.rs               trait Reader, Kind, Checkpoint
      serializer.rs           trait Serializer
      decode.rs               trait Decoder + primitive/structural combinators
      encode.rs               trait Encoder + combinators
      codec.rs                trait Codec, bimap, try_bimap, list, optional, map...
      record.rs               applicative record builder, tuple impls (macro_rules!)
      union.rs                tagged_union, adjacent_union, one_of
      versioned.rs            versioned codec
      source.rs               byte sources shared by JSON and CBOR: SliceSource, ReadSource<R>, PushSource
      schema.rs               SchemaNode, SchemaCx, refinements
  girders-cbor/               CBOR format (§10), published as `girders-cbor`
  girders-json/               JSON format, published as `girders-json`
    src/
      lib.rs                  decode/encode/parse entry points
      cursor.rs               strict RFC 8259 byte cursor
      source.rs               re-exports girders::source
      reader.rs               impl Reader for JsonReader<S>
      serializer.rs           compact / pretty, over io::Write or Vec<u8>
      value.rs                Value<M>, ObjectMap trait, OrderedMap
      pointer.rs              RFC 6901
      diagnostic.rs           line/col resolution, caret rendering
      schema.rs               SchemaNode -> JSON Schema emitter
    examples/                 the 7 ported examples
    tests/                    conformance, snapshots, property tests
  benches/                    gungraun runtime benchmarks (workspace member, publish = false)
  bench-fixtures/             compile-time benchmark crates (publish = false)
    models-serde/
    models-girders/
```

Crate names checked as free on crates.io on 2026-10-20: `girders` and
`girders-json`. (`girder` is taken.)

---

## 4. Format contract

### 4.1 Gleam to Rust mapping

Gleam's `Reader(src)` is a record of functions over an **immutable**
cursor that is threaded through every call. Rust has traits and `&mut`,
so:

- `Reader` is a **trait** implemented by a mutable cursor type.
  Primitives take `&mut self`.
- Gleam got "go back to an earlier cursor" for free from persistence.
  In Rust this is an explicit **checkpoint** API (§4.3), which matters
  for streaming because a checkpoint pins buffered bytes.

### 4.2 `Reader`

```rust
pub trait Reader {
    /// What an error points at. JSON: `Span` (byte range). An env-var
    /// reader: the variable name. A binary format might use `()`.
    type Location: Clone + Debug;
    type Checkpoint;

    fn peek_kind(&mut self) -> Result<Spanned<Kind, Self::Location>, DecodeError<Self::Location>>;

    fn read_bool(&mut self) -> R<bool>;
    fn read_null(&mut self) -> R<()>;
    fn read_str(&mut self, buf: &mut String) -> R<()>;   // appends; caller owns allocation
    fn read_i64(&mut self) -> R<i64>;
    fn read_u64(&mut self) -> R<u64>;
    fn read_f64(&mut self) -> R<f64>;
    fn read_number_text(&mut self, buf: &mut String) -> R<()>; // raw lexeme, for i128/big ints

    fn begin_array(&mut self) -> R<()>;
    fn array_has_next(&mut self, first: bool) -> R<bool>;
    fn end_array(&mut self) -> R<()>;
    fn begin_object(&mut self) -> R<()>;
    fn object_next_key(&mut self, first: bool, buf: &mut String) -> R<bool>;
    fn end_object(&mut self) -> R<()>;
    fn skip_value(&mut self) -> R<()>;

    fn checkpoint(&mut self) -> Self::Checkpoint;
    fn rewind(&mut self, cp: &Self::Checkpoint);
    fn release(&mut self, cp: Self::Checkpoint);

    /// Location of the value most recently read, if tracking is on.
    fn last_location(&self) -> Option<Self::Location>;
}
// R<T> = Result<T, DecodeError<Self::Location>>
```

Notes:

- **`Kind`** is the same enum as in Gleam: String, Number, Bool, Null,
  Array, Object, Eof.
- **Strings are read into a buffer the caller supplies.** That is the
  natural shape for a streaming reader, and it lets record decoding reuse
  one key buffer instead of allocating per key.
- **Numbers.** There are dedicated `i64`, `u64` and `f64` reads. Narrower
  integers are range-checked in the combinators (`OutOfRange`). `i128`,
  `u128` and arbitrary precision go through `read_number_text`. See
  [OPEN Q5].
- **`first: bool`** on `array_has_next` and `object_next_key` is kept
  from Gleam. The reader therefore needs no state stack for separators;
  the combinators already know whether they are on the first element.

### 4.3 Checkpoints

A checkpoint is an opaque position. For `SliceSource` it is a byte
offset. For streaming sources, an outstanding checkpoint **keeps every
buffered byte from that position onwards**. Checkpoints are released in
stack (LIFO) order.

Uses:

- `one_of`: try a branch, rewind on failure, and report the deepest
  error, as in Gleam.
- `tagged_union` and `versioned`, when the tag is not the first key
  (§8.3).

The record decoder does **not** use checkpoints (§6.3). That is a real
improvement on the Gleam version, which kept a cursor snapshot for every
key.

### 4.4 Streaming

The Gleam cursor has an `eof` flag and returns `Incomplete` when it runs
out of bytes before input has ended, but no driver exists. Rust needs a
choice here, because combinators are ordinary recursive Rust code and
cannot suspend partway through.

| Model | How | Cost |
|---|---|---|
| **Pull** | `ReadSource<R: io::Read>` keeps its own buffer and refills by blocking on `read`. `Incomplete` never reaches the combinators. | Blocks the thread. One pass, no re-work. Same model as serde_json's `IoRead`, but with buffering built in (design report §7.7.1). |
| **Push, restart per value** | Caller `feed`s chunks. On `Incomplete` the driver rewinds to the start of the current *top-level value* and retries once more bytes arrive. | Sans-IO and usable from any runtime. Worst case O(n²) for one huge value. Fine for NDJSON and framed messages. |
| **Push, resumable state machine** | Turn every combinator into a resumable state machine. | Not feasible with a combinator API. Rejected. |

**Recommendation:** ship both of the first two.

- `girders_json::from_reader(impl io::Read, codec)` is the general
  streaming entry point.
- `girders_json::StreamDecoder` is a push feeder that yields one value
  per top-level JSON value (NDJSON or whitespace-separated). It works
  for callers with no blocking reader, such as an async socket handing
  over chunks.
- Both use the same `JsonReader<S: Source>`. Only the `Source` differs.
  The sources live in `girders::source`, and the CBOR reader uses them too
  (§10.2).
- `Problem::Incomplete` stays in the error model as the signal between a
  source and the push driver. Combinators pass it through unchanged and
  must never treat it as a failed branch (`one_of`) or a missing field.

[OPEN Q6] Is restarting per top-level value acceptable for the push
model, or is resuming partway through a value a requirement?

### 4.5 Owned output

Decoded strings are always owned `String`. Borrowing (`&'de str` or
`Cow`) would spread a lifetime through every trait and combinator
(design report §3.6). Streaming input can't be borrowed from anyway, and
Gleam can't borrow either, so the comparison stays like for like.
Benchmarks will show what this costs against serde_json's borrowing
mode. [Accepted unless challenged: Q4]

### 4.6 `Serializer`

```rust
pub trait Serializer {
    type Error;
    fn write_bool(&mut self, v: bool) -> Result<(), Self::Error>;
    fn write_null(&mut self) -> ...;
    fn write_i64(&mut self, v: i64) -> ...;
    fn write_u64(&mut self, v: u64) -> ...;
    fn write_f64(&mut self, v: f64) -> ...;      // JSON: error on NaN/inf
    fn write_number_text(&mut self, v: &str) -> ...;
    fn write_str(&mut self, v: &str) -> ...;
    fn begin_array(&mut self, len: Option<usize>) -> ...;
    fn array_element(&mut self) -> ...;
    fn end_array(&mut self) -> ...;
    fn begin_object(&mut self, len: Option<usize>) -> ...;
    fn object_key(&mut self, key: &str) -> ...;
    fn end_object(&mut self) -> ...;
}
```

This uses the same per-element framing as Gleam. Differences:

- `len` is an `Option`, because Rust iterators don't always know their
  length.
- Encoding can fail. Gleam's encoders were infallible, but Rust has I/O
  errors and non-finite floats to deal with.
- The JSON serializer writes to any `io::Write`, so streaming output
  comes for free. `Vec<u8>` is the fast in-memory path.

---

## 5. Errors

### 5.1 Shape

```rust
pub struct DecodeError<L> (Box<Inner<L>>);   // keeps Result<T, E> small
struct Inner<L> {
    path: Vec<PathSegment>,        // built innermost-first, reversed for display
    location: Option<L>,
    problem: Problem,
    context: Vec<ContextFrame<L>>,
}
pub enum PathSegment { Key(String), Index(usize) }
pub enum Problem {
    TypeMismatch { expected: Cow<'static, str>, found: Kind },
    MissingField(String),
    UnknownField { name: String, expected: Vec<&'static str> },
    DuplicateField(String),               // new: strict default (§6.4)
    UnknownVariant { name: String, expected: Vec<String> },
    UnknownVersion { found: String, known: Vec<String> },  // new (§8)
    ValidationFailed(String),
    OutOfRange(String),
    Syntax(String),
    DepthLimitExceeded(usize),             // new (§5.4)
    Incomplete,
}
```

The location type is generic, so every format chooses what "where"
means. *Spike:* one `user()` codec decoded from a token reader (location
= token index) and from an env-var reader (location = variable name,
e.g. `APP_AGE`), with no change to the codec. This is the env-var case from notes1:

- `girders-json` uses `Span { start: u64, end: u64 }`.
- A test-only `EnvReader` uses the variable name. It is built as part of
  the test suite to prove the abstraction works on a non-text source.

Gleam's `synthetic()` span becomes `None`.

### 5.2 Rendering

- Format-neutral: `Display` gives `users[0].age: expected int, found
  string`, followed by the location's own `Debug` or `Display`.
- JSON adds `girders_json::diagnostic`, which resolves line and column at
  render time and draws a caret snippet with context frames. This is a
  port of the Gleam renderer and is written by hand (§9.5).
- `DecodeError` implements `std::error::Error`. That is what replaces
  glerror.

### 5.3 Accumulating errors

In accumulate mode:

- A record keeps going after a field fails: it skips that value, carries
  on with the remaining keys, and returns every error found.
- Lists keep going after a bad element.
- A failure returns `Errors<L>` (a non-empty `Vec<DecodeError<L>>`).

Fail-fast mode returns a single error in the same type. Syntax errors
and `Incomplete` always stop decoding, because nothing after them can be
trusted.

### 5.4 Decode options

These are runtime switches in a `DecodeContext` passed down through decoding
(design report §7.8):

```rust
pub struct DecodeOptions {
    pub track_locations: bool,   // default true
    pub track_path: bool,        // default true
    pub accumulate: bool,        // default false
    pub max_depth: Option<usize>,// default Some(128); stack safety (design report §9.5)
}
```

Presets: `default()`, `strict()` (everything on, accumulate),
`fast()` (everything off).

[OPEN Q7] Runtime flags cost one branch per primitive. The alternative
is to fix them at compile time with a type parameter (zero cost, but
more types everywhere). Recommendation: start with runtime flags, and
let the benchmarks decide whether to change.

---

## 6. Codecs and combinators

### 6.1 Representation

**Recommendation [OPEN Q3]: generic combinator types behind traits,
with the format chosen per method.**

```rust
pub trait Decoder {
    type Output;
    fn decode<R: Reader>(&self, r: &mut R, cx: &mut DecodeContext<R::Location>)
        -> Result<Self::Output, Errors<R::Location>>;
    fn schema(&self, cx: &mut SchemaCx) -> SchemaNode;
}
pub trait Encoder {
    type Input: ?Sized;
    fn encode<S: Serializer>(&self, v: &Self::Input, s: &mut S, cx: &mut EncodeContext) -> Result<(), S::Error>;
    fn schema(&self, cx: &mut SchemaCx) -> SchemaNode;
}
pub trait Codec: Decoder + Encoder<Input = <Self as Decoder>::Output> {}
```

- Because the reader and serializer types are parameters on the
  *methods*, a codec value works with **any** format. Gleam's
  `Codec(a, src, f)` needed the format in its type; here it isn't needed.
- Combinators are concrete generic structs (`Map<C, F>`, `List<C>`, …)
  with no boxing, so the benchmarks measure the design and not
  allocation overhead.
- The price is that generic methods are not object-safe, so codecs can't
  be stored as `dyn` directly. Boxing ties a codec to one format:
  `codec.boxed::<JsonReader<_>>()`. That's acceptable for the rare
  heterogeneous case. Versioned legacy decoders are handled differently
  (§8.2).
- Constructor functions return `impl Codec<…>`:
  `fn user() -> impl Codec<Output = User>`.

### 6.2 Combinator inventory

Ported from Gleam:

- Primitives: `string`, `bool`, `null`, `i8`…`i128`, `u8`…`u128`,
  `f32`, `f64`.
- Basic combinators: `optional`, `success`, `fail`, `map`, `then`,
  `try_map`, `guard`, `spanned`, `list`, `entries`, `dict`
  (`HashMap`/`BTreeMap`), `one_of`, `tagged_union`.
- Codec combinators: `bimap`, `try_bimap`, `refine` (§7.3), `recursive`
  (§7.4).

### 6.3 Record builder: applicative, not monadic

The Gleam builder is a monadic `use`-chain. It doesn't port to Rust (no
`use` sugar), and it **can't describe its own shape**: later fields only
exist inside closures that need values. That rules out schema
extraction, strict unknown-field checking and `fields_of`, which all
need the field list up front. girders uses a static field list instead:

```rust
fn user() -> impl Codec<Output = User> {
    record((
        field("id", string(), |u: &User| &u.id),
        field("age", u32(), |u: &User| &u.age),
        optional_field("email", string(), |u: &User| &u.email),
    ))
    .build(|(id, age, email)| User { id, age, email })
}
```

- **Tuples are implemented with `macro_rules!` inside the library** for
  arities 1 to 16 [OPEN Q8: 16 or 32?]. Users never write a macro.
  *Spike:* arity 16 works. Slots are created with a `new_slots()`
  method, because std implements `Default` for tuples only up to 12. A
  16-field record adds about 30–50 ms to a debug build.
  Wider records nest tuples:
  `record(((a, b, …), (p, q, …)))`.
- **Exhaustive in both directions.** The struct literal in `build`
  checks decoding, and encoding can't skip a field because `build` needs
  them all.
- **The one mistake the compiler misses** is the same as in Gleam: a
  getter returning the wrong field of the same type. Round-trip property
  tests catch it.
- **Decoding takes one pass and needs no snapshots.** The record keeps a
  tuple of `Option<T_i>` slots. Each key is looked up in the static
  field list and its value decoded straight into the matching slot. At
  `end_object`, a slot still empty is either `MissingField` or `None`
  for an optional field. This works unchanged over streaming input with
  no buffering. In Gleam, every key's cursor position was kept and read
  afterwards.
- **What we give up from Gleam:** branching on a field's value partway
  through a record. Use `then` or `tagged_union` for that.
- **Getters need a type annotation** (`|u: &User| …`). Without one,
  rustc gives E0282 with a clear hint. An alternative still to try:
  `record::<User>(|f| (f.field("id", string(), |u| &u.id), …))`.
  [OPEN Q13]
- **Reusing a record.** `record(…).build(…)` returns a `Record` that also
  implements a `RecordCodec: Codec` trait, which exposes the field set.
  Reusable records return `impl RecordCodec<Output = T>` so that
  `fields_of` can take them. In the spike, without this trait, the
  return type had to be spelled out in full.
- **Compile errors.** The spike's usual mistakes all give short errors
  that point at the user's code: a getter with the wrong type, a field
  missing from the tuple, a missing annotation. See FINDINGS.md.

### 6.4 Unknown and duplicate fields

Strict by default (design report §9.1):

- An unknown key is an `UnknownField { name, expected }` error.
- `.allow_unknown()` skips unknown keys.
- `.with_extras(name, getter, value_codec)` collects unknown keys into a
  `Vec<(String, V)>` field. The user supplies a codec for `V`, such as
  `girders_json::value()`, so the core never needs its own dynamic value
  type.

Duplicate keys:

- [OPEN Q9] The Gleam version uses the first value and ignores later
  ones. The recommendation is to make a duplicate an error
  (`DuplicateField`) by default, which fits "strict by default", with
  `.first_wins()` and `.last_wins()` as opt-outs.

### 6.5 `fields_of`

`fields_of(other_record)` adds another record's fields to the current
object (design report §9.3):

```rust
record((fields_of(pagination()), field("users", list(user()), |p: &Page| &p.users)))
    .build(|(pagination, users)| Page { pagination, users })
```

- Both field sets are static, so they merge. Unknown-field checking sees
  the combined set, and a key name that appears in both sets fails when
  the codec is built.
- Encoding writes the inner record's entries into the same object.

### 6.6 Variants

Tagged unions use the variant list for decoding and an exhaustive
`match` for encoding, like Gleam's `case`. The `match` arms return a
positional sum, `Of2…Of16` (generated by the library with
`macro_rules!`):

```rust
tagged_union(
    "type",
    (
        variant("circle", circle(), Shape::Circle),
        variant("square", square(), Shape::Square),
    ),
    |s| match s {
        Shape::Circle(c) => Pick2::Variant0(c),
        Shape::Square(q) => Pick2::Variant1(q),
    },
)
```

- *Spike:* rustc checks that the `match` is exhaustive (E0004) and that
  each `Vn` payload's type matches that variant's codec (E0308 if two
  positions are swapped). The closure needs no type annotation. No
  dynamic dispatch, no tag strings in the encoder.
- The one mistake the compiler can't see: two variants with the *same*
  payload type and their positions swapped. Round-trip property tests
  catch it.
- The tag key is *claimed* by the union, so strict unknown-field
  checking in the variant's record doesn't reject it. Encoding injects
  `tag: value` with an `InjectTag` serializer wrapper, so variant codecs
  don't need to know about tagging.
- The claim must be **scoped**: in the spike, a claim made for a
  non-record variant leaked to the next nested record. The real version
  clears it in every non-record decoder, or passes it explicitly.
- **Rule (spiked): combinators never change the `Reader` or `Serializer`
  type they pass to inner codecs.** The first union encoder wrapped the
  serializer (`InjectTag<S>`). Through a recursive union that
  instantiates `InjectTag<InjectTag<…>>` forever, and rustc hits its
  recursion limit. Per-call state travels in context parameters
  instead: `Cx` when decoding (claimed tag key) and `Ecx` when encoding
  (pending tag entry, written first by the next record). So
  `Encoder::encode` takes `(&self, v, s, cx: &mut Ecx)`.
- `adjacent_union(tag, content)` is included.
  [OPEN Q10] Should `shape_dispatch` (design report §9.4) be in v1?

---

## 7. Schema

**Removed (decision S1).** Schemas are out of scope for this codebase.
When a schema exists, it is needed at compile time, so it should drive
code generation (codecs generated from the schema). Deriving a schema
from codecs at runtime doesn't fit that. The rest of this section
records the design that was explored and dropped.

### 7.1 Source of truth [OPEN S1]

**Recommendation:** the codec is canonical, and the schema is a view
derived from it (design report §8.5 A). Every combinator implements
`schema()` alongside decode and encode. Because one definition produces
all three, drift is impossible by construction.

### 7.2 Model

`girders::schema::SchemaNode` is format-agnostic:

- Primitive kinds, with integer width and range.
- `Array`, `Map`, `Nullable`.
- `Object { fields, required, additional: Deny | Allow | Typed(node) }`.
- `Union` (tagged, adjacent, untagged).
- `Const`, `Described`, `Refined(node, Vec<Refinement>)`.
- `Ref(name)`.

There's also an `Opaque` node for encode-only escape hatches.

`girders-json` emits JSON Schema from it. [OPEN S3]
- **Draft 7** works best with yaml-language-server and similar tooling
  (the editor-validation case from notes1).
- **2020-12** is the current standard.
- Recommendation: Draft 7 first, with 2020-12 as an option on the
  emitter.

### 7.3 Refinement

`try_map` hides a type change from the schema. `refine(codec, hints,
validate)` attaches `Refinement::{Format, Pattern, Range, Length}` hints.
Without hints, the schema falls back to the inner node (design report
§8.3).

### 7.4 Recursion

**Decision (spiked): the `Recursive` trait (option A).**

```rust
pub trait Recursive {
    type Output;
    const NAME: &'static str;
    fn codec(&self) -> impl Codec<Output = Self::Output>;
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
```

- No macros. Format-agnostic. No `dyn`, no allocation.
- The schema emits `Ref(NAME)` and registers the body once.
- The spike covers a self-recursive record, a recursive tagged union
  with `Box`, mutual recursion, and generic recursion where the def
  carries an element codec. Generic recursion needs the def to be
  `Clone`, so all library codecs derive `Clone`.

Background: why a plain combinator doesn't work. A purely
combinator-based `recursive("Tree", tree)` **does not compile**. rustc overflows (E0275):
`Record<…List<Recursive<tree>>…>: Decoder` depends on itself, and Rust
only accepts that kind of cycle for auto traits. Something *nominal*
has to break it. Two options work:

- **A unit struct** whose impls delegate to a body function (about 15
  lines). It stays format-agnostic and costs nothing at runtime.
- **A boxed codec tied to one format** (`dyn` is nominal). Short to
  write, but the codec then works for that format only.

The schema side is solved separately by `named("Tree", codec)`, which
emits `Ref("Tree")` and registers the body once. The spike shows this
works.

The `Recursive` trait above breaks the cycle in a nominal way:
proving `Rec<D>: Decoder` only needs `D: Recursive`. The alternatives
considered are recorded in the spike's FINDINGS.md: a hand-written unit
struct, a library macro, a boxed codec tied to one format, and an
`unsafe` fixpoint.

### 7.5 Interaction with other features

- **Strict records** emit `additionalProperties: false`.
- **`with_extras`** emits a typed `additionalProperties`.
- **`fields_of`** merges the field sets.
- **Unions** emit `oneOf` with a tag `const`.
- **Versioned codecs** emit two schemas (§8.4).

### 7.6 Deferred

- Schema-first code generation (a typify-style tool). Rust already has
  typify for this.
- [OPEN S5] Testing the emitted schemas: snapshot tests only (insta), or
  also a JSON Schema validator as a dev-dependency? Recommendation:
  snapshots only for v1.

---

## 8. Versioning

### 8.1 Model

The model is the same as design report §9.2:

- Each version is its own Rust type.
- Migrations are plain functions, or `From` impls.
- Decoding reads the version tag, dispatches to that version's decoder,
  and migrates the result to the latest type.
- Encoding always writes the latest version, with its tag.
- There is no try-each-version fallback.

### 8.2 API

```rust
versioned("schema_version", 3, user_v3())          // latest: full codec
    .default_version(1)                            // tag absent -> v1
    .legacy(1, user_v1().map(UserV2::from).map(UserV3::from))
    .legacy(2, user_v2().map(UserV3::from))
```

Each legacy decoder has a different type. *Spike:* they are kept in a
type-level cons list, `(Legacy<D1>, (Legacy<D2>, ()))`. Each `.legacy()`
call returns a new `Versioned<C, (Legacy<D>, L)>` type. There's no
boxing, no `dyn`, and no fixed format. Dispatch is a chain of
comparisons. Encoding wraps the serializer in `InjectTag`, so the
latest codec is written unchanged and gets the tag added as its first
key.

### 8.3 Open decisions

- **[OPEN V1] Where the tag lives.**
  - Recommendation: inside the object, `{"schema_version": 2, …}`, as
    design report §9.2 has it.
  - Also offer an envelope option, `{"version": 2, "data": {…}}`.
  - As with unions, the tag key is claimed so strict checking doesn't
    reject it.
- **[OPEN V2] Streaming when the tag isn't the first key.**
  - Recommendation: take a checkpoint when the object starts, scan for
    the tag, rewind, then decode with the selected version.
  - When the tag *is* first there's a fast path: no rewind, and only a
    small amount of buffering.
  - The same mechanism serves `tagged_union`.
  - *Spike:* the scan, rewind and claim approach works with the tag in
    any position, both for versions and for union tags.
- **[OPEN V3] Tag type.** An integer, a string, or either? Recommendation:
  a `VersionTag` that accepts both. Number tags are compared as numbers,
  and string tags exactly (for example `"2024-06"`).
- **[OPEN V4] Missing or unknown versions.** A missing tag falls back to
  `default_version`, or is `MissingField` if no default is set. A tag
  that isn't registered is `UnknownVersion { found, known }`.

### 8.4 Schema [OPEN V5]

Emit two schemas:

- a **write schema**: the latest version only, which is what the
  encoder produces;
- a **read schema**: `oneOf` over every version, keyed by the tag
  `const`.

### 8.5 Out of scope [OPEN V6]

Encoding an old version on purpose. That can be done by composing
codecs by hand.

---

## 9. girders-json

### 9.1 Cursor and reader

- A port of the strict RFC 8259 cursor: no trailing commas, no comments,
  no NaN, UTF-8 validated.
- Float parsing uses std `str::parse::<f64>`, which is correctly
  rounded. Float formatting uses std's shortest-round-trip `Display`.
  Integer formatting also uses std. If the benchmarks show these are hot
  paths, we write our own rather than taking a dependency.
- `JsonReader<S: Source>`. The sources are `SliceSource<'a>` (whole
  input in memory, no copying, borrowing input only *within* the reader)
  and `ReadSource<R: io::Read>` (a buffer with checkpoint pinning). The
  push feeder is described in §4.4.

### 9.2 Entry points

```rust
girders_json::decode(&[u8] | &str, &codec) -> Result<T, Errors<Span>>
girders_json::decode_with(input, &codec, DecodeOptions)
girders_json::from_reader(impl io::Read, &codec)
girders_json::StreamDecoder::new(codec)   // push: feed(&[u8]) / finish(); yields values
girders_json::encode(&value, &codec) -> String
girders_json::encode_pretty(&value, &codec, indent)
girders_json::encode_to(impl io::Write, &value, &codec, Style)
girders_json::parse::<M>(input) -> Result<Value<M>, …>
girders_json::schema(&codec, Draft) -> Value
```

### 9.3 `Value`: the caller picks the object representation

```rust
pub struct Value<M: ObjectMap = Ordered> { pub node: Node<M>, pub span: Option<Span> }
pub enum Node<M> { Null, Bool(bool), Number(Number), String(String), Array(Vec<Value<M>>), Object(M) }
pub enum Number { I64(i64), U64(u64), F64(f64), Text(String) } // Text: ints that don't fit
pub trait ObjectMap { /* from_entries, get, iter, insert, len */ }
```

The `ObjectMap` implementations are all std-only:

| Type | Key order | Lookup | Notes |
|---|---|---|---|
| `Ordered` (default) | input order | linear | Our own `Vec<(Spanned<String>, Value)>`. Keeps key spans and preserves order on round trips. Matches Gleam's internal OrderedMap. |
| `Sorted` = `BTreeMap<String, Value>` | sorted | log n | Deterministic output. |
| `Hashed` = `HashMap<String, Value>` | none | O(1) | Fastest lookup. |

The type is chosen when parsing: `parse::<Hashed>(input)`, or
`value_codec::<Sorted>()`. Changing the map later is an explicit
conversion. Duplicate keys in `Value` parsing follow the duplicate-key
policy decided in Q9.

[OPEN Q11] Does `Ordered` need a hash index for large objects, built
past some size threshold? Recommendation: no for v1. We measure first.

### 9.4 JSON Pointer

A port of the RFC 6901 module: parse, escape, `from_path(&[PathSegment])`,
`Value::pointer(&Pointer)`, and the URI-fragment form.
`DecodeError::to_pointer()` is included.

### 9.5 Diagnostics

A hand-written port of `diagnostic.gleam`:

- line and column resolved at render time, counted in UTF-8 characters;
- a caret snippet under the offending range;
- context frames;
- a low-detail one-line mode.

No miette or ariadne (zero dependencies). The structured fields are
public so users can feed their own diagnostic systems.

---

## 10. girders-cbor

CBOR (RFC 8949) is the second format. It was chosen to test the
format-neutral parts of the design: it has byte strings, tags, half
floats, bignums, exact decimals and map keys of any kind, none of which
JSON exercises.

### 10.1 Format contract additions

Each addition has a format-neutral default, so formats that don't need it
change nothing.

| Addition | Default for formats without it | CBOR |
|---|---|---|
| `Kind::Bytes`, `read_bytes`, `write_bytes` | An array of numbers 0–255 | Byte strings |
| `Kind::Tag`, `supports_tags` (reader and serializer), `read_tag`, `write_tag` | No tags: `tagged(n, c)` neither checks nor writes them | Tags; `tagged(n, c)` requires tag `n` |
| `Kind::Simple`, `read_simple`, `write_simple` | Written as `null` | `undefined` and other simple values |
| `object_next_entry` (reader) | `Problem::Unsupported` | Keys of any kind, read as values |
| `object_key_value` (serializer, required) | JSON: strings and integers (quoted) only | Keys of any kind |
| `Problem::Unsupported` | | |

The JSON serializer's key mode costs up to 1% of encode instructions
(RESULTS.md).

### 10.2 Reader

- `CborReader<S: Source>`, over the same sources as JSON:
  - a slice (`decode`);
  - an `io::Read` (`from_reader`);
  - pushed chunks (`StreamDecoder`, which decodes a CBOR sequence per
    RFC 8742, like JSON's NDJSON decoder).

  Consumed input is discarded at item boundaries once 64 KiB is buffered
  and no checkpoint pins it. Spans stay absolute. A fast path for heads
  already in the buffer keeps slice decoding within about 1% of the
  slice-only reader.
- Strict well-formedness: every RFC 8949 Appendix F example is rejected.
- Text must be valid UTF-8. Indefinite-length strings must be made of
  definite chunks of the same type.
- Records read text keys; any other key type is a type mismatch.
- **Numbers keep full precision.**
  - Integers cover CBOR's whole native range.
  - Bignums (tags 2 and 3) and decimal fractions (tag 4) are exact
    through `read_number_text`, so `i128` and `u128` work across the
    full range.
  - Half, single and double floats are all read.
  - Integers are accepted where a float is expected; floats are never
    accepted as integers.
- `skip_value` is iterative, so nesting depth can't overflow the stack.

### 10.3 Serializer

- Preferred serialization: the shortest head for every length and
  integer, and the shortest float width that is exact (half, single or
  double). The output is byte-for-byte identical to `ciborium`'s.
- `EncodeOptions { floats: Floats::Declared }` writes each float at its
  Rust type's width instead. This is for decoders that require that
  width, such as `cbor4ii`'s serde support.
- Number text becomes an integer, a bignum, or a decimal fraction (tag 4).
- Containers of unknown length are written with indefinite length.

### 10.4 `Value`

- Integers are `i128`; bignums too large for that stay as tags over their
  bytes.
- Floats keep their value but not their width.
- Maps keep their order, with keys of any kind.
- It prints as RFC 8949 diagnostic notation.
- `girders_json::Value` also round-trips through CBOR, since it only uses
  the JSON-shaped parts of the contract.

### 10.5 Half floats and typed arrays

- **`f16()`** holds a half-precision value as an `f32`. Decoding and
  encoding round to the nearest half (ties to even); numbers beyond
  ±65504 are out of range. CBOR writes it in 2 bytes plus the type byte.
  The conversions live in `girders::half`, with no dependencies, since
  Rust has no stable `f16`.
- **`packed(element)`**: a `Vec` of numbers. With a format that has tags
  it is an RFC 8746 typed array, a tag over a byte string with no
  per-element encoding. It is written little-endian, and either byte
  order is read, as is a plain array. JSON uses a plain array.
  - Elements: every integer type up to 64 bits, plus `f16()`, `f32()`
    and `f64()`.
  - 1,000 halves: 2,005 bytes packed, 3,003 as a plain CBOR array.
  - 1,000 integers from 0 to 99: 1,005 bytes packed, 1,763 as an array.

### 10.6 Tests

- RFC 8949 Appendix A (81 examples) and Appendix F (94 malformed inputs)
  are vendored from the RFC under its Simplified BSD License, in
  `girders-cbor/tests/rfc8949/`.
- Typed codecs and cross-format behaviour:
  - absent versus `null`, and `undefined` versus `null`;
  - float narrowing;
  - input from streaming encoders;
  - unions and duplicate keys;
  - accumulated errors with byte offsets;
  - depth limits and truncation.
- Property tests: arbitrary `Value`s round-trip, and arbitrary bytes
  never panic.
- Streaming:
  - `io::Read` sources, including one that returns a byte at a time;
  - inputs several times the compaction threshold, with absolute error
    offsets after compaction, and accumulated errors rewound across it;
  - I/O errors;
  - CBOR sequences split at every chunk size, and under random chunking.
- The three json-benchmark models round-trip JSON → CBOR → model, and
  Python's `cbor2` reads the output as the same data.

### 10.7 Not yet

- Multi-dimensional typed arrays (RFC 8746 tags 40 and 1040).
- Typed map keys (N3).

---

## 11. Testing

### 11.1 Dev-dependencies (researched 2026-10-20; latest versions)

| Need | Choice | Alternatives considered | Why |
|---|---|---|---|
| Snapshot tests (replacing birdie) | **insta 1.49.0** | expect-test 1.5.1 (324★, no release since 2024-12), snapbox 1.2.2 (185★) | 3.0k★, released 2026-10-03, very widely used, `cargo insta review` workflow. |
| Property tests | **proptest 1.12.0** | quickcheck 1.1.0 (2.8k★, simpler shrinking), bolero 0.13.7 (260★, fuzz-oriented) | 2.3k★, released 2026-10-10, composable strategies with integrated shrinking. A `rust-proptest` skill is available. |

Snapshot testing is simple enough to write ourselves, but insta's review
workflow is worth having as a dev-only dependency.

### 11.2 Suites

- **JSONTestSuite conformance**, pinned to the same commit as the Gleam
  repo (`1ef36fa01286573e846ac449e8683f8833c5b26a`) and fetched by a
  mise task:
  - `y_` files must be accepted;
  - `n_` files must be rejected;
  - `i_` files are recorded as snapshots.
  - The suite runs over **both** the slice source and the streaming
    source. The streaming run uses chunk sizes of 1, 7 and 4096 bytes,
    so every token gets split across a chunk boundary somewhere.
- **Snapshot tests** port the 12 birdie snapshots: error rendering,
  pretty-printing, diagnostics.
- **Property tests** cover:
  - round trips: `decode(encode(x)) == x` for generated values and for
    each example codec;
  - `Value` round trips;
  - streaming at any chunk size gives the same result as decoding the
    whole slice;
  - accumulate mode reports a superset of the fail-fast error;
  - pointer escaping round trips.
- **Env-var reader** (test only) proves the location type is generic
  (§5.1).

### 11.3 Examples

The 7 Gleam examples are ported to `girders-json/examples/*.rs`:
record, tagged_union, collections, validation, error_reporting,
dynamic_value, pretty_printing. Each one asserts its own output, and CI
runs them with `cargo run --example`. They are the acceptance suite and
the main place to compare ergonomics with Gleam.

---

## 12. Benchmarks

### 12.1 Runtime: gungraun

- **gungraun 0.20.0** (formerly iai-callgrind; 334★; released
  2026-09-26; active).
  - It reports instruction counts through Valgrind's Callgrind, so
    results are deterministic, and heap allocation counts and bytes
    through DHAT.
  - It needs `gungraun-runner` at the **exact same version** (0.20.0),
    pinned through mise's cargo backend.
  - It also needs Valgrind, installed as a system package. Linux only.
- **Alternatives considered:**
  - criterion 0.8.2 (5.5k★, now maintained by the criterion-rs org)
    measures wall-clock time. Deferred; it could be added later as a
    cross-check for cache and branch effects that instruction counts
    miss.
  - divan 0.1.21 (1.5k★) has had no release since 2025-04, and users
    are asking whether it is still maintained.
  - tango-bench 0.9.0 (166★).
- **Baseline:** serde 1.0.229 (with `derive`) and serde_json 1.0.154,
  dev-only.

Scenarios. Each one reports instructions and allocations for girders
and serde_json side by side.

1. Decoding into typed structs, using the `canada.json`,
   `twitter.json` and `citm_catalog.json` corpus. The files are fetched
   by a mise task from serde-rs/json-benchmark (Apache-2.0) at a pinned
   commit. [TODO: confirm the licence of each data file before
   committing any.]
2. Encoding typed structs, compact and pretty.
3. Parsing to `Value`: `Ordered`, `Sorted` and `Hashed` compared with
   `serde_json::Value`.
4. girders' own options: spans on vs off, paths on vs off, fail-fast vs
   accumulate.
5. Streaming: `from_reader` vs slice, and the push feeder at several
   chunk sizes.
6. Error path: decoding invalid input and rendering the diagnostic.

The input files are plain JSON, so the Gleam version can be run over
the same corpus later for a rough comparison.

### 12.2 Compile time: hyperfine and cargo timings

- There are two fixture crates, `bench-fixtures/models-serde` and
  `bench-fixtures/models-girders`. Each defines the **same ~20 model
  types**, including nested records, unions, optionals, maps, and the
  corpus types from §12.1. One uses derive, the other girders codecs.
- **hyperfine 2.0.0** (29k★, released 2026-10-07) is a command-line tool
  installed through mise, not a crate dependency. It measures:
  1. a clean build of the fixture crate alone, with its dependencies
     already built, in debug and release;
  2. an incremental rebuild after touching the models file;
  3. a full clean build including dependencies, where serde's derive
     toolchain counts against it;
  4. release binary size, for a small `main` that decodes and encodes
     one model.
- `cargo build --timings` reports are kept for a per-crate breakdown.

### 12.3 Comparing with Gleam

The comparison has three parts:
- the same corpus;
- the examples side by side, for ergonomics;
- an instructions-per-byte figure.

Instruction counts aren't comparable across the two runtimes, so the
Gleam numbers will be wall-clock times, presented separately and
labelled as such.

---

## 13. Tooling

- **mise**, matching the Gleam repo:
  - Tools: Rust toolchain (pinned stable, currently 1.96), hyperfine
    2.0.0, `cargo:gungraun-runner@0.20.0`.
  - Tasks: `ci`, `test`, `fmt`, `fmt-check`, `clippy`,
    `fetch-jsontestsuite`, `fetch-bench-data`, `bench`, `bench:compile`,
    `examples`.
- **Valgrind** is a system package, and the bench task checks that it is
  present.
- **[OPEN Q12] Version control.**
  - The Gleam repo uses jj. girder-rs is a git repo, and git reports
    "dubious ownership" for it.
  - Recommendation: colocate jj with git (`jj git init --colocate`) to
    match the Gleam repo.
  - The ownership warning needs fixing in your git config
    (`safe.directory`) or by changing the directory's owner. I won't
    change it without your go-ahead.
- No MSRV policy. Edition 2024. Clippy runs with `-D warnings` in CI.

---

## 14. Open questions

### 14.1 Settled by the implementation (please confirm)

These were implemented as recommended, and the evidence supports them.

| ID | Decision | Evidence |
|---|---|---|
| S2 | Applicative tuple record builder | All three json-benchmark models and the 7 examples are written with it. Errors are short. |
| V1 | Tag inside the object, plus `.envelope("data")` | Both tested. The tag key is claimed, so strict records don't reject it. The claim is scoped to its depth (tested with a nested `"type"` field). |
| V2 | Scan under a checkpoint, then rewind | Works with the tag in any position, over every source. Correctness is covered by the push and reader tests. The cost of the scan hasn't been benchmarked separately. |
| V3 | Integer or label tags (`Version::Int`/`Label`) | Tested (`1`, `"2-beta"`, `3`). |
| V4 | `default_version`, else `MissingField`; `UnknownVersion { found, known }` | Tested. |
| Q4 | Owned output only | Allocations match serde within 0.2% on canada and citm, and are +18% on twitter. |
| Q5 | `i64`/`u64`/`f64` fast paths; `i128`, `u128` and big integers via number text | Tested at the extremes. |
| Q7 | Runtime option flags | Locations and paths cost 0.0–0.4% (`RESULTS.md`). Type-level flags aren't worth it. |
| Q8 | Arity cap of 16 | Works, but see N2. |
| Q13 | Typed field factory `fields_for::<T>()` | Works: getters need no annotation. Both styles are available. |
| no_std | `no_std` + `alloc`, with an optional `std` feature | Both crates build for Cortex-M4F and ESP32-C3. |

### 14.2 Still open

(S1, S3 and S5 are closed: schema support was removed, see §7.)

| ID | Question | Data / recommendation |
|---|---|---|
| V6 | Writing old versions | Still out of scope. |
| ~~Q3~~ | Codec representation | **Decided: compact only.** Generic combinator types, with child codecs called through the never-inlined `decode_child`. Records use one shared key loop with index dispatch. JSON entry points pass the reader as a `dyn Reader`. Compared with the removed inline mode: decode is 6–14% slower; release builds of a codec-heavy crate are ~40% faster; binaries are 14–18% smaller than serde's (ESP32-C3 `opt-level = "s"`, x86). |
| Q6 | Push decoder restarting per top-level value | One 632 KB document in ten 64 KB chunks costs 5.7× (restarts are O(k·n)). NDJSON-sized values are unaffected. Accept and document, or require `from_reader` for large single documents? |
| Q9 | Duplicate keys | Records default to `Duplicates::Error`, with `FirstWins` and `LastWins`. `Value` must accept duplicates (JSONTestSuite `y_object_duplicated_key`): `Ordered` keeps both (lossless, lookups take the last), `Sorted` and `Hashed` keep the last. Confirm? |
| Q10 | `shape_dispatch` | Not built. `one_of` covers untagged cases. |
| Q11 | Hash index for `Ordered` | Lookup cost not measured. Parsing into `Ordered` is cheapest (twitter: 19.6 M vs 25.2 M Sorted, 27.2 M Hashed). |
| Q12 | jj, and the git ownership warning | Unchanged. Nothing is committed. |
| N1 | Union `Payload` for struct-like variants | `Pick2::Variant0(x.into())` borrows; `Payload::Owned((w, h))` builds the payload. Every arm needs `.into()` (§6.6 changed). Accept, or allow only newtype variants? |
| ~~N2~~ | Wide records | **Decided: tuple records, cap 16, `.build(|(a, b, …)| …)`.** Alternatives were spiked and rejected, with the numbers in RESULTS.md ("Record styles"): typed keys (`v.take(id.key)`) read well but cost about 30% more in debug builds; dropping `.key` as well (`v.take(id)`) needs `Copy` codecs and stores each codec twice, costing +45% in release; a `record!` macro_rules front end costs 3–7% and was removed (no macros); raising the cap costs +1 s (32) or +3.7 s (64) on every clean library build; right-nested tuples break rustfmt. Tuples have the best compile and run time, stay readable, and models are expected to be written with an LLM anyway. Side results kept: field codecs no longer go through a wrapper; per-field constants are associated consts; per-field method bodies live in helpers shared per value type. |
| N3 | Missing primitives | An empty or fixed-length array (`array::<N>`), typed map keys (`map(key_codec, value_codec)`), a number inside a string (`int_str::<u64>()`). Recommendation: add all three. |
| N4 | `Value` memory | **Done:** a niche-optimized 16-byte `Option<Span>` (64-bit offsets, no size limit) and key spans in an optional side table (`DecodeOptions::key_spans`). Node 56 → 48 bytes, entry 104 → 72 bytes. Remaining gap to serde_json on number-heavy data: each node still carries a span slot. |
| N5 | `#![forbid(unsafe_code)]` | Costs on byte input: the ~10% `from_utf8` on canada number lexemes, plus UTF-8 checks on strings. `&str` input now avoids most of it with no `unsafe`. Keep the forbid, or allow audited `unsafe` in the reader? |
| N6 | Float formatting with zero dependencies | canada encode is 4.6× serde, almost all in core's `{:?}` float formatting. Options: accept; port a shortest round-trip float formatter (Ryū is Apache-2.0/BSL-1.0 but large); or allow one dependency (`ryu` or `zmij`; would need researching). |
| N7 | `optional_field` writes `null` for `None` (Gleam and serde parity) | `.skip_none()` omits the key instead. Keep `null` as the default? |
| N8 | Remaining decode gap of 1.3–1.9× | Next levers: (a) dispatch records by key length or first byte instead of a linear compare chain; (b) a whole-buffer fast path for scanning slices; (c) number lexing without per-byte `peek`. Worth pursuing in a throwaway, or stop here? |
| ~~N10~~ | Encoding strategy | **Decided: encoding stays inline (generic over the serializer).** Out-of-line child calls, a `dyn` serializer, and both together were measured (RESULTS.md, "Encoding strategy"). They save at most 5.6% of release compile instructions and 2% of binary text, but every variant makes debug builds 0.9–1.3% slower and encoding up to 12% slower. Debug build time is the priority. |
| N9 | Gleam comparison | Not run. The corpus is plain JSON in `benches/data`, ready for the Gleam side. |

## 15. Where the implementation departs from the text

- **§7 (schema):** removed entirely: no `schema()` on codecs, no
  refinement hints, no JSON Schema emitter, and no `Recursive::NAME`.
- **§6.1 (representation):** codecs stay generic types, but child codecs
  are called through `decode_child`, which is never inlined. JSON
  decoding passes the reader as a trait object. This is the "compact"
  strategy, and it is the only mode; see Q3 and `RESULTS.md`.

Each item points back to the section of this document it changes.

- **§6.6 (union projections):** picks take `Payload<'a, T>`, not `&T`. Struct-like
  variants have no single value to borrow (N1). There's also an
  `empty_object()` codec for fieldless variants (`{"type":"point"}`).
- **§6.4 (records):**
  - `Duplicates::{Error, FirstWins, LastWins}`.
  - `field_default(...)`, `extras(codec, getter)` for unknown keys, and
    `OptField::skip_none()`.
  - The field-name list for `UnknownField` is computed only when the
    error happens (it used to allocate per record).
- **§5.4 (decode context):** `DecodeContext` also pools key buffers, so
  objects don't allocate a key `String` each. The claimed tag key is
  scoped to its depth (`with_claim`).
- **§4.6 (encoding):** `Encoder::encode` takes an `EncodeContext`; unions and
  versioned codecs leave a pending tag in it for the next record. This
  follows the spike rule that combinators never wrap `R` or `S`.
- **§5.1 (errors):**
  - `Problem` gained `Io` (fatal) and `DuplicateField`. Fatal problems
    stop accumulation and `one_of`.
  - `Span` displays as `byte 71` / `bytes 71..80`; `DecodeError`'s
    one-line `Display` matches the Gleam `to_string` snapshot exactly.
- **§9.1 (sources):**
  - `SliceSource::from_text(&str)` lets the reader slice already-valid
    text instead of re-validating UTF-8; `decode(&str, …)` uses it.
  - Compaction happens in `skip_ws` once 64 KB has been consumed and
    no checkpoint is outstanding.
  - `skip_value` is iterative, so arbitrarily deep input never overflows
    the stack.
- **§9.3 (`Value`):** `Ordered` keeps duplicate keys (lookups take the
  last). `Sorted` and `Hashed` keep the last value. `Value::without_spans()`
  is there for structural comparison.
- **§7 (schema):**
  - `Tagging::{Untagged, Version { default }}`.
  - `SchemaMode::{Read, Write}`.
  - Tuple codecs emit an array of the first element's type (no
    `prefixItems` yet).
- **New combinators:**
  - `tuple2`/`tuple3`/`tuple4` and `string_enum([...])`, needed by the
    benchmark models.
  - `refine(codec, validate, hints)` and `TryBiMap::hint(...)`.
  - `context(description, codec)` for context frames.
  - `fields_for::<T>()` (Q13).
- **Diagnostics:** columns count `char`s, not grapheme clusters (no
  dependency for segmentation).
- **Spike:** `spikes/codec-shape/` was converted to `no_std` for the
  ESP32/Cortex-M build check. Its findings still apply (`FINDINGS.md`).
