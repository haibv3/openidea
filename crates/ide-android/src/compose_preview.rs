//! Compose previews through the Android CLI and the project's Gradle model.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(600);
const MAX_PNG: u64 = 16 * 1024 * 1024;
// Gradle's launcher does not accept Windows verbatim project paths.
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


#[derive(Deserialize)]
struct Model {
    error: Option<String>,
    module: Option<String>,
    variant: Option<String>,
    #[serde(default)]
    classes: Vec<PathBuf>,
}
#[derive(Deserialize)]
struct PreviewList { success: bool, #[serde(default)] previews: Vec<Preview> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preview { method_fqn: String }
#[derive(Deserialize)]
struct RenderOutput { success: bool, #[serde(default)] results: Vec<Rendered> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rendered { method_fqn: String, preview_id: String, image_path: PathBuf }

fn run(cmd: Command, operation: &str) -> Result<String, String> {
    let output = crate::process::run(cmd, TIMEOUT, None)
        .map_err(|e| format!("{operation} could not complete: {e}"))?;
    if !output.success { return Err(format!("{operation} failed; run it locally for diagnostics")); }
    Ok(output.stdout)
}

fn saved(file: &Path, content: &str) -> Result<(), String> {
    let bytes = std::fs::read_to_string(file).map_err(|e| format!("Cannot read Compose source: {e}"))?;
    if bytes.replace("\r\n", "\n") != content.replace("\r\n", "\n") {
        return Err("Save required: Compose preview only renders saved source".into());
    }
    Ok(())
}

fn model(root: &Path, file: &Path, temp: &Path) -> Result<Model, String> {
    let script = temp.join("compose-preview.gradle");
    let output = temp.join("model.json");
    std::fs::write(&script, include_str!("compose_preview.gradle")).map_err(|e| format!("Cannot write Gradle init script: {e}"))?;
    let wrapper = root.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" });
    if !wrapper.is_file() { return Err("Gradle wrapper not found".into()); }
    let mut cmd = Command::new(wrapper);
    cmd.current_dir(root).arg("--no-daemon").arg("-Pkotlin.compiler.execution.strategy=in-process")
        .arg("--console=plain").arg("--init-script").arg(script).arg("openideaComposePreviewModel")
        .env("OPENIDEA_COMPOSE_SOURCE", file).env("OPENIDEA_COMPOSE_MODEL", &output);
    if std::env::var_os("JAVA_HOME").is_none() {
        if let Some(studio) = crate::studio_home() {
            let jbr = studio.join("jbr");
            if jbr.is_dir() { cmd.env("JAVA_HOME", jbr); }
        }
    }
    run(cmd, "Gradle Compose model")?;
    let bytes = std::fs::read(output).map_err(|e| format!("Cannot read Gradle Compose model: {e}"))?;
    let model: Model = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Gradle Compose model: {e}"))?;
    if let Some(error) = &model.error { return Err(error.clone()); }
    if model.module.as_deref().is_none_or(str::is_empty) || model.variant.as_deref().is_none_or(str::is_empty) || model.classes.is_empty() {
        return Err("Gradle did not provide a module, variant and compiled class directories".into());
    }
    Ok(model)
}

fn cli(root: &Path, action: &str, module: &str) -> Command {
    let mut command = Command::new(std::env::var_os("ANDROID_CLI").unwrap_or_else(|| "android".into()));
    command.current_dir(root).arg("--no-metrics").arg("preview").arg(action).arg(module);
    command
}
fn build_args(command: &mut Command) {
    command.arg("--format=json").arg("--").arg("--no-daemon").arg("-Pkotlin.compiler.execution.strategy=in-process");
}

// Read the JVM class's SourceFile attribute rather than guessing Kotlin facade names.
fn source_file(bytes: &[u8]) -> Option<String> {
    struct Reader<'a> { b: &'a [u8], p: usize }
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> Option<&'a [u8]> { let end = self.p.checked_add(n)?; let v = self.b.get(self.p..end)?; self.p = end; Some(v) }
        fn u1(&mut self) -> Option<u8> { Some(self.take(1)?[0]) }
        fn u2(&mut self) -> Option<u16> { Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?)) }
        fn u4(&mut self) -> Option<u32> { Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?)) }
        fn attributes(&mut self) -> Option<()> { let n = self.u2()?; for _ in 0..n { self.u2()?; let len = self.u4()? as usize; self.take(len)?; } Some(()) }
        fn members(&mut self) -> Option<()> { let n = self.u2()?; for _ in 0..n { self.take(6)?; self.attributes()?; } Some(()) }
    }
    let mut r = Reader { b: bytes, p: 0 };
    if r.u4()? != 0xcafebabe { return None; }
    r.take(4)?;
    let count = r.u2()? as usize;
    let mut utf = vec![None; count];
    let mut i = 1;
    while i < count {
        match r.u1()? {
            1 => { let n = r.u2()? as usize; utf[i] = Some(String::from_utf8_lossy(r.take(n)?).into_owned()); }
            3 | 4 => { r.take(4)?; }
            5 | 6 => { r.take(8)?; i += 1; }
            7 | 8 | 16 | 19 | 20 => { r.take(2)?; }
            9 | 10 | 11 | 12 | 17 | 18 => { r.take(4)?; }
            15 => { r.take(3)?; }
            _ => return None,
        }
        i += 1;
    }
    r.take(6)?;
    let interfaces = r.u2()? as usize; r.take(interfaces.checked_mul(2)?)?;
    r.members()?; r.members()?;
    let attributes = r.u2()?;
    for _ in 0..attributes {
        let name = r.u2()? as usize;
        let len = r.u4()? as usize;
        if utf.get(name)?.as_deref() == Some("SourceFile") && len == 2 {
            return utf.get(r.u2()? as usize)?.clone();
        }
        r.take(len)?;
    }
    None
}

fn owns_preview(fqn: &str, file: &Path, classes: &[PathBuf]) -> Result<bool, String> {
    let (class, method) = fqn.rsplit_once('.').ok_or("Invalid preview method FQN")?;
    if method.is_empty() || class.split('.').any(|part| part.is_empty() || !part.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')) {
        return Err("Invalid preview class FQN".into());
    }
    let relative = format!("{}.class", class.replace('.', "/"));
    let mut found = 0;
    let mut matches = false;
    for dir in classes {
        let path = dir.join(&relative);
        if path.is_file() {
            found += 1;
            let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read preview class: {e}"))?;
            let source = source_file(&bytes).ok_or("Preview class has no usable SourceFile metadata")?;
            matches = file.file_name().is_some_and(|name| name == source.as_str());
        }
    }
    if found > 1 { return Err(format!("Ambiguous compiled class for preview {fqn}")); }
    if found == 0 { return Err(format!("Compiled class for preview {fqn} not found in the selected variant")); }
    Ok(matches)
}

fn png(path: &Path, directory: &Path) -> Result<Vec<u8>, String> {
    let path = if path.is_absolute() { path.to_path_buf() } else { directory.join(path) };
    let path = path.canonicalize().map_err(|e| format!("Cannot find rendered image: {e}"))?;
    let dir = directory.canonicalize().map_err(|e| format!("Cannot find preview output directory: {e}"))?;
    if !path.starts_with(dir) { return Err("Rendered image is outside the preview output directory".into()); }
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open rendered image: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_PNG { return Err("Rendered PNG exceeds the size limit".into()); }
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_PNG + 1).read_to_end(&mut bytes).map_err(|e| format!("Cannot read rendered image: {e}"))?;
    if bytes.len() as u64 > MAX_PNG || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") { return Err("Invalid or oversized rendered PNG".into()); }
    let mut pos = 8usize;
    let mut header = false;
    let mut data = false;
    while pos.checked_add(12).is_some_and(|n| n <= bytes.len()) {
        let len = u32::from_be_bytes(bytes[pos..pos+4].try_into().unwrap()) as usize;
        let end = pos.checked_add(12).and_then(|n| n.checked_add(len)).ok_or("Invalid PNG chunk size")?;
        if end > bytes.len() { return Err("Truncated PNG chunk".into()); }
        let kind = &bytes[pos+4..pos+8];
        let mut crc = !0u32;
        for &b in &bytes[pos+4..end-4] { crc ^= b as u32; for _ in 0..8 { crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1))); } }
        if !crc != u32::from_be_bytes(bytes[end-4..end].try_into().unwrap()) { return Err("Invalid PNG checksum".into()); }
        if !header {
            if kind != b"IHDR" || len != 13 { return Err("Invalid PNG header".into()); }
            let w = u32::from_be_bytes(bytes[pos+8..pos+12].try_into().unwrap());
            let h = u32::from_be_bytes(bytes[pos+12..pos+16].try_into().unwrap());
            if w == 0 || h == 0 || w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > 32_000_000 { return Err("Rendered PNG dimensions exceed the limit".into()); }
            header = true;
        } else if kind == b"IDAT" { data = true; }
        if kind == b"IEND" {
            if len != 0 || !data || end != bytes.len() { return Err("Invalid PNG end".into()); }
            return Ok(bytes);
        }
        pos = end;
    }
    Err("Incomplete rendered PNG".into())
}

/// Render previews belonging to a saved source file in its Android Gradle module.
pub fn render_compose_preview(root: &Path, file: &Path, content: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let root = gradle_path(root.canonicalize().map_err(|e| format!("Invalid project directory: {e}"))?);
    let file = gradle_path(file.canonicalize().map_err(|e| format!("Invalid Compose source file: {e}"))?);
    if !file.starts_with(&root) { return Err("Compose source is outside the project".into()); }
    saved(&file, content)?;
    let result = (|| {
        let temp = tempfile::Builder::new().prefix("openidea-compose-").tempdir().map_err(|e| format!("Cannot create preview directory: {e}"))?;
        let model = model(&root, &file, temp.path())?;
        let target = format!("gradle://?{}@{}", model.module.as_deref().unwrap(), model.variant.as_deref().unwrap());
        let module = target.as_str();
        let mut list = cli(&root, "list", module);
        build_args(&mut list);
        let output = run(list, "Android CLI preview list")?;
        let previews: PreviewList = serde_json::from_str(&output).map_err(|e| format!("Invalid Android CLI preview list: {e}"))?;
        if !previews.success { return Err("Android CLI could not list previews".into()); }
        let mut methods = BTreeSet::new();
        for preview in previews.previews {
            if owns_preview(&preview.method_fqn, &file, &model.classes)? { methods.insert(preview.method_fqn); }
        }
        let mut images = Vec::new();
        for method in methods {
            let output_dir = temp.path().join(format!("render-{}", images.len()));
            std::fs::create_dir(&output_dir).map_err(|e| format!("Cannot create render output directory: {e}"))?;
            let mut command = cli(&root, "render", module);
            command.arg(format!("--fqn={method}")).arg(format!("--output={}", output_dir.display()));
            build_args(&mut command);
            let output = run(command, "Android CLI preview render")?;
            let rendered: RenderOutput = serde_json::from_str(&output).map_err(|e| format!("Invalid Android CLI render output: {e}"))?;
            if !rendered.success || rendered.results.is_empty() { return Err(format!("Android CLI could not render {method}")); }
            for item in rendered.results {
                if item.method_fqn != method || item.preview_id.is_empty() { return Err("Android CLI returned an unexpected preview result".into()); }
                let label = format!("{}: {} ({})", item.method_fqn, item.preview_id, images.len() + 1);
                images.push((label, png(&item.image_path, &output_dir)?));
            }
        }
        Ok(images)
    })();
    saved(&file, content)?;
    result
}

#[cfg(test)]
mod tests {
    use super::saved;

    #[test]
    fn saved_buffer_accepts_editor_line_endings_but_rejects_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Preview.kt");
        std::fs::write(&path, "first\r\nsecond\r\n").unwrap();
        assert!(saved(&path, "first\nsecond\n").is_ok());
        assert!(saved(&path, "first\nchanged\n").unwrap_err().contains("Save required"));
    }
}
