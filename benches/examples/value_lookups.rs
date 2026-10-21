//! Wall-clock cost of `Value` objects with many keys: parsing one object of
//! n keys, then looking up every key once. Release build:
//! `cargo run --release -p girders-benches --example value_lookups`.
//! An optional argument names a key set: `normal` (default) or `colliding`
//! (keys built to collide under the object index's hash, if it has one).

use std::hint::black_box;
use std::time::{Duration, Instant};

fn keys(n: usize, set: &str) -> Vec<String> {
    match set {
        "colliding" => girders_json::value::colliding_keys(n),
        _ => (0..n)
            .map(|i| format!("key_{i:x}_{}", i * 7919 % 1000))
            .collect(),
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn main() {
    let set = std::env::args().nth(1).unwrap_or_else(|| "normal".into());
    println!("key set: {set}");
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
                    black_box(girders_json::parse(black_box(&json)).unwrap());
                    t.elapsed()
                })
                .collect(),
        );
        let v = girders_json::parse(&json).unwrap();
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
