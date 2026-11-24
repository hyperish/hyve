//! The structured error value (`docs/02-architecture.md` §2.1).

use std::borrow::Cow;
use std::fmt;

use crate::{RequestId, Value};

/// Error classification shared by every layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Expected failure the caller can act on (validation, not found, conflict…).
    Domain,
    /// Temporary infrastructure failure (I/O, timeout, unavailable dependency).
    Infra,
    /// Defect or broken invariant. Its message never leaves the process.
    Internal,
}

impl ErrorKind {
    /// Stable lowercase name (`domain`, `infra`, `internal`).
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Domain => "domain",
            ErrorKind::Infra => "infra",
            ErrorKind::Internal => "internal",
        }
    }
}

/// Shorthand result type whose error is [`HyveError`].
pub type HyveResult<T> = Result<T, HyveError>;

/// Structured error: kind, stable `snake_case` code, human message, structured
/// details, retryability and correlation ID.
///
/// Callers branch on [`code`](Self::code), never on [`message`](Self::message).
/// Mapping to external status codes is an adapter concern, not this type's.
#[derive(Debug, Clone, PartialEq)]
pub struct HyveError {
    kind: ErrorKind,
    code: Cow<'static, str>,
    message: String,
    details: Value,
    retryable: bool,
    request_id: Option<RequestId>,
    public_code: bool,
}

impl HyveError {
    fn new(kind: ErrorKind, code: Cow<'static, str>, message: String) -> Self {
        debug_assert!(
            is_snake_case(&code),
            "error code must be snake_case: {code:?}"
        );
        Self {
            kind,
            code,
            message,
            details: Value::Null,
            retryable: false,
            request_id: None,
            public_code: kind == ErrorKind::Domain,
        }
    }

    /// A domain error with a caller-visible `code`.
    pub fn domain(code: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Domain, code.into(), message.into())
    }

    /// `<target>_not_found` domain error.
    pub fn not_found(target: &str) -> Self {
        Self::domain(format!("{target}_not_found"), format!("{target} not found"))
    }

    /// `invalid_request` domain error (validation failures put field errors in details).
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::domain("invalid_request", message)
    }

    /// An infrastructure error. Its code is replaced by `service_unavailable` at
    /// a trust boundary unless [`with_public_code`](Self::with_public_code) is set.
    pub fn infra(code: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Infra, code.into(), message.into())
    }

    /// An internal defect. Always `internal_error`; the message stays server-side.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::Internal,
            Cow::Borrowed("internal_error"),
            message.into(),
        )
    }

    /// Error kind.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Stable `snake_case` code to branch on.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human-readable message. Not meant to be parsed.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Structured details (`Value::Null` when there are none).
    pub fn details(&self) -> &Value {
        &self.details
    }

    /// Whether retrying the same operation may succeed.
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Correlation ID of the request that produced this error, if known.
    pub fn request_id(&self) -> Option<&RequestId> {
        self.request_id.as_ref()
    }

    /// Whether `code` may cross a trust boundary unchanged (always true for
    /// `Domain`, opt-in for `Infra`, never for `Internal`).
    pub fn is_code_public(&self) -> bool {
        self.public_code && self.kind != ErrorKind::Internal
    }

    /// Sets retryability.
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Adds one detail entry (details become a map).
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.details.insert(key, value);
        self
    }

    /// Replaces the details.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    /// Attaches the correlation ID.
    pub fn with_request_id(mut self, id: RequestId) -> Self {
        self.request_id = Some(id);
        self
    }

    /// Declares an `Infra` code safe to expose (no effect on `Internal`).
    pub fn with_public_code(mut self) -> Self {
        self.public_code = true;
        self
    }

    /// The form that may cross a trust boundary: `Infra` codes that are not
    /// public become `service_unavailable`, `Internal` errors lose their message
    /// and details. The same value is used on every rail
    /// (`orbic/docs/03-adapters-and-pipeline.md` §1).
    pub fn to_public(&self) -> HyveError {
        match self.kind {
            ErrorKind::Domain => self.clone(),
            ErrorKind::Infra if self.public_code => self.clone(),
            ErrorKind::Infra => HyveError {
                code: Cow::Borrowed("service_unavailable"),
                message: "the service is temporarily unavailable".into(),
                details: Value::Null,
                ..self.clone()
            },
            ErrorKind::Internal => HyveError {
                code: Cow::Borrowed("internal_error"),
                message: "internal error".into(),
                details: Value::Null,
                retryable: false,
                ..self.clone()
            },
        }
    }

    /// Rebuilds an error from its public parts (decoders on the receiving side).
    pub fn from_parts(
        kind: ErrorKind,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
        details: Value,
        retryable: bool,
        request_id: Option<RequestId>,
    ) -> Self {
        let mut e = Self::new(kind, code.into(), message.into());
        e.details = details;
        e.retryable = retryable;
        e.request_id = request_id;
        e.public_code = kind != ErrorKind::Internal;
        e
    }
}

impl fmt::Display for HyveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} error `{}`: {}",
            self.kind.as_str(),
            self.code,
            self.message
        )?;
        if let Some(id) = &self.request_id {
            write!(f, " (request {id})")?;
        }
        Ok(())
    }
}

impl std::error::Error for HyveError {}

pub(crate) fn is_snake_case(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
        && !s.ends_with('_')
        && !s.contains("__")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_builds_target_code() {
        let e = HyveError::not_found("project");
        assert_eq!(e.code(), "project_not_found");
        assert_eq!(e.kind(), ErrorKind::Domain);
        assert!(e.is_code_public());
    }

    #[test]
    fn public_form_hides_infra_and_internal() {
        let infra = HyveError::infra("db_pool_exhausted", "pool exhausted")
            .retryable(true)
            .with_detail("pool", "primary");
        let p = infra.to_public();
        assert_eq!(p.code(), "service_unavailable");
        assert!(p.details().is_empty());
        assert!(p.is_retryable());
        assert_eq!(p.kind(), ErrorKind::Infra);

        let public_infra = HyveError::infra("upstream_timeout", "x").with_public_code();
        assert_eq!(public_infra.to_public().code(), "upstream_timeout");

        let internal = HyveError::internal("index out of bounds in Foo::bar").with_detail("k", 1);
        let p = internal.to_public();
        assert_eq!(p.code(), "internal_error");
        assert!(!p.message().contains("Foo"));
        assert!(p.details().is_empty());
    }

    #[test]
    fn domain_keeps_everything() {
        let rid = RequestId::new();
        let e = HyveError::domain("version_conflict", "stale")
            .with_detail("expectedVersion", 3)
            .with_request_id(rid.clone());
        assert_eq!(e.to_public(), e);
        assert_eq!(e.request_id(), Some(&rid));
    }

    #[test]
    fn snake_case_check() {
        assert!(is_snake_case("rate_limited"));
        assert!(is_snake_case("a1"));
        assert!(!is_snake_case("RateLimited"));
        assert!(!is_snake_case("_x"));
        assert!(!is_snake_case("x_"));
        assert!(!is_snake_case("a__b"));
        assert!(!is_snake_case(""));
    }
}
