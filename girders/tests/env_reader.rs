//! A flat env-var "format" whose error location is the variable name
//! (error context isn't always a byte span). One codec value
//! decodes from it unchanged.

use girders::prelude::*;
use girders::{Number, reader::DecodeResult};

/// `APP_*` variables as a flat object of strings; numbers and bools are
/// parsed from the text.
struct EnvReader {
    vars: Vec<(String, String)>,
    i: usize,
    in_value: bool,
    opened: bool,
}

impl EnvReader {
    fn new(vars: &[(&str, &str)]) -> Self {
        EnvReader {
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            i: 0,
            in_value: false,
            opened: false,
        }
    }
    fn var(&self) -> String {
        self.vars
            .get(self.i)
            .map(|v| v.0.clone())
            .unwrap_or_default()
    }
    fn take(&mut self) -> (String, String) {
        self.in_value = false;
        let v = self.vars[self.i].clone();
        self.i += 1;
        v
    }
    fn parse<T: std::str::FromStr>(&mut self, expected: &'static str) -> DecodeResult<T, Self> {
        let (name, text) = self.take();
        text.parse().map_err(|_| {
            DecodeError::at(Problem::mismatch(expected, format!("`{text}`")), Some(name))
        })
    }
}

impl Reader for EnvReader {
    type Location = String;
    type Checkpoint = (usize, bool);
    fn peek_kind(&mut self) -> DecodeResult<Kind, Self> {
        Ok(if !self.opened {
            Kind::Object
        } else if self.in_value {
            Kind::String
        } else {
            Kind::Eof
        })
    }
    fn here(&mut self) -> Option<String> {
        Some(if self.opened {
            self.var()
        } else {
            "environment".into()
        })
    }
    fn since(&mut self, start: &String) -> String {
        start.clone()
    }
    fn read_bool(&mut self) -> DecodeResult<bool, Self> {
        self.parse("bool")
    }
    fn read_null(&mut self) -> DecodeResult<(), Self> {
        Err(DecodeError::at(
            Problem::mismatch("null", "string"),
            Some(self.var()),
        ))
    }
    fn read_str(&mut self, buf: &mut String) -> DecodeResult<(), Self> {
        buf.push_str(&self.take().1);
        Ok(())
    }
    fn read_i64(&mut self) -> DecodeResult<i64, Self> {
        self.parse("int")
    }
    fn read_u64(&mut self) -> DecodeResult<u64, Self> {
        self.parse("int")
    }
    fn read_f64(&mut self) -> DecodeResult<f64, Self> {
        self.parse("float")
    }
    fn read_number(&mut self) -> DecodeResult<Number, Self> {
        self.parse::<f64>("number").map(Number::F64)
    }
    fn read_number_text(&mut self, buf: &mut String) -> DecodeResult<(), Self> {
        self.read_str(buf)
    }
    fn begin_array(&mut self) -> DecodeResult<(), Self> {
        Err(DecodeError::at(
            Problem::mismatch("array", "string"),
            Some(self.var()),
        ))
    }
    fn array_has_next(&mut self, _: bool) -> DecodeResult<bool, Self> {
        Ok(false)
    }
    fn end_array(&mut self) -> DecodeResult<(), Self> {
        Ok(())
    }
    fn begin_object(&mut self) -> DecodeResult<(), Self> {
        self.opened = true;
        Ok(())
    }
    fn object_next_key(&mut self, _: bool, buf: &mut String) -> DecodeResult<bool, Self> {
        match self.vars.get(self.i) {
            None => Ok(false),
            Some((name, _)) => {
                buf.push_str(&name.strip_prefix("APP_").unwrap_or(name).to_lowercase());
                self.in_value = true;
                Ok(true)
            }
        }
    }
    fn end_object(&mut self) -> DecodeResult<(), Self> {
        Ok(())
    }
    fn skip_value(&mut self) -> DecodeResult<(), Self> {
        self.take();
        Ok(())
    }
    fn checkpoint(&mut self) -> (usize, bool) {
        (self.i, self.in_value)
    }
    fn rewind(&mut self, cp: &(usize, bool)) {
        (self.i, self.in_value) = *cp;
    }
    fn release(&mut self, _: (usize, bool)) {}
}

#[derive(Debug, PartialEq)]
struct Config {
    host: String,
    port: u16,
    debug: bool,
}

fn config() -> impl Codec<Output = Config> {
    record((
        field("host", string(), |c: &Config| &c.host),
        field("port", u16(), |c: &Config| &c.port),
        field_default("debug", bool(), |c: &Config| &c.debug, || false),
    ))
    .build(|(host, port, debug)| Config { host, port, debug })
}

fn decode(vars: &[(&str, &str)], options: DecodeOptions) -> Result<Config, DecodeError<String>> {
    config().decode(&mut EnvReader::new(vars), &mut DecodeContext::new(options))
}

#[test]
fn decodes_env() {
    let c = decode(
        &[("APP_PORT", "8080"), ("APP_HOST", "db")],
        DecodeOptions::new(),
    )
    .unwrap();
    assert_eq!(
        c,
        Config {
            host: "db".into(),
            port: 8080,
            debug: false
        }
    );
}

#[test]
fn errors_point_at_the_variable() {
    let e = decode(
        &[("APP_HOST", "db"), ("APP_PORT", "eighty")],
        DecodeOptions::new(),
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "port: expected int, found `eighty` (at APP_PORT)"
    );

    let e = decode(
        &[
            ("APP_HOST", "db"),
            ("APP_PORT", "70000"),
            ("APP_DEBUG", "maybe"),
            ("APP_HSOT", "x"),
        ],
        DecodeOptions::strict(),
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "port: 70000 is out of range for u16 (0..=65535) (at APP_PORT)\n\
         debug: expected bool, found `maybe` (at APP_DEBUG)\n\
         hsot: unknown field `hsot` (at APP_HSOT)"
    );

    let e = decode(&[("APP_PORT", "1")], DecodeOptions::new()).unwrap_err();
    assert_eq!(
        e.to_string(),
        "missing required field `host` (at environment)"
    );
}
