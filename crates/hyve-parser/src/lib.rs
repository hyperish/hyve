//! `hyve-parser` — small, zero-copy grammars shared by the Hyperish macros and
//! runtimes. Every parser reports failures as a [`Diagnostic`] whose spans point
//! into the input, so callers can re-anchor them (a string literal inside a macro
//! invocation, a line of a `.env` file…).
//!
//! - [`parse_duration`]: `500ms`, `30s`, `1h30m`, `7d`.
//! - [`parse_interpolation`]: `"Count: {count()}"`-style text with `{expr[:spec]}`.
//! - [`parse_dotenv`]: `.env` files.
//!
//! This crate is a build dependency of the proc-macro crates, so it never depends
//! on `hyve-macros` or the `hyve` facade (`docs/02-architecture.md` §2.3).

mod dotenv;
mod duration;
mod interpolation;
mod toml;

pub use dotenv::{DotenvEntry, parse_dotenv};
pub use duration::parse_duration;
pub use hyve_diagnostics::{Diagnostic, Span};
pub use interpolation::{Segment, parse_interpolation};
pub use toml::{Spanned, TomlTable, TomlValue, parse_toml};
