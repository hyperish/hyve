//! The Cargo workspace `hyve` works on, from `cargo metadata`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hyve_core::Value;
use hyve_serde::from_json;

use crate::Fail;

/// Frameworks whose stages `hyve` calls through `hyve-<framework>`, in order.
const FRAMEWORKS: [&str; 2] = ["orbic", "serge"];

pub(crate) struct MobileApp {
    has_widgets: bool,
    platform: &'static str,
    pub name: String,
    pub dir: PathBuf,
    pub settings: Value,
}

pub(crate) struct Workspace {
    root: PathBuf,
    pub(crate) api_schema: Option<Value>,
    /// Frameworks some member depends on directly.
    frameworks: Vec<&'static str>,
    /// Member directories and their qualified orbic server features.
    server_features: Vec<(PathBuf, Vec<String>)>,
    mobile_apps: Vec<MobileApp>,
    pub target_dir: PathBuf,
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
        let mut server_features = Vec::new();
        let mut mobile_apps = Vec::new();
        if let Some(Value::List(packages)) = meta.get("packages") {
            for p in packages {
                if let (Some(Value::String(name)), Some(Value::String(manifest))) =
                    (p.get("name"), p.get("manifest_path"))
                {
                    for platform in ["android", "ios"] {
                        if let (Some(settings @ Value::Map(_)), Some(dir)) = (
                            p.get("metadata").and_then(|m| m.get(platform)),
                            Path::new(manifest).parent(),
                        ) {
                            mobile_apps.push(MobileApp {
                                has_widgets: p
                                    .get("metadata")
                                    .and_then(|m| m.get("serge"))
                                    .and_then(|s| s.get("widgets"))
                                    .is_some(),
                                platform,
                                name: name.clone(),
                                dir: dir.into(),
                                settings: settings.clone(),
                            });
                        }
                    }
                }
                if let Some(server) = declared_server_features(p) {
                    server_features.push(server);
                }
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
        let target_dir = match meta.get("target_directory") {
            Some(Value::String(dir)) => PathBuf::from(dir),
            _ => root.join("target"),
        };
        Ok(Workspace {
            target_dir,
            root,
            api_schema: meta
                .get("metadata")
                .and_then(|m| m.get("hyve"))
                .and_then(|m| m.get("api"))
                .cloned(),
            frameworks,
            server_features,
            mobile_apps,
        })
    }

    /// Whether a member depends on `orbic` or `serge` directly.
    pub(crate) fn uses_frameworks(&self) -> bool {
        !self.frameworks.is_empty()
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// `cargo <subcommand>` run in the workspace root.
    pub(crate) fn cargo(&self, subcommand: &str) -> Command {
        let mut cmd = Command::new(cargo());
        cmd.arg(subcommand).current_dir(&self.root);
        cmd
    }

    /// Resolves Cargo package names, globs and package ID specifications.
    pub(crate) fn selected_roots(&self, packages: &[String]) -> Result<Option<String>, Fail> {
        if packages.is_empty() {
            Ok(None)
        } else {
            let mut tree = self.cargo("tree");
            crate::select(&mut tree, packages);
            let out = tree
                .args([
                    "--depth", "0", "--prefix", "none", "--format", "{p}", "--color", "never",
                ])
                .stderr(Stdio::inherit())
                .output()
                .map_err(|e| Fail::Failed(format!("could not run cargo: {e}"), None))?;
            if !out.status.success() {
                return Err(Fail::Child(out.status.code()));
            }
            Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
        }
    }

    /// Mobile targets explicitly declared by selected application members.
    pub(crate) fn mobile_packages(
        &self,
        platform: &str,
        roots: Option<&str>,
    ) -> Result<Vec<&MobileApp>, Fail> {
        let available = self
            .mobile_apps
            .iter()
            .filter(|app| app.platform == platform);
        let apps: Vec<_> =
            match roots {
                Some(roots) => roots
                    .lines()
                    .filter_map(|line| line.split_whitespace().next())
                    .map(|name| {
                        available.clone().find(|app| app.name == name).ok_or_else(|| Fail::Failed(
                    format!("{name} has no {platform} application metadata"),
                    Some(format!("add [package.metadata.{platform}] to the application manifest")),
                ))
                    })
                    .collect::<Result<_, _>>()?,
                None => available.collect(),
            };
        if apps.is_empty() {
            return Err(Fail::Failed(
                format!("no {platform} application selected"),
                Some(format!(
                    "select an app declaring [package.metadata.{platform}]"
                )),
            ));
        }
        Ok(apps)
    }

    /// Features for the declared servers among the packages under test.
    pub(crate) fn test_features(&self, packages: &[String]) -> Result<Vec<&str>, Fail> {
        let roots = if self.server_features.is_empty() {
            None
        } else {
            self.selected_roots(packages)?
        };
        let mut features = Vec::new();
        for (dir, declared) in &self.server_features {
            if let Some(roots) = &roots {
                // Cargo prints a path package's directory in parentheses. A
                // dependency can share the member's name and version.
                let source = format!(" ({})", dir.display());
                if !roots.lines().any(|line| line.ends_with(&source)) {
                    continue;
                }
            }
            features.extend(declared.iter().map(String::as_str));
        }
        Ok(features)
    }

    /// Runs `stage` of every framework the workspace uses that provides it,
    /// `orbic` before `serge` (`docs/01-dx-and-dsl.md` §2, 프레임워크 단계
    /// 호출): `hyve-<framework> stages` says the executable's version (first
    /// line) and its stages (one per line); the version must equal the
    /// framework's in the workspace's `Cargo.lock`.
    pub(crate) fn widget_sources(&self, app: &MobileApp) -> Result<Option<PathBuf>, Fail> {
        if !app.has_widgets {
            return Ok(None);
        }
        let providers = self.tools_for("widgets")?;
        let [tool] = providers.as_slice() else {
            return Err(Fail::Failed(
                "this app's widgets require a matching hyve-serge with widget generation".into(),
                None,
            ));
        };
        let out = Command::new(tool)
            .args(["widgets", "-p", &app.name])
            .current_dir(&self.root)
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| Fail::Failed(e.to_string(), None))?;
        if !out.status.success() {
            return Err(Fail::Child(out.status.code()));
        }
        match from_json::<Value>(&String::from_utf8_lossy(&out.stdout)) {
            Ok(Value::String(s)) if Path::new(&s).is_dir() => Ok(Some(s.into())),
            _ => Err(Fail::Failed(
                format!("no generated widget sources for {}", app.name),
                None,
            )),
        }
    }

    /// Runs `stage` in every framework tool that provides it (orbic, then
    /// serge); returns how many ran it.
    pub(crate) fn run_stages(&self, stage: &str, args: &[&str]) -> Result<usize, Fail> {
        let tools = self.tools_for(stage)?;
        for exe in &tools {
            let status = Command::new(exe)
                .arg(stage)
                .args(args)
                .current_dir(&self.root)
                .status()
                .map_err(|e| Fail::Failed(format!("`{exe}` could not be run: {e}"), None))?;
            if !status.success() {
                return Err(Fail::Child(status.code()));
            }
        }
        Ok(tools.len())
    }

    /// Runs `stage` in every framework tool that provides it, all at once
    /// (`dev`: long-running servers and windows). Each tool's stdin is a
    /// pipe from `hyve` and `HYVE_STAGE_LIFELINE=stdin` tells it so: when the
    /// pipe closes (`hyve` stops it or exits), the tool stops what it
    /// started and exits. When any tool ends (a window closed, a failure),
    /// the development session ends: `hyve` stops the others and ends with that
    /// tool's exit code. Returns how many ran.
    pub(crate) fn run_stage_together(&self, stage: &str, args: &[&str]) -> Result<usize, Fail> {
        use std::time::{Duration, Instant};
        let tools = self.tools_for(stage)?;
        let mut running = Vec::new();
        for exe in &tools {
            match Command::new(exe)
                .arg(stage)
                .args(args)
                .current_dir(&self.root)
                .env("HYVE_STAGE_LIFELINE", "stdin")
                .stdin(Stdio::piped())
                .spawn()
            {
                Ok(child) => running.push((exe.clone(), child)),
                Err(e) => {
                    stop_all(&mut running);
                    return Err(Fail::Failed(format!("`{exe}` could not be run: {e}"), None));
                }
            }
        }
        loop {
            let mut ended = None;
            running.retain_mut(|(_, child)| match child.try_wait() {
                Ok(Some(status)) => {
                    ended.get_or_insert(status);
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            });
            if let Some(status) = ended {
                stop_all(&mut running);
                return if status.success() {
                    Ok(tools.len())
                } else {
                    Err(Fail::Child(status.code()))
                };
            }
            // No tool provides the stage (or none can be waited on).
            if running.is_empty() {
                return Ok(tools.len());
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        /// Closes every tool's stdin, waits a moment for it to stop what it
        /// started, then kills what is left.
        fn stop_all(running: &mut Vec<(String, std::process::Child)>) {
            for (_, child) in running.iter_mut() {
                drop(child.stdin.take());
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                running.retain_mut(|(_, c)| matches!(c.try_wait(), Ok(None)));
                if running.is_empty() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            for (exe, child) in running.iter_mut() {
                eprintln!("hyve: `{exe}` did not stop when asked; killing it");
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// The framework tools that provide `stage` (orbic, then serge), after
    /// checking each one's version against the workspace's `Cargo.lock`.
    fn tools_for(&self, stage: &str) -> Result<Vec<String>, Fail> {
        let mut tools = Vec::new();
        if self.frameworks.is_empty() {
            return Ok(tools);
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
            if lines.clone().any(|l| l == stage) {
                tools.push(exe);
            } else if stage == "dev" && lines.any(|l| l == "preview") {
                return Err(Fail::Failed(
                    format!("`{exe}` still advertises the old `preview` stage"),
                    Some(format!(
                        "update `{framework}-cli` and the workspace's `{framework}` dependency together for `hyve dev`"
                    )),
                ));
            }
        }
        Ok(tools)
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

/// The declaring member's directory and features as accepted by `cargo test`.
fn declared_server_features(package: &Value) -> Option<(PathBuf, Vec<String>)> {
    let declared = package.get("metadata")?.get("orbic")?.get("server")?;
    let (Value::List(features), Value::String(name), Value::String(manifest)) = (
        declared.get("features")?,
        package.get("name")?,
        package.get("manifest_path")?,
    ) else {
        return None;
    };
    let features: Vec<_> = features
        .iter()
        .filter_map(|feature| match feature {
            Value::String(f) if f.contains('/') => Some(f.clone()),
            Value::String(f) => Some(format!("{name}/{f}")),
            _ => None,
        })
        .collect();
    let dir = Path::new(manifest).parent()?;
    (!features.is_empty()).then(|| (dir.to_owned(), features))
}

/// The `cargo` that runs `hyve`'s commands: `$CARGO` when set (inside a
/// Cargo invocation), otherwise `cargo` from `PATH`.
fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
