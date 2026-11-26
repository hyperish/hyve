//! `hyve` — the Hyperish workflow on top of Cargo (`docs/01-dx-and-dsl.md` §2).
//!
//! Runs `test`, `lint` and `build` on the enclosing Cargo workspace, runs
//! the frameworks' stages (`hyve-orbic`, `hyve-serge`) for `codegen`,
//! `build`, `lint` and `dev`, and creates projects with `init`.
//!
//! Exit codes: `0` success, [`EXIT_FAILED`] the work failed (or the exit code
//! of the `cargo` command it ran), [`EXIT_USAGE`] unknown command, option or
//! template.

mod android;
mod ios;
mod lint;
mod templates;
mod update;
mod workspace;

use std::ffi::OsString;
use std::process::{Command, ExitCode};

use hyve_diagnostics::suggest;

use templates::TEMPLATES;
use workspace::Workspace;

/// The work failed: no workspace, or a failed `cargo` run without its own code.
const EXIT_FAILED: u8 = 1;
/// Unknown command, option or template name.
const EXIT_USAGE: u8 = 2;

const COMMANDS: [&str; 7] = ["init", "dev", "lint", "test", "build", "codegen", "update"];

const USAGE: &str = "\
hyve — the Hyperish workflow on top of Cargo

Usage: hyve <command> [options]

Commands:
  test [-p <package>]... [--release] [-- <test args>...]
                      run the workspace's tests with declared server features
  build [-p <package>]... [--debug] [--format <app,dmg,wix,nsis,deb,appimage,apk,aab,ios-sim,ipa>]
                      build the workspace, optimized unless --debug (cargo build)
  update keygen|create|apply  sign or apply desktop artifact deltas (hyve update --help)
  codegen [--check]   run the frameworks' code generation stages
  init [--template fullstack|serge-app|orbic-service] [--force] [<path>]
                      create a project from a template (default fullstack)
  dev [--web] [--native]
                      start the local development environment (Ctrl-C stops all)
  lint                check formatting, Clippy, framework stages and configured API compatibility
  lint --schema <baseline> <current>  compare two local OpenAPI documents
  lint --expand [cargo expand options]  print expanded Rust for the current package

Options:
  -h, --help          print this help
  -V, --version       print the version

Exit codes: 0 success, 1 failure, 2 usage error. Cargo failures retain their exit code.";

/// Why a command did not succeed.
enum Fail {
    Usage(String, Option<String>),
    Failed(String, Option<String>),
    /// A `cargo` run failed; it already printed why.
    Child(Option<i32>),
}

impl Fail {
    fn report(self) -> ExitCode {
        let (code, message, help) = match self {
            Fail::Usage(m, h) => (
                EXIT_USAGE,
                m,
                h.or_else(|| Some("run `hyve --help` for usage".into())),
            ),
            Fail::Failed(m, h) => (EXIT_FAILED, m, h),
            Fail::Child(code) => {
                return ExitCode::from(
                    code.and_then(|c| u8::try_from(c).ok())
                        .filter(|&c| c != 0)
                        .unwrap_or(EXIT_FAILED),
                );
            }
        };
        eprintln!("error: {message}");
        if let Some(help) = help {
            eprintln!("  help: {help}");
        }
        ExitCode::from(code)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect()
    {
        Ok(args) => args,
        Err(_) => return Fail::Usage("arguments must be valid UTF-8".into(), None).report(),
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => f.report(),
    }
}

fn run(args: &[String]) -> Result<(), Fail> {
    let Some((command, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return Err(Fail::Usage("missing command".into(), None));
    };
    match command.as_str() {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("hyve {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "test" => test(rest),
        "build" => build(rest),
        "update" => update::run(rest),
        "codegen" => codegen(rest),
        "init" => init(rest),
        "dev" => dev(rest),
        "preview" => Err(Fail::Usage(
            "`hyve preview` was renamed to `hyve dev`".into(),
            Some("use `hyve dev` with the same target options".into()),
        )),
        "lint" => lint::run(rest),
        other => {
            let help = suggest(other, COMMANDS).map(|c| format!("did you mean `hyve {c}`?"));
            Err(Fail::Usage(format!("unknown command `{other}`"), help))
        }
    }
}

/// Options shared by `test` and `build`.
#[derive(Default)]
struct CargoOptions {
    packages: Vec<String>,
    formats: Vec<String>,
    release: bool,
    debug: bool,
    passthrough: Vec<String>,
}

fn cargo_options(command: &str, args: &[String], accepts: &[&str]) -> Result<CargoOptions, Fail> {
    let mut o = CargoOptions::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--" if accepts.contains(&"--") => {
                o.passthrough = it.by_ref().cloned().collect();
            }
            "-p" | "--package" => match it.next() {
                Some(p) => o.packages.push(p.clone()),
                None => return Err(Fail::Usage(format!("`{arg}` needs a package name"), None)),
            },
            "--release" if accepts.contains(&"--release") => o.release = true,
            "--debug" if accepts.contains(&"--debug") => o.debug = true,
            "--format" if accepts.contains(&"--format") => {
                let formats = it
                    .next()
                    .ok_or_else(|| Fail::Usage("`--format` needs a package format".into(), None))?;
                for format in formats.split(',') {
                    if ![
                        "app", "dmg", "wix", "nsis", "deb", "appimage", "apk", "aab", "ios-sim",
                        "ipa",
                    ]
                    .contains(&format)
                    {
                        return Err(Fail::Usage(
                            format!("unknown package format `{format}`"),
                            None,
                        ));
                    }
                    o.formats.push(format.to_owned());
                }
            }
            other => return Err(unknown_option(command, other, accepts)),
        }
    }
    Ok(o)
}

fn unknown_option(command: &str, option: &str, known: &[&str]) -> Fail {
    let known: Vec<&str> = known
        .iter()
        .copied()
        .chain(["--package"])
        .filter(|k| *k != "--")
        .collect();
    let help = suggest(option, known.iter().copied())
        .map(|k| format!("did you mean `{k}`?"))
        .or_else(|| Some(format!("`hyve {command}` takes: {}", known.join(", "))));
    if option.starts_with('-') {
        Fail::Usage(
            format!("unknown option `{option}` for `hyve {command}`"),
            help,
        )
    } else {
        Fail::Usage(
            format!("unexpected argument `{option}` for `hyve {command}`"),
            help,
        )
    }
}

fn test(args: &[String]) -> Result<(), Fail> {
    let o = cargo_options("test", args, &["--release", "--"])?;
    let ws = Workspace::find()?;
    let mut cmd = ws.cargo("test");
    select(&mut cmd, &o.packages);
    for feature in ws.test_features(&o.packages)? {
        cmd.arg("--features").arg(feature);
    }
    if o.release {
        cmd.arg("--release");
    }
    if !o.passthrough.is_empty() {
        cmd.arg("--").args(&o.passthrough);
    }
    run_cargo(cmd)
}

fn build(args: &[String]) -> Result<(), Fail> {
    let mut o = cargo_options("build", args, &["--debug", "--format"])?;
    let aab = o.formats.iter().any(|format| format == "aab");
    let want_apk = o.formats.iter().any(|format| format == "apk");
    let android = aab || want_apk;
    let sim = o.formats.iter().any(|f| f == "ios-sim");
    let ipa = o.formats.iter().any(|f| f == "ipa");
    if (sim || ipa) && !cfg!(target_os = "macos") {
        return Err(Fail::Failed(
            "iOS packaging requires Xcode on macOS".into(),
            None,
        ));
    }
    o.formats
        .retain(|f| !matches!(f.as_str(), "apk" | "aab" | "ios-sim" | "ipa"));
    for format in &o.formats {
        let host = match format.as_str() {
            "app" | "dmg" => "macos",
            "wix" | "nsis" => "windows",
            "deb" | "appimage" => "linux",
            _ => unreachable!("validated format"),
        };
        if std::env::consts::OS != host {
            return Err(Fail::Failed(
                format!("the `{format}` package needs a {host} executable"),
                Some(format!("run this build on {host}")),
            ));
        }
    }
    let ws = Workspace::find()?;
    let roots = ws.selected_roots(&o.packages)?;
    ws.run_stages("codegen", &[])?;
    if android {
        for app in ws.mobile_packages("android", roots.as_deref())? {
            let library = ws
                .widget_sources(app)?
                .map(|p| p.join("android"))
                .filter(|p| p.is_dir());
            if android::compose(&ws, app, library.as_deref(), o.debug, want_apk, aab)? {
                continue;
            }
            let mut apk = ws.cargo("apk");
            apk.args(["build", "--lib", "-p", &app.name]);
            if !o.debug {
                apk.arg("--release");
            }
            run_cargo(apk)?;
            if aab {
                android::bundle(&ws, app, o.debug)?;
            }
        }
    }
    if sim || ipa {
        for app in ws.mobile_packages("ios", roots.as_deref())? {
            ios::build(&ws, app, sim, ipa, o.debug)?;
        }
    }
    if (android || sim || ipa) && o.formats.is_empty() {
        return Ok(());
    }
    let options = if o.debug { vec!["--debug"] } else { Vec::new() };
    if let Some(roots) = &roots {
        for name in roots
            .lines()
            .filter_map(|line| line.split_whitespace().next())
        {
            let mut args = options.clone();
            args.extend(["-p", name]);
            ws.run_stages("build", &args)?;
        }
    } else {
        ws.run_stages("build", &options)?;
    }
    let mut cmd = ws.cargo("build");
    select(&mut cmd, &o.packages);
    if !o.debug {
        cmd.arg("--release");
    }
    run_cargo(cmd)?;
    if !o.formats.is_empty() {
        let mut packager = ws.cargo("packager");
        let tool = ws.cargo("packager").arg("--version").output();
        if !tool.is_ok_and(|out| out.status.success()) {
            return Err(Fail::Failed(
                "desktop packaging requires `cargo-packager`".into(),
                Some("cargo install cargo-packager --version 0.11.8 --locked".into()),
            ));
        }
        packager.arg("--formats").arg(o.formats.join(","));
        if !o.debug {
            packager.arg("--release");
        }
        if !o.packages.is_empty() {
            packager.arg("--packages").arg(o.packages.join(","));
        }
        run_cargo(packager)?;
    }
    Ok(())
}

fn codegen(args: &[String]) -> Result<(), Fail> {
    for a in args {
        if a != "--check" {
            return Err(unknown_option("codegen", a, &["--check"]));
        }
    }
    let ws = Workspace::find()?;
    if !ws.uses_frameworks() {
        eprintln!("hyve: no code generation stages in this workspace (no framework dependency)");
        return Ok(());
    }
    let check = args.iter().any(|a| a == "--check");
    ws.run_stages("codegen", if check { &["--check"] } else { &[] })
        .map(|_| ())
}

fn init(args: &[String]) -> Result<(), Fail> {
    let mut it = args.iter();
    let mut path = None;
    let mut template = TEMPLATES[0];
    let mut force = false;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--template" => {
                let Some(t) = it.next() else {
                    return Err(Fail::Usage(
                        "`--template` needs a name".into(),
                        Some(format!("one of {}", TEMPLATES.join(", "))),
                    ));
                };
                let Some(known) = TEMPLATES.iter().find(|k| **k == t.as_str()) else {
                    let help = suggest(t, TEMPLATES)
                        .map(|s| format!("did you mean `{s}`?"))
                        .or_else(|| Some(format!("templates: {}", TEMPLATES.join(", "))));
                    return Err(Fail::Usage(format!("unknown template `{t}`"), help));
                };
                template = known;
            }
            "--force" => force = true,
            a if !a.starts_with('-') && path.is_none() => path = Some(a.to_owned()),
            other => return Err(unknown_option("init", other, &["--template", "--force"])),
        }
    }
    let cwd = std::env::current_dir()
        .map_err(|e| Fail::Failed(format!("no working directory: {e}"), None))?;
    let dir = match &path {
        Some(p) => cwd.join(p),
        None => cwd.clone(),
    };
    // The package is named after the directory.
    let name = std::path::Path::new(&dir)
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .next_back()
        .unwrap_or_default();
    if !templates::valid_name(&name) {
        return Err(Fail::Usage(
            format!("`{name}` cannot be a package name (the directory names the package)"),
            Some(
                "use letters, digits, `-` and `_`, starting with a letter: `hyve init my-app`"
                    .into(),
            ),
        ));
    }
    let files = templates::render(template, &name);
    let taken: Vec<&str> = files
        .iter()
        .filter(|(p, _)| dir.join(p).exists())
        .map(|(p, _)| p.as_str())
        .collect();
    if !taken.is_empty() && !force {
        return Err(Fail::Failed(
            format!(
                "{} already exist{} in {}: {}",
                if taken.len() == 1 { "a file" } else { "files" },
                if taken.len() == 1 { "s" } else { "" },
                dir.display(),
                taken.join(", ")
            ),
            Some("nothing was written; `--force` overwrites them".into()),
        ));
    }
    for (p, text) in &files {
        let target = dir.join(p);
        let written = target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&target, text));
        if let Err(e) = written {
            return Err(Fail::Failed(format!("{}: {e}", target.display()), None));
        }
    }
    println!(
        "created `{name}` from the `{template}` template in {}",
        dir.display()
    );
    let into = path.map(|p| format!("cd {p} && ")).unwrap_or_default();
    match template {
        "orbic-service" => {
            println!("next: {into}hyve dev   (the server, with a local database)")
        }
        "serge-app" => {
            println!("next: {into}hyve dev --web   (or --native for the desktop window)")
        }
        _ => println!(
            "next: {into}hyve dev --web   (the page and its backend; without --web, the desktop window too)"
        ),
    }
    Ok(())
}

fn dev(args: &[String]) -> Result<(), Fail> {
    for a in args {
        if a != "--web" && a != "--native" {
            return Err(unknown_option("dev", a, &["--web", "--native"]));
        }
    }
    let ws = Workspace::find()?;
    let flags: Vec<&str> = args.iter().map(String::as_str).collect();
    match ws.run_stage_together("dev", &flags)? {
        0 => Err(Fail::Failed(
            "no framework in this workspace provides a `dev` stage".into(),
            Some(
                "add a serge app or declare an orbic server in `[package.metadata.orbic.server]`"
                    .into(),
            ),
        )),
        _ => Ok(()),
    }
}

fn select(cmd: &mut Command, packages: &[String]) {
    if packages.is_empty() {
        cmd.arg("--workspace");
    } else {
        for p in packages {
            cmd.arg("--package").arg(p);
        }
    }
}

fn run_cargo(mut cmd: Command) -> Result<(), Fail> {
    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(Fail::Child(status.code())),
        Err(e) => Err(Fail::Failed(
            format!("could not run cargo: {e}"),
            Some("install Rust with rustup: https://rustup.rs".into()),
        )),
    }
}
