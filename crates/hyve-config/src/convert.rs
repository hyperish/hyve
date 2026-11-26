//! Schema-directed conversion of TOML values and plain text into `Value`s
//! the model's JSON decoder reads, recording where each value came from.

use hyve_core::Value;
use hyve_diagnostics::{Span, suggest};
use hyve_parser::{Spanned, TomlTable, TomlValue, parse_toml};
use hyve_validate::{Definition, FieldSchema, SchemaNode, SchemaRegistry};

use crate::error::{Problem, Source};

/// Where the value at `path` came from.
#[derive(Debug, Clone)]
pub(crate) struct Origin {
    pub path: String,
    pub source: Source,
    pub key: String,
    pub span: Option<Span>,
}

/// Whether `prefix` is `path` or one of its ancestors.
pub(crate) fn covers(prefix: &str, path: &str) -> bool {
    path == prefix
        || (path.starts_with(prefix) && matches!(path.as_bytes()[prefix.len()], b'.' | b'['))
}

/// The origin of the closest recorded ancestor of `path`.
pub(crate) fn origin_for<'o>(origins: &'o [Origin], path: &str) -> Option<&'o Origin> {
    origins
        .iter()
        .filter(|o| covers(&o.path, path))
        .max_by_key(|o| o.path.len())
}

/// Conversion state shared by all layers.
pub(crate) struct Cx<'r> {
    pub reg: &'r SchemaRegistry,
    pub source: Source,
    pub origins: Vec<Origin>,
    pub problems: Vec<Problem>,
    /// For text sources: every value converted from the text gets this span
    /// (the spans inside the parsed text are meaningless to the user).
    fixed_span: Option<Option<Span>>,
}

impl<'r> Cx<'r> {
    pub fn new(reg: &'r SchemaRegistry) -> Self {
        Cx {
            reg,
            source: Source::Default,
            origins: Vec::new(),
            problems: Vec::new(),
            fixed_span: None,
        }
    }

    pub fn problem(
        &mut self,
        key: &str,
        path: &str,
        span: Option<Span>,
        message: String,
        help: Option<String>,
    ) {
        self.problems.push(Problem {
            source: Some(self.source.clone()),
            key: key.to_owned(),
            path: path.to_owned(),
            message,
            help,
            span,
        });
    }

    /// Forgets the origins of a value about to be replaced.
    pub fn replace(&mut self, path: &str) {
        self.origins.retain(|o| !covers(path, &o.path));
    }

    fn record(&mut self, path: &str, key: &str, span: Span) -> Option<Span> {
        let span = self.fixed_span.unwrap_or(Some(span));
        self.origins.push(Origin {
            path: path.to_owned(),
            source: self.source.clone(),
            key: key.to_owned(),
            span,
        });
        span
    }

    /// Converts the keys of a TOML table naming `fields` (by Rust name).
    /// Returns `(field index, value)` pairs; unknown keys are problems.
    pub fn fields(
        &mut self,
        table: &TomlTable,
        fields: &[FieldSchema],
        path: &str,
        key: &str,
    ) -> Vec<(usize, Option<Value>)> {
        let mut out = Vec::new();
        for (k, v) in table.entries() {
            let span = self.fixed_span.unwrap_or(Some(k.span));
            match fields.iter().position(|f| f.name == k.value) {
                Some(i) => {
                    let f = &fields[i];
                    let child = format!("{path}.{}", f.json_name);
                    self.replace(&child);
                    let value = self.toml(v, &f.schema, &child, &join(key, f.name));
                    out.push((i, value));
                }
                None => {
                    let help = match suggest(&k.value, fields.iter().map(|f| f.name)) {
                        Some(s) => format!("did you mean `{s}`?"),
                        None => format!("known keys: {}", list(fields.iter().map(|f| f.name))),
                    };
                    self.problem(
                        &join(key, &k.value),
                        path,
                        span,
                        format!("unknown key `{}`", k.value),
                        Some(help),
                    );
                }
            }
        }
        out
    }

    /// Converts a TOML value to the shape `node` describes.
    pub fn toml(
        &mut self,
        v: &Spanned<TomlValue>,
        node: &SchemaNode,
        path: &str,
        key: &str,
    ) -> Option<Value> {
        if let SchemaNode::Nullable(inner) = node {
            return self.toml(v, inner, path, key);
        }
        let span = self.record(path, key, v.span);
        let reg = self.reg;
        let mismatch = |cx: &mut Self| {
            let message = format!(
                "expected {}, found {}",
                describe(reg, node),
                found(&v.value)
            );
            cx.problem(key, path, span, message, None);
            None
        };
        match (node, &v.value) {
            (SchemaNode::Any, value) => Some(any(value)),
            (SchemaNode::Null, _) => Some(Value::Null),
            (SchemaNode::Bool, TomlValue::Bool(b)) => Some(Value::Bool(*b)),
            (SchemaNode::Integer { bits, signed }, TomlValue::Integer(i)) => {
                let (min, max) = int_range(*bits, *signed);
                if (min..=max).contains(&i128::from(*i)) {
                    Some(Value::Int(*i))
                } else {
                    let ty = format!("{}{bits}", if *signed { 'i' } else { 'u' });
                    self.problem(
                        key,
                        path,
                        span,
                        format!("{i} is out of range for {ty} ({min}..={max})"),
                        None,
                    );
                    None
                }
            }
            (SchemaNode::Float { .. }, TomlValue::Integer(i)) => Some(Value::Float(*i as f64)),
            (SchemaNode::Float { .. }, TomlValue::Float(f)) => Some(Value::Float(*f)),
            (SchemaNode::String { .. }, TomlValue::String(s)) => Some(Value::String(s.clone())),
            (SchemaNode::List { items, len }, TomlValue::Array(xs)) => {
                if let Some(len) = len {
                    if xs.len() as u64 != *len {
                        self.problem(
                            key,
                            path,
                            span,
                            format!("expected {len} items, found {}", xs.len()),
                            None,
                        );
                        return None;
                    }
                }
                let out: Vec<_> = xs
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        self.toml(x, items, &format!("{path}[{i}]"), &format!("{key}[{i}]"))
                    })
                    .collect();
                out.into_iter().collect::<Option<Vec<_>>>().map(Value::List)
            }
            (SchemaNode::Map { values }, TomlValue::Table(t)) => {
                let out: Vec<_> = t
                    .entries()
                    .map(|(k, x)| {
                        let child = format!("{path}.{}", k.value);
                        (
                            k.value.clone(),
                            self.toml(x, values, &child, &join(key, &k.value)),
                        )
                    })
                    .collect();
                out.into_iter()
                    .map(|(k, x)| x.map(|x| (k, x)))
                    .collect::<Option<Vec<_>>>()
                    .map(Value::Map)
            }
            (SchemaNode::Ref(name), value) => match (reg.get(name), value) {
                (Some(Definition::Object(fields)), TomlValue::Table(t)) => {
                    let converted = self.fields(t, fields, path, key);
                    let failed = self.problems.len();
                    let mut entries = Vec::new();
                    for (i, x) in converted {
                        entries.push((fields[i].json_name.to_owned(), x?));
                    }
                    (failed == self.problems.len()).then_some(Value::Map(entries))
                }
                (Some(Definition::Enum(values)), TomlValue::String(s)) => {
                    if values.contains(&s.as_str()) {
                        Some(Value::String(s.clone()))
                    } else {
                        let help = suggest(s, values.iter().copied())
                            .map(|v| format!("did you mean `{v}`?"));
                        self.problem(
                            key,
                            path,
                            span,
                            format!("`{s}` is not one of {}", list(values.iter().copied())),
                            help,
                        );
                        None
                    }
                }
                (None, value) => Some(any(value)),
                _ => mismatch(self),
            },
            _ => mismatch(self),
        }
    }

    /// Converts the text of an environment variable or `--set` argument.
    /// Text fields take it as is; other fields read it as a TOML value
    /// (`144`, `true`, `["a", "b"]`, `{ width = 800 }`).
    pub fn text(
        &mut self,
        text: &str,
        node: &SchemaNode,
        path: &str,
        key: &str,
        span: Option<Span>,
    ) -> Option<Value> {
        let inner = unwrap_nullable(node);
        if self.is_text(inner) {
            self.origins.push(Origin {
                path: path.to_owned(),
                source: self.source.clone(),
                key: key.to_owned(),
                span,
            });
            return Some(Value::String(text.to_owned()));
        }
        let parsed = parse_toml(&format!("v = {text}\n"))
            .ok()
            .and_then(|t| t.get("v").cloned());
        let Some(value) = parsed else {
            let message = format!("expected {}, found `{text}`", describe(self.reg, inner));
            let help = matches!(
                inner,
                SchemaNode::List { .. } | SchemaNode::Map { .. } | SchemaNode::Ref(_)
            )
            .then(|| {
                "write the value in TOML syntax: `[\"a\", \"b\"]`, `{ width = 800 }`".to_owned()
            });
            self.problem(key, path, span, message, help);
            return None;
        };
        self.fixed_span = Some(span);
        let out = self.toml(&value, inner, path, key);
        self.fixed_span = None;
        out
    }

    fn is_text(&self, node: &SchemaNode) -> bool {
        match node {
            SchemaNode::String { .. } => true,
            SchemaNode::Ref(name) => matches!(self.reg.get(name), Some(Definition::Enum(_))),
            _ => false,
        }
    }
}

fn unwrap_nullable(node: &SchemaNode) -> &SchemaNode {
    match node {
        SchemaNode::Nullable(inner) => unwrap_nullable(inner),
        other => other,
    }
}

fn join(key: &str, name: &str) -> String {
    if key.is_empty() {
        name.to_owned()
    } else {
        format!("{key}.{name}")
    }
}

fn list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items
        .map(|s| format!("`{s}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn int_range(bits: u8, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}

fn any(v: &TomlValue) -> Value {
    match v {
        TomlValue::String(s) => Value::String(s.clone()),
        TomlValue::Integer(i) => Value::Int(*i),
        TomlValue::Float(f) => Value::Float(*f),
        TomlValue::Bool(b) => Value::Bool(*b),
        TomlValue::Array(xs) => Value::List(xs.iter().map(|x| any(&x.value)).collect()),
        TomlValue::Table(t) => Value::Map(
            t.entries()
                .map(|(k, x)| (k.value.clone(), any(&x.value)))
                .collect(),
        ),
    }
}

fn describe(reg: &SchemaRegistry, node: &SchemaNode) -> String {
    match node {
        SchemaNode::Null => "nothing".into(),
        SchemaNode::Bool => "`true` or `false`".into(),
        SchemaNode::Integer { bits, signed } => {
            format!("an integer ({}{bits})", if *signed { 'i' } else { 'u' })
        }
        SchemaNode::Float { .. } => "a number".into(),
        SchemaNode::String { format: Some(f) } => format!("a string ({f})"),
        SchemaNode::String { format: None } => "a string".into(),
        SchemaNode::List { .. } => "an array".into(),
        SchemaNode::Map { .. } => "a table".into(),
        SchemaNode::Nullable(inner) => describe(reg, inner),
        SchemaNode::Any => "a value".into(),
        SchemaNode::Ref(name) => match reg.get(name) {
            Some(Definition::Enum(values)) => format!("one of {}", list(values.iter().copied())),
            _ => "a table".into(),
        },
    }
}

fn found(v: &TomlValue) -> &'static str {
    match v {
        TomlValue::String(_) => "a string",
        TomlValue::Integer(_) => "an integer",
        TomlValue::Float(_) => "a float",
        TomlValue::Bool(_) => "a boolean",
        TomlValue::Array(_) => "an array",
        TomlValue::Table(_) => "a table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covering_paths() {
        assert!(covers("$.a", "$.a"));
        assert!(covers("$.a", "$.a.b"));
        assert!(covers("$.a", "$.a[0]"));
        assert!(!covers("$.a", "$.ab"));
        let o = |p: &str| Origin {
            path: p.into(),
            source: Source::Env,
            key: p.into(),
            span: None,
        };
        let origins = [o("$.a"), o("$.a.b"), o("$.c")];
        assert_eq!(
            origin_for(&origins, "$.a.b[2]").map(|o| o.path.as_str()),
            Some("$.a.b")
        );
        assert_eq!(
            origin_for(&origins, "$.a.x").map(|o| o.path.as_str()),
            Some("$.a")
        );
        assert!(origin_for(&origins, "$.d").is_none());
    }

    #[test]
    fn integer_ranges() {
        assert_eq!(int_range(8, false), (0, 255));
        assert_eq!(int_range(8, true), (-128, 127));
        assert_eq!(int_range(64, false).1, u64::MAX as i128);
    }
}
