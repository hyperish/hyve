//! The contract descriptor `#[server]` leaves in every build.

use std::fmt;

use hyve_serde::{FORMAT_FINGERPRINT, Fingerprint, mix};
use hyve_validate::{SchemaNode, SchemaRegistry};

use crate::ErrorWire;

/// Binding method. Metadata only: `hyve` never interprets it; `orbic` maps it
/// to an HTTP route and the transports use it for the standard rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
}

impl Method {
    /// The upper-case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }

    /// Whether requests with this method carry no body on the standard rail
    /// (fields travel as query/path parameters instead).
    pub const fn is_bodyless(self) -> bool {
        matches!(self, Method::Get | Method::Delete)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Everything known about a `#[server]` function without its body.
#[derive(Clone, Copy)]
pub struct ServerFnDescriptor {
    /// Function identifier: crate name, module path and function name
    /// (`my_app::projects::create_project`). Sent as [`HEADER_FN`](crate::HEADER_FN).
    pub id: &'static str,
    /// Bound method.
    pub method: Method,
    /// Bound path, starting with `/`; `{name}` placeholders.
    pub path: &'static str,
    /// Names of the transported arguments in declaration order (`#[server_ctx]`
    /// arguments excluded). With one argument that is not a model, standard-rail
    /// adapters use this name for the query/path parameter.
    pub args: &'static [&'static str],
    /// Schema fingerprint of request, response and error forms; see
    /// [`server_fn_fingerprint`].
    pub fingerprint: u64,
    /// Schema of the request model.
    pub request_schema: fn(&mut SchemaRegistry) -> SchemaNode,
    /// Schema of the response model.
    pub response_schema: fn(&mut SchemaRegistry) -> SchemaNode,
}

impl ServerFnDescriptor {
    /// The fingerprint as sent in [`HEADER_SCHEMA`](crate::HEADER_SCHEMA).
    pub fn fingerprint_hex(&self) -> String {
        format!("{:016x}", self.fingerprint)
    }
}

impl fmt::Debug for ServerFnDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerFnDescriptor")
            .field("id", &self.id)
            .field("method", &self.method)
            .field("path", &self.path)
            .field("args", &self.args)
            .field("fingerprint", &format_args!("{:016x}", self.fingerprint))
            .finish_non_exhaustive()
    }
}

/// The fingerprint of a call contract: codec format, request model, response
/// model and the error wire form (`docs/03-adapters-and-pipeline.md` §3.1).
pub const fn server_fn_fingerprint(request: u64, response: u64) -> u64 {
    mix(
        mix(mix(FORMAT_FINGERPRINT, request), response),
        <ErrorWire as Fingerprint>::FINGERPRINT,
    )
}
