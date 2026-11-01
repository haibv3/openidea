//! The Android `res/` tree resolved through host file access (wasm) or an
//! empty in-memory source elsewhere (native tests never touch host imports).

use crate::resources::ResourceSource;

#[cfg(not(target_arch = "wasm32"))]
use crate::resources::MemorySource;

/// The source to use for one render call: host-backed under wasm, empty in
/// native builds, so host imports are never referenced outside wasm.
/// `project_path` (project-relative, `/` separators) is the reliable input —
/// the absolute `file_path` uses `\` on Windows.
pub fn res_source(project_path: &str, file_path: &str) -> Box<dyn ResourceSource> {
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(HostSource::for_file(project_path, file_path))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (project_path, file_path);
        Box::new(MemorySource::new())
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::ResourceSource;

    /// Reads `res/…` paths from the project through the host `read-file` /
    /// `list-dir` / `read-file-bytes` imports, layering several resource
    /// roots like Gradle does: the module's own `res/` first, then the merged
    /// library resources Gradle leaves in `build/intermediates` (that's where
    /// library styles, theme attributes and `drawable-*dpi` live — Android
    /// Studio previews against the same merged set). Project-relative reads
    /// (`read_project`) also reach the `AndroidManifest.xml` and the Kotlin/Java
    /// sources, for the app theme and custom view superclasses. Missing files
    /// and directories simply resolve to "unresolved reference" in the renderer.
    pub struct HostSource {
        /// Project-relative resource roots, most authoritative first.
        pub roots: Vec<String>,
        /// Gradle module the previewed file belongs to (`""` = project root).
        pub module: String,
    }

    /// Project-relative `res/` directory of a module (`app/src/main/res`).
    fn module_res(module: &str) -> String {
        format!("{module}/src/main/res")
    }

    /// The module a `res/` root belongs to (`app/src/main/res` → `app`).
    fn module_of(res_root: &str) -> String {
        res_root.strip_suffix("/src/main/res").unwrap_or("").to_owned()
    }

    /// Capitalised variant name (`debug` → `Debug`, `freeDebug` → `FreeDebug`),
    /// which is how AGP names its merged-resource directories.
    fn capitalize(variant: &str) -> String {
        let mut c = variant.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    /// Merged-resource fallbacks for a module `res/` path, if the project has a
    /// Gradle build directory (`<module>/build/intermediates/…`). Variants are
    /// discovered rather than assumed, so flavours (`freeDebug`) work too.
    fn merged_roots(module_res: &str) -> Vec<String> {
        let Some(module) = module_res.strip_suffix("/src/main/res") else { return Vec::new() };
        let intermediates = format!("{module}/build/intermediates");
        let mut roots = Vec::new();
        for kind in ["packaged_res", "merged_res"] {
            let base = format!("{intermediates}/{kind}");
            let variants = crate::openidea::plugin::host::list_dir(&base)
                .map(|entries| entries.into_iter().filter(|e| e.is_dir).map(|e| e.name).collect::<Vec<_>>())
                .unwrap_or_default();
            let variants = if variants.is_empty() { vec!["debug".to_owned(), "release".to_owned()] } else { variants };
            for variant in variants {
                let cap = capitalize(&variant);
                for cand in [
                    format!("{base}/{variant}/package{cap}Resources"),
                    format!("{base}/{variant}/merge{cap}Resources"),
                    format!("{base}/{variant}"),
                    format!("{intermediates}/incremental/{variant}/merge{cap}Resources/merged.dir"),
                ] {
                    if crate::openidea::plugin::host::list_dir(&cand).is_ok() {
                        roots.push(cand);
                    }
                }
            }
            if !roots.is_empty() {
                break;
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    impl HostSource {
        /// Picks the res root for a previewed file: the `res/` segment of the
        /// file's own path, else the first module that has one.
        pub fn for_file(project_path: &str, file_path: &str) -> Self {
            let mut primary = None;
            for path in [project_path, file_path] {
                let normalized = path.replace('\\', "/");
                if let Some(idx) = normalized.find("/res/") {
                    primary = Some(normalized[..idx + 4].to_owned());
                    break;
                }
            }
            let primary = primary.unwrap_or_else(|| {
                // A Kotlin/Java file or an odd path: look for a module `res/`.
                let mut candidates = vec!["app/src/main/res".to_owned(), "src/main/res".to_owned(), "res".to_owned()];
                if let Ok(entries) = crate::openidea::plugin::host::list_dir("") {
                    for entry in entries.into_iter().filter(|e| e.is_dir) {
                        let candidate = module_res(&entry.name);
                        if !candidates.contains(&candidate) {
                            candidates.push(candidate);
                        }
                    }
                }
                candidates
                    .into_iter()
                    .find(|c| crate::openidea::plugin::host::list_dir(c).is_ok())
                    .unwrap_or_else(|| "res".to_owned())
            });
            let module = module_of(&primary);
            let mut roots = vec![primary.clone()];
            roots.extend(merged_roots(&primary));
            Self { roots, module }
        }
    }

    impl ResourceSource for HostSource {
        fn read(&self, path: &str) -> Option<String> {
            self.roots.iter().find_map(|root| crate::openidea::plugin::host::read_file(&format!("{root}/{path}")).ok())
        }

        fn read_bytes(&self, path: &str) -> Option<Vec<u8>> {
            self.roots
                .iter()
                .find_map(|root| crate::openidea::plugin::host::read_file_bytes(&format!("{root}/{path}")).ok())
        }

        fn read_project(&self, path: &str) -> Option<String> {
            crate::openidea::plugin::host::read_file(path).ok()
        }

        fn list_project(&self, dir: &str) -> Vec<String> {
            self.list_root(dir)
        }

        fn module(&self) -> &str {
            &self.module
        }

        fn list(&self, dir: &str) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for root in &self.roots {
                let path = if dir.is_empty() { root.clone() } else { format!("{root}/{dir}") };
                for name in self.list_root(&path) {
                    if !out.iter().any(|n| *n == name) {
                        out.push(name);
                    }
                }
            }
            out.sort();
            out
        }

        fn values_files(&self) -> Vec<String> {
            // Least authoritative first: merged library values, packaged, then
            // the module's own values so the app wins on every conflict.
            let mut out = Vec::new();
            for root in self.roots.iter().rev() {
                let mut values_dirs: Vec<(u32, String)> = self
                    .list_root(root)
                    .into_iter()
                    .filter_map(|entry| crate::resources::values_dir_version(&entry).map(|v| (v, entry)))
                    .collect();
                values_dirs.sort();
                for (_, entry) in values_dirs {
                    for file in self.list_root(&format!("{root}/{entry}")) {
                        if file.ends_with(".xml") {
                            if let Ok(text) =
                                crate::openidea::plugin::host::read_file(&format!("{root}/{entry}/{file}"))
                            {
                                out.push(text);
                            }
                        }
                    }
                }
            }
            out
        }
    }

    impl HostSource {
        /// `list-dir` of one absolute project-relative path (empty vec on error).
        fn list_root(&self, path: &str) -> Vec<String> {
            crate::openidea::plugin::host::list_dir(path)
                .map(|entries| entries.into_iter().map(|e| e.name).collect())
                .unwrap_or_default()
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::HostSource;
