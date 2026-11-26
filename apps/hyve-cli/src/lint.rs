//! Workspace lint and macro inspection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::process::Stdio;
use std::{io::Write, path::Path, process::Command};

use hyve_core::Value;
use hyve_diagnostics::boundary::{CORE, FORBIDDEN};

use crate::{Fail, Workspace, run_cargo, unknown_option};

pub(super) fn run(args: &[String]) -> Result<(), Fail> {
    if args.first().is_some_and(|s| s == "--schema") {
        let [_, base, current] = args else {
            return Err(Fail::Usage(
                "lint --schema needs a baseline and a current OpenAPI document".into(),
                None,
            ));
        };
        return compare(Path::new(base), Path::new(current));
    }
    if matches!(args.first(), Some(flag) if flag == "--expand") {
        let ws = Workspace::find()?;
        let mut cmd = ws.cargo("expand");
        cmd.current_dir(std::env::current_dir().map_err(|e| Fail::Failed(e.to_string(), None))?);
        cmd.args(&args[1..]);
        return run_cargo(cmd);
    }
    if let Some(arg) = args.first() {
        return Err(unknown_option("lint", arg, &["--expand", "--schema"]));
    }
    let ws = Workspace::find()?;
    let mut fmt = ws.cargo("fmt");
    fmt.args(["--all", "--", "--check"]);
    run_cargo(fmt)?;
    check_boundaries(&ws)?;
    ws.run_stages("codegen", &["--check"])?;
    ws.run_stages("lint", &[])?;
    if let Some(api) = &ws.api_schema {
        check_api(&ws, api)?;
    }
    let mut clippy = ws.cargo("clippy");
    clippy.args(["--workspace", "--all-targets"]);
    for feature in ws.test_features(&[])? {
        clippy.arg("--features").arg(feature);
    }
    clippy.args(["--", "-D", "warnings"]);
    run_cargo(clippy)
}

fn compare(base: &Path, current: &Path) -> Result<(), Fail> {
    let file = |p: &Path| {
        p.canonicalize().map_err(|e| {
            Fail::Failed(
                format!("cannot open API document {}: {e}", p.display()),
                None,
            )
        })
    };
    let tool = std::env::var_os("HYVE_OASDIFF").unwrap_or_else(|| "oasdiff".into());
    let status = Command::new(tool)
        .args(["breaking", "--fail-on", "ERR", "--open=false"])
        .arg(file(base)?)
        .arg(file(current)?)
        .status()
        .map_err(|e| {
            Fail::Failed(
                format!("could not run oasdiff: {e}"),
                Some("install oasdiff v1.41.0 or set HYVE_OASDIFF to the executable".into()),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Fail::Child(status.code()))
    }
}

fn check_api(ws: &Workspace, api: &Value) -> Result<(), Fail> {
    let config = || {
        Fail::Failed(
            "invalid [workspace.metadata.hyve.api]".into(),
            Some("set baseline and either current, or package and example".into()),
        )
    };
    let string = |key: &str| match api.get(key) {
        Some(Value::String(v)) if !v.is_empty() => Some(v.as_str()),
        _ => None,
    };
    let base = ws.root().join(string("baseline").ok_or_else(config)?);
    match (string("current"), string("package"), string("example")) {
        (Some(current), None, None) => compare(&base, &ws.root().join(current)),
        (None, Some(package), Some(example)) => {
            let mut export = ws.cargo("run");
            export
                .args(["--quiet", "-p", package, "--example", example])
                .stderr(Stdio::inherit());
            for feature in ws.test_features(&[package.into()])? {
                export.arg("--features").arg(feature);
            }
            let result = export.output().map_err(|e| {
                Fail::Failed(format!("could not export API from {example}: {e}"), None)
            })?;
            if !result.status.success() {
                return Err(Fail::Child(result.status.code()));
            }
            let io = |e: std::io::Error| {
                Fail::Failed(format!("could not write current API document: {e}"), None)
            };
            let mut current = tempfile::Builder::new()
                .suffix(".json")
                .tempfile()
                .map_err(io)?;
            current.write_all(&result.stdout).map_err(io)?;
            compare(&base, current.path())
        }
        _ => Err(config()),
    }
}

fn invalid_metadata() -> Fail {
    Fail::Failed("invalid Cargo dependency metadata".into(), None)
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], Fail> {
    match value.get(key) {
        Some(Value::List(v)) => Ok(v),
        _ => Err(invalid_metadata()),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, Fail> {
    match value.get(key) {
        Some(Value::String(v)) => Ok(v),
        _ => Err(invalid_metadata()),
    }
}

fn check_boundaries(ws: &Workspace) -> Result<(), Fail> {
    let out = ws
        .cargo("metadata")
        .args(["--format-version", "1", "--all-features"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| Fail::Failed(e.to_string(), None))?;
    if !out.status.success() {
        return Err(Fail::Child(out.status.code()));
    }
    let text = std::str::from_utf8(&out.stdout).map_err(|_| invalid_metadata())?;
    let meta = hyve_serde::from_json::<Value>(text).map_err(|_| invalid_metadata())?;
    let mut names = BTreeMap::new();
    for p in list(&meta, "packages")? {
        names.insert(field(p, "id")?, field(p, "name")?);
    }
    let mut edges = BTreeMap::new();
    let resolve = meta.get("resolve").ok_or_else(invalid_metadata)?;
    for node in list(resolve, "nodes")? {
        let mut dependencies = Vec::new();
        for dep in list(node, "deps")? {
            if list(dep, "dep_kinds")?
                .iter()
                .any(|kind| kind.get("kind") == Some(&Value::Null))
            {
                dependencies.push(field(dep, "pkg")?);
            }
        }
        edges.insert(field(node, "id")?, dependencies);
    }
    for (&root, name) in &names {
        if !CORE.contains(name) {
            continue;
        }
        let mut seen = BTreeSet::from([root]);
        let mut paths = VecDeque::from([vec![root]]);
        while let Some(path) = paths.pop_front() {
            let id = path[path.len() - 1];
            for &next in edges.get(id).ok_or_else(invalid_metadata)? {
                if !seen.insert(next) {
                    continue;
                }
                let mut chain = path.clone();
                chain.push(next);
                let dependency = names.get(next).ok_or_else(invalid_metadata)?;
                if let Some((_, domain)) = FORBIDDEN.iter().find(|(name, _)| name == dependency) {
                    let message = chain
                        .iter()
                        .map(|id| names[id])
                        .collect::<Vec<_>>()
                        .join(" -> ");
                    return Err(Fail::Failed(
                        format!("core dependency on {domain}: {message}"),
                        None,
                    ));
                }
                paths.push_back(chain);
            }
        }
    }
    Ok(())
}
