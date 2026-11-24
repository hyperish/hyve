//! The Cargo workspace `hyve` works on, from `cargo metadata`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use hyve_core::Value;
use hyve_serde::from_json;

use crate::Fail;

/// Frameworks whose stages `hyve` calls through `hyve-<framework>`, in order.
const FRAMEWORKS: [&str; 2] = ["orbic", "serge"];

pub(crate) struct Workspace {
    root: PathBuf,
    /// Frameworks some member depends on directly.
    frameworks: Vec<&'static str>,
}

impl Workspace {
    /// The workspace enclosing the working directory.
    pub(crate) fn find() -> Result<Self, Fail> {
        let out = Command::new(cargo())
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .output()
            .map_err(|e| {
                Fail::Failed(
                    format!("could not run cargo: {e}"),
                    Some("install Rust with rustup: https://rustup.rs".into()),
                )
            })?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr);
            let why = why
                .lines()
                .find(|l| l.starts_with("error"))
                .unwrap_or("cargo metadata failed")
                .trim_start_matches("error: ");
            return Err(Fail::Failed(
                format!("no Cargo workspace here: {why}"),
                Some("run `hyve` inside a Cargo workspace (a directory with Cargo.toml, or below one)".into()),
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let meta = from_json::<Value>(&text)
            .map_err(|e| Fail::Failed(format!("unexpected `cargo metadata` output: {e}"), None))?;
        let root = match meta.get("workspace_root") {
            Some(Value::String(s)) => PathBuf::from(s),
            _ => {
                return Err(Fail::Failed(
                    "unexpected `cargo metadata` output: no workspace_root".into(),
                    None,
                ));
            }
        };
        let mut frameworks = Vec::new();
        if let Some(Value::List(packages)) = meta.get("packages") {
            for p in packages {
                if let Some(Value::List(deps)) = p.get("dependencies") {
                    for d in deps {
                        if let Some(Value::String(name)) = d.get("name") {
                            if let Some(f) = FRAMEWORKS.iter().find(|f| *f == name) {
                                if !frameworks.contains(f) {
                                    frameworks.push(*f);
                                }
                            }
                        }
                    }
                }
            }
        }
        frameworks.sort_by_key(|f| FRAMEWORKS.iter().position(|x| x == f));
        Ok(Workspace { root, frameworks })
    }

    /// `cargo <subcommand>` run in the workspace root.
    pub(crate) fn cargo(&self, subcommand: &str) -> Command {
        let mut cmd = Command::new(cargo());
        cmd.arg(subcommand).current_dir(&self.root);
        cmd
    }

    /// Fails when the workspace needs framework stages, which this release
    /// cannot call yet (`docs/01-dx-and-dsl.md` §2, "프레임워크 단계 호출").
    pub(crate) fn require_no_framework_stages(&self, command: &str) -> Result<(), Fail> {
        if self.frameworks.is_empty() {
            return Ok(());
        }
        let names: Vec<String> = self
            .frameworks
            .iter()
            .map(|f| format!("hyve-{f}"))
            .collect();
        Err(Fail::NotImplemented(format!(
            "`{command}` for a workspace that uses {} (calling the {} stages)",
            self.frameworks.join(" and "),
            names.join(" and ")
        )))
    }
}

/// The `cargo` that runs `hyve`'s commands: `$CARGO` when set (inside a
/// Cargo invocation), otherwise `cargo` from `PATH`.
fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
