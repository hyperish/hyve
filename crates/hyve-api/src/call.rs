//! Server function traits and the call value the stub returns.

use std::future::IntoFuture;

use hyve_core::{CancellationToken, HyveError, HyveResult, Scope};
use hyve_serde::json::Json;
use hyve_serde::{Binary, Fingerprint};
use hyve_validate::Schema;

use crate::transport::{BoxFuture, Call, TransportHandle};
use crate::{ServerFnDescriptor, errors};

/// Types that can be a request or response model: both codecs, a schema and a
/// fingerprint. `#[model]` types and the supported std/`hyve-core` types
/// qualify.
pub trait Wire: Binary + Json + Schema + Fingerprint + Send + 'static {}

impl<T: Binary + Json + Schema + Fingerprint + Send + 'static> Wire for T {}

/// A `#[server]` function's contract, implemented on a hidden marker type.
pub trait ServerFn: Sized + 'static {
    /// The request model: `()` for no transported arguments, the argument
    /// itself for one, a generated model with the argument names as fields
    /// for more (`Valid<T>` arguments contribute `T`).
    type Request: Wire;
    /// The response model (the `T` of `HyveResult<T>`).
    type Response: Wire;

    /// The descriptor.
    fn descriptor() -> &'static ServerFnDescriptor;
}

/// The handler body of a `#[server]` function. Generated only in builds with
/// the declaring crate's `server` feature, and called only by the request
/// pipeline `orbic` builds from `.handle(...)`, so guards always run.
pub trait ServerHandler: ServerFn {
    /// Runs the body for `request`. `#[server_ctx]` arguments are taken from
    /// `scope` (the request scope); `Valid<T>` arguments are validated first.
    fn handle(
        scope: Scope,
        request: Self::Request,
    ) -> BoxFuture<'static, HyveResult<Self::Response>>;
}

/// Recovers the marker of a `#[server]` function from the function item, so
/// `orbic` can register it by name: `.handle(create_project)`.
pub trait StubFn<Args> {
    /// The function's [`ServerFn`] marker.
    type Marker: ServerFn;
}

macro_rules! stub_fn {
    ($($a:ident),*) => {
        impl<F, M: ServerFn, $($a),*> StubFn<($($a,)*)> for F
        where
            F: Fn($($a),*) -> ServerCall<M>,
        {
            type Marker = M;
        }
    };
}

stub_fn!();
stub_fn!(A1);
stub_fn!(A1, A2);
stub_fn!(A1, A2, A3);
stub_fn!(A1, A2, A3, A4);
stub_fn!(A1, A2, A3, A4, A5);
stub_fn!(A1, A2, A3, A4, A5, A6);
stub_fn!(A1, A2, A3, A4, A5, A6, A7);
stub_fn!(A1, A2, A3, A4, A5, A6, A7, A8);
stub_fn!(A1, A2, A3, A4, A5, A6, A7, A8, A9);
stub_fn!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10);
stub_fn!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11);
stub_fn!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11, A12);

/// What calling a `#[server]` function returns. Nothing happens until it is
/// awaited; then the transport provided in the current scope makes the call.
#[must_use = "a server call does nothing until it is awaited"]
pub struct ServerCall<M: ServerFn> {
    request: M::Request,
    rejected: Option<HyveError>,
}

impl<M: ServerFn> ServerCall<M> {
    /// A call carrying `request`.
    pub fn new(request: M::Request) -> Self {
        ServerCall {
            request,
            rejected: None,
        }
    }

    /// Records the result of a client-side pre-check (validation of a
    /// `Valid<T>` argument). The first failure is returned on await without
    /// touching the transport.
    pub fn check(mut self, result: Result<(), HyveError>) -> Self {
        if self.rejected.is_none() {
            self.rejected = result.err();
        }
        self
    }

    /// The request model.
    pub fn request(&self) -> &M::Request {
        &self.request
    }

    /// The descriptor of the function being called.
    pub fn descriptor(&self) -> &'static ServerFnDescriptor {
        M::descriptor()
    }
}

impl<M: ServerFn> IntoFuture for ServerCall<M> {
    type Output = HyveResult<M::Response>;
    type IntoFuture = BoxFuture<'static, HyveResult<M::Response>>;

    fn into_future(self) -> Self::IntoFuture {
        // Look the transport up now, inside the caller's scope.
        let scope = Scope::current();
        let transport = scope.as_ref().and_then(|s| s.get::<TransportHandle>());
        let cancellation = scope.map_or_else(CancellationToken::new, |s| s.cancellation().child());
        Box::pin(async move {
            if let Some(e) = self.rejected {
                return Err(e);
            }
            let Some(transport) = transport else {
                return Err(errors::transport_unavailable(format!(
                    "no transport is provided in the current scope for `{}`; provide one at the app root \
                     (`hyve_api::provide_transport(&scope, transport)`)",
                    M::descriptor().id
                )));
            };
            let reply = transport
                .call(Call::new::<M>(self.request, cancellation))
                .await?;
            reply.decode::<M::Response>()
        })
    }
}
