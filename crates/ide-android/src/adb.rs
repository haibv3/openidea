//! adb: devices, installs, launches and logcat streams.

use crate::process;
use ide_core::{CancellationToken, IdeError, IdeResult};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Listing answers quickly; a wedged adb server must not keep a worker forever.
const QUICK: Duration = Duration::from_secs(20);

/// The port an adb that does not print its allocation uses for JDWP.
const JDWP_PORT: u16 = 5005;

/// One device or emulator as `adb devices -l` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// `emulator-5554`, `R58M12345`, or `host:port`.
    pub serial: String,
    /// `device` (ready) | `offline` | `unauthorized` | `bootloader` | `recovery` | …
    pub state: String,
    pub model: Option<String>,
    pub product: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<String>,
    pub usb: Option<String>,
}

impl Device {
    /// `device`: connected and authorized.
    pub fn online(&self) -> bool {
        self.state == "device"
    }
}

/// What to show from logcat.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    /// Only the process of this package (`pidof` at start; restart logcat after the app restarts).
    pub package: Option<String>,
    /// Lowest level kept: `V` | `D` | `I` | `W` | `E`.
    pub level: Option<String>,
}

/// One `logcat -v threadtime` line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// The line as adb printed it.
    pub raw: String,
    /// `MM-DD HH:MM:SS.mmm`.
    pub timestamp: Option<String>,
    pub pid: Option<u32>,
    pub tid: Option<u32>,
    /// `V` | `D` | `I` | `W` | `E` | `F` | `A` | `S`.
    pub level: Option<String>,
    pub tag: Option<String>,
    pub message: String,
}

/// An adb executable (`platform-tools/adb`).
#[derive(Clone, Debug)]
pub struct Adb {
    path: PathBuf,
}

impl Adb {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn command(&self, serial: Option<&str>, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.path);
        if let Some(serial) = serial {
            cmd.arg("-s").arg(serial);
        }
        cmd.args(args);
        cmd
    }

    /// `Android Debug Bridge version …`.
    pub fn version(&self, cancel: &CancellationToken) -> IdeResult<String> {
        let out = process::run(self.command(None, &args(["version"])), QUICK, Some(cancel))?;
        if !out.success {
            return Err(failed("adb version", &out));
        }
        Ok(out.stdout.lines().next().unwrap_or_default().trim().to_owned())
    }

    /// Connected devices and emulators, including unauthorized / offline ones.
    pub fn devices(&self, cancel: &CancellationToken) -> IdeResult<Vec<Device>> {
        let out = process::run(self.command(None, &args(["devices", "-l"])), QUICK, Some(cancel))?;
        if !out.success {
            return Err(failed("adb devices", &out));
        }
        Ok(parse_devices(&out.stdout))
    }

    /// `adb install [-r] <apk>`; output lines stream through `on_line`.
    pub fn install(
        &self,
        serial: Option<&str>,
        apk: &Path,
        reinstall: bool,
        cancel: &CancellationToken,
        on_line: &mut dyn FnMut(String),
    ) -> IdeResult<()> {
        let mut cmd_args = vec!["install".to_owned()];
        if reinstall {
            cmd_args.push("-r".to_owned());
        }
        cmd_args.push(apk.to_string_lossy().into_owned());
        let (ok, tail) = process::stream(self.command(serial, &cmd_args), cancel, on_line)?;
        if !ok {
            return Err(IdeError::unavailable(format!("adb install failed: {}", tail.join("\n"))));
        }
        Ok(())
    }

    /// Starts `package/activity` (or its launcher activity when `activity` is `None`).
    pub fn launch(
        &self,
        serial: Option<&str>,
        package: &str,
        activity: Option<&str>,
        cancel: &CancellationToken,
    ) -> IdeResult<String> {
        let out = match activity {
            Some(activity) => {
                let words = shell_words(&["shell", "am", "start", "-n", &format!("{package}/{activity}")]);
                process::run(self.command(serial, &words), QUICK, Some(cancel))?
            }
            None => {
                let words =
                    shell_words(&["shell", "monkey", "-p", package, "-c", "android.intent.category.LAUNCHER", "1"]);
                process::run(self.command(serial, &words), QUICK, Some(cancel))?
            }
        };
        if !out.success {
            return Err(failed("adb launch", &out));
        }
        let text = if out.stdout.trim().is_empty() { &out.stderr } else { &out.stdout };
        Ok(text.trim().to_owned())
    }

    pub fn force_stop(&self, serial: Option<&str>, package: &str, cancel: &CancellationToken) -> IdeResult<()> {
        let words = vec!["shell".to_owned(), "am".to_owned(), "force-stop".to_owned(), shell_word(package)?];
        let out = process::run(self.command(serial, &words), QUICK, Some(cancel))?;
        if !out.success {
            return Err(failed("adb force-stop", &out));
        }
        Ok(())
    }

    /// Asks an emulator to shut down (`adb emu kill`). Only works on an
    /// `emulator-<port>` serial; a physical device has no console to talk to.
    pub fn emu_kill(&self, serial: &str, cancel: &CancellationToken) -> IdeResult<()> {
        let out = process::run(self.command(Some(serial), &args(["emu", "kill"])), QUICK, Some(cancel))?;
        if !out.success {
            return Err(failed("adb emu kill", &out));
        }
        Ok(())
    }

    /// The pid of a running app (`adb shell pidof <package>`).
    pub fn pid_of(&self, serial: &str, package: &str, cancel: &CancellationToken) -> IdeResult<u32> {
        let words = vec!["shell".to_owned(), "pidof".to_owned(), shell_word(package)?];
        let out = process::run(self.command(Some(serial), &words), QUICK, Some(cancel))?;
        if !out.success {
            return Err(failed("adb shell pidof", &out));
        }
        parse_pid(&out.stdout)
            .ok_or_else(|| IdeError::unavailable(format!("no process for package '{package}' on the device")))
    }

    /// Forwards the app's JDWP socket to a free local port (`adb forward tcp:0`
    /// prints the one it picked) and answers that port. The debugger then
    /// attaches to `127.0.0.1:<port>` like it would to any JVM.
    pub fn forward_jdwp(&self, serial: &str, package: &str, cancel: &CancellationToken) -> IdeResult<u16> {
        let pid = self.pid_of(serial, package, cancel)?;
        let out = process::run(
            self.command(Some(serial), &["forward".into(), "tcp:0".into(), format!("jdwp:{pid}")]),
            QUICK,
            Some(cancel),
        )?;
        if !out.success {
            return Err(failed("adb forward", &out));
        }
        if let Ok(port) = out.stdout.trim().parse::<u16>() {
            return Ok(port);
        }
        // An adb that prints nothing keeps its default debug port.
        let fixed = JDWP_PORT.to_string();
        let out = process::run(
            self.command(Some(serial), &["forward".into(), format!("tcp:{fixed}"), format!("jdwp:{pid}")]),
            QUICK,
            Some(cancel),
        )?;
        if !out.success {
            return Err(failed("adb forward", &out));
        }
        fixed.parse::<u16>().map_err(|e| IdeError::internal(e.to_string()))
    }

    /// Streams `logcat -v threadtime` until `cancel` fires or adb exits.
    pub fn logcat(
        &self,
        serial: Option<&str>,
        filter: &LogFilter,
        cancel: &CancellationToken,
        on_entry: &mut dyn FnMut(LogEntry),
    ) -> IdeResult<()> {
        let pid = match filter.package.as_deref() {
            Some(package) => self.pidof(serial, package, cancel)?,
            None => None,
        };
        let mut cmd_args = args(["logcat", "-v", "threadtime"]);
        if let Some(pid) = pid {
            cmd_args.push(format!("--pid={pid}"));
        }
        let min = filter.level.as_deref().and_then(level_rank);
        let mut on_line = |line: String| {
            let entry = parse_threadtime(&line);
            if let (Some(min), Some(rank)) = (min, entry.level.as_deref().and_then(level_rank))
                && rank < min
            {
                return;
            }
            on_entry(entry);
        };
        let (ok, tail) = process::stream(self.command(serial, &cmd_args), cancel, &mut on_line)?;
        if !ok && !cancel.is_cancelled() {
            return Err(IdeError::unavailable(format!("adb logcat stopped: {}", tail.join("\n"))));
        }
        Ok(())
    }

    fn pidof(&self, serial: Option<&str>, package: &str, cancel: &CancellationToken) -> IdeResult<Option<u32>> {
        let out = process::run(self.command(serial, &args(["shell", "pidof", package])), QUICK, Some(cancel))?;
        Ok(out.stdout.split_whitespace().next().and_then(|pid| pid.parse().ok()))
    }
}

fn failed(what: &str, out: &process::Output) -> IdeError {
    let detail = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
    IdeError::unavailable(format!("{what}: {detail}"))
}

/// Builds a device shell command: every word is quoted, so a package or path
/// from the project cannot add commands of its own.
fn shell_words(items: &[&str]) -> Vec<String> {
    items.iter().map(|w| shell_word_unchecked(w)).collect()
}

fn shell_word_unchecked(text: &str) -> String {
    if text.chars().all(|c| c.is_ascii_alphanumeric() || ".-_/:$@".contains(c)) {
        return text.to_owned();
    }
    format!("'{}'", text.replace("'", ""))
}

/// A package or process name reaches the device through `/system/bin/sh`, so
/// it travels as one single-quoted word; a quote in it cannot be expressed.
fn shell_word(text: &str) -> IdeResult<String> {
    if text.contains('\'') {
        return Err(IdeError::invalid("package name: quotes are not accepted"));
    }
    Ok(format!("'{text}'"))
}

/// The first pid of `adb shell pidof` output (it can list several).
fn parse_pid(output: &str) -> Option<u32> {
    output.split_whitespace().next()?.parse().ok()
}

fn args<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_owned).collect()
}

fn level_rank(level: &str) -> Option<u8> {
    match level {
        "S" | "V" => Some(0),
        "D" => Some(1),
        "I" => Some(2),
        "W" => Some(3),
        "E" => Some(4),
        "F" | "A" => Some(5),
        _ => None,
    }
}

/// Parses `adb devices -l` output (header, daemon notices and blank lines skipped).
pub fn parse_devices(output: &str) -> Vec<Device> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            !line.starts_with("List of devices") && !line.starts_with('*') && !line.starts_with("adb server")
        })
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_owned();
            let state = parts.next()?.to_owned();
            let mut device =
                Device { serial, state, model: None, product: None, device: None, transport_id: None, usb: None };
            for part in parts {
                let Some((key, value)) = part.split_once(':') else { continue };
                match key {
                    "model" => device.model = Some(value.to_owned()),
                    "product" => device.product = Some(value.to_owned()),
                    "device" => device.device = Some(value.to_owned()),
                    "transport_id" => device.transport_id = Some(value.to_owned()),
                    "usb" => device.usb = Some(value.to_owned()),
                    _ => {}
                }
            }
            Some(device)
        })
        .collect()
}

/// Parses one `logcat -v threadtime` line; anything else (separator banners,
/// wrapped continuation lines) keeps the raw text as its message.
pub fn parse_threadtime(line: &str) -> LogEntry {
    let Some((date, rest)) = take_token(line) else { return raw_entry(line) };
    let Some((time, rest)) = take_token(rest) else { return raw_entry(line) };
    let Some((pid, rest)) = take_token(rest) else { return raw_entry(line) };
    let Some((tid, rest)) = take_token(rest) else { return raw_entry(line) };
    let Some((level, rest)) = take_token(rest) else { return raw_entry(line) };
    if !is_date(date) || !is_time(time) || !is_level(level) {
        return raw_entry(line);
    }
    let body = rest.trim_start();
    let (tag, message) = match body.find(": ") {
        Some(at) => (body[..at].trim_end(), &body[at + 2..]),
        None => (body.trim_end(), ""),
    };
    LogEntry {
        raw: line.to_owned(),
        timestamp: Some(format!("{date} {time}")),
        pid: pid.parse().ok(),
        tid: tid.parse().ok(),
        level: Some(level.to_owned()),
        tag: (!tag.is_empty()).then(|| tag.to_owned()),
        message: message.to_owned(),
    }
}

fn raw_entry(line: &str) -> LogEntry {
    LogEntry {
        raw: line.to_owned(),
        timestamp: None,
        pid: None,
        tid: None,
        level: None,
        tag: None,
        message: line.to_owned(),
    }
}

fn take_token(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let end = line.find(char::is_whitespace)?;
    Some((&line[..end], &line[end..]))
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5 && bytes[2] == b'-' && bytes.iter().enumerate().all(|(i, b)| i == 2 || b.is_ascii_digit())
}

fn is_time(text: &str) -> bool {
    text.len() >= 8 && text.contains(':')
}

fn is_level(text: &str) -> bool {
    matches!(text, "V" | "D" | "I" | "W" | "E" | "F" | "A" | "S")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_shell_words_cannot_carry_extra_commands() {
        assert_eq!(
            shell_words(&["shell", "am", "force-stop", "com.example.app"]),
            ["shell", "am", "force-stop", "com.example.app"]
        );
        assert_eq!(
            shell_words(&["shell", "am", "start", "-n", "com.example/.Main$Activity"])[3..],
            ["-n", "com.example/.Main$Activity"]
        );
        assert_eq!(shell_words(&["shell", "pidof", "a;rm -rf /"]), ["shell", "pidof", "'a;rm -rf /'"]);
        assert_eq!(
            parse_pid(
                " 1234 5678
"
            ),
            Some(1234)
        );
        assert_eq!(
            parse_pid(
                "
"
            ),
            None
        );
    }

    #[test]
    fn parses_devices_with_properties_and_skips_noise() {
        let output = "\
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
emulator-5554          device product:sdk_gphone64_x86_64 model:sdk_gphone64_x86_64 device:emu64xa transport_id:1
R58M12345              unauthorized usb:1-1 transport_id:2
0123456789ABCDEF       offline

";
        let devices = parse_devices(output);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert!(devices[0].online());
        assert_eq!(devices[0].model.as_deref(), Some("sdk_gphone64_x86_64"));
        assert_eq!(devices[0].transport_id.as_deref(), Some("1"));
        assert_eq!(devices[1].state, "unauthorized");
        assert!(!devices[1].online());
        assert_eq!(devices[1].usb.as_deref(), Some("1-1"));
        assert_eq!(devices[2].state, "offline");
    }

    #[test]
    fn parses_threadtime_lines() {
        let entry =
            parse_threadtime("08-20 12:34:56.789  1234  5678 I ActivityManager: Start proc 1234:com.example/u0a123");
        assert_eq!(entry.timestamp.as_deref(), Some("08-20 12:34:56.789"));
        assert_eq!(entry.pid, Some(1234));
        assert_eq!(entry.tid, Some(5678));
        assert_eq!(entry.level.as_deref(), Some("I"));
        assert_eq!(entry.tag.as_deref(), Some("ActivityManager"));
        assert_eq!(entry.message, "Start proc 1234:com.example/u0a123");
    }

    #[test]
    fn banners_and_continuations_stay_raw() {
        let line = "--------- beginning of main";
        let entry = parse_threadtime(line);
        assert_eq!(entry.raw, line);
        assert_eq!(entry.message, line);
        assert!(entry.level.is_none());
    }

    #[test]
    fn level_ranks_order_severity() {
        assert!(level_rank("E") > level_rank("I"));
        assert!(level_rank("I") > level_rank("D"));
        assert_eq!(level_rank("?"), None);
    }
}
