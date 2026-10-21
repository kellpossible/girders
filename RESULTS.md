# girders: experiment results

Measured on 2026-10-20: 16-core x86_64 Linux, rustc 1.96.0, Valgrind
3.24.0, gungraun 0.20.0, hyperfine 2.0.0.

The baseline is serde 1.0.229 and serde_json 1.0.154. Both sides decode
equally strictly: serde uses `deny_unknown_fields`, girders is strict by
default. The models are the serde-rs/json-benchmark ones; the girders
versions are in `benches/src/`.

To reproduce:
- runtime: `mise run bench` (optionally with a filter such as `'*decode_typed*'`);
- compile time: `mise run bench:compile`.

## Correctness

- **JSONTestSuite** (pinned `1ef36fa0`, 318 files): 0 failures on every
  input path: slice, `io::Read` at 1-, 7- and 4096-byte chunks, and the
  push decoder at the same chunk sizes.
  - All 95 `y_` files are accepted and all 188 `n_` files rejected.
  - Of the 35 `i_` files, 5 are accepted and 30 rejected, and every path
    agrees.
  - Every path produces the same `Value` tree.
- **Same output as serde:** girders and serde decode all three corpora
  to equal values, and girders' encodings are byte-for-byte the same
  length as serde's.
- **Gleam parity:** the 7 Gleam examples are ported and pass. The 12
  Gleam diagnostic and pretty-print snapshots match the original output
  exactly.
- **Property tests:** pass at 3,000 cases each. They cover `Value` and
  record round trips, streaming at any chunk size matching the slice
  result, accumulate mode containing the fail-fast error, and pointer
  escaping.
- **`no_std`:** both crates build with `--no-default-features` for
  `thumbv7em-none-eabihf` and `riscv32imc-unknown-none-elf` (ESP32-C3).

## Runtime (instructions, Callgrind)

Ratios are girders ÷ serde_json; lower is better.

| Benchmark | serde_json | girders | ratio |
|---|---:|---:|---:|
| decode canada (number-heavy) | 55.8 M | 115.3 M | 2.06 |
| decode citm | 26.5 M | 42.2 M | 1.59 |
| decode twitter (string/key-heavy) | 8.98 M | 18.1 M | 2.01 |
| encode canada | 22.4 M | 103.5 M | **4.63** |
| encode citm | 7.19 M | 11.1 M | 1.55 |
| encode twitter | 6.43 M | 10.4 M | 1.62 |
| `Value` canada | 84.1 M | 150.8 M | 1.79 |
| `Value` citm | 42.9 M | 51.0 M | 1.19 |
| `Value` twitter | 18.6 M | 21.2 M | 1.14 |
| twitter from `io::Read` | 25.7 M (`from_reader` + BufReader) | 21.2 M (4 KB chunks) | **0.82** |

Refreshed on 2026-10-21. The first version of this table (decode canada
102.2 M, citm 34.2 M, twitter 16.9 M; `Value` canada 129.6 M) predates
later code changes and was never re-measured. By the first commit in
version control, decode was already at today's figures, so the increase
came from changes made before then (the record-style experiments), not
from CBOR or the later changes. Differences of a few percent between
builds are noise: rebuilding the bench binary moves serde_json's own
numbers by 1–4%.

Where the gap comes from:

- **canada encode:** float formatting through core's `{:?}`. serde_json
  uses a dedicated float formatter; girders takes no dependencies.
- **canada decode:** std float parsing, UTF-8 checks on number lexemes
  for byte input, and per-byte `peek` overhead.
- **twitter decode:** string copying, plus the linear field-name chain
  in records (`User` has 40 fields).

Optimizations applied during the experiment, measured on twitter decode:

| Change | Instructions | Allocations (blocks) |
|---|---:|---:|
| First version | 21.9 M | 12,441 |
| Compute the record's field-name list only on error, and pool key buffers in `DecodeContext` | 18.8 M | 4,886 |
| `&str` input skips UTF-8 re-validation, sliced from the original `&str` with no `unsafe` (`SliceSource::from_text`) | 16.9 M | 4,886 |

### Allocations (DHAT, whole process; setup is identical on both sides)

| Benchmark | serde_json | girders |
|---|---:|---:|
| decode canada | 2,129 blocks | 2,138 |
| decode citm | 4,530 | 4,538 |
| decode twitter | 4,134 | 4,886 (+18%) |
| encode twitter | 4,147 | 4,151 |
| `Value` canada | 14.6 MB | 23.8 MB (+63%) |
| `Value` twitter | 2.7 MB | 4.9 MB Ordered / 3.4 MB Sorted |

Note: gungraun's DHAT summary lines only count allocations whose stack
contains the benchmark function. serde's inlined code drops that frame,
so its summary shows almost nothing. The figures here come from the
whole-process DHAT logs instead.

`Value` is larger because every node carries `Option<Span>` (24 bytes)
even in fast mode. `size_of::<Value<Ordered>>()` is 56 bytes, against
32 for `serde_json::Value`. An `Ordered` entry is 104 bytes, 24 of them
an always-`None` key span.

### Cost knobs (twitter decode)

| Options | Instructions | vs default |
|---|---:|---:|
| default (locations + paths) | 16.91 M | n/a |
| no locations | 16.84 M | −0.4% |
| no paths | 16.91 M | 0.0% |
| fast (neither) | 16.84 M | −0.4% |
| accumulate | 17.16 M | +1.5% |

Locations and paths cost almost nothing on the success path. girders
records nothing for values that decode successfully: errors are built
only when something fails. The Gleam design returned a `Spanned` value
from every read; the Rust design doesn't need to.

### Streaming (twitter, 632 KB, one document)

| Path | Instructions |
|---|---:|
| slice | 16.9 M |
| `io::Read`, 4 KB buffer | 20.6 M |
| `io::Read`, 64 KB buffer | 21.7 M |
| push decoder, whole document in one chunk | 20.4 M |
| push decoder, 64 KB chunks (10 chunks, so 9 restarts) | 116.0 M (**5.7×**) |

The push decoder restarts the document from the beginning whenever it
runs out of input. For values much smaller than a chunk (NDJSON) that
cost is negligible.

## Compile time (wall clock, mean of 3–8 runs) and binary size

The fixture crates `bench-fixtures/models-{serde,girders}` hold the same
three models. They are generated from `benches/src` with the derive and
codec halves split apart, and both produce identical output.

| Build | serde | girders |
|---|---:|---:|
| model crate only, debug | 1.52 s | 1.74 s (+14%) |
| model crate only, release | 3.71 s | 4.90 s (+32%) |
| incremental after touching `twitter.rs`, debug | 748 ms | 590 ms (−21%)* |
| full clean build including deps, debug | 5.04 s | 3.33 s (−34%) |
| full clean build including deps, release | 7.18 s | 6.45 s (−10%) |
| release binary, stripped | 779 KB | 911 KB (+17%) |

\* Flawed: `touch` doesn't change file contents, and rustc's incremental
cache detects that, so this measures little more than cargo overhead. A
real incremental benchmark has to edit the file.

girders avoids the syn/proc-macro toolchain and derive expansion, so
clean and incremental builds are faster. Release builds of the model
crate are slower and the binary is bigger because every combinator
type is monomorphized.

### Record styles (model crate only, compiler instructions, mean of 2–8 runs)

Wall-clock time on this machine drifts by about 10% between sessions
and up to ±15% within a release run, so styles are compared by the
instructions rustc executes (hyperfine's perf counters), which vary
by less than 0.1%. Same three models in every fixture; all round-trip
identically to serde.

**Decision: tuple records.** A `record!` macro_rules front end (9.67 G
debug, 25.06 G release) was also measured and removed: the point is to
avoid macros. Final numbers after the changes below:

| Style (fixture) | Debug | Release |
|---|---:|---:|
| serde derive (`models-serde`) | 7.79 G | 26.66 G |
| tuple (`models-girders`) | 9.03 G | 24.24 G |

Rejected styles, measured before the last two shims (both are now removed):

| Style | Debug | Release | Twitter codec value | Twitter decode |
|---|---:|---:|---:|---:|
| tuple, for comparison | 9.91 G | 25.76 G | 192 B | 17.90 M instr |
| `let id = field(…)`; `id` in the tuple, `v.take(id.key)` | 12.73 G | 27.85 G | 192 B | 17.89 M instr |
| same, but `v.take(id)` | 12.74 G | 39.40 G | 664 B | 18.50 M instr |

- **Keys cost type checking.** Each `take` has to resolve a type-level
  position. At first it navigated the tuple of field codecs, whose
  getter closures stay partly inferred until their function finishes
  type checking, so no proof could use rustc's global cache: `typeck`
  took 620 ms against 116 ms, plus 340 ms of borrowck predicates.
  Navigating only the slot tuple brought keys from 15.97 G to 12.73 G
  (debug), still about 30% over tuples.
- **`v.take(id)` doubles the stored codecs.** The `move` closure
  captures a copy of every field codec, nested through inner records.
  Release LLVM time roughly doubles, and every field codec must be `Copy`.
- **The `Field { codec, key }` wrapper cost 21% (debug).** Forwarding
  every `Fields` method through it doubled the functions instantiated
  per field.

Monomorphization shims (kept):

- **Per-field constants are associated consts.** `NAME_COUNT` and
  `CLAIMS_UNKNOWN_KEYS` replace `name_count()` and the default
  `decode_extra()`. rustc doesn't instantiate calls inside an `if` on a
  constant that is false, so those per-field copies disappear.
- **Per-field method bodies moved into helpers generic only over the
  value type** (`encode_field`, `Slot::check_required`, …). Every field
  has its own type (its getter closure differs), so a method on it is
  compiled once per field; the helpers are compiled once per value type.
- Together with removing the key machinery, these took the tuple style
  from 9.91 G to 9.03 G (debug) and from 25.76 G to 24.24 G (release).
  LLVM IR fell by about 13%, but compile time much less: debug builds
  are dominated by LLVM's fixed per-function cost and by trait solving
  during code generation.

Tried and dropped:

- **Shared `decode_into` behind a `dyn` callback.** The closure and its
  vtable shims cost as much IR as they saved.
- **`lazy(user)`** (nested codecs referenced through their zero-sized
  function item, to keep outer types small): +0.8% debug, −1% release,
  which is noise.

### Encoding strategy (N10)

Decoding calls child codecs out of line and through a `dyn` reader.
The same two techniques were tried for encoding, separately and
together. Encode columns are runtime instructions per document;
binary text is the `models-girders` release binary; compile columns
are rustc instructions for the model crate.

| Encoding | canada | citm | twitter | Binary text | Debug | Release |
|---|---:|---:|---:|---:|---:|---:|
| Inline, generic serializer (**kept**) | 102.4 M | 11.02 M | 10.37 M | 725.5 KB | 9.03 G | 24.24 G |
| Out-of-line child calls only | +2.2% | +6.2% | +1.9% | −0.4% | +0.9% | −2.5% |
| `dyn` serializer only | +4.1% | +4.8% | +0.3% | −1.5% | +1.3% | −4.1% |
| Both | +5.9% | +11.8% | +1.3% | −2.0% | +1.3% | −5.6% |

Encoding gains much less than decoding did: encoding a record is
already a short sequence of calls, with none of decoding's slots,
missing-field checks or error recovery to share. Every variant makes
debug builds slightly slower, and debug build time takes priority, so
encoding stays inline. A `dyn` serializer would help more in code that
encodes to several writer types (each codec is compiled once per
serializer type); this fixture encodes only to `Vec`.

## CBOR (girders-cbor)

The same typed models, encoded as CBOR instead of JSON. Runtime figures are
instructions per document (Callgrind, `cbor` group). The serde baselines are
dev-dependencies of the benches crate only:

- `ciborium` 0.2.2: the common serde choice, with 70M recent downloads,
  but no release since January 2024.
- `cbor4ii` 1.2.4: actively maintained and performance-focused.

`ciborium` decodes girders' bytes, which are identical to its own;
`cbor4ii` decodes its own encoding (see "Interoperability" below).

| Document | girders | ciborium | cbor4ii | girders JSON |
|---|---:|---:|---:|---:|
| decode canada | 43.5 M | 34.9 M | 9.07 M | 102.2 M |
| decode citm | 24.0 M | 15.9 M | 8.84 M | 34.2 M |
| decode twitter | 13.4 M | 10.6 M | 7.06 M | 16.9 M |
| encode canada | 12.7 M | 16.9 M | 2.08 M | 102.4 M |
| encode citm | 4.12 M | 5.13 M | 2.52 M | 11.0 M |
| encode twitter | 2.47 M | 3.14 M | 2.75 M | 10.4 M |

- **Encoding:** girders beats `ciborium` on all three documents, and
  `cbor4ii` on twitter. On canada, `cbor4ii` writes each `f32` as 4 raw
  bytes, while preferred serialization first checks whether a half float
  would be exact.
- **Decoding:** girders takes 1.3–1.5× `ciborium`'s instructions and
  1.9–4.8× `cbor4ii`'s. The gap is per value: canada costs girders about
  390 instructions per float against `cbor4ii`'s 80. This is the same
  per-value overhead as N8's JSON decode gap, through the reader trait
  object and out-of-line child decodes, rather than anything specific
  to CBOR.
- **Against girders' own JSON**, decoding CBOR takes 21–57% fewer
  instructions and encoding 3–9× fewer. No float formatting or parsing is
  involved.

**Wire size** (bytes), written by the models:

| Document | JSON | CBOR |
|---|---:|---:|
| canada | 1,203,993 | 611,232 |
| citm | 500,299 | 342,373 |
| twitter | 477,706 | 410,131 |

canada's model uses `f32` coordinates, following json-benchmark, so its
floats are written as single precision (5 bytes each).

**Interoperability.**

- **`ciborium`:** both libraries write preferred serialization, and
  their output is byte-for-byte identical on all three documents. Each
  reads the other's.
- **`cbor4ii`:** two conventions differ.
  - serde's `()` is an empty array in `cbor4ii`, but `null` in girders'
    `null()` and in `ciborium`. Models with unit fields (twitter, citm)
    therefore can't be exchanged with `cbor4ii`.
  - `cbor4ii` reads a float only at its field's declared width, so it
    rejects preferred serialization's shorter floats (canada). girders
    offers `Floats::Declared` for this; with it, canada interoperates both
    ways.
- **Python's `cbor2`** decodes all three outputs.
  - citm equals its source JSON exactly.
  - canada differs only by the model's `f32` narrowing.
  - twitter differs only where the model writes absent optional fields
    as `null`.

**Cost to JSON.** The JSON serializer's new key mode (for `Value` maps
with non-string keys) costs 0.4% (canada, twitter) to 1.0% (citm) of
encode instructions.

## Spikes: size hints and a hash index for large objects

### Size hints (preallocation)

`Reader::size_hint` gives the length of a CBOR definite-length array or
map. JSON never gives one. `list()` and both `Value` types pre-size their
`Vec`s from it, capped at 1 MiB of elements. CBOR decode, in instructions
(Callgrind), with hints off and then on:

| CBOR decode | Without hints | With hints | Change | Bytes allocated |
|---|---:|---:|---:|---:|
| typed canada | 43.36 M | 42.66 M | −1.6% | −13% |
| typed citm | 24.57 M | 23.64 M | −3.8% | −3% |
| typed twitter | 13.56 M | 13.51 M | −0.4% | −6% |
| `Value` canada | 76.27 M | 70.49 M | −7.6% | −19% |
| `Value` citm | 35.25 M | 33.68 M | −4.5% | −17% |
| `Value` twitter | 17.47 M | 16.36 M | −6.4% | −20% |

Bytes are whole-process figures (setup included), so the percentages
understate the decoder's own saving. typed canada gains little because its
coordinate pairs are fixed-size tuples, not `Vec`s.

### A hash index for large `Value` objects

Objects with more than 64 entries used to get a sorted index of
positions. Now they get a hash table instead, at most half full, with a
small built-in FxHash-style hash.

- **Collisions:** the hash is fixed, because there's no randomness
  without `std`, so input can be crafted to collide. If any key lands more
  than 64 slots from its home bucket while the table is being built, the
  object falls back to the sorted index.
- **Lookups:** that same bound caps every lookup at 65 probes.

Wall clock is from `benches/examples/value_lookups.rs`: one object of n keys,
looked up in a scrambled order, release build. The attack uses keys built
to share one 64-bit hash (`colliding_keys`).

| n | Lookup, sorted | Lookup, hash | Parse, sorted | Parse, hash | Attack lookup, sorted | Attack lookup, hash | Attack parse, sorted | Attack parse, hash |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 57 ns | 18 ns | 9.5 µs | 9.6 µs | 58 ns | 58 ns | 17.1 µs | 28.8 µs |
| 1,000 | 98 ns | 22 ns | 94 µs | 98 µs | 98 ns | 99 ns | 237 µs | 260 µs |
| 10,000 | 228 ns | 48 ns | 0.95 ms | 1.04 ms | 243 ns | 242 ns | 2.73 ms | 2.84 ms |
| 100,000 | 1,267 ns | 264 ns | 10.9 ms | 12.0 ms | 1,243 ns | 1,401 ns | 38.0 ms | 39.9 ms |

Lookups are per lookup; parse is the whole object. For comparison,
`indexmap` measured 19–34 ns per lookup in the earlier table, but in
insertion order.

- **Lookups** are 3–5× faster at every size.
- **Parsing** a large object costs up to 10% more, for hashing every key
  and a table of 16–32 bytes per entry (the sorted index used 4).
- **Under attack**, lookups are the same as the sorted index's, because
  that is what the object falls back to. Parsing pays for the abandoned
  table: about 12 µs per object, which is +68% for a 100-key object and
  +5% at 100,000 keys.

**Then: keyed SipHash and Robin Hood probing.** The fixed FxHash-style hash
had seed-independent collisions: flipping a word's top bit changes the
state predictably, so even a secret seed wouldn't help. It was replaced
by keyed SipHash-1-3 from the `siphasher` crate, using its one-shot
`hash`; the incremental `Hasher::write` doubled hashing cost for short
keys. The key is caller-supplied, else random per process with `std`.

Probing became Robin Hood, which keeps displacement low enough to lower
the fallback bound to 32. The worst displacement measured for honest keys
at half load was 3, 5, 7, 11 and 16 slots at 2^6, 2^10, 2^12, 2^18 and
2^22 entries. Same benchmark as above, after a warm-up pass:

| n | Lookup | Parse | Attacker knows the key: lookup | Attacker knows the key: parse |
|---:|---:|---:|---:|---:|
| 100 | 14 ns | 8.8 µs | 58 ns | 11.5 µs |
| 1,000 | 18 ns | 91 µs | 96 ns | 111 µs |
| 10,000 | 36 ns | 0.95 ms | 237 ns | 1.13 ms |
| 100,000 | 220 ns | 12.4 ms | 1,485 ns | 13.2 ms |

"Attacker knows the key" is the worst case: no `std` and no key given.
The object then falls back to the sorted index. citm's 184 lookups take
49,978 instructions (104,835 sorted, 34,718 with the unkeyed hash).
Corpus parsing is within 1%.

The earlier FxHash-style measurements follow.

On the benchmark documents (Callgrind instructions):

- **citm:** its 184 lookups in a 184-key object go from 104,835 to 34,718
  instructions (−67%).
- **twitter:** its 301 lookups in small objects are unchanged, since no
  index is built below 64 keys.
- **Parsing** canada, twitter and citm changes by 0–1%. That is within the
  noise between builds: serde_json's own numbers, whose code didn't
  change, moved by 1–4% when the bench binary was rebuilt.
- **Memory:** citm grows by 3 KB.

## Ergonomics (from writing the twitter and citm models)

- **Wide records:** the 16-element tuple cap means nesting
  (`User`: 16 + 16 + 8 fields). Each field name is written three times:
  in the getter, the `build` pattern and the struct literal. The `User`
  builder closure alone is about 95 lines, and the split points must
  match by hand.
- **Missing primitives:**
  - an empty or fixed-length array codec (both agents wrote
    `try_bimap(list(null()), …)`);
  - typed map keys (keys are always `String`);
  - a "number inside a string" codec (for `id_str`).
- **Recursive `Status`:** the codec lives in an
  `impl Recursive for StatusDef`, which breaks the `fn x() -> impl Codec`
  style. It is rebuilt on each call; that's free because it is
  zero-sized.
- **Struct-like enum variants** (`Rectangle { w, h }`) have no single
  value to borrow, so union projections now take a `Payload`: either
  `x.into()` (borrowed) or `Payload::Owned((w, h))`.
- **Getter annotations:** `fields_for::<T>()` (Q13) removes them. The
  models still use the annotated style.

## Code size and the compact strategy

**Decision:** compact is now the only decoding strategy; the inline mode and the `compact` feature were removed. The inline numbers below are kept for reference.

Decoding can use one of two strategies, through the same codec API:

- **inline** (the default): child codecs are inlined, the fastest option.
- **compact**: enabled with the `compact` cargo feature, or explicitly
  with `girders_json::decode_compact`. It changes three things:
  - child codecs are called through never-inlined trampolines;
  - the reader is used as a trait object;
  - records use one shared key loop plus an index dispatch.

Results and errors are identical in both modes: the whole test suite,
including JSONTestSuite, the snapshots and the property tests, passes
with `--features compact`.

Changes that also shrank the inline mode:

- accumulate-mode bookkeeping and field error handling moved into
  non-generic cold functions;
- records check for missing fields before moving any values out
  (`check`/`take`), which removes the early-exit drop glue that grew
  with the square of the field count.

### Decode-only, x86_64, std (model code = text minus a 337 KB empty-program baseline)

| | unwind | `panic = "abort"` |
|---|---:|---:|
| serde + serde_json | 383 KB | 278 KB |
| girders inline (first version) | 436 KB | 318 KB |
| girders inline (now) | 359 KB | 277 KB |
| girders compact | 309 KB | 228 KB |

### Decode-only, ESP32-C3 firmware (`riscv32imc-unknown-none-elf`, `no_std`, LTO, `panic = "abort"`, `codegen-units = 1`)

Sizes are `.text` + `.rodata`; three models, 24 structs, 159 fields.
Harness: `bench-fixtures/esp`.

| opt-level | serde | girders inline | girders compact |
|---|---:|---:|---:|
| `"z"` | 123 KB | 131 KB | 125 KB |
| `"s"` | 162 KB | 168 KB | 139 KB |
| `3` | 159 KB | 199 KB | 149 KB |

At `opt-level = "z"`, serde is already compact (about 250 bytes of
code per field), and girders compact matches it rather than beating
it. About 34 KB of girders' code is the per-record assembly inlined
into each codec type's trampoline: slots, `check`/`take`, and moving
the values into the struct.

### Speed of compact (decode instructions)

| | inline | compact | slowdown |
|---|---:|---:|---:|
| twitter | 16.7 M | 17.6 M | +5.7% |
| canada | 101.9 M | 109.9 M | +7.9% |
| citm | 34.2 M | 39.0 M | +13.9% |

## `Value` object maps

*Historical: these measurements chose the ordered representation, and
`Value` has since lost its type parameter; only that representation
remains (DESIGN.md §9.3). The benchmarks for the alternatives were
removed with it.*

`Ordered` is the default. It is a `Vec` in document order and keeps
duplicate keys. Objects with more than 64 entries also get a sorted
index of positions, so lookups there are binary searches: O(log n),
with no hashing to attack.

The other maps compared:

- **Linear:** the previous `Ordered`, a plain `Vec` with linear lookups.
- **Sorted:** `BTreeMap`.
- **Hashed:** `HashMap`.
- **Indexed:** `indexmap` 2.14.2, used in the benchmark crate only.

Measurement notes:

- **Instructions:** Callgrind.
- **Times:** wall-clock medians from release builds on a 16-core x86 machine (`benches/examples/value_wall.rs`, `value_worst_case.rs`).
- **Memory:** total bytes allocated by the whole process, input included.

| | serde_json | Linear | **Ordered** | Sorted | Hashed | Indexed |
|---|---:|---:|---:|---:|---:|---:|
| parse canada: instructions | 83.4 M | 129.9 M | 130.4 M | 129.6 M | 132.4 M | 134.6 M |
| parse canada: time | 11.9 ms | 10.6 ms | 11.0 ms | 10.9 ms | 14.8 ms | 17.8 ms |
| parse twitter: instructions | 17.3 M | 20.0 M | 19.7 M | 25.2 M | 27.2 M | 26.6 M |
| parse twitter: time | 1.88 ms | 1.68 ms | 1.76 ms | 2.30 ms | 2.43 ms | 2.36 ms |
| parse citm: instructions | 40.8 M | 45.7 M | 45.4 M | 49.6 M | 54.9 M | 60.1 M |
| parse citm: time | 4.87 ms | 3.38 ms | 3.50 ms | 4.75 ms | 4.82 ms | 5.02 ms |
| twitter, 301 lookups in small objects: instructions | – | 85 K | 91 K | 157 K | 103 K | 109 K |
| twitter lookups: time | 11.4 µs | 8.5 µs | 9.3 µs | 12.1 µs | 7.6 µs | 8.2 µs |
| citm, 184 lookups in a 184-key object: instructions | – | 550 K | 104 K | 94 K | 51 K | 54 K |
| citm lookups: time | 6.5 µs | 51.1 µs | 12.4 µs | 5.7 µs | 3.8 µs | 3.6 µs |
| encode twitter: time | 365 µs | 612 µs | 592 µs | 648 µs | 717 µs | 594 µs |
| memory canada | 14.6 MB | 23.8 MB | 23.8 MB | 23.8 MB | 33.1 MB | 39.2 MB |
| memory twitter | 2.7 MB | 4.0 MB | 4.9 MB | 3.4 MB | 5.2 MB | 5.8 MB |
| memory citm | 9.7 MB | 7.6 MB | 8.8 MB | 13.2 MB | 10.0 MB | 10.6 MB |

Worst case: one object with n keys, every key looked up once (wall
clock, cost per lookup):

| n | Linear | **Ordered** | Sorted | Hashed | Indexed |
|---:|---:|---:|---:|---:|---:|
| 100 | 152 ns | 56 ns | 29 ns | 18 ns | 19 ns |
| 1,000 | 1.4 µs | 85 ns | 49 ns | 19 ns | 19 ns |
| 10,000 | 14 µs | 137 ns | 84 ns | 28 ns | 22 ns |
| 100,000 | 148 µs (14.8 s in total) | 170 ns (17 ms in total) | 107 ns | 90 ns | 34 ns |

Choosing the index threshold (twitter's `user` objects have 40 keys):

| Threshold | twitter parse | twitter lookups | citm lookups | lookups at n = 100 |
|---:|---:|---:|---:|---:|
| 32 | 1.89 ms | 16.6 µs | 12.6 µs | 85 ns |
| **64** | 1.69 ms | 8.9 µs | 12.4 µs | 56 ns |
| 128 | 1.66 ms | 9.4 µs | 12.5 µs | 163 ns |

Below about 64 keys, a linear scan that rejects most keys on length
beats both building an index and binary search's full string
comparisons.

`Ordered` uses more memory than Linear on twitter because each entry
carries a `key_span` (24 bytes) that is never filled in. Dropping it
would bring `Ordered` down to Linear's figure.

### Span layout and key spans

- `Span` stores its end as `end + 1` in a `NonZeroU64`, so `Option<Span>`
  takes 16 bytes instead of 24. Offsets stay 64-bit. A `Value` node drops
  from 56 to 48 bytes.
- Key spans moved out of each `Ordered` entry (entries drop from 104 to
  72 bytes). They now live in an optional side table, filled only with
  `DecodeOptions::key_spans(true)`.

| Memory (whole process) | serde_json | before | **now** | now + key spans |
|---|---:|---:|---:|---:|
| canada | 14.6 MB | 23.8 MB | 20.7 MB | 20.7 MB |
| twitter | 2.7 MB | 4.9 MB | 3.7 MB | 4.4 MB |
| citm | 9.7 MB | 8.8 MB | 6.9 MB | 8.3 MB |

| Parse time (median) | serde_json | **Ordered** | Ordered + key spans |
|---|---:|---:|---:|
| canada | 11.6 ms | 11.3 ms | 11.2 ms |
| twitter | 1.90 ms | 1.71 ms | 1.97 ms |
| citm | 4.68 ms | 3.37 ms | 4.23 ms |
