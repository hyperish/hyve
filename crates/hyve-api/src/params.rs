//! Text parameters (path placeholders, query strings) to and from models,
//! shared by `orbic` handlers (`Path<T>`, `Query<T>`), `serge` route
//! components and client transports.

use hyve_core::{HyveError, Value};
use hyve_serde::{Json, from_json, to_json};
use hyve_validate::{Definition, FieldSchema, Schema, SchemaNode, SchemaRegistry};

/// Splits and percent-decodes `a=1&b=x` (`+` is a space).
pub fn parse_query(query: &str) -> Result<Vec<(String, String)>, HyveError> {
    let mut out = Vec::new();
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        out.push((percent_decode(k)?, percent_decode(v)?));
    }
    Ok(out)
}

/// Decodes `%XX` escapes and `+`.
pub fn percent_decode(s: &str) -> Result<String, HyveError> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = s
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                let Some(byte) = hex else {
                    return Err(HyveError::invalid_request(format!(
                        "invalid percent-encoding in `{s}`"
                    )));
                };
                out.push(byte);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out)
        .map_err(|_| HyveError::invalid_request(format!("`{s}` is not valid UTF-8 once decoded")))
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Builds `T` from text parameters, guided by its schema. A model takes its
/// fields by JSON name (lists repeat the parameter); any other type reads
/// the single parameter `single` (or the only one present). `what` names the
/// source in errors (`"path"`, `"query"`).
pub fn from_params<T: Json + Schema>(
    pairs: &[(String, String)],
    single: Option<&str>,
    what: &str,
) -> Result<T, HyveError> {
    let mut reg = SchemaRegistry::new();
    let node = T::schema(&mut reg);
    let value = match object_fields(&reg, &node) {
        Some(fields) => {
            let mut map = Vec::new();
            for f in fields {
                let values: Vec<&str> = pairs
                    .iter()
                    .filter(|(k, _)| k == f.json_name)
                    .map(|(_, v)| v.as_str())
                    .collect();
                if values.is_empty() {
                    continue;
                }
                map.push((
                    f.json_name.to_owned(),
                    param_value(&f.schema, &values, f.json_name, what)?,
                ));
            }
            Value::Map(map)
        }
        None => {
            let values: Vec<&str> = match single {
                Some(name) => pairs
                    .iter()
                    .filter(|(k, _)| k == name)
                    .map(|(_, v)| v.as_str())
                    .collect(),
                None => pairs.iter().map(|(_, v)| v.as_str()).collect(),
            };
            if values.is_empty() {
                return Err(HyveError::invalid_request(format!(
                    "missing {what} parameter `{}`",
                    single.unwrap_or("value")
                )));
            }
            param_value(&node, &values, single.unwrap_or("value"), what)?
        }
    };
    from_json::<T>(&to_json(&value)).map_err(|e| {
        let at = e.path().strip_prefix("$.").unwrap_or(e.path());
        HyveError::invalid_request(format!("invalid {what} parameter `{at}`: {}", e.message()))
    })
}

fn object_fields<'r>(reg: &'r SchemaRegistry, node: &SchemaNode) -> Option<&'r [FieldSchema]> {
    match node {
        SchemaNode::Ref(name) => match reg.get(name) {
            Some(Definition::Object(fields)) => Some(fields),
            _ => None,
        },
        _ => None,
    }
}

fn param_value(
    node: &SchemaNode,
    values: &[&str],
    name: &str,
    what: &str,
) -> Result<Value, HyveError> {
    let bad = |expected: &str, v: &str| {
        HyveError::invalid_request(format!(
            "invalid {what} parameter `{name}`: expected {expected}, found `{v}`"
        ))
    };
    if let SchemaNode::List { items, .. } = node {
        return values
            .iter()
            .map(|v| param_value(items, &[v], name, what))
            .collect::<Result<_, _>>()
            .map(Value::List);
    }
    let [v] = values else {
        return Err(HyveError::invalid_request(format!(
            "{what} parameter `{name}` is repeated"
        )));
    };
    Ok(match node {
        SchemaNode::Nullable(inner) => return param_value(inner, values, name, what),
        SchemaNode::Bool => match *v {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(bad("`true` or `false`", v)),
        },
        SchemaNode::Integer { signed: true, .. } => v
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| bad("an integer", v))?,
        SchemaNode::Integer { signed: false, .. } => match v.parse::<u64>() {
            Ok(n) => i64::try_from(n).map(Value::Int).unwrap_or(Value::UInt(n)),
            Err(_) => return Err(bad("a non-negative integer", v)),
        },
        SchemaNode::Float { .. } => v
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| bad("a number", v))?,
        _ => Value::String((*v).to_owned()),
    })
}

/// Argument names travel as JSON names (`owner_id` → `ownerId`), like the
/// fields of the generated request model.
pub fn arg_json_name(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_are_split_and_decoded() {
        let pairs = parse_query("a=1&b=x+y%2Fz&&c&d=%E2%9C%93").unwrap();
        let pairs: Vec<_> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(pairs, [("a", "1"), ("b", "x y/z"), ("c", ""), ("d", "✓")]);
        assert!(parse_query("a=%zz").is_err());
        assert_eq!(percent_encode("a b/✓"), "a%20b%2F%E2%9C%93");
    }

    #[test]
    fn scalars_and_arg_names() {
        assert_eq!(
            from_params::<u32>(&[("id".into(), "7".into())], Some("id"), "path").unwrap(),
            7
        );
        assert!(from_params::<u32>(&[("id".into(), "x".into())], Some("id"), "path").is_err());
        assert_eq!(arg_json_name("owner_id"), "ownerId");
    }
}
