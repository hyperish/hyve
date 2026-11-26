//! Build an optional WidgetKit extension for the selected Xcode application and SDK.
use super::{Fail, MobileApp, fail, run, setting};
use hyve_core::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) struct Extension {
    _project: tempfile::TempDir,
    appex: PathBuf,
    host_info: PathBuf,
    bridge: PathBuf,
    group: Option<(String, PathBuf)>,
}

impl Extension {
    pub fn build(
        app: &MobileApp,
        sources: &Path,
        mut host: Command,
        sdk: &str,
        arch: Option<&str>,
        debug: bool,
    ) -> Result<Self, Fail> {
        host.args(["-sdk", sdk, "-showBuildSettings", "-json"]);
        let output = host.output().map_err(fail)?;
        if !output.status.success() {
            return Err(fail(String::from_utf8_lossy(&output.stderr)));
        }
        let Value::List(targets) =
            hyve_serde::from_json::<Value>(&String::from_utf8_lossy(&output.stdout))
                .map_err(fail)?
        else {
            return Err(fail("expected Xcode target settings"));
        };
        let target = targets
            .iter()
            .filter_map(|v| v.get("buildSettings"))
            .find(|s| s.get("WRAPPER_EXTENSION") == Some(&Value::from("app")))
            .ok_or_else(|| fail("expected an application target in the Xcode scheme"))?;
        let value = |key: &str| match target.get(key) {
            Some(Value::String(s)) if !s.is_empty() => Ok(s.as_str()),
            _ => Err(fail(format!("missing Xcode application setting {key}"))),
        };
        let bundle = value("PRODUCT_BUNDLE_IDENTIFIER")?;
        let project = tempfile::tempdir().map_err(fail)?;
        let dir = project.path();
        let pbx = dir.join("Widgets.xcodeproj");
        fs::create_dir(&pbx).map_err(fail)?;
        fs::write(
            pbx.join("project.pbxproj"),
            include_str!("../../templates/mobile/widget-extension/project.pbxproj"),
        )
        .map_err(fail)?;
        fs::write(
            dir.join("WidgetInfo.plist"),
            include_str!("../../templates/mobile/widget-extension/Info.plist"),
        )
        .map_err(fail)?;
        let requirements = fs::read_to_string(sources.join("requirements.json")).map_err(fail)?;
        let requirements: Value = hyve_serde::from_json(&requirements).map_err(fail)?;
        let mut files = fs::read_dir(sources)
            .map_err(fail)?
            .map(|e| e.map(|f| f.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(fail)?;
        files.sort();
        let mut swift = String::new();
        let mut host_swift = fs::read_to_string(sources.join("host/Bridge.swift")).map_err(fail)?;
        for file in files
            .iter()
            .filter(|p| p.extension().is_some_and(|s| s == "swift"))
        {
            let contents = fs::read_to_string(file).map_err(fail)?;
            if file
                .file_name()
                .is_some_and(|n| n != "SergeWidgetBundle.swift")
            {
                host_swift.push_str(&contents);
                host_swift.push('\n');
            }
            swift.push_str(&contents);
            swift.push('\n');
        }
        fs::write(dir.join("Widgets.swift"), swift).map_err(fail)?;
        let bridge = dir.join("Host.swift");
        fs::write(&bridge, host_swift).map_err(fail)?;
        let mut cmd = Command::new("xcrun");
        cmd.current_dir(dir)
            .args(["xcodebuild", "-project"])
            .arg(pbx)
            .args([
                "-target",
                "SergeWidgets",
                "-sdk",
                sdk,
                "-configuration",
                if debug { "Debug" } else { "Release" },
            ])
            .arg(format!("PRODUCT_BUNDLE_IDENTIFIER={bundle}.SergeWidgets"))
            .arg(format!(
                "CURRENT_PROJECT_VERSION={}",
                value("CURRENT_PROJECT_VERSION")?
            ))
            .arg(format!("MARKETING_VERSION={}", value("MARKETING_VERSION")?))
            .arg(setting("CONFIGURATION_BUILD_DIR", &dir.join("products")));
        if let Ok(team) = value("DEVELOPMENT_TEAM") {
            cmd.arg(format!("DEVELOPMENT_TEAM={team}"));
        }
        if let Some(arch) = arch {
            cmd.arg(arch);
        }
        let mut info = String::from("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>");
        let group = if requirements.get("appGroup") == Some(&Value::Bool(true)) {
            let name = match app.settings.get("widget_app_group") {
                Some(Value::String(s)) => s.clone(),
                None => format!("group.{bundle}"),
                _ => return Err(fail("expected a widget_app_group string")),
            };
            if !name.starts_with("group.")
                || !name.split('.').all(|n| {
                    !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return Err(fail("invalid iOS widget App Group identifier"));
            }
            let entitlements = dir.join("Group.entitlements");
            fs::write(&entitlements, format!("<plist version=\"1.0\"><dict><key>com.apple.security.application-groups</key><array><string>{name}</string></array></dict></plist>")).map_err(fail)?;
            cmd.arg(format!("HYVE_WIDGET_APP_GROUP={name}"))
                .arg(setting("HYVE_WIDGET_ENTITLEMENTS", &entitlements));
            info.push_str(&format!(
                "<key>SergeWidgetAppGroup</key><string>{name}</string>"
            ));
            Some((name, entitlements))
        } else {
            None
        };
        if requirements.get("liveActivities") == Some(&Value::Bool(true)) {
            info.push_str("<key>NSSupportsLiveActivities</key><true/>");
        }
        info.push_str("</dict></plist>");
        let host_info = dir.join("HostInfo.plist");
        fs::write(&host_info, info).map_err(fail)?;
        run(cmd.arg("build"))?;
        let appex = dir.join("products/SergeWidgets.appex");
        if !appex.join("SergeWidgets").is_file() {
            return Err(fail("Xcode did not build SergeWidgets.appex"));
        }
        Ok(Self {
            appex,
            host_info,
            bridge,
            group,
            _project: project,
        })
    }

    pub fn configure(&self, cmd: &mut Command) {
        cmd.arg(setting("HYVE_WIDGET_BRIDGE", &self.bridge))
            .arg(setting("HYVE_WIDGET_EXTENSION", &self.appex))
            .arg(setting("HYVE_WIDGET_HOST_PLIST", &self.host_info));
        if let Some((name, path)) = &self.group {
            cmd.arg(format!("HYVE_WIDGET_APP_GROUP={name}"))
                .arg(setting("HYVE_WIDGET_ENTITLEMENTS", path));
        }
    }
    pub fn verify(&self, application: &Path) -> Result<(), Fail> {
        if !application
            .join("PlugIns/SergeWidgets.appex/SergeWidgets")
            .is_file()
        {
            return Err(fail(
                "the Xcode application must embed HYVE_WIDGET_EXTENSION in its PlugIns directory before code signing",
            ));
        }
        Ok(())
    }
}
