//! Derived from the single-source caches in ariadne 0.6.0, src/source.rs,
//! revision afeff531d1571316cbe23cf41cc23be0a117be8e. See LICENSE-MIT.
//! Keep the byte index for every report on this input and display the borrowed
//! source name directly; the generic upstream tuple boxes its display value.
use ariadne::{Cache, Source};
use std::{borrow::Cow, fmt};

/// The line index of a named UTF-8 source, reusable across diagnostics.
pub struct DiagnosticSource<'src> {
    pub(super) name: Cow<'src, str>,
    pub(super) index: Source<&'src str>,
}

impl<'src> DiagnosticSource<'src> {
    /// Indexes the text once. The index borrows the original source.
    pub fn new(name: impl Into<Cow<'src, str>>, text: &'src str) -> Self {
        Self {
            name: name.into(),
            index: Source::from(text),
        }
    }
    /// Renders one report with byte spans into this source.
    pub fn render(&self, diagnostic: &super::Diagnostic) -> String {
        diagnostic.render_with(self, false)
    }
}

impl<'src> Cache<&str> for &DiagnosticSource<'src> {
    type Storage = &'src str;

    fn fetch(&mut self, id: &&str) -> Result<&Source<Self::Storage>, impl fmt::Debug> {
        if *id == self.name {
            Ok(&self.index)
        } else {
            Err("unknown diagnostic source")
        }
    }
    fn display<'a>(&self, id: &'a &str) -> Option<impl fmt::Display + 'a> {
        Some(id)
    }
}
