//! Rough wall-clock timings for Value map types (median of N runs).
use girders_benches::{Indexed, Linear, data};
use girders_json::{Hashed, Node, ObjectMap, Ordered, Sorted, Value, value};
use std::hint::black_box;
use std::time::Instant;

fn median_us(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t[runs / 2]
}

fn twitter_lookups<M: ObjectMap>(v: &Value<M>) -> usize {
    let mut n = 0;
    if let Some(Node::Array(statuses)) = v.get("statuses").map(|s| &s.node) {
        for st in statuses {
            n += st.get("id").is_some() as usize;
            n += st.get("text").is_some() as usize;
            if let Some(u) = st.get("user") {
                n += u.get("screen_name").is_some() as usize;
            }
        }
    }
    n
}

fn row<M: ObjectMap>(name: &str, docs: &[(String, String)]) {
    print!("{name:<10}");
    for (_, json) in docs {
        let us = median_us(31, || {
            black_box(girders_json::parse::<M>(black_box(json)).unwrap());
        });
        print!(" {us:>10.0}");
    }
    let tw: Value<M> = girders_json::parse(&docs[1].1).unwrap();
    let us = median_us(201, || {
        black_box(twitter_lookups(black_box(&tw)));
    });
    print!(" {us:>12.1}");
    let citm: Value<M> = girders_json::parse(&docs[2].1).unwrap();
    let events = citm.get("events").unwrap();
    let keys: Vec<String> = events.keys().into_iter().map(String::from).collect();
    let us = median_us(201, || {
        black_box(keys.iter().filter(|k| events.get(k).is_some()).count());
    });
    print!(" {us:>12.1}");
    let us = median_us(31, || {
        black_box(girders_json::encode(black_box(&tw), &value::<M>()));
    });
    println!(" {us:>12.0}");
}

fn main() {
    let docs: Vec<(String, String)> = ["canada.json", "twitter.json", "citm_catalog.json"]
        .iter()
        .map(|n| (n.to_string(), data(n)))
        .collect();
    println!(
        "{:<10} {:>10} {:>10} {:>10} {:>12} {:>12} {:>12}",
        "µs", "parse can", "parse twt", "parse citm", "lookup twt", "lookup citm", "encode twt"
    );
    print!("{:<10}", "serde");
    for (_, json) in &docs {
        let us = median_us(31, || {
            black_box(serde_json::from_str::<serde_json::Value>(black_box(json)).unwrap());
        });
        print!(" {us:>10.0}");
    }
    let tw: serde_json::Value = serde_json::from_str(&docs[1].1).unwrap();
    let us = median_us(201, || {
        let mut n = 0;
        for st in tw["statuses"].as_array().unwrap() {
            n += st.get("id").is_some() as usize + st.get("text").is_some() as usize;
            if let Some(u) = st.get("user") {
                n += u.get("screen_name").is_some() as usize;
            }
        }
        black_box(n);
    });
    print!(" {us:>12.1}");
    let citm: serde_json::Value = serde_json::from_str(&docs[2].1).unwrap();
    let events = citm["events"].as_object().unwrap();
    let keys: Vec<String> = events.keys().cloned().collect();
    let us = median_us(201, || {
        black_box(
            keys.iter()
                .filter(|k| events.get(k.as_str()).is_some())
                .count(),
        );
    });
    print!(" {us:>12.1}");
    let us = median_us(31, || {
        black_box(serde_json::to_string(black_box(&tw)).unwrap());
    });
    println!(" {us:>12.0}");
    row::<Linear>("Linear", &docs);
    row::<Ordered>("Ordered", &docs);
    print!("{:<10}", "Ord+keysp");
    for (_, json) in &docs {
        let us = median_us(31, || {
            black_box(
                girders_json::decode_with(
                    black_box(json),
                    &value::<Ordered>(),
                    girders::DecodeOptions::new().key_spans(true),
                )
                .unwrap(),
            );
        });
        print!(" {us:>10.0}");
    }
    println!();
    row::<Sorted>("Sorted", &docs);
    row::<Hashed>("Hashed", &docs);
    row::<Indexed>("Indexed", &docs);
}
