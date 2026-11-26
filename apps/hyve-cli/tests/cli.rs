//! The CLI common contract (`docs/01-dx-and-dsl.md` §2), against throwaway
//! workspaces. Nested `cargo` runs are offline and use their own target
//! directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn temp(files: &[(&str, &str)]) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "hyve-cli-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    for (name, text) in files {
        let path = d.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hyve(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hyve"))
        .args(args)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn package(name: &str, lib: &str) -> [(String, String); 2] {
    [
        (
            "Cargo.toml".into(),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n"
            ),
        ),
        ("src/lib.rs".into(), lib.into()),
    ]
}

fn crate_dir(name: &str, lib: &str) -> PathBuf {
    let files = package(name, lib);
    temp(
        &files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn help_and_version() {
    let d = temp(&[]);
    let o = hyve(&d, &["--version"]);
    assert!(o.status.success());
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        concat!("hyve ", env!("CARGO_PKG_VERSION"))
    );
    let help = String::from_utf8_lossy(&hyve(&d, &["--help"]).stdout).into_owned();
    assert!(help.contains("Usage: hyve <command>"), "{help}");
    assert!(help.contains("dev [--web] [--native]"), "{help}");
    assert!(!help.contains("not in this release"), "{help}");
    let o = hyve(&d, &[]);
    assert_eq!(o.status.code(), Some(2), "no command is a usage error");
}

#[test]
fn unknown_commands_and_options_are_usage_errors() {
    let d = temp(&[]);
    let o = hyve(&d, &["tset"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("unknown command `tset`")
            && stderr(&o).contains("did you mean `hyve test`?"),
        "{}",
        stderr(&o)
    );

    let o = hyve(&d, &["test", "--relase"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("did you mean `--release`?"),
        "{}",
        stderr(&o)
    );
    assert_eq!(
        hyve(&d, &["build", "--release"]).status.code(),
        Some(2),
        "build is optimized by default; --release is unknown"
    );
    assert_eq!(hyve(&d, &["codegen", "--fix"]).status.code(), Some(2));
    assert_eq!(hyve(&d, &["dev", "--desktop"]).status.code(), Some(2));
    let o = hyve(&d, &["preview", "--web"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("renamed to `hyve dev`"),
        "{}",
        stderr(&o)
    );
    assert_eq!(hyve(&d, &["lint", "--fix"]).status.code(), Some(2));
}

#[test]
fn unknown_templates_are_usage_errors() {
    let d = temp(&[]);
    let o = hyve(&d, &["init", "--template", "fullstak"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("did you mean `fullstack`?"),
        "{}",
        stderr(&o)
    );
    assert_eq!(hyve(&d, &["init", "--template"]).status.code(), Some(2));
}

#[test]
fn lint_runs_format_and_clippy_on_the_workspace() {
    let d = crate_dir("linted", "pub fn answer() -> i32 {\n    42\n}\n");
    let clean = hyve(&d, &["lint"]);
    assert!(clean.status.success(), "{}", stderr(&clean));
    std::fs::write(d.join("src/lib.rs"), "pub fn unformatted()->i32{3}\n").unwrap();
    assert!(!hyve(&d, &["lint"]).status.success());
    std::fs::write(
        d.join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    return 42;\n}\n",
    )
    .unwrap();
    let clippy = hyve(&d, &["lint"]);
    assert!(!clippy.status.success());
    assert!(stderr(&clippy).contains("return"), "{}", stderr(&clippy));
}

#[test]
fn init_writes_a_template_and_never_overwrites() {
    let d = temp(&[]);
    // The default template is `fullstack`; the directory names the package.
    let o = hyve(&d, &["init", "notes-app"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let next = String::from_utf8_lossy(&o.stdout);
    assert!(next.contains("cd notes-app && hyve dev --web"), "{next}");
    let app = d.join("notes-app");
    let manifest = std::fs::read_to_string(app.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"notes-app\""), "{manifest}");
    assert!(
        manifest.contains("[package.metadata.orbic.server]"),
        "{manifest}"
    );
    assert!(manifest.contains("[package.metadata.serge]"), "{manifest}");
    for f in [
        "hyve.toml",
        ".gitignore",
        "migrations/0001_notes.sql",
        "src/notes.rs",
        "src/bin/server.rs",
    ] {
        assert!(app.join(f).is_file(), "{f}");
    }

    // A second run refuses, names the files, and writes nothing.
    std::fs::write(app.join("src/lib.rs"), "// mine\n").unwrap();
    let o = hyve(&d, &["init", "notes-app"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("already exist")
            && stderr(&o).contains("src/lib.rs")
            && stderr(&o).contains("--force"),
        "{}",
        stderr(&o)
    );
    assert_eq!(
        std::fs::read_to_string(app.join("src/lib.rs")).unwrap(),
        "// mine\n"
    );
    // `--force` overwrites.
    let o = hyve(&d, &["init", "--force", "notes-app"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_ne!(
        std::fs::read_to_string(app.join("src/lib.rs")).unwrap(),
        "// mine\n"
    );

    // The other templates, in the working directory.
    let svc = d.join("svc");
    std::fs::create_dir(&svc).unwrap();
    let o = hyve(&svc, &["init", "--template", "orbic-service"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(svc.join("migrations/0001_notes.sql").is_file());
    assert!(!svc.join("src/notes.rs").exists());
    let o = hyve(&d, &["init", "--template", "serge-app", "ui"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        !d.join("ui/hyve.toml").exists(),
        "a UI-only app needs no hyve.toml"
    );

    // A directory that cannot name a package.
    let o = hyve(&d, &["init", "1app"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!d.join("1app").exists());
}

#[test]
fn commands_outside_a_workspace_fail() {
    let d = temp(&[]);
    for cmd in ["test", "build", "codegen"] {
        let o = hyve(&d, &[cmd]);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        assert!(
            stderr(&o).contains("no Cargo workspace here"),
            "{cmd}: {}",
            stderr(&o)
        );
    }
}

#[test]
fn test_and_build_exit_codes_follow_cargo() {
    let pass = crate_dir("passing", "#[test]\nfn ok() {}\n");
    let o = hyve(&pass, &["test", "--", "--quiet"]);
    assert!(o.status.success(), "{}", stderr(&o));
    // Run from a subdirectory: the enclosing workspace is found.
    assert!(hyve(&pass.join("src"), &["test"]).status.success());

    let fail = crate_dir("failing", "#[test]\nfn broken() { panic!(\"no\") }\n");
    let o = hyve(&fail, &["test"]);
    assert_eq!(o.status.code(), Some(101), "cargo test's own exit code");

    let o = hyve(&pass, &["build"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(pass.join("target/release").is_dir(), "optimized by default");
    let o = hyve(&pass, &["build", "--debug", "-p", "passing"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(pass.join("target/debug").is_dir());
    let o = hyve(&pass, &["build", "-p", "nope"]);
    assert_ne!(o.status.code(), Some(0));

    let broken = crate_dir("broken", "pub fn f() -> u32 { \"x\" }\n");
    assert_eq!(
        hyve(&broken, &["build", "--debug"]).status.code(),
        Some(101)
    );
}

#[test]
fn test_enables_declared_server_features_for_selected_packages() {
    let d = temp(&[
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"notes-app\", \"ui\"]\nexclude = [\"legacy\"]\nresolver = \"3\"\n",
        ),
        (
            "notes-app/Cargo.toml",
            r#"
[package]
name = "notes-app"
version = "0.1.0"
edition = "2024"
[features]
server = []
sqlite = []
[package.metadata.orbic.server]
features = ["server", "sqlite"]
"#,
        ),
        ("notes-app/src/lib.rs", ""),
        (
            "notes-app/tests/notes.rs",
            "#![cfg(all(feature = \"server\", feature = \"sqlite\"))]\n#[test] fn notes() {}",
        ),
        (
            "ui/Cargo.toml",
            r#"
[package]
name = "ui"
version = "0.1.0"
edition = "2024"
[features]
server = []
[dependencies]
legacy = { package = "notes-app", path = "../legacy" }
"#,
        ),
        (
            "ui/src/lib.rs",
            "#[cfg(feature = \"server\")] compile_error!(\"unexpected server feature\");\n#[test] fn renders() {}",
        ),
        (
            "legacy/Cargo.toml",
            "[package]\nname = \"notes-app\"\nversion = \"0.0.1\"\nedition = \"2024\"",
        ),
        ("legacy/src/lib.rs", "#[test] fn legacy() {}"),
    ]);

    for (args, expected) in [
        (vec!["test"], vec!["notes", "renders"]),
        (
            vec!["test", "-p", "notes-*", "--", "--exact", "notes"],
            vec!["notes"],
        ),
        (vec!["test", "-p", "ui"], vec!["renders"]),
        (vec!["test", "-p", "notes-app@0.0.1"], vec!["legacy"]),
    ] {
        let o = hyve(&d, &args);
        assert!(o.status.success(), "{args:?}: {}", stderr(&o));
        let stdout = String::from_utf8_lossy(&o.stdout);
        for test in ["notes", "renders", "legacy"] {
            assert_eq!(
                stdout.contains(&format!("test {test} ... ok")),
                expected.contains(&test),
                "{args:?}: {stdout}"
            );
        }
    }
}

#[test]
fn codegen_without_frameworks_has_nothing_to_do() {
    let d = crate_dir("plain", "");
    assert!(hyve(&d, &["codegen"]).status.success());
    assert!(hyve(&d, &["codegen", "--check"]).status.success());
}

#[test]
#[ignore = "requires oasdiff v1.41.0 on PATH or HYVE_OASDIFF"]
fn lint_compares_a_fresh_export_against_the_released_api() {
    let root = crate_dir("api_fixture", "");
    let mut toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    toml.push_str("\n[workspace.metadata.hyve.api]\nbaseline='released.json'\npackage='api_fixture'\nexample='contract'\n");
    std::fs::write(root.join("Cargo.toml"), toml).unwrap();
    std::fs::create_dir_all(root.join("examples")).unwrap();
    std::fs::write(
        root.join("examples/contract.rs"),
        "fn main() {\n    println!(\"{}\", include_str!(\"../current.json\"));\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "\n").unwrap();
    let base = r#"{"openapi":"3.1.0","info":{"title":"Notes","version":"1.0.0"},"paths":{"/notes":{"get":{"parameters":[],"responses":{"200":{"description":"OK"}}}}}}"#;
    std::fs::write(root.join("released.json"), base).unwrap();
    let optional=base.replace(r#""parameters":[]"#,r#""parameters":[{"name":"search","in":"query","required":false,"schema":{"type":"string"}}]"#);
    std::fs::write(root.join("current.json"), &optional).unwrap();
    let passing = hyve(&root, &["lint"]);
    assert!(passing.status.success(), "{}", stderr(&passing));
    std::fs::write(
        root.join("current.json"),
        optional.replace("\"required\":false", "\"required\":true"),
    )
    .unwrap();
    for args in [
        vec!["lint"],
        vec!["lint", "--schema", "released.json", "current.json"],
    ] {
        let failing = hyve(&root, &args);
        assert_eq!(failing.status.code(), Some(1));
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&failing.stdout),
            stderr(&failing)
        );
        assert!(
            diagnostic.contains("search") && diagnostic.contains("/notes"),
            "{diagnostic}"
        );
    }
}

/// A workspace whose member depends on `orbic` (and `serge`) at the given
/// versions, through local stand-ins.
fn framework_workspace(orbic: &str, serge: Option<&str>) -> PathBuf {
    let serge_dep = serge.map_or(String::new(), |_| "serge = { path = \"serge\" }\n".into());
    let mut files = vec![
        (
            "Cargo.toml".to_owned(),
            format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\norbic = {{ path = \"orbic\" }}\n{serge_dep}\n[workspace]\n"
            ),
        ),
        ("src/lib.rs".to_owned(), String::new()),
        (
            "orbic/Cargo.toml".to_owned(),
            format!("[package]\nname = \"orbic\"\nversion = \"{orbic}\"\nedition = \"2024\"\n"),
        ),
        ("orbic/src/lib.rs".to_owned(), String::new()),
    ];
    if let Some(v) = serge {
        files.push((
            "serge/Cargo.toml".to_owned(),
            format!("[package]\nname = \"serge\"\nversion = \"{v}\"\nedition = \"2024\"\n"),
        ));
        files.push(("serge/src/lib.rs".to_owned(), String::new()));
    }
    temp(
        &files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect::<Vec<_>>(),
    )
}

/// A stand-in `hyve-<framework>` that reports `version` and `stages` and
/// logs each stage it runs to `log`.
#[cfg(unix)]
fn fake_tool(bin: &Path, framework: &str, version: &str, stages: &[&str], exit: i32) {
    use std::os::unix::fs::PermissionsExt;
    let path = bin.join(format!("hyve-{framework}"));
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = stages ]; then printf '%s\\n' {version} {stages}; exit 0; fi\necho \"{framework} $*\" >> \"$HYVE_FAKE_LOG\"\nexit {exit}\n",
        stages = stages.join(" ")
    );
    std::fs::create_dir_all(bin).unwrap();
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn hyve_with_tools(dir: &Path, bin: &Path, args: &[&str]) -> (Output, String) {
    let log = dir.join("stages.log");
    let _ = std::fs::remove_file(&log);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let o = Command::new(env!("CARGO_BIN_EXE_hyve"))
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
        .env("HYVE_FAKE_LOG", &log)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    (o, std::fs::read_to_string(&log).unwrap_or_default())
}

#[cfg(unix)]
#[test]
fn codegen_and_build_call_the_framework_stages() {
    let d = framework_workspace("0.1.0", Some("0.2.0"));
    let bin = d.join("bin");
    fake_tool(&bin, "orbic", "0.1.0", &["codegen"], 0);
    fake_tool(&bin, "serge", "0.2.0", &["build"], 0);

    let (o, log) = hyve_with_tools(&d, &bin, &["codegen"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        log, "orbic codegen\n",
        "only the stages a tool provides, in order"
    );
    let (o, log) = hyve_with_tools(&d, &bin, &["codegen", "--check"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(log, "orbic codegen --check\n");
    let (o, log) = hyve_with_tools(&d, &bin, &["build", "--debug"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(log, "orbic codegen\nserge build --debug\n");
    let (o, log) = hyve_with_tools(&d, &bin, &["build", "--debug", "-p", "ap*"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(log, "orbic codegen\nserge build --debug -p app\n");
    // The selected framework receives the UI target flags. Without a tool
    // providing the stage there is no local app or server to run.
    let (o, log) = hyve_with_tools(&d, &bin, &["dev", "--web"]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("no framework in this workspace provides a `dev` stage")
            && log.is_empty(),
        "{}",
        stderr(&o)
    );
    fake_tool(&bin, "serge", "0.2.0", &["build", "dev"], 0);
    let (o, log) = hyve_with_tools(&d, &bin, &["dev", "--native"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(log, "serge dev --native\n");
    // The old server tool and the new UI tool would start only half the app.
    fake_tool(&bin, "orbic", "0.1.0", &["codegen", "preview"], 0);
    let (o, log) = hyve_with_tools(&d, &bin, &["dev", "--web"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("`hyve-orbic` still advertises the old `preview` stage"));
    assert!(log.is_empty());
    // Tests need no stages.
    let (o, log) = hyve_with_tools(&d, &bin, &["test"]);
    assert!(o.status.success() && log.is_empty(), "{}", stderr(&o));
}

#[cfg(unix)]
#[test]
fn stage_failures_and_version_mismatches_fail() {
    let d = framework_workspace("0.1.0", None);
    let bin = d.join("bin");
    fake_tool(&bin, "orbic", "0.1.0", &["codegen"], 7);
    let (o, _) = hyve_with_tools(&d, &bin, &["codegen", "--check"]);
    assert_eq!(
        o.status.code(),
        Some(7),
        "the stage's exit code: {}",
        stderr(&o)
    );

    fake_tool(&bin, "orbic", "0.0.9", &["codegen"], 0);
    let (o, log) = hyve_with_tools(&d, &bin, &["codegen"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(log.is_empty(), "nothing ran");
    let e = stderr(&o);
    assert!(
        e.contains("`hyve-orbic` is orbic 0.0.9, but this workspace uses orbic 0.1.0"),
        "{e}"
    );
    assert!(
        e.contains("cargo install orbic-cli --version =0.1.0"),
        "{e}"
    );

    let empty = d.join("empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_hyve"))
        .arg("codegen")
        .current_dir(&d)
        .env("PATH", empty.display().to_string())
        .env(
            "CARGO",
            std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()),
        )
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("`hyve-orbic` could not be run"),
        "{}",
        stderr(&o)
    );
}

/// A stand-in `hyve-<framework>` providing `dev`, whose `dev` runs
/// the shell `body`.
#[cfg(unix)]
fn fake_dev_tool(bin: &Path, framework: &str, version: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = bin.join(format!("hyve-{framework}"));
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = stages ]; then printf '%s\\n' {version} dev; exit 0; fi\n{body}\n"
    );
    std::fs::create_dir_all(bin).unwrap();
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `dev` stages run at the same time with a stdin lifeline: a stage
/// that ends (fails, or succeeds: its window closed) stops the others (they
/// see their stdin close) and `hyve` ends with its code; a stage that
/// ignores the lifeline is killed after a grace period.
#[cfg(unix)]
#[test]
fn dev_stages_run_together_and_stop_together() {
    use std::time::{Duration, Instant};
    let d = framework_workspace("0.1.0", Some("0.2.0"));
    let bin = d.join("bin");
    // orbic serves until its stdin closes; serge fails after a moment.
    fake_dev_tool(
        &bin,
        "orbic",
        "0.1.0",
        "[ \"$HYVE_STAGE_LIFELINE\" = stdin ] || exit 9\necho \"orbic up $*\" >> \"$HYVE_FAKE_LOG\"\ncat > /dev/null\necho 'orbic stopped' >> \"$HYVE_FAKE_LOG\"",
    );
    fake_dev_tool(
        &bin,
        "serge",
        "0.2.0",
        "echo \"serge up $*\" >> \"$HYVE_FAKE_LOG\"\nsleep 1\nexit 4",
    );
    let t = Instant::now();
    let (o, log) = hyve_with_tools(&d, &bin, &["dev", "--web"]);
    assert_eq!(o.status.code(), Some(4), "{}", stderr(&o));
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    assert!(
        log.contains("orbic up dev --web") && log.contains("serge up dev --web"),
        "{log}"
    );
    assert!(log.ends_with("orbic stopped\n"), "orbic was stopped: {log}");

    // A stage that ends well (its window closed) ends the development session too.
    fake_dev_tool(
        &bin,
        "serge",
        "0.2.0",
        "sleep 0.5\necho 'window closed' >> \"$HYVE_FAKE_LOG\"",
    );
    let (o, log) = hyve_with_tools(&d, &bin, &["dev", "--native"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(log, "orbic up dev --native\nwindow closed\norbic stopped\n");

    // A stage that ignores its lifeline is killed.
    fake_dev_tool(&bin, "orbic", "0.1.0", "exec sleep 60");
    fake_dev_tool(&bin, "serge", "0.2.0", "sleep 0.3\nexit 5");
    let t = Instant::now();
    let (o, _) = hyve_with_tools(&d, &bin, &["dev"]);
    assert_eq!(o.status.code(), Some(5), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("did not stop when asked"),
        "{}",
        stderr(&o)
    );
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
}
