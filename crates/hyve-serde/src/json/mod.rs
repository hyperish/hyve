//! Direct JSON codec (RFC 8259) used by the standard rail.
//!
//! `#[model]` generates [`Json`] implementations that write and read fields in
//! one pass, with `camelCase` field names (`hyperish/docs` `03-api-protocol.md`
//! §1). Rules shared by both directions:
//!
//! - Integers whose magnitude exceeds 2^53 are written as decimal strings;
//!   readers accept a number or such a string for any integer field.
//! - `NaN`, `Infinity` and `-Infinity` are written as those strings and read back.
//! - Missing `Option` fields and `null` both decode to `None`; `None` encodes as `null`.
//! - Unknown object keys are skipped (forward compatibility); duplicate keys are
//!   rejected; trailing input is rejected.
//! - Nesting depth and input size are bounded by [`JsonLimits`].

mod impls;
mod reader;
mod writer;

pub use reader::{JsonReader, Peek};
pub use writer::JsonWriter;

use crate::DecodeError;

/// Bounds applied while decoding untrusted JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonLimits {
    /// Maximum nesting depth of arrays and objects.
    pub max_depth: usize,
    /// Maximum input length in bytes.
    pub max_len: usize,
}

impl JsonLimits {
    /// Defaults: depth 128, 16 MiB. Transport adapters usually enforce a smaller
    /// body limit before decoding (`413 payload_too_large`).
    pub const DEFAULT: JsonLimits = JsonLimits {
        max_depth: 128,
        max_len: 16 * 1024 * 1024,
    };
}

impl Default for JsonLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A type with a JSON representation.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no JSON codec, so it cannot appear in a `#[model]` or `#[server]` signature",
    label = "not a wire type",
    note = "declare it with `#[model]`, or use a supported type (integers except `usize`/`isize`, floats, `bool`, `char`, `String`, `Ulid`, `Id<T>`, `Timestamp`, `Value`, `Option`, `Vec`, `Box`, arrays, `BTreeMap<String, _>`, `HashMap<String, _>`)"
)]
pub trait Json: Sized {
    /// Writes `self` as one JSON value.
    fn encode_json(&self, w: &mut JsonWriter);

    /// Reads one JSON value.
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError>;

    /// The value of an absent object field, or `None` if the field is
    /// required. `Option<T>` returns `Some(None)`: optional fields may be
    /// omitted (adding one is backward compatible).
    fn missing() -> Option<Self> {
        None
    }

    /// Writes the standard-rail success body for `self`: `{"data": <value>}`
    /// (`hyperish/docs` `03-api-protocol.md` §4). A list page overrides it
    /// with `{"data": [...], "page": {...}}`.
    fn encode_data(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("data");
        self.encode_json(w);
        w.end_object();
    }

    /// Reads a body written by [`encode_data`](Json::encode_data); other keys
    /// are skipped.
    fn decode_data(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let mut data = None;
        while let Some(key) = r.next_key()? {
            if key == "data" {
                if data.is_some() {
                    return Err(r.duplicate_field("data"));
                }
                data = Some(Self::decode_json(r)?);
            } else {
                r.skip_value()?;
            }
        }
        data.ok_or_else(|| r.missing_field("data"))
    }
}

/// Encodes the standard-rail success body for `value` ([`Json::encode_data`]).
pub fn to_json_data<T: Json>(value: &T) -> String {
    let mut w = JsonWriter::new();
    value.encode_data(&mut w);
    w.into_string()
}

/// Decodes a standard-rail success body ([`Json::decode_data`]) with the
/// default limits.
pub fn from_json_data<T: Json>(input: &str) -> Result<T, DecodeError> {
    let mut r = JsonReader::new(input, JsonLimits::DEFAULT)?;
    let v = T::decode_data(&mut r)?;
    r.finish()?;
    Ok(v)
}

/// Encodes `value` as a JSON string.
pub fn to_json<T: Json>(value: &T) -> String {
    let mut w = JsonWriter::new();
    value.encode_json(&mut w);
    w.into_string()
}

/// Decodes a complete JSON document with the default limits.
pub fn from_json<T: Json>(input: &str) -> Result<T, DecodeError> {
    from_json_with(input, JsonLimits::DEFAULT)
}

/// Decodes a complete JSON document with explicit limits.
pub fn from_json_with<T: Json>(input: &str, limits: JsonLimits) -> Result<T, DecodeError> {
    let mut r = JsonReader::new(input, limits)?;
    let v = T::decode_json(&mut r)?;
    r.finish()?;
    Ok(v)
}

/// Decodes JSON from bytes, validating UTF-8 first.
pub fn from_json_bytes<T: Json>(input: &[u8], limits: JsonLimits) -> Result<T, DecodeError> {
    let s = std::str::from_utf8(input)
        .map_err(|e| DecodeError::new("input is not valid UTF-8", "$", Some(e.valid_up_to())))?;
    from_json_with(s, limits)
}
