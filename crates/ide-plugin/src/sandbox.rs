//! Capability checks for host functions that touch the file system.

use std::path::{Component, Path, PathBuf};

/// Largest file `read-file` returns.
pub const MAX_READ_BYTES: u64 = 16 << 20;
/// Largest body `write-file` accepts.
pub const MAX_WRITE_BYTES: usize = 8 << 20;
/// Largest file `write-file` replaces.
pub const MAX_EXISTING_WRITE_BYTES: u64 = 8 << 20;
/// Entries `list-dir` returns.
pub const MAX_DIR_ENTRIES: usize = 2000;

/// Resolves `path` (absolute, or relative to `root`) and checks that the real
/// path — after following `..` and symlinks — stays inside `root`.
pub fn resolve_in_root(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    let root = root.canonicalize().map_err(|e| format!("project root unavailable: {e}"))?;
    let requested = Path::new(path);
    let joined = if requested.is_absolute() { requested.to_path_buf() } else { contained(&root, requested)? };
    // Rejecting a lexical escape before touching the disk keeps the message the
    // same whether or not the target exists.
    let real = joined.canonicalize().map_err(|_| format!("{path}: no such file"))?;
    if !real.starts_with(&root) {
        return Err(format!("{path}: access outside the project root is not allowed"));
    }
    Ok(real)
}

/// Walks `relative` from `root`, refusing anything that climbs out of it.
fn contained(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let outside = || Err(format!("{}: access outside the project root is not allowed", relative.display()));
    let mut real = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => real.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !real.pop() || !real.starts_with(root) {
                    return outside();
                }
            }
            _ => return outside(),
        }
    }
    Ok(real)
}

/// Reads a UTF-8 file that `resolve_in_root` already approved.
pub fn read_text(real: &Path) -> Result<String, String> {
    let bytes = read_bytes(real)?;
    String::from_utf8(bytes).map_err(|_| format!("{}: not valid UTF-8 text", real.display()))
}

/// Reads a file that `resolve_in_root` already approved, as raw bytes.
pub fn read_bytes(real: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(real).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("{}: not a file", real.display()));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!("{}: file is larger than {} MiB", real.display(), MAX_READ_BYTES >> 20));
    }
    std::fs::read(real).map_err(|e| e.to_string())
}

/// Resolves a path that may not exist yet, keeping it inside `root`.
///
/// `path` must be relative to `root`; `..` may not climb out of it, and the
/// nearest existing ancestor is canonicalized so a symlink there cannot leave
/// the project.
pub fn resolve_new_in_root(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    let root = root.canonicalize().map_err(|e| format!("project root unavailable: {e}"))?;
    let requested = Path::new(path);
    if requested.is_absolute() {
        return Err(format!("{path}: write paths must be relative to the project root"));
    }
    let real = contained(&root, requested)?;
    if real == root {
        return Err(format!("{path}: the project root itself is not a target"));
    }
    // The nearest existing ancestor is canonicalized, so a symlink there cannot
    // lead outside the project.
    let mut anchor = real.as_path();
    while !anchor.exists() {
        match anchor.parent() {
            Some(parent) => anchor = parent,
            None => break,
        }
    }
    let anchor_real = anchor.canonicalize().map_err(|e| format!("{path}: {e}"))?;
    if !anchor_real.starts_with(&root) {
        return Err(format!("{path}: access outside the project root is not allowed"));
    }
    Ok(real)
}

/// Writes UTF-8 text over `real` (already resolved inside the root).
pub fn write_text(real: &Path, text: &str) -> Result<(), String> {
    if text.len() > MAX_WRITE_BYTES {
        return Err(format!("{}: more than {} MiB of text", real.display(), MAX_WRITE_BYTES >> 20));
    }
    if let Ok(meta) = std::fs::metadata(real) {
        if !meta.is_file() {
            return Err(format!("{}: not a file", real.display()));
        }
        if meta.len() > MAX_EXISTING_WRITE_BYTES {
            return Err(format!("{}: larger than {} MiB", real.display(), MAX_EXISTING_WRITE_BYTES >> 20));
        }
    }
    let Some(parent) = real.parent() else { return Err(format!("{}: has no parent", real.display())) };
    if !parent.is_dir() {
        return Err(format!("{}: no such directory (use create-dir)", parent.display()));
    }
    std::fs::write(real, text).map_err(|e| format!("{}: {e}", real.display()))
}

/// Creates `real` and its parents inside the root.
pub fn create_dir(real: &Path) -> Result<(), String> {
    std::fs::create_dir_all(real).map_err(|e| format!("{}: {e}", real.display()))
}

/// Deletes `real`, which must be an existing file.
pub fn delete_file(real: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(real).map_err(|e| format!("{}: {e}", real.display()))?;
    if meta.is_dir() {
        return Err(format!("{}: is a directory; only files can be deleted", real.display()));
    }
    std::fs::remove_file(real).map_err(|e| format!("{}: {e}", real.display()))
}

/// One entry of a directory listing.
#[derive(Debug)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Lists `real` (already resolved), sorted, capped at [`MAX_DIR_ENTRIES`].
pub fn list_dir(real: &Path) -> Result<Vec<DirEntry>, String> {
    let meta = std::fs::metadata(real).map_err(|e| format!("{}: {e}", real.display()))?;
    if !meta.is_dir() {
        return Err(format!("{}: not a directory", real.display()));
    }
    let mut entries: Vec<DirEntry> = std::fs::read_dir(real)
        .map_err(|e| format!("{}: {e}", real.display()))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = path.is_dir();
            let size = if is_dir { 0 } else { path.metadata().ok()?.len() };
            Some(DirEntry { name, is_dir, size })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries.truncate(MAX_DIR_ENTRIES);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_stay_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();

        let new = resolve_new_in_root(&root, "src/gen.kt").unwrap();
        write_text(&new, "fun main() {}").unwrap();
        assert!(new.is_file());

        let nested = resolve_new_in_root(&root, "out/deep/file.txt").unwrap();
        assert!(write_text(&nested, "x").unwrap_err().contains("no such directory"));
        create_dir(nested.parent().unwrap()).unwrap();
        write_text(&nested, "ok").unwrap();
        assert!(nested.is_file());

        assert!(resolve_new_in_root(&root, "../escape.txt").is_err());
        assert!(resolve_new_in_root(&root, "src/../../escape.txt").is_err());
        assert!(resolve_new_in_root(&root, tmp.path().join("abs.txt").to_str().unwrap()).is_err());
        assert!(resolve_new_in_root(&root, ".").is_err());

        let abs_target = root.join("src/abs.txt");
        write_text(&abs_target, "y").unwrap();
        assert!(delete_file(&abs_target).is_ok());
        assert!(!abs_target.exists());
        assert!(delete_file(&root.join("src")).unwrap_err().contains("directory"));

        let listed = list_dir(&root.join("src")).unwrap();
        assert!(listed.iter().any(|e| e.name == "gen.kt" && !e.is_dir));
        assert!(list_dir(&root.join("src/gen.kt")).unwrap_err().contains("not a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_ancestor_cannot_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(tmp.path(), root.join("link")).unwrap();
        assert!(resolve_new_in_root(&root, "link/outside.txt").unwrap_err().contains("outside"));
    }

    #[test]
    fn rejects_oversized_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a.txt");
        let huge = "x".repeat(MAX_WRITE_BYTES + 1);
        assert!(write_text(&target, &huge).unwrap_err().contains("MiB"));
    }

    #[test]
    fn confines_reads_to_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "hi").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "s").unwrap();

        let ok = resolve_in_root(&root, "src/a.txt").unwrap();
        assert_eq!(read_text(&ok).unwrap(), "hi");
        let abs = root.join("src/a.txt");
        assert!(resolve_in_root(&root, abs.to_str().unwrap()).is_ok());
        assert!(resolve_in_root(&root, "src/../src/a.txt").is_ok());

        let escape = resolve_in_root(&root, "../secret.txt").unwrap_err();
        assert!(escape.contains("outside the project root"), "{escape}");
        let abs_out = tmp.path().join("secret.txt");
        assert!(resolve_in_root(&root, abs_out.to_str().unwrap()).is_err());
        assert!(resolve_in_root(&root, "/etc/passwd").is_err());
        assert!(resolve_in_root(&root, "").is_err());
        assert!(resolve_in_root(&root, "missing.txt").unwrap_err().contains("no such file"));
        assert!(read_text(&resolve_in_root(&root, "src").unwrap()).unwrap_err().contains("not a file"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("secret.txt"), root.join("link")).unwrap();
        assert!(resolve_in_root(&root, "link").unwrap_err().contains("outside"));
    }

    #[test]
    fn rejects_binary() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("b.bin"), [0xff, 0xfe, 0x00]).unwrap();
        let p = resolve_in_root(tmp.path(), "b.bin").unwrap();
        assert!(read_text(&p).unwrap_err().contains("UTF-8"));
    }
}
