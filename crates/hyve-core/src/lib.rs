//! `hyve-core` — foundational, runtime-neutral contracts for the Hyperish family.
//!
//! Everything here is shared by every other layer (`orbic`, `serge`, products):
//!
//! - [`HyveError`] / [`HyveResult`]: the structured error value (kind, stable `code`,
//!   `message`, `details`, retryability, correlation ID).
//! - [`Value`]: protocol-neutral structured data used for error details.
//! - [`Ulid`] / [`Id`]: sortable identifiers whose randomness comes from a CSPRNG.
//!   IDs identify things; they are never credentials.
//! - [`Timestamp`]: UTC instants with RFC 3339 text form.
//! - [`RequestId`] / [`TraceId`]: correlation identifiers.
//! - [`CancellationToken`]: hierarchical cooperative cancellation.
//! - [`Scope`] / [`Ctx`]: scoped dependency provision (app → component → request).
//! - [`SharedRingBuf`]: an overwriting ring of POD samples read as tear-free snapshots.
//!
//! This crate never depends on an async runtime, HTTP, DOM or GPU types
//! (`docs/02-architecture.md` §2.5).

mod cancel;
mod correlation;
mod error;
mod id;
mod platform;
mod ring;
mod scope;
mod time;
mod value;

pub use cancel::{CancellationToken, Cancelled};
pub use correlation::{InvalidRequestId, RequestId, TraceId};
pub use error::{ErrorKind, HyveError, HyveResult};
pub use id::{Id, ParseUlidError, Ulid};
pub use ring::SharedRingBuf;
pub use scope::{Ctx, Scope, Scoped};
pub use time::{ParseTimestampError, Timestamp};
pub use value::Value;

/// Re-export used by `SharedRingBuf` element types (`#[derive(Pod, Zeroable)]`).
pub use bytemuck;
