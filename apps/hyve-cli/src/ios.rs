//! Build the application's Xcode target and export signed archives locally.
mod widgets;
use crate::{
    Fail,
    workspace::{MobileApp, Workspace},
};
use hyve_core::Value;
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use widgets::Extension;

fn fail(e: impl std::fmt::Display) -> Fail {
    Fail::Failed(e.to_string(), None)
}
fn field<'a>(app: &'a MobileApp, key: &str) -> Result<&'a str, Fail> {
    match app.settings.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s),
        _ => Err(fail(format!(
            "missing [package.metadata.ios] {key} in {}",
            app.name
        ))),
    }
}
fn run(cmd: &mut Command) -> Result<(), Fail> {
    let s = cmd
        .status()
        .map_err(|e| fail(format!("could not start xcodebuild: {e}")))?;
    if s.success() {
        Ok(())
    } else {
        Err(Fail::Child(s.code()))
    }
}
fn setting(key: &str, value: &Path) -> OsString {
    let mut arg = OsString::from(key);
    arg.push("=");
    arg.push(value);
    arg
}
fn artifact(dir: &Path, ext: &str) -> Result<PathBuf, Fail> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_some_and(|s| s == ext) {
            files.push(path);
        }
    }
    match files.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err(fail(format!("expected one .{ext} in {}", dir.display()))),
    }
}
fn export_options(app: &MobileApp) -> Result<PathBuf, Fail> {
    let file = app
        .dir
        .join(field(app, "export_options")?)
        .canonicalize()
        .map_err(fail)?;
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(&file)
        .output()
        .map_err(fail)?;
    if !output.status.success() {
        return Err(fail("invalid iOS export options property list"));
    }
    let options =
        hyve_serde::from_json::<Value>(&String::from_utf8_lossy(&output.stdout)).map_err(fail)?;
    if !matches!(&options, Value::Map(_))
        || options
            .get("destination")
            .is_some_and(|v| !matches!(v, Value::String(s) if s == "export"))
    {
        return Err(fail(
            "hyve build requires a local export destination in the iOS export options",
        ));
    }
    Ok(file)
}
pub(super) fn build(
    ws: &Workspace,
    app: &MobileApp,
    simulator: bool,
    ipa: bool,
    debug: bool,
) -> Result<(), Fail> {
    let project = app
        .dir
        .join(field(app, "project")?)
        .canonicalize()
        .map_err(fail)?;
    let widgets = ws
        .widget_sources(app)?
        .map(|p| p.join("ios"))
        .filter(|p| p.is_dir());
    let options = ipa.then(|| export_options(app)).transpose()?;
    let project_flag = match project.extension().and_then(|s| s.to_str()) {
        Some("xcodeproj") => "-project",
        Some("xcworkspace") => "-workspace",
        _ => {
            return Err(fail(
                "expected an .xcodeproj or .xcworkspace in [package.metadata.ios]",
            ));
        }
    };
    let scheme = field(app, "scheme")?;
    let dest = ws
        .target_dir
        .join(if debug { "debug" } else { "release" })
        .join("ios")
        .join(&app.name);
    fs::create_dir_all(&dest).map_err(fail)?;
    let derived = ws.target_dir.join("hyve-ios").join(&app.name);
    let xcode = || {
        let mut cmd = Command::new("xcrun");
        cmd.current_dir(&app.dir)
            .arg("xcodebuild")
            .arg(project_flag)
            .arg(&project)
            .args([
                "-scheme",
                scheme,
                "-configuration",
                if debug { "Debug" } else { "Release" },
                "-derivedDataPath",
            ])
            .arg(&derived);
        if let Some(team) = env::var_os("HYVE_IOS_TEAM") {
            let mut arg = OsString::from("DEVELOPMENT_TEAM=");
            arg.push(team);
            cmd.arg(arg);
        }
        cmd
    };
    if simulator {
        let products = dest.join("simulator");
        let arch = if env::consts::ARCH == "aarch64" {
            "ARCHS=arm64"
        } else {
            "ARCHS=x86_64"
        };
        let extension = widgets
            .as_ref()
            .map(|s| Extension::build(app, s, xcode(), "iphonesimulator", Some(arch), debug))
            .transpose()?;
        let mut host = xcode();
        if let Some(w) = &extension {
            w.configure(&mut host);
        }
        run(host
            .args([
                "-sdk",
                "iphonesimulator",
                "-destination",
                "generic/platform=iOS Simulator",
                arch,
                "build",
            ])
            .arg(setting("CONFIGURATION_BUILD_DIR", &products)))?;
        if let Some(w) = extension {
            w.verify(&artifact(&products, "app")?)?;
        }
        println!(
            "iOS simulator app {}",
            artifact(&products, "app")?.display()
        );
    }
    if let Some(options) = options {
        let tmp = tempfile::tempdir_in(&dest).map_err(fail)?;
        let archive = tmp.path().join("app.xcarchive");
        let extension = widgets
            .as_ref()
            .map(|s| Extension::build(app, s, xcode(), "iphoneos", None, debug))
            .transpose()?;
        let mut host = xcode();
        if let Some(w) = &extension {
            w.configure(&mut host);
        }
        run(host
            .args([
                "-sdk",
                "iphoneos",
                "-destination",
                "generic/platform=iOS",
                "-archivePath",
            ])
            .arg(&archive)
            .arg("archive"))?;
        if let Some(w) = extension {
            w.verify(&artifact(&archive.join("Products/Applications"), "app")?)?;
        }
        let export = tmp.path().join("export");
        run(Command::new("xcrun")
            .args(["xcodebuild", "-exportArchive", "-archivePath"])
            .arg(archive)
            .arg("-exportOptionsPlist")
            .arg(options)
            .arg("-exportPath")
            .arg(&export))?;
        let path = dest.join(format!("{}.ipa", app.name));
        let mut temp = tempfile::NamedTempFile::new_in(&dest).map_err(fail)?;
        io::copy(
            &mut fs::File::open(artifact(&export, "ipa")?).map_err(fail)?,
            temp.as_file_mut(),
        )
        .map_err(fail)?;
        temp.as_file().sync_all().map_err(fail)?;
        temp.persist(&path).map_err(fail)?;
        println!("iOS application archive {}", path.display());
    }
    Ok(())
}
