//! The TOML subset `hyve.toml` uses (TOML 1.0, https://toml.io/en/v1.0.0).
//!
//! Supported: comments, `[table]` headers with dotted and quoted keys,
//! `key = value` with dotted keys, basic and literal strings, integers
//! (decimal, `0x`, `0o`, `0b`, `_` separators), floats (including `inf` and
//! `nan`), booleans, arrays (multi-line, trailing comma) and inline tables.
//!
//! Not supported in Phase 1, each reported with a diagnostic rather than
//! misread: multi-line strings, dates and times, arrays of tables (`[[x]]`).
//! Every key and value keeps its span for diagnostics.

use hyve_diagnostics::{Diagnostic, Span};

/// A value with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Where it is in the source.
    pub span: Span,
}

/// A TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    /// A string.
    String(String),
    /// A 64-bit integer.
    Integer(i64),
    /// A float.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// An array.
    Array(Vec<Spanned<TomlValue>>),
    /// A table (from a header or an inline table).
    Table(TomlTable),
}

/// A table: keys in source order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TomlTable {
    entries: Vec<(Spanned<String>, Spanned<TomlValue>)>,
    /// Defined with a `[header]` (as opposed to implicitly by a dotted key or
    /// a nested header); a second header for it is an error.
    explicit: bool,
}

impl TomlTable {
    /// Entries in source order.
    pub fn entries(&self) -> impl Iterator<Item = (&Spanned<String>, &Spanned<TomlValue>)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// The entry for `key`.
    pub fn get(&self, key: &str) -> Option<&Spanned<TomlValue>> {
        self.entries
            .iter()
            .find(|(k, _)| k.value == key)
            .map(|(_, v)| v)
    }

    /// The key's span, if present.
    pub fn key_span(&self, key: &str) -> Option<Span> {
        self.entries
            .iter()
            .find(|(k, _)| k.value == key)
            .map(|(k, _)| k.span)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut Spanned<TomlValue>> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k.value == key)
            .map(|(_, v)| v)
    }
}

/// Parses a TOML document into its root table.
pub fn parse_toml(src: &str) -> Result<TomlTable, Diagnostic> {
    let mut p = Parser {
        src,
        b: src.as_bytes(),
        pos: 0,
    };
    let mut root = TomlTable {
        explicit: true,
        ..TomlTable::default()
    };
    let mut current: Vec<Spanned<String>> = Vec::new();
    loop {
        p.skip_ws_comments_newlines()?;
        if p.eof() {
            return Ok(root);
        }
        if p.peek() == Some(b'[') {
            let start = p.pos;
            if p.b.get(p.pos + 1) == Some(&b'[') {
                return Err(p.unsupported(
                    start,
                    start + 2,
                    "arrays of tables (`[[...]]`)",
                    "use an inline array of tables: `items = [{ name = \"a\" }, { name = \"b\" }]`",
                ));
            }
            p.pos += 1;
            p.skip_ws();
            let keys = p.key_path()?;
            p.skip_ws();
            p.expect(b']', "expected `]` to close the table header")?;
            let header_span = Span::new(start, p.pos);
            p.end_of_line()?;
            open_table(&mut root, &keys, header_span)?;
            current = keys;
        } else {
            let keys = p.key_path()?;
            p.skip_ws();
            p.expect(b'=', "expected `=` after the key")?;
            p.skip_ws();
            let value = p.value()?;
            p.end_of_line()?;
            let table = table_at(&mut root, &current).expect("current table exists");
            insert(table, &keys, value)?;
        }
    }
}

/// Opens `[a.b.c]`, creating intermediate tables.
fn open_table(
    root: &mut TomlTable,
    keys: &[Spanned<String>],
    header: Span,
) -> Result<(), Diagnostic> {
    let mut t = root;
    for (i, k) in keys.iter().enumerate() {
        let last = i + 1 == keys.len();
        if t.get(&k.value).is_none() {
            t.entries.push((
                k.clone(),
                Spanned {
                    value: TomlValue::Table(TomlTable {
                        entries: Vec::new(),
                        explicit: last,
                    }),
                    span: header,
                },
            ));
        } else if last {
            let existing = t.get_mut(&k.value).expect("present");
            match &mut existing.value {
                TomlValue::Table(tt) if !tt.explicit => tt.explicit = true,
                TomlValue::Table(_) => {
                    return Err(Diagnostic::error(format!(
                        "table `{}` is defined twice",
                        dotted(keys)
                    ))
                    .with_label(header, "defined again here")
                    .with_label(existing.span, "first defined here")
                    .with_help("merge the two sections into one"));
                }
                // A value under the same key is reported below.
                _ => {}
            }
        }
        let next = t.get_mut(&k.value).expect("present");
        let span = next.span;
        t = match &mut next.value {
            TomlValue::Table(tt) => tt,
            _ => {
                return Err(
                    Diagnostic::error(format!("`{}` is a value, not a table", k.value))
                        .with_label(span, "defined as a value here")
                        .with_label(header, "used as a table here")
                        .with_help("rename one of them"),
                );
            }
        };
    }
    Ok(())
}

fn table_at<'t>(root: &'t mut TomlTable, keys: &[Spanned<String>]) -> Option<&'t mut TomlTable> {
    let mut t = root;
    for k in keys {
        t = match &mut t.get_mut(&k.value)?.value {
            TomlValue::Table(tt) => tt,
            _ => return None,
        };
    }
    Some(t)
}

/// Inserts `a.b.c = value`, creating implicit tables for dotted keys.
fn insert(
    table: &mut TomlTable,
    keys: &[Spanned<String>],
    value: Spanned<TomlValue>,
) -> Result<(), Diagnostic> {
    let (last, parents) = keys.split_last().expect("at least one key");
    let mut t = table;
    for k in parents {
        if t.get(&k.value).is_none() {
            t.entries.push((
                k.clone(),
                Spanned {
                    value: TomlValue::Table(TomlTable::default()),
                    span: k.span,
                },
            ));
        }
        let entry = t.get_mut(&k.value).expect("present");
        let span = entry.span;
        t = match &mut entry.value {
            TomlValue::Table(tt) => tt,
            _ => {
                return Err(
                    Diagnostic::error(format!("`{}` is a value, not a table", k.value))
                        .with_label(span, "defined as a value here")
                        .with_label(k.span, "used as a table here")
                        .with_help("rename one of them"),
                );
            }
        };
    }
    if let Some(first) = t.key_span(&last.value) {
        return Err(
            Diagnostic::error(format!("key `{}` is defined twice", dotted(keys)))
                .with_label(last.span, "defined again here")
                .with_label(first, "first defined here")
                .with_help("keep one of the two lines"),
        );
    }
    t.entries.push((last.clone(), value));
    Ok(())
}

fn dotted(keys: &[Spanned<String>]) -> String {
    keys.iter()
        .map(|k| k.value.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

struct Parser<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn eof(&self) -> bool {
        self.pos >= self.b.len()
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn err(&self, start: usize, end: usize, message: impl Into<String>, label: &str) -> Diagnostic {
        let end = end.max(start + 1).min(self.b.len().max(start + 1));
        Diagnostic::error(message).with_label(start..end, label)
    }

    fn unsupported(&self, start: usize, end: usize, what: &str, fix: &str) -> Diagnostic {
        self.err(
            start,
            end,
            format!("{what} are not supported in hyve.toml (Phase 1)"),
            "not supported",
        )
        .with_help(fix)
    }

    fn expect(&mut self, c: u8, message: &str) -> Result<(), Diagnostic> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(self.pos, self.pos + 1, message, "here"))
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn skip_comment(&mut self) -> Result<(), Diagnostic> {
        if self.peek() == Some(b'#') {
            while let Some(c) = self.peek() {
                if c == b'\n' {
                    break;
                }
                if c < 0x20 && c != b'\t' && c != b'\r' {
                    return Err(self.err(
                        self.pos,
                        self.pos + 1,
                        "control character in comment",
                        "here",
                    ));
                }
                self.pos += 1;
            }
        }
        Ok(())
    }

    fn skip_newline(&mut self) -> bool {
        match self.peek() {
            Some(b'\n') => {
                self.pos += 1;
                true
            }
            Some(b'\r') if self.b.get(self.pos + 1) == Some(&b'\n') => {
                self.pos += 2;
                true
            }
            _ => false,
        }
    }

    fn skip_ws_comments_newlines(&mut self) -> Result<(), Diagnostic> {
        loop {
            self.skip_ws();
            self.skip_comment()?;
            if !self.skip_newline() {
                return Ok(());
            }
        }
    }

    fn end_of_line(&mut self) -> Result<(), Diagnostic> {
        self.skip_ws();
        self.skip_comment()?;
        if self.eof() || self.skip_newline() {
            Ok(())
        } else {
            Err(self
                .err(
                    self.pos,
                    self.pos + 1,
                    "expected the end of the line",
                    "unexpected text",
                )
                .with_help("put each `key = value` on its own line"))
        }
    }

    fn key_path(&mut self) -> Result<Vec<Spanned<String>>, Diagnostic> {
        let mut keys = vec![self.key()?];
        loop {
            self.skip_ws();
            if self.peek() != Some(b'.') {
                return Ok(keys);
            }
            self.pos += 1;
            self.skip_ws();
            keys.push(self.key()?);
        }
    }

    fn key(&mut self) -> Result<Spanned<String>, Diagnostic> {
        let start = self.pos;
        let value = match self.peek() {
            Some(b'"') => self.basic_string()?,
            Some(b'\'') => self.literal_string()?,
            Some(c) if is_bare(c) => {
                while self.peek().is_some_and(is_bare) {
                    self.pos += 1;
                }
                self.src[start..self.pos].to_owned()
            }
            _ => {
                return Err(self
                    .err(self.pos, self.pos + 1, "expected a key", "here")
                    .with_help(
                        "keys are letters, digits, `_` and `-`, or quoted: `name = \"value\"`",
                    ));
            }
        };
        Ok(Spanned {
            value,
            span: Span::new(start, self.pos),
        })
    }

    fn value(&mut self) -> Result<Spanned<TomlValue>, Diagnostic> {
        let start = self.pos;
        let value = match self.peek() {
            Some(b'"') if self.src[self.pos..].starts_with("\"\"\"") => {
                return Err(self.unsupported(
                    start,
                    start + 3,
                    "multi-line strings",
                    "write the string on one line using `\\n` escapes",
                ));
            }
            Some(b'\'') if self.src[self.pos..].starts_with("'''") => {
                return Err(self.unsupported(
                    start,
                    start + 3,
                    "multi-line strings",
                    "write the string on one line using `\\n` escapes",
                ));
            }
            Some(b'"') => TomlValue::String(self.basic_string()?),
            Some(b'\'') => TomlValue::String(self.literal_string()?),
            Some(b'[') => self.array()?,
            Some(b'{') => self.inline_table()?,
            Some(b't') if self.src[self.pos..].starts_with("true") => {
                self.pos += 4;
                TomlValue::Bool(true)
            }
            Some(b'f') if self.src[self.pos..].starts_with("false") => {
                self.pos += 5;
                TomlValue::Bool(false)
            }
            Some(b'+' | b'-' | b'0'..=b'9' | b'i' | b'n') => self.number()?,
            _ => {
                return Err(self
                    .err(start, start + 1, "expected a value", "here")
                    .with_help("values are strings (\"…\"), numbers, `true`/`false`, arrays ([…]) or inline tables ({…})"));
            }
        };
        Ok(Spanned {
            value,
            span: Span::new(start, self.pos),
        })
    }

    fn basic_string(&mut self) -> Result<String, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self
                    .err(start, self.pos, "unterminated string", "starts here")
                    .with_help("close it with `\"`"));
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\n' | b'\r' => {
                    return Err(self
                        .err(start, self.pos, "unterminated string", "starts here")
                        .with_help("close it with `\"` on the same line"));
                }
                b'\\' => {
                    let esc = self.pos;
                    self.pos += 1;
                    let ch = match self.peek() {
                        Some(b'b') => '\u{8}',
                        Some(b't') => '\t',
                        Some(b'n') => '\n',
                        Some(b'f') => '\u{c}',
                        Some(b'r') => '\r',
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'u') => self.unicode(esc, 4)?,
                        Some(b'U') => self.unicode(esc, 8)?,
                        _ => {
                            return Err(self
                                .err(esc, self.pos + 1, "invalid escape in string", "here")
                                .with_help("valid escapes are \\b \\t \\n \\f \\r \\\" \\\\ \\uXXXX \\UXXXXXXXX; use a literal string ('…') to keep backslashes"));
                        }
                    };
                    if !matches!(self.b[esc + 1], b'u' | b'U') {
                        self.pos += 1;
                    }
                    out.push(ch);
                }
                c if c < 0x20 && c != b'\t' => {
                    return Err(self
                        .err(
                            self.pos,
                            self.pos + 1,
                            "control character in string",
                            "here",
                        )
                        .with_help("escape it (`\\n`, `\\t`, `\\uXXXX`)"));
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().expect("in bounds");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn unicode(&mut self, esc: usize, digits: usize) -> Result<char, Diagnostic> {
        let from = self.pos + 1;
        let hex = self
            .src
            .get(from..from + digits)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        let ch = hex
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .and_then(char::from_u32);
        match ch {
            Some(c) => {
                self.pos = from + digits;
                Ok(c)
            }
            None => Err(self
                .err(esc, from + digits, "invalid unicode escape", "here")
                .with_help("use \\uXXXX or \\UXXXXXXXX with a valid scalar value")),
        }
    }

    fn literal_string(&mut self) -> Result<String, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let body = self.pos;
        loop {
            match self.peek() {
                Some(b'\'') => {
                    let s = self.src[body..self.pos].to_owned();
                    self.pos += 1;
                    return Ok(s);
                }
                None | Some(b'\n' | b'\r') => {
                    return Err(self
                        .err(start, self.pos, "unterminated string", "starts here")
                        .with_help("close it with `'` on the same line"));
                }
                Some(_) => {
                    self.pos += self.src[self.pos..]
                        .chars()
                        .next()
                        .map_or(1, char::len_utf8)
                }
            }
        }
    }

    fn array(&mut self) -> Result<TomlValue, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws_comments_newlines()?;
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(TomlValue::Array(items));
            }
            if self.eof() {
                return Err(self
                    .err(start, start + 1, "unterminated array", "starts here")
                    .with_help("close it with `]`"));
            }
            items.push(self.value()?);
            self.skip_ws_comments_newlines()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {}
                _ => {
                    return Err(self.err(
                        self.pos,
                        self.pos + 1,
                        "expected `,` or `]` in array",
                        "here",
                    ));
                }
            }
        }
    }

    fn inline_table(&mut self) -> Result<TomlValue, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let mut table = TomlTable {
            entries: Vec::new(),
            explicit: true,
        };
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(TomlValue::Table(table));
        }
        loop {
            self.skip_ws();
            let keys = self.key_path()?;
            self.skip_ws();
            self.expect(b'=', "expected `=` after the key")?;
            self.skip_ws();
            let value = self.value()?;
            insert(&mut table, &keys, value)?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(TomlValue::Table(table));
                }
                _ => {
                    return Err(self
                        .err(start, self.pos, "unterminated inline table", "starts here")
                        .with_help("inline tables stay on one line: `{ a = 1, b = 2 }`"));
                }
            }
        }
    }

    fn number(&mut self) -> Result<TomlValue, Diagnostic> {
        let start = self.pos;
        while self.peek().is_some_and(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'_' | b'.' | b':')
        }) {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        let bad = |p: &Self, what: &str| {
            Err(p.err(start, p.pos, format!("invalid number `{text}`"), what)
                .with_help("write integers like `42`, `-7`, `1_000`, `0xff` and floats like `0.5`, `1e6`, `inf`"))
        };
        if text.contains(':')
            || (text.len() >= 5
                && text.as_bytes()[4] == b'-'
                && text[..4].bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(self.unsupported(
                start,
                self.pos,
                "dates and times",
                "write the time as a string: `created = \"2024-01-01T00:00:00Z\"`",
            ));
        }
        let (sign, body) = match text.as_bytes().first() {
            Some(b'+') => (1, &text[1..]),
            Some(b'-') => (-1, &text[1..]),
            _ => (1, text),
        };
        match body {
            "inf" => return Ok(TomlValue::Float(sign as f64 * f64::INFINITY)),
            "nan" => return Ok(TomlValue::Float(f64::NAN)),
            _ => {}
        }
        if !valid_underscores(body) {
            return bad(self, "`_` must sit between digits");
        }
        let clean: String = body.chars().filter(|&c| c != '_').collect();
        for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
            if let Some(digits) = clean.strip_prefix(prefix) {
                if sign != 1 || text.starts_with('+') {
                    return bad(self, "prefixed integers take no sign");
                }
                return match i64::from_str_radix(digits, radix) {
                    Ok(v) if !digits.is_empty() => Ok(TomlValue::Integer(v)),
                    _ => bad(self, "not a valid integer"),
                };
            }
        }
        if !clean
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
            || clean.is_empty()
        {
            return bad(self, "not a number");
        }
        let int_part = clean.split(['.', 'e', 'E']).next().unwrap_or("");
        if int_part.is_empty() || (int_part.len() > 1 && int_part.starts_with('0')) {
            return bad(self, "leading zeros are not allowed");
        }
        if clean.contains(['.', 'e', 'E']) {
            if clean.contains('.') {
                let frac = clean.split('.').nth(1).unwrap_or("");
                if !frac.starts_with(|c: char| c.is_ascii_digit()) {
                    return bad(self, "a `.` needs digits on both sides");
                }
            }
            return match clean.parse::<f64>() {
                Ok(v) => Ok(TomlValue::Float(sign as f64 * v)),
                Err(_) => bad(self, "not a valid float"),
            };
        }
        let signed = if sign < 0 { format!("-{clean}") } else { clean };
        match signed.parse::<i64>() {
            Ok(v) => Ok(TomlValue::Integer(v)),
            Err(_) => bad(self, "out of the 64-bit integer range"),
        }
    }
}

fn is_bare(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

fn valid_underscores(s: &str) -> bool {
    let b = s.as_bytes();
    b.iter().enumerate().all(|(i, &c)| {
        c != b'_'
            || (i > 0
                && i + 1 < b.len()
                && b[i - 1].is_ascii_alphanumeric()
                && b[i + 1].is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(src: &str) -> TomlValue {
        let t = parse_toml(&format!("x = {src}\n"))
            .unwrap_or_else(|d| panic!("{src}: {}", d.summary()));
        t.get("x").unwrap().value.clone()
    }

    #[test]
    fn values() {
        assert_eq!(v("\"a\\tb\\u00e9\""), TomlValue::String("a\tbé".into()));
        assert_eq!(v("'C:\\path'"), TomlValue::String("C:\\path".into()));
        assert_eq!(v("1_000"), TomlValue::Integer(1000));
        assert_eq!(v("-17"), TomlValue::Integer(-17));
        assert_eq!(v("0xff"), TomlValue::Integer(255));
        assert_eq!(v("0b101"), TomlValue::Integer(5));
        assert_eq!(v("6.5e-1"), TomlValue::Float(0.65));
        assert_eq!(v("-inf"), TomlValue::Float(f64::NEG_INFINITY));
        assert_eq!(v("true"), TomlValue::Bool(true));
        let TomlValue::Array(items) = v("[ 1, 2,\n  3, # comment\n]") else {
            panic!()
        };
        assert_eq!(items.len(), 3);
        let TomlValue::Table(t) = v("{ a = 1, b.c = \"x\" }") else {
            panic!()
        };
        assert!(matches!(t.get("b").unwrap().value, TomlValue::Table(_)));
    }

    #[test]
    fn tables_and_dotted_keys() {
        let src = "# config\n[app]\nname = \"demo\" # trailing\nmax_fps = 144\n\n[hyve]\nenv_prefix = \"DEMO\"\n[app.window]\nwidth = 800\nsize.height = 600\n";
        let root = parse_toml(src).unwrap();
        let TomlValue::Table(app) = &root.get("app").unwrap().value else {
            panic!()
        };
        assert_eq!(app.get("max_fps").unwrap().value, TomlValue::Integer(144));
        assert_eq!(&src[app.key_span("max_fps").unwrap().range()], "max_fps");
        let TomlValue::Table(window) = &app.get("window").unwrap().value else {
            panic!()
        };
        assert_eq!(window.len(), 2);
        let TomlValue::Table(hyve) = &root.get("hyve").unwrap().value else {
            panic!()
        };
        assert_eq!(
            hyve.get("env_prefix").unwrap().value,
            TomlValue::String("DEMO".into())
        );
    }

    #[test]
    fn errors_point_at_the_problem() {
        for (src, needle) in [
            ("a = 1\na = 2\n", "defined twice"),
            ("[a]\n[a]\n", "defined twice"),
            ("[[a]]\n", "arrays of tables"),
            ("a = \"\"\"x\"\"\"\n", "multi-line strings"),
            ("a = 2024-01-01\n", "dates and times"),
            ("a = 01\n", "invalid number"),
            ("a = 1__0\n", "invalid number"),
            ("a = \"open\n", "unterminated string"),
            ("a = 1 b = 2\n", "end of the line"),
            ("a = [1 2]\n", "expected `,` or `]`"),
            ("= 1\n", "expected a key"),
            ("a = \"\\q\"\n", "invalid escape"),
            ("a = 1\n[a]\n", "is a value, not a table"),
        ] {
            let e = parse_toml(src).expect_err(src);
            assert!(e.message().contains(needle), "{src:?}: {}", e.summary());
            assert!(e.primary_span().is_some(), "{src:?} has a span");
        }
    }
}
