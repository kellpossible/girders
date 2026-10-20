//! JSONTestSuite conformance (pinned commit, fetched by
//! `mise run fetch-jsontestsuite`), over every input path: slice, an
//! `io::Read` source at several chunk sizes, and the push decoder.

use girders::DecodeOptions;
use girders_json::{Ordered, StreamDecoder, Value, value};
use std::path::PathBuf;

const COMMIT: &str = "1ef36fa01286573e846ac449e8683f8833c5b26a";

fn suite() -> Option<Vec<(String, Vec<u8>)>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("JSONTestSuite-{COMMIT}"))
        .join("test_parsing");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!(
            "JSONTestSuite missing at {} — run `mise run fetch-jsontestsuite`; skipping",
            dir.display()
        );
        return None;
    };
    let mut files: Vec<_> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    files.sort();
    Some(files)
}

type Parse = fn(&[u8]) -> Result<Value<Ordered>, String>;
type ParseFn = dyn Fn(&[u8]) -> Result<Value<Ordered>, String>;

fn via_slice(b: &[u8]) -> Result<Value<Ordered>, String> {
    girders_json::decode_bytes(b, &value::<Ordered>()).map_err(|e| e.to_string())
}

fn via_reader(chunk: usize) -> impl Fn(&[u8]) -> Result<Value<Ordered>, String> {
    move |b| {
        girders_json::from_reader_with(b, &value::<Ordered>(), DecodeOptions::new(), chunk)
            .map_err(|e| e.to_string())
    }
}

/// Push decoder, fed `chunk` bytes at a time; exactly one value expected.
fn via_push(chunk: usize) -> impl Fn(&[u8]) -> Result<Value<Ordered>, String> {
    move |b| {
        let mut d = StreamDecoder::new(value::<Ordered>());
        let mut out = Vec::new();
        for c in b.chunks(chunk.max(1)) {
            d.push(c);
            while let Some(r) = d.next() {
                out.push(r.map_err(|e| e.to_string())?);
            }
        }
        d.finish();
        while let Some(r) = d.next() {
            out.push(r.map_err(|e| e.to_string())?);
        }
        match out.len() {
            1 => Ok(out.pop().unwrap()),
            0 => Err("no value".into()),
            n => Err(format!("{n} values")),
        }
    }
}

fn run(name: &str, parse: &ParseFn) -> (usize, usize, Vec<String>) {
    let Some(files) = suite() else {
        return (0, 0, Vec::new());
    };
    let mut failures = Vec::new();
    let (mut i_accept, mut i_reject) = (0, 0);
    for (file, bytes) in &files {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(bytes)))
            .map_err(|_| "panic".to_string())
            .and_then(|r| r.map(|_| ()));
        match file.as_bytes()[0] {
            b'y' if ok.is_err() => failures.push(format!("{file}: rejected: {}", ok.unwrap_err())),
            b'n' if ok.is_ok() => failures.push(format!("{file}: accepted")),
            b'i' if ok.is_ok() => i_accept += 1,
            b'i' => i_reject += 1,
            _ => {}
        }
    }
    eprintln!(
        "[{name}] {} files, {} failures, i: {i_accept} accept / {i_reject} reject",
        files.len(),
        failures.len()
    );
    (i_accept, i_reject, failures)
}

#[test]
fn slice() {
    let (.., failures) = run("slice", &(via_slice as Parse));
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn reader_chunked() {
    let baseline = run("slice", &(via_slice as Parse));
    for chunk in [1, 7, 4096] {
        let r = run(&format!("reader/{chunk}"), &via_reader(chunk));
        assert!(r.2.is_empty(), "chunk {chunk}: {:#?}", r.2);
        assert_eq!(
            (r.0, r.1),
            (baseline.0, baseline.1),
            "chunk {chunk}: i_ outcomes differ from slice"
        );
    }
}

#[test]
fn push_chunked() {
    let baseline = run("slice", &(via_slice as Parse));
    for chunk in [1, 7, 4096] {
        let r = run(&format!("push/{chunk}"), &via_push(chunk));
        assert!(r.2.is_empty(), "chunk {chunk}: {:#?}", r.2);
        assert_eq!(
            (r.0, r.1),
            (baseline.0, baseline.1),
            "chunk {chunk}: i_ outcomes differ from slice"
        );
    }
}

/// Every path must produce the same tree as the slice path.
#[test]
fn paths_agree() {
    let Some(files) = suite() else { return };
    for (file, bytes) in files.iter().filter(|(f, _)| f.starts_with('y')) {
        let a = via_slice(bytes).unwrap().without_spans();
        assert_eq!(via_reader(3)(bytes).unwrap().without_spans(), a, "{file}");
        assert_eq!(via_push(5)(bytes).unwrap().without_spans(), a, "{file}");
    }
}
