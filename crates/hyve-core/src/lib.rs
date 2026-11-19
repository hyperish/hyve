//! `hyve-core` — Target-neutral foundational contracts for the Hyperish family.
//!
//! Provides structured error classification, sortable ULIDs and typed IDs,
//! UTC timestamps, correlation IDs, hierarchical cooperative cancellation,
//! immutable `Copy` context handles, and tearing-free `SharedRingBuf`.

use std::collections::VecDeque;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

static LAST_ULID_MILLIS: AtomicU64 = AtomicU64::new(0);
static ULID_SEQ: AtomicU64 = AtomicU64::new(1);

/// High-level error classification shared across all Hyperish layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Expected domain or validation failure that callers can branch on.
    Domain,
    /// Infrastructure, I/O, timeout, or external service failure.
    Infra,
    /// Unexpected invariant violation or internal defect.
    Internal,
}

/// Structured error value carrying classification, stable `snake_case` code,
/// retryability, and optional correlation identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyveError {
    kind: ErrorKind,
    code: String,
    message: String,
    retryable: bool,
    request_id: Option<String>,
    retry_after_secs: Option<u64>,
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
            retry_after_secs: None,
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
            retry_after_secs: None,
        }
    }

    /// Creates an internal defect error (`internal_error`).
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            code: "internal_error".to_owned(),
            message: message.into(),
            retryable: false,
            request_id: None,
            retry_after_secs: None,
        }
    }

    /// Creates a domain `not_found` error (`<target>_not_found`).
    pub fn not_found(target: &str, message: impl Into<String>) -> Self {
        Self::domain(format!("{target}_not_found"), message)
    }

    /// Creates an `unauthenticated` domain error.
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::domain("unauthenticated", message)
    }

    /// Creates a `forbidden` domain error.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::domain("forbidden", message)
    }

    /// Creates a `409` conflict domain error (`version_conflict`, `idempotency_key_mismatch`, etc.).
    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::domain(code, message)
    }

    /// Creates a `rate_limited` error with a mandatory `Retry-After` duration in seconds.
    pub fn rate_limited(message: impl Into<String>, retry_after_secs: u64) -> Self {
        let mut err = Self::infra("rate_limited", message, true);
        err.retry_after_secs = Some(retry_after_secs);
        err
    }

    /// Attaches a correlation/request identifier for log and response tracing.
    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// Attaches a `Retry-After` interval in seconds.
    #[must_use]
    pub fn with_retry_after(mut self, secs: u64) -> Self {
        self.retry_after_secs = Some(secs);
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

    /// Returns the `Retry-After` hint in seconds, if present.
    #[must_use]
    pub fn retry_after_secs(&self) -> Option<u64> {
        self.retry_after_secs
    }

    /// Maps structured error codes/kinds onto HTTP status codes defined in `docs/03-api-protocol.md` §4.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self.code.as_str() {
            "invalid_request" => 400,
            "unauthenticated" => 401,
            "forbidden" => 403,
            "version_conflict"
            | "idempotency_key_mismatch"
            | "request_in_progress"
            | "conflict" => 409,
            "rate_limited" => 429,
            "internal_error" => 500,
            c if c == "not_found" || c.ends_with("_not_found") => 404,
            _ => match self.kind {
                ErrorKind::Domain => 400,
                ErrorKind::Infra | ErrorKind::Internal => 500,
            },
        }
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

/// Time-sortable 26-character Crockford Base32 universally unique lexicographical identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ulid([u8; 26]);

impl Ulid {
    /// Generates a monotonically increasing sortable ULID based on UTC milliseconds and sequence bits.
    #[must_use]
    pub fn generate() -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut prev = LAST_ULID_MILLIS.load(Ordering::Relaxed);
        let ts_ms = loop {
            let next = now_ms.max(prev);
            match LAST_ULID_MILLIS.compare_exchange_weak(
                prev,
                next,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break next,
                Err(actual) => prev = actual,
            }
        };

        let seq = ULID_SEQ.fetch_add(1, Ordering::Relaxed);
        // Mix sequence and timestamp into 80 randomness bits while keeping 48 timestamp bits monotonic.
        let rand_val: u128 = ((seq as u128).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            ^ ((seq as u128) << 40)
            ^ (ts_ms as u128);
        let value: u128 = ((ts_ms as u128) << 80) | (rand_val & ((1u128 << 80) - 1));

        let mut buf = [b'0'; 26];
        let mut v = value;
        for idx in (0..26).rev() {
            buf[idx] = CROCKFORD_ALPHABET[(v & 0x1f) as usize];
            v >>= 5;
        }
        Self(buf)
    }

    /// Parses and validates a 26-character Crockford Base32 ULID string.
    pub fn parse(raw: &str) -> HyveResult<Self> {
        let bytes = raw.as_bytes();
        if bytes.len() != 26 {
            return Err(HyveError::domain(
                "invalid_ulid",
                format!("ULID must be 26 characters, got {}", bytes.len()),
            ));
        }
        let mut out = [0u8; 26];
        for (idx, &b) in bytes.iter().enumerate() {
            let upper = b.to_ascii_uppercase();
            if !CROCKFORD_ALPHABET.contains(&upper) {
                return Err(HyveError::domain(
                    "invalid_ulid",
                    format!(
                        "invalid Crockford Base32 character '{}' at index {idx}",
                        b as char
                    ),
                ));
            }
            out[idx] = upper;
        }
        Ok(Self(out))
    }

    /// Returns the ULID as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl Default for Ulid {
    fn default() -> Self {
        Self::generate()
    }
}

impl FromStr for Ulid {
    type Err = HyveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Debug for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ulid({})", self.as_str())
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy)]
enum IdRepr {
    Inline { len: u8, buf: [u8; 64] },
    Static(&'static str),
}

/// Opaque, phantom-typed identifier shared across `hyve`, `orbic`, and `serge`.
pub struct Id<T: ?Sized> {
    repr: IdRepr,
    _marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Id<T> {
    /// Allocates a new opaque ULID-backed identifier.
    #[must_use]
    pub fn new() -> Self {
        Self::from_raw(Ulid::generate().as_str())
    }

    /// Wraps an existing string identifier without leaking fixed ULID/short IDs.
    #[must_use]
    pub fn from_raw(raw: impl AsRef<str>) -> Self {
        let s = raw.as_ref();
        let bytes = s.as_bytes();
        let repr = if bytes.len() <= 64 {
            let mut buf = [0u8; 64];
            buf[..bytes.len()].copy_from_slice(bytes);
            IdRepr::Inline {
                len: bytes.len() as u8,
                buf,
            }
        } else {
            IdRepr::Static(Box::leak(s.to_owned().into_boxed_str()))
        };
        Self {
            repr,
            _marker: PhantomData,
        }
    }

    /// Returns the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.repr {
            IdRepr::Inline { len, buf } => std::str::from_utf8(&buf[..*len as usize]).unwrap_or(""),
            IdRepr::Static(s) => s,
        }
    }
}

impl<T: ?Sized> Default for Id<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for Id<T> {}

impl<T: ?Sized> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl<T: ?Sized> Eq for Id<T> {}

impl<T: ?Sized> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl<T: ?Sized> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl<T: ?Sized> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.as_str())
    }
}

impl<T: ?Sized> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// UTC timestamp stored as epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Captures the current UTC timestamp.
    #[must_use]
    pub fn now() -> Self {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self(ms)
    }

    /// Constructs a timestamp from UTC epoch milliseconds.
    #[must_use]
    pub const fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    /// Returns UTC epoch milliseconds.
    #[must_use]
    pub const fn as_millis(&self) -> i64 {
        self.0
    }

    /// Formats the timestamp as an RFC 3339 UTC string (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
    #[must_use]
    pub fn to_rfc3339(&self) -> String {
        let secs = self.0.div_euclid(1_000);
        let millis = self.0.rem_euclid(1_000);
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400);
        let hour = sod / 3_600;
        let min = (sod % 3_600) / 60;
        let sec = sod % 60;
        let (year, month, day) = civil_from_days(days);
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{millis:03}Z")
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::now()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    // Howard Hinnant's Euclidean civil-from-days algorithm in safe integer arithmetic.
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (year, m as u32, d as u32)
}

/// Correlation identifier for a single incoming request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(String);

impl RequestId {
    /// Generates a fresh `req_<ulid>` correlation ID.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("req_{}", Ulid::generate()))
    }

    /// Wraps a provided request ID string.
    #[must_use]
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Returns the request ID string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::generate()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Distributed trace identifier propagated across client, server, DB, and jobs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TraceId(String);

impl TraceId {
    /// Generates a fresh `trc_<ulid>` trace ID.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("trc_{}", Ulid::generate()))
    }

    /// Wraps a provided trace ID string.
    #[must_use]
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Returns the trace ID string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::generate()
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Cooperative, hierarchical cancellation token propagated across sync, async, and RPC boundaries.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    parent: Option<Box<CancellationToken>>,
}

impl CancellationToken {
    /// Creates a new root uncancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a child token linked to this token; cancelling any ancestor cancels all descendants.
    #[must_use]
    pub fn child_token(&self) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            parent: Some(Box::new(self.clone())),
        }
    }

    /// Signals cancellation for this token and any cloned handles or child tokens derived from it.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns `true` if this token or any ancestor token has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        if self.cancelled.load(Ordering::Acquire) {
            return true;
        }
        self.parent.as_deref().is_some_and(Self::is_cancelled)
    }

    /// Returns `Err(HyveError)` if cancelled, allowing early bailout at cancellation checkpoints.
    pub fn check(&self) -> HyveResult<()> {
        if self.is_cancelled() {
            Err(HyveError::infra("cancelled", "operation cancelled", false))
        } else {
            Ok(())
        }
    }
}

/// Copyable context extractor handle referencing shared state provided by `provide(...)`.
pub struct Ctx<T: ?Sized + 'static> {
    inner: &'static T,
}

impl<T: 'static> Ctx<T> {
    /// Wraps a context value into an immutable, `Copy` handle.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            inner: Box::leak(Box::new(value)),
        }
    }

    /// Creates a context handle from a `'static` reference without allocating.
    #[must_use]
    pub const fn from_static(inner: &'static T) -> Self {
        Self { inner }
    }

    /// Returns a `'static` reference to the underlying context value.
    #[must_use]
    pub const fn get_ref(&self) -> &'static T {
        self.inner
    }
}

impl<T: ?Sized + 'static> Clone for Ctx<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized + 'static> Copy for Ctx<T> {}

impl<T: ?Sized + 'static> Deref for Ctx<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<T: ?Sized + fmt::Debug + 'static> fmt::Debug for Ctx<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Ctx").field(&self.inner).finish()
    }
}

/// Sequence-synchronized tearing-free ring buffer for POD telemetry and media frames.
///
/// Designed as a safe baseline contract matching `SharedRingBuf<T>` for Tier 1 in-process
/// and Tier 2 local shared-memory boundaries (where entries are copied as consistent snapshots).
#[derive(Debug, Clone)]
pub struct SharedRingBuf<T: Copy + Default> {
    capacity: usize,
    inner: Arc<Mutex<VecDeque<T>>>,
}

impl<T: Copy + Default> SharedRingBuf<T> {
    /// Creates a ring buffer holding up to `capacity` POD items (minimum 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            capacity: cap,
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
        }
    }

    /// Appends a single POD sample, evicting the oldest item if at capacity.
    pub fn write(&self, item: T) {
        if let Ok(mut guard) = self.inner.lock() {
            if guard.len() == self.capacity {
                guard.pop_front();
            }
            guard.push_back(item);
        }
    }

    /// Appends a slice of POD samples cleanly under one snapshot boundary.
    pub fn write_slice(&self, items: &[T]) {
        if let Ok(mut guard) = self.inner.lock() {
            for &item in items {
                if guard.len() == self.capacity {
                    guard.pop_front();
                }
                guard.push_back(item);
            }
        }
    }

    /// Copies the latest up to `out.len()` items in chronological order without tearing,
    /// returning the number of samples written into `out`.
    pub fn read_latest_snapshot(&self, out: &mut [T]) -> usize {
        if out.is_empty() {
            return 0;
        }
        let Ok(guard) = self.inner.lock() else {
            return 0;
        };
        let count = guard.len().min(out.len());
        let start = guard.len() - count;
        for (dst, src) in out.iter_mut().take(count).zip(guard.iter().skip(start)) {
            *dst = *src;
        }
        count
    }

    /// Returns the number of items currently retained in the ring buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// Returns `true` if the ring buffer contains no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the fixed maximum capacity of the ring buffer.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_and_typed_id_are_sortable_and_copy() {
        let u1 = Ulid::generate();
        let u2 = Ulid::generate();
        assert_eq!(u1.as_str().len(), 26);
        assert!(u2 > u1);
        assert_eq!(Ulid::parse(u1.as_str()).unwrap(), u1);

        struct UserMarker;
        let id: Id<UserMarker> = Id::new();
        let copied = id;
        assert_eq!(id, copied);
        assert_eq!(id.as_str().len(), 26);
    }

    #[test]
    fn cancellation_tree_propagates_to_descendants() {
        let root = CancellationToken::new();
        let child = root.child_token();
        let grandchild = child.child_token();
        assert!(!grandchild.is_cancelled());
        root.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
        assert!(grandchild.check().is_err());
    }

    #[test]
    fn error_http_status_mapping_follows_protocol() {
        assert_eq!(
            HyveError::domain("invalid_request", "bad").http_status(),
            400
        );
        assert_eq!(HyveError::unauthenticated("nope").http_status(), 401);
        assert_eq!(HyveError::forbidden("denied").http_status(), 403);
        assert_eq!(
            HyveError::not_found("project", "missing").http_status(),
            404
        );
        assert_eq!(
            HyveError::conflict("version_conflict", "stale").http_status(),
            409
        );
        assert_eq!(HyveError::rate_limited("slow down", 12).http_status(), 429);
        assert_eq!(HyveError::internal("boom").http_status(), 500);
    }

    #[test]
    fn shared_ring_buf_reads_latest_snapshot_without_tearing() {
        let rb = SharedRingBuf::new(4);
        rb.write_slice(&[10, 20, 30, 40, 50]);
        assert_eq!(rb.len(), 4);
        let mut out = [0i32; 3];
        let n = rb.read_latest_snapshot(&mut out);
        assert_eq!(n, 3);
        assert_eq!(out, [30, 40, 50]);
    }

    #[test]
    fn timestamp_formats_unix_epoch_cleanly() {
        assert_eq!(
            Timestamp::from_millis(0).to_rfc3339(),
            "1970-01-01T00:00:00.000Z"
        );
    }
}
