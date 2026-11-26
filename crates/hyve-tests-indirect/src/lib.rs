//! A crate that reaches `hyve` only through a re-export (`hyve_tests::hyve`),
//! as an app reaches it through a framework. Every macro takes
//! `crate = ::hyve_tests::hyve`; a generated path that ignored the override
//! would name `::hyve`, which does not resolve here.
//!
//! A framework binds the override once in its own forwarding attribute
//! (`#[orbic::model]` expanding to `#[::orbic::__private::hyve::model(crate = ::orbic::__private::hyve)]`);
//! this fixture writes the override out.

#![allow(missing_docs)]

use hyve_tests::hyve::prelude::{HyveResult, Id, Valid};
use hyve_tests::hyve::{model, server};

#[model(crate = ::hyve_tests::hyve)]
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    #[validate(len = 1..=20)]
    pub text: String,
    pub pinned: bool,
}

#[model(crate = ::hyve_tests::hyve)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    Red,
    Blue,
}

#[model(crate = ::hyve_tests::hyve)]
#[derive(Debug, Clone, PartialEq)]
pub struct Boxed<T> {
    pub item: T,
}

#[cfg(feature = "server")]
pub struct Store {
    pub prefix: String,
}

#[server(POST "/notes", crate = ::hyve_tests::hyve)]
pub async fn add_note(
    #[server_ctx] store: hyve_tests::hyve::Ctx<Store>,
    note: Valid<Note>,
    color: Color,
) -> HyveResult<Note> {
    let mut note = note.into_inner();
    note.text = format!("{}{}:{:?}", store.prefix, note.text, color);
    Ok(note)
}

#[server(DELETE "/notes/{id}", crate = ::hyve_tests::hyve)]
pub async fn remove_note(id: Id<Note>) -> HyveResult<()> {
    // Names used only in a body go inside it (or behind the `server`
    // feature): the client build compiles the body out.
    use hyve_tests::hyve::HyveError;
    let _ = id;
    Err(HyveError::not_found("note"))
}
