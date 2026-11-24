//! Strict pull reader for JSON (RFC 8259).

use std::borrow::Cow;
use std::fmt::Write as _;

use super::JsonLimits;
use crate::DecodeError;

/// Kind of the next value, without consuming it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peek {
    /// `null`
    Null,
    /// `true` / `false`
    Bool,
    /// A number.
    Number,
    /// A string.
    String,
    /// `[`
    Array,
    /// `{`
    Object,
}

enum Seg<'a> {
    Key(Cow<'a, str>),
    Index(usize),
}

struct Frame {
    first: bool,
    pushed: bool,
    index: usize,
}

/// Pull reader over one JSON document.
///
/// Generated decoders call [`begin_object`](Self::begin_object) then
/// [`next_key`](Self::next_key) until it returns `None`, or
/// [`begin_array`](Self::begin_array) then [`next_element`](Self::next_element)
/// until it returns `false`. Errors carry the path of the offending value.
pub struct JsonReader<'a> {
    src: &'a str,
    pos: usize,
    limits: JsonLimits,
    frames: Vec<Frame>,
    path: Vec<Seg<'a>>,
}

impl<'a> JsonReader<'a> {
    /// A reader over `src`; fails if `src` exceeds `limits.max_len`.
    pub fn new(src: &'a str, limits: JsonLimits) -> Result<Self, DecodeError> {
        if src.len() > limits.max_len {
            return Err(DecodeError::new(
                format!(
                    "input is {} bytes, more than the limit of {}",
                    src.len(),
                    limits.max_len
                ),
                "$",
                None,
            ));
        }
        Ok(JsonReader {
            src,
            pos: 0,
            limits,
            frames: Vec::new(),
            path: Vec::new(),
        })
    }

    fn bytes(&self) -> &'a [u8] {
        self.src.as_bytes()
    }

    /// The current path (`$`, `$.items[3].name`).
    pub fn path(&self) -> String {
        let mut s = String::from("$");
        for seg in &self.path {
            match seg {
                Seg::Key(k) => {
                    s.push('.');
                    s.push_str(k);
                }
                Seg::Index(i) => {
                    let _ = write!(s, "[{i}]");
                }
            }
        }
        s
    }

    /// An error at the current path and position.
    pub fn error(&self, message: impl Into<String>) -> DecodeError {
        DecodeError::new(message, self.path(), Some(self.pos))
    }

    /// Error for a required key that was not present in the current object.
    pub fn missing_field(&self, json_name: &str) -> DecodeError {
        DecodeError::new(
            format!("missing field `{json_name}`"),
            self.path_without_last(),
            Some(self.pos),
        )
    }

    /// Error for a key that appeared twice in the current object.
    pub fn duplicate_field(&self, json_name: &str) -> DecodeError {
        DecodeError::new(
            format!("duplicate field `{json_name}`"),
            self.path_without_last(),
            Some(self.pos),
        )
    }

    fn path_without_last(&self) -> String {
        // After an object ends its key segment is already popped, so the
        // current path is the object's own path.
        self.path()
    }

    fn ws(&mut self) {
        let b = self.bytes();
        while self.pos < b.len() && matches!(b[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn peek_byte(&mut self) -> Option<u8> {
        self.ws();
        self.bytes().get(self.pos).copied()
    }

    /// The kind of the next value.
    pub fn peek(&mut self) -> Result<Peek, DecodeError> {
        match self.peek_byte() {
            Some(b'n') => Ok(Peek::Null),
            Some(b't' | b'f') => Ok(Peek::Bool),
            Some(b'"') => Ok(Peek::String),
            Some(b'[') => Ok(Peek::Array),
            Some(b'{') => Ok(Peek::Object),
            Some(b'-' | b'0'..=b'9') => Ok(Peek::Number),
            Some(_) => Err(self.error("expected a JSON value")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn literal(&mut self, lit: &str) -> Result<(), DecodeError> {
        if self.src[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(self.error(format!("expected `{lit}`")))
        }
    }

    /// Reads `null`.
    pub fn read_null(&mut self) -> Result<(), DecodeError> {
        self.ws();
        self.literal("null")
    }

    /// Reads `true`/`false`.
    pub fn read_bool(&mut self) -> Result<bool, DecodeError> {
        match self.peek_byte() {
            Some(b't') => self.literal("true").map(|_| true),
            Some(b'f') => self.literal("false").map(|_| false),
            _ => Err(self.error("expected a boolean")),
        }
    }

    /// Reads a number and returns its text, validated against the JSON grammar.
    pub fn read_number(&mut self) -> Result<&'a str, DecodeError> {
        self.ws();
        let b = self.bytes();
        let start = self.pos;
        let mut i = start;
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        match b.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => {
                while matches!(b.get(i), Some(b'0'..=b'9')) {
                    i += 1;
                }
            }
            _ => return Err(self.error("expected a number")),
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            let d = i;
            while matches!(b.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
            if i == d {
                return Err(self.error("expected digits after `.`"));
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let d = i;
            while matches!(b.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
            if i == d {
                return Err(self.error("expected exponent digits"));
            }
        }
        self.pos = i;
        Ok(&self.src[start..i])
    }

    /// Reads a string, borrowing when it has no escapes.
    pub fn read_string(&mut self) -> Result<Cow<'a, str>, DecodeError> {
        if self.peek_byte() != Some(b'"') {
            return Err(self.error("expected a string"));
        }
        let b = self.bytes();
        let start = self.pos + 1;
        let mut i = start;
        // Fast path: no escapes.
        while i < b.len() {
            match b[i] {
                b'"' => {
                    self.pos = i + 1;
                    return Ok(Cow::Borrowed(&self.src[start..i]));
                }
                b'\\' => break,
                0x00..=0x1f => {
                    self.pos = i;
                    return Err(self.error("control character in string"));
                }
                _ => i += 1,
            }
        }
        let mut out = String::with_capacity(i - start + 16);
        out.push_str(&self.src[start..i]);
        loop {
            let Some(&c) = b.get(i) else {
                self.pos = i;
                return Err(self.error("unterminated string"));
            };
            match c {
                b'"' => {
                    self.pos = i + 1;
                    return Ok(Cow::Owned(out));
                }
                b'\\' => {
                    let e = b.get(i + 1).copied();
                    i += 2;
                    match e {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{8}'),
                        Some(b'f') => out.push('\u{c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let hi = self.hex4(i)?;
                            i += 4;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                if b.get(i) != Some(&b'\\') || b.get(i + 1) != Some(&b'u') {
                                    self.pos = i;
                                    return Err(self.error("unpaired surrogate in string escape"));
                                }
                                let lo = self.hex4(i + 2)?;
                                i += 6;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    self.pos = i;
                                    return Err(
                                        self.error("invalid low surrogate in string escape")
                                    );
                                }
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                            } else {
                                char::from_u32(hi)
                            };
                            match ch {
                                Some(ch) => out.push(ch),
                                None => {
                                    self.pos = i;
                                    return Err(self.error("unpaired surrogate in string escape"));
                                }
                            }
                        }
                        _ => {
                            self.pos = i - 2;
                            return Err(self.error("invalid escape in string"));
                        }
                    }
                }
                0x00..=0x1f => {
                    self.pos = i;
                    return Err(self.error("control character in string"));
                }
                _ => {
                    // Copy one UTF-8 scalar (the input is a &str, so boundaries are valid).
                    let ch_len = utf8_len(c);
                    out.push_str(&self.src[i..i + ch_len]);
                    i += ch_len;
                }
            }
        }
    }

    fn hex4(&mut self, at: usize) -> Result<u32, DecodeError> {
        let b = self.bytes();
        let digits = b
            .get(at..at + 4)
            .ok_or_else(|| self.error("truncated \\u escape"))?;
        let mut v = 0u32;
        for &d in digits {
            let n = (d as char)
                .to_digit(16)
                .ok_or_else(|| self.error("invalid \\u escape"))?;
            v = v * 16 + n;
        }
        Ok(v)
    }

    fn enter(&mut self) -> Result<(), DecodeError> {
        if self.frames.len() >= self.limits.max_depth {
            return Err(self.error(format!("nesting deeper than {}", self.limits.max_depth)));
        }
        Ok(())
    }

    /// Consumes `{`.
    pub fn begin_object(&mut self) -> Result<(), DecodeError> {
        if self.peek_byte() != Some(b'{') {
            return Err(self.error("expected an object"));
        }
        self.enter()?;
        self.pos += 1;
        self.frames.push(Frame {
            first: true,
            pushed: false,
            index: 0,
        });
        Ok(())
    }

    /// The next key of the current object, or `None` after consuming `}`.
    pub fn next_key(&mut self) -> Result<Option<Cow<'a, str>>, DecodeError> {
        let first = self.frames.last().map(|f| f.first).unwrap_or(true);
        self.pop_seg();
        match self.peek_byte() {
            Some(b'}') => {
                self.pos += 1;
                self.frames.pop();
                return Ok(None);
            }
            Some(b',') if !first => {
                self.pos += 1;
                if self.peek_byte() != Some(b'"') {
                    return Err(self.error("expected a key after `,`"));
                }
            }
            Some(b'"') if first => {}
            _ => {
                return Err(self.error(if first {
                    "expected a key or `}`"
                } else {
                    "expected `,` or `}`"
                }));
            }
        }
        let key = self.read_string()?;
        if self.peek_byte() != Some(b':') {
            return Err(self.error("expected `:` after key"));
        }
        self.pos += 1;
        if let Some(f) = self.frames.last_mut() {
            f.first = false;
            f.pushed = true;
        }
        self.path.push(Seg::Key(key.clone()));
        Ok(Some(key))
    }

    /// Consumes `[`.
    pub fn begin_array(&mut self) -> Result<(), DecodeError> {
        if self.peek_byte() != Some(b'[') {
            return Err(self.error("expected an array"));
        }
        self.enter()?;
        self.pos += 1;
        self.frames.push(Frame {
            first: true,
            pushed: false,
            index: 0,
        });
        Ok(())
    }

    /// Whether another element follows; consumes `]` when not.
    pub fn next_element(&mut self) -> Result<bool, DecodeError> {
        let first = self.frames.last().map(|f| f.first).unwrap_or(true);
        self.pop_seg();
        match self.peek_byte() {
            Some(b']') => {
                self.pos += 1;
                self.frames.pop();
                return Ok(false);
            }
            Some(b',') if !first => {
                self.pos += 1;
                if self.peek_byte() == Some(b']') {
                    return Err(self.error("trailing `,` in array"));
                }
            }
            Some(_) if first => {}
            _ => {
                return Err(self.error(if first {
                    "expected a value or `]`"
                } else {
                    "expected `,` or `]`"
                }));
            }
        }
        let idx = match self.frames.last_mut() {
            Some(f) => {
                let i = f.index;
                f.first = false;
                f.pushed = true;
                f.index += 1;
                i
            }
            None => 0,
        };
        self.path.push(Seg::Index(idx));
        Ok(true)
    }

    fn pop_seg(&mut self) {
        if let Some(f) = self.frames.last_mut() {
            if f.pushed {
                f.pushed = false;
                self.path.pop();
            }
        }
    }

    /// Skips one value of any kind (validating it).
    pub fn skip_value(&mut self) -> Result<(), DecodeError> {
        match self.peek()? {
            Peek::Null => self.read_null(),
            Peek::Bool => self.read_bool().map(drop),
            Peek::Number => self.read_number().map(drop),
            Peek::String => self.read_string().map(drop),
            Peek::Array => {
                self.begin_array()?;
                while self.next_element()? {
                    self.skip_value()?;
                }
                Ok(())
            }
            Peek::Object => {
                self.begin_object()?;
                while self.next_key()?.is_some() {
                    self.skip_value()?;
                }
                Ok(())
            }
        }
    }

    /// Ensures only whitespace remains.
    pub fn finish(&mut self) -> Result<(), DecodeError> {
        self.ws();
        if self.pos == self.src.len() {
            Ok(())
        } else {
            Err(self.error("unexpected trailing characters"))
        }
    }

    /// Reads an integer given as a number or a decimal string, as `i128`.
    pub fn read_int(&mut self) -> Result<i128, DecodeError> {
        let text: Cow<'a, str> = match self.peek()? {
            Peek::Number => Cow::Borrowed(self.read_number()?),
            Peek::String => self.read_string()?,
            _ => return Err(self.error("expected an integer")),
        };
        if !is_int_syntax(&text) {
            return Err(self.error(format!("expected an integer, found `{text}`")));
        }
        text.parse::<i128>()
            .map_err(|_| self.error(format!("integer `{text}` is out of range")))
    }

    /// Reads an unsigned integer given as a number or a decimal string.
    pub fn read_uint(&mut self) -> Result<u128, DecodeError> {
        let text: Cow<'a, str> = match self.peek()? {
            Peek::Number => Cow::Borrowed(self.read_number()?),
            Peek::String => self.read_string()?,
            _ => return Err(self.error("expected an integer")),
        };
        if !is_int_syntax(&text) || text.starts_with('-') {
            return Err(self.error(format!("expected a non-negative integer, found `{text}`")));
        }
        text.parse::<u128>()
            .map_err(|_| self.error(format!("integer `{text}` is out of range")))
    }

    /// Reads a float given as a number or one of `"NaN"`, `"Infinity"`, `"-Infinity"`.
    pub fn read_float(&mut self) -> Result<f64, DecodeError> {
        match self.peek()? {
            Peek::Number => {
                let t = self.read_number()?;
                t.parse::<f64>().map_err(|_| self.error("invalid number"))
            }
            Peek::String => match &*self.read_string()? {
                "NaN" => Ok(f64::NAN),
                "Infinity" => Ok(f64::INFINITY),
                "-Infinity" => Ok(f64::NEG_INFINITY),
                other => Err(self.error(format!("expected a number, found string `{other}`"))),
            },
            _ => Err(self.error("expected a number")),
        }
    }
}

fn is_int_syntax(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}
