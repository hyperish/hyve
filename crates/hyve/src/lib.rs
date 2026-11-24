//! `hyve` — the single dependency for the Hyperish core: models, validation,
//! codecs, reactive state, contracts and the procedural macros that tie them
//! together.
//!
//! ```
//! use hyve::prelude::*;
//!
//! #[model]
//! pub struct SceneConfig {
//!     #[validate(len = 1..=80)]
//!     pub name: String,
//!     #[validate(range = 1..=240)]
//!     pub target_fps: u32,
//! }
//!
//! let scene = SceneConfig { name: "atrium".into(), target_fps: 120 };
//! let json = hyve::serde::json::to_json(&scene);
//! assert_eq!(json, r#"{"name":"atrium","targetFps":120}"#);
//! assert!(Valid::new(scene).is_ok());
//! ```
//!
//! Startup configuration is a model too ([`config`]):
//!
//! ```no_run
//! use hyve::prelude::*;
//!
//! #[model]
//! pub struct EngineConfig {
//!     #[validate(range = 30..=240)]
//!     pub max_fps: u32,
//! }
//!
//! fn main() -> HyveResult<()> {
//!     // `[app] max_fps = 144` in hyve.toml, `APP_MAX_FPS=144`, or `--set max_fps=144`.
//!     let cfg = config::load::<EngineConfig>()?;
//!     let fps = signal(cfg.max_fps);
//!     fps.set(120);
//!     Ok(())
//! }
//! ```
//!
//! | Module | Crate | What |
//! |---|---|---|
//! | crate root | `hyve-core` | errors, IDs, time, correlation, cancellation, scopes, ring buffer |
//! | [`state`] | `hyve-state` | `signal`, `memo`, `effect`, owner scopes |
//! | [`serde`] | `hyve-serde` | binary (fast rail) and JSON (standard rail) codecs, fingerprints |
//! | [`validate`] | `hyve-validate` | `Valid<T>`, rules, schema metadata |
//! | [`api`] | `hyve-api` | server function contracts, transports, `Page`, `Path`, `Query` |
//! | [`config`] | `hyve-config` | startup configuration (`hyve.toml`, `.env`, environment, `--set`) |
//! | [`parser`], [`diagnostics`] | `hyve-parser`, `hyve-diagnostics` | shared grammars and diagnostics |

extern crate self as hyve;

pub use hyve_core::*;
pub use hyve_macros::{model, server, server_ctx, test, validate};

pub use hyve_api as api;
pub use hyve_config as config;
pub use hyve_diagnostics as diagnostics;
pub use hyve_parser as parser;
pub use hyve_serde as serde;
pub use hyve_state as state;
pub use hyve_validate as validate;

/// Everything most code needs: `use hyve::prelude::*;`.
///
/// `#[hyve::test]` is not in the prelude (it would clash with the built-in
/// `#[test]`); write it with its path.
pub mod prelude {
    pub use hyve_api::{CursorParams, Page, Path, Query};
    pub use hyve_config as config;
    pub use hyve_core::{
        CancellationToken, Ctx, ErrorKind, HyveError, HyveResult, Id, RequestId, Scope, Timestamp,
        TraceId, Ulid, Value,
    };
    pub use hyve_macros::{model, server, server_ctx, validate};
    pub use hyve_state::{
        Effect, LocalSignal, Memo, Owner, Signal, batch, effect, local_signal, memo, on_cleanup,
        signal, untrack,
    };
    pub use hyve_validate::Valid;
}

#[doc(hidden)]
pub mod __private {
    pub use hyve_api as api;
    pub use hyve_core as core;
    pub use hyve_serde as serde;
    pub use hyve_serde::__private::rkyv;
    pub use hyve_validate as validate;

    use std::future::Future;

    /// Marker naming the test runtime in `run_test`'s bound.
    pub struct TestRuntime;

    /// Implemented when the `test` feature provides a runtime.
    #[diagnostic::on_unimplemented(
        message = "`#[hyve::test]` needs the `test` feature of the `hyve` crate",
        label = "no test runtime",
        note = "enable it for tests: `[dev-dependencies] hyve = {{ version = \"…\", features = [\"test\"] }}`"
    )]
    pub trait RuntimeAvailable {}

    #[cfg(all(feature = "test", not(target_arch = "wasm32")))]
    impl RuntimeAvailable for TestRuntime {}

    /// Drives an async test body on a `tokio` current-thread runtime inside a
    /// fresh root scope (`#[hyve::test]`).
    pub fn run_test<F: Future, R: RuntimeAvailable>(future: F) -> F::Output {
        #[cfg(all(feature = "test", not(target_arch = "wasm32")))]
        {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("hyve: failed to start the test runtime");
            let scope = hyve_core::Scope::root();
            let out = runtime.block_on(scope.scoped(future));
            scope.dispose();
            out
        }
        #[cfg(not(all(feature = "test", not(target_arch = "wasm32"))))]
        {
            drop(future);
            unreachable!("`RuntimeAvailable` is only implemented with the `test` feature")
        }
    }
}
