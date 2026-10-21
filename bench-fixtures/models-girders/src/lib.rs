//! Compile-time benchmark fixture: the json-benchmark models with girders codecs.
#![allow(dead_code)]

pub mod canada;
pub mod citm;
pub mod twitter;

use girders_json::decode as dec;

pub fn run(kind: &str, json: &str) -> usize {
    #[cfg(not(feature = "decode-only"))]
    return match kind {
        "twitter" => girders_json::encode(&dec(json, &twitter::codec()).unwrap(), &twitter::codec()).unwrap().len(),
        "citm" => girders_json::encode(&dec(json, &citm::codec()).unwrap(), &citm::codec()).unwrap().len(),
        _ => girders_json::encode(&dec(json, &canada::codec()).unwrap(), &canada::codec()).unwrap().len(),
    };
    #[cfg(feature = "decode-only")]
    return match kind {
        "twitter" => dec(json, &twitter::codec()).unwrap().statuses.len(),
        "citm" => dec(json, &citm::codec()).unwrap().events.len(),
        _ => dec(json, &canada::codec()).unwrap().features.len(),
    };
}
