//! `hyve-config` — startup configuration, validated before the app runs
//! (`docs/01-dx-and-dsl.md` §1).
//!
//! [`load`] reads the `[app]` table of `hyve.toml`, a `.env` file, environment
//! variables and `--set <field>=<value>` arguments into a `#[model]` struct and
//! validates it. Later sources replace whole top-level fields of earlier ones:
//!
//! | Priority | Source | Key for field `max_fps` |
//! |---|---|---|
//! | 1 | `--set` argument | `--set max_fps=144` |
//! | 2 | environment variable | `APP_MAX_FPS=144` |
//! | 3 | `.env` next to `hyve.toml` | `APP_MAX_FPS=144` |
//! | 4 | `hyve.toml` | `[app]` `max_fps = 144` |
//! | 5 | the model's `Default` | |
//!
//! - `hyve.toml` is found in the working directory or its nearest ancestor
//!   that has one; `.env` is read from the same directory.
//! - Keys are the Rust field names. Environment variable names are the field
//!   name in SCREAMING_SNAKE_CASE after a prefix, `[hyve] env_prefix` (default
//!   `APP`). Variables with the prefix that name no field are ignored, since the
//!   environment is shared with other programs.
//! - Text fields take environment and `--set` values as written; other fields
//!   read them as TOML values (`144`, `true`, `["a", "b"]`, `{ width = 800 }`).
//! - Without `Default`, every non-`Option` field must come from some source.
//! - Every failure names its source and key ([`ConfigError`]).
//!
//! Top-level tables of `hyve.toml` belong to their readers: `[app]` to the app,
//! `[hyve]`, `[orbic]` and `[serge]` to the tools and frameworks, which load
//! their table with [`Loader::section`].

mod convert;
mod error;

use std::io;
use std::path::{Path, PathBuf};

use hyve_core::Value;
use hyve_diagnostics::suggest;
use hyve_parser::{TomlTable, TomlValue, parse_dotenv, parse_toml};
use hyve_serde::{Json, from_json, to_json};
use hyve_validate::{Definition, FieldSchema, Schema, SchemaNode, SchemaRegistry, Valid, Validate};

use convert::{Cx, origin_for};
pub use error::{ConfigError, Problem, Source};

/// The configuration file name.
pub const FILE_NAME: &str = "hyve.toml";
/// The dotenv file name.
pub const DOTENV_FILE_NAME: &str = ".env";
/// The environment variable prefix for `[app]` when `[hyve] env_prefix` is not set.
pub const DEFAULT_ENV_PREFIX: &str = "APP";
/// Stands in for a value that failed to convert (already reported), so the
/// field is not also reported as missing; loading stops before decoding.
const FAILED: Value = Value::Null;

/// The top-level tables `hyve.toml` may have.
pub const SECTIONS: [&str; 4] = ["app", "hyve", "orbic", "serge"];

/// Loads and validates the app configuration from the process's working
/// directory, environment and arguments. See the [crate docs](crate).
pub fn load<T: Json + Schema + Validate>() -> Result<Valid<T>, ConfigError> {
    Loader::new().load()
}

/// The nearest directory at or above `start` that contains `hyve.toml`.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(FILE_NAME).is_file())
        .map(Path::to_path_buf)
}

/// Configuration loading with explicit inputs (tests, tools).
#[derive(Debug, Clone)]
pub struct Loader {
    dir: Option<PathBuf>,
    env: Option<Vec<(String, String)>>,
    args: Option<Vec<String>>,
    section: String,
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

impl Loader {
    /// Reads the process's working directory, environment and arguments, and
    /// the `[app]` table.
    pub fn new() -> Self {
        Loader {
            dir: None,
            env: None,
            args: None,
            section: "app".into(),
        }
    }

    /// Reads `hyve.toml` and `.env` from exactly this directory.
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Uses these variables instead of the process environment.
    pub fn env<K: Into<String>, V: Into<String>>(
        mut self,
        vars: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.env = Some(
            vars.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    /// Uses these arguments (without the program name) instead of the
    /// process's.
    pub fn args<A: Into<String>>(mut self, args: impl IntoIterator<Item = A>) -> Self {
        self.args = Some(args.into_iter().map(Into::into).collect());
        self
    }

    /// Loads another table of `hyve.toml` (`hyve`, `orbic`, `serge`). Its
    /// environment variables use the prefix `HYPERISH_<SECTION>`
    /// (`HYPERISH_ORBIC_DIALECT`); `--set` applies only to `[app]`.
    pub fn section(mut self, name: &str) -> Self {
        self.section = name.to_owned();
        self
    }

    /// Loads and validates `T`.
    pub fn load<T: Json + Schema + Validate>(&self) -> Result<Valid<T>, ConfigError> {
        let mut files = Vec::new();
        let mut problems = Vec::new();
        let dir = self.dir.clone().or_else(|| {
            let cwd = std::env::current_dir().ok()?;
            Some(find_root(&cwd).unwrap_or(cwd))
        });
        let fail = |problems: Vec<Problem>, files| Err(ConfigError::new(problems, files));
        let missing_problem = |source: Source, message: String| Problem {
            source: Some(source),
            key: String::new(),
            path: "$".into(),
            message,
            help: None,
            span: None,
        };

        // hyve.toml
        let mut root = TomlTable::default();
        if let Some(dir) = &dir {
            let path = dir.join(FILE_NAME);
            match read_optional(&path) {
                Ok(Some(text)) => {
                    match parse_toml(&text) {
                        Ok(t) => root = t,
                        Err(d) => problems.push(Problem {
                            source: Some(Source::File(path.clone())),
                            key: String::new(),
                            path: "$".into(),
                            message: d.message().to_owned(),
                            help: d.help().first().cloned(),
                            span: d.primary_span(),
                        }),
                    }
                    files.push((path, text));
                }
                Ok(None) => {}
                Err(e) => problems.push(missing_problem(
                    Source::File(path),
                    format!("cannot read the file: {e}"),
                )),
            }
        }
        if !problems.is_empty() {
            return fail(problems, files);
        }
        let toml_source =
            || Source::File(files.first().map(|(p, _)| p.clone()).unwrap_or_default());
        let mut cx_problems = Vec::new();
        let section = check_sections(&root, &self.section, &mut cx_problems, &toml_source);
        let prefix = match self.section.as_str() {
            "app" => env_prefix(&root, &mut cx_problems, &toml_source),
            other => format!("HYPERISH_{}", other.to_ascii_uppercase()),
        };

        // The model's shape.
        let mut reg = SchemaRegistry::new();
        let node = T::schema(&mut reg);
        let fields: Vec<FieldSchema> = match &node {
            SchemaNode::Ref(name) => match reg.get(name) {
                Some(Definition::Object(fields)) => fields.clone(),
                _ => Vec::new(),
            },
            _ => {
                problems.push(Problem {
                    source: None,
                    key: String::new(),
                    path: "$".into(),
                    message: "configuration must be a `#[model]` struct with named fields".into(),
                    help: None,
                    span: None,
                });
                return fail(problems, files);
            }
        };
        let mut values: Vec<Option<Value>> = vec![None; fields.len()];
        let mut cx = Cx::new(&reg);
        cx.problems = cx_problems;
        let field_path = |f: &FieldSchema| format!("$.{}", f.json_name);

        // 5. Default
        if let Some(default) = T::default_value() {
            cx.source = Source::Default;
            if let Ok(Value::Map(entries)) = from_json::<Value>(&to_json(&default)) {
                for (k, v) in entries {
                    if let Some(i) = fields.iter().position(|f| f.json_name == k) {
                        let path = field_path(&fields[i]);
                        cx.replace(&path);
                        cx.origins.push(convert::Origin {
                            path,
                            source: Source::Default,
                            key: fields[i].name.to_owned(),
                            span: None,
                        });
                        values[i] = Some(v);
                    }
                }
            }
        }

        // 4. hyve.toml
        if let Some(table) = section {
            cx.source = toml_source();
            for (i, v) in cx.fields(table, &fields, "$", &self.section) {
                values[i] = Some(v.unwrap_or(FAILED));
            }
        }

        // 3. .env, 2. environment
        if let Some(dir) = &dir {
            let path = dir.join(DOTENV_FILE_NAME);
            match read_optional(&path) {
                Ok(Some(text)) => {
                    cx.source = Source::DotEnv(path.clone());
                    match parse_dotenv(&text) {
                        Ok(entries) => {
                            for e in entries {
                                if let Some(i) = field_for(&fields, &prefix, e.key) {
                                    let p = field_path(&fields[i]);
                                    cx.replace(&p);
                                    let v = cx.text(
                                        &e.value,
                                        &fields[i].schema,
                                        &p,
                                        e.key,
                                        Some(e.span),
                                    );
                                    values[i] = Some(v.unwrap_or(FAILED));
                                }
                            }
                        }
                        Err(d) => {
                            let help = d.help().first().cloned();
                            cx.problem("", "$", d.primary_span(), d.message().to_owned(), help);
                        }
                    }
                    files.push((path, text));
                }
                Ok(None) => {}
                Err(e) => cx.problems.push(missing_problem(
                    Source::DotEnv(path),
                    format!("cannot read the file: {e}"),
                )),
            }
        }
        cx.source = Source::Env;
        for (k, v) in self.env_vars(&prefix, &mut cx) {
            if let Some(i) = field_for(&fields, &prefix, &k) {
                let p = field_path(&fields[i]);
                cx.replace(&p);
                let v = cx.text(&v, &fields[i].schema, &p, &k, None);
                values[i] = Some(v.unwrap_or(FAILED));
            }
        }

        // 1. --set
        if self.section == "app" {
            cx.source = Source::Arg;
            for set in self.sets(&mut cx) {
                let (name, text) = set;
                let key = format!("--set {name}");
                if name.contains('.') {
                    let top = name.split('.').next().unwrap_or_default();
                    cx.problem(
                        &key,
                        "$",
                        None,
                        "nested keys are not supported by `--set`".into(),
                        Some(format!("set the whole field: `--set {top}={{ ... }}`")),
                    );
                    continue;
                }
                match fields.iter().position(|f| f.name == name) {
                    Some(i) => {
                        let p = field_path(&fields[i]);
                        cx.replace(&p);
                        let v = cx.text(&text, &fields[i].schema, &p, &key, None);
                        values[i] = Some(v.unwrap_or(FAILED));
                    }
                    None => {
                        let help = suggest(&name, fields.iter().map(|f| f.name))
                            .map(|s| format!("did you mean `{s}`?"));
                        cx.problem(&key, "$", None, format!("unknown field `{name}`"), help);
                    }
                }
            }
        }

        // Missing fields.
        for (f, v) in fields.iter().zip(&values) {
            if v.is_none() && f.required {
                let env = format!("{prefix}_{}", f.name.to_ascii_uppercase());
                let arg = if self.section == "app" {
                    format!(", or pass `--set {}=…`", f.name)
                } else {
                    String::new()
                };
                cx.problems.push(Problem {
                    source: None,
                    key: f.name.to_owned(),
                    path: field_path(f),
                    message: "is not set".into(),
                    help: Some(format!(
                        "set `{} = …` under `[{}]` in {FILE_NAME}, the environment variable `{env}`{arg}",
                        f.name, self.section
                    )),
                    span: None,
                });
            }
        }
        let Cx {
            origins,
            problems: mut found,
            ..
        } = cx;
        problems.append(&mut found);
        if !problems.is_empty() {
            return fail(problems, files);
        }

        // Decode and validate.
        let object = Value::Map(
            fields
                .iter()
                .zip(values)
                .filter_map(|(f, v)| Some((f.json_name.to_owned(), v?)))
                .collect(),
        );
        let located = |path: &str, message: String| match origin_for(&origins, path) {
            Some(o) => Problem {
                source: Some(o.source.clone()),
                key: o.key.clone(),
                path: path.to_owned(),
                message,
                help: None,
                span: o.span,
            },
            None => Problem {
                source: None,
                key: String::new(),
                path: path.to_owned(),
                message,
                help: None,
                span: None,
            },
        };
        let value = match from_json::<T>(&to_json(&object)) {
            Ok(v) => v,
            Err(e) => return fail(vec![located(e.path(), e.message().to_owned())], files),
        };
        if let Err(violations) = value.validate() {
            let problems = violations
                .iter()
                .map(|v| located(&v.path, v.message.clone()))
                .collect();
            return fail(problems, files);
        }
        Valid::new(value)
            .map_err(|e| ConfigError::new(vec![located("$", e.message().to_owned())], files))
    }

    /// Environment variables starting with `prefix_`.
    fn env_vars(&self, prefix: &str, cx: &mut Cx<'_>) -> Vec<(String, String)> {
        let start = format!("{prefix}_");
        match &self.env {
            Some(vars) => vars
                .iter()
                .filter(|(k, _)| k.starts_with(&start))
                .cloned()
                .collect(),
            None => {
                let mut out = Vec::new();
                for (k, v) in std::env::vars_os() {
                    let Some(k) = k.to_str().filter(|k| k.starts_with(&start)) else {
                        continue;
                    };
                    match v.into_string() {
                        Ok(v) => out.push((k.to_owned(), v)),
                        Err(_) => {
                            cx.problem(k, "$", None, "the value is not valid UTF-8".into(), None)
                        }
                    }
                }
                out.sort();
                out
            }
        }
    }

    /// `--set <field>=<value>` pairs, in order.
    fn sets(&self, cx: &mut Cx<'_>) -> Vec<(String, String)> {
        let args: Vec<String> = match &self.args {
            Some(args) => args.clone(),
            None => std::env::args_os()
                .skip(1)
                .filter_map(|a| a.into_string().ok())
                .collect(),
        };
        let mut out = Vec::new();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let pair = if arg == "--set" {
                match it.next() {
                    Some(p) => p,
                    None => {
                        cx.problem(
                            "--set",
                            "$",
                            None,
                            "`--set` needs `<field>=<value>`".into(),
                            None,
                        );
                        break;
                    }
                }
            } else if let Some(p) = arg.strip_prefix("--set=") {
                p.to_owned()
            } else {
                continue;
            };
            match pair.split_once('=') {
                Some((k, v)) if !k.trim().is_empty() => {
                    out.push((k.trim().to_owned(), v.to_owned()))
                }
                _ => cx.problem(
                    &format!("--set {pair}"),
                    "$",
                    None,
                    "expected `<field>=<value>`".into(),
                    Some("for example `--set max_fps=144`".into()),
                ),
            }
        }
        out
    }
}

/// Reads a file; `None` when it does not exist (or the platform has no files).
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Field index for `<prefix>_<FIELD>`.
fn field_for(fields: &[FieldSchema], prefix: &str, var: &str) -> Option<usize> {
    let rest = var.strip_prefix(prefix)?.strip_prefix('_')?;
    fields
        .iter()
        .position(|f| f.name.to_ascii_uppercase() == rest)
}

/// Checks the top level of `hyve.toml` and returns the requested table.
fn check_sections<'t>(
    root: &'t TomlTable,
    wanted: &str,
    problems: &mut Vec<Problem>,
    source: &dyn Fn() -> Source,
) -> Option<&'t TomlTable> {
    let mut found = None;
    for (k, v) in root.entries() {
        let problem = |message: String, help: Option<String>| Problem {
            source: Some(source()),
            key: k.value.clone(),
            path: "$".into(),
            message,
            help,
            span: Some(k.span),
        };
        match (&v.value, SECTIONS.contains(&k.value.as_str())) {
            (TomlValue::Table(t), true) => {
                if k.value == wanted {
                    found = Some(t);
                }
            }
            (TomlValue::Table(_), false) => {
                let help = match suggest(&k.value, SECTIONS) {
                    Some(s) => format!("did you mean `[{s}]`?"),
                    None => "app settings go under `[app]`".to_owned(),
                };
                problems.push(problem(
                    format!("unknown section `[{}]`", k.value),
                    Some(help),
                ));
            }
            (_, true) => problems.push(problem(
                format!("`{}` must be a table: `[{}]`", k.value, k.value),
                None,
            )),
            (_, false) => problems.push(problem(
                format!("`{}` is outside any section", k.value),
                Some(format!(
                    "app settings go under `[app]`: `[app]\\n{} = …`",
                    k.value
                )),
            )),
        }
    }
    found
}

/// `[hyve] env_prefix`, checked.
fn env_prefix(
    root: &TomlTable,
    problems: &mut Vec<Problem>,
    source: &dyn Fn() -> Source,
) -> String {
    let Some(TomlValue::Table(hyve)) = root.get("hyve").map(|v| &v.value) else {
        return DEFAULT_ENV_PREFIX.to_owned();
    };
    let Some(v) = hyve.get("env_prefix") else {
        return DEFAULT_ENV_PREFIX.to_owned();
    };
    match &v.value {
        TomlValue::String(s)
            if s.starts_with(|c: char| c.is_ascii_uppercase())
                && s.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') =>
        {
            s.trim_end_matches('_').to_owned()
        }
        _ => {
            problems.push(Problem {
                source: Some(source()),
                key: "hyve.env_prefix".into(),
                path: "$".into(),
                message: "`env_prefix` must be an upper-case name such as \"APP\"".into(),
                help: Some("use letters, digits and `_`, starting with a letter".into()),
                span: Some(v.span),
            });
            DEFAULT_ENV_PREFIX.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_names_follow_the_field_names() {
        let f = |name: &'static str| FieldSchema {
            name,
            json_name: name,
            schema: SchemaNode::Bool,
            required: true,
            constraints: vec![],
        };
        let fields = [f("max_fps"), f("name")];
        assert_eq!(field_for(&fields, "APP", "APP_MAX_FPS"), Some(0));
        assert_eq!(field_for(&fields, "APP", "APP_NAME"), Some(1));
        assert_eq!(field_for(&fields, "APP", "APPNAME"), None);
        assert_eq!(field_for(&fields, "APP", "OTHER_NAME"), None);
    }
}
