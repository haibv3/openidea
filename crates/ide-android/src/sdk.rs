//! Finding the Android SDK installed on this machine, and the tools inside it.

use crate::project::AndroidModule;
use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// One `platforms/android-<api>` directory the SDK has installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AndroidPlatform {
    /// API level, e.g. `35`.
    pub api: u32,
    /// Directory name, e.g. `android-35`.
    pub name: String,
    pub path: IpcPath,
}

/// An installed Android SDK: where it is, which tools it has and what it contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AndroidSdk {
    pub root: IpcPath,
    /// `platform-tools/adb`; absent when platform-tools is not installed.
    pub adb: Option<IpcPath>,
    /// `emulator/emulator`; absent when the emulator package is not installed.
    pub emulator: Option<IpcPath>,
    /// `cmdline-tools/latest/bin/sdkmanager`.
    pub sdkmanager: Option<IpcPath>,
    /// `cmdline-tools/latest/bin/avdmanager`.
    pub avdmanager: Option<IpcPath>,
    pub platforms: Vec<AndroidPlatform>,
    /// Installed `build-tools` versions, ascending.
    pub build_tools: Vec<String>,
    /// The open project is an Android one (manifest or Android Gradle plugin).
    pub android_project: bool,
    /// `com.android.application` modules of the open project.
    pub modules: Vec<AndroidModule>,
    /// Where the SDK was found: `ANDROID_HOME`, `ANDROID_SDK_ROOT`,
    /// `local.properties` or `default`.
    pub source: String,
}

impl AndroidSdk {
    /// adb of this SDK, when `platform-tools` is installed.
    pub fn adb(&self) -> Option<crate::Adb> {
        self.adb.as_ref().map(|p| crate::Adb::new(p.0.clone()))
    }
}

/// The SDK a project uses: `ANDROID_HOME` / `ANDROID_SDK_ROOT`, then the
/// project's `local.properties` (`sdk.dir`, written by Android Studio), then the
/// usual install locations.
pub fn discover(project_root: Option<&Path>) -> Option<AndroidSdk> {
    let mut candidates: Vec<(PathBuf, &'static str)> = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(dir) = std::env::var_os(var).map(PathBuf::from) {
            candidates.push((dir, var));
        }
    }
    if let Some(project) = project_root
        && let Some(dir) = local_properties_sdk(project)
    {
        candidates.push((dir, "local.properties"));
    }
    candidates.extend(default_roots().into_iter().map(|dir| (dir, "default")));
    candidates.into_iter().find(|(dir, _)| is_sdk(dir)).map(|(root, source)| {
        let mut sdk = load(root, source);
        sdk.android_project = project_root.is_some_and(is_android_project);
        if sdk.android_project
            && let Some(project) = project_root
        {
            sdk.modules = crate::project::modules(project);
        }
        sdk
    })
}

/// Whether `root` looks like an Android project: a manifest or the Android Gradle plugin.
pub fn is_android_project(root: &Path) -> bool {
    const MANIFESTS: [&str; 4] = [
        "AndroidManifest.xml",
        "app/src/main/AndroidManifest.xml",
        "src/main/AndroidManifest.xml",
        "app/src/androidTest/AndroidManifest.xml",
    ];
    if MANIFESTS.iter().any(|rel| root.join(rel).is_file()) {
        return true;
    }
    const SCRIPTS: [&str; 6] = [
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "app/build.gradle",
        "app/build.gradle.kts",
    ];
    SCRIPTS.iter().filter_map(|rel| std::fs::read_to_string(root.join(rel)).ok()).any(|text| {
        text.contains("com.android.application")
            || text.contains("com.android.library")
            || text.contains("org.jetbrains.kotlin.android")
    })
}

/// `<project>/local.properties` `sdk.dir`, when set.
pub fn local_properties_sdk(project: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(project.join("local.properties")).ok()?;
    let value = text.lines().map(str::trim).find_map(|line| {
        let value = line.strip_prefix("sdk.dir")?.trim_start().strip_prefix('=')?.trim();
        (!value.is_empty()).then_some(value)
    })?;
    Some(PathBuf::from(unescape_property(value)))
}

fn is_sdk(dir: &Path) -> bool {
    dir.is_dir() && ["platform-tools", "platforms", "cmdline-tools", "tools"].iter().any(|sub| dir.join(sub).is_dir())
}

fn load(root: PathBuf, source: &str) -> AndroidSdk {
    AndroidSdk {
        adb: tool(&root, &["platform-tools/adb"]),
        emulator: tool(&root, &["emulator/emulator"]),
        sdkmanager: cmdline_tool(&root, "sdkmanager"),
        avdmanager: cmdline_tool(&root, "avdmanager"),
        platforms: platforms(&root),
        build_tools: build_tools(&root),
        android_project: false,
        modules: Vec::new(),
        root: root.into(),
        source: source.to_owned(),
    }
}

fn tool(root: &Path, rels: &[&str]) -> Option<IpcPath> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    rels.iter().map(|rel| root.join(format!("{rel}{suffix}"))).find(|p| p.is_file()).map(Into::into)
}

fn cmdline_tool(root: &Path, name: &str) -> Option<IpcPath> {
    let file = if cfg!(windows) { format!("{name}.bat") } else { name.to_owned() };
    let mut candidates = vec![root.join("cmdline-tools/latest/bin").join(&file)];
    let mut versions: Vec<PathBuf> = std::fs::read_dir(root.join("cmdline-tools"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("bin").join(&file))
        .collect();
    versions.sort();
    candidates.extend(versions);
    candidates.push(root.join("tools/bin").join(&file));
    candidates.into_iter().find(|p| p.is_file()).map(Into::into)
}

fn platforms(root: &Path) -> Vec<AndroidPlatform> {
    let mut out: Vec<AndroidPlatform> = std::fs::read_dir(root.join("platforms"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let api = name.strip_prefix("android-")?.parse().ok()?;
            Some(AndroidPlatform { api, name, path: entry.path().into() })
        })
        .collect();
    out.sort_by_key(|p| p.api);
    out
}

fn build_tools(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join("build-tools"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort_by_key(|version| version_key(version));
    out
}

fn version_key(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn default_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            out.push(local.join("Android/Sdk"));
        }
        if let Some(home) = home {
            out.push(home.join("AppData/Local/Android/Sdk"));
        }
        out.push(PathBuf::from(r"C:\Android\Sdk"));
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = home {
            out.push(home.join("Library/Android/sdk"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(home) = home {
            out.push(home.join("Android/Sdk"));
        }
        out.push(PathBuf::from("/usr/lib/android-sdk"));
        out.push(PathBuf::from("/opt/android-sdk"));
    }
    out
}

/// Java properties escapes, as written by Android Studio (`C\:\\Users\\…`).
fn unescape_property(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdk_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let adb = dir.path().join("platform-tools").join(if cfg!(windows) { "adb.exe" } else { "adb" });
        std::fs::create_dir_all(adb.parent().unwrap()).unwrap();
        std::fs::write(adb, "").unwrap();
        for platform in ["android-34", "android-35"] {
            std::fs::create_dir_all(dir.path().join("platforms").join(platform)).unwrap();
        }
        for version in ["34.0.0", "35.0.1", "35.0.0"] {
            std::fs::create_dir_all(dir.path().join("build-tools").join(version)).unwrap();
        }
        let bin = dir.path().join("cmdline-tools/latest/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(if cfg!(windows) { "sdkmanager.bat" } else { "sdkmanager" }), "").unwrap();
        dir
    }

    #[test]
    fn loads_platforms_build_tools_and_tools() {
        let dir = sdk_fixture();
        let sdk = load(dir.path().to_path_buf(), "test");
        assert_eq!(sdk.platforms.iter().map(|p| p.api).collect::<Vec<_>>(), vec![34, 35]);
        assert_eq!(sdk.build_tools, vec!["34.0.0", "35.0.0", "35.0.1"]);
        assert!(sdk.adb.is_some());
        assert!(sdk.sdkmanager.is_some());
        assert!(sdk.avdmanager.is_none());
        assert!(sdk.emulator.is_none());
    }

    #[test]
    fn reads_sdk_dir_from_local_properties() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("local.properties"), "sdk.dir=C\\:\\\\Android\\\\Sdk\n").unwrap();
        assert_eq!(local_properties_sdk(dir.path()), Some(PathBuf::from(r"C:\Android\Sdk")));
    }

    #[test]
    fn ignores_local_properties_without_sdk_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("local.properties"), "other=1\n").unwrap();
        assert_eq!(local_properties_sdk(dir.path()), None);
    }

    #[test]
    fn recognizes_android_projects() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_android_project(dir.path()));
        std::fs::create_dir_all(dir.path().join("app/src/main")).unwrap();
        std::fs::write(dir.path().join("app/src/main/AndroidManifest.xml"), "<manifest/>").unwrap();
        assert!(is_android_project(dir.path()));

        let gradle = tempfile::tempdir().unwrap();
        std::fs::write(gradle.path().join("build.gradle.kts"), "plugins { id(\"com.android.application\") }").unwrap();
        assert!(is_android_project(gradle.path()));
    }
}
