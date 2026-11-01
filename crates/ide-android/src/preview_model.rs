//! Resolve the Android Gradle variant and its existing preview inputs.
//!
//! The init script runs in the trusted project's Gradle process. Its result is
//! written to a private temporary file, not mixed with arbitrary build output.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Inputs for layoutlib, all associated with one Gradle module and variant.
#[derive(Clone, Debug)]
pub struct PreviewModel {
    pub module: String,
    pub variant: String,
    pub namespace: String,
    pub res_dir: PathBuf,
    pub extra_res: Vec<PathBuf>,
    pub merged_dir: Option<PathBuf>,
    pub r_txt: Option<PathBuf>,
    pub app_apk: Option<PathBuf>,
    pub classpath: Vec<PathBuf>,
    pub platform: PathBuf,
    pub theme: String,
}

// Gradle's launcher does not recognize Windows verbatim paths as project paths.
#[cfg(windows)]
fn gradle_path(path: PathBuf) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    let plain = if wide.starts_with(&unc) {
        let mut result: Vec<u16> = r"\\".encode_utf16().collect();
        result.extend_from_slice(&wide[unc.len()..]);
        result
    } else if wide.starts_with(&prefix) {
        wide[prefix.len()..].to_vec()
    } else {
        return path;
    };
    PathBuf::from(std::ffi::OsString::from_wide(&plain))
}

#[cfg(not(windows))]
fn gradle_path(path: PathBuf) -> PathBuf { path }

fn run_gradle(root: &Path, script: &Path, model: &Path, layout: &Path, buffer: &Path, task: &str) -> Result<(), String> {
    let wrapper = root.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" });
    if !wrapper.is_file() {
        return Err(format!("Gradle wrapper not found at {}", wrapper.display()));
    }
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd.exe");
        c.arg("/d").arg("/c").arg(&wrapper);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = Command::new(&wrapper);
    cmd.current_dir(root)
        .arg("--no-daemon")
        .arg("-Pkotlin.compiler.execution.strategy=in-process")
        .arg("--console=plain")
        .arg("--init-script").arg(script)
        .arg(task)
        .env("OPENIDEA_PREVIEW_LAYOUT", layout)
        .env("OPENIDEA_PREVIEW_BUFFER", buffer)
        .env("OPENIDEA_PREVIEW_MODEL", model);
    if std::env::var_os("JAVA_HOME").is_none() {
        if let Some(studio) = crate::studio_home() {
            let jbr = studio.join("jbr");
            if jbr.is_dir() { cmd.env("JAVA_HOME", jbr); }
        }
    }
    let output = crate::process::run(cmd, Duration::from_secs(600), None)
        .map_err(|_| format!("Could not run Gradle task {task}; check the Gradle installation and project configuration"))?;
    if !output.success {
        // Build output can contain credentials; never forward it to the renderer.
        return Err(format!("Gradle task {task} failed; run it locally for diagnostics"));
    }
    Ok(())
}

fn decode(value: &str) -> Result<String, String> {
    if value.len() % 2 != 0 { return Err("Malformed Gradle preview model value".into()); }
    let bytes: Result<Vec<_>, _> = value.as_bytes().chunks_exact(2).map(|pair| {
        let digit = |b: u8| (b as char).to_digit(16).ok_or(());
        Ok::<_, ()>((digit(pair[0])? * 16 + digit(pair[1])?) as u8)
    }).collect();
    String::from_utf8(bytes.map_err(|_| "Malformed Gradle preview model value")?)
        .map_err(|_| "Gradle preview model is not UTF-8".into())
}

fn read_model(path: &Path) -> Result<HashMap<String, Vec<String>>, String> {
    let data = std::fs::read_to_string(path).map_err(|_| "Gradle did not produce a preview model".to_string())?;
    let mut fields: HashMap<String, Vec<String>> = HashMap::new();
    for line in data.lines() {
        let (key, value) = line.split_once('=').ok_or("Malformed Gradle preview model")?;
        fields.entry(key.to_string()).or_default().push(decode(value)?);
    }
    if let Some(message) = fields.get("error").and_then(|v| v.first()) {
        return Err(format!("Gradle preview model: {message}"));
    }
    Ok(fields)
}

fn one<'a>(fields: &'a HashMap<String, Vec<String>>, name: &str) -> Result<&'a str, String> {
    match fields.get(name).map(Vec::as_slice) {
        Some([value]) if !value.is_empty() => Ok(value),
        _ => Err(format!("Gradle preview model cannot determine {name}")),
    }
}

/// Selects the matching variant (debug first), reusing existing Gradle outputs.
/// Missing build outputs trigger one build of the selected variant, then a re-query.
pub fn resolve_preview_model(root: &Path, layout: &Path, content: &str) -> Result<PreviewModel, String> {
    let root = root.canonicalize().map_err(|e| format!("Invalid Android project directory: {e}"))?;
    let layout = layout.canonicalize().map_err(|e| format!("Invalid layout file: {e}"))?;
    if !layout.starts_with(&root) { return Err("Layout is outside the Android project".into()); }
    let root = gradle_path(root);
    let layout = gradle_path(layout);
    let temp = tempfile::Builder::new().prefix("openidea-gradle-model-").tempdir()
        .map_err(|e| format!("Cannot create Gradle model directory: {e}"))?;
    let script = temp.path().join("preview.gradle");
    let output = temp.path().join("model.txt");
    let buffer = temp.path().join("layout.xml");
    std::fs::write(&buffer, content).map_err(|e| format!("Cannot write preview buffer: {e}"))?;
    std::fs::write(&script, include_str!("preview_model.gradle")).map_err(|e| format!("Cannot write Gradle init script: {e}"))?;
    run_gradle(&root, &script, &output, &layout, &buffer, "openideaPreviewModel")?;
    let mut fields = read_model(&output)?;
    let module = one(&fields, "module")?.to_owned();
    let variant = one(&fields, "variant")?.to_owned();
    let missing = !Path::new(one(&fields, "manifest")?).is_file()
        || !Path::new(one(&fields, "symbols")?).is_file()
        || (fields.contains_key("apkdir") && fields.get("apk").and_then(|v| v.first()).is_none_or(|p| !Path::new(p).is_file()))
        || fields.contains_key("missingClasses");
    if missing {
        let mut chars = variant.chars();
        let cap = chars.next().ok_or("Missing Gradle variant")?.to_uppercase().collect::<String>() + chars.as_str();
        let task = if module == ":" { format!(":assemble{cap}") } else { format!("{module}:assemble{cap}") };
        run_gradle(&root, &script, &output, &layout, &buffer, &task)?;
        run_gradle(&root, &script, &output, &layout, &buffer, "openideaPreviewModel")?;
        fields = read_model(&output)?;
        if one(&fields, "module")? != module || one(&fields, "variant")? != variant {
            return Err("Gradle selected a different module or variant after the build".into());
        }
    }
    let context = format!("module {module}, variant {variant}");
    if let Some(tasks) = fields.get("missingClasses") {
        return Err(format!("{context}: compiled classes are missing after building: {}", tasks.join(", ")));
    }
    let symbols = PathBuf::from(one(&fields, "symbols")?);
    if !symbols.is_file() { return Err(format!("{context}: runtime resource symbols are missing")); }
    let apk = if fields.contains_key("apkdir") {
        let path = PathBuf::from(one(&fields, "apk").map_err(|_| format!("{context}: no single universal APK found in variant output metadata"))?);
        if !path.is_file() { return Err(format!("{context}: APK is missing")); }
        Some(path)
    } else { None };
    let resources: Vec<PathBuf> = fields.get("res").into_iter().flatten().map(PathBuf::from).filter(|p| p.is_dir()).collect();
    let res_dir = resources.iter().find(|p| p.canonicalize().is_ok_and(|p| layout.starts_with(gradle_path(p)))).cloned()
        .ok_or_else(|| format!("{context}: layout resource root is unavailable"))?;
    let mut extra_res: Vec<PathBuf> = resources.into_iter().filter(|p| p != &res_dir).collect();
    for path in fields.get("dependencyResources").into_iter().flatten().map(PathBuf::from) {
        if path.is_dir() && !extra_res.contains(&path) { extra_res.push(path); }
    }
    let compile_sdk = one(&fields, "compileSdk")?;
    let mut classpath: Vec<PathBuf> = fields.get("boot").into_iter().flatten().map(PathBuf::from).collect();
    let android_jar = classpath.iter().find(|p| p.file_name().is_some_and(|n| n == "android.jar") && p.is_file())
        .ok_or_else(|| format!("{context}: Gradle did not resolve the android.jar for compile SDK {compile_sdk}"))?;
    let platform = android_jar.parent().ok_or_else(|| format!("{context}: invalid Android platform path"))?.to_path_buf();
    for path in fields.get("resourceClasses").into_iter().chain(fields.get("classpath"))
        .chain(fields.get("projectClasses")).flatten().map(PathBuf::from) {
        if !path.exists() {
            return Err(format!("{context}: classpath entry is missing: {}", path.display()));
        }
        if !classpath.contains(&path) { classpath.push(path); }
    }
    let theme = one(&fields, "theme").map_err(|_| format!("{context}: cannot determine layout or application theme"))?;
    let theme = theme.strip_prefix("@style/").ok_or_else(|| format!("{context}: unsupported theme reference {theme}"))?.to_owned();
    Ok(PreviewModel {
        module, variant, namespace: one(&fields, "namespace")?.to_owned(), res_dir, extra_res,
        merged_dir: fields.get("merged").and_then(|v| v.first()).map(PathBuf::from).filter(|p| p.is_dir()),
        r_txt: Some(symbols), app_apk: apk, classpath, platform, theme,
    })
}
