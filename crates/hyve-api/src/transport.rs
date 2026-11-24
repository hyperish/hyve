//! The transport trait: typed calls, type-erased so implementations can be
//! trait objects provided in a scope.

use std::any::Any;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use hyve_core::{CancellationToken, HyveResult, Scope, Scoped};
use hyve_serde::json::{from_json, to_json};
use hyve_serde::{AlignedBytes, EncodeError};

use crate::{ServerFn, ServerFnDescriptor, Wire, errors};

/// A boxed future; `Send` except on `wasm32-unknown-unknown`, where browser
/// futures are not.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A boxed future; `Send` except on `wasm32-unknown-unknown`, where browser
/// futures are not.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Which rail a call travels on (`docs/03-adapters-and-pipeline.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rail {
    /// Binary (`hyve-serde`) bodies; falls back to [`Standard`](Rail::Standard)
    /// on `415`.
    Fast,
    /// HTTP/JSON per `hyperish/docs` `03-api-protocol.md`.
    Standard,
}

/// Makes typed calls to `#[server]` functions.
///
/// Implementations: `serge-adapter-client` (remote, over HTTP) and `orbic`
/// (in-process, used by port-less tests and single binaries). Rail choice,
/// encoding, fallback, credentials and cancellation are theirs.
pub trait Transport: Send + Sync + 'static {
    /// Performs `call` and returns the response, or the restored error.
    fn call(&self, call: Call) -> BoxFuture<'_, HyveResult<Reply>>;
}

/// A [`Transport`] as provided in a scope.
#[derive(Clone)]
pub struct TransportHandle(Arc<dyn Transport>);

impl TransportHandle {
    /// Wraps a transport.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        TransportHandle(transport)
    }

    /// Performs a call.
    pub fn call(&self, call: Call) -> BoxFuture<'_, HyveResult<Reply>> {
        self.0.call(call)
    }
}

impl fmt::Debug for TransportHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TransportHandle")
    }
}

/// Provides `transport` to every `#[server]` call made inside `scope` (and its
/// children). Apps do this once at the root.
pub fn provide_transport(scope: &Scope, transport: Arc<dyn Transport>) {
    scope.provide(TransportHandle::new(transport));
}

trait ErasedRequest: Send {
    fn encode_binary(&self) -> Result<AlignedBytes, EncodeError>;
    fn encode_json(&self) -> String;
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

struct Typed<T>(T);

impl<T: Wire> ErasedRequest for Typed<T> {
    fn encode_binary(&self) -> Result<AlignedBytes, EncodeError> {
        self.0.to_binary()
    }
    fn encode_json(&self) -> String {
        to_json(&self.0)
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        Box::new(self.0)
    }
}

/// One typed call: the descriptor plus the request model, encodable for
/// either rail or handed over as-is in process.
pub struct Call {
    descriptor: &'static ServerFnDescriptor,
    request: Box<dyn ErasedRequest>,
    cancellation: CancellationToken,
}

impl Call {
    /// A call of `M` with `request`.
    pub fn new<M: ServerFn>(request: M::Request, cancellation: CancellationToken) -> Self {
        Call {
            descriptor: M::descriptor(),
            request: Box::new(Typed(request)),
            cancellation,
        }
    }

    /// What is being called.
    pub fn descriptor(&self) -> &'static ServerFnDescriptor {
        self.descriptor
    }

    /// Cancelled when the caller goes away (for example the view unmounts);
    /// transports abort in-flight requests.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    /// The fast-rail request body. A `()` request (no transported arguments)
    /// archives to zero bytes.
    pub fn encode_binary(&self) -> Result<AlignedBytes, EncodeError> {
        self.request.encode_binary()
    }

    /// The standard-rail request model as JSON.
    pub fn encode_json(&self) -> String {
        self.request.encode_json()
    }

    /// The request model itself, for in-process transports that know the
    /// function's types (`orbic` registers them with `.handle(...)`).
    pub fn into_request<T: 'static>(self) -> Result<T, Self> {
        let Call {
            descriptor,
            request,
            cancellation,
        } = self;
        match request.into_any().downcast::<T>() {
            Ok(t) => Ok(*t),
            Err(any) => Err(Call {
                descriptor,
                request: Box::new(Opaque(any)),
                cancellation,
            }),
        }
    }
}

/// A request that has already been unwrapped once and did not match.
struct Opaque(Box<dyn Any + Send>);

impl ErasedRequest for Opaque {
    fn encode_binary(&self) -> Result<AlignedBytes, EncodeError> {
        Err(EncodeError::new(
            "the request was already taken out of the call",
        ))
    }
    fn encode_json(&self) -> String {
        String::from("null")
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self.0
    }
}

impl fmt::Debug for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// A successful response as the transport received it.
pub enum Reply {
    /// Fast rail: the archived response model.
    Binary(AlignedBytes),
    /// Standard rail: the response model as JSON (the envelope's `data`).
    Json(String),
    /// Standard rail `204`: success without a value.
    NoContent,
    /// In process: the response model itself.
    Value(Box<dyn Any + Send>),
}

impl Reply {
    /// An in-process reply.
    pub fn value<T: Wire>(value: T) -> Self {
        Reply::Value(Box::new(value))
    }

    /// Reads the response model; failures are `Infra` `decode_failed`.
    pub fn decode<T: Wire>(self) -> HyveResult<T> {
        match self {
            Reply::Binary(bytes) => T::from_binary(&bytes).map_err(|e| e.into_response_error()),
            Reply::Json(text) => from_json::<T>(&text).map_err(|e| e.into_response_error()),
            // `()` and `Option<T>` accept "no value"; anything else expected one.
            Reply::NoContent => from_json::<T>("null").map_err(|_| {
                errors::decode_failed(
                    "the server returned no content, but the call expects a value",
                )
            }),
            Reply::Value(any) => any.downcast::<T>().map(|b| *b).map_err(|_| {
                errors::decode_failed("in-process reply has a different type than the call expects")
            }),
        }
    }
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reply::Binary(b) => write!(f, "Reply::Binary({} bytes)", b.len()),
            Reply::Json(s) => write!(f, "Reply::Json({s})"),
            Reply::NoContent => f.write_str("Reply::NoContent"),
            Reply::Value(_) => f.write_str("Reply::Value(..)"),
        }
    }
}

/// A test client that can make `#[server]` calls through a whole server
/// pipeline on a chosen rail (`orbic`'s `TestClient` implements it).
pub trait TestClient {
    /// A transport that sends calls on `rail` (the fast rail still falls back
    /// on `415`, exactly like production transports).
    fn transport(&self, rail: Rail) -> Arc<dyn Transport>;

    /// Awaits `call` with this client's transport for `rail` provided in a
    /// child of the current scope:
    /// `client.on(Rail::Fast, create_project(input)).await`.
    fn on<C: IntoFuture>(&self, rail: Rail, call: C) -> Scoped<C::IntoFuture>
    where
        Self: Sized,
    {
        let scope = Scope::current().map_or_else(Scope::root, |s| s.child());
        provide_transport(&scope, self.transport(rail));
        let future = scope.enter(|| call.into_future());
        scope.scoped(future)
    }
}
