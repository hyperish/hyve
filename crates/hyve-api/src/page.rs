//! Cursor pagination shapes (`hyperish/docs` `03-api-protocol.md` §4–§5).
//!
//! These are written out by hand in the exact form `#[model]` generates (this
//! crate cannot use the macro); a facade test compares them with `#[model]`
//! twins so they cannot drift.

use hyve_serde::__private::rkyv;
use hyve_serde::json::{Json, JsonReader, JsonWriter};
use hyve_serde::{DecodeError, Fingerprint, fnv1a, mix};
use hyve_validate::{
    Definition, FieldPath, FieldSchema, Schema, SchemaNode, SchemaRegistry, Validate, Violations,
    generic_name,
};

/// One page of a list. Standard-rail adapters map it to
/// `{ "data": [...], "page": { "nextCursor": ..., "hasMore": ... } }`.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
pub struct Page<T> {
    /// The items of this page.
    pub items: Vec<T>,
    /// Cursor for the next page, if any.
    pub next_cursor: Option<String>,
    /// Whether more items follow.
    pub has_more: bool,
}

impl<T> Page<T> {
    /// A page with a continuation cursor (`has_more` follows from it).
    pub fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        let has_more = next_cursor.is_some();
        Page {
            items,
            next_cursor,
            has_more,
        }
    }

    /// The last page.
    pub fn last(items: Vec<T>) -> Self {
        Page::new(items, None)
    }
}

/// Cursor request parameters.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
pub struct CursorParams {
    /// Continue after this cursor (the previous page's `next_cursor`).
    pub after: Option<String>,
    /// Requested page size; servers clamp it to their documented maximum.
    pub limit: Option<u32>,
}

// ----- the expansion `#[model]` produces, by hand -----

impl<T: Json> Json for Page<T> {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("items");
        Json::encode_json(&self.items, w);
        w.key("nextCursor");
        Json::encode_json(&self.next_cursor, w);
        w.key("hasMore");
        Json::encode_json(&self.has_more, w);
        w.end_object();
    }

    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let mut f0: Option<Vec<T>> = None;
        let mut f1: Option<Option<String>> = None;
        let mut f2: Option<bool> = None;
        while let Some(key) = r.next_key()? {
            match &*key {
                "items" => {
                    if f0.is_some() {
                        return Err(r.duplicate_field("items"));
                    }
                    f0 = Some(Json::decode_json(r)?);
                }
                "nextCursor" => {
                    if f1.is_some() {
                        return Err(r.duplicate_field("nextCursor"));
                    }
                    f1 = Some(Json::decode_json(r)?);
                }
                "hasMore" => {
                    if f2.is_some() {
                        return Err(r.duplicate_field("hasMore"));
                    }
                    f2 = Some(Json::decode_json(r)?);
                }
                _ => r.skip_value()?,
            }
        }
        Ok(Page {
            items: match f0 {
                Some(v) => v,
                None => <Vec<T> as Json>::missing().ok_or_else(|| r.missing_field("items"))?,
            },
            next_cursor: match f1 {
                Some(v) => v,
                None => <Option<String> as Json>::missing()
                    .ok_or_else(|| r.missing_field("nextCursor"))?,
            },
            has_more: match f2 {
                Some(v) => v,
                None => <bool as Json>::missing().ok_or_else(|| r.missing_field("hasMore"))?,
            },
        })
    }

    /// `{"data": [...], "page": {"nextCursor": ..., "hasMore": ...}}`
    /// (`hyperish/docs` `03-api-protocol.md` §4).
    fn encode_data(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("data");
        Json::encode_json(&self.items, w);
        w.key("page");
        w.begin_object();
        w.key("nextCursor");
        Json::encode_json(&self.next_cursor, w);
        w.key("hasMore");
        Json::encode_json(&self.has_more, w);
        w.end_object();
        w.end_object();
    }

    fn decode_data(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let (mut items, mut page) = (None, None);
        while let Some(key) = r.next_key()? {
            match &*key {
                "data" if items.is_none() => items = Some(<Vec<T> as Json>::decode_json(r)?),
                "page" if page.is_none() => {
                    r.begin_object()?;
                    let (mut next, mut more) = (None, None);
                    while let Some(key) = r.next_key()? {
                        match &*key {
                            "nextCursor" if next.is_none() => {
                                next = Some(<Option<String> as Json>::decode_json(r)?)
                            }
                            "hasMore" if more.is_none() => more = Some(bool::decode_json(r)?),
                            "nextCursor" | "hasMore" => return Err(r.duplicate_field(&key)),
                            _ => r.skip_value()?,
                        }
                    }
                    page = Some((
                        next.unwrap_or(None),
                        more.ok_or_else(|| r.missing_field("hasMore"))?,
                    ));
                }
                "data" | "page" => return Err(r.duplicate_field(&key)),
                _ => r.skip_value()?,
            }
        }
        let items = items.ok_or_else(|| r.missing_field("data"))?;
        let (next_cursor, has_more) = page.ok_or_else(|| r.missing_field("page"))?;
        Ok(Page {
            items,
            next_cursor,
            has_more,
        })
    }
}

impl<T: Validate> Validate for Page<T> {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        let mark = path.mark();
        path.push_field("items");
        Validate::validate_at(&self.items, path, out);
        path.truncate(mark);
        path.push_field("nextCursor");
        Validate::validate_at(&self.next_cursor, path, out);
        path.truncate(mark);
        path.push_field("hasMore");
        Validate::validate_at(&self.has_more, path, out);
        path.truncate(mark);
    }
}

impl<T: Schema + 'static> Schema for Page<T> {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        let name = generic_name("Page", &[<T as Schema>::schema(r)]);
        r.define::<Self>(name, |r| {
            Definition::Object(vec![
                FieldSchema {
                    name: "items",
                    json_name: "items",
                    schema: <Vec<T> as Schema>::schema(r),
                    required: !<Vec<T> as Schema>::optional(),
                    constraints: vec![],
                },
                FieldSchema {
                    name: "next_cursor",
                    json_name: "nextCursor",
                    schema: <Option<String> as Schema>::schema(r),
                    required: !<Option<String> as Schema>::optional(),
                    constraints: vec![],
                },
                FieldSchema {
                    name: "has_more",
                    json_name: "hasMore",
                    schema: <bool as Schema>::schema(r),
                    required: !<bool as Schema>::optional(),
                    constraints: vec![],
                },
            ])
        })
    }
}

impl<T: Fingerprint> Fingerprint for Page<T> {
    const FINGERPRINT: u64 = mix(
        fnv1a(b"Page{items,nextCursor,hasMore}"),
        mix(
            <Vec<T> as Fingerprint>::FINGERPRINT,
            mix(
                <Option<String> as Fingerprint>::FINGERPRINT,
                <bool as Fingerprint>::FINGERPRINT,
            ),
        ),
    );
}

impl Json for CursorParams {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("after");
        Json::encode_json(&self.after, w);
        w.key("limit");
        Json::encode_json(&self.limit, w);
        w.end_object();
    }

    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let mut f0: Option<Option<String>> = None;
        let mut f1: Option<Option<u32>> = None;
        while let Some(key) = r.next_key()? {
            match &*key {
                "after" => {
                    if f0.is_some() {
                        return Err(r.duplicate_field("after"));
                    }
                    f0 = Some(Json::decode_json(r)?);
                }
                "limit" => {
                    if f1.is_some() {
                        return Err(r.duplicate_field("limit"));
                    }
                    f1 = Some(Json::decode_json(r)?);
                }
                _ => r.skip_value()?,
            }
        }
        Ok(CursorParams {
            after: match f0 {
                Some(v) => v,
                None => {
                    <Option<String> as Json>::missing().ok_or_else(|| r.missing_field("after"))?
                }
            },
            limit: match f1 {
                Some(v) => v,
                None => <Option<u32> as Json>::missing().ok_or_else(|| r.missing_field("limit"))?,
            },
        })
    }
}

impl Validate for CursorParams {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        let mark = path.mark();
        path.push_field("after");
        Validate::validate_at(&self.after, path, out);
        path.truncate(mark);
        path.push_field("limit");
        Validate::validate_at(&self.limit, path, out);
        path.truncate(mark);
    }
}

impl Schema for CursorParams {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        r.define::<Self>("CursorParams", |r| {
            Definition::Object(vec![
                FieldSchema {
                    name: "after",
                    json_name: "after",
                    schema: <Option<String> as Schema>::schema(r),
                    required: !<Option<String> as Schema>::optional(),
                    constraints: vec![],
                },
                FieldSchema {
                    name: "limit",
                    json_name: "limit",
                    schema: <Option<u32> as Schema>::schema(r),
                    required: !<Option<u32> as Schema>::optional(),
                    constraints: vec![],
                },
            ])
        })
    }
}

impl Fingerprint for CursorParams {
    const FINGERPRINT: u64 = mix(
        fnv1a(b"CursorParams{after,limit}"),
        mix(
            <Option<String> as Fingerprint>::FINGERPRINT,
            <Option<u32> as Fingerprint>::FINGERPRINT,
        ),
    );
}
