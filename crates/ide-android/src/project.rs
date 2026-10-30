//! Android Gradle projects: application modules, application ids and APKs.

use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One `com.android.application` module of the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AndroidModule {
    /// Project-relative directory (`app`); empty for a module at the project root.
    pub dir: String,
    /// `applicationId` of the build script, else the manifest's `package`.
    pub application_id: Option<String>,
    /// Build types declared in `buildTypes`; `debug` first.
    pub variants: Vec<String>,
    pub path: IpcPath,
}

/// Application modules of the Android project at `root`, from `settings.gradle*`
/// includes (or the root and its immediate subdirectories without one).
pub fn modules(root: &Path) -> Vec<AndroidModule> {
    let mut out = Vec::new();
    for dir in candidate_dirs(root) {
        let module_dir = if dir.is_empty() { root.to_path_buf() } else { root.join(&dir) };
        let Some(script) = read_script(&module_dir) else { continue };
        if !script.contains("com.android.application") {
            continue;
        }
        out.push(AndroidModule {
            application_id: parse_application_id(&script).or_else(|| manifest_package(&module_dir)),
            variants: variants(&script),
            path: module_dir.into(),
            dir,
        });
    }
    out
}

/// Gradle task that builds `variant` of `module` (`assembleDebug`, `:app:assembleDebug`).
pub fn assemble_task(module: &str, variant: &str) -> String {
    let mut chars = variant.chars();
    let task = match chars.next() {
        Some(first) => format!("assemble{}{}", first.to_uppercase(), chars.as_str()),
        None => "assemble".to_owned(),
    };
    if module.is_empty() { task } else { format!(":{module}:{task}") }
}

/// The newest APK of `variant` under `<module>/build/outputs/apk`, if the build produced one.
pub fn find_apk(module_dir: &Path, variant: &str) -> Option<PathBuf> {
    let mut apks = Vec::new();
    collect_apks(&module_dir.join("build/outputs/apk"), 0, &mut apks);
    apks.retain(|path| path.components().any(|part| part.as_os_str().eq_ignore_ascii_case(variant)));
    apks.into_iter().max_by_key(|path| path.metadata().and_then(|meta| meta.modified()).ok())
}

fn collect_apks(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_apks(&path, depth + 1, out);
        } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("apk")) {
            out.push(path);
        }
    }
}

fn candidate_dirs(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(settings) = read_settings(root) {
        for line in settings.lines().filter(|line| line.contains("include")) {
            for quoted in line.split(['"', '\'']).skip(1).step_by(2) {
                for name in quoted.split(',') {
                    let name = name.trim().trim_start_matches(':').trim();
                    if !name.is_empty() {
                        out.push(name.replace(':', "/"));
                    }
                }
            }
        }
    }
    if out.is_empty() {
        if has_build_script(root) {
            out.push(String::new());
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut dirs: Vec<String> = entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !name.starts_with('.') && name != "build")
                .filter(|name| has_build_script(&root.join(name)))
                .collect();
            dirs.sort();
            out.extend(dirs);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn read_settings(root: &Path) -> Option<String> {
    read_first(root, &["settings.gradle", "settings.gradle.kts"])
}

fn read_script(module_dir: &Path) -> Option<String> {
    read_first(module_dir, &["build.gradle", "build.gradle.kts"])
}

fn read_first(dir: &Path, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| std::fs::read_to_string(dir.join(name)).ok())
}

fn has_build_script(dir: &Path) -> bool {
    dir.join("build.gradle").is_file() || dir.join("build.gradle.kts").is_file()
}

/// `applicationId "com.example"` / `applicationId = "com.example"`; variables are not resolved.
fn parse_application_id(script: &str) -> Option<String> {
    for (at, _) in script.match_indices("applicationId") {
        let rest = &script[at + "applicationId".len()..];
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '=') {
            continue;
        }
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('=').map(str::trim_start).unwrap_or(rest);
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let Some(end) = rest[1..].find(quote) else { continue };
        let id = &rest[1..1 + end];
        if !id.is_empty() {
            return Some(id.to_owned());
        }
    }
    None
}

/// `<manifest package="…">`, for projects that do not set `applicationId`.
fn manifest_package(module_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(module_dir.join("src/main/AndroidManifest.xml")).ok()?;
    let at = text.find("package=")?;
    let rest = &text[at + "package=".len()..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = rest[1..].find(quote)?;
    Some(rest[1..1 + end].to_owned())
}

/// Build types of the `buildTypes { … }` block, `debug` first.
fn variants(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(start) = script.find("buildTypes")
        && let Some(open) = script[start..].find('{').map(|at| start + at)
    {
        let mut depth = 0usize;
        let mut word = String::new();
        for ch in script[open..].chars() {
            match ch {
                '{' => {
                    depth += 1;
                    let name = word.trim();
                    if depth == 2 && !name.is_empty() {
                        out.push(name.to_owned());
                    }
                    word.clear();
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    word.clear();
                    if depth == 0 {
                        break;
                    }
                }
                c if c.is_ascii_alphanumeric() || c == '_' => word.push(c),
                c if c.is_whitespace() => {}
                _ => word.clear(),
            }
        }
    }
    if !out.iter().any(|variant| variant == "debug") {
        out.insert(0, "debug".to_owned());
    }
    let mut seen = HashSet::new();
    out.retain(|variant| seen.insert(variant.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn finds_application_modules_and_ids() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("settings.gradle.kts"), "include(\":app\", \":lib\")\n");
        write(
            &dir.path().join("app/build.gradle.kts"),
            "plugins { id(\"com.android.application\"); id(\"org.jetbrains.kotlin.android\") }\nandroid {\n  defaultConfig { applicationId = \"com.example.app\" }\n}\n",
        );
        write(&dir.path().join("lib/build.gradle"), "plugins { id 'com.android.library' }\n");
        let modules = modules(dir.path());
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].dir, "app");
        assert_eq!(modules[0].application_id.as_deref(), Some("com.example.app"));
        assert!(modules[0].variants.contains(&"debug".to_owned()));
    }

    #[test]
    fn falls_back_to_the_manifest_package() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("build.gradle"), "plugins { id 'com.android.application' }\n");
        write(
            &dir.path().join("src/main/AndroidManifest.xml"),
            "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\" package=\"com.example.root\">\n</manifest>\n",
        );
        let modules = modules(dir.path());
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].dir, "");
        assert_eq!(modules[0].application_id.as_deref(), Some("com.example.root"));
    }

    #[test]
    fn reads_build_types() {
        let script = "android {\n  buildTypes {\n    debug { }\n    staging { applicationIdSuffix = \".staging\" }\n    release { isMinifyEnabled = true }\n  }\n}\n";
        assert_eq!(variants(script), vec!["debug", "staging", "release"]);
        assert_eq!(variants("android { }"), vec!["debug"]);
    }

    #[test]
    fn names_tasks_and_finds_the_variant_apk() {
        assert_eq!(assemble_task("app", "debug"), ":app:assembleDebug");
        assert_eq!(assemble_task("", "release"), "assembleRelease");

        let dir = tempfile::tempdir().unwrap();
        let apk = dir.path().join("build/outputs/apk/debug/app-debug.apk");
        write(&apk, "");
        write(&dir.path().join("build/outputs/apk/release/app-release.apk"), "");
        assert_eq!(find_apk(dir.path(), "debug"), Some(apk));
        assert_eq!(find_apk(dir.path(), "release").unwrap().file_name().unwrap(), "app-release.apk");
        assert_eq!(find_apk(dir.path(), "profile"), None);
    }
}
