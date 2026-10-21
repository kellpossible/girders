//! Wall-clock cost of `Value` objects with many keys: parsing one object of
//! n keys, then looking up every key once. Release build:
//! `cargo run --release -p girders-benches --example value_lookups`.
//! An optional argument names a key set: `normal` (default) or `attack`
//! (keys built to collide by someone who knows the hash key, decoded with
//! that key: the worst case, as without `std` and without a key).

use girders::DecodeOptions;
use girders_json::value;
use std::hint::black_box;
use std::time::{Duration, Instant};

fn keys(n: usize, set: &str) -> Vec<String> {
    match set {
        "attack" => girders_json::value::keys_forcing_fallback(n, KNOWN_KEY),
        _ => (0..n)
            .map(|i| format!("key_{i:x}_{}", i * 7919 % 1000))
            .collect(),
    }
}

/// The hash key the attacker knows (in `attack` mode).
const KNOWN_KEY: [u64; 2] = [1, 2];

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn main() {
    let set = std::env::args().nth(1).unwrap_or_else(|| "normal".into());
    println!("key set: {set}");
    let options = match set.as_str() {
        "attack" => DecodeOptions::new().hash_key(KNOWN_KEY),
        _ => DecodeOptions::new(),
    };
    // Warm up (caches, CPU frequency, the per-process hash key) so the
    // first row isn't penalised.
    for _ in 0..200 {
        let _ = girders_json::decode_with(r#"{"a":1}"#, &value(), options);
    }
    let warm: String = format!(
        "{{{}}}",
        (0..1000)
            .map(|i| format!("\"w{i}\":{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    for _ in 0..200 {
        black_box(girders_json::decode_with(&warm, &value(), options).unwrap());
    }
    println!("{:>8} {:>14} {:>14}", "n", "parse", "per lookup");
    for n in [100, 1_000, 10_000, 100_000] {
        let keys = keys(n, &set);
        let json = format!(
            "{{{}}}",
            keys.iter()
                .enumerate()
                .map(|(i, k)| format!("{}:{i}", girders_json::encode(k, &girders::codec::string())))
                .collect::<Vec<_>>()
                .join(",")
        );
        let runs = (2_000_000 / n).clamp(5, 200);
        let parse = median(
            (0..runs)
                .map(|_| {
                    let t = Instant::now();
                    black_box(
                        girders_json::decode_with(black_box(&json), &value(), options).unwrap(),
                    );
                    t.elapsed()
                })
                .collect(),
        );
        let v = girders_json::decode_with(&json, &value(), options).unwrap();
        // Look keys up in a scrambled order, so neither index benefits from
        // lookups that follow its layout.
        let mut keys = keys.clone();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for i in (1..keys.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            keys.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        let lookups = median(
            (0..runs)
                .map(|_| {
                    let t = Instant::now();
                    let mut found = 0;
                    for k in &keys {
                        found += black_box(&v).get(black_box(k)).is_some() as usize;
                    }
                    assert_eq!(found, n);
                    t.elapsed()
                })
                .collect(),
        );
        println!("{n:>8} {:>14?} {:>14?}", parse, lookups / n as u32);
    }
}
