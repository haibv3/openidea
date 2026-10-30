//! An online plugin repository: the index of published packages and installing
//! one from a downloaded ZIP, like JetBrains Marketplace.
//!
//! The registry is a static `index.json` plus package archives; anything can
//! host it (a git page, an object store, an intranet server). Nothing here
//! talks to the network — the shell downloads and hands the bytes over.

use crate::manifest::{MANIFEST_FILE, validate_plugin_id};
use ide_core::{IdeError, IdeResult};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// Largest accepted index payload (4 MiB of JSON).
pub const MAX_INDEX_BYTES: usize = 4 << 20;
/// Largest accepted package, compressed and unpacked.
pub const MAX_PACKAGE_BYTES: u64 = 64 << 20;
/// ZIP entries per package.
const MAX_ENTRIES: usize = 2048;

/// One published package. `id`, `name`, `version` and `url` are required; the
/// rest fall back to their default when the registry omits them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub change_notes: Option<String>,
    /// `openidea-api` the package targets; the UI warns before a mismatch is downloaded.
    #[serde(default)]
    pub openidea_api: Option<String>,
    /// Download URL of the package ZIP.
    pub url: String,
    /// Lowercase hex SHA-256 of the ZIP; entries without one install unverified.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Declared size in bytes, for the UI.
    #[serde(default)]
    pub size: Option<u64>,
}

/// `Downloading` / `Installing` progress of [`crate::PluginHost`] package installs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RegistryProgress {
    /// Bytes received so far; `total` when the server sends a length.
    Downloading { downloaded: f64, total: Option<f64> },
    /// Download done; the package is being unpacked and loaded.
    Installing,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Index {
    #[serde(default)]
    plugins: Vec<RegistryPlugin>,
}

/// Parses an `index.json`, dropping entries this host could never install.
pub fn parse_index(text: &str) -> IdeResult<Vec<RegistryPlugin>> {
    if text.len() > MAX_INDEX_BYTES {
        return Err(IdeError::invalid("the plugin index is too large"));
    }
    let index: Index = serde_json::from_str(text).map_err(|e| IdeError::invalid(format!("plugin index: {e}")))?;
    let mut out: Vec<RegistryPlugin> = Vec::new();
    for plugin in index.plugins {
        match problem(&plugin) {
            None if !out.iter().any(|seen: &RegistryPlugin| seen.id == plugin.id) => out.push(plugin),
            None => {}
            Some(reason) => tracing::warn!("registry entry {}: {reason}", plugin.id),
        }
    }
    Ok(out)
}

/// Why the entry cannot be installed, `None` when it can.
fn problem(plugin: &RegistryPlugin) -> Option<&'static str> {
    if validate_plugin_id(&plugin.id).is_err() {
        return Some("has an invalid id");
    }
    if plugin.name.trim().is_empty() {
        return Some("has no name");
    }
    if plugin.version.trim().is_empty() {
        return Some("has no version");
    }
    if package_url_ok(&plugin.url).is_err() {
        return Some("has an unsafe download url");
    }
    match plugin.sha256.as_deref() {
        Some(sha) if sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) => None,
        Some(_) => Some("has a malformed sha256"),
        None => None,
    }
}

/// Requires `https`, except `http://localhost` in debug builds (a local registry).
pub fn package_url_ok(url: &str) -> IdeResult<()> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if cfg!(debug_assertions)
        && ["http://localhost/", "http://127.0.0.1/", "http://[::1]/"].iter().any(|p| lower.starts_with(p))
    {
        return Ok(());
    }
    Err(IdeError::invalid(format!("insecure plugin url: {url}")))
}

/// `true` when `bytes` hash to the lowercase hex `expected`.
pub fn verify_sha256(bytes: &[u8], expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    hex == expected.to_ascii_lowercase()
}

/// Unpacks a package ZIP into `into` and returns the directory holding
/// `plugin.toml` — the archive root, or its single top-level folder.
pub fn extract_package(bytes: &[u8], into: &Path) -> IdeResult<PathBuf> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(IdeError::invalid("the plugin package is too large"));
    }
    fs::create_dir_all(into)?;
    let mut zip = zip::ZipArchive::new(io::Cursor::new(bytes))
        .map_err(|e| IdeError::invalid(format!("reading plugin package: {e}")))?;
    if zip.len() > MAX_ENTRIES {
        return Err(IdeError::invalid("the plugin package has too many files"));
    }
    let mut unpacked: u64 = 0;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|e| IdeError::invalid(format!("reading plugin package: {e}")))?;
        // `enclosed_name` drops absolute paths and anything reaching outside the archive.
        let Some(relative) = file.enclosed_name() else { continue };
        if file.is_symlink() {
            continue;
        }
        let target = into.join(&relative);
        if file.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        unpacked = unpacked.saturating_add(file.size());
        if unpacked > MAX_PACKAGE_BYTES {
            return Err(IdeError::invalid("the plugin package unpacks to too much data"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        io::copy(&mut file, &mut out)?;
    }
    find_package_dir(into).ok_or_else(|| IdeError::invalid("the plugin package has no plugin.toml"))
}

fn find_package_dir(dir: &Path) -> Option<PathBuf> {
    if dir.join(MANIFEST_FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    let mut subdirs: Vec<PathBuf> =
        fs::read_dir(dir).ok()?.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect();
    subdirs.sort();
    (subdirs.len() == 1).then(|| find_package_dir(&subdirs[0])).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
      "plugins": [
        {"id": "hello", "name": "Hello", "version": "0.2.0", "description": "Says hi",
         "authors": ["You"], "url": "https://example.com/hello.zip",
         "sha256": "0000000000000000000000000000000000000000000000000000000000000000"}
      ]
    }"#;

    #[test]
    fn parses_a_valid_entry() {
        let plugins = parse_index(GOOD).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "hello");
        assert_eq!(plugins[0].sha256.as_deref(), Some("0".repeat(64).as_str()));
    }

    #[test]
    fn drops_uninstallable_entries() {
        let text = r#"{"plugins": [
            {"id": "Bad Id", "name": "x", "version": "1", "url": "https://example.com/a.zip"},
            {"id": "ok", "name": "  ", "version": "1", "url": "https://example.com/a.zip"},
            {"id": "ok2", "name": "ok", "version": "", "url": "https://example.com/a.zip"},
            {"id": "ok3", "name": "ok", "version": "1", "url": "http://evil.example.com/a.zip"},
            {"id": "ok4", "name": "ok", "version": "1", "url": "https://example.com/a.zip", "sha256": "zz"}
        ]}"#;
        assert!(parse_index(text).unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_indexes() {
        assert!(parse_index("not json").is_err());
        assert!(parse_index(r#"{"plugins": "nope"}"#).is_err());
    }

    #[test]
    fn sha256_matches_the_known_digest() {
        // echo -n "abc" | sha256sum
        assert!(verify_sha256(b"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(!verify_sha256(b"abc", "0".repeat(64).as_str()));
    }

    fn zip_with(entries: &[(&str, &str)]) -> Vec<u8> {
        use io::Write;
        let mut buffer = io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for (name, content) in entries {
                writer.start_file(*name, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn extracts_a_package_root_or_its_single_folder() {
        let dir = tempfile::tempdir().unwrap();
        let flat = zip_with(&[("plugin.toml", "id = \"a\"\n"), ("dist/plugin.wasm", "x")]);
        let package = extract_package(&flat, &dir.path().join("flat")).unwrap();
        assert!(package.join("plugin.toml").is_file());

        let nested = zip_with(&[("hello-1.0.0/plugin.toml", "id = \"a\"\n")]);
        let package = extract_package(&nested, &dir.path().join("nested")).unwrap();
        assert_eq!(package.file_name().unwrap(), "hello-1.0.0");
    }

    #[test]
    fn refuses_packages_without_a_manifest_or_with_escaping_paths() {
        let dir = tempfile::tempdir().unwrap();
        let no_manifest = zip_with(&[("readme.md", "hi")]);
        assert!(extract_package(&no_manifest, &dir.path().join("none")).is_err());

        let escaping = zip_with(&[("../evil.txt", "hi"), ("plugin.toml", "id = \"a\"\n")]);
        let package = extract_package(&escaping, &dir.path().join("escape")).unwrap();
        assert!(package.join("plugin.toml").is_file());
        assert!(!dir.path().join("evil.txt").exists());
    }
}
