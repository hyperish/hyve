//! `hyve-config`: precedence, defaults, error locations, sections.

#![allow(missing_docs)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use hyve::config::{ConfigError, Loader, Source, find_root};
use hyve::prelude::*;

#[model]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Fast,
    Quality,
}

#[model]
#[derive(Debug)]
pub struct Window {
    pub width: u32,
    pub height: u32,
}

#[model]
#[derive(Debug)]
pub struct Settings {
    #[validate(len = 1..=64)]
    pub app_name: String,
    #[validate(range = 30..=240)]
    pub max_fps: u32,
    pub ratio: f64,
    pub debug: bool,
    pub tags: Vec<String>,
    pub mode: Mode,
    pub window: Window,
    pub note: Option<String>,
}

#[model]
#[derive(Debug, Default)]
pub struct Tuning {
    pub threads: u8,
    pub label: String,
}

#[model]
#[derive(Debug)]
pub struct OrbicSection {
    pub dialect: String,
}

/// A fresh directory with the given files.
fn dir(files: &[(&str, &str)]) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "hyve-config-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for (name, text) in files {
        std::fs::write(d.join(name), text).unwrap();
    }
    d
}

const FULL: &str = r#"
[app]
app_name = "demo"
max_fps = 60
ratio = 1
debug = false
tags = ["a", "b"]
mode = "fast"
window = { width = 800, height = 600 }
"#;

fn loader(files: &[(&str, &str)]) -> Loader {
    Loader::new()
        .dir(dir(files))
        .env(Vec::<(String, String)>::new())
        .args(Vec::<String>::new())
}

fn err<T: std::fmt::Debug>(r: Result<T, ConfigError>) -> ConfigError {
    r.expect_err("configuration must be rejected")
}

#[test]
fn hyve_toml_fills_the_model() {
    let s = loader(&[("hyve.toml", FULL)]).load::<Settings>().unwrap();
    assert_eq!(
        (s.app_name.as_str(), s.max_fps, s.ratio, s.debug),
        ("demo", 60, 1.0, false)
    );
    assert_eq!(
        (s.tags.len(), s.mode, s.window.width, s.note.clone()),
        (2, Mode::Fast, 800, None)
    );
}

#[test]
fn later_sources_win_field_by_field() {
    let files = [
        ("hyve.toml", FULL),
        (".env", "APP_MAX_FPS=90\nAPP_RATIO=0.5\n"),
    ];
    let env = [
        ("APP_MAX_FPS", "120"),
        ("APP_NOTE", "from env"),
        ("OTHER_MAX_FPS", "1"),
        ("APP_UNKNOWN", "x"),
    ];
    let args = ["--verbose", "--set", "max_fps=144", "--set=mode=quality"];

    let all = loader(&files)
        .env(env)
        .args(args)
        .load::<Settings>()
        .unwrap();
    assert_eq!(
        (all.max_fps, all.mode, all.ratio),
        (144, Mode::Quality, 0.5)
    );
    assert_eq!(all.note.as_deref(), Some("from env"));

    let no_args = loader(&files).env(env).load::<Settings>().unwrap();
    assert_eq!(no_args.max_fps, 120);
    let no_env = loader(&files).load::<Settings>().unwrap();
    assert_eq!(no_env.max_fps, 90);
    let file_only = loader(&files[..1]).load::<Settings>().unwrap();
    assert_eq!(file_only.max_fps, 60);
}

#[test]
fn structured_values_from_text_sources_use_toml_syntax() {
    let env = [
        ("APP_TAGS", r#"["x", "y", "z"]"#),
        ("APP_WINDOW", "{ width = 1, height = 2 }"),
        ("APP_DEBUG", "true"),
    ];
    let s = loader(&[("hyve.toml", FULL)])
        .env(env)
        .load::<Settings>()
        .unwrap();
    assert_eq!((s.tags.len(), s.window.height, s.debug), (3, 2, true));
    // Text fields take the value as written, even when it looks like TOML.
    let s = loader(&[("hyve.toml", FULL)])
        .args(["--set", "app_name=true"])
        .load::<Settings>()
        .unwrap();
    assert_eq!(s.app_name, "true");
}

#[test]
fn the_model_default_is_the_lowest_layer() {
    let t = loader(&[]).load::<Tuning>().unwrap();
    assert_eq!((t.threads, t.label.as_str()), (0, ""));
    let t = loader(&[("hyve.toml", "[app]\nthreads = 4\n")])
        .env([("APP_LABEL", "x")])
        .load::<Tuning>()
        .unwrap();
    assert_eq!((t.threads, t.label.as_str()), (4, "x"));
}

#[test]
fn missing_fields_say_where_they_can_be_set() {
    let e = err(loader(&[("hyve.toml", "[app]\napp_name = \"demo\"\n")]).load::<Settings>());
    let missing: Vec<_> = e.problems().iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        missing,
        ["max_fps", "ratio", "debug", "tags", "mode", "window"],
        "Option fields may be absent"
    );
    let first = &e.problems()[0];
    assert_eq!(first.source, None);
    let help = first.help.as_deref().unwrap();
    assert!(
        help.contains("`max_fps = …` under `[app]`")
            && help.contains("APP_MAX_FPS")
            && help.contains("--set max_fps=…"),
        "{help}"
    );
}

#[test]
fn validation_failures_point_at_the_source_and_key() {
    let toml = FULL.replace("max_fps = 60", "max_fps = 500");
    let e = err(loader(&[("hyve.toml", &toml)]).load::<Settings>());
    let p = &e.problems()[0];
    assert!(matches!(&p.source, Some(Source::File(f)) if f.ends_with("hyve.toml")));
    assert_eq!(
        (p.key.as_str(), p.path.as_str()),
        ("app.max_fps", "$.maxFps")
    );
    assert_eq!(&toml[p.span.unwrap().range()], "500");
    let text = e.to_string();
    assert!(text.contains("hyve.toml:4:11: `app.max_fps`:"), "{text}");
    assert!(e.render().contains("500"), "{}", e.render());

    let e = err(
        loader(&[("hyve.toml", FULL), (".env", "# comment\nAPP_APP_NAME=\n")]).load::<Settings>(),
    );
    let p = &e.problems()[0];
    assert!(matches!(&p.source, Some(Source::DotEnv(_))));
    assert_eq!(p.key, "APP_APP_NAME");
    assert!(e.to_string().contains(".env:2:1: `APP_APP_NAME`:"), "{e}");
}

#[test]
fn conversion_failures_point_at_the_source_and_key() {
    let e = err(loader(&[("hyve.toml", FULL)])
        .env([("APP_MAX_FPS", "fast")])
        .load::<Settings>());
    assert_eq!(e.problems().len(), 1);
    let p = &e.problems()[0];
    assert_eq!(
        (p.source.clone(), p.key.as_str()),
        (Some(Source::Env), "APP_MAX_FPS")
    );
    assert_eq!(p.message, "expected an integer (u32), found `fast`");
    assert!(
        e.to_string().contains("environment variable `APP_MAX_FPS`"),
        "{e}"
    );

    let toml = FULL
        .replace("max_fps = 60", "max_fps = \"60\"")
        .replace("width = 800", "width = -1");
    let e = err(loader(&[("hyve.toml", &toml)]).load::<Settings>());
    let messages: Vec<_> = e
        .problems()
        .iter()
        .map(|p| (p.key.as_str(), p.message.as_str()))
        .collect();
    assert_eq!(
        messages,
        [
            ("app.max_fps", "expected an integer (u32), found a string"),
            (
                "app.window.width",
                "-1 is out of range for u32 (0..=4294967295)"
            ),
        ]
    );

    let e = err(loader(&[("hyve.toml", &FULL.replace("\"fast\"", "\"fsat\""))]).load::<Settings>());
    let p = &e.problems()[0];
    assert_eq!(p.message, "`fsat` is not one of `fast`, `quality`");
    assert_eq!(p.help.as_deref(), Some("did you mean `fast`?"));
}

#[test]
fn typos_get_suggestions() {
    let toml = format!("{FULL}max_fsp = 1\n[ap]\nx = 1\n");
    let e = err(loader(&[("hyve.toml", &toml)]).load::<Settings>());
    let found: Vec<_> = e
        .problems()
        .iter()
        .map(|p| (p.message.as_str(), p.help.as_deref()))
        .collect();
    assert!(
        found.contains(&("unknown section `[ap]`", Some("did you mean `[app]`?"))),
        "{found:?}"
    );
    assert!(
        found.contains(&("unknown key `max_fsp`", Some("did you mean `max_fps`?"))),
        "{found:?}"
    );

    let e = err(loader(&[("hyve.toml", &format!("debug = true\n{FULL}"))]).load::<Settings>());
    assert_eq!(e.problems()[0].message, "`debug` is outside any section");

    let e = err(loader(&[("hyve.toml", FULL)])
        .args(["--set", "max_fsp=1"])
        .load::<Settings>());
    assert_eq!(
        e.problems()[0].help.as_deref(),
        Some("did you mean `max_fps`?")
    );
    let e = err(loader(&[("hyve.toml", FULL)])
        .args(["--set", "window.width=1"])
        .load::<Settings>());
    assert_eq!(
        e.problems()[0].message,
        "nested keys are not supported by `--set`"
    );
}

#[test]
fn syntax_errors_carry_their_location() {
    let e = err(loader(&[("hyve.toml", "[app]\nmax_fps = 01\n")]).load::<Settings>());
    let p = &e.problems()[0];
    assert!(p.message.contains("invalid number"), "{e}");
    assert!(e.to_string().contains("hyve.toml:2:11"), "{e}");
}

#[test]
fn env_prefix_comes_from_the_hyve_section() {
    let toml = format!("[hyve]\nenv_prefix = \"DEMO\"\n{FULL}");
    let s = loader(&[("hyve.toml", &toml)])
        .env([("APP_MAX_FPS", "100"), ("DEMO_MAX_FPS", "200")])
        .load::<Settings>()
        .unwrap();
    assert_eq!(s.max_fps, 200);
    let e = err(loader(&[("hyve.toml", "[hyve]\nenv_prefix = \"lower\"\n")]).load::<Tuning>());
    assert_eq!(e.problems()[0].key, "hyve.env_prefix");
}

#[test]
fn frameworks_load_their_own_section() {
    let toml = format!("{FULL}\n[orbic]\ndialect = \"sqlite\"\n");
    let l = loader(&[("hyve.toml", &toml)]).section("orbic");
    assert_eq!(l.clone().load::<OrbicSection>().unwrap().dialect, "sqlite");
    let o = l
        .env([("HYPERISH_ORBIC_DIALECT", "postgres")])
        .args(["--set", "dialect=x"])
        .load::<OrbicSection>()
        .unwrap();
    assert_eq!(o.dialect, "postgres", "--set applies only to [app]");
}

#[test]
fn errors_convert_to_hyve_errors() {
    fn start() -> HyveResult<u32> {
        let s = loader(&[]).load::<Settings>()?;
        Ok(s.max_fps)
    }
    let e = start().unwrap_err();
    assert_eq!((e.kind(), e.code()), (ErrorKind::Infra, "invalid_config"));
    let Some(Value::List(problems)) = e.details().get("problems") else {
        panic!("{e:?}")
    };
    assert_eq!(problems[0].get("source"), Some(&Value::from("missing")));
}

#[test]
fn the_root_is_the_nearest_directory_with_hyve_toml() {
    let root = dir(&[("hyve.toml", FULL)]);
    let nested = root.join("crates/app/src");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(find_root(&nested), Some(root.clone()));
    assert_eq!(
        find_root(&std::env::temp_dir().join("hyve-config-nowhere")),
        None
    );
}
