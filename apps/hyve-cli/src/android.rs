//! Package the APK produced by cargo-apk as a signed Android App Bundle.
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

fn problem(e: impl std::fmt::Display) -> Fail {
    Fail::Failed(e.to_string(), None)
}
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    if let Value::String(s) = value.get(key)? {
        Some(s)
    } else {
        None
    }
}
fn jdk(tool: &str) -> Command {
    let path = env::var_os("JAVA_HOME").map(|p| PathBuf::from(p).join("bin").join(tool));
    Command::new(path.unwrap_or_else(|| PathBuf::from(tool)))
}
fn run(cmd: &mut Command) -> Result<(), Fail> {
    let status = cmd.status().map_err(|e| {
        problem(format!(
            "could not run {}: {e}",
            cmd.get_program().to_string_lossy()
        ))
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(Fail::Child(status.code()))
    }
}
fn prefixed(prefix: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(prefix);
    arg.push(path.as_os_str());
    arg
}
fn aapt2() -> Result<PathBuf, Fail> {
    let root = env::var_os("ANDROID_HOME")
        .or_else(|| env::var_os("ANDROID_SDK_ROOT"))
        .ok_or_else(|| problem("set ANDROID_HOME to the Android SDK"))?;
    fs::read_dir(PathBuf::from(root).join("build-tools"))
        .map_err(problem)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let version = e
                .file_name()
                .to_str()?
                .split('.')
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let tool = e.path().join(format!("aapt2{}", env::consts::EXE_SUFFIX));
            tool.is_file().then_some((version, tool))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, tool)| tool)
        .ok_or_else(|| problem("install Android SDK build-tools with aapt2"))
}
fn signing_key(app: &MobileApp, debug: bool) -> Result<(PathBuf, OsString, OsString), Fail> {
    let profile = if debug { "DEV" } else { "RELEASE" };
    let manifest = app
        .settings
        .get("signing")
        .and_then(|s| s.get(if debug { "dev" } else { "release" }));
    let store = env::var_os(format!("CARGO_APK_{profile}_KEYSTORE"))
        .map(PathBuf::from)
        .or_else(|| {
            manifest
                .and_then(|s| text(s, "path"))
                .map(|p| app.dir.join(p))
        });
    let (store, password, alias) = if let Some(store) = store {
        let password = env::var_os(format!("CARGO_APK_{profile}_KEYSTORE_PASSWORD"))
            .or_else(|| {
                manifest
                    .and_then(|s| text(s, "keystore_password"))
                    .map(Into::into)
            })
            .ok_or_else(|| problem("missing Android signing password"))?;
        let alias = env::var_os("HYVE_ANDROID_SIGNING_ALIAS")
            .ok_or_else(|| problem("set HYVE_ANDROID_SIGNING_ALIAS to the upload key's alias"))?;
        (store, password, alias)
    } else if debug {
        let user = env::var_os("ANDROID_USER_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("ANDROID_SDK_HOME")
                    .or_else(|| env::var_os("HOME"))
                    .or_else(|| env::var_os("USERPROFILE"))
                    .map(|p| PathBuf::from(p).join(".android"))
            })
            .ok_or_else(|| problem("could not locate Android debug keystore"))?;
        (
            user.join("debug.keystore"),
            "android".into(),
            "androiddebugkey".into(),
        )
    } else {
        return Err(problem("configure the release upload keystore"));
    };
    Ok((store.canonicalize().map_err(problem)?, password, alias))
}
fn sign(bundle: &Path, app: &MobileApp, debug: bool) -> Result<(), Fail> {
    let (store, password, alias) = signing_key(app, debug)?;
    run(jdk("jarsigner")
        .arg("-keystore")
        .arg(store)
        .args([
            "-storepass:env",
            "HYVE_BUNDLE_PASSWORD",
            "-keypass:env",
            "HYVE_BUNDLE_PASSWORD",
        ])
        .env("HYVE_BUNDLE_PASSWORD", password)
        .arg(bundle)
        .arg(alias))
}
fn apk_name(app: &MobileApp) -> Result<&str, Fail> {
    let name = text(&app.settings, "apk_name").unwrap_or(&app.name);
    if Path::new(name).file_name() != Some(name.as_ref()) {
        return Err(problem("invalid APK filename"));
    }
    Ok(name)
}

/// Keep the NativeActivity, manifest, original resources and each ABI produced by cargo-apk.
/// The Android manifest merger and resource linker compose them with actual widget receivers.
pub(super) fn compose(
    ws: &Workspace,
    app: &MobileApp,
    library: Option<&Path>,
    debug: bool,
    want_apk: bool,
    want_bundle: bool,
) -> Result<bool, Fail> {
    let enabled = |option: &str| match app.settings.get(option) {
        Some(Value::Bool(value)) => Ok(*value),
        None => Ok(false),
        _ => Err(problem(format!("android.{option} must be a boolean"))),
    };
    let window_layout = enabled("window_layout")?;
    let document_picker = enabled("document_picker")?;
    let background_task = enabled("background_task")?;
    let notifications = enabled("notifications")?;
    let webview = enabled("webview")?;
    let overlay = match app.settings.get("manifest_overlay") {
        Some(Value::String(s)) => Some(app.dir.join(s).canonicalize().map_err(problem)?),
        None => None,
        _ => return Err(problem("android.manifest_overlay must be a file path")),
    };
    if library.is_none()
        && overlay.is_none()
        && !window_layout
        && !document_picker
        && !background_task
        && !notifications
        && !webview
    {
        return Ok(false);
    }
    let profile = if debug { "debug" } else { "release" };
    let variant = if debug { "Debug" } else { "Release" };
    let name = apk_name(app)?;
    let dest = ws.target_dir.join(profile).join("apk");
    fs::create_dir_all(&dest).map_err(problem)?;
    let scratch = tempfile::tempdir_in(&dest).map_err(problem)?;
    let project = scratch.path();
    let target = project.join("cargo-target");
    let mut native = ws.cargo("apk");
    native
        .env("CARGO_TARGET_DIR", &target)
        .args(["build", "--lib", "-p", &app.name]);
    if !debug {
        native.arg("--release");
    }
    run(&mut native)?;
    let source = target.join(profile).join("apk");
    fs::copy(
        source.join("AndroidManifest.xml"),
        project.join("native-manifest.xml"),
    )
    .map_err(problem)?;
    let original = project.join("original");
    fs::create_dir(&original).map_err(problem)?;
    run(jdk("jar")
        .current_dir(original)
        .arg("xf")
        .arg(source.join(format!("{name}.apk")))
        .args(["lib", "assets"]))?;
    let mut settings = include_str!("../templates/mobile/android/settings.gradle.kts").to_string();
    if library.is_some() {
        settings.push_str("\ninclude(\":sergeWidgets\")\nproject(\":sergeWidgets\").projectDir = file(System.getenv(\"HYVE_ANDROID_WIDGETS\"))\n");
    }
    if window_layout {
        let java = project.join("src/main/java/org/hyperish/serge/window");
        fs::create_dir_all(&java).map_err(problem)?;
        fs::write(
            java.join("WindowPosture.java"),
            include_str!("../templates/mobile/android/WindowPosture.java"),
        )
        .map_err(problem)?;
    }
    if document_picker {
        let java = project.join("src/main/java/org/hyperish/serge/documents");
        fs::create_dir_all(&java).map_err(problem)?;
        fs::write(
            java.join("DocumentPicker.java"),
            include_str!("../templates/mobile/android/DocumentPicker.java"),
        )
        .map_err(problem)?;
    }
    if background_task {
        let java = project.join("src/main/java/org/hyperish/serge/background");
        fs::create_dir_all(&java).map_err(problem)?;
        fs::write(
            java.join("WorkService.java"),
            include_str!("../templates/mobile/android/WorkService.java"),
        )
        .map_err(problem)?;
    }
    if webview {
        let java = project.join("src/main/java/org/hyperish/serge/webview");
        fs::create_dir_all(&java).map_err(problem)?;
        fs::write(
            java.join("LocalWebView.java"),
            include_str!("../templates/mobile/android/LocalWebView.java"),
        )
        .map_err(problem)?;
    }
    if notifications {
        let java = project.join("src/main/java/org/hyperish/serge/notifications");
        fs::create_dir_all(&java).map_err(problem)?;
        fs::write(
            java.join("Actions.java"),
            include_str!("../templates/mobile/android/Actions.java"),
        )
        .map_err(problem)?;
    }
    fs::write(project.join("settings.gradle.kts"), settings).map_err(problem)?;
    fs::write(
        project.join("gradle.properties"),
        "android.useAndroidX=true\n",
    )
    .map_err(problem)?;
    fs::write(
        project.join("build.gradle"),
        include_str!("../templates/mobile/android/widgets.gradle"),
    )
    .map_err(problem)?;
    let (store, password, alias) = signing_key(app, debug)?;
    let mut gradle = Command::new(env::var_os("HYVE_GRADLE").unwrap_or_else(|| "gradle".into()));
    gradle
        .current_dir(project)
        .env_remove("HYVE_ANDROID_WIDGETS")
        .env_remove("HYVE_ANDROID_OVERLAY")
        .env(
            "HYVE_ANDROID_WINDOW_LAYOUT",
            if window_layout { "1" } else { "0" },
        )
        .env("HYVE_ANDROID_STORE", store)
        .env("HYVE_ANDROID_PASSWORD", password)
        .env("HYVE_ANDROID_ALIAS", alias)
        .env_remove("HYVE_ANDROID_RESOURCES")
        .arg("--no-daemon");
    if let Some(library) = library {
        gradle.env("HYVE_ANDROID_WIDGETS", library);
    }
    if let Some(overlay) = overlay {
        gradle.env("HYVE_ANDROID_OVERLAY", overlay);
    }
    if let Some(res) = text(&app.settings, "resources") {
        gradle.env("HYVE_ANDROID_RESOURCES", app.dir.join(res));
    }
    let mut outputs = Vec::new();
    if want_apk {
        gradle.arg(format!(":assemble{variant}"));
        outputs.push((format!("apk/{profile}"), "apk"));
    }
    if want_bundle {
        gradle.arg(format!(":bundle{variant}"));
        outputs.push((format!("bundle/{profile}"), "aab"));
    }
    run(&mut gradle)?;
    for (subdir, ext) in outputs {
        let mut files = fs::read_dir(project.join("build/outputs").join(&subdir))
            .map_err(problem)?
            .filter_map(Result::ok)
            .map(|f| f.path())
            .filter(|p| p.extension().is_some_and(|e| e == ext));
        let file = files
            .next()
            .filter(|_| files.next().is_none())
            .ok_or_else(|| problem(format!("expected one .{ext} in {subdir}")))?;
        let final_path = dest.join(format!("{name}.{ext}"));
        let mut temp = tempfile::NamedTempFile::new_in(&dest).map_err(problem)?;
        io::copy(
            &mut fs::File::open(file).map_err(problem)?,
            temp.as_file_mut(),
        )
        .map_err(problem)?;
        temp.as_file().sync_all().map_err(problem)?;
        temp.persist(&final_path).map_err(problem)?;
        // cargo-apk also emits a v4 sidecar. It belongs to the previous APK generation.
        if ext == "apk" {
            let sidecar = dest.join(format!("{name}.apk.idsig"));
            if let Err(e) = fs::remove_file(sidecar) {
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(problem(e));
                }
            }
        }
        println!("Android package {}", final_path.display());
    }
    Ok(true)
}

pub(super) fn bundle(ws: &Workspace, app: &MobileApp, debug: bool) -> Result<(), Fail> {
    let tool = env::var_os("HYVE_BUNDLETOOL")
        .ok_or_else(|| problem("set HYVE_BUNDLETOOL to the bundletool-all JAR"))?;
    let tool = fs::canonicalize(tool).map_err(problem)?;
    let name = apk_name(app)?;
    let dest = ws
        .target_dir
        .join(if debug { "debug" } else { "release" })
        .join("apk");
    let stage = tempfile::tempdir_in(&dest).map_err(problem)?;
    let dir = stage.path();
    let apk = dest.join(format!("{name}.apk"));
    let proto = dir.join("proto.apk");
    run(Command::new(aapt2()?)
        .args(["convert", "--output-format", "proto", "-o"])
        .arg(&proto)
        .arg(&apk))?;
    let unpacked = dir.join("unpacked");
    let base = dir.join("base");
    fs::create_dir(&unpacked).map_err(problem)?;
    fs::create_dir_all(base.join("manifest")).map_err(problem)?;
    run(jdk("jar").current_dir(&unpacked).arg("xf").arg(&proto))?;
    for entry in fs::read_dir(&unpacked).map_err(problem)? {
        let entry = entry.map_err(problem)?;
        let name = entry.file_name();
        let file = name.to_string_lossy();
        let target = match file.as_ref() {
            "AndroidManifest.xml" => base.join("manifest").join(&name),
            "res" | "resources.pb" | "assets" | "lib" => base.join(&name),
            _ if file.starts_with("classes") && file.ends_with(".dex") => {
                base.join("dex").join(&name)
            }
            "META-INF" => {
                for meta in fs::read_dir(entry.path()).map_err(problem)? {
                    let meta = meta.map_err(problem)?;
                    let upper = meta.file_name().to_string_lossy().to_ascii_uppercase();
                    if meta.file_type().map_err(problem)?.is_file()
                        && (upper == "MANIFEST.MF"
                            || upper.starts_with("SIG-")
                            || [".SF", ".RSA", ".DSA", ".EC"]
                                .iter()
                                .any(|s| upper.ends_with(s)))
                    {
                        fs::remove_file(meta.path()).map_err(problem)?;
                    }
                }
                if fs::read_dir(entry.path())
                    .map_err(problem)?
                    .next()
                    .is_none()
                {
                    continue;
                }
                base.join("root").join(&name)
            }
            _ => base.join("root").join(&name),
        };
        fs::create_dir_all(target.parent().expect("module directory")).map_err(problem)?;
        fs::rename(entry.path(), target).map_err(problem)?;
    }
    let module = dir.join("base.zip");
    run(jdk("jar")
        .args(["cMf"])
        .arg(&module)
        .arg("-C")
        .arg(&base)
        .arg("."))?;
    let bundle = dir.join("release.aab");
    run(jdk("java")
        .arg("-jar")
        .arg(&tool)
        .arg("build-bundle")
        .arg(prefixed("--modules=", &module))
        .arg(prefixed("--output=", &bundle)))?;
    sign(&bundle, app, debug)?;
    run(jdk("java")
        .arg("-jar")
        .arg(tool)
        .arg("validate")
        .arg(prefixed("--bundle=", &bundle)))?;
    let mut finished = tempfile::NamedTempFile::new_in(&dest).map_err(problem)?;
    io::copy(
        &mut fs::File::open(bundle).map_err(problem)?,
        finished.as_file_mut(),
    )
    .map_err(problem)?;
    finished.as_file().sync_all().map_err(problem)?;
    let package = dest.join(format!("{name}.aab"));
    finished.persist(&package).map_err(problem)?;
    println!("Android App Bundle {}", package.display());
    Ok(())
}
