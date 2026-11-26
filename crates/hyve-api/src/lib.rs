//! `hyve-api` — contracts that cross module, process and network boundaries
//! (`docs/02-architecture.md` §2.4, `docs/03-adapters-and-pipeline.md` §3).
//!
//! - [`ServerFnDescriptor`] and [`ServerFn`]: what `#[server]` leaves behind in
//!   every build — identifier, binding metadata (method, path, argument names;
//!   interpreted only by `orbic`), request/response schemas and the schema
//!   fingerprint.
//! - [`ServerCall`]: what calling a `#[server]` function returns. Awaiting it
//!   finds the [`Transport`] provided in the current scope and makes a typed
//!   call; with none provided it fails with `transport_unavailable`.
//! - [`ServerHandler`]: the handler body, present only in `server`-feature
//!   builds and reachable only through `orbic`'s `.handle(...)` registration.
//! - [`Transport`]: the typed-call interface implemented by
//!   `serge-adapter-client` (remote) and `orbic` (in-process, `TestClient`).
//!   Rails, fallback and sending are the implementation's job; the mapping of
//!   a call to an HTTP request and of a response back to a reply is shared
//!   in [`http`] (pure functions, no I/O), and text parameters to and from
//!   models in [`params`].
//! - Shared request/response shapes: [`Path`], [`Query`], [`Page`],
//!   [`CursorParams`], and the wire form of errors ([`ErrorWire`],
//!   [`ErrorBody`]).
//! - Transport contract constants: [`MEDIA_TYPE_BINARY`], [`HEADER_FN`],
//!   [`HEADER_SCHEMA`] and the error [`codes`].

mod call;
mod descriptor;
mod error_wire;
mod extract;
pub mod http;
mod page;
pub mod params;
pub mod realtime;
mod transport;

pub use call::{ContextNeed, ServerCall, ServerFn, ServerHandler, StubFn, Wire};
pub use descriptor::{Method, ServerFnDescriptor, server_fn_fingerprint};
pub use error_wire::{ErrorBody, ErrorWire, WireKind};
pub use extract::{Path, Query};
pub use page::{CursorParams, Page};
pub use transport::{
    BoxFuture, Call, Rail, Reply, TestClient, Transport, TransportHandle, provide_transport,
};

/// Media type of fast-rail bodies (`docs/03-adapters-and-pipeline.md` §3.3).
pub const MEDIA_TYPE_BINARY: &str = "application/vnd.hyperish.rkyv";

/// Media type of standard-rail bodies.
pub const MEDIA_TYPE_JSON: &str = "application/json";

/// Header carrying the server function identifier on the fast rail.
pub const HEADER_FN: &str = "Hyperish-Fn";

/// Header carrying the schema fingerprint on the fast rail (16 lowercase hex
/// digits, see [`ServerFnDescriptor::fingerprint_hex`]).
pub const HEADER_SCHEMA: &str = "Hyperish-Schema";

/// Error codes defined by the call contract.
pub mod codes {
    /// No transport is provided in the current scope, or the peer is unreachable.
    pub const TRANSPORT_UNAVAILABLE: &str = "transport_unavailable";
    /// The call did not complete in time.
    pub const TIMEOUT: &str = "timeout";
    /// The response could not be read on either rail.
    pub const DECODE_FAILED: &str = "decode_failed";
    /// The caller cancelled the call (its `CancellationToken`) before it
    /// completed; the transport abandoned the request.
    pub const CANCELLED: &str = "cancelled";
    /// The fast-rail schema fingerprint differs from the server's (`415`).
    pub const SCHEMA_MISMATCH: &str = "schema_mismatch";
    /// The route does not accept the fast rail or does not know the function (`415`).
    pub const UNSUPPORTED_MEDIA_TYPE: &str = "unsupported_media_type";
}

/// Transport-level failures, as `Infra` errors with public codes.
pub mod errors {
    use hyve_core::HyveError;

    use crate::codes;

    /// No transport in scope, or the peer cannot be reached.
    pub fn transport_unavailable(message: impl Into<String>) -> HyveError {
        HyveError::infra(codes::TRANSPORT_UNAVAILABLE, message).with_public_code()
    }

    /// The call timed out.
    pub fn timeout(message: impl Into<String>) -> HyveError {
        HyveError::infra(codes::TIMEOUT, message).with_public_code()
    }

    /// The response could not be decoded.
    pub fn decode_failed(message: impl Into<String>) -> HyveError {
        HyveError::infra(codes::DECODE_FAILED, message).with_public_code()
    }

    /// The caller cancelled the call; not retryable (the caller is gone).
    pub fn cancelled(message: impl Into<String>) -> HyveError {
        HyveError::infra(codes::CANCELLED, message).with_public_code()
    }
}

#[cfg(test)]
mod tests;
