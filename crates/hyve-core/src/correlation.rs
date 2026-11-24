//! Correlation identifiers.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::{Ulid, platform};

const MAX_REQUEST_ID_LEN: usize = 128;

/// Correlation ID of one request, carried in logs and in every error response
/// (`hyperish/docs` `03-api-protocol.md` §4). Cheap to clone.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(Arc<str>);

impl RequestId {
    /// A fresh ID (ULID text).
    pub fn new() -> Self {
        RequestId(Arc::from(Ulid::new().to_string()))
    }

    /// Accepts an externally supplied ID: 1–128 visible ASCII characters.
    pub fn parse(s: &str) -> Result<Self, InvalidRequestId> {
        if s.is_empty() || s.len() > MAX_REQUEST_ID_LEN || !s.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(InvalidRequestId);
        }
        Ok(RequestId(Arc::from(s)))
    }

    /// The textual form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RequestId({})", self.0)
    }
}

impl FromStr for RequestId {
    type Err = InvalidRequestId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Error for a request ID that is empty, too long or contains non-visible characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRequestId;

impl fmt::Display for InvalidRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "request id must be 1-{MAX_REQUEST_ID_LEN} visible ASCII characters"
        )
    }
}

impl std::error::Error for InvalidRequestId {}

/// 128-bit trace identifier (lowercase hex, 32 characters), never all zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// A fresh random trace ID.
    pub fn new() -> Self {
        loop {
            let mut b = [0u8; 16];
            platform::fill_random(&mut b);
            if b != [0u8; 16] {
                return TraceId(b);
            }
        }
    }

    /// Raw bytes.
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraceId({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_validation() {
        assert!(RequestId::parse("abc-123").is_ok());
        assert!(RequestId::parse("").is_err());
        assert!(RequestId::parse("has space").is_err());
        assert!(RequestId::parse(&"x".repeat(129)).is_err());
        assert_eq!(RequestId::new().as_str().len(), 26);
    }

    #[test]
    fn trace_id_hex() {
        let t = TraceId::new();
        let s = t.to_string();
        assert_eq!(s.len(), 32);
        assert!(
            s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(TraceId::new(), t);
    }
}
