//! Conformance with RFC 8949: every Appendix A example decodes to the
//! value its diagnostic notation describes and re-encodes to the same
//! bytes, and every Appendix F example of malformed CBOR is rejected.

use girders::Problem;
use girders_cbor::{Malformed, Value, decode, encode, value};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Non-comment, non-blank lines of a vendored file.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') || l.starts_with("##"))
}

#[test]
fn appendix_a_examples() {
    let mut not_preferred = Vec::new();
    let mut count = 0;
    for line in lines(include_str!("rfc8949/appendix_a.txt")) {
        let (encoded, diagnostic) = line.split_once(' ').unwrap();
        let bytes = hex(encoded);
        let expected = Diag::new(diagnostic).parse();
        let decoded = decode(&bytes, &value()).unwrap_or_else(|e| panic!("{encoded}: {e}"));
        assert_eq!(decoded, expected, "{encoded} should decode as {diagnostic}");
        // Indefinite lengths aren't kept, and floats are re-encoded at
        // their shortest exact width.
        if !diagnostic.contains('_') {
            let again = encode(&decoded, &value());
            if again != bytes {
                not_preferred.push(encoded.to_string());
            }
        }
        count += 1;
    }
    assert_eq!(count, 81);
    // Only the single- and double-precision NaNs and infinities: preferred
    // serialization writes them as half floats.
    assert_eq!(
        not_preferred,
        [
            "fa7f800000",
            "fa7fc00000",
            "faff800000",
            "fb7ff0000000000000",
            "fb7ff8000000000000",
            "fbfff0000000000000"
        ]
    );
}

/// The error each group of Appendix F examples is reported as.
fn expected_malformed(group: &str) -> Malformed {
    let groups = [
        ("Too little data", Malformed::UnexpectedEnd),
        ("Syntax error 1", Malformed::ReservedAdditionalInformation),
        ("Syntax error 2", Malformed::TwoByteSimpleValueBelow32),
        ("Syntax error 3", Malformed::InvalidStringChunk),
        ("Syntax error 4", Malformed::UnexpectedBreak),
        ("Syntax error 5", Malformed::IndefiniteLengthNotAllowed),
    ];
    match groups
        .into_iter()
        .find(|(prefix, _)| group.starts_with(prefix))
    {
        Some((_, malformed)) => malformed,
        None => panic!("unknown Appendix F group {group:?}"),
    }
}

#[test]
fn appendix_f_examples_are_rejected() {
    let mut group = "";
    let mut count = 0;
    for line in lines(include_str!("rfc8949/appendix_f.txt")) {
        if let Some(title) = line.strip_prefix("## ") {
            group = title;
            continue;
        }
        let expected = expected_malformed(group);
        match decode(&hex(line), &value()) {
            Ok(v) => panic!("{group}: {line} decoded as {v}"),
            Err(e) => assert_eq!(e.problem(), &Problem::Format(expected), "{group}: {line}"),
        }
        count += 1;
    }
    assert_eq!(count, 94);
}

#[test]
fn too_much_data_is_rejected() {
    let err = decode(&hex("0000"), &value()).unwrap_err();
    assert_eq!(err.problem(), &Problem::Format(Malformed::TrailingBytes));
}

/// A parser for the diagnostic notation used in Appendix A.
struct Diag<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Diag<'a> {
    fn new(s: &'a str) -> Self {
        Diag {
            s: s.as_bytes(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Value {
        let v = self.value();
        self.ws();
        assert_eq!(self.pos, self.s.len(), "trailing diagnostic text");
        v
    }

    fn ws(&mut self) {
        while self.s.get(self.pos) == Some(&b' ') {
            self.pos += 1;
        }
    }

    fn eat(&mut self, token: &str) -> bool {
        self.ws();
        if self.s[self.pos..].starts_with(token.as_bytes()) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    /// Items up to `close`, after an optional `_` marking indefinite length.
    fn items(&mut self, close: &str, mut item: impl FnMut(&mut Self)) {
        self.eat("_");
        if self.eat(close) {
            return;
        }
        loop {
            item(self);
            if self.eat(close) {
                return;
            }
            assert!(self.eat(","), "expected , at {}", self.pos);
        }
    }

    fn value(&mut self) -> Value {
        self.ws();
        for (word, v) in [
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("null", Value::Null),
            ("undefined", Value::Undefined),
            ("Infinity", Value::Float(f64::INFINITY)),
            ("-Infinity", Value::Float(f64::NEG_INFINITY)),
            ("NaN", Value::Float(f64::NAN)),
        ] {
            if self.eat(word) {
                return v;
            }
        }
        if self.eat("simple(") {
            let n = self.number();
            assert!(self.eat(")"));
            let Value::Integer(n) = n else { panic!() };
            return Value::Simple(n as u8);
        }
        if self.eat("h'") {
            let start = self.pos;
            while self.s[self.pos] != b'\'' {
                self.pos += 1;
            }
            let bytes = hex(std::str::from_utf8(&self.s[start..self.pos]).unwrap());
            self.pos += 1;
            return Value::Bytes(bytes);
        }
        if self.s[self.pos] == b'"' {
            return Value::Text(self.string());
        }
        if self.eat("[") {
            let mut items = Vec::new();
            self.items("]", |d| items.push(d.value()));
            return Value::Array(items);
        }
        if self.eat("{") {
            let mut entries = Vec::new();
            self.items("}", |d| {
                let k = d.value();
                assert!(d.eat(":"));
                entries.push((k, d.value()));
            });
            return Value::Map(entries);
        }
        if self.eat("(") {
            // An indefinite-length string: its chunks concatenated.
            let mut chunks = Vec::new();
            self.items(")", |d| chunks.push(d.value()));
            return match &chunks[..] {
                [Value::Text(_), ..] => Value::Text(
                    chunks
                        .iter()
                        .map(|c| {
                            if let Value::Text(t) = c {
                                t.as_str()
                            } else {
                                panic!()
                            }
                        })
                        .collect(),
                ),
                _ => Value::Bytes(
                    chunks
                        .iter()
                        .flat_map(|c| {
                            if let Value::Bytes(b) = c {
                                b.clone()
                            } else {
                                panic!()
                            }
                        })
                        .collect(),
                ),
            };
        }
        let n = self.number();
        if self.eat("(") {
            let Value::Integer(tag) = n else {
                panic!("tag number")
            };
            let content = self.value();
            assert!(self.eat(")"));
            return Value::Tag(tag as u64, Box::new(content));
        }
        n
    }

    fn number(&mut self) -> Value {
        self.ws();
        let start = self.pos;
        while self.pos < self.s.len()
            && matches!(self.s[self.pos], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e')
        {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.s[start..self.pos]).unwrap();
        if text.contains(['.', 'e']) {
            Value::Float(text.parse().unwrap())
        } else {
            Value::Integer(text.parse().unwrap())
        }
    }

    fn string(&mut self) -> String {
        self.pos += 1;
        let mut units: Vec<u16> = Vec::new();
        loop {
            match self.s[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return String::from_utf16(&units).unwrap();
                }
                b'\\' => {
                    self.pos += 1;
                    match self.s[self.pos] {
                        b'u' => {
                            let h =
                                std::str::from_utf8(&self.s[self.pos + 1..self.pos + 5]).unwrap();
                            units.push(u16::from_str_radix(h, 16).unwrap());
                            self.pos += 5;
                        }
                        c => {
                            units.push(c as u16);
                            self.pos += 1;
                        }
                    }
                }
                _ => {
                    let rest = std::str::from_utf8(&self.s[self.pos..]).unwrap();
                    let c = rest.chars().next().unwrap();
                    let mut buf = [0u16; 2];
                    units.extend_from_slice(c.encode_utf16(&mut buf));
                    self.pos += c.len_utf8();
                }
            }
        }
    }
}

#[test]
fn diagnostic_notation_round_trips_through_display() {
    for line in lines(include_str!("rfc8949/appendix_a.txt")) {
        let (encoded, _) = line.split_once(' ').unwrap();
        let v = decode(&hex(encoded), &value()).unwrap();
        let printed = v.to_string();
        assert_eq!(
            Diag::new(&printed).parse(),
            v,
            "{encoded} printed as {printed}"
        );
    }
}
