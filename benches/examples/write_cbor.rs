//! Write each corpus as CBOR (via the typed models) next to the scratch dir
//! given as the first argument, for checking with other CBOR decoders.
use girders_benches::{canada, citm, data, twitter};

fn main() {
    let out = std::env::args().nth(1).expect("output directory");
    let write =
        |name: &str, bytes: Vec<u8>| std::fs::write(format!("{out}/{name}.cbor"), bytes).unwrap();
    write(
        "twitter",
        girders_cbor::encode(
            &girders_json::decode(&data("twitter.json"), &twitter::codec()).unwrap(),
            &twitter::codec(),
        ),
    );
    write(
        "citm_catalog",
        girders_cbor::encode(
            &girders_json::decode(&data("citm_catalog.json"), &citm::codec()).unwrap(),
            &citm::codec(),
        ),
    );
    write(
        "canada",
        girders_cbor::encode(
            &girders_json::decode(&data("canada.json"), &canada::codec()).unwrap(),
            &canada::codec(),
        ),
    );
}
