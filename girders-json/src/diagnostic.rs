//! Text diagnostics: resolve an error's byte span against the source into
//! line/column and a caret snippet. Line and column are computed only
//! when rendering; the decoder carries byte offsets.

use crate::pointer::Pointer;
use alloc::string::String;
use core::fmt::Write;
use girders::error::ErrorDetail;
use girders::{DecodeError, Span};

/// Resolve a 0-based byte offset to a 1-based `(line, column)`. Columns
/// count `char`s, not grapheme clusters.
pub fn resolve(source: &str, byte_offset: u64) -> (usize, usize) {
    let (line, col, _) = locate(source, byte_offset as usize);
    (line, col)
}

fn locate(source: &str, target: usize) -> (usize, usize, &str) {
    let target = target.min(source.len());
    let mut line = 1;
    let mut line_start = 0;
    for (i, b) in source.as_bytes()[..target].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    // A target inside a multi-byte char snaps back to its start.
    let mut t = target;
    while !source.is_char_boundary(t) {
        t -= 1;
    }
    let col = source[line_start..t].chars().count() + 1;
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |n| line_start + n);
    let text = source[line_start..line_end].trim_end_matches('\r');
    (line, col, text)
}

/// `<pointer>: <problem>`
pub fn headline<F: core::fmt::Display>(detail: &ErrorDetail<Span, F>) -> String {
    let mut out = String::new();
    if detail.path_len() > 0 {
        let _ = write!(out, "{}: ", Pointer::from_path(detail.path()));
    }
    let _ = write!(out, "{}", detail.problem);
    out
}

fn snippet(out: &mut String, line: usize, col: usize, text: &str) {
    let no = alloc::format!("{line}");
    let pad: String = core::iter::repeat_n(' ', no.len()).collect();
    let _ = write!(out, " {pad} |\n {no} | {text}\n {pad} | ");
    for _ in 1..col {
        out.push(' ');
    }
    out.push('^');
}

fn render_detail<F: core::fmt::Display>(
    out: &mut String,
    d: &ErrorDetail<Span, F>,
    source: &str,
    with_context: bool,
) {
    out.push_str(&headline(d));
    if let Some(span) = &d.location {
        let (line, col, text) = locate(source, span.start() as usize);
        let _ = write!(out, "\n  at line {line}, column {col}\n");
        snippet(out, line, col, text);
    }
    if with_context && !d.context.is_empty() {
        out.push_str("\ncontext:");
        for (i, frame) in d.context.iter().enumerate() {
            let _ = write!(out, "\n  {i}: {}", frame.description);
            if let Some(span) = &frame.location {
                let (line, col) = resolve(source, span.start());
                let _ = write!(out, " (line {line}, column {col})");
            }
        }
    }
}

/// Compiler-style rendering with a caret snippet:
///
/// ```text
/// /server/port: expected int, found string
///   at line 5, column 13
///    |
///  5 |     "port": "eighty"
///    |             ^
/// ```
pub fn render<F: core::fmt::Display>(err: &DecodeError<Span, F>, source: &str) -> String {
    let mut out = String::new();
    for (i, d) in err.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        render_detail(&mut out, d, source, false);
    }
    out
}

/// `render` plus any context frames.
pub fn render_detailed<F: core::fmt::Display>(err: &DecodeError<Span, F>, source: &str) -> String {
    let mut out = String::new();
    for (i, d) in err.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        render_detail(&mut out, d, source, true);
    }
    out
}

/// One line per error: `/server/port: expected int, found string (at
/// line 3, column 13) [while decoding the server entry]`.
pub fn render_short<F: core::fmt::Display>(err: &DecodeError<Span, F>, source: &str) -> String {
    let mut out = String::new();
    for (i, d) in err.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&headline(d));
        if let Some(span) = &d.location {
            let (line, col) = resolve(source, span.start());
            let _ = write!(out, " (at line {line}, column {col})");
        }
        if !d.context.is_empty() {
            out.push_str(" [");
            for (j, frame) in d.context.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                out.push_str(&frame.description);
            }
            out.push(']');
        }
    }
    out
}
