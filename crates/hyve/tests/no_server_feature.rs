//! `#[model]` and `#[server]` in a crate that has no `server` feature, under
//! `deny(warnings)`: generated code must not trigger `unexpected_cfgs`,
//! `missing_docs` or unused-item lints in user crates.

#![deny(warnings, missing_docs)]

use hyve::prelude::*;

/// A request model.
#[model]
pub struct Rename {
    /// The new title.
    #[validate(len = 1..=80)]
    pub title: String,
}

/// A server function; its body is removed because this crate has no `server`
/// feature, so only the stub and the descriptor remain.
#[server(PATCH "/projects/{id}")]
pub async fn rename(id: Id<Rename>, input: Valid<Rename>) -> HyveResult<()> {
    let _ = (id, input);
    Ok(())
}

#[test]
fn only_the_stub_and_descriptor_exist() {
    let call = rename(Id::new(), Rename { title: "x".into() });
    let d = call.descriptor();
    assert_eq!(d.id, "no_server_feature::rename");
    assert_eq!(
        (d.method, d.path, d.args),
        (
            hyve::api::Method::Patch,
            "/projects/{id}",
            &["id", "input"][..]
        )
    );
}
