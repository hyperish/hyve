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

    /// Whether a member depends on `orbic` or `serge` directly.
    pub(crate) fn uses_frameworks(&self) -> bool {
        !self.frameworks.is_empty()
    }

    /// `cargo <subcommand>` run in the workspace root.
    pub(crate) fn cargo(&self, subcommand: &str) -> Command {
        let mut cmd = Command::new(cargo());
        cmd.arg(subcommand).current_dir(&self.root);
        cmd
    }

    /// Runs `stage` of every framework the workspace uses that provides it,
    /// `orbic` before `serge` (`docs/01-dx-and-dsl.md` §2, 프레임워크 단계
    /// 호출): `hyve-<framework> stages` says the executable's version (first
    /// line) and its stages (one per line); the version must equal the
    /// framework's in the workspace's `Cargo.lock`.
    pub(crate) fn run_stages(&self, stage: &str, args: &[&str]) -> Result<(), Fail> {
        if self.frameworks.is_empty() {
            return Ok(());
        }
        let locked = self.locked_versions()?;
        for framework in &self.frameworks {
            let exe = format!("hyve-{framework}");
            let want = locked
                .iter()
                .find(|(n, _)| n == framework)
                .map(|(_, v)| v.as_str());
            let install = format!(
                "install it: `cargo install {framework}-cli{}`",
                want.map(|v| format!(" --version ={v}")).unwrap_or_default()
            );
            let out = Command::new(&exe)
                .arg("stages")
                .current_dir(&self.root)
                .output()
                .map_err(|e| {
                    Fail::Failed(
                        format!("`{exe}` could not be run: {e}"),
                        Some(install.clone()),
                    )
                })?;
            if !out.status.success() {
                return Err(Fail::Failed(
                    format!("`{exe} stages` failed"),
                    Some(install),
                ));
            }
            let text = String::from_utf8_lossy(&out.stdout);
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            let version = lines.next().unwrap_or_default().to_owned();
            if let Some(want) = want.filter(|w| *w != version) {
                return Err(Fail::Failed(
                    format!(
                        "`{exe}` is {framework} {version}, but this workspace uses {framework} {want}"
                    ),
                    Some(install),
                ));
            }
            if !lines.any(|l| l == stage) {
                continue;
            }
            let status = Command::new(&exe)
                .arg(stage)
                .args(args)
                .current_dir(&self.root)
                .status()
                .map_err(|e| {
                    Fail::Failed(format!("`{exe}` could not be run: {e}"), Some(install))
                })?;
            if !status.success() {
                return Err(Fail::Child(status.code()));
            }
        }
        Ok(())
    }

    /// `(name, version)` of the frameworks in `Cargo.lock` (written by a full
    /// `cargo metadata` when missing).
    fn locked_versions(&self) -> Result<Vec<(String, String)>, Fail> {
        let lock = self.root.join("Cargo.lock");
        if !lock.exists() {
            let out = self
                .cargo("metadata")
                .args(["--format-version", "1"])
                .output()
                .map_err(|e| Fail::Failed(format!("could not run cargo: {e}"), None))?;
            if !out.status.success() {
                return Err(Fail::Failed(
                    "`cargo metadata` could not resolve the workspace".into(),
                    None,
                ));
            }
        }
        let text = std::fs::read_to_string(&lock)
            .map_err(|e| Fail::Failed(format!("could not read Cargo.lock: {e}"), None))?;
        let mut found = Vec::new();
        for block in text.split("[[package]]").skip(1) {
            let field = |key: &str| {
                block.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
                })
            };
            if let (Some(name), Some(version)) = (field("name"), field("version")) {
                if FRAMEWORKS.contains(&name.as_str()) {
                    found.push((name, version));
                }
            }
        }
        Ok(found)
    }
}

/// The `cargo` that runs `hyve`'s commands: `$CARGO` when set (inside a
/// Cargo invocation), otherwise `cargo` from `PATH`.
fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
