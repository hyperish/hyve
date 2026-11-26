//! What went wrong, and where it came from.

use std::path::{Path, PathBuf};
use std::{collections::HashMap, fmt};

use hyve_core::{HyveError, Value};
use hyve_diagnostics::{Diagnostic, DiagnosticSource, Span};

/// Where a configuration value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The model's `Default` implementation.
    Default,
    /// A `hyve.toml` file.
    File(PathBuf),
    /// A `.env` file.
    DotEnv(PathBuf),
    /// A process environment variable.
    Env,
    /// A `--set` command-line argument.
    Arg,
}

impl Source {
    /// Short machine-readable name (`default`, `file`, `dotenv`, `env`, `arg`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::File(_) => "file",
            Source::DotEnv(_) => "dotenv",
            Source::Env => "env",
            Source::Arg => "arg",
        }
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Source::File(p) | Source::DotEnv(p) => Some(p),
            _ => None,
        }
    }
}

/// One problem with the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The source of the offending value; `None` when a value is missing
    /// everywhere.
    pub source: Option<Source>,
    /// The key as written in that source (`app.max_fps`, `APP_MAX_FPS`,
    /// `--set max_fps`); the field name when missing.
    pub key: String,
    /// The value's path in the model, with JSON field names (`$.maxFps`).
    pub path: String,
    /// What is wrong.
    pub message: String,
    /// How to fix it.
    pub help: Option<String>,
    /// Where in the source file (byte offsets), for file sources.
    pub span: Option<Span>,
}

/// Configuration could not be loaded; lists every problem found.
#[derive(Debug, Clone)]
pub struct ConfigError {
    problems: Vec<Problem>,
    /// Text of the files read, for locations and snippets.
    files: Vec<(PathBuf, String)>,
}

impl ConfigError {
    pub(crate) fn new(problems: Vec<Problem>, files: Vec<(PathBuf, String)>) -> Self {
        debug_assert!(!problems.is_empty());
        ConfigError { problems, files }
    }

    /// The problems, in the order they were found.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// A report with source snippets for problems located in files (for
    /// terminals; [`Display`](fmt::Display) gives one line per problem).
    pub fn render(&self) -> String {
        let mut out = String::from("invalid configuration\n");
        let mut sources = HashMap::new();
        for p in &self.problems {
            let source = p
                .source
                .as_ref()
                .and_then(Source::path)
                .and_then(|path| self.text(path).map(|text| (path, text)));
            match (source, p.span) {
                (Some((path, text)), Some(span)) => {
                    let source = sources
                        .entry(path)
                        .or_insert_with(|| DiagnosticSource::new(display_path(path), text));
                    let mut d = Diagnostic::error(format!("`{}`: {}", p.key, p.message))
                        .with_label(span, "here");
                    if let Some(help) = &p.help {
                        d = d.with_help(help.clone());
                    }
                    out.push_str(&source.render(&d));
                }
                _ => {
                    out.push_str(&format!("error: {}\n", self.line(p)));
                    if let Some(help) = &p.help {
                        out.push_str(&format!("  = help: {help}\n"));
                    }
                }
            }
        }
        out
    }

    fn text(&self, path: &Path) -> Option<&str> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, t)| t.as_str())
    }

    fn line(&self, p: &Problem) -> String {
        let key = if p.key.is_empty() {
            String::new()
        } else {
            format!("`{}`", p.key)
        };
        let at = match &p.source {
            None => key,
            Some(Source::Default) => format!("default value of {key}"),
            Some(Source::Env) => format!("environment variable {key}"),
            Some(Source::Arg) => format!("argument {key}"),
            Some(Source::File(path) | Source::DotEnv(path)) => {
                let mut at = display_path(path);
                if let (Some(span), Some(text)) = (p.span, self.text(path)) {
                    let (line, col) = line_col(text, span.start);
                    at.push_str(&format!(":{line}:{col}"));
                }
                if key.is_empty() {
                    at
                } else {
                    format!("{at}: {key}")
                }
            }
        };
        if at.is_empty() {
            p.message.clone()
        } else {
            format!("{at}: {}", p.message)
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid configuration")?;
        for p in &self.problems {
            write!(f, "\n  - {}", self.line(p))?;
            if let Some(help) = &p.help {
                write!(f, "\n    help: {help}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// `invalid_config` infrastructure error; `details.problems` lists
/// `{source, file?, key, path, message}`.
impl From<ConfigError> for HyveError {
    fn from(e: ConfigError) -> Self {
        let problems = e
            .problems
            .iter()
            .map(|p| {
                let mut m = Value::map();
                m.insert(
                    "source",
                    p.source.as_ref().map_or("missing", Source::as_str),
                );
                if let Some(path) = p.source.as_ref().and_then(Source::path) {
                    m.insert("file", display_path(path));
                }
                m.insert("key", p.key.clone());
                m.insert("path", p.path.clone());
                m.insert("message", p.message.clone());
                m
            })
            .collect();
        HyveError::infra("invalid_config", e.to_string())
            .with_detail("problems", Value::List(problems))
    }
}

/// The path relative to the working directory when it is inside it.
fn display_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// 1-based line and column (in characters) of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_column() {
        assert_eq!(line_col("a\nbc = 1\n", 5), (2, 4));
        assert_eq!(line_col("x", 0), (1, 1));
    }

    #[test]
    fn display_names_the_source_and_key() {
        let e = ConfigError::new(
            vec![
                Problem {
                    source: Some(Source::Env),
                    key: "APP_MAX_FPS".into(),
                    path: "$.maxFps".into(),
                    message: "expected an integer (u32), found `fast`".into(),
                    help: None,
                    span: None,
                },
                Problem {
                    source: Some(Source::File(PathBuf::from("/nowhere/hyve.toml"))),
                    key: "app.title".into(),
                    path: "$.title".into(),
                    message: "length must be at least 1".into(),
                    help: Some("fix it".into()),
                    span: Some(Span::new(14, 16)),
                },
            ],
            vec![(
                PathBuf::from("/nowhere/hyve.toml"),
                "[app]\ntitle = \"\"\n".into(),
            )],
        );
        let s = e.to_string();
        assert!(
            s.contains("environment variable `APP_MAX_FPS`: expected an integer"),
            "{s}"
        );
        assert!(
            s.contains(
                "/nowhere/hyve.toml:2:9: `app.title`: length must be at least 1\n    help: fix it"
            ),
            "{s}"
        );
        let err = HyveError::from(e.clone());
        assert_eq!(err.code(), "invalid_config");
        assert!(e.render().contains("title"), "{}", e.render());
    }
}
