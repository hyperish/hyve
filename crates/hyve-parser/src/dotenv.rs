//! `.env` files.
//!
//! One `KEY=VALUE` per line; blank lines and lines starting with `#` are ignored;
//! an optional `export ` prefix is allowed. Keys match `[A-Za-z_][A-Za-z0-9_]*`.
//! Values are unquoted (trailing whitespace and a ` #` comment are dropped),
//! single-quoted (taken literally) or double-quoted (`\n`, `\t`, `\r`, `\\`, `\"`
//! escapes). Values do not span lines and variables are not expanded. When a
//! key repeats, every occurrence is returned in order; the consumer decides
//! (`hyve-config` keeps the last).

use std::borrow::Cow;

use hyve_diagnostics::{Diagnostic, Span};
use winnow::combinator::opt;
use winnow::prelude::*;
use winnow::token::{one_of, take_while};

/// One assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotenvEntry<'a> {
    /// Variable name.
    pub key: &'a str,
    /// Value with quotes removed and escapes applied.
    pub value: Cow<'a, str>,
    /// Span of the whole assignment.
    pub span: Span,
    /// Span of the key.
    pub key_span: Span,
}

fn key<'a>(input: &mut &'a str) -> winnow::Result<&'a str> {
    (
        one_of(|c: char| c.is_ascii_alphabetic() || c == '_'),
        take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_'),
    )
        .take()
        .parse_next(input)
}

/// Parses a `.env` file.
pub fn parse_dotenv(text: &str) -> Result<Vec<DotenvEntry<'_>>, Diagnostic> {
    let mut out = Vec::new();
    let mut line_start = 0;
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\n', '\r']);
        let base = line_start;
        line_start += raw_line.len();
        let lead = line.len() - line.trim_start().len();
        let body = line.trim_start();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let mut rest = body;
        let _: winnow::Result<_> =
            opt(("export", take_while(1.., [' ', '\t']))).parse_next(&mut rest);
        let key_at = base + lead + (body.len() - rest.len());
        let k = key.parse_next(&mut rest).map_err(|_| {
            Diagnostic::error("invalid variable name in .env")
                .with_label(
                    Span::new(key_at, key_at + rest.len().min(1)),
                    "expected a name like `APP_PORT`",
                )
                .with_help(
                    "names start with a letter or `_` and contain only letters, digits and `_`",
                )
        })?;
        let key_span = Span::new(key_at, key_at + k.len());
        let rest_trim = rest.trim_start_matches([' ', '\t']);
        let Some(after_eq) = rest_trim.strip_prefix('=') else {
            return Err(
                Diagnostic::error(format!("missing `=` after `{k}` in .env"))
                    .with_label(key_span, "expected `=` after this name")
                    .with_help(format!("write `{k}=value`")),
            );
        };
        let value_src = after_eq.trim_start_matches([' ', '\t']);
        let value_at = base + (line.len() - value_src.len());
        let value = parse_value(value_src, value_at)?;
        out.push(DotenvEntry {
            key: k,
            value,
            span: Span::new(base + lead, base + line.len()),
            key_span,
        });
    }
    Ok(out)
}

fn parse_value(src: &str, at: usize) -> Result<Cow<'_, str>, Diagnostic> {
    let unclosed = |q: char| {
        Diagnostic::error(format!("unclosed {q} quote in .env value"))
            .with_label(Span::new(at, at + src.len()), "the value starts here")
            .with_help(format!("close the value with {q} on the same line"))
    };
    let trailing = |end: usize| -> Result<(), Diagnostic> {
        let tail = src[end..].trim_start_matches([' ', '\t']);
        if tail.is_empty() || tail.starts_with('#') {
            Ok(())
        } else {
            Err(
                Diagnostic::error("unexpected text after a quoted .env value")
                    .with_label(
                        Span::new(at + end, at + src.len()),
                        "after the closing quote",
                    )
                    .with_help("remove it or start a comment with `#`"),
            )
        }
    };
    if let Some(inner) = src.strip_prefix('\'') {
        let close = inner.find('\'').ok_or_else(|| unclosed('\''))?;
        trailing(close + 2)?;
        return Ok(Cow::Borrowed(&inner[..close]));
    }
    if let Some(inner) = src.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = inner.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    trailing(i + 2)?;
                    return Ok(Cow::Owned(value));
                }
                '\\' => match chars.next() {
                    Some((_, 'n')) => value.push('\n'),
                    Some((_, 't')) => value.push('\t'),
                    Some((_, 'r')) => value.push('\r'),
                    Some((_, '\\')) => value.push('\\'),
                    Some((_, '"')) => value.push('"'),
                    Some((j, other)) => {
                        let s = at + 1 + j - 1;
                        return Err(Diagnostic::error(format!(
                            "unknown escape `\\{other}` in .env value"
                        ))
                        .with_label(Span::new(s, s + 1 + other.len_utf8()), "unknown escape")
                        .with_help("supported escapes: \\n \\t \\r \\\\ \\\""));
                    }
                    None => return Err(unclosed('"')),
                },
                c => value.push(c),
            }
        }
        return Err(unclosed('"'));
    }
    let end = src
        .find(" #")
        .or_else(|| src.find("\t#"))
        .unwrap_or(src.len());
    Ok(Cow::Borrowed(src[..end].trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(text: &str) -> Vec<(String, String)> {
        parse_dotenv(text)
            .unwrap()
            .into_iter()
            .map(|e| (e.key.to_owned(), e.value.into_owned()))
            .collect()
    }

    #[test]
    fn parses_common_forms() {
        let src = "# comment\n\nAPP_PORT=8080\nexport APP_NAME = demo # trailing\nQ='a #b'\nD=\"x\\ny \\\"z\\\"\" # c\nEMPTY=\r\n";
        assert_eq!(
            kv(src),
            vec![
                ("APP_PORT".into(), "8080".into()),
                ("APP_NAME".into(), "demo".into()),
                ("Q".into(), "a #b".into()),
                ("D".into(), "x\ny \"z\"".into()),
                ("EMPTY".into(), "".into()),
            ]
        );
    }

    #[test]
    fn spans_point_at_keys() {
        let src = "A=1\n  B=2\n";
        let entries = parse_dotenv(src).unwrap();
        assert_eq!(&src[entries[1].key_span.range()], "B");
        assert_eq!(&src[entries[1].span.range()], "B=2");
    }

    #[test]
    fn errors() {
        for bad in [
            "1A=x",
            "A x",
            "A=\"open",
            "A='open",
            "A=\"\\q\"",
            "A='x' junk",
            "=x",
        ] {
            let e = parse_dotenv(bad).unwrap_err();
            assert!(e.primary_span().is_some(), "{bad}");
            assert!(!e.help().is_empty(), "{bad}");
        }
    }
}
