//! Fixture crate for the macro and facade tests (not published).
//!
//! It plays the role of an app's shared crate: models plus `#[server]`
//! functions, with its own `server` feature. CI tests it both ways:
//! `cargo test -p hyve-tests` (client build: handler bodies and the
//! server-only `Db` type do not exist) and `--features server`.

// Fixture types are self-explanatory; generated code must not need docs
// (checked by the facade's `no_server_feature` test under `deny(warnings)`).
#![allow(missing_docs)]

use hyve::prelude::*;

/// `README.md` example.
#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct SceneConfig {
    #[validate(len = 1..=80)]
    pub name: String,
    #[validate(range = 1..=240)]
    pub target_fps: u32,
}

/// `docs/01-dx-and-dsl.md` §1 example.
#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    #[validate(len = 1..=64)]
    pub app_name: String,
    #[validate(range = 30..=240)]
    pub max_fps: u32,
}

/// A unit enum (`snake_case` strings in JSON).
#[model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Draft,
    InReview,
    Published,
}

/// A nested model.
#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    #[validate(len = 1..=16)]
    pub name: String,
}

/// A model using most supported field types.
#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: Id<Project>,
    pub owner: String,
    #[validate(len = 1..=80)]
    pub title: String,
    pub status: Status,
    pub tags: Vec<Tag>,
    pub note: Option<String>,
    pub version: u64,
    #[validate(range = 0.0..=1.0)]
    pub score: f64,
    pub created_at: Timestamp,
}

/// A directly recursive model.
#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub body: String,
    pub replies: Vec<Comment>,
    pub parent: Option<Box<Self>>,
}

/// A generic model.
#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope<T> {
    pub item: T,
    #[validate(range = ..=100)]
    pub count: u32,
}

/// `#[model]` twins of the shapes `hyve-api` writes out by hand; tests check
/// they agree (JSON, fingerprint, schema).
pub mod twins {
    use hyve::model;

    #[model]
    #[derive(Debug, Clone, PartialEq)]
    pub struct Page<T> {
        pub items: Vec<T>,
        pub next_cursor: Option<String>,
        pub has_more: bool,
    }

    #[model]
    #[derive(Debug, Clone, PartialEq)]
    pub struct CursorParams {
        pub after: Option<String>,
        pub limit: Option<u32>,
    }
}

// ----- server functions -----

/// Server-only state; absent from client builds, which proves handler bodies
/// are removed there.
#[cfg(feature = "server")]
pub struct Db {
    pub owner: String,
}

#[model]
#[derive(Debug, Clone, PartialEq)]
pub struct CreateProject {
    #[validate(len = 1..=80)]
    pub title: String,
}

#[cfg_attr(not(feature = "server"), allow(dead_code))]
fn project(owner: &str, title: String) -> Project {
    Project {
        id: Id::from_ulid(Ulid::from_parts(1_700_000_000_000, 1)),
        owner: owner.into(),
        title,
        status: Status::Draft,
        tags: vec![],
        note: None,
        version: 1,
        score: 0.5,
        created_at: Timestamp::from_unix_micros(1_700_000_000_000_000).expect("in range"),
    }
}

/// Creates a project (one model argument, validated).
#[server(POST "/projects")]
pub async fn create_project(
    #[server_ctx] db: Ctx<Db>,
    input: Valid<CreateProject>,
) -> HyveResult<Project> {
    let input = input.into_inner();
    if input.title == "taken" {
        return Err(HyveError::domain("title_taken", "that title is taken")
            .with_detail("title", input.title));
    }
    Ok(project(&db.owner, input.title))
}

/// One non-model argument (`GET` with a path placeholder).
#[server(GET "/projects/{id}")]
pub async fn get_project(id: Id<Project>) -> HyveResult<Project> {
    let _ = id;
    Err(HyveError::not_found("project"))
}

/// Two transported arguments (a generated request model) plus an injected one.
#[server(PATCH "/projects/{id}")]
pub async fn rename_project(
    #[server_ctx] db: Ctx<Db>,
    id: Id<Project>,
    title: Valid<Tag>,
) -> HyveResult<Project> {
    let mut p = project(&db.owner, title.into_inner().name);
    p.id = id;
    Ok(p)
}

/// No arguments, no value.
#[server(DELETE "/sessions")]
pub async fn logout() -> HyveResult<()> {
    Ok(())
}
