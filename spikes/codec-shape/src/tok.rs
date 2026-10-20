//! A token-stream "format" so the spike can exercise the traits without
//! a real JSON cursor. Location = token index.

use crate::core::*;
#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}, vec, vec::Vec};


#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Str(String),
    Int(i64),
    Null,
    BeginArr,
    EndArr,
    BeginObj,
    Key(String),
    EndObj,
}

pub struct TokReader {
    toks: Vec<Tok>,
    pos: usize,
}

impl TokReader {
    pub fn new(toks: Vec<Tok>) -> Self {
        TokReader { toks, pos: 0 }
    }
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn kind_of(t: Option<&Tok>) -> Kind {
        match t {
            None => Kind::Eof,
            Some(Tok::Str(_)) => Kind::String,
            Some(Tok::Int(_)) => Kind::Number,
            Some(Tok::Null) => Kind::Null,
            Some(Tok::BeginArr) => Kind::Array,
            Some(Tok::BeginObj) => Kind::Object,
            Some(_) => Kind::Eof,
        }
    }
    fn mismatch<T>(&self, expected: &'static str) -> Res<T, Self> {
        Err(Error::at(
            Problem::TypeMismatch { expected, found: Self::kind_of(self.peek()) },
            self.pos,
        ))
    }
    fn expect(&mut self, t: Tok, what: &'static str) -> Res<(), Self> {
        if self.peek() == Some(&t) {
            self.pos += 1;
            Ok(())
        } else {
            self.mismatch(what)
        }
    }
}

impl Reader for TokReader {
    type Location = usize;
    type Checkpoint = usize;

    fn peek_kind(&mut self) -> Res<Kind, Self> {
        Ok(Self::kind_of(self.peek()))
    }
    fn read_str(&mut self, buf: &mut String) -> Res<(), Self> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                buf.push_str(s);
                self.pos += 1;
                Ok(())
            }
            _ => self.mismatch("string"),
        }
    }
    fn read_i64(&mut self) -> Res<i64, Self> {
        match self.peek() {
            Some(Tok::Int(i)) => {
                let i = *i;
                self.pos += 1;
                Ok(i)
            }
            _ => self.mismatch("integer"),
        }
    }
    fn read_null(&mut self) -> Res<(), Self> {
        self.expect(Tok::Null, "null")
    }
    fn begin_array(&mut self) -> Res<(), Self> {
        self.expect(Tok::BeginArr, "array")
    }
    fn array_has_next(&mut self, _first: bool) -> Res<bool, Self> {
        Ok(self.peek() != Some(&Tok::EndArr))
    }
    fn end_array(&mut self) -> Res<(), Self> {
        self.expect(Tok::EndArr, "end of array")
    }
    fn begin_object(&mut self) -> Res<(), Self> {
        self.expect(Tok::BeginObj, "object")
    }
    fn object_next_key(&mut self, _first: bool, buf: &mut String) -> Res<bool, Self> {
        match self.peek() {
            Some(Tok::Key(k)) => {
                buf.push_str(k);
                self.pos += 1;
                Ok(true)
            }
            Some(Tok::EndObj) => Ok(false),
            _ => Err(Error::at(Problem::Syntax("expected key".into()), self.pos)),
        }
    }
    fn end_object(&mut self) -> Res<(), Self> {
        self.expect(Tok::EndObj, "end of object")
    }
    fn skip_value(&mut self) -> Res<(), Self> {
        let mut depth = 0usize;
        loop {
            let t = self.toks.get(self.pos).cloned();
            self.pos += 1;
            match t {
                None => return Err(Error::at(Problem::Syntax("eof".into()), self.pos)),
                Some(Tok::BeginArr | Tok::BeginObj) => depth += 1,
                Some(Tok::EndArr | Tok::EndObj) => depth -= 1,
                Some(Tok::Key(_)) => continue,
                Some(_) => {}
            }
            if depth == 0 {
                return Ok(());
            }
        }
    }
    fn checkpoint(&mut self) -> usize {
        self.pos
    }
    fn rewind(&mut self, cp: &usize) {
        self.pos = *cp;
    }
    fn release(&mut self, _cp: usize) {}
}

#[derive(Default)]
pub struct TokWriter {
    pub toks: Vec<Tok>,
}

impl Serializer for TokWriter {
    type Error = core::convert::Infallible;
    fn write_str(&mut self, v: &str) -> Result<(), Self::Error> {
        self.toks.push(Tok::Str(v.into()));
        Ok(())
    }
    fn write_i64(&mut self, v: i64) -> Result<(), Self::Error> {
        self.toks.push(Tok::Int(v));
        Ok(())
    }
    fn write_null(&mut self) -> Result<(), Self::Error> {
        self.toks.push(Tok::Null);
        Ok(())
    }
    fn begin_array(&mut self, _len: Option<usize>) -> Result<(), Self::Error> {
        self.toks.push(Tok::BeginArr);
        Ok(())
    }
    fn array_element(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn end_array(&mut self) -> Result<(), Self::Error> {
        self.toks.push(Tok::EndArr);
        Ok(())
    }
    fn begin_object(&mut self, _len: Option<usize>) -> Result<(), Self::Error> {
        self.toks.push(Tok::BeginObj);
        Ok(())
    }
    fn object_key(&mut self, key: &str) -> Result<(), Self::Error> {
        self.toks.push(Tok::Key(key.into()));
        Ok(())
    }
    fn end_object(&mut self) -> Result<(), Self::Error> {
        self.toks.push(Tok::EndObj);
        Ok(())
    }
}

pub fn decode<C: Decoder>(c: &C, toks: Vec<Tok>) -> Result<C::Output, Error<usize>> {
    c.decode(&mut TokReader::new(toks), &mut Cx::default())
}

pub fn encode<C: Encoder>(c: &C, v: &C::Input) -> Vec<Tok> {
    let mut w = TokWriter::default();
    let Ok(()) = c.encode(v, &mut w, &mut Ecx::default());
    w.toks
}
