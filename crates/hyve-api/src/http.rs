//! The client side of the HTTP translation (`docs/03-adapters-and-pipeline.md`
//! §3.3, §3.4) as pure functions: a [`Call`] becomes a request description,
//! and a response's status, media type, `Retry-After` presence and body
//! become a [`Reply`] or the error. Remote transports and test clients share
//! it so they cannot drift; sending the bytes is theirs.

use hyve_core::{ErrorKind, HyveError, HyveResult, Value};
use hyve_serde::{AlignedBytes, Binary as _, from_json, to_json};

use crate::params::{arg_json_name, percent_encode};
use crate::{
    Call, ErrorBody, ErrorWire, HEADER_FN, HEADER_SCHEMA, MEDIA_TYPE_BINARY, MEDIA_TYPE_JSON,
    Method, Rail, Reply, ServerFnDescriptor, errors,
};

/// What to send for one call on one rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// The HTTP method.
    pub method: Method,
    /// The path with placeholders filled (and, for bodyless standard-rail
    /// calls, the query string), relative to where the server functions are
    /// mounted.
    pub path: String,
    /// Headers to set (`Content-Type`, and on the fast rail the function id
    /// and schema fingerprint). Credentials are the transport's.
    pub headers: Vec<(&'static str, String)>,
    /// The body (empty when there is none).
    pub body: Vec<u8>,
}

/// Encodes `call` for `rail` (`Fast` or `Standard`). `pattern` is the bound
/// path (`/projects/{id}`) as the caller reaches it; `fingerprint` replaces the
/// schema fingerprint (tests of the mismatch fallback).
pub fn encode(
    call: &Call,
    rail: Rail,
    pattern: &str,
    fingerprint: Option<u64>,
) -> HyveResult<HttpRequest> {
    let d = call.descriptor();
    let model: Value = from_json(&call.encode_json())
        .map_err(|e| HyveError::internal(format!("the request model is not JSON: {e}")))?;
    let params = named_values(d, model);
    if rail == Rail::Fast {
        let fingerprint = fingerprint.unwrap_or(d.fingerprint);
        return Ok(HttpRequest {
            method: Method::Post,
            path: fill(pattern, &params, false)?,
            headers: vec![
                ("Content-Type", MEDIA_TYPE_BINARY.to_owned()),
                (HEADER_FN, d.id.to_owned()),
                (HEADER_SCHEMA, format!("{fingerprint:016x}")),
            ],
            body: call.encode_binary()?.into_vec(),
        });
    }
    let bodyless = matches!(d.method, Method::Get | Method::Delete);
    let body = if bodyless || d.args.is_empty() {
        Vec::new()
    } else {
        call.encode_json().into_bytes()
    };
    let headers = if body.is_empty() {
        Vec::new()
    } else {
        vec![("Content-Type", MEDIA_TYPE_JSON.to_owned())]
    };
    Ok(HttpRequest {
        method: d.method,
        path: fill(pattern, &params, bodyless)?,
        headers,
        body,
    })
}

/// Whether a fast-rail answer means "use the standard rail" (§3.3: `415`,
/// whatever the code); the transport then sends the call once more there.
pub fn wants_standard_rail(status: u16) -> bool {
    status == 415
}

/// The parts of a response a transport reads.
#[derive(Debug, Clone, Copy)]
pub struct HttpAnswer<'a> {
    /// The status code.
    pub status: u16,
    /// The `Content-Type` value, if any.
    pub content_type: Option<&'a str>,
    /// Whether `Retry-After` is present (it marks retryable errors).
    pub retry_after: bool,
    /// The body.
    pub body: &'a [u8],
}

/// Reads an answer as a transport does (§3.3, §3.4).
pub fn decode(answer: HttpAnswer<'_>) -> HyveResult<Reply> {
    let status = answer.status;
    let success = (200..300).contains(&status);
    let binary = answer
        .content_type
        .is_some_and(|c| c.split(';').next().unwrap_or("").trim() == MEDIA_TYPE_BINARY);
    if binary {
        let bytes = AlignedBytes::copy_from(answer.body);
        if success {
            return Ok(Reply::Binary(bytes));
        }
        let wire = ErrorWire::from_binary(&bytes)
            .map_err(|e| errors::decode_failed(format!("unreadable binary error: {e}")))?;
        return Err(wire.into());
    }
    let text = std::str::from_utf8(answer.body)
        .map_err(|_| errors::decode_failed("the answer is not UTF-8"))?;
    if status == 204 {
        return Ok(Reply::NoContent);
    }
    if success {
        return Ok(Reply::Json(text.to_owned()));
    }
    let kind = if (400..500).contains(&status) {
        ErrorKind::Domain
    } else if status == 503 {
        ErrorKind::Infra
    } else {
        ErrorKind::Internal
    };
    let envelope: Value = from_json(text)
        .map_err(|e| errors::decode_failed(format!("not an error envelope ({status}): {e}")))?;
    let body = envelope
        .get("error")
        .map(|e| from_json::<ErrorBody>(&to_json(e)))
        .ok_or_else(|| errors::decode_failed(format!("not an error envelope ({status})")))?
        .map_err(|e| errors::decode_failed(format!("not an error envelope ({status}): {e}")))?;
    Err(body.into_error(kind, answer.retry_after))
}

/// The request model as named values, for path placeholders and (for
/// `GET`/`DELETE`) the query string.
fn named_values(d: &ServerFnDescriptor, model: Value) -> Vec<(String, Value)> {
    match model {
        _ if d.args.is_empty() => Vec::new(),
        Value::Map(fields) => fields,
        single => vec![(arg_json_name(d.args[0]), single)],
    }
}

fn fill(pattern: &str, values: &[(String, Value)], with_query: bool) -> HyveResult<String> {
    let mut path = String::new();
    let mut used = Vec::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        path.push_str(&rest[..start]);
        let end = rest[start..].find('}').map_or(rest.len(), |e| start + e);
        let name = rest[start + 1..end].trim_start_matches('*');
        let value = values.iter().find(|(k, _)| k == name).ok_or_else(|| {
            HyveError::invalid_request(format!("the call has no `{name}` for the path"))
        })?;
        path.push_str(&percent_encode(&scalar(&value.1)?));
        used.push(name.to_owned());
        rest = &rest[(end + 1).min(rest.len())..];
    }
    path.push_str(rest);
    if with_query {
        let mut query = Vec::new();
        for (k, v) in values.iter().filter(|(k, _)| !used.contains(k)) {
            match v {
                Value::Null => {}
                Value::List(items) => {
                    for item in items {
                        query.push(format!(
                            "{}={}",
                            percent_encode(k),
                            percent_encode(&scalar(item)?)
                        ));
                    }
                }
                v => query.push(format!(
                    "{}={}",
                    percent_encode(k),
                    percent_encode(&scalar(v)?)
                )),
            }
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
    }
    Ok(path)
}

fn scalar(v: &Value) -> HyveResult<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Bool(_) | Value::Int(_) | Value::UInt(_) | Value::Float(_) => Ok(to_json(v)),
        other => Err(HyveError::invalid_request(format!(
            "{other:?} cannot travel in a path or query string"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(
        status: u16,
        content_type: Option<&'static str>,
        body: &'static str,
    ) -> HttpAnswer<'static> {
        HttpAnswer {
            status,
            content_type,
            retry_after: false,
            body: body.as_bytes(),
        }
    }

    #[test]
    fn answers_restore_like_a_transport() {
        assert!(matches!(
            decode(answer(204, None, "")),
            Ok(Reply::NoContent)
        ));
        assert!(matches!(
            decode(answer(200, Some("application/json"), "{\"data\":1}")),
            Ok(Reply::Json(_))
        ));
        let e = decode(HttpAnswer {
            retry_after: true,
            ..answer(
                409,
                Some("application/json"),
                r#"{"error":{"code":"request_in_progress","message":"wait"}}"#,
            )
        })
        .unwrap_err();
        assert_eq!(
            (e.code(), e.kind(), e.is_retryable()),
            ("request_in_progress", ErrorKind::Domain, true)
        );
        let e = decode(answer(502, Some("text/plain"), "bad gateway")).unwrap_err();
        assert_eq!(e.code(), crate::codes::DECODE_FAILED);
        assert!(wants_standard_rail(415) && !wants_standard_rail(400));
    }
}
