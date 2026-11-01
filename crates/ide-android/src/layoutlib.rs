//! Android Studio's preview renderer (layoutlib) driven from this machine.
//!
//! The IDE preview pane is not a reimplementation of Android: it inflates the
//! layout with the real framework code shipped in Android Studio's
//! `layoutlib.jar` and rasterises it with `layoutlib_jni`. This module locates
//! that installation, assembles the classpath the bridge needs, and runs
//! [`RenderBridge`](../bridge/RenderBridge.java) (Java) as a one-shot process
//! that writes a PNG. XML preview uses this experimental bridge; it does not
//! yet reproduce Studio's resource selection or project classloading, so its
//! output must not be treated as Android Studio-equivalent.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Android Studio's layoutlib installation: the renderer plus its data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layoutlib {
    /// `…/plugins/design-tools/lib/layoutlib.jar` (the framework itself).
    pub jar: PathBuf,
    /// `…/plugins/design-tools/resources/layoutlib` (`data/`, `build.prop`).
    pub data: PathBuf,
    /// `…/plugins/android/lib` (`layoutlib-api.jar`, `sdk-common.jar`, …).
    pub lib: PathBuf,
    /// Bundled JDK (`jbr`), which is the JVM layoutlib was built against.
    pub java: PathBuf,
}

/// Where Android Studio is installed: `ANDROID_STUDIO_HOME`, then the usual
/// install locations (the same ones `sdk::discover` uses for the SDK).
pub fn studio_home() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for var in ["ANDROID_STUDIO_HOME", "STUDIO_HOME", "ANDROID_STUDIO"] {
        if let Some(dir) = std::env::var_os(var).map(PathBuf::from) {
            candidates.push(dir);
        }
    }
    #[cfg(windows)]
    {
        for base in ["C:/Program Files/Android", "C:/Program Files", "D:/Software", "D:/Android"] {
            candidates.push(PathBuf::from(base).join("Android Studio"));
            candidates.push(PathBuf::from(base).join("AndroidStudio"));
        }
    }
    #[cfg(target_os = "macos")]
    candidates.push(PathBuf::from("/Applications/Android Studio.app/Contents"));
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        candidates.push(PathBuf::from("/opt/android-studio"));
        candidates.push(PathBuf::from("/usr/local/android-studio"));
    }
    candidates.into_iter().find(|dir| dir.join("plugins/design-tools/lib/layoutlib.jar").is_file())
}

impl Layoutlib {
    /// The layoutlib installation under `studio` (its root, or `Contents` on
    /// macOS). `None` when the design-tools plugin is missing.
    pub fn at(studio: &Path) -> Option<Self> {
        let lib = studio.join("plugins/android/lib");
        let jar = studio.join("plugins/design-tools/lib/layoutlib.jar");
        let data = studio.join("plugins/design-tools/resources/layoutlib");
        if !jar.is_file() || !data.join("data/framework_res.jar").is_file() {
            return None;
        }
        let exe = if cfg!(windows) { "java.exe" } else { "java" };
        let java =
            [studio.join("jbr/bin").join(exe), studio.join("jre/bin").join(exe)].into_iter().find(|p| p.is_file())?;
        Some(Self { jar, data, lib, java })
    }

    /// The layoutlib on this machine, if Android Studio is installed.
    pub fn discover() -> Option<Self> {
        Self::at(&studio_home()?)
    }

    /// Jars needed by layoutlib and Android Studio's framework class loader.
    pub fn classpath(&self) -> Vec<PathBuf> {
        let studio = self.studio_root();
        let mut out = vec![self.jar.clone(), studio.join("plugins/design-tools/lib/design-tools.jar")];
        for name in ["layoutlib-api.jar", "sdk-common.jar", "kxml2-2.3.0.jar", "android.jar"] {
            out.push(self.lib.join(name));
        }
        // Studio versions ship versioned ASM jars.
        if let Ok(entries) = std::fs::read_dir(&self.lib) {
            let mut asm: Vec<_> = entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                    (name.starts_with("asm-") && name.as_bytes().get(4).is_some_and(u8::is_ascii_digit)
                        || name.starts_with("asm-commons-"))
                        && name.ends_with(".jar")
                })
            }).collect();
            asm.sort();
            out.extend(asm);
        }
        for name in ["intellij.libraries.guava.jar", "util-8.jar", "util_rt.jar"] {
            out.push(studio.join("lib").join(name));
        }
        // Kotlin stdlib: sdk-common's resource maps are written in Kotlin.
        for name in ["kotlin-stdlib.jar"] {
            out.push(studio.join("plugins/Kotlin/kotlinc/lib").join(name));
        }
        out
    }

    /// Studio root: `<studio>/plugins/design-tools/lib/layoutlib.jar` → `<studio>`.
    pub fn studio_root(&self) -> &Path {
        self.jar.ancestors().nth(4).unwrap_or(Path::new("."))
    }

    /// Native libraries directory (`layoutlib_jni.dll` and its dependency).
    /// It has to be on `PATH` for the JVM to load them on Windows.
    pub fn native_dir(&self) -> PathBuf {
        #[cfg(windows)]
        return self.data.join("data/win/lib64");
        #[cfg(target_os = "macos")]
        return self.data.join("data/mac");
        #[cfg(all(unix, not(target_os = "macos")))]
        self.data.join("data/linux/lib64")
    }

    /// Fonts directory layoutlib reads.
    pub fn fonts(&self) -> PathBuf {
        self.data.join("data/fonts")
    }

    /// ICU data file (`icudt*.dat`) — layoutlib needs the file, not its dir.
    pub fn icu_data(&self) -> Option<PathBuf> {
        let dir = self.data.join("data/icu");
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "dat"))
            .collect();
        entries.sort();
        entries.pop()
    }

    /// `build.prop` next to `data/` — becomes layoutlib's platform properties.
    pub fn build_prop(&self) -> PathBuf {
        self.data.join("build.prop")
    }

    /// SDK platform resources matching the framework bundled with this Layoutlib.
    /// A project compile SDK may differ from Studio's embedded framework.
    pub fn framework_platform(&self, sdk_root: &Path) -> Result<PathBuf, String> {
        let path = self.build_prop();
        let props = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let value = |name: &str| {
            props.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == name).then(|| value.trim()).filter(|v| !v.is_empty())
            })
        };
        let major = value("ro.build.version.sdk").ok_or("Layoutlib build.prop has no SDK version")?;
        let full = value("ro.build.version.sdk_full").unwrap_or(major);
        for version in [full, major] {
            if !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return Err(format!("invalid Layoutlib SDK version: {version}"));
            }
            let platform = sdk_root.join("platforms").join(format!("android-{version}"));
            if platform.join("data/res/values/attrs.xml").is_file() {
                return Ok(platform);
            }
        }
        Err(format!("Layoutlib requires SDK platform android-{full} (or android-{major}) under {}", sdk_root.display()))
    }
}

/// One render request, resolved to absolute paths.
#[derive(Clone, Debug)]
pub struct RenderRequest {
    pub layout: PathBuf,
    /// Editor buffer to render, including changes not yet saved to disk.
    pub content: String,
    /// Android module resource namespace (not necessarily its application ID).
    pub namespace: String,
    pub res_dir: PathBuf,
    /// Extra `res` roots (Gradle's `packaged_res`, `merged.dir`), in order.
    pub extra_res: Vec<PathBuf>,
    /// Gradle's merged `values*/` directory, when the build output is present.
    pub merged_dir: Option<PathBuf>,
    /// Runtime symbol list (`R.txt`) with the real resource ids.
    pub r_txt: Option<PathBuf>,
    /// The built APK, whose `resources.arsc` is the app's compiled table.
    pub app_apk: Option<PathBuf>,
    /// Project classpath: `android.jar`, module classes and dependency jars.
    pub classpath: Vec<PathBuf>,
    pub platform: PathBuf,
    pub theme: String,
    pub width: u32,
    pub height: u32,
}

/// Renders `request` to PNG bytes with `lib`.
pub fn render(lib: &Layoutlib, request: &RenderRequest, bridge_jar: &Path) -> Result<Vec<u8>, String> {
    let native_dir = lib.native_dir();
    if !native_dir.is_dir() {
        return Err(format!("layoutlib native directory not found: {}", native_dir.display()));
    }
    let icu_data = lib.icu_data().ok_or_else(|| "layoutlib ICU data not found".to_owned())?;
    let temp = tempfile::Builder::new()
        .prefix("openidea-layoutlib-")
        .tempdir()
        .map_err(|e| format!("cannot create layoutlib request directory: {e}"))?;
    let dir = temp.path();
    let out = dir.join("out.png");
    let cfg = dir.join("request.properties");
    let cp_file = dir.join("classpath.txt");
    if !request.layout.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("xml")) {
        return Err("layout path must name an XML file".to_owned());
    }
    let name = request.layout.file_name().ok_or_else(|| "layout path has no file name".to_owned())?;
    let layout = dir.join(name);
    std::fs::write(&layout, &request.content).map_err(|e| format!("{}: {e}", layout.display()))?;

    let mut props = String::new();
    let mut put = |k: &str, v: &str| {
        props.push_str(k);
        props.push('=');
        let mut leading = true;
        for ch in v.chars() {
            match ch {
                '\\' => props.push_str("\\\\"),
                '\n' => props.push_str("\\n"),
                '\r' => props.push_str("\\r"),
                '\t' => props.push_str("\\t"),
                ' ' if leading => props.push_str("\\ "),
                _ => props.push(ch),
            }
            leading &= ch == ' ';
        }
        props.push('\n');
    };
    put("studio", &lib.studio_root().to_string_lossy());
    put("nativeDir", &native_dir.to_string_lossy());
    put("icuData", &icu_data.to_string_lossy());
    let sdk_root = request.platform.parent().and_then(Path::parent)
        .ok_or_else(|| format!("invalid Android platform path: {}", request.platform.display()))?;
    let framework_platform = lib.framework_platform(sdk_root)?;
    put("platform", &framework_platform.to_string_lossy());
    put("layout", &layout.to_string_lossy());
    put("namespace", &request.namespace);
    put("res", &request.res_dir.to_string_lossy());
    put("resRoots", &join_paths(&request.extra_res)?);
    put("mergedDir", &request.merged_dir.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    put("rTxt", &request.r_txt.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    put("appApk", &request.app_apk.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    put("theme", &request.theme);
    put("width", &request.width.to_string());
    put("height", &request.height.to_string());
    put("out", &out.to_string_lossy());
    put("classpathFile", &cp_file.to_string_lossy());
    std::fs::write(&cfg, props).map_err(|e| format!("{}: {e}", cfg.display()))?;
    std::fs::write(&cp_file, join_paths(&request.classpath)?).map_err(|e| format!("{}: {e}", cp_file.display()))?;

    let mut classpath = lib.classpath();
    classpath.push(bridge_jar.to_path_buf());
    let classpath = join_paths(&classpath)?;

    let mut cmd = Command::new(&lib.java);
    cmd.arg("-cp").arg(classpath).arg("RenderBridge").arg(&cfg);
    // The JNI libraries resolve each other through PATH on Windows.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![native_dir];
    paths.extend(std::env::split_paths(&path));
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.env("PATH", joined);
    }
    let output = crate::process::run(cmd, Duration::from_secs(60), None)
        .map_err(|e| format!("layoutlib process failed: {}", e.message))?;
    if !output.success {
        return Err(format!("layoutlib failed: {}", output.stderr.trim()));
    }
    std::fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))
}

/// Compiles the bridge into a cache keyed by source and Studio dependencies.
///
/// Compilation happens in a private directory and is published only after
/// javac finishes, so concurrent previews never observe partial class files.
pub fn compile_bridge(lib: &Layoutlib, source: &Path, cache_root: &Path) -> Result<PathBuf, String> {
    let text = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    for path in lib.classpath().iter().chain(std::iter::once(&lib.java)) {
        path.hash(&mut hash);
        if let Ok(meta) = path.metadata() {
            meta.len().hash(&mut hash);
            meta.modified().ok().hash(&mut hash);
        }
    }
    let out = cache_root.join(format!("bridge-v2-{:016x}", hash.finish()));
    let marker = out.join("RenderBridge.class");
    if marker.is_file() {
        return Ok(out);
    }
    std::fs::create_dir_all(cache_root).map_err(|e| format!("{}: {e}", cache_root.display()))?;
    let temp = tempfile::Builder::new().prefix("compile-").tempdir_in(cache_root)
        .map_err(|e| format!("{}: {e}", cache_root.display()))?;
    let javac = lib.java.with_file_name(if cfg!(windows) { "javac.exe" } else { "javac" });
    let javac = if javac.is_file() { javac } else { PathBuf::from("javac") };
    let classpath = join_paths(&lib.classpath())?;
    let mut command = Command::new(&javac);
    command.arg("-nowarn").arg("-cp").arg(&classpath).arg("-d").arg(temp.path()).arg(source);
    let output = crate::process::run(command, Duration::from_secs(120), None)
        .map_err(|e| format!("javac failed: {}", e.message))?;
    if !output.success {
        return Err(format!("javac failed: {}", output.stderr.trim()));
    }
    if let Err(e) = std::fs::rename(temp.path(), &out) {
        if !marker.is_file() {
            return Err(format!("cannot publish {}: {e}", out.display()));
        }
    }
    Ok(out)
}

/// Joins paths with the platform separator (`;` on Windows, `:` elsewhere) so
/// the same string works for `javac -cp` and for the bridge's own classpath
/// file (Java splits it on `File.pathSeparator`).
fn join_paths(paths: &[PathBuf]) -> Result<String, String> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    if let Some(path) = paths.iter().find(|p| p.to_string_lossy().contains(separator)) {
        return Err(format!("path contains the classpath separator: {}", path.display()));
    }
    std::env::join_paths(paths)
        .map(|joined| joined.to_string_lossy().into_owned())
        .map_err(|e| format!("invalid classpath: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_studio_is_not_an_error() {
        assert!(Layoutlib::at(Path::new("C:/definitely/not/here")).is_none());
    }

    #[test]
    fn a_studio_tree_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let studio = dir.path();
        std::fs::create_dir_all(studio.join("plugins/design-tools/lib")).unwrap();
        std::fs::create_dir_all(studio.join("plugins/design-tools/resources/layoutlib/data")).unwrap();
        std::fs::create_dir_all(studio.join("plugins/android/lib")).unwrap();
        std::fs::create_dir_all(studio.join("jbr/bin")).unwrap();
        std::fs::write(studio.join("plugins/design-tools/lib/layoutlib.jar"), b"").unwrap();
        std::fs::write(studio.join("plugins/design-tools/resources/layoutlib/data/framework_res.jar"), b"").unwrap();
        let exe = if cfg!(windows) { "java.exe" } else { "java" };
        std::fs::write(studio.join("jbr/bin").join(exe), b"").unwrap();

        let lib = Layoutlib::at(studio).expect("recognised");
        assert!(lib.classpath()[0].ends_with("layoutlib.jar"));
        assert!(lib.classpath().iter().any(|p| p.ends_with("layoutlib-api.jar")));
    }

    #[test]
    fn selects_resources_matching_layoutlib_framework_not_project_compile_sdk() {
        let dir = tempfile::tempdir().unwrap();
        let sdk = dir.path().join("sdk");
        let platform = sdk.join("platforms/android-37.0/data/res/values");
        std::fs::create_dir_all(&platform).unwrap();
        std::fs::write(platform.join("attrs.xml"), "<resources/>").unwrap();
        std::fs::write(dir.path().join("build.prop"), "ro.build.version.sdk=37\nro.build.version.sdk_full=37.0\n").unwrap();
        let lib = Layoutlib {
            jar: dir.path().join("layoutlib.jar"),
            data: dir.path().to_path_buf(),
            lib: dir.path().to_path_buf(),
            java: dir.path().join("java"),
        };
        assert_eq!(lib.framework_platform(&sdk).unwrap(), sdk.join("platforms/android-37.0"));
        std::fs::remove_file(platform.join("attrs.xml")).unwrap();
        assert!(lib.framework_platform(&sdk).unwrap_err().contains("android-37.0"));
    }

    #[test]
    fn classpath_uses_the_platform_separator() {
        let joined = join_paths(&[PathBuf::from("a.jar"), PathBuf::from("b.jar")]).unwrap();
        assert_eq!(joined, format!("a.jar{}b.jar", if cfg!(windows) { ';' } else { ':' }));
    }

    #[test]
    fn rejects_paths_containing_the_classpath_separator() {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let path = PathBuf::from(format!("dependency{separator}other.jar"));
        assert!(join_paths(&[path]).is_err());
    }

    #[test]
    fn icu_data_picks_the_dat_file() {
        let dir = tempfile::tempdir().unwrap();
        let icu = dir.path().join("data/icu");
        std::fs::create_dir_all(&icu).unwrap();
        std::fs::write(icu.join("icudt78l.dat"), b"").unwrap();
        std::fs::write(icu.join("README"), b"").unwrap();
        let lib = Layoutlib {
            jar: dir.path().join("layoutlib.jar"),
            data: dir.path().to_path_buf(),
            lib: dir.path().to_path_buf(),
            java: dir.path().join("java"),
        };
        assert!(lib.icu_data().unwrap().ends_with("icudt78l.dat"));
    }
}
