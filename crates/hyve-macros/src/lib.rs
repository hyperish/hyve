//! `hyve-macros` — Zero-dependency procedural macros (`#[model]`, `#[hyve::test]`, `#[server]`, `asset!`)
//! for the Hyperish family.

use proc_macro::{Delimiter, Literal, Span, TokenStream, TokenTree};
use std::str::FromStr;

fn compile_err(span: Span, message: &str) -> TokenStream {
    let lit = Literal::string(message);
    let ts: TokenStream = format!("::core::compile_error!({lit});")
        .parse()
        .unwrap_or_default();
    ts.into_iter()
        .map(|mut tt| {
            tt.set_span(span);
            tt
        })
        .collect()
}

fn snake_to_camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper_next = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn ts_to_compact_string(tokens: &[TokenTree]) -> String {
    tokens
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone)]
enum ValidateConstraintSpec {
    Len(usize, usize),
    Range(i64, i64),
    NonEmpty,
}

struct ParsedField {
    attrs_to_keep: Vec<TokenStream>,
    vis: TokenStream,
    name: String,
    ty: TokenStream,
    ty_string: String,
    is_pk: bool,
    is_option: bool,
    constraints: Vec<ValidateConstraintSpec>,
}

fn extract_leading_attrs(tokens: &[TokenTree]) -> (Vec<TokenStream>, usize) {
    let mut attrs = Vec::new();
    let mut idx = 0;
    while idx + 1 < tokens.len() {
        if let TokenTree::Punct(p) = &tokens[idx]
            && p.as_char() == '#'
            && let TokenTree::Group(g) = &tokens[idx + 1]
            && g.delimiter() == Delimiter::Bracket
        {
            attrs.push(g.stream());
            idx += 2;
            continue;
        }
        break;
    }
    (attrs, idx)
}

fn attr_head_ident(attr: &TokenStream) -> Option<String> {
    let first = attr.clone().into_iter().next()?;
    match first {
        TokenTree::Ident(id) => Some(id.to_string()),
        _ => None,
    }
}

fn parse_table_name_from_attr(attr: &TokenStream) -> Option<String> {
    let s = attr.to_string();
    let idx = s.find("table")?;
    let after = &s[idx + "table".len()..];
    let q1 = after.find('"')?;
    let rest = &after[q1 + 1..];
    let q2 = rest.find('"')?;
    Some(rest[..q2].to_owned())
}

fn parse_range_bounds(expr: &str) -> Result<(i64, i64), String> {
    let compact: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some((a, b)) = compact.split_once("..=") {
        let min: i64 = a
            .parse()
            .map_err(|_| format!("invalid range start '{a}' in `#[validate]`"))?;
        let max: i64 = b
            .parse()
            .map_err(|_| format!("invalid range end '{b}' in `#[validate]`"))?;
        Ok((min, max))
    } else if let Some((a, b)) = compact.split_once("..") {
        let min: i64 = a
            .parse()
            .map_err(|_| format!("invalid range start '{a}' in `#[validate]`"))?;
        let max: i64 = b
            .parse::<i64>()
            .map_err(|_| format!("invalid range end '{b}' in `#[validate]`"))?
            .saturating_sub(1);
        Ok((min, max))
    } else {
        Err(format!(
            "expected inclusive range like `1..=80`, found `{compact}`"
        ))
    }
}

fn parse_validate_attr(attr: &TokenStream) -> Result<Vec<ValidateConstraintSpec>, String> {
    let mut it = attr.clone().into_iter();
    let _ident = it.next();
    let Some(TokenTree::Group(group)) = it.next() else {
        return Err("expected `#[validate(...)]` parenthesized constraint".to_owned());
    };
    let mut specs = Vec::new();
    let inner = group.stream().to_string();
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part == "non_empty" {
            specs.push(ValidateConstraintSpec::NonEmpty);
        } else if let Some((lhs, rhs)) = part.split_once('=') {
            let key = lhs.trim();
            let (min, max) = parse_range_bounds(rhs.trim())?;
            match key {
                "len" => {
                    if min < 0 || max < 0 {
                        return Err(
                            "`#[validate(len = ...)]` bounds must be non-negative".to_owned()
                        );
                    }
                    specs.push(ValidateConstraintSpec::Len(min as usize, max as usize));
                }
                "range" => specs.push(ValidateConstraintSpec::Range(min, max)),
                other => {
                    return Err(format!(
                        "unsupported #[validate] rule '{other}'; supported: len, range, non_empty"
                    ));
                }
            }
        } else {
            return Err(format!("invalid #[validate] clause '{part}'"));
        }
    }
    Ok(specs)
}

fn parse_struct_fields(body: TokenStream) -> Result<Vec<ParsedField>, String> {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    let mut chunks: Vec<Vec<TokenTree>> = Vec::new();
    let mut cur = Vec::new();
    let mut angle_depth = 0usize;
    for tt in tokens {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == '<' => {
                angle_depth += 1;
                cur.push(tt);
            }
            TokenTree::Punct(p) if p.as_char() == '>' => {
                angle_depth = angle_depth.saturating_sub(1);
                cur.push(tt);
            }
            TokenTree::Punct(p) if p.as_char() == ',' && angle_depth == 0 => {
                if !cur.is_empty() {
                    chunks.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(tt),
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }

    let mut fields = Vec::new();
    for chunk in chunks {
        let (attrs, after_attrs) = extract_leading_attrs(&chunk);
        let mut attrs_to_keep = Vec::new();
        let mut is_pk = false;
        let mut constraints = Vec::new();

        for attr in attrs {
            match attr_head_ident(&attr).as_deref() {
                Some("pk") => is_pk = true,
                Some("validate") => {
                    constraints.extend(parse_validate_attr(&attr)?);
                }
                _ => attrs_to_keep.push(attr),
            }
        }

        let rest = &chunk[after_attrs..];
        let colon_pos = rest
            .iter()
            .position(|tt| matches!(tt, TokenTree::Punct(p) if p.as_char() == ':'))
            .ok_or_else(|| "expected `field: Type` inside #[model] struct".to_owned())?;
        if colon_pos == 0 {
            return Err("missing field name before `:` in #[model] struct".to_owned());
        }
        let name_idx = colon_pos - 1;
        let name = match &rest[name_idx] {
            TokenTree::Ident(id) => id.to_string(),
            other => return Err(format!("expected field identifier, found `{other}`")),
        };
        let vis: TokenStream = rest[..name_idx].iter().cloned().collect();
        let ty_tokens = &rest[colon_pos + 1..];
        let ty: TokenStream = ty_tokens.iter().cloned().collect();
        let ty_string = ts_to_compact_string(ty_tokens);
        let is_option = ty_string.starts_with("Option <") || ty_string.starts_with("Option<");

        fields.push(ParsedField {
            attrs_to_keep,
            vis,
            name,
            ty,
            ty_string,
            is_pk,
            is_option,
            constraints,
        });
    }
    Ok(fields)
}

/// Declares a unified Hyperish domain/config/API model.
///
/// Strips `#[validate(...)]` and `#[pk]` while preserving `#[table("...")]`,
/// derives `Clone, Debug, PartialEq`, and implements `BinaryEncode`, `BinaryDecode`,
/// `ToJson`, `FromJson`, `Validate`, and `HasSchema`.
#[proc_macro_attribute]
pub fn model(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = item.into_iter().collect();
    let (outer_attrs, after_attrs) = extract_leading_attrs(&tokens);

    let mut table_name: Option<String> = None;
    let mut has_derive = false;
    let mut kept_outer_attrs = String::new();

    for attr in &outer_attrs {
        let head = attr_head_ident(attr);
        if head.as_deref() == Some("derive") {
            has_derive = true;
        }
        if let Some(tbl) = parse_table_name_from_attr(attr) {
            table_name = Some(tbl);
        }
        kept_outer_attrs.push_str("#[");
        kept_outer_attrs.push_str(&attr.to_string());
        kept_outer_attrs.push_str("]\n");
    }

    let rest = &tokens[after_attrs..];
    let Some(struct_pos) = rest
        .iter()
        .position(|tt| matches!(tt, TokenTree::Ident(id) if id.to_string() == "struct"))
    else {
        return compile_err(Span::call_site(), "#[model] can only be applied to structs");
    };

    let vis: String = rest[..struct_pos]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");

    let Some(TokenTree::Ident(struct_ident)) = rest.get(struct_pos + 1) else {
        return compile_err(Span::call_site(), "expected struct name after `struct`");
    };
    let struct_name = struct_ident.to_string();

    let Some(TokenTree::Group(body_group)) = rest.get(struct_pos + 2) else {
        return compile_err(
            Span::call_site(),
            "#[model] requires a braced struct `struct Foo { ... }`",
        );
    };
    if body_group.delimiter() != Delimiter::Brace {
        return compile_err(
            body_group.span(),
            "#[model] requires a braced struct `struct Foo { ... }`",
        );
    }

    let fields = match parse_struct_fields(body_group.stream()) {
        Ok(f) => f,
        Err(msg) => return compile_err(Span::call_site(), &msg),
    };

    let derive_line = if has_derive {
        ""
    } else {
        "#[derive(Clone, Debug, PartialEq)]\n"
    };

    let mut struct_fields_code = String::new();
    let mut bin_encode_code = String::new();
    let mut bin_decode_fields = String::new();
    let mut json_encode_fields = String::new();
    let mut json_decode_fields = String::new();
    let mut validate_code = String::new();
    let mut schema_fields_code = String::new();

    for field in &fields {
        for attr in &field.attrs_to_keep {
            struct_fields_code.push_str("    #[");
            struct_fields_code.push_str(&attr.to_string());
            struct_fields_code.push_str("]\n");
        }
        struct_fields_code.push_str(&format!(
            "    {} {}: {},\n",
            field.vis, field.name, field.ty
        ));

        let fname = &field.name;
        let camel = snake_to_camel(fname);
        let fname_lit = Literal::string(fname).to_string();
        let camel_lit = Literal::string(&camel).to_string();
        let ty_lit = Literal::string(&field.ty_string).to_string();

        bin_encode_code.push_str(&format!(
            "        ::hyve::serde::BinaryEncode::binary_encode(&self.{fname}, out);\n"
        ));
        bin_decode_fields.push_str(&format!(
            "            {fname}: ::hyve::serde::BinaryDecode::binary_decode(reader)?,\n"
        ));

        json_encode_fields.push_str(&format!(
            "        fields.push(({camel_lit}.to_owned(), ::hyve::serde::ToJson::to_json(&self.{fname})));\n"
        ));

        if field.is_option {
            json_decode_fields.push_str(&format!(
                "            {fname}: match obj.get({camel_lit}).or_else(|| obj.get({fname_lit})) {{\n\
                 \x20               Some(v) => ::hyve::serde::FromJson::from_json(v)?,\n\
                 \x20               None => None,\n\
                 \x20           }},\n"
            ));
        } else {
            json_decode_fields.push_str(&format!(
                "            {fname}: match obj.get({camel_lit}).or_else(|| obj.get({fname_lit})) {{\n\
                 \x20               Some(v) => ::hyve::serde::FromJson::from_json(v)?,\n\
                 \x20               None => return Err(::hyve::HyveError::domain(\"invalid_request\", format!(\"missing field '{{}}'\", {fname_lit}))),\n\
                 \x20           }},\n"
            ));
        }

        let mut constraint_exprs = Vec::new();
        for c in &field.constraints {
            match c {
                ValidateConstraintSpec::Len(min, max) => {
                    validate_code.push_str(&format!(
                        "        ::hyve::validate::check_len({fname_lit}, &self.{fname}, {min}, {max}, &mut errors);\n"
                    ));
                    constraint_exprs.push(format!(
                        "::hyve::validate::FieldConstraint::Len {{ min: {min}, max: {max} }}"
                    ));
                }
                ValidateConstraintSpec::Range(min, max) => {
                    validate_code.push_str(&format!(
                        "        ::hyve::validate::check_range({fname_lit}, &self.{fname}, {min}, {max}, &mut errors);\n"
                    ));
                    constraint_exprs.push(format!(
                        "::hyve::validate::FieldConstraint::Range {{ min: {min}, max: {max} }}"
                    ));
                }
                ValidateConstraintSpec::NonEmpty => {
                    validate_code.push_str(&format!(
                        "        ::hyve::validate::check_len({fname_lit}, &self.{fname}, 1, usize::MAX, &mut errors);\n"
                    ));
                    constraint_exprs.push("::hyve::validate::FieldConstraint::NonEmpty".to_owned());
                }
            }
        }

        let is_pk = field.is_pk;
        let constraints_vec = constraint_exprs.join(", ");
        schema_fields_code.push_str(&format!(
            "            ::hyve::validate::FieldSchema {{\n\
             \x20               name: {fname_lit},\n\
             \x20               ty: {ty_lit},\n\
             \x20               constraints: vec![{constraints_vec}],\n\
             \x20               is_pk: {is_pk},\n\
             \x20           }},\n"
        ));
    }

    let struct_name_lit = Literal::string(&struct_name).to_string();
    let table_opt_expr = match &table_name {
        Some(t) => format!("Some({})", Literal::string(t)),
        None => "None".to_owned(),
    };

    let expanded = format!(
        "{derive_line}{kept_outer_attrs}{vis} struct {struct_name} {{\n{struct_fields_code}}}\n\n\
         impl ::hyve::serde::BinaryEncode for {struct_name} {{\n\
             fn binary_encode(&self, out: &mut ::std::vec::Vec<u8>) {{\n\
         {bin_encode_code}\
             }}\n\
         }}\n\n\
         impl ::hyve::serde::BinaryDecode for {struct_name} {{\n\
             fn binary_decode(reader: &mut ::hyve::serde::BinaryReader<'_>) -> ::hyve::HyveResult<Self> {{\n\
                 Ok(Self {{\n\
         {bin_decode_fields}\
                 }})\n\
             }}\n\
         }}\n\n\
         impl ::hyve::serde::ToJson for {struct_name} {{\n\
             fn to_json(&self) -> ::hyve::serde::JsonValue {{\n\
                 let mut fields = ::std::vec::Vec::new();\n\
         {json_encode_fields}\
                 ::hyve::serde::JsonValue::Object(fields)\n\
             }}\n\
         }}\n\n\
         impl ::hyve::serde::FromJson for {struct_name} {{\n\
             fn from_json(val: &::hyve::serde::JsonValue) -> ::hyve::HyveResult<Self> {{\n\
                 let obj = val;\n\
                 Ok(Self {{\n\
         {json_decode_fields}\
                 }})\n\
             }}\n\
         }}\n\n\
         impl ::hyve::validate::Validate for {struct_name} {{\n\
             fn validate(&self) -> ::core::result::Result<(), ::std::vec::Vec<::hyve::validate::ValidationError>> {{\n\
                 let mut errors = ::std::vec::Vec::new();\n\
         {validate_code}\
                 if errors.is_empty() {{ Ok(()) }} else {{ Err(errors) }}\n\
             }}\n\
         }}\n\n\
         impl ::hyve::validate::HasSchema for {struct_name} {{\n\
             fn schema() -> ::hyve::validate::ModelSchema {{\n\
                 ::hyve::validate::ModelSchema {{\n\
                     name: {struct_name_lit},\n\
                     table_name: {table_opt_expr},\n\
                     fields: vec![\n\
         {schema_fields_code}\
                     ],\n\
                 }}\n\
             }}\n\
         }}\n"
    );

    TokenStream::from_str(&expanded).unwrap_or_else(|e| {
        compile_err(
            Span::call_site(),
            &format!("internal #[model] expansion failure: {e}"),
        )
    })
}

/// Runs synchronous or asynchronous unit tests cleanly, supporting both `()` and `HyveResult<()>` returns.
#[proc_macro_attribute]
pub fn test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = item.into_iter().collect();
    let (attrs, after_attrs) = extract_leading_attrs(&tokens);
    let mut kept_attrs = String::new();
    for attr in attrs {
        kept_attrs.push_str("#[");
        kept_attrs.push_str(&attr.to_string());
        kept_attrs.push_str("]\n");
    }

    let rest = &tokens[after_attrs..];
    let Some(fn_pos) = rest
        .iter()
        .position(|tt| matches!(tt, TokenTree::Ident(id) if id.to_string() == "fn"))
    else {
        return compile_err(Span::call_site(), "#[hyve::test] must decorate a function");
    };

    let is_async = rest[..fn_pos]
        .iter()
        .any(|tt| matches!(tt, TokenTree::Ident(id) if id.to_string() == "async"));

    let Some(TokenTree::Ident(fn_name)) = rest.get(fn_pos + 1) else {
        return compile_err(Span::call_site(), "expected function name in #[hyve::test]");
    };

    let rest_str: TokenStream = rest.iter().cloned().collect();
    let call_expr = if is_async {
        format!("::hyve::api::block_on({fn_name}())")
    } else {
        format!("{fn_name}()")
    };

    let expanded = format!(
        "{kept_attrs}#[::core::prelude::v1::test]\n\
         fn {fn_name}() {{\n\
             {rest_str}\n\
             ::hyve::api::IntoTestOutcome::into_test_outcome({call_expr});\n\
         }}"
    );

    TokenStream::from_str(&expanded).unwrap_or_else(|e| {
        compile_err(
            Span::call_site(),
            &format!("internal #[hyve::test] error: {e}"),
        )
    })
}

struct ServerParam {
    is_server_ctx: bool,
    name: String,
    ty: String,
    payload_inner_ty: Option<String>,
}

fn split_params(params_stream: TokenStream) -> Result<Vec<Vec<TokenTree>>, String> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut angle = 0usize;
    for tt in params_stream {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == '<' => {
                angle += 1;
                cur.push(tt);
            }
            TokenTree::Punct(p) if p.as_char() == '>' => {
                angle = angle.saturating_sub(1);
                cur.push(tt);
            }
            TokenTree::Punct(p) if p.as_char() == ',' && angle == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(tt),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn unwrap_valid_inner(ty_str: &str) -> Option<String> {
    let trimmed = ty_str.trim();
    let valid_pos = trimmed.find("Valid")?;
    let before = trimmed[..valid_pos].trim();
    if !before.is_empty() && !before.ends_with("::") {
        return None;
    }
    let after = trimmed[valid_pos + "Valid".len()..].trim();
    let inner = after.strip_prefix('<')?.strip_suffix('>')?.trim();
    Some(inner.to_owned())
}

fn extract_hyve_result_inner(ret_str: &str) -> Option<String> {
    let trimmed = ret_str.trim();
    let arrow = trimmed.strip_prefix("->")?.trim();
    let lt = arrow.find('<')?;
    let gt = arrow.rfind('>')?;
    Some(arrow[lt + 1..gt].trim().to_owned())
}

/// Marks a shared inline server function (`#[server(POST "/projects")]`) that can be called
/// directly on the client while requiring explicit mounting on `orbic::component().handle(...)`.
#[proc_macro_attribute]
pub fn server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_str = attr.to_string();
    let parts: Vec<&str> = attr_str.split_whitespace().collect();
    if parts.len() < 2 {
        return compile_err(
            Span::call_site(),
            "expected #[server(METHOD \"/path\")], e.g. #[server(POST \"/projects\")]",
        );
    }
    let method = parts[0].trim().to_ascii_uppercase();
    let raw_path_part = parts[1..].join("");
    let Some(q1) = raw_path_part.find('"') else {
        return compile_err(
            Span::call_site(),
            "expected quoted path in #[server(METHOD \"/path\")]",
        );
    };
    let rest_path = &raw_path_part[q1 + 1..];
    let Some(q2) = rest_path.find('"') else {
        return compile_err(Span::call_site(), "unterminated path string in #[server]");
    };
    let path = rest_path[..q2].to_owned();

    let tokens: Vec<TokenTree> = item.into_iter().collect();
    let (outer_attrs, after_attrs) = extract_leading_attrs(&tokens);
    let mut kept_outer_attrs = String::new();
    for a in outer_attrs {
        kept_outer_attrs.push_str("#[");
        kept_outer_attrs.push_str(&a.to_string());
        kept_outer_attrs.push_str("]\n");
    }

    let rest = &tokens[after_attrs..];
    let Some(fn_pos) = rest
        .iter()
        .position(|tt| matches!(tt, TokenTree::Ident(id) if id.to_string() == "fn"))
    else {
        return compile_err(
            Span::call_site(),
            "#[server] must decorate an async function",
        );
    };
    let vis_tokens: String = rest[..fn_pos]
        .iter()
        .filter(|tt| !matches!(tt, TokenTree::Ident(id) if id.to_string() == "async"))
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");

    let Some(TokenTree::Ident(fn_ident)) = rest.get(fn_pos + 1) else {
        return compile_err(Span::call_site(), "expected function name after `fn`");
    };
    let fn_name = fn_ident.to_string();

    let Some(TokenTree::Group(params_group)) = rest.get(fn_pos + 2) else {
        return compile_err(Span::call_site(), "expected parameter list in #[server]");
    };
    if params_group.delimiter() != Delimiter::Parenthesis {
        return compile_err(params_group.span(), "expected `(...)` parameter list");
    }

    let Some(body_idx) = rest
        .iter()
        .rposition(|tt| matches!(tt, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace))
    else {
        return compile_err(
            Span::call_site(),
            "expected function body block in #[server]",
        );
    };
    let ret_tokens: TokenStream = rest[fn_pos + 3..body_idx].iter().cloned().collect();
    let ret_str = ret_tokens.to_string();
    let res_inner_ty = extract_hyve_result_inner(&ret_str).unwrap_or_else(|| "()".to_owned());

    let body_stream = match &rest[body_idx] {
        TokenTree::Group(g) => g.stream(),
        _ => unreachable!(),
    };

    let param_chunks = match split_params(params_group.stream()) {
        Ok(c) => c,
        Err(e) => return compile_err(Span::call_site(), &e),
    };

    let mut params = Vec::new();
    for chunk in param_chunks {
        let (p_attrs, p_after) = extract_leading_attrs(&chunk);
        let is_server_ctx = p_attrs
            .iter()
            .any(|a| attr_head_ident(a).as_deref() == Some("server_ctx"));
        let pre = &chunk[p_after..];
        let Some(colon) = pre
            .iter()
            .position(|tt| matches!(tt, TokenTree::Punct(p) if p.as_char() == ':'))
            .filter(|&c| c > 0)
        else {
            return compile_err(Span::call_site(), "expected `param: Type` in #[server]");
        };
        let p_name = pre[colon - 1].to_string();
        let p_ty: TokenStream = pre[colon + 1..].iter().cloned().collect();
        let p_ty_str = p_ty.to_string();
        let payload_inner_ty = if !is_server_ctx {
            unwrap_valid_inner(&p_ty_str)
        } else {
            None
        };
        params.push(ServerParam {
            is_server_ctx,
            name: p_name,
            ty: p_ty_str,
            payload_inner_ty,
        });
    }

    let client_payload_param = params.iter().find(|p| !p.is_server_ctx);
    let (client_arg_name, client_arg_ty, needs_valid_wrap) = match client_payload_param {
        Some(p) => {
            if let Some(inner) = &p.payload_inner_ty {
                (p.name.as_str(), inner.clone(), true)
            } else {
                (p.name.as_str(), p.ty.clone(), false)
            }
        }
        None => ("_unit", "()".to_owned(), false),
    };

    let mut server_resolve_args = String::new();
    let mut server_call_args = Vec::new();
    for p in &params {
        if p.is_server_ctx {
            let p_name = &p.name;
            let p_ty = &p.ty;
            server_resolve_args.push_str(&format!("let {p_name}: {p_ty} = __ctx.resolve()?;\n"));
            server_call_args.push(p_name.clone());
        } else if needs_valid_wrap {
            let p_name = &p.name;
            server_resolve_args.push_str(&format!(
                "let __raw_input: {client_arg_ty} = __ctx.decode_payload()?;\n\
                 let {p_name} = ::hyve::validate::Valid::new(__raw_input)?;\n"
            ));
            server_call_args.push(p_name.clone());
        } else {
            let p_name = &p.name;
            server_resolve_args.push_str(&format!(
                "let {p_name}: {client_arg_ty} = __ctx.decode_payload()?;\n"
            ));
            server_call_args.push(p_name.clone());
        }
    }

    let method_lit = Literal::string(&method).to_string();
    let path_lit = Literal::string(&path).to_string();
    let fn_name_lit = Literal::string(&fn_name).to_string();
    let future_ident = format!("{fn_name}__ServerCall");
    let endpoint_fn = format!("{fn_name}_endpoint");
    let server_inner_fn = format!("__hyve_server_impl_{fn_name}");
    let body_str = body_stream.to_string();

    let server_inner_params: String = params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.ty))
        .collect::<Vec<_>>()
        .join(", ");
    let server_call_arg_list = server_call_args.join(", ");

    let expanded = format!(
        "#[allow(non_camel_case_types, missing_docs)]\n\
         pub struct {future_ident} {{\n\
             inner: ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = ::hyve::HyveResult<{res_inner_ty}>> + 'static>>,\n\
         }}\n\n\
         impl ::std::future::Future for {future_ident} {{\n\
             type Output = ::hyve::HyveResult<{res_inner_ty}>;\n\
             fn poll(mut self: ::std::pin::Pin<&mut Self>, cx: &mut ::std::task::Context<'_>) -> ::std::task::Poll<Self::Output> {{\n\
                 self.inner.as_mut().poll(cx)\n\
             }}\n\
         }}\n\n\
         impl ::hyve::api::IntoEndpointDescriptor for {future_ident} {{\n\
             fn endpoint_descriptor() -> ::hyve::api::EndpointDescriptor {{\n\
                 {endpoint_fn}()\n\
             }}\n\
         }}\n\n\
         #[allow(missing_docs)]\n\
         {vis_tokens} fn {endpoint_fn}() -> ::hyve::api::EndpointDescriptor {{\n\
             #[cfg(not(target_arch = \"wasm32\"))]\n\
             async fn {server_inner_fn}({server_inner_params}) {ret_str} {{\n\
                 {body_str}\n\
             }}\n\
             ::hyve::api::EndpointDescriptor::new(\n\
                 {fn_name_lit},\n\
                 {method_lit},\n\
                 {path_lit},\n\
                 ::std::sync::Arc::new(|__ctx| {{\n\
                     ::std::boxed::Box::pin(async move {{\n\
                         #[cfg(not(target_arch = \"wasm32\"))]\n\
                         {{\n\
                             {server_resolve_args}\n\
                             let out = {server_inner_fn}({server_call_arg_list}).await?;\n\
                             __ctx.encode_response(&out)\n\
                         }}\n\
                         #[cfg(target_arch = \"wasm32\")]\n\
                         {{\n\
                             let _ = __ctx;\n\
                             Err(::hyve::HyveError::internal(\"server handler stripped on wasm target\"))\n\
                         }}\n\
                     }})\n\
                 }}),\n\
             )\n\
         }}\n\n\
         {kept_outer_attrs}\
         {vis_tokens} fn {fn_name}({client_arg_name}: {client_arg_ty}) -> {future_ident} {{\n\
             {future_ident} {{\n\
                 inner: ::std::boxed::Box::pin(async move {{\n\
                     ::hyve::api::dispatch_server_call::<{client_arg_ty}, {res_inner_ty}>(\n\
                         {method_lit},\n\
                         {path_lit},\n\
                         &{client_arg_name},\n\
                     ).await\n\
                 }}),\n\
             }}\n\
         }}"
    );

    TokenStream::from_str(&expanded).unwrap_or_else(|e| {
        compile_err(
            Span::call_site(),
            &format!("internal #[server] expansion failure: {e}"),
        )
    })
}

/// Validates and bundles an asset path at compile time, returning a [`::hyve::api::AssetRef`].
#[proc_macro]
pub fn asset(input: TokenStream) -> TokenStream {
    let raw = input.to_string();
    let trimmed = raw.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 3) {
        return compile_err(
            Span::call_site(),
            "asset!(...) requires a non-empty string literal starting with \"/\", e.g. asset!(\"/shaders/pbr.wgsl\")",
        );
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if !inner.starts_with('/') || inner.len() < 2 {
        return compile_err(
            Span::call_site(),
            "asset! path must start with '/' and name a resource (e.g. \"/shaders/pbr.wgsl\")",
        );
    }

    let lit = Literal::string(inner).to_string();
    let expanded = format!("::hyve::api::AssetRef::new({lit}, env!(\"CARGO_MANIFEST_DIR\"))");
    TokenStream::from_str(&expanded)
        .unwrap_or_else(|e| compile_err(Span::call_site(), &format!("internal asset! error: {e}")))
}
