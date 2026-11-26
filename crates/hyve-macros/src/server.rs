//! `#[server(METHOD "/path")]`: the contract descriptor and call stub in every
//! build, the handler body only with the declaring crate's `server` feature
//! (`docs/02-architecture.md` §2.4, `docs/03-adapters-and-pipeline.md` §3.1).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{FnArg, Ident, ItemFn, ItemStruct, LitStr, Pat, ReturnType, Token, Type};

use crate::model;
use crate::util::{Krate, error, error_at, generic_args, pascal_case, single_generic};

/// Most transported arguments a stub can take (`hyve_api::StubFn` arities).
const MAX_ARGS: usize = 12;

const SYNTAX: &str = "write `#[server(POST \"/projects\")]`: one of GET, POST, PATCH, DELETE and a path starting with `/`";

pub(crate) struct ServerArgs {
    method: Ident,
    path: LitStr,
    krate: Krate,
}

impl Parse for ServerArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(error(
                input.span(),
                "`#[server]` needs a method and a path",
                SYNTAX,
            ));
        }
        let method: Ident = input.call(Ident::parse_any)?;
        let upper = method.to_string().to_uppercase();
        if !matches!(upper.as_str(), "GET" | "POST" | "PATCH" | "DELETE") || method != upper {
            let hint = if matches!(upper.as_str(), "GET" | "POST" | "PATCH" | "DELETE") {
                format!("write the method in upper case: `{upper}`")
            } else {
                format!("`{method}` is not supported; {SYNTAX}")
            };
            return Err(error_at(
                &method,
                format!("unknown method `{method}` in `#[server]`"),
                hint,
            ));
        }
        if !input.peek(LitStr) {
            return Err(error(
                input.span(),
                "`#[server]` needs a path after the method",
                SYNTAX,
            ));
        }
        let path: LitStr = input.parse()?;
        check_path(&path)?;
        let mut krate = Krate::default();
        if input.parse::<Option<Token![,]>>()?.is_some() && !input.is_empty() {
            let key: Ident = input.call(Ident::parse_any)?;
            if key != "crate" {
                return Err(error_at(
                    &key,
                    format!("unexpected `{key}` in `#[server]`"),
                    SYNTAX,
                ));
            }
            input.parse::<Token![=]>()?;
            krate = Krate::from_path(&input.parse()?);
        }
        if !input.is_empty() {
            return Err(error(
                input.span(),
                "unexpected tokens in `#[server]`",
                SYNTAX,
            ));
        }
        Ok(ServerArgs {
            method,
            path,
            krate,
        })
    }
}

fn check_path(path: &LitStr) -> syn::Result<()> {
    let p = path.value();
    let bad = |what: &str| {
        Err(error_at(
            path,
            format!("invalid `#[server]` path {p:?}: {what}"),
            "write a path such as \"/projects\" or \"/projects/{id}\"",
        ))
    };
    if !p.starts_with('/') {
        return bad("it must start with `/`");
    }
    if p.chars().any(char::is_whitespace) || p.contains('?') || p.contains('#') {
        return bad("it cannot contain whitespace, `?` or `#`");
    }
    let mut rest = p.as_str();
    while let Some(open) = rest.find(['{', '}']) {
        if &rest[open..open + 1] == "}" {
            return bad("unmatched `}`");
        }
        let Some(close) = rest[open..].find('}') else {
            return bad("unmatched `{`");
        };
        let name = &rest[open + 1..open + close];
        let ident_like = !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit());
        if !ident_like {
            return bad("placeholders are `{name}` with a field or argument name");
        }
        rest = &rest[open + close + 1..];
    }
    Ok(())
}

enum ArgKind {
    /// `#[server_ctx] name: Ctx<T>`: taken from the request scope.
    Ctx(Type),
    /// `name: Valid<T>`: transported as `T`, validated on both sides.
    Valid(Type),
    /// Transported as is.
    Plain,
}

struct Arg {
    ident: Ident,
    /// The argument as written for the handler body (`#[server_ctx]` removed).
    body: FnArg,
    ty: Type,
    kind: ArgKind,
}

impl Arg {
    fn wire_ty(&self) -> &Type {
        match &self.kind {
            ArgKind::Valid(t) => t,
            _ => &self.ty,
        }
    }
}

fn parse_arg(arg: &FnArg) -> syn::Result<Arg> {
    let FnArg::Typed(pt) = arg else {
        return Err(error_at(
            arg,
            "`#[server]` functions cannot take `self`",
            "make it a free function: `pub async fn create_project(input: Valid<CreateProject>) -> HyveResult<Project>`",
        ));
    };
    let Pat::Ident(pi) = &*pt.pat else {
        return Err(error_at(
            &pt.pat,
            "`#[server]` arguments need plain names",
            "bind the argument to a name and destructure it in the body: `input: Valid<CreateProject>`",
        ));
    };
    let mut body = pt.clone();
    let mut is_ctx = false;
    let mut errs = Ok(());
    body.attrs.retain(|a| {
        if a.path().is_ident("server_ctx") {
            if !matches!(a.meta, syn::Meta::Path(_)) {
                errs = Err(error_at(
                    a,
                    "`#[server_ctx]` takes no arguments",
                    "write `#[server_ctx] db: Ctx<Db>`",
                ));
            }
            is_ctx = true;
            false
        } else {
            true
        }
    });
    errs?;
    let ty = (*pt.ty).clone();
    let ctx_inner = single_generic(&ty, "Ctx").cloned();
    let kind = match (is_ctx, ctx_inner) {
        (true, Some(inner)) => ArgKind::Ctx(inner),
        (true, None) => {
            return Err(error_at(
                &pt.ty,
                "`#[server_ctx]` applies only to `Ctx<T>` arguments",
                "inject server-side values as `#[server_ctx] db: Ctx<Db>`; other arguments are sent by the caller",
            ));
        }
        (false, Some(inner)) => {
            let inner = quote!(#inner).to_string().replace(' ', "");
            return Err(error_at(
                &pt.ty,
                format!("`Ctx<{inner}>` is provided by the server, not sent by the caller"),
                format!("mark it `#[server_ctx] {}: Ctx<{inner}>`", pi.ident),
            ));
        }
        (false, None) => match single_generic(&ty, "Valid") {
            Some(inner) => ArgKind::Valid(inner.clone()),
            None => ArgKind::Plain,
        },
    };
    Ok(Arg {
        ident: pi.ident.clone(),
        body: FnArg::Typed(body),
        ty,
        kind,
    })
}

fn response_type(sig: &syn::Signature) -> syn::Result<Type> {
    let fix = "return `HyveResult<T>` (or `Result<T, HyveError>`), e.g. `-> HyveResult<Project>`; use `HyveResult<()>` for no value";
    let ReturnType::Type(_, ty) = &sig.output else {
        return Err(error_at(
            &sig.ident,
            "`#[server]` functions must return a result",
            fix,
        ));
    };
    if let Some(t) = single_generic(ty, "HyveResult") {
        return Ok(t.clone());
    }
    if let Some(args) = generic_args(ty, "Result") {
        if let [ok, Type::Path(err)] = args.as_slice() {
            if err
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "HyveError")
            {
                return Ok((*ok).clone());
            }
        }
    }
    Err(error_at(
        ty,
        "`#[server]` functions must return `HyveResult<T>`",
        fix,
    ))
}

pub(crate) fn expand(args: ServerArgs, f: ItemFn) -> syn::Result<TokenStream> {
    let sig = &f.sig;
    if sig.asyncness.is_none() {
        return Err(error_at(
            &sig.fn_token,
            "`#[server]` functions must be `async`",
            "write `pub async fn …`",
        ));
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(error_at(
            &sig.generics,
            "`#[server]` functions cannot be generic",
            "use concrete request and response types (declare them with `#[model]`)",
        ));
    }
    if let Some(u) = &sig.unsafety {
        return Err(error_at(
            u,
            "`#[server]` functions cannot be `unsafe`",
            "remove `unsafe`",
        ));
    }
    if let Some(v) = &sig.variadic {
        return Err(error_at(
            v,
            "`#[server]` functions cannot be variadic",
            "take a list model instead",
        ));
    }
    let response = response_type(sig)?;
    let parsed: Vec<Arg> = sig
        .inputs
        .iter()
        .map(parse_arg)
        .collect::<syn::Result<_>>()?;
    let wire: Vec<&Arg> = parsed
        .iter()
        .filter(|a| !matches!(a.kind, ArgKind::Ctx(_)))
        .collect();
    if wire.len() > MAX_ARGS {
        return Err(error_at(
            &sig.inputs,
            format!("`#[server]` functions take at most {MAX_ARGS} transported arguments"),
            "group them into one `#[model]` struct",
        ));
    }

    let krate = &args.krate;
    let api = krate.api();
    let core = krate.core();
    let serde = krate.serde();
    let validate = krate.validate();
    let name = &sig.ident;
    let name_str = name.to_string();
    let vis = &f.vis;
    let marker = format_ident!("__hyve_server_{}", name);
    let sig_types: Vec<&Type> = wire.iter().map(|a| &a.ty).collect();
    let path = &args.path;
    let method = format_ident!("{}", pascal_case(&args.method.to_string().to_lowercase()));
    let arg_names: Vec<String> = wire.iter().map(|a| a.ident.to_string()).collect();

    // Request model.
    let (request_ty, request_item, request_expr, destructure) = match wire.as_slice() {
        [] => (
            quote!(()),
            quote!(),
            quote!(()),
            quote!(let () = __request;),
        ),
        [one] => {
            let ident = &one.ident;
            let ty = one.wire_ty();
            (
                quote!(#ty),
                quote!(),
                quote!(#ident),
                quote!(let #ident = __request;),
            )
        }
        many => {
            let req = format_ident!("__hyve_request_{}", name);
            let fields = many.iter().map(|a| {
                let (ident, ty) = (&a.ident, a.wire_ty());
                quote!(pub #ident: #ty)
            });
            let item: ItemStruct = syn::parse_quote! {
                #[doc(hidden)]
                #[allow(non_camel_case_types)]
                #vis struct #req { #(#fields),* }
            };
            let expanded = model::expand_struct(
                item,
                krate,
                Some(format!("{}Request", pascal_case(&name_str))),
                None,
            )?;
            let idents: Vec<&Ident> = many.iter().map(|a| &a.ident).collect();
            (
                quote!(#req),
                expanded,
                quote!(#req { #(#idents),* }),
                quote!(let #req { #(#idents),* } = __request;),
            )
        }
    };

    // Stub.
    let stub_params = wire.iter().map(|a| {
        let (ident, ty) = (&a.ident, a.wire_ty());
        quote!(#ident: #ty)
    });
    let valid_idents: Vec<&Ident> = wire
        .iter()
        .filter(|a| matches!(a.kind, ArgKind::Valid(_)))
        .map(|a| &a.ident)
        .collect();
    let stub_body = if valid_idents.is_empty() {
        quote!(#api::ServerCall::new(#request_expr))
    } else {
        quote! {
            let __hyve_check: ::core::result::Result<(), #core::HyveError> = ::core::result::Result::Ok(());
            #(
                let __hyve_check = __hyve_check.and_then(|()| {
                    #validate::Validate::validate(&#valid_idents).map_err(#validate::Violations::into_error)
                });
            )*
            #api::ServerCall::new(#request_expr).check(__hyve_check)
        }
    };
    let attrs = &f.attrs;

    // Handler (server builds only).
    let body_params = parsed.iter().map(|a| &a.body);
    let ret = &sig.output;
    let block = &f.block;
    let ctx_needs: Vec<&Type> = parsed
        .iter()
        .filter_map(|a| match &a.kind {
            ArgKind::Ctx(inner) => Some(inner),
            _ => None,
        })
        .collect();
    let ctx_lets = parsed.iter().filter_map(|a| match &a.kind {
        ArgKind::Ctx(inner) => {
            let ident = &a.ident;
            Some(quote!(let #ident = __scope.require::<#inner>()?;))
        }
        _ => None,
    });
    let valid_lets = valid_idents
        .iter()
        .map(|i| quote!(let #i = #validate::Valid::new(#i)?;));
    let call_args = parsed.iter().map(|a| &a.ident);

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #vis struct #marker;

        #request_item

        impl #api::ServerFn for #marker {
            type Request = #request_ty;
            type Response = #response;

            fn descriptor() -> &'static #api::ServerFnDescriptor {
                static DESCRIPTOR: #api::ServerFnDescriptor = #api::ServerFnDescriptor {
                    id: ::core::concat!(::core::module_path!(), "::", #name_str),
                    method: #api::Method::#method,
                    path: #path,
                    args: &[#(#arg_names),*],
                    fingerprint: #api::server_fn_fingerprint(
                        <#request_ty as #serde::Fingerprint>::FINGERPRINT,
                        <#response as #serde::Fingerprint>::FINGERPRINT,
                    ),
                    request_schema: <#request_ty as #validate::Schema>::schema,
                    response_schema: <#response as #validate::Schema>::schema,
                };
                &DESCRIPTOR
            }
        }

        #(#attrs)*
        #vis fn #name(#(#stub_params),*) -> #api::ServerCall<#marker> {
            #stub_body
        }

        // Names the signature's types as written in every build, so imports
        // used only by the signature stay used when the body is compiled out.
        // `Ctx` arguments are left out: their types are often server-only.
        const _: ::core::marker::PhantomData<fn(#(#sig_types),*) #ret> = ::core::marker::PhantomData;

        // `server` is the declaring crate's feature. The allow sits on an
        // enclosing item because rustc evaluates a `cfg` before the lint
        // attributes of the item that carries it.
        #[allow(unexpected_cfgs)]
        const _: () = {
            #[cfg(feature = "server")]
            const _: () = {
                #[allow(clippy::too_many_arguments)]
                async fn __hyve_body(#(#body_params),*) #ret #block

                impl #api::ServerHandler for #marker {
                    fn context() -> ::std::vec::Vec<#api::ContextNeed> {
                        ::std::vec![#(#api::ContextNeed::of::<#ctx_needs>()),*]
                    }

                    fn handle(
                        __scope: #core::Scope,
                        __request: #request_ty,
                    ) -> #api::BoxFuture<'static, #core::HyveResult<#response>> {
                        ::std::boxed::Box::pin(async move {
                            #destructure
                            #(#ctx_lets)*
                            #(#valid_lets)*
                            __hyve_body(#(#call_args),*).await
                        })
                    }
                }
            };
        };
    })
}
