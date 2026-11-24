//! `hyve-validate` — validation and schema metadata shared by every layer
//! (`docs/02-architecture.md` §2.3).
//!
//! - [`Validate`]: `#[model]` implements it from `#[validate(...)]` field rules
//!   (Phase 1 rules: `len = <range>` and `range = <range>`, see [`rules`]).
//!   Every field is also validated recursively, so nested models are checked.
//! - [`Valid<T>`]: proof that a value passed validation. Handlers that take
//!   `Valid<T>` never see invalid input; servers always re-validate at the
//!   trust boundary.
//! - [`Schema`]: type and constraint metadata used to generate API documents
//!   (`orbic-openapi`) without either side knowing about the other.

pub mod rules;
mod schema;

pub use schema::{Constraint, Definition, FieldSchema, Schema, SchemaNode, SchemaRegistry};

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::Deref;

use hyve_core::{HyveError, Id, Timestamp, Ulid, Value};

/// One failed rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// Where (`$.items[2].name`, using JSON field names).
    pub path: String,
    /// Which rule (`len`, `range`).
    pub rule: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Rule parameters (`min`, `max`), merged into the error details.
    pub params: Value,
}

/// All failed rules of one validation run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Violations(Vec<Violation>);

impl Violations {
    /// No violations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a violation.
    pub fn push(&mut self, v: Violation) {
        self.0.push(v);
    }

    /// Whether nothing failed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The violations in the order they were found.
    pub fn iter(&self) -> impl Iterator<Item = &Violation> {
        self.0.iter()
    }

    /// `invalid_request` with `details.fields = [{path, rule, message, ...params}]`.
    pub fn into_error(self) -> HyveError {
        let first = self
            .0
            .first()
            .map(|v| format!("{}: {}", v.path, v.message))
            .unwrap_or_default();
        let fields: Vec<Value> = self
            .0
            .into_iter()
            .map(|v| {
                let mut m = Value::map();
                m.insert("path", v.path);
                m.insert("rule", v.rule);
                m.insert("message", v.message);
                if let Value::Map(params) = v.params {
                    for (k, val) in params {
                        m.insert(k, val);
                    }
                }
                m
            })
            .collect();
        HyveError::invalid_request(format!("validation failed: {first}"))
            .with_detail("fields", Value::List(fields))
    }
}

impl fmt::Display for Violations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, v) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{}: {}", v.path, v.message)?;
        }
        Ok(())
    }
}

impl From<Violations> for HyveError {
    fn from(v: Violations) -> Self {
        v.into_error()
    }
}

/// A path being built while validating nested values.
#[derive(Debug, Clone)]
pub struct FieldPath(String);

impl FieldPath {
    /// The root (`$`).
    pub fn root() -> Self {
        FieldPath(String::from("$"))
    }

    /// Current length, to [`truncate`](Self::truncate) back to after a child.
    pub fn mark(&self) -> usize {
        self.0.len()
    }

    /// Appends `.name`.
    pub fn push_field(&mut self, json_name: &str) {
        self.0.push('.');
        self.0.push_str(json_name);
    }

    /// Appends `[index]`.
    pub fn push_index(&mut self, index: usize) {
        use fmt::Write as _;
        let _ = write!(self.0, "[{index}]");
    }

    /// Appends `.key` for a map entry.
    pub fn push_key(&mut self, key: &str) {
        self.push_field(key);
    }

    /// Restores a previous [`mark`](Self::mark).
    pub fn truncate(&mut self, mark: usize) {
        self.0.truncate(mark);
    }

    /// The path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Types that can check their own invariants.
///
/// `#[model]` implements this; std and `hyve-core` types implement it as "always
/// valid" while still recursing into containers.
pub trait Validate {
    /// Records violations of `self` (located at `path`) into `out`.
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations);

    /// Validates from the root.
    fn validate(&self) -> Result<(), Violations> {
        let mut out = Violations::new();
        self.validate_at(&mut FieldPath::root(), &mut out);
        if out.is_empty() { Ok(()) } else { Err(out) }
    }
}

macro_rules! always_valid {
    ($($t:ty),* $(,)?) => {$(
        impl Validate for $t {
            fn validate_at(&self, _: &mut FieldPath, _: &mut Violations) {}
        }
    )*};
}

always_valid!(
    (),
    bool,
    char,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    f32,
    f64,
    String,
    Ulid,
    Timestamp,
    Value
);

impl<T: ?Sized> Validate for Id<T> {
    fn validate_at(&self, _: &mut FieldPath, _: &mut Violations) {}
}

impl<T: Validate> Validate for Option<T> {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        if let Some(v) = self {
            v.validate_at(path, out);
        }
    }
}

impl<T: Validate> Validate for Box<T> {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        (**self).validate_at(path, out);
    }
}

impl<T: Validate> Validate for Vec<T> {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        self.as_slice().validate_at(path, out);
    }
}

impl<T: Validate, const N: usize> Validate for [T; N] {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        self.as_slice().validate_at(path, out);
    }
}

impl<T: Validate> Validate for [T] {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        for (i, v) in self.iter().enumerate() {
            let mark = path.mark();
            path.push_index(i);
            v.validate_at(path, out);
            path.truncate(mark);
        }
    }
}

impl<V: Validate> Validate for BTreeMap<String, V> {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        for (k, v) in self {
            let mark = path.mark();
            path.push_key(k);
            v.validate_at(path, out);
            path.truncate(mark);
        }
    }
}

impl<V: Validate, S> Validate for HashMap<String, V, S> {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        for (k, v) in self {
            let mark = path.mark();
            path.push_key(k);
            v.validate_at(path, out);
            path.truncate(mark);
        }
    }
}

/// A value that passed [`Validate`].
///
/// The only constructor is [`Valid::new`], so holding a `Valid<T>` proves the
/// rules held when it was built. Request extractors build it after decoding;
/// `#[server]` stubs take the plain `T` and the server re-validates.
#[derive(Debug, Clone, PartialEq)]
pub struct Valid<T>(T);

impl<T: Validate> Valid<T> {
    /// Validates `value`; violations become an `invalid_request` error.
    pub fn new(value: T) -> Result<Self, HyveError> {
        value.validate().map_err(Violations::into_error)?;
        Ok(Valid(value))
    }
}

impl<T> Valid<T> {
    /// The validated value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Valid<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> AsRef<T> for Valid<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Input {
        title: String,
        tags: Vec<Tag>,
        fps: Option<u32>,
    }
    #[derive(Debug)]
    struct Tag {
        name: String,
    }

    impl Validate for Tag {
        fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
            let m = path.mark();
            path.push_field("name");
            rules::len(&self.name, 1..=3, path, out);
            self.name.validate_at(path, out);
            path.truncate(m);
        }
    }

    impl Validate for Input {
        fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
            let m = path.mark();
            path.push_field("title");
            rules::len(&self.title, 1..=80, path, out);
            path.truncate(m);
            path.push_field("tags");
            self.tags.validate_at(path, out);
            path.truncate(m);
            path.push_field("fps");
            rules::range(&self.fps, 1..=240, path, out);
            path.truncate(m);
        }
    }

    #[test]
    fn valid_input_passes() {
        let v = Valid::new(Input {
            title: "ok".into(),
            tags: vec![Tag { name: "a".into() }],
            fps: None,
        })
        .unwrap();
        assert_eq!(v.title, "ok");
    }

    #[test]
    fn violations_carry_paths_and_params() {
        let err = Valid::new(Input {
            title: String::new(),
            tags: vec![
                Tag { name: "ok".into() },
                Tag {
                    name: "toolong".into(),
                },
            ],
            fps: Some(0),
        })
        .unwrap_err();
        assert_eq!(err.code(), "invalid_request");
        let Some(Value::List(fields)) = err.details().get("fields") else {
            panic!("{err:?}")
        };
        let paths: Vec<_> = fields.iter().map(|f| f.get("path").cloned()).collect();
        assert_eq!(
            paths,
            vec![
                Some("$.title".into()),
                Some("$.tags[1].name".into()),
                Some("$.fps".into())
            ]
        );
        assert_eq!(fields[0].get("min"), Some(&Value::Int(1)));
        assert_eq!(fields[0].get("max"), Some(&Value::Int(80)));
        assert_eq!(fields[2].get("rule"), Some(&Value::from("range")));
    }
}
