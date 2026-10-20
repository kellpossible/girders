//! Compile-time benchmark fixture: the json-benchmark models with serde derives.
#![allow(dead_code)]

pub mod canada;
pub mod citm;
pub mod twitter;

pub fn run(kind: &str, json: &str) -> usize {
    #[cfg(not(feature = "decode-only"))]
    return match kind {
        "twitter" => serde_json::to_string(&serde_json::from_str::<twitter::Twitter>(json).unwrap()).unwrap().len(),
        "citm" => serde_json::to_string(&serde_json::from_str::<citm::CitmCatalog>(json).unwrap()).unwrap().len(),
        _ => serde_json::to_string(&serde_json::from_str::<canada::Canada>(json).unwrap()).unwrap().len(),
    };
    #[cfg(feature = "decode-only")]
    return match kind {
        "twitter" => serde_json::from_str::<twitter::Twitter>(json).unwrap().statuses.len(),
        "citm" => serde_json::from_str::<citm::CitmCatalog>(json).unwrap().events.len(),
        _ => serde_json::from_str::<canada::Canada>(json).unwrap().features.len(),
    };
}
