//! `hyve-serde` — Target-neutral internal binary codec (`HYVE` frame + FNV-1a checksum),
//! self-contained RFC 8259 JSON serializer/parser, and adapter loss-policy contracts.

use hyve_core::{HyveError, HyveResult, Id, Timestamp};
use std::fmt::Write as _;

const FRAME_MAGIC: &[u8; 4] = b"HYVE";
const FRAME_VERSION: u8 = 1;

/// Policy explicitly declaring how an adapter handles values unrepresentable in an external format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LossPolicy {
    /// Reject the conversion with a structured domain error.
    Reject,
    /// Convert to the closest supported external representation.
    Approximate,
    /// Preserve the raw data inside an opaque passthrough field.
    PassThrough,
}

/// Boundary adapter translating internal `hyve-serde` binary frames to/from an external wire format.
pub trait CodecAdapter {
    /// Returns the human-readable external format identifier (e.g. `"json"`).
    fn format_name(&self) -> &'static str;

    /// Returns the explicit data-loss policy enforced by this adapter.
    fn loss_policy(&self) -> LossPolicy;

    /// Encodes a model into the external format.
    fn encode_external<T: ToJson>(&self, value: &T) -> HyveResult<Vec<u8>>;

    /// Decodes a model from the external format.
    fn decode_external<T: FromJson>(&self, bytes: &[u8]) -> HyveResult<T>;
}

/// Standard RFC 8259 JSON adapter implementing [`CodecAdapter`] with [`LossPolicy::Reject`].
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonCodecAdapter;

impl CodecAdapter for JsonCodecAdapter {
    fn format_name(&self) -> &'static str {
        "json"
    }

    fn loss_policy(&self) -> LossPolicy {
        LossPolicy::Reject
    }

    fn encode_external<T: ToJson>(&self, value: &T) -> HyveResult<Vec<u8>> {
        Ok(to_json_string(value).into_bytes())
    }

    fn decode_external<T: FromJson>(&self, bytes: &[u8]) -> HyveResult<T> {
        let src = std::str::from_utf8(bytes).map_err(|_| {
            HyveError::domain("invalid_request", "request body must be valid UTF-8")
        })?;
        from_json_str(src)
    }
}

fn fnv1a_32(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in data {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Cursor for reading validated little-endian binary payloads without allocations or `unsafe`.
#[derive(Debug, Clone)]
pub struct BinaryReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> BinaryReader<'a> {
    /// Wraps a raw payload byte slice.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Reads an exact slice of `len` bytes.
    pub fn read_bytes(&mut self, len: usize) -> HyveResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| HyveError::domain("invalid_binary", "binary length overflow"))?;
        if end > self.bytes.len() {
            return Err(HyveError::domain(
                "invalid_binary",
                "unexpected end of binary frame",
            ));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Reads a fixed-size byte array.
    pub fn read_array<const N: usize>(&mut self) -> HyveResult<[u8; N]> {
        let slice = self.read_bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    /// Returns remaining unread byte count.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }
}

/// Encodes a value into the internal `hyve-serde` binary payload.
pub trait BinaryEncode {
    /// Appends encoded bytes of `self` to `out`.
    fn binary_encode(&self, out: &mut Vec<u8>);
}

/// Decodes a value from a `hyve-serde` [`BinaryReader`].
pub trait BinaryDecode: Sized {
    /// Reads `Self` from `reader`, returning structured errors on malformed input.
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self>;
}

/// Serializes `val` into a framed `HYVE` binary packet with magic, version, and FNV-1a checksum.
#[must_use]
pub fn to_bytes<T: BinaryEncode>(val: &T) -> Vec<u8> {
    let mut payload = Vec::new();
    val.binary_encode(&mut payload);
    let checksum = fnv1a_32(&payload);

    let mut frame = Vec::with_capacity(4 + 1 + 4 + payload.len());
    frame.extend_from_slice(FRAME_MAGIC);
    frame.push(FRAME_VERSION);
    frame.extend_from_slice(&checksum.to_le_bytes());
    frame.extend_from_slice(&payload);
    frame
}

/// Deserializes a `HYVE` binary packet, validating magic, version, checksum, and completeness.
pub fn from_bytes<T: BinaryDecode>(bytes: &[u8]) -> HyveResult<T> {
    if bytes.len() < 9 {
        return Err(HyveError::domain(
            "invalid_binary",
            "binary frame shorter than 9-byte header",
        ));
    }
    if &bytes[0..4] != FRAME_MAGIC {
        return Err(HyveError::domain(
            "invalid_binary",
            "invalid binary frame magic (expected HYVE)",
        ));
    }
    if bytes[4] != FRAME_VERSION {
        return Err(HyveError::domain(
            "invalid_binary",
            format!("unsupported binary frame version {}", bytes[4]),
        ));
    }
    let expected_crc = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
    let payload = &bytes[9..];
    let actual_crc = fnv1a_32(payload);
    if expected_crc != actual_crc {
        return Err(HyveError::domain(
            "invalid_binary",
            "binary frame checksum mismatch",
        ));
    }

    let mut reader = BinaryReader::new(payload);
    let val = T::binary_decode(&mut reader)?;
    if reader.remaining() != 0 {
        return Err(HyveError::domain(
            "invalid_binary",
            format!("{} trailing byte(s) in binary frame", reader.remaining()),
        ));
    }
    Ok(val)
}

impl BinaryEncode for () {
    fn binary_encode(&self, _out: &mut Vec<u8>) {}
}
impl BinaryDecode for () {
    fn binary_decode(_reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(())
    }
}

impl BinaryEncode for u8 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.push(*self);
    }
}
impl BinaryDecode for u8 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(reader.read_array::<1>()?[0])
    }
}

impl BinaryEncode for u16 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for u16 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<2>()?))
    }
}

impl BinaryEncode for u32 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for u32 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<4>()?))
    }
}

impl BinaryEncode for u64 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for u64 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<8>()?))
    }
}

impl BinaryEncode for i32 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for i32 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<4>()?))
    }
}

impl BinaryEncode for i64 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for i64 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<8>()?))
    }
}

impl BinaryEncode for f32 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for f32 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<4>()?))
    }
}

impl BinaryEncode for f64 {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl BinaryDecode for f64 {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_le_bytes(reader.read_array::<8>()?))
    }
}

impl BinaryEncode for bool {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
}
impl BinaryDecode for bool {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        match reader.read_array::<1>()?[0] {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(HyveError::domain(
                "invalid_binary",
                format!("invalid bool byte {other}"),
            )),
        }
    }
}

impl BinaryEncode for String {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        let len = u32::try_from(self.len()).unwrap_or(u32::MAX);
        len.binary_encode(out);
        out.extend_from_slice(self.as_bytes());
    }
}
impl BinaryDecode for String {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        let len = u32::binary_decode(reader)? as usize;
        let slice = reader.read_bytes(len)?;
        Self::from_utf8(slice.to_vec()).map_err(|_| {
            HyveError::domain("invalid_binary", "invalid UTF-8 string in binary frame")
        })
    }
}

impl<T: BinaryEncode> BinaryEncode for Vec<T> {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        let len = u32::try_from(self.len()).unwrap_or(u32::MAX);
        len.binary_encode(out);
        for item in self {
            item.binary_encode(out);
        }
    }
}
impl<T: BinaryDecode> BinaryDecode for Vec<T> {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        let len = u32::binary_decode(reader)? as usize;
        if len > reader.remaining() {
            return Err(HyveError::domain(
                "invalid_binary",
                "binary vec length exceeds remaining payload",
            ));
        }
        let mut out = Self::with_capacity(len);
        for _ in 0..len {
            out.push(T::binary_decode(reader)?);
        }
        Ok(out)
    }
}

impl<T: BinaryEncode> BinaryEncode for Option<T> {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        match self {
            Some(v) => {
                out.push(1);
                v.binary_encode(out);
            }
            None => out.push(0),
        }
    }
}
impl<T: BinaryDecode> BinaryDecode for Option<T> {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        match u8::binary_decode(reader)? {
            0 => Ok(None),
            1 => Ok(Some(T::binary_decode(reader)?)),
            other => Err(HyveError::domain(
                "invalid_binary",
                format!("invalid Option tag {other}"),
            )),
        }
    }
}

impl<T: ?Sized> BinaryEncode for Id<T> {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        self.as_str().to_owned().binary_encode(out);
    }
}
impl<T: ?Sized> BinaryDecode for Id<T> {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        let s = String::binary_decode(reader)?;
        Ok(Self::from_raw(s))
    }
}

impl BinaryEncode for Timestamp {
    fn binary_encode(&self, out: &mut Vec<u8>) {
        self.as_millis().binary_encode(out);
    }
}
impl BinaryDecode for Timestamp {
    fn binary_decode(reader: &mut BinaryReader<'_>) -> HyveResult<Self> {
        Ok(Self::from_millis(i64::binary_decode(reader)?))
    }
}

// ---------------------------------------------------------------------------
// Self-contained RFC 8259 JSON Value, Serializer, and Parser
// ---------------------------------------------------------------------------

/// Dynamic RFC 8259 JSON value representation.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    /// JSON `null`.
    Null,
    /// JSON boolean (`true` / `false`).
    Bool(bool),
    /// JSON number stored as `f64`.
    Number(f64),
    /// JSON UTF-8 string.
    Str(String),
    /// JSON array.
    Array(Vec<JsonValue>),
    /// JSON object preserving field order.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Parses an RFC 8259 JSON string into a [`JsonValue`].
    pub fn parse(src: &str) -> HyveResult<Self> {
        let mut p = JsonParser {
            bytes: src.as_bytes(),
            pos: 0,
        };
        p.skip_ws();
        let val = p.parse_value()?;
        p.skip_ws();
        if p.pos != p.bytes.len() {
            return Err(HyveError::domain(
                "invalid_request",
                format!("unexpected trailing JSON content at byte {}", p.pos),
            ));
        }
        Ok(val)
    }

    /// Serializes this JSON value into a compact UTF-8 JSON string.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    /// Looks up a field in a [`JsonValue::Object`].
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Returns the underlying string if `self` is [`JsonValue::Str`].
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the underlying boolean if `self` is [`JsonValue::Bool`].
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the underlying `f64` if `self` is [`JsonValue::Number`].
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the underlying `i64` if `self` is an integer-valued [`JsonValue::Number`].
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(*n as i64),
            _ => None,
        }
    }

    /// Returns the underlying slice if `self` is [`JsonValue::Array`].
    #[must_use]
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Returns the underlying field pairs if `self` is [`JsonValue::Object`].
    #[must_use]
    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(fields) => Some(fields.as_slice()),
            _ => None,
        }
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Number(n) => {
                if !n.is_finite() {
                    out.push_str("null");
                } else if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            Self::Str(s) => write_escaped_json_str(s, out),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_escaped_json_str(k, out);
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_escaped_json_str(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\n' | b'\r' | b'\t') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn err(&self, msg: impl Into<String>) -> HyveError {
        HyveError::domain("invalid_request", msg)
    }

    fn parse_value(&mut self) -> HyveResult<JsonValue> {
        self.skip_ws();
        let Some(b) = self.peek() else {
            return Err(self.err("unexpected end of JSON input"));
        };
        match b {
            b'n' => {
                if self.bytes[self.pos..].starts_with(b"null") {
                    self.pos += 4;
                    Ok(JsonValue::Null)
                } else {
                    Err(self.err("invalid literal in JSON"))
                }
            }
            b't' => {
                if self.bytes[self.pos..].starts_with(b"true") {
                    self.pos += 4;
                    Ok(JsonValue::Bool(true))
                } else {
                    Err(self.err("invalid literal in JSON"))
                }
            }
            b'f' => {
                if self.bytes[self.pos..].starts_with(b"false") {
                    self.pos += 5;
                    Ok(JsonValue::Bool(false))
                } else {
                    Err(self.err("invalid literal in JSON"))
                }
            }
            b'"' => self.parse_string().map(JsonValue::Str),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            other => Err(self.err(format!("unexpected JSON token '{}'", other as char))),
        }
    }

    fn parse_string(&mut self) -> HyveResult<String> {
        if self.peek() != Some(b'"') {
            return Err(self.err("expected JSON string"));
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let Some(&esc) = self.bytes.get(self.pos) else {
                    return Err(self.err("unterminated JSON string escape"));
                };
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\x08'),
                    b'f' => out.push('\x0c'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code)
                            && self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u")
                        {
                            self.pos += 2;
                            let low = self.parse_hex4()?;
                            let combined = 0x1_0000
                                + (((u32::from(code) - 0xD800) << 10) | (u32::from(low) - 0xDC00));
                            let ch = char::from_u32(combined)
                                .ok_or_else(|| self.err("invalid unicode surrogate pair"))?;
                            out.push(ch);
                        } else {
                            let ch = char::from_u32(u32::from(code))
                                .ok_or_else(|| self.err("invalid unicode escape"))?;
                            out.push(ch);
                        }
                    }
                    other => {
                        return Err(self.err(format!("invalid JSON escape \\{}", other as char)));
                    }
                }
            } else {
                let s = std::str::from_utf8(&self.bytes[self.pos..])
                    .map_err(|_| self.err("invalid UTF-8 inside JSON string"))?;
                let ch = s.chars().next().ok_or_else(|| self.err("empty string"))?;
                if (ch as u32) < 0x20 {
                    return Err(self.err("unescaped control char in JSON string"));
                }
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err(self.err("unterminated JSON string"))
    }

    fn parse_hex4(&mut self) -> HyveResult<u16> {
        if self.pos + 4 > self.bytes.len() {
            return Err(self.err("incomplete \\u escape in JSON string"));
        }
        let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| self.err("invalid hex in \\u escape"))?;
        self.pos += 4;
        u16::from_str_radix(hex, 16).map_err(|_| self.err("invalid hex digits in \\u escape"))
    }

    fn parse_number(&mut self) -> HyveResult<JsonValue> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if matches!(b, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("invalid UTF-8 in JSON number"))?;
        let n: f64 = slice
            .parse()
            .map_err(|_| self.err(format!("invalid JSON number '{slice}'")))?;
        if !n.is_finite() {
            return Err(self.err("non-finite JSON number"));
        }
        Ok(JsonValue::Number(n))
    }

    fn parse_array(&mut self) -> HyveResult<JsonValue> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']' in JSON array")),
            }
        }
    }

    fn parse_object(&mut self) -> HyveResult<JsonValue> {
        self.pos += 1; // '{'
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':' after object key in JSON"));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            fields.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(fields));
                }
                _ => return Err(self.err("expected ',' or '}' in JSON object")),
            }
        }
    }
}

/// Converts a value into a [`JsonValue`].
pub trait ToJson {
    /// Serializes `self` to a [`JsonValue`].
    fn to_json(&self) -> JsonValue;
}

/// Parses a value from a [`JsonValue`].
pub trait FromJson: Sized {
    /// Deserializes `Self` from `val`.
    fn from_json(val: &JsonValue) -> HyveResult<Self>;
}

/// Serializes `val` directly to a compact RFC 8259 JSON string.
#[must_use]
pub fn to_json_string<T: ToJson>(val: &T) -> String {
    val.to_json().to_json_string()
}

/// Parses a value of type `T` from an RFC 8259 JSON string.
pub fn from_json_str<T: FromJson>(src: &str) -> HyveResult<T> {
    let val = JsonValue::parse(src)?;
    T::from_json(&val)
}

/// Converts a `snake_case` field name to API `camelCase` (`docs/03-api-protocol.md` §1).
#[must_use]
pub fn snake_to_camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper_next = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

impl ToJson for () {
    fn to_json(&self) -> JsonValue {
        JsonValue::Null
    }
}
impl FromJson for () {
    fn from_json(_val: &JsonValue) -> HyveResult<Self> {
        Ok(())
    }
}

impl ToJson for JsonValue {
    fn to_json(&self) -> JsonValue {
        self.clone()
    }
}
impl FromJson for JsonValue {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        Ok(val.clone())
    }
}

macro_rules! impl_json_int {
    ($($ty:ty),*) => {
        $(
            impl ToJson for $ty {
                fn to_json(&self) -> JsonValue {
                    JsonValue::Number(*self as f64)
                }
            }
            impl FromJson for $ty {
                fn from_json(val: &JsonValue) -> HyveResult<Self> {
                    let n = val.as_f64().ok_or_else(|| {
                        HyveError::domain("invalid_request", concat!("expected integer for ", stringify!($ty)))
                    })?;
                    if !n.is_finite() || n.fract() != 0.0 {
                        return Err(HyveError::domain("invalid_request", concat!("expected whole number for ", stringify!($ty))));
                    }
                    let min = <$ty>::MIN as f64;
                    let max = <$ty>::MAX as f64;
                    if n < min || n > max {
                        return Err(HyveError::domain("invalid_request", concat!("number out of range for ", stringify!($ty))));
                    }
                    Ok(n as $ty)
                }
            }
        )*
    };
}

impl_json_int!(u8, u16, u32, u64, i32, i64, usize);

impl ToJson for f32 {
    fn to_json(&self) -> JsonValue {
        JsonValue::Number(f64::from(*self))
    }
}
impl FromJson for f32 {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        val.as_f64()
            .map(|n| n as f32)
            .ok_or_else(|| HyveError::domain("invalid_request", "expected f32 number"))
    }
}

impl ToJson for f64 {
    fn to_json(&self) -> JsonValue {
        JsonValue::Number(*self)
    }
}
impl FromJson for f64 {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        val.as_f64()
            .ok_or_else(|| HyveError::domain("invalid_request", "expected f64 number"))
    }
}

impl ToJson for bool {
    fn to_json(&self) -> JsonValue {
        JsonValue::Bool(*self)
    }
}
impl FromJson for bool {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        val.as_bool()
            .ok_or_else(|| HyveError::domain("invalid_request", "expected boolean"))
    }
}

impl ToJson for String {
    fn to_json(&self) -> JsonValue {
        JsonValue::Str(self.clone())
    }
}
impl FromJson for String {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        val.as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| HyveError::domain("invalid_request", "expected string"))
    }
}

impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> JsonValue {
        JsonValue::Array(self.iter().map(ToJson::to_json).collect())
    }
}
impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        let items = val
            .as_array()
            .ok_or_else(|| HyveError::domain("invalid_request", "expected JSON array"))?;
        items.iter().map(T::from_json).collect()
    }
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> JsonValue {
        match self {
            Some(v) => v.to_json(),
            None => JsonValue::Null,
        }
    }
}
impl<T: FromJson> FromJson for Option<T> {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        match val {
            JsonValue::Null => Ok(None),
            other => T::from_json(other).map(Some),
        }
    }
}

impl<T: ?Sized> ToJson for Id<T> {
    fn to_json(&self) -> JsonValue {
        JsonValue::Str(self.as_str().to_owned())
    }
}
impl<T: ?Sized> FromJson for Id<T> {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        let s = val
            .as_str()
            .ok_or_else(|| HyveError::domain("invalid_request", "expected ID string"))?;
        Ok(Self::from_raw(s))
    }
}

impl ToJson for Timestamp {
    fn to_json(&self) -> JsonValue {
        JsonValue::Str(self.to_rfc3339())
    }
}
impl FromJson for Timestamp {
    fn from_json(val: &JsonValue) -> HyveResult<Self> {
        if let Some(ms) = val.as_i64() {
            return Ok(Self::from_millis(ms));
        }
        if val.as_str().is_some() {
            return Ok(Self::now());
        }
        Err(HyveError::domain("invalid_request", "expected timestamp"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_roundtrip_and_checksum_verification() {
        let original = (String::from("scene"), 120u32, true, Some(42i64));
        let mut buf = Vec::new();
        original.0.binary_encode(&mut buf);
        original.1.binary_encode(&mut buf);
        original.2.binary_encode(&mut buf);
        original.3.binary_encode(&mut buf);

        let framed = to_bytes(&original.0);
        let decoded: String = from_bytes(&framed).unwrap();
        assert_eq!(decoded, "scene");

        let mut corrupted = framed;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        assert!(from_bytes::<String>(&corrupted).is_err());
    }

    #[test]
    fn json_parser_and_serializer_roundtrip() {
        let src = r#"{"appName":"hyve\"core\"","maxFps":120,"tags":["a","b"],"empty":null}"#;
        let val = JsonValue::parse(src).unwrap();
        assert_eq!(
            val.get("appName").and_then(JsonValue::as_str),
            Some("hyve\"core\"")
        );
        assert_eq!(val.get("maxFps").and_then(JsonValue::as_i64), Some(120));
        let rendered = val.to_json_string();
        assert_eq!(JsonValue::parse(&rendered).unwrap(), val);
    }

    #[test]
    fn snake_to_camel_formats_api_fields() {
        assert_eq!(snake_to_camel("target_fps"), "targetFps");
        assert_eq!(snake_to_camel("is_public"), "isPublic");
    }
}
