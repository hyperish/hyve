//! An app provides a [`Tracer`] in its root scope. Spans, structured logs
//! and measurements in its descendants share request and trace IDs across
//! async polls on native and web targets.

use hyve_core::{RequestId, Scope, Timestamp, TraceId, Ulid, Value};
use std::sync::Arc;

/// The identity of a span and the request it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    /// Correlation ID of the entire trace.
    pub trace_id: TraceId,
    /// Correlation ID returned to the caller.
    pub request_id: RequestId,
    /// ID of this span.
    pub span_id: Ulid,
    /// Enclosing span, if any.
    pub parent: Option<Ulid>,
    /// Sampling decision propagated between services.
    pub sampled: bool,
}

/// The trace and calling span received at a transport boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceParent {
    trace: TraceId,
    span: u64,
    sampled: bool,
}

impl TraceParent {
    /// Accepts a W3C version 00 traceparent. Malformed, zero and unsupported
    /// identifiers start a fresh trace at the receiver.
    pub fn parse(header: &str) -> Option<Self> {
        if header.len() != 55 {
            return None;
        }
        let mut parts = header.split('-');
        if parts.next()? != "00" {
            return None;
        }
        let hex = |s: &str, width| {
            s.len() == width
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let trace = parts.next()?;
        let span = parts.next()?;
        let flags = parts.next()?;
        if !hex(trace, 32) || !hex(span, 16) || !hex(flags, 2) || parts.next().is_some() {
            return None;
        }
        let trace = TraceId::from_bytes(u128::from_str_radix(trace, 16).ok()?.to_be_bytes())?;
        let span = u64::from_str_radix(span, 16).ok()?;
        let sampled = u8::from_str_radix(flags, 16).ok()? & 1 != 0;
        (span != 0).then_some(Self {
            trace,
            span,
            sampled,
        })
    }
}
impl std::fmt::Display for TraceParent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "00-{}-{:016x}-{:02x}",
            self.trace,
            self.span,
            u8::from(self.sampled)
        )
    }
}

impl TraceContext {
    /// The W3C traceparent for an outbound call.
    pub fn trace_parent(&self) -> TraceParent {
        TraceParent {
            trace: self.trace_id,
            span: self.span_id.to_u128() as u64,
            sampled: self.sampled,
        }
    }
    /// Returns the context of the code currently executing.
    pub fn current() -> Option<Self> {
        Some((*Scope::current()?.get::<Self>()?).clone())
    }
}

/// Severity of a structured log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Developer diagnostics.
    Debug,
    /// Normal progress.
    Info,
    /// An unexpected, recoverable condition.
    Warn,
    /// A failed operation.
    Error,
}

/// Kind of telemetry record.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A span started.
    Open,
    /// A span was dropped, including when a task was cancelled.
    Close,
    /// A structured message.
    Log(Level),
    /// Increment to a counter.
    Count(u64),
    /// Latest numeric measurement.
    Gauge(f64),
}

/// An owned record suitable for buffering or forwarding to an app-selected sink.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Emission time in UTC.
    pub at: Timestamp,
    /// Request and span identity.
    pub context: TraceContext,
    /// Operation, message or instrument name.
    pub name: String,
    /// Type of event or measurement.
    pub kind: Kind,
    /// Application-defined structured fields.
    pub fields: Value,
}

/// App-selected receiver. Clones share one sink.
#[derive(Clone)]
pub struct Tracer(Arc<dyn Fn(Record) + Send + Sync>);

impl Tracer {
    /// Wraps a receiver that can be called from a request or UI thread.
    pub fn new(sink: impl Fn(Record) + Send + Sync + 'static) -> Self {
        Self(Arc::new(sink))
    }
}

/// An owned span with a child scope. Dropping the span also cancels work
/// started in that scope.
pub struct Span {
    scope: Scope,
    name: String,
}

impl Span {
    /// Starts a request, or a related subrequest when called within a trace.
    pub fn request(parent: &Scope, request_id: RequestId, name: impl Into<String>) -> Self {
        Self::incoming(parent, request_id, None, name)
    }

    /// Starts a request under a validated remote parent, or the current local span.
    pub fn incoming(
        parent: &Scope,
        request_id: RequestId,
        remote: Option<TraceParent>,
        name: impl Into<String>,
    ) -> Self {
        let enclosing = remote
            .or_else(|| TraceContext::current().map(|c| c.trace_parent()))
            .or_else(|| parent.get::<TraceContext>().map(|c| c.trace_parent()));
        Self::start(parent, request_id, enclosing, name.into())
    }

    fn start(
        parent: &Scope,
        request_id: RequestId,
        enclosing: Option<TraceParent>,
        name: String,
    ) -> Self {
        let context = TraceContext {
            trace_id: enclosing.map(|c| c.trace).unwrap_or_default(),
            request_id: request_id.clone(),
            span_id: loop {
                let low = Ulid::new().to_u128() as u64;
                if low != 0 {
                    break Ulid::from_u128(u128::from(low));
                }
            },
            parent: enclosing.map(|c| Ulid::from_u128(u128::from(c.span))),
            sampled: enclosing.is_none_or(|c| c.sampled),
        };
        let scope = parent.child();
        scope.provide(context).provide(request_id);
        let span = Self { scope, name };
        record_in(&span.scope, &span.name, Kind::Open, Value::Null);
        span
    }

    /// Starts a child operation with the same request and trace IDs.
    pub fn child(&self, name: impl Into<String>) -> Self {
        let parent = self.scope.get::<TraceContext>().expect("live span");
        Self::start(
            &self.scope,
            parent.request_id.clone(),
            Some(parent.trace_parent()),
            name.into(),
        )
    }

    /// Enter for synchronous work or wrap each poll with `scope().scoped(future)`.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        record_in(&self.scope, &self.name, Kind::Close, Value::Null);
        self.scope.dispose();
    }
}

/// Emits a log or measurement to the sink of the current span.
pub fn record(name: &str, kind: Kind, fields: Value) {
    if let Some(scope) = Scope::current() {
        record_in(&scope, name, kind, fields);
    }
}

fn record_in(scope: &Scope, name: &str, kind: Kind, fields: Value) {
    if let (Some(sink), Some(context)) = (scope.get::<Tracer>(), scope.get::<TraceContext>()) {
        (sink.0)(Record {
            at: Timestamp::now(),
            context: (*context).clone(),
            name: name.into(),
            kind,
            fields,
        });
    }
}
