//! Worst case for `Ordered` (linear-scan lookups): one object with `n`
//! keys, look up every key (hits) and as many absent keys (misses).
//! Wall-clock medians.
use girders_benches::{Indexed, Linear};
use girders_json::{Hashed, ObjectMap, Ordered, Sorted, Value};
use std::hint::black_box;
use std::time::Instant;

fn doc(n: usize) -> (String, Vec<String>, Vec<String>) {
    let keys: Vec<String> = (0..n).map(|i| format!("key_{i:06}")).collect();
    let misses: Vec<String> = (0..n).map(|i| format!("nope_{i:06}")).collect();
    let body: Vec<String> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| format!("\"{k}\":{i}"))
        .collect();
    (format!("{{{}}}", body.join(",")), keys, misses)
}

fn median(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64()
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t[runs / 2]
}

fn fmt(secs: f64) -> String {
    if secs < 1e-6 {
        format!("{:.0} ns", secs * 1e9)
    } else if secs >= 1.0 {
        format!("{secs:.2} s")
    } else if secs >= 1e-3 {
        format!("{:.2} ms", secs * 1e3)
    } else {
        format!("{:.1} µs", secs * 1e6)
    }
}

fn measure<M: ObjectMap>(
    json: &str,
    keys: &[String],
    misses: &[String],
    runs: usize,
) -> (f64, f64, f64) {
    let parse = median(runs, || {
        black_box(girders_json::parse::<M>(black_box(json)).unwrap());
    });
    let v: Value<M> = girders_json::parse(json).unwrap();
    let hits = median(runs, || {
        black_box(keys.iter().filter(|k| v.get(k).is_some()).count());
    });
    let miss = median(runs, || {
        black_box(misses.iter().filter(|k| v.get(k).is_some()).count());
    });
    (parse, hits, miss)
}

fn main() {
    println!(
        "{:>7}  {:<8} {:>10} {:>14} {:>14} {:>16}",
        "n", "map", "parse", "n hit lookups", "n miss lookups", "per hit lookup"
    );
    for n in [10usize, 100, 1_000, 10_000, 100_000] {
        let (json, keys, misses) = doc(n);
        let runs = if n >= 100_000 {
            3
        } else if n >= 10_000 {
            7
        } else {
            51
        };
        for (name, (p, h, m)) in [
            (
                "Linear",
                measure::<Linear>(&json, &keys, &misses, if n >= 100_000 { 1 } else { runs }),
            ),
            ("Ordered", measure::<Ordered>(&json, &keys, &misses, runs)),
            ("Sorted", measure::<Sorted>(&json, &keys, &misses, runs)),
            ("Hashed", measure::<Hashed>(&json, &keys, &misses, runs)),
            ("Indexed", measure::<Indexed>(&json, &keys, &misses, runs)),
        ] {
            println!(
                "{n:>7}  {name:<8} {:>10} {:>14} {:>14} {:>16}",
                fmt(p),
                fmt(h),
                fmt(m),
                fmt(h / n as f64)
            );
        }
    }
}
