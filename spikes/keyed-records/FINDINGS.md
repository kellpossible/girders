# Spike: keyed records (rejected for compile time)

Question (N2): can wide records avoid both the 16-field tuple cap and the
positional `build` pattern, without macros, allocation or `unsafe`?

## The API tried

```rust
let r = keyed_record::<User>();
let (r, id) = r.field("id", u32(), |u| &u.id);
let (r, name) = r.field("name", string(), |u| &u.name);
r.build(move |v| User { id: v.take(id), name: v.take(name) })
```

Keys carry their value type and their position as types. `take`
resolves the position at compile time, so there is no type erasure, no
allocation and no `unsafe`. It has no field limit and needs no getter
annotations, and completeness is still checked at compile time (the
struct literal needs every field, and values come only from keys).

## Alternatives rejected along the way

- **Right-nested tuples written by hand.** rustfmt indents every level
  (a staircase), and formatting a right-nested `build` pattern takes
  exponential time: 14 fields 45 ms, 18 fields 312 ms, 22 fields 4.5 s,
  40 fields didn't finish.
- **Type-erased storage through libraries.** Researched bumpalo,
  smallbox, stack_dst, anymap* and typed-arena. Each one allocates per
  value, uses `unsafe` internally, or doesn't fit; see the conversation
  record in DESIGN.md N2.

## Results

| | Tuple records | Keyed (`keyed_unchunked.rs`) | Keyed, chunked (`keyed_chunked.rs`) |
|---|---|---|---|
| twitter `User` (40 fields) codec | 175 lines | 114 lines | 114 lines |
| Decode / encode instructions (twitter) | 17.89 M / 9.56 M | – | +2% / −0.5% |
| ESP32-C3 decode size (`opt-level = "s"`) | 140.0 KB | 140.6 KB | 140.7 KB |
| 40-field record, clean crate build (debug) | 0.25 s | 3.2 s | 0.59 s |
| Model fixture crate (twitter keyed), debug | 1.4 s | 13.6 s | 2.9 s |
| Model fixture crate, release | 2.2 s | 15.0 s | 4.7 s |

`-Z time-passes` (via `RUSTC_BOOTSTRAP=1`) put the cost in
monomorphization and type checking. Every `take` walks a type-level list
and every `.field()` creates a new builder type. Unchunked, the cost grows
roughly with the cube of the field count. Chunks of 16 cut it to about 2×
tuple records, but no further.

## Decision

Rejected: compile time is a primary goal, and keyed records cost about
2× crate compile time in codec-heavy code. Tuple records stay, with a cap
of 16. Wide models are left to schema-driven code generation, which can
emit tuple records.

Files: `keyed_unchunked.rs`, `keyed_chunked.rs` (the generated
per-chunk-size impls), `record_with_build_trait.rs` (the `Build` trait
that let both styles share decoding), `keyed_tests.rs`,
`twitter_keyed.rs`, and `to_keyed.py` (converts tuple records to keyed
ones).
