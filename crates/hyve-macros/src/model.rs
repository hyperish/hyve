//! `#[model]`: one declaration → binary codec (rkyv derive), JSON codec,
//! validation, schema metadata and fingerprint.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, Fields, GenericParam, Generics, Ident, Item, ItemEnum, ItemStruct, Type,
};

use crate::util::{Krate, camel_case, error, error_at, mentions, snake_case};

/// `#[model]` / `#[model(crate = path)]`.
pub(crate) struct ModelArgs {
    pub(crate) krate: Krate,
}

impl Parse for ModelArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut krate = Krate::default();
        if input.is_empty() {
            return Ok(ModelArgs { krate });
        }
        let key: Ident = input.call(syn::ext::IdentExt::parse_any)?;
        if key != "crate" {
            return Err(error_at(
                &key,
                format!("`#[model]` takes no arguments, found `{key}`"),
                "write `#[model]` and put rules on fields: `#[validate(len = 1..=80)] pub name: String`",
            ));
        }
        input.parse::<syn::Token![=]>()?;
        let path: syn::Path = input.parse()?;
        krate = Krate::from_path(&path);
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after `crate = <path>`"));
        }
        Ok(ModelArgs { krate })
    }
}

pub(crate) fn expand(args: ModelArgs, item: Item) -> syn::Result<TokenStream> {
    match item {
        Item::Struct(s) => expand_struct(s, &args.krate, None),
        Item::Enum(e) => expand_enum(e, &args.krate),
        other => Err(error_at(
            &other,
            "`#[model]` applies to structs with named fields and to enums with unit variants",
            "declare a struct such as `#[model] pub struct Project { pub title: String }`",
        )),
    }
}

struct Rule {
    kind: RuleKind,
    expr: Expr,
}

enum RuleKind {
    Len,
    Range,
}

struct Field {
    ident: Ident,
    ty: Type,
    json: String,
    rules: Vec<Rule>,
    recursive: bool,
}

/// Takes `#[validate(...)]` attributes off a field.
fn take_rules(attrs: &mut Vec<Attribute>) -> syn::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    let mut err: Option<syn::Error> = None;
    attrs.retain(|attr| {
        if !attr.path().is_ident("validate") {
            return true;
        }
        let parsed = match &attr.meta {
            syn::Meta::List(_) => attr.parse_nested_meta(|meta| {
                let kind = if meta.path.is_ident("len") {
                    RuleKind::Len
                } else if meta.path.is_ident("range") {
                    RuleKind::Range
                } else {
                    let name = meta.path.get_ident().map(|i| i.to_string()).unwrap_or_else(|| "?".into());
                    return Err(error_at(
                        &meta.path,
                        format!("unknown validation rule `{name}`"),
                        "Phase 1 rules are `len = <range>` and `range = <range>`, e.g. `#[validate(len = 1..=80)]` or `#[validate(range = 30..=240)]`",
                    ));
                };
                let expr: Expr = meta.value().map_err(|_| {
                    error_at(&meta.path, "a validation rule needs a range", "write it as `len = 1..=80` or `range = 0.0..=1.0`")
                })?
                .parse()?;
                rules.push(Rule { kind, expr });
                Ok(())
            }),
            _ => Err(error_at(
                attr,
                "`#[validate]` needs at least one rule",
                "write `#[validate(len = 1..=80)]` or `#[validate(range = 30..=240)]`",
            )),
        };
        if let Err(e) = parsed {
            match &mut err {
                Some(acc) => acc.combine(e),
                None => err = Some(e),
            }
        }
        false
    });
    match err {
        Some(e) => Err(e),
        None => Ok(rules),
    }
}

fn reject_validate_attr(attrs: &[Attribute], what: &str) -> syn::Result<()> {
    match attrs.iter().find(|a| a.path().is_ident("validate")) {
        Some(a) => Err(error_at(
            a,
            format!("`#[validate]` does not apply to {what}"),
            "put it on a field of a `#[model]` struct: `#[validate(len = 1..=80)] pub name: String`",
        )),
        None => Ok(()),
    }
}

fn check_generics(generics: &Generics) -> syn::Result<()> {
    for p in &generics.params {
        if let GenericParam::Lifetime(l) = p {
            return Err(error_at(
                l,
                "`#[model]` types cannot borrow (lifetime parameters are not supported)",
                "use owned fields such as `String` and `Vec<T>` instead of `&'a str` and `&'a [T]`",
            ));
        }
    }
    Ok(())
}

fn type_params(generics: &Generics) -> Vec<&Ident> {
    generics.type_params().map(|t| &t.ident).collect()
}

/// `impl` generics with `bound` added to every type parameter.
fn bounded(generics: &Generics, bound: TokenStream) -> Generics {
    let mut g = generics.clone();
    let params = type_params(generics)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let wc = g.make_where_clause();
    for p in params {
        wc.predicates.push(syn::parse_quote!(#p: #bound));
    }
    g
}

/// Folds field fingerprints the way `hyve-api`'s hand-written shapes do:
/// `[]` → 0, `[a]` → a, `[a, rest..]` → mix(a, fold(rest)).
fn fold_fingerprints(serde: &TokenStream, fps: &[TokenStream]) -> TokenStream {
    match fps {
        [] => quote!(0u64),
        [one] => one.clone(),
        [first, rest @ ..] => {
            let rest = fold_fingerprints(serde, rest);
            quote!(#serde::mix(#first, #rest))
        }
    }
}

/// Expands a struct. `schema_name` overrides the schema definition name (used
/// for the request models `#[server]` generates).
pub(crate) fn expand_struct(
    mut s: ItemStruct,
    krate: &Krate,
    schema_name: Option<String>,
) -> syn::Result<TokenStream> {
    reject_validate_attr(&s.attrs, "a whole struct")?;
    check_generics(&s.generics)?;
    let Fields::Named(named) = &mut s.fields else {
        return Err(error_at(
            &s.fields,
            "`#[model]` structs need named fields",
            format!(
                "give each field a name: `pub struct {} {{ pub x: f64, pub y: f64 }}`",
                s.ident
            ),
        ));
    };

    let name = s.ident.clone();
    let name_str = name.to_string();
    // Whether `Default` is implemented is only knowable for a concrete type;
    // generic models report no default.
    let default_value = if s.generics.params.is_empty() {
        let validate = krate.validate();
        quote! {
            #[allow(clippy::needless_borrow)]
            fn default_value() -> ::core::option::Option<Self> {
                #[allow(unused_imports)]
                use #validate::__private::{NoDefault as _, ViaDefault as _};
                (&#validate::__private::DefaultProbe::<Self>::new()).probe_default()
            }
        }
    } else {
        quote! {}
    };
    let mut fields = Vec::new();
    let mut errors: Option<syn::Error> = None;
    let mut push_err = |e: syn::Error| match &mut errors {
        Some(acc) => acc.combine(e),
        None => errors = Some(e),
    };
    for f in named.named.iter_mut() {
        let rules = match take_rules(&mut f.attrs) {
            Ok(r) => r,
            Err(e) => {
                push_err(e);
                Vec::new()
            }
        };
        if let Some(span) = mentions(&f.ty, &["usize", "isize"]) {
            push_err(error(
                span,
                "`usize`/`isize` are not wire types",
                "use `u32`/`u64` or `i32`/`i64`: the binary format fixes `usize` at 32 bits, so larger values would not survive the fast rail",
            ));
        }
        let recursive = mentions(&f.ty, &[name_str.as_str(), "Self"]).is_some();
        if recursive {
            f.attrs.push(syn::parse_quote!(#[rkyv(omit_bounds)]));
        }
        let ident = f.ident.clone().expect("named field");
        fields.push(Field {
            json: camel_case(&ident.to_string()),
            ident,
            ty: f.ty.clone(),
            rules,
            recursive,
        });
    }
    if let Some(e) = errors {
        return Err(e);
    }

    let rkyv = krate.rkyv();
    let serde = krate.serde();
    let validate = krate.validate();
    let json = quote!(#serde::json);
    // Our derive goes after the user's attributes, so attribute macros such as
    // `orbic`'s `#[table]` run first and remove their field helpers.
    s.attrs
        .push(syn::parse_quote!(#[derive(#rkyv::Archive, #rkyv::Serialize, #rkyv::Deserialize)]));
    s.attrs.push(syn::parse_quote!(#[rkyv(crate = #rkyv)]));
    if fields.iter().any(|f| f.recursive) {
        s.attrs.push(syn::parse_quote!(#[rkyv(serialize_bounds(
            __S: #rkyv::ser::Writer + #rkyv::ser::Allocator,
            __S::Error: #rkyv::rancor::Source,
        ))]));
        s.attrs.push(
            syn::parse_quote!(#[rkyv(deserialize_bounds(__D::Error: #rkyv::rancor::Source))]),
        );
        s.attrs.push(syn::parse_quote!(
            #[rkyv(bytecheck(bounds(__C: #rkyv::validation::ArchiveContext, __C::Error: #rkyv::rancor::Source)))]
        ));
    }

    // ----- JSON -----
    let enc = fields.iter().map(|f| {
        let (ident, key) = (&f.ident, &f.json);
        quote! {
            __w.key(#key);
            #json::Json::encode_json(&self.#ident, __w);
        }
    });
    let slots: Vec<Ident> = (0..fields.len()).map(|i| format_ident!("__f{i}")).collect();
    let slot_decls = fields.iter().zip(&slots).map(|(f, slot)| {
        let ty = &f.ty;
        quote!(let mut #slot: ::core::option::Option<#ty> = ::core::option::Option::None;)
    });
    let arms = fields.iter().zip(&slots).map(|(f, slot)| {
        let key = &f.json;
        quote! {
            #key => {
                if #slot.is_some() {
                    return ::core::result::Result::Err(__r.duplicate_field(#key));
                }
                #slot = ::core::option::Option::Some(#json::Json::decode_json(__r)?);
            }
        }
    });
    let inits = fields.iter().zip(&slots).map(|(f, slot)| {
        let (ident, ty, key) = (&f.ident, &f.ty, &f.json);
        quote! {
            #ident: match #slot {
                ::core::option::Option::Some(v) => v,
                ::core::option::Option::None => <#ty as #json::Json>::missing()
                    .ok_or_else(|| __r.missing_field(#key))?,
            }
        }
    });

    // ----- validation -----
    let checks = fields.iter().map(|f| {
        let (ident, key) = (&f.ident, &f.json);
        let rules = f.rules.iter().map(|r| {
            let expr = &r.expr;
            let span = expr.span();
            match r.kind {
                RuleKind::Len => quote_spanned!(span=> #validate::rules::len(&self.#ident, #expr, __path, __out);),
                RuleKind::Range => quote_spanned!(span=> #validate::rules::range(&self.#ident, #expr, __path, __out);),
            }
        });
        quote! {
            __path.push_field(#key);
            #(#rules)*
            #validate::Validate::validate_at(&self.#ident, __path, __out);
            __path.truncate(__mark);
        }
    });

    // ----- schema -----
    let field_schemas = fields.iter().map(|f| {
        let (ty, key) = (&f.ty, &f.json);
        let rust_name = f.ident.to_string();
        let rust_name = rust_name.strip_prefix("r#").unwrap_or(&rust_name).to_owned();
        let constraints = f.rules.iter().map(|r| {
            let expr = &r.expr;
            match r.kind {
                RuleKind::Len => quote!(#validate::rules::len_constraint(#expr)),
                RuleKind::Range => {
                    quote!(#validate::rules::range_constraint(::core::marker::PhantomData::<fn(&#ty)>, #expr))
                }
            }
        });
        quote! {
            #validate::FieldSchema {
                name: #rust_name,
                json_name: #key,
                schema: <#ty as #validate::Schema>::schema(__reg),
                required: !<#ty as #validate::Schema>::optional(),
                constraints: ::std::vec![#(#constraints),*],
            }
        }
    });
    let params = type_params(&s.generics);
    let schema_name = schema_name.unwrap_or_else(|| name_str.clone());
    let def_name = if params.is_empty() {
        quote!(#schema_name)
    } else {
        quote!(#validate::generic_name(#schema_name, &[#(<#params as #validate::Schema>::schema(__reg)),*]))
    };

    // ----- fingerprint -----
    let shape = format!(
        "{schema_name}{{{}}}",
        fields
            .iter()
            .map(|f| f.json.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    let fps: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let ty = &f.ty;
            if f.recursive {
                // Recursive positions are fingerprinted by their spelling (a
                // const cannot depend on itself).
                let spelled = quote!(#ty).to_string();
                quote!(#serde::fnv1a(#spelled.as_bytes()))
            } else {
                quote!(<#ty as #serde::Fingerprint>::FINGERPRINT)
            }
        })
        .collect();
    let fold = fold_fingerprints(&serde, &fps);

    let gj = bounded(&s.generics, quote!(#json::Json));
    let (ij, tj, wj) = gj.split_for_impl();
    let gv = bounded(&s.generics, quote!(#validate::Validate));
    let (iv, tv, wv) = gv.split_for_impl();
    let gs = bounded(&s.generics, quote!(#validate::Schema + 'static));
    let (is, ts, ws) = gs.split_for_impl();
    let gf = bounded(&s.generics, quote!(#serde::Fingerprint));
    let (iff, tf, wf) = gf.split_for_impl();

    Ok(quote! {
        #s

        impl #ij #json::Json for #name #tj #wj {
            fn encode_json(&self, __w: &mut #json::JsonWriter) {
                __w.begin_object();
                #(#enc)*
                __w.end_object();
            }

            fn decode_json(__r: &mut #json::JsonReader<'_>) -> ::core::result::Result<Self, #serde::DecodeError> {
                __r.begin_object()?;
                #(#slot_decls)*
                while let ::core::option::Option::Some(__key) = __r.next_key()? {
                    match &*__key {
                        #(#arms)*
                        _ => __r.skip_value()?,
                    }
                }
                ::core::result::Result::Ok(#name { #(#inits),* })
            }
        }

        impl #iv #validate::Validate for #name #tv #wv {
            fn validate_at(&self, __path: &mut #validate::FieldPath, __out: &mut #validate::Violations) {
                let __mark = __path.mark();
                #(#checks)*
                let _ = __mark;
            }
        }

        impl #is #validate::Schema for #name #ts #ws {
            fn schema(__reg: &mut #validate::SchemaRegistry) -> #validate::SchemaNode {
                let __name = #def_name;
                __reg.define::<Self>(__name, |__reg| {
                    #validate::Definition::Object(::std::vec![#(#field_schemas),*])
                })
            }

            #default_value
        }

        impl #iff #serde::Fingerprint for #name #tf #wf {
            const FINGERPRINT: u64 = #serde::mix(#serde::fnv1a(#shape.as_bytes()), #fold);
        }
    })
}

fn expand_enum(e: ItemEnum, krate: &Krate) -> syn::Result<TokenStream> {
    reject_validate_attr(&e.attrs, "an enum")?;
    if !e.generics.params.is_empty() {
        return Err(error_at(
            &e.generics,
            "`#[model]` enums cannot be generic",
            "use a plain enum with unit variants: `#[model] pub enum Status { Draft, Published }`",
        ));
    }
    let mut values = Vec::new();
    for v in &e.variants {
        reject_validate_attr(&v.attrs, "an enum variant")?;
        if !matches!(v.fields, Fields::Unit) {
            return Err(error_at(
                v,
                format!(
                    "`#[model]` enums take unit variants only; `{}` carries data",
                    v.ident
                ),
                "move the data into a `#[model]` struct (for example a `kind` enum field plus the data fields)",
            ));
        }
        values.push((v.ident.clone(), snake_case(&v.ident.to_string())));
    }

    let mut e = e;
    let rkyv = krate.rkyv();
    e.attrs
        .push(syn::parse_quote!(#[derive(#rkyv::Archive, #rkyv::Serialize, #rkyv::Deserialize)]));
    e.attrs.push(syn::parse_quote!(#[rkyv(crate = #rkyv)]));
    let name = &e.ident;
    let name_str = name.to_string();
    let serde = krate.serde();
    let validate = krate.validate();
    let json = quote!(#serde::json);
    let enc = values.iter().map(|(v, s)| quote!(#name::#v => #s,));
    let dec = values.iter().map(|(v, s)| quote!(#s => #name::#v,));
    let names: Vec<&String> = values.iter().map(|(_, s)| s).collect();
    let expected = names
        .iter()
        .map(|s| format!("`{s}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let shape = format!(
        "enum {name_str}{{{}}}",
        names
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );

    Ok(quote! {
        #e

        impl #json::Json for #name {
            fn encode_json(&self, __w: &mut #json::JsonWriter) {
                __w.string(match self { #(#enc)* });
            }

            fn decode_json(__r: &mut #json::JsonReader<'_>) -> ::core::result::Result<Self, #serde::DecodeError> {
                let __s = __r.read_string()?;
                ::core::result::Result::Ok(match &*__s {
                    #(#dec)*
                    __other => {
                        return ::core::result::Result::Err(
                            __r.error(::std::format!("unknown value `{}`; expected one of {}", __other, #expected)),
                        )
                    }
                })
            }
        }

        impl #validate::Validate for #name {
            fn validate_at(&self, _: &mut #validate::FieldPath, _: &mut #validate::Violations) {}
        }

        impl #validate::Schema for #name {
            fn schema(__reg: &mut #validate::SchemaRegistry) -> #validate::SchemaNode {
                __reg.define::<Self>(#name_str, |_| #validate::Definition::Enum(::std::vec![#(#names),*]))
            }
        }

        impl #serde::Fingerprint for #name {
            const FINGERPRINT: u64 = #serde::fnv1a(#shape.as_bytes());
        }
    })
}
