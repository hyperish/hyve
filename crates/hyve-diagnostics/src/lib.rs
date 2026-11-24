//! `hyve-diagnostics` — structured diagnostics for compile time and run time.
//!
//! A [`Diagnostic`] always says what is wrong and, through [`help`](Diagnostic::with_help),
//! how to fix it (`hyperish/docs` `01-dx-principles.md`). It can be rendered
//! against its source text (configuration files, DSL snippets) or summarised
//! without source for proc-macro `compile_error!` output.
//!
//! This crate is a build dependency of the proc-macro crates, so it never depends
//! on `hyve-macros` or the `hyve` facade (`docs/02-architecture.md` §2.3).

use std::fmt;
use std::ops::Range;

/// A byte range into some source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// Start byte offset (inclusive).
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
}

impl Span {
    /// A span from `start` to `end`.
    ///
    /// # Panics
    ///
    /// Panics if `end < start`.
    pub const fn new(start: usize, end: usize) -> Self {
        assert!(end >= start, "span end before start");
        Span { start, end }
    }

    /// Shifts the span by `offset` bytes (for spans relative to a sub-slice).
    pub const fn offset(self, offset: usize) -> Self {
        Span {
            start: self.start + offset,
            end: self.end + offset,
        }
    }

    /// Byte range form.
    pub const fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

impl From<Range<usize>> for Span {
    fn from(r: Range<usize>) -> Self {
        Span::new(r.start, r.end)
    }
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The input is rejected.
    Error,
    /// The input is accepted but probably wrong.
    Warning,
}

/// A message attached to a span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// Where.
    pub span: Span,
    /// What is wrong there.
    pub message: String,
}

/// A structured diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    severity: Severity,
    message: String,
    labels: Vec<Label>,
    help: Vec<String>,
    notes: Vec<String>,
}

impl Diagnostic {
    /// An error.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message.into())
    }

    /// A warning.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message.into())
    }

    fn new(severity: Severity, message: String) -> Self {
        Diagnostic {
            severity,
            message,
            labels: Vec::new(),
            help: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Points at a span with a message.
    pub fn with_label(mut self, span: impl Into<Span>, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span: span.into(),
            message: message.into(),
        });
        self
    }

    /// Says how to fix the problem.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }

    /// Adds background information.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Severity.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// Main message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Labels in insertion order.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// The span of the first label, if any.
    pub fn primary_span(&self) -> Option<Span> {
        self.labels.first().map(|l| l.span)
    }

    /// Fix suggestions.
    pub fn help(&self) -> &[String] {
        &self.help
    }

    /// Notes.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Shifts every label by `offset` bytes.
    pub fn offset(mut self, offset: usize) -> Self {
        for l in &mut self.labels {
            l.span = l.span.offset(offset);
        }
        self
    }

    /// One-block text without source excerpts, used for `compile_error!`
    /// messages where the compiler already shows the span:
    ///
    /// ```text
    /// message
    ///   = label message
    ///   = help: …
    ///   = note: …
    /// ```
    pub fn summary(&self) -> String {
        let mut out = self.message.clone();
        for l in &self.labels {
            if !l.message.is_empty() && l.message != self.message {
                out.push_str("\n  = ");
                out.push_str(&l.message);
            }
        }
        for h in &self.help {
            out.push_str("\n  = help: ");
            out.push_str(h);
        }
        for n in &self.notes {
            out.push_str("\n  = note: ");
            out.push_str(n);
        }
        out
    }

    /// Renders the diagnostic with excerpts of `source` (no colours), e.g. for a
    /// configuration file error printed at start-up.
    pub fn render(&self, source_name: &str, source: &str) -> String {
        self.render_with(source_name, source, false)
    }

    /// Like [`render`](Self::render) with ANSI colours.
    pub fn render_colored(&self, source_name: &str, source: &str) -> String {
        self.render_with(source_name, source, true)
    }

    fn render_with(&self, source_name: &str, source: &str, color: bool) -> String {
        use ariadne::{Config, IndexType, Label as ALabel, Report, ReportKind, Source};

        let clamp = |s: Span| {
            let end = s.end.min(source.len());
            s.start.min(end)..end
        };
        let kind = match self.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let anchor = self.primary_span().map(clamp).unwrap_or(0..0);
        let config = Config::new()
            .with_color(color)
            .with_index_type(IndexType::Byte);
        let mut report = Report::build(kind, (source_name, anchor))
            .with_config(config)
            .with_message(&self.message);
        for l in &self.labels {
            report = report
                .with_label(ALabel::new((source_name, clamp(l.span))).with_message(&l.message));
        }
        for h in &self.help {
            report = report.with_help(h);
        }
        for n in &self.notes {
            report = report.with_note(n);
        }
        let mut buf = Vec::new();
        if report
            .finish()
            .write((source_name, Source::from(source)), &mut buf)
            .is_err()
        {
            return self.summary();
        }
        String::from_utf8_lossy(&buf).into_owned()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{prefix}: {}", self.summary())
    }
}

impl std::error::Error for Diagnostic {}

/// The candidate closest to `input` by edit distance, if it is close enough to
/// be a plausible typo (used for "did you mean …?" help).
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let max = (input.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|c| (edit_distance(input, c), c))
        .filter(|(d, _)| *d <= max)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// Optimal-string-alignment distance (insert, delete, substitute, swap adjacent).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev2: Vec<usize> = vec![0; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 0..a.len() {
        cur[0] = i + 1;
        for j in 0..b.len() {
            let cost = usize::from(a[i] != b[j]);
            let mut d = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            if i > 0 && j > 0 && a[i] == b[j - 1] && a[i - 1] == b[j] {
                d = d.min(prev2[j - 1] + 1);
            }
            cur[j + 1] = d;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_has_label_help_and_note() {
        let d = Diagnostic::error("unknown key `fsp`")
            .with_label(4..7, "not a field of `EngineConfig`")
            .with_help("did you mean `fps`?")
            .with_note("keys are matched case-sensitively");
        assert_eq!(
            d.summary(),
            "unknown key `fsp`\n  = not a field of `EngineConfig`\n  = help: did you mean `fps`?\n  = note: keys are matched case-sensitively"
        );
        assert!(d.to_string().starts_with("error: unknown key"));
    }

    #[test]
    fn render_points_at_source() {
        let src = "[app]\nfsp = 120\n";
        let d = Diagnostic::error("unknown key `fsp`")
            .with_label(6..9, "unknown key")
            .with_help("did you mean `fps`?");
        let out = d.render("hyve.toml", src);
        assert!(out.contains("hyve.toml:2:1"), "{out}");
        assert!(out.contains("fsp = 120"), "{out}");
        assert!(out.contains("did you mean `fps`?"), "{out}");
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn out_of_range_spans_are_clamped() {
        let d = Diagnostic::error("x").with_label(100..200, "far");
        let _ = d.render("f", "short");
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("fsp", ["fps", "name", "backend"]), Some("fps"));
        assert_eq!(suggest("zzzz", ["fps", "name"]), None);
    }
}
