//! Shared helpers: diagnostics, crate paths, naming, type inspection.

use hyve_diagnostics::Diagnostic;
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::quote;
use syn::spanned::Spanned;
use syn::{GenericArgument, Path, PathArguments, Type};

/// A compile error that says what is wrong and how to fix it
/// (`docs/01-dx-and-dsl.md` §3).
pub(crate) fn error(span: Span, what: impl Into<String>, fix: impl Into<String>) -> syn::Error {
    syn::Error::new(span, Diagnostic::error(what).with_help(fix).summary())
}

/// Like [`error`], spanning a syntax node.
pub(crate) fn error_at(
    node: &impl Spanned,
    what: impl Into<String>,
    fix: impl Into<String>,
) -> syn::Error {
    error(node.span(), what, fix)
}

/// The path generated code uses to reach `hyve` (`::hyve` unless a framework
/// that re-exports `hyve` passes `crate = ...`).
#[derive(Clone)]
pub(crate) struct Krate(pub(crate) TokenStream);

impl Default for Krate {
    fn default() -> Self {
        Krate(quote!(::hyve))
    }
}

impl Krate {
    pub(crate) fn private(&self) -> TokenStream {
        let k = &self.0;
        quote!(#k::__private)
    }
    pub(crate) fn rkyv(&self) -> TokenStream {
        let p = self.private();
        quote!(#p::rkyv)
    }
    pub(crate) fn serde(&self) -> TokenStream {
        let p = self.private();
        quote!(#p::serde)
    }
    pub(crate) fn validate(&self) -> TokenStream {
        let p = self.private();
        quote!(#p::validate)
    }
    pub(crate) fn api(&self) -> TokenStream {
        let p = self.private();
        quote!(#p::api)
    }
    pub(crate) fn core(&self) -> TokenStream {
        let p = self.private();
        quote!(#p::core)
    }

    pub(crate) fn from_path(path: &Path) -> Self {
        Krate(quote!(#path))
    }
}

/// `snake_case` field name → `camelCase` JSON name (`max_fps` → `maxFps`).
pub(crate) fn camel_case(name: &str) -> String {
    let name = name.strip_prefix("r#").unwrap_or(name);
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    let mut seen_other = false;
    for c in name.chars() {
        if c == '_' && seen_other {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
            seen_other |= c != '_';
        }
    }
    out
}

/// `PascalCase` variant name → `snake_case` JSON value (`InProgress` →
/// `in_progress`, `HTTPError` → `http_error`).
pub(crate) fn snake_case(name: &str) -> String {
    let name = name.strip_prefix("r#").unwrap_or(name);
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_uppercase() => next.is_some_and(char::is_lowercase),
                _ => false,
            };
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `snake_case` function name → `PascalCase` (`create_project` → `CreateProject`).
pub(crate) fn pascal_case(name: &str) -> String {
    let camel = camel_case(name);
    let mut c = camel.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Whether `ty` mentions any of `idents` anywhere (`Vec<Option<usize>>`).
pub(crate) fn mentions(ty: &Type, idents: &[&str]) -> Option<Span> {
    fn walk(ts: TokenStream, idents: &[&str]) -> Option<Span> {
        for tt in ts {
            match tt {
                TokenTree::Ident(i) if idents.iter().any(|n| i == n) => return Some(i.span()),
                TokenTree::Group(g) => {
                    if let Some(s) = walk(g.stream(), idents) {
                        return Some(s);
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(quote!(#ty), idents)
}

/// If `ty` is `Name<Inner>` (by last path segment), returns `Inner`.
pub(crate) fn single_generic<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() {
        return None;
    }
    let seg = p.path.segments.last()?;
    if seg.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    let mut types = args.args.iter().filter_map(|a| match a {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    });
    let first = types.next()?;
    (args.args.len() == 1).then_some(first)
}

/// The generic arguments of `ty`'s last path segment named `name`.
pub(crate) fn generic_args<'a>(ty: &'a Type, name: &str) -> Option<Vec<&'a Type>> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    Some(
        args.args
            .iter()
            .filter_map(|a| match a {
                GenericArgument::Type(t) => Some(t),
                _ => None,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(camel_case("max_fps"), "maxFps");
        assert_eq!(camel_case("app_name"), "appName");
        assert_eq!(camel_case("r#type"), "type");
        assert_eq!(camel_case("url2_path"), "url2Path");
        assert_eq!(camel_case("_hidden_x"), "_hiddenX");
        assert_eq!(snake_case("InProgress"), "in_progress");
        assert_eq!(snake_case("HTTPError"), "http_error");
        assert_eq!(snake_case("V2Beta"), "v2_beta");
        assert_eq!(snake_case("Draft"), "draft");
        assert_eq!(pascal_case("create_project"), "CreateProject");
    }
}
