//! Errors on the wire.
//!
//! - Fast rail: the archived [`ErrorWire`] is the error response body.
//! - Standard rail: [`ErrorBody`] is the JSON object under `error` in the
//!   envelope (`hyperish/docs` `03-api-protocol.md` §4); kind and
//!   retryability travel as the status code and `Retry-After`, which the
//!   transport maps (`docs/03-adapters-and-pipeline.md` §3.4).
//!
//! Servers convert errors to their public form before choosing a rail, so
//! both carry the same content.

use hyve_core::{ErrorKind, HyveError, RequestId, Value};
use hyve_serde::__private::rkyv;
use hyve_serde::json::{Json, JsonReader, JsonWriter};
use hyve_serde::{DecodeError, Fingerprint, fnv1a, mix};

/// [`ErrorKind`] as archived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
#[rkyv(derive(Debug, PartialEq))]
pub enum WireKind {
    /// [`ErrorKind::Domain`]
    Domain,
    /// [`ErrorKind::Infra`]
    Infra,
    /// [`ErrorKind::Internal`]
    Internal,
}

impl From<ErrorKind> for WireKind {
    fn from(k: ErrorKind) -> Self {
        match k {
            ErrorKind::Domain => WireKind::Domain,
            ErrorKind::Infra => WireKind::Infra,
            ErrorKind::Internal => WireKind::Internal,
        }
    }
}

impl From<WireKind> for ErrorKind {
    fn from(k: WireKind) -> Self {
        match k {
            WireKind::Domain => ErrorKind::Domain,
            WireKind::Infra => ErrorKind::Infra,
            WireKind::Internal => ErrorKind::Internal,
        }
    }
}

/// Every field of [`HyveError`] that crosses a boundary (fast-rail body).
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
pub struct ErrorWire {
    /// Error kind.
    pub kind: WireKind,
    /// Stable `snake_case` code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Structured details.
    pub details: Value,
    /// Whether retrying may succeed.
    pub retryable: bool,
    /// Correlation ID.
    pub request_id: Option<String>,
}

impl Fingerprint for ErrorWire {
    const FINGERPRINT: u64 = mix(
        fnv1a(b"ErrorWire{kind:Domain|Infra|Internal,code,message,details,retryable,requestId}"),
        mix(
            <String as Fingerprint>::FINGERPRINT,
            mix(
                <Value as Fingerprint>::FINGERPRINT,
                <Option<String> as Fingerprint>::FINGERPRINT,
            ),
        ),
    );
}

impl From<&HyveError> for ErrorWire {
    fn from(e: &HyveError) -> Self {
        ErrorWire {
            kind: e.kind().into(),
            code: e.code().to_owned(),
            message: e.message().to_owned(),
            details: e.details().clone(),
            retryable: e.is_retryable(),
            request_id: e.request_id().map(|r| r.as_str().to_owned()),
        }
    }
}

impl From<ErrorWire> for HyveError {
    fn from(w: ErrorWire) -> Self {
        HyveError::from_parts(
            w.kind.into(),
            w.code,
            w.message,
            w.details,
            w.retryable,
            w.request_id.and_then(|r| RequestId::parse(&r).ok()),
        )
    }
}

/// The JSON object under `error` in a standard-rail failure envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBody {
    /// Stable `snake_case` code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Structured details (`{}` when there are none).
    pub details: Value,
    /// Correlation ID.
    pub request_id: Option<String>,
}

impl ErrorBody {
    /// The body of `e` (already in public form).
    pub fn from_error(e: &HyveError) -> Self {
        ErrorBody {
            code: e.code().to_owned(),
            message: e.message().to_owned(),
            details: e.details().clone(),
            request_id: e.request_id().map(|r| r.as_str().to_owned()),
        }
    }

    /// Restores the error, with the kind and retryability the transport
    /// derived from the response.
    pub fn into_error(self, kind: ErrorKind, retryable: bool) -> HyveError {
        let details = match self.details {
            Value::Map(m) if m.is_empty() => Value::Null,
            other => other,
        };
        HyveError::from_parts(
            kind,
            self.code,
            self.message,
            details,
            retryable,
            self.request_id.and_then(|r| RequestId::parse(&r).ok()),
        )
    }
}

impl Json for ErrorBody {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("code");
        w.string(&self.code);
        w.key("message");
        w.string(&self.message);
        w.key("details");
        match &self.details {
            Value::Null => {
                w.begin_object();
                w.end_object();
            }
            d => d.encode_json(w),
        }
        if let Some(id) = &self.request_id {
            w.key("requestId");
            w.string(id);
        }
        w.end_object();
    }

    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let (mut code, mut message, mut details, mut request_id) = (None, None, None, None);
        while let Some(key) = r.next_key()? {
            let slot_taken = match &*key {
                "code" => code.replace(String::decode_json(r)?).is_some(),
                "message" => message.replace(String::decode_json(r)?).is_some(),
                "details" => details.replace(Value::decode_json(r)?).is_some(),
                "requestId" => request_id
                    .replace(Option::<String>::decode_json(r)?)
                    .is_some(),
                _ => {
                    r.skip_value()?;
                    false
                }
            };
            if slot_taken {
                return Err(r.duplicate_field(&key));
            }
        }
        Ok(ErrorBody {
            code: code.ok_or_else(|| r.missing_field("code"))?,
            message: message.unwrap_or_default(),
            details: details.unwrap_or(Value::Null),
            request_id: request_id.flatten(),
        })
    }
}
