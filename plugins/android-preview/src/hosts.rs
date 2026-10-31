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
    /// `list-dir` imports. Missing files and directories simply resolve to
    /// "unresolved reference" in the renderer.
    pub struct HostSource {
        /// Project-relative path of the `res` directory (no trailing slash).
        pub root: String,
    }

    /// Project-relative `res/` directory of a module (`app/src/main/res`).
    fn module_res(module: &str) -> String {
        format!("{module}/src/main/res")
    }

    impl HostSource {
        /// Picks the res root for a previewed file: the `res/` segment of the
        /// file's own path, else the first module that has one.
        pub fn for_file(project_path: &str, file_path: &str) -> Self {
            for path in [project_path, file_path] {
                let normalized = path.replace('\\', "/");
                if let Some(idx) = normalized.find("/res/") {
                    return Self { root: normalized[..idx + 4].to_owned() };
                }
            }
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
            for candidate in candidates {
                if crate::openidea::plugin::host::list_dir(&candidate).is_ok() {
                    return Self { root: candidate };
                }
            }
            Self { root: "res".to_owned() }
        }
    }

    impl ResourceSource for HostSource {
        fn read(&self, path: &str) -> Option<String> {
            crate::openidea::plugin::host::read_file(&format!("{}/{}", self.root, path)).ok()
        }

        fn list(&self, dir: &str) -> Vec<String> {
            let path = if dir.is_empty() { self.root.clone() } else { format!("{}/{}", self.root, dir) };
            match crate::openidea::plugin::host::list_dir(&path) {
                Ok(entries) => entries.into_iter().map(|e| e.name).collect(),
                Err(_) => Vec::new(),
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::HostSource;
