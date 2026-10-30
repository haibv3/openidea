//! The emulator: listing AVDs and starting / stopping one.

use crate::process;
use ide_core::{CancellationToken, IdeError, IdeResult};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// `emulator -list-avds` reads `~/.android/avd`; it must not hold a worker forever.
const QUICK: Duration = Duration::from_secs(30);

/// Names of the virtual devices, as `emulator -list-avds` prints them (one per
/// line, `INFO` / `WARNING` noise around it).
pub fn parse_avds(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("INFO") && !line.starts_with("WARNING") && !line.starts_with("ERROR"))
        .map(str::to_owned)
        .collect()
}

/// The AVDs of this emulator installation.
pub fn list_avds(emulator: &Path, cancel: &CancellationToken) -> IdeResult<Vec<String>> {
    let mut cmd = Command::new(emulator);
    cmd.arg("-list-avds");
    let out = process::run(cmd, QUICK, Some(cancel))?;
    if !out.success {
        let text = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
        return Err(IdeError::unavailable(format!("emulator -list-avds: {text}")));
    }
    Ok(parse_avds(&out.stdout))
}

/// An AVD name becomes `emulator -avd <name>`: keep it to what a name can be, so
/// no option can be smuggled in.
fn check_name(name: &str) -> IdeResult<&str> {
    let name = name.trim();
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok { Ok(name) } else { Err(IdeError::invalid(format!("not a usable AVD name: {name:?}"))) }
}

/// Starts an emulator in the background; returns once it is launched, not once
/// it has booted (that takes a while — the device appears in `adb devices`).
pub fn start_emulator(emulator: &Path, avd: &str, cold_boot: bool) -> IdeResult<String> {
    let avd = check_name(avd)?;
    let mut cmd = Command::new(emulator);
    cmd.arg("-avd").arg(avd);
    if cold_boot {
        cmd.arg("-no-snapshot-load");
    }
    // Its own process group, no pipes: the IDE is gone long before it exits.
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    ide_core::process::prepare(&mut cmd);
    let child = cmd.spawn().map_err(|e| IdeError::unavailable(format!("starting {avd}: {e}")))?;
    // Dropping the handle leaves the emulator running; only adb talks to it after this.
    Ok(format!("started “{avd}” (pid {})", child.id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_avd_names_and_skips_noise() {
        let output = "\
INFO    | Activity Manager started
pixel_7
old-tablet

WARNING: some message
nightly_api_35
";
        assert_eq!(parse_avds(output), ["pixel_7", "old-tablet", "nightly_api_35"]);
    }

    #[test]
    fn avd_names_cannot_carry_options() {
        assert_eq!(check_name(" pixel_7 ").unwrap(), "pixel_7");
        assert!(check_name("").is_err());
        assert!(check_name("-no-window").is_err());
        assert!(check_name("pixel 7").is_err());
        assert!(check_name("pixel/../x").is_err());
        assert!(check_name(&"a".repeat(65)).is_err());
    }
}
