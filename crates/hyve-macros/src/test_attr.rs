//! `#[hyve::test]`: async tests on a `tokio` current-thread runtime that lives
//! behind the facade's `test` feature (`docs/02-architecture.md` §2.5).

use proc_macro2::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, ItemFn, Token};

use crate::util::{Krate, error_at};

pub(crate) struct TestArgs {
    krate: Krate,
}

impl Parse for TestArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut krate = Krate::default();
        if !input.is_empty() {
            let key: Ident = input.call(Ident::parse_any)?;
            if key != "crate" {
                return Err(error_at(
                    &key,
                    format!("`#[hyve::test]` takes no arguments, found `{key}`"),
                    "write `#[hyve::test]` above an `async fn`",
                ));
            }
            input.parse::<Token![=]>()?;
            krate = Krate::from_path(&input.parse()?);
        }
        Ok(TestArgs { krate })
    }
}

pub(crate) fn expand(args: TestArgs, f: ItemFn) -> syn::Result<TokenStream> {
    if !f.sig.inputs.is_empty() {
        return Err(error_at(
            &f.sig.inputs,
            "test functions take no arguments",
            "build what the test needs inside its body",
        ));
    }
    if !f.sig.generics.params.is_empty() {
        return Err(error_at(
            &f.sig.generics,
            "test functions cannot be generic",
            "remove the type parameters",
        ));
    }
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = f;
    if sig.asyncness.is_none() {
        // Nothing to drive: a plain test.
        return Ok(quote! {
            #[::core::prelude::v1::test]
            #(#attrs)*
            #vis #sig #block
        });
    }
    let private = args.krate.private();
    let name = &sig.ident;
    let ret = &sig.output;
    Ok(quote! {
        #[cfg(target_arch = "wasm32")]
        ::core::compile_error!(
            "`#[hyve::test]` is not available on wasm32 in Phase 1\n  = help: use `wasm_bindgen_test` for tests that run in the browser"
        );

        #[cfg(not(target_arch = "wasm32"))]
        #[::core::prelude::v1::test]
        #(#attrs)*
        #vis fn #name() #ret {
            #private::run_test::<_, #private::TestRuntime>(async move #block)
        }
    })
}
