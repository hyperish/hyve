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
    assert!(
        String::from_utf8_lossy(&hyve(&d, &["--help"]).stdout).contains("Usage: hyve <command>")
    );
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
    assert_eq!(hyve(&d, &["preview", "--desktop"]).status.code(), Some(2));
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
fn unimplemented_commands_never_succeed() {
    let d = temp(&[]);
    for args in [
        &["init", "--template", "fullstack"][..],
        &["init"],
        &["preview", "--web"],
        &["lint"],
    ] {
        let o = hyve(&d, args);
        assert_eq!(o.status.code(), Some(3), "{args:?}");
        assert!(
            stderr(&o).contains("is not implemented in this release"),
            "{args:?}: {}",
            stderr(&o)
        );
    }
    // `init` writes nothing.
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
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
fn codegen_without_frameworks_has_nothing_to_do() {
    let d = crate_dir("plain", "");
    assert!(hyve(&d, &["codegen"]).status.success());
    assert!(hyve(&d, &["codegen", "--check"]).status.success());
}

#[test]
fn framework_workspaces_need_the_stage_calls() {
    let d = temp(&[
        (
            "Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\norbic = { path = \"orbic\" }\n\n[workspace]\n",
        ),
        ("src/lib.rs", ""),
        (
            "orbic/Cargo.toml",
            "[package]\nname = \"orbic\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        ),
        ("orbic/src/lib.rs", ""),
    ]);
    for cmd in ["build", "codegen"] {
        let o = hyve(&d, &[cmd]);
        assert_eq!(o.status.code(), Some(3), "{cmd}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("uses orbic (calling the hyve-orbic stages)"),
            "{}",
            stderr(&o)
        );
    }
    // Tests need no stages.
    assert!(hyve(&d, &["test"]).status.success());
}
