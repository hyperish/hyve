//! Core boundary (`docs/05-implementation-plan.md` §3, `docs/02-architecture.md`
//! §2.1): the normal dependency graph of the core crates has no async runtime,
//! HTTP, DOM or GPU crate, on any target.
//!
//! The graph comes from `Cargo.lock`, except that workspace members contribute
//! only their normal dependencies (`Cargo.lock` also lists their
//! dev-dependencies), read from their manifests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use hyve::parser::{TomlTable, TomlValue, parse_toml};

use hyve::diagnostics::boundary::{CORE, FORBIDDEN};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `name -> dependency names` from `Cargo.lock`.
fn lock_graph() -> BTreeMap<String, Vec<String>> {
    let text = std::fs::read_to_string(root().join("Cargo.lock")).expect("Cargo.lock");
    let mut graph = BTreeMap::new();
    let mut name: Option<String> = None;
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_deps = false;
            name = None;
        } else if let Some(n) = line.strip_prefix("name = ") {
            let n = n.trim_matches('"').to_owned();
            graph.entry(n.clone()).or_insert_with(Vec::new);
            name = Some(n);
        } else if line == "dependencies = [" {
            in_deps = true;
        } else if in_deps && line == "]" {
            in_deps = false;
        } else if in_deps {
            // ` "name"`, ` "name 1.2.3"` or ` "name 1.2.3 (registry+...)"`.
            let dep = line
                .trim_matches(|c| c == '"' || c == ',')
                .split(' ')
                .next()
                .unwrap_or_default();
            if let Some(n) = &name {
                graph.get_mut(n).unwrap().push(dep.to_owned());
            }
        }
    }
    graph
}

/// Normal dependencies (all targets) of a workspace member, or `None` if
/// `name` is not one.
fn member_deps(name: &str) -> Option<Vec<String>> {
    let path = ["crates", "apps"]
        .iter()
        .map(|d| root().join(d).join(name).join("Cargo.toml"))
        .find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&path).unwrap();
    let manifest =
        parse_toml(&text).unwrap_or_else(|d| panic!("{}: {}", path.display(), d.summary()));
    let mut out = Vec::new();
    let mut add = |deps: &TomlTable| {
        for (k, v) in deps.entries() {
            let renamed = match &v.value {
                TomlValue::Table(t) => t.get("package").and_then(|p| match &p.value {
                    TomlValue::String(s) => Some(s.clone()),
                    _ => None,
                }),
                _ => None,
            };
            out.push(renamed.unwrap_or_else(|| k.value.clone()));
        }
    };
    let table = |t: &TomlTable, key: &str| match t.get(key).map(|v| &v.value) {
        Some(TomlValue::Table(t)) => Some(t.clone()),
        _ => None,
    };
    if let Some(deps) = table(&manifest, "dependencies") {
        add(&deps);
    }
    if let Some(targets) = table(&manifest, "target") {
        for (_, cfg) in targets.entries() {
            if let TomlValue::Table(cfg) = &cfg.value {
                if let Some(deps) = table(cfg, "dependencies") {
                    add(&deps);
                }
            }
        }
    }
    Some(out)
}

/// Every crate reachable from `start`, with the chain that reaches it.
fn reachable(start: &str, lock: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let mut seen = BTreeMap::new();
    let mut queue = VecDeque::from([vec![start.to_owned()]]);
    while let Some(chain) = queue.pop_front() {
        let name = chain.last().unwrap().clone();
        if seen.contains_key(&name) {
            continue;
        }
        let deps =
            member_deps(&name).unwrap_or_else(|| lock.get(&name).cloned().unwrap_or_default());
        for d in deps {
            let mut next = chain.clone();
            next.push(d);
            queue.push_back(next);
        }
        seen.insert(name, chain);
    }
    seen
}

#[test]
fn core_crates_stay_free_of_runtime_http_dom_and_gpu_dependencies() {
    let lock = lock_graph();
    let mut violations = BTreeSet::new();
    for core in CORE {
        let graph = reachable(core, &lock);
        for (name, what) in FORBIDDEN {
            if let Some(chain) = graph.get(name) {
                violations.insert(format!("{what}: {}", chain.join(" -> ")));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "core boundary broken:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_check_sees_real_dependencies() {
    let lock = lock_graph();
    assert!(reachable("hyve-serde", &lock).contains_key("rkyv"));
    assert!(reachable("hyve-core", &lock).contains_key("getrandom"));
    // The facade's optional, native-only test runtime is found, so a core
    // crate depending on it would be too.
    assert!(reachable("hyve", &lock).contains_key("tokio"));
    // Dev-dependencies of members do not count.
    assert!(!reachable("hyve-state", &lock).contains_key("wasm-bindgen-test"));
}
