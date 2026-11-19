//! `hyve` — All-in-one core framework foundation for the Hyperish family.
//!
//! This initial pre-release establishes the foundational error classification
//! and cooperative cancellation contracts defined in `hyperish/docs`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// High-level error classification shared across Hyperish layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Expected domain or validation failure that callers can branch on.
    Domain,
    /// Infrastructure, I/O, timeout, or external service failure.
    Infra,
    /// Unexpected invariant violation or internal defect.
    Internal,
}

/// Structured error value carrying classification, stable code, retryability,
/// and optional correlation identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyveError {
    kind: ErrorKind,
    code: String,
    message: String,
    retryable: bool,
    request_id: Option<String>,
}

impl HyveError {
    /// Creates a structured domain error (non-retryable by default).
    pub fn domain(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Domain,
            code: code.into(),
            message: message.into(),
            retryable: false,
            request_id: None,
        }
    }

    /// Creates an infrastructure error with explicit retryability.
    pub fn infra(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind: ErrorKind::Infra,
            code: code.into(),
            message: message.into(),
            retryable,
            request_id: None,
        }
    }

    /// Creates an internal defect error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            code: "internal_error".to_owned(),
            message: message.into(),
            retryable: false,
            request_id: None,
        }
    }

    /// Attaches a correlation/request identifier for log and response tracing.
    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// Returns the error classification.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the stable `snake_case` machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the human-readable error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether the caller may retry the operation.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Returns the attached correlation identifier, if any.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

impl fmt::Display for HyveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for HyveError {}

/// Convenience result alias for `hyve` operations.
pub type HyveResult<T> = Result<T, HyveError>;

/// Cooperative, lock-free cancellation token propagated across async and sync boundaries.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a new uncancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Signals cancellation to all clones of this token.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns `true` if cancellation has been signalled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Commonly used types re-exported for ergonomic imports.
pub mod prelude {
    pub use crate::{CancellationToken, ErrorKind, HyveError, HyveResult};
}

#[cfg(test)]
mod tests {
    use super::prelude::*;

    #[test]
    fn error_preserves_kind_and_request_id() {
        let err = HyveError::domain("invalid_request", "name must not be empty")
            .with_request_id("req_01");
        assert_eq!(err.kind(), ErrorKind::Domain);
        assert_eq!(err.code(), "invalid_request");
        assert!(!err.is_retryable());
        assert_eq!(err.request_id(), Some("req_01"));
    }

    #[test]
    fn cancellation_token_propagates_across_clones() {
        let root = CancellationToken::new();
        let child = root.clone();
        assert!(!child.is_cancelled());
        root.cancel();
        assert!(child.is_cancelled());
    }
}
