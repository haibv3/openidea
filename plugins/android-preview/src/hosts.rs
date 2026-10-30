//! The Android `res/` tree resolved through host file access (wasm) or an
//! empty in-memory source elsewhere (native tests never touch host imports).

use crate::resources::ResourceSource;

#[cfg(not(target_arch = "wasm32"))]
use crate::resources::MemorySource;

/// The source to use for one render call: host-backed under wasm, empty in
/// native builds, so host imports are never referenced outside wasm.
pub fn res_source(file_path: &str) -> Box<dyn ResourceSource> {
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(HostSource::for_file(file_path))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = file_path;
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

    impl HostSource {
        /// Picks the res root for a previewed file: the file's own `res/`
        /// segment when present, else the standard module locations.
        pub fn for_file(file_path: &str) -> Self {
            if let Some(idx) = file_path.find("/res/") {
                return Self { root: file_path[..idx + 4].to_owned() };
            }
            // Fall back to the first res root that actually lists.
            for candidate in ["app/src/main/res", "res"] {
                if crate::openidea::plugin::host::list_dir(candidate).is_ok() {
                    return Self { root: candidate.to_owned() };
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
