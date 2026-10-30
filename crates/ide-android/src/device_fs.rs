//! The device's own file system: directory listings (`adb shell ls -l`) and
//! file transfers (`adb pull` / `adb push`), which is what the Device File
//! Explorer needs.

use crate::adb::Adb;
use crate::process;
use ide_core::{CancellationToken, IdeError, IdeResult};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::time::Duration;

/// A listing answers quickly; a wedged adb server must not keep a worker forever.
const QUICK: Duration = Duration::from_secs(20);
/// A whole APK or a log archive can take a while over USB.
const TRANSFER: Duration = Duration::from_secs(300);

/// What kind of entry `ls -l` reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    Dir,
    File,
    /// Symlink or special file: shown, never entered.
    Link,
}

/// One entry of a device directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEntry {
    pub name: String,
    pub kind: DeviceKind,
    /// Bytes; a directory reports its block size, as `ls` does.
    pub size: u64,
    /// `2024-05-01 12:00`, as the device prints it.
    pub modified: Option<String>,
    /// Owner and group, `system:system`.
    pub owner: Option<String>,
    /// The `ls` permission string, `drwxrwx---`.
    pub permissions: String,
}

impl Adb {
    /// `adb -s <serial> shell ls -l <path>`.
    pub fn list_dir(&self, serial: &str, path: &str, cancel: &CancellationToken) -> IdeResult<Vec<DeviceEntry>> {
        let cmd = self.command(Some(serial), &shell_args(&["ls", "-l", path])?);
        let out = process::run(cmd, QUICK, Some(cancel))?;
        if !out.success {
            return Err(failed("adb shell ls", &out));
        }
        Ok(parse_ls(&out.stdout))
    }

    /// `adb -s <serial> pull <remote> <local>`; answers the size written.
    pub fn pull(&self, serial: &str, remote: &str, local: &Path, cancel: &CancellationToken) -> IdeResult<u64> {
        let cmd = self.command(Some(serial), &["pull".into(), word(remote)?, local.display().to_string()]);
        let out = process::run(cmd, TRANSFER, Some(cancel))?;
        if !out.success {
            return Err(failed("adb pull", &out));
        }
        Ok(std::fs::metadata(local).map(|m| m.len()).unwrap_or(0))
    }

    /// `adb -s <serial> push <local> <remote>`.
    pub fn push(&self, serial: &str, local: &Path, remote: &str, cancel: &CancellationToken) -> IdeResult<()> {
        let cmd = self.command(Some(serial), &["push".into(), local.display().to_string(), word(remote)?]);
        let out = process::run(cmd, TRANSFER, Some(cancel))?;
        if !out.success {
            return Err(failed("adb push", &out));
        }
        Ok(())
    }
}

/// `adb shell` hands its arguments to `/system/bin/sh` on the device, so every
/// path becomes one single-quoted word. A quote in a path cannot be expressed
/// that way, and a relative path would list whatever the shell's cwd happens
/// to be — both are refused instead of run.
fn word(path: &str) -> IdeResult<String> {
    if path.contains('\'') {
        return Err(IdeError::invalid("device path: paths containing a quote are not supported"));
    }
    if !path.starts_with('/') {
        return Err(IdeError::invalid("device path: expected an absolute path like /sdcard/Download"));
    }
    Ok(format!("'{path}'"))
}

fn shell_args(items: &[&str]) -> IdeResult<Vec<String>> {
    let mut args = vec!["shell".to_owned()];
    for item in items {
        // The leading words are the command itself; only paths need quoting.
        args.push(if item.starts_with('/') { word(item)? } else { (*item).to_owned() });
    }
    Ok(args)
}

/// Parses `ls -l` output. The `total …` banner, lines that do not start with a
/// permission column, and the older `May 1 2024` date format are skipped: a
/// device that formats dates differently lists fewer entries rather than
/// garbage ones.
pub fn parse_ls(output: &str) -> Vec<DeviceEntry> {
    output.lines().filter_map(|line| parse_entry(line.trim_end())).collect()
}

fn parse_entry(line: &str) -> Option<DeviceEntry> {
    let mut rest = line.trim_start();
    let mut cols: [&str; 7] = [""; 7];
    for col in cols.iter_mut() {
        let end = rest.find(char::is_whitespace)?;
        *col = &rest[..end];
        rest = rest[end..].trim_start();
    }
    if rest.is_empty() {
        return None;
    }
    let kind = match cols[0].as_bytes().first()? {
        b'd' => DeviceKind::Dir,
        b'l' => DeviceKind::Link,
        b'-' => DeviceKind::File,
        _ => return None,
    };
    let size = cols[4].parse::<u64>().ok()?;
    let (date, time) = (cols[5], cols[6]);
    if !date.contains('-') || !time.contains(':') {
        return None;
    }
    let name = match rest.split_once(" -> ") {
        Some((name, _target)) => name,
        None => rest,
    };
    let name = name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')).unwrap_or(name);
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(DeviceEntry {
        name: name.to_owned(),
        kind,
        size,
        modified: Some(format!("{date} {time}")),
        owner: Some(format!("{}:{}", cols[2], cols[3])),
        permissions: cols[0].to_owned(),
    })
}

fn failed(what: &str, out: &process::Output) -> IdeError {
    let detail = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
    IdeError::unavailable(format!("{what}: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "total 132
drwxrwx--x  3 system system 4096 2024-05-01 12:00 adb
-rw-r--r--  1 root root 1289 2024-05-01 09:14 build.prop
lrwxrwxrwx  1 root root 16 2024-05-01 09:14 system -> /system/bin/sh
-rw-------  1 u0_a128 u0_a128 10485760 2024-06-11 23:59 My Report.pdf
drwxrwx---  2 root cache 32768 2024-01-01 00:00 vendor
not a listing
total 8
";

    #[test]
    fn lists_the_entries_of_a_device_directory() {
        let entries = parse_ls(SAMPLE);
        let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            names,
            vec![
                ("adb", DeviceKind::Dir),
                ("build.prop", DeviceKind::File),
                ("system", DeviceKind::Link),
                ("My Report.pdf", DeviceKind::File),
                ("vendor", DeviceKind::Dir),
            ]
        );
        let report = entries.iter().find(|e| e.name == "My Report.pdf").unwrap();
        assert_eq!(
            (report.size, report.modified.as_deref(), report.owner.as_deref()),
            (10485760, Some("2024-06-11 23:59"), Some("u0_a128:u0_a128"))
        );
        assert_eq!(entries[0].permissions, "drwxrwx--x");
    }

    #[test]
    fn skips_what_is_not_a_listing_line() {
        let legacy = "total 4
drwxrwx--x 3 root root 4096 May  1 2024 adb
";
        assert!(parse_ls(legacy).is_empty());
        assert!(parse_ls("").is_empty());
        assert!(parse_ls("-rw-r--r-- 1 root root 5 2024-05-01 10:00 .").is_empty());
    }

    #[test]
    fn device_paths_are_absolute_and_quote_safe() {
        assert_eq!(word("/sdcard/Download").unwrap(), "'/sdcard/Download'");
        assert!(word("sdcard/x").unwrap_err().message.contains("absolute"));
        assert!(word("/tmp/a'b").unwrap_err().message.contains("quote"));
        assert_eq!(shell_args(&["ls", "-l", "/data/local/tmp"]).unwrap(), ["shell", "ls", "-l", "'/data/local/tmp'"]);
    }
}
