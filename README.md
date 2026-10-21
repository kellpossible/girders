# girders

Bidirectional codecs for Rust, ported from the Gleam library
[girder](../girder). A codec is a plain value that both decodes and
encodes a type:

```rust
use girders::prelude::*;

fn user() -> impl Codec<Output = User> {
    record((
        field("id", u32(), |u: &User| &u.id),
        field("name", string(), |u: &User| &u.name),
        optional_field("email", string(), |u: &User| &u.email),
    ))
    .build(|(id, name, email)| User { id, name, email })
}

let user: User = girders_json::decode(json, &user())?;
let json = girders_json::encode(&user, &user())?;
```

This is an experiment, and a comparison with both the Gleam original
and serde.

## Why

- **Compile time.** serde's derives go through `syn` and proc-macro
  expansion, and every format gets its own fully inlined copy of every
  type's code. girders has no macros, and nested values decode through a
  shared out-of-line call. Clean builds skip the proc-macro toolchain
  entirely, and the model crate compiles in fewer instructions than
  serde's in release.
- **Small binaries.** Out-of-line decoding keeps code size flat as
  models grow, which matters on targets like the ESP32. Both crates are
  `no_std` + `alloc`.
- **Almost no dependencies.** The one exception is `siphasher`, for
  the keyed hash that protects `Value`'s index of large objects against
  crafted collisions: security-sensitive code is better taken from a
  vetted crate.
- **Codecs are values, not trait impls.** The same type can have
  several codecs: one per API version, a lenient one and a strict one,
  one for a wire format and one for storage. Codecs are built and
  combined at runtime like any other value: renaming, defaults,
  validation (`refine`), conversion (`convert`), tagged unions, versioned
  documents with migrations, and splicing one record's fields into
  another (`flatten`).
- **One definition for both directions.** A record lists each field
  once, with the getter used to encode it, so decode and encode can't
  drift apart.
- **Errors you can show users.** Decode errors carry byte spans, JSON
  pointers and field paths, can accumulate rather than stop at the
  first, and render as diagnostics against the source.
- **Parse, don't validate.** Validators return your own error types,
  and you get them back by type (`error.custom::<EmailError>()`) with
  the path where each happened, rather than as strings. Syntax errors
  are an enum per format (`girders_json::Malformed`).

## Compared with serde

| | serde + serde_json | girders |
|---|---|---|
| Definition | `#[derive]` and attributes on the type | Codec values, separate from the type |
| Codecs per type | One (`Serialize`/`Deserialize` impls) | As many as you like |
| Macros | Proc-macro derives | None |
| Dependencies | `serde`, `serde_derive`, `syn`, `quote`, `proc-macro2`, … | `siphasher` (girders-json only) |
| Unknown fields | Accepted unless `deny_unknown_fields` | Rejected unless `.allow_unknown()` |
| Errors | Line and column of the first error; custom errors as strings | Spans, paths, pointers; optionally every error; custom errors keep their type |
| Speed | Faster: girders decodes in 1.3–1.9× serde_json's instructions | Slower, in exchange for size and build time |

Measurements, including where the speed gap comes from, are in
[RESULTS.md](RESULTS.md); the design and its open questions are in
[DESIGN.md](DESIGN.md).

## Using it

Not on crates.io yet, and still changing; depend on the git repository.
Rust 1.88 or later.

```toml
[dependencies]
girders = { git = "https://github.com/kellpossible/girders", branch = "main" }
girders-json = { git = "https://github.com/kellpossible/girders", branch = "main" }
# girders-cbor = { git = "https://github.com/kellpossible/girders", branch = "main" }
```

`Cargo.lock` pins the exact commit; `cargo update -p girders` moves to the
latest.

Before 1.0, minor versions may change the interface. Most public enums
and option structs are `#[non_exhaustive]`, so additions to them don't
break code. The `Reader` and `Serializer` traits, for writing your own
format, are the least settled part.

## Layout

- `girders/`: format-agnostic codecs, records, unions, versioning.
- `girders-json/`: JSON reader and serializer, streaming, the dynamic
  `Value` type.
- `girders-cbor/`: CBOR (RFC 8949) reader and serializer with full
  numeric precision, and a CBOR `Value`. The same codecs read and write
  both formats.
- `benches/`, `bench-fixtures/`: runtime, compile-time and binary-size
  comparisons with serde.
- `spikes/`: experiments that shaped the API.
