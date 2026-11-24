//! `hyve-macros` — procedural macros of the `hyve` core. Use them through the
//! `hyve` crate (`hyve::prelude::*`), whose paths the generated code refers to.
//!
//! Frameworks that re-export `hyve` (so their users may not depend on it
//! directly) pass the path explicitly: `#[model(crate = ::orbic::__private::hyve)]`,
//! `#[server(POST "/x", crate = ...)]`, `#[hyve::test(crate = ...)]`.

use proc_macro::TokenStream;
use syn::parse_macro_input;

mod model;
mod server;
mod test_attr;
mod util;

/// Declares a model: binary (fast rail) and JSON (standard rail) codecs,
/// validation, schema metadata and a schema fingerprint from one definition.
///
/// - Structs with named fields (generics allowed) and enums with unit
///   variants. JSON field names are `camelCase`; enum values are `snake_case`
///   strings. `Option` fields may be omitted in JSON.
/// - Field rules: `#[validate(len = <range>)]` (strings in characters, lists,
///   maps) and `#[validate(range = <range>)]` (numbers).
/// - Other attributes, including `#[derive(...)]`, pass through unchanged;
///   `#[model]` derives nothing else.
/// - `usize`/`isize` fields are rejected (their binary width is fixed at 32
///   bits). Directly recursive fields (`Vec<Self>`, `Option<Box<Node>>`) are
///   supported; mutually recursive models are not.
#[proc_macro_attribute]
pub fn model(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as model::ModelArgs);
    let item = parse_macro_input!(item as syn::Item);
    model::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Field rules of a `#[model]` struct. Read by `#[model]`; on its own it only
/// reports how to use it.
#[proc_macro_attribute]
pub fn validate(_args: TokenStream, item: TokenStream) -> TokenStream {
    let span = proc_macro2::TokenStream::from(item.clone());
    let err = util::error_at(
        &span,
        "`#[validate]` works only on fields of a `#[model]` struct",
        "add `#[model]` to the struct and put the rule on a field: `#[validate(len = 1..=80)] pub title: String`",
    );
    let mut out = err.into_compile_error();
    out.extend(span);
    out.into()
}

/// A server function: `#[server(POST "/projects")]`.
///
/// Calling the function by name returns a [`ServerCall`] in every build; awaiting
/// it sends the transported arguments through the transport provided in the
/// current scope. The body exists only with the declaring crate's `server`
/// feature and is reached only through `orbic`'s `.handle(create_project)`.
///
/// - Method is one of `GET`, `POST`, `PATCH`, `DELETE`; the path starts with
///   `/` and may contain `{name}` placeholders.
/// - `#[server_ctx] name: Ctx<T>` arguments come from the server's request
///   scope and are not sent. `Valid<T>` arguments are sent as `T`, checked
///   before sending and validated again by the server.
/// - The function must be `async`, non-generic, and return `HyveResult<T>`.
///
/// [`ServerCall`]: https://docs.rs/hyve-api
#[proc_macro_attribute]
pub fn server(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as server::ServerArgs);
    let item = parse_macro_input!(item as syn::ItemFn);
    server::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Marks a `Ctx<T>` argument of a `#[server]` function as injected on the
/// server. Read by `#[server]`; on its own it only reports how to use it.
#[proc_macro_attribute]
pub fn server_ctx(_args: TokenStream, item: TokenStream) -> TokenStream {
    let span = proc_macro2::TokenStream::from(item.clone());
    let err = util::error_at(
        &span,
        "`#[server_ctx]` works only on `Ctx<T>` arguments of a `#[server]` function",
        "write `#[server(POST \"/projects\")] pub async fn create(#[server_ctx] db: Ctx<Db>, …)`",
    );
    let mut out = err.into_compile_error();
    out.extend(span);
    out.into()
}

/// An async test on a `tokio` current-thread runtime, inside a fresh root
/// scope. Needs the `test` feature of `hyve` (dev-dependency); not available
/// on `wasm32` in Phase 1.
#[proc_macro_attribute]
pub fn test(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as test_attr::TestArgs);
    let item = parse_macro_input!(item as syn::ItemFn);
    test_attr::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
