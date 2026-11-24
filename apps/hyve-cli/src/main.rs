//! `hyve` — the Hyperish workflow on top of Cargo (`docs/01-dx-and-dsl.md` §2).
//!
//! Phase 1 runs `test` and `build` on the enclosing Cargo workspace. The
//! commands that need the framework stages (`hyve-orbic`, `hyve-serge`) or
//! belong to later phases report that they are not implemented and exit
//! with [`EXIT_NOT_IMPLEMENTED`]; nothing unimplemented ends in success.
//!
//! Exit codes: `0` success, [`EXIT_FAILED`] the work failed (or the exit code
//! of the `cargo` command it ran), [`EXIT_USAGE`] unknown command, option or
//! template, [`EXIT_NOT_IMPLEMENTED`] not in this release.

mod workspace;

use std::ffi::OsString;
use std::process::{Command, ExitCode};

use hyve_diagnostics::suggest;

use workspace::Workspace;

/// The work failed: no workspace, or a failed `cargo` run without its own code.
const EXIT_FAILED: u8 = 1;
/// Unknown command, option or template name.
const EXIT_USAGE: u8 = 2;
/// The command or option is not implemented in this release.
const EXIT_NOT_IMPLEMENTED: u8 = 3;

const COMMANDS: [&str; 6] = ["init", "preview", "lint", "test", "build", "codegen"];
const TEMPLATES: [&str; 3] = ["fullstack", "serge-app", "orbic-service"];

const USAGE: &str = "\
hyve — the Hyperish workflow on top of Cargo

Usage: hyve <command> [options]

Commands:
  test [-p <package>]... [--release] [-- <test args>...]
                      run the workspace's tests (cargo test)
  build [-p <package>]... [--debug]
                      build the workspace, optimized unless --debug (cargo build)
  codegen [--check]   run the frameworks' code generation stages
  init [--template fullstack|serge-app|orbic-service] [--force] [<path>]
                      create a project (not in this release)
  preview [--web] [--native]
                      run with live reload (not in this release)
  lint                lint the workspace (Phase 2)

Options:
  -h, --help          print this help
  -V, --version       print the version

Exit codes: 0 success, 1 failure, 2 usage error, 3 not implemented in this release.";

/// Why a command did not succeed.
enum Fail {
    Usage(String, Option<String>),
    NotImplemented(String),
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
            Fail::NotImplemented(m) => (
                EXIT_NOT_IMPLEMENTED,
                format!("{m} is not implemented in this release"),
                None,
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
        "codegen" => codegen(rest),
        "init" => init(rest),
        "preview" => preview(rest),
        "lint" => {
            no_options("lint", rest)?;
            Err(Fail::NotImplemented("`hyve lint` (Phase 2)".into()))
        }
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

fn no_options(command: &str, args: &[String]) -> Result<(), Fail> {
    match args.first() {
        Some(a) => Err(unknown_option(command, a, &[])),
        None => Ok(()),
    }
}

fn test(args: &[String]) -> Result<(), Fail> {
    let o = cargo_options("test", args, &["--release", "--"])?;
    let ws = Workspace::find()?;
    let mut cmd = ws.cargo("test");
    select(&mut cmd, &o.packages);
    if o.release {
        cmd.arg("--release");
    }
    if !o.passthrough.is_empty() {
        cmd.arg("--").args(&o.passthrough);
    }
    run_cargo(cmd)
}

fn build(args: &[String]) -> Result<(), Fail> {
    let o = cargo_options("build", args, &["--debug"])?;
    let ws = Workspace::find()?;
    ws.run_stages("build", if o.debug { &["--debug"] } else { &[] })?;
    let mut cmd = ws.cargo("build");
    select(&mut cmd, &o.packages);
    if !o.debug {
        cmd.arg("--release");
    }
    run_cargo(cmd)
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
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--template" => {
                let Some(t) = it.next() else {
                    return Err(Fail::Usage(
                        "`--template` needs a name".into(),
                        Some(format!("one of {}", TEMPLATES.join(", "))),
                    ));
                };
                if !TEMPLATES.contains(&t.as_str()) {
                    let help = suggest(t, TEMPLATES)
                        .map(|s| format!("did you mean `{s}`?"))
                        .or_else(|| Some(format!("templates: {}", TEMPLATES.join(", "))));
                    return Err(Fail::Usage(format!("unknown template `{t}`"), help));
                }
            }
            "--force" => {}
            a if !a.starts_with('-') && path.is_none() => path = Some(a.to_owned()),
            other => return Err(unknown_option("init", other, &["--template", "--force"])),
        }
    }
    Err(Fail::NotImplemented(
        "`hyve init` (its templates pin orbic and serge versions, which have no prerelease yet)"
            .into(),
    ))
}

fn preview(args: &[String]) -> Result<(), Fail> {
    for a in args {
        if a != "--web" && a != "--native" {
            return Err(unknown_option("preview", a, &["--web", "--native"]));
        }
    }
    let ws = Workspace::find()?;
    let flags: Vec<&str> = args.iter().map(String::as_str).collect();
    match ws.run_stage_together("preview", &flags)? {
        0 => Err(Fail::Failed(
            "nothing to preview: no framework of this workspace provides a `preview` stage".into(),
            Some(
                "`hyve preview` runs `hyve-serge preview` and `hyve-orbic preview`; add a serge app, or declare an orbic server (`[package.metadata.orbic.server]`)".into(),
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
