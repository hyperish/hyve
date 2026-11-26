//! Codec errors.

use std::fmt;

use hyve_core::HyveError;

/// A value could not be decoded (malformed input, wrong shape, limits exceeded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    message: String,
    path: String,
    offset: Option<usize>,
}

impl DecodeError {
    /// A decode error at `path` (JSON-path-like, `$` is the root).
    pub fn new(message: impl Into<String>, path: impl Into<String>, offset: Option<usize>) -> Self {
        DecodeError {
            message: message.into(),
            path: path.into(),
            offset,
        }
    }

    /// What is wrong.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Where in the value (`$.tags[2]`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Byte offset in the input, when known.
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// As a request rejection: `invalid_request` with `path` and `reason` details.
    pub fn into_request_error(self) -> HyveError {
        HyveError::invalid_request(format!("malformed request body: {}", self.message))
            .with_detail("path", self.path)
            .with_detail("reason", self.message)
    }

    /// As a failure to read a peer's response: `Infra` `decode_failed`.
    pub fn into_response_error(self) -> HyveError {
        HyveError::infra(
            "decode_failed",
            format!(
                "could not decode response at {}: {}",
                self.path, self.message
            ),
        )
        .with_public_code()
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.message, self.path)?;
        if let Some(o) = self.offset {
            write!(f, " (byte {o})")?;
        }
        Ok(())
    }
}

impl std::error::Error for DecodeError {}

/// A value could not be encoded (for example a `usize` above `u32::MAX` under
/// the 32-bit pointer-width format).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub(crate) String);

impl EncodeError {
    /// An encode error with `message` (for codecs outside this crate).
    pub fn new(message: impl Into<String>) -> Self {
        EncodeError(message.into())
    }

    /// What went wrong.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "encode failed: {}", self.0)
    }
}

impl std::error::Error for EncodeError {}

impl From<EncodeError> for HyveError {
    fn from(e: EncodeError) -> Self {
        HyveError::internal(e.to_string())
    }
}
