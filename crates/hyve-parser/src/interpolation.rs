//! Interpolated text: literal text with `{expr}` / `{expr:spec}` holes.
//!
//! - `{{` and `}}` are literal braces (as in `format!`).
//! - A hole ends at the matching `}`; braces, brackets and parentheses inside
//!   it nest, and string/char literals are skipped.
//! - The format spec starts after the last top-level `:` that is not part of a
//!   `::` path separator (`{exposure():.2}`, `{a::b()}`), matching `format!`'s
//!   spec syntax for what follows.

use hyve_diagnostics::{Diagnostic, Span};

/// A piece of interpolated text. All slices borrow from the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment<'a> {
    /// Literal text (escaped braces arrive as their own one-character segment).
    Text(&'a str),
    /// A hole.
    Expr {
        /// The expression source, trimmed.
        expr: &'a str,
        /// The format spec after `:`, if any (without the colon).
        spec: Option<&'a str>,
        /// Span of the whole hole including braces.
        span: Span,
    },
}

/// Splits `text` into literal and expression segments.
pub fn parse_interpolation(text: &str) -> Result<Vec<Segment<'_>>, Diagnostic> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut lit_start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => {
                push_text(&mut out, &text[lit_start..i]);
                out.push(Segment::Text(&text[i..i + 1]));
                i += 2;
                lit_start = i;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                push_text(&mut out, &text[lit_start..i]);
                out.push(Segment::Text(&text[i..i + 1]));
                i += 2;
                lit_start = i;
            }
            b'}' => {
                return Err(Diagnostic::error("unmatched `}` in text")
                    .with_label(Span::new(i, i + 1), "no `{` opens this")
                    .with_help("write `}}` for a literal `}`"));
            }
            b'{' => {
                push_text(&mut out, &text[lit_start..i]);
                let close = find_close(text, i)?;
                out.push(hole(text, i, close)?);
                i = close + 1;
                lit_start = i;
            }
            _ => i += 1,
        }
    }
    push_text(&mut out, &text[lit_start..]);
    Ok(out)
}

fn push_text<'a>(out: &mut Vec<Segment<'a>>, s: &'a str) {
    if !s.is_empty() {
        out.push(Segment::Text(s));
    }
}

/// Index of the `}` closing the hole that opens at `open`.
fn find_close(text: &str, open: usize) -> Result<usize, Diagnostic> {
    let b = text.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut i = open + 1;
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i).ok_or_else(|| unclosed(open, text, "string literal"))?,
            b'\'' => i = skip_char(b, i),
            c @ (b'(' | b'[' | b'{') => stack.push(c),
            c @ (b')' | b']' | b'}') => {
                let want = match c {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                match stack.pop() {
                    None if c == b'}' => return Ok(i),
                    Some(o) if o == want => {}
                    _ => {
                        return Err(Diagnostic::error("mismatched delimiter in `{…}`")
                            .with_label(Span::new(i, i + 1), "does not match the opening delimiter")
                            .with_help("balance the brackets inside the interpolation"));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(unclosed(open, text, "`{`"))
}

fn unclosed(open: usize, text: &str, what: &str) -> Diagnostic {
    Diagnostic::error(format!("unclosed {what} in text"))
        .with_label(Span::new(open, text.len()), "interpolation starts here")
        .with_help("close it with `}`, or write `{{` for a literal `{`")
}

/// Returns the index of the closing quote of a string starting at `i`.
fn skip_string(b: &[u8], mut i: usize) -> Option<usize> {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Skips a char literal (`'x'`, `'\n'`, `'\''`) starting at `i`; leaves
/// lifetimes and labels (`'a`) alone. Returns the index of the last byte consumed.
fn skip_char(b: &[u8], i: usize) -> usize {
    if b.get(i + 1) == Some(&b'\\') {
        let mut j = i + 2;
        while j < b.len() && b[j] != b'\'' {
            j += 1;
        }
        return j.min(b.len() - 1);
    }
    // 'x' where x may be multi-byte UTF-8.
    let mut j = i + 1;
    if j < b.len() {
        j += 1;
        while j < b.len() && (b[j] & 0xC0) == 0x80 {
            j += 1;
        }
        if b.get(j) == Some(&b'\'') {
            return j;
        }
    }
    i
}

fn hole(text: &str, open: usize, close: usize) -> Result<Segment<'_>, Diagnostic> {
    let inner = &text[open + 1..close];
    let span = Span::new(open, close + 1);
    let b = inner.as_bytes();
    let mut depth = 0i32;
    let mut colon: Option<usize> = None;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i).unwrap_or(b.len()),
            b'\'' => i = skip_char(b, i),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b':' if depth == 0 => {
                let path_sep = b.get(i + 1) == Some(&b':') || (i > 0 && b[i - 1] == b':');
                if !path_sep {
                    colon = Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    let (expr, spec) = match colon {
        Some(c) => (&inner[..c], Some(&inner[c + 1..])),
        None => (inner, None),
    };
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(Diagnostic::error("empty interpolation `{}`")
            .with_label(span, "no expression")
            .with_help("put an expression inside the braces, e.g. `{count()}`, or write `{{}}` for literal braces"));
    }
    Ok(Segment::Expr { expr, spec, span })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exprs(t: &str) -> Vec<(String, Option<String>)> {
        parse_interpolation(t)
            .unwrap()
            .into_iter()
            .filter_map(|s| match s {
                Segment::Expr { expr, spec, .. } => {
                    Some((expr.to_owned(), spec.map(str::to_owned)))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_and_holes() {
        let segs = parse_interpolation("Count: {count()}!").unwrap();
        assert_eq!(segs[0], Segment::Text("Count: "));
        assert!(
            matches!(segs[1], Segment::Expr { expr: "count()", spec: None, span } if span == Span::new(7, 16))
        );
        assert_eq!(segs[2], Segment::Text("!"));
    }

    #[test]
    fn specs_and_paths() {
        assert_eq!(
            exprs("{exposure():.2}"),
            vec![("exposure()".into(), Some(".2".into()))]
        );
        assert_eq!(exprs("{a::b(1)}"), vec![("a::b(1)".into(), None)]);
        assert_eq!(exprs("{x:>8}"), vec![("x".into(), Some(">8".into()))]);
        assert_eq!(
            exprs("{(state.progress * 100.0) as u32}%"),
            vec![("(state.progress * 100.0) as u32".into(), None)]
        );
        assert_eq!(
            exprs("{if a { 1 } else { 2 }}"),
            vec![("if a { 1 } else { 2 }".into(), None)]
        );
        assert_eq!(exprs("{f(\"}\")}"), vec![("f(\"}\")".into(), None)]);
        assert_eq!(exprs("{g('}')}"), vec![("g('}')".into(), None)]);
        assert_eq!(
            exprs("{m.get(\"a:b\"):?}"),
            vec![("m.get(\"a:b\")".into(), Some("?".into()))]
        );
    }

    #[test]
    fn escapes() {
        let segs = parse_interpolation("{{a}} {b}").unwrap();
        assert_eq!(segs[0], Segment::Text("{"));
        assert_eq!(segs[1], Segment::Text("a"));
        assert_eq!(segs[2], Segment::Text("}"));
        assert_eq!(segs[3], Segment::Text(" "));
        assert!(matches!(segs[4], Segment::Expr { expr: "b", .. }));
    }

    #[test]
    fn errors_terminate_with_spans() {
        for bad in ["{", "}", "a {b", "{}", "{ }", "{(}", "{\"}", "{[)]}"] {
            let e = parse_interpolation(bad).unwrap_err();
            assert!(e.primary_span().is_some(), "{bad}");
            assert!(!e.help().is_empty(), "{bad}");
        }
    }
}
