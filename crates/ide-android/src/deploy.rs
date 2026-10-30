//! Building an Android module and running it on a device: `assemble`, `adb install`, `am start`.

use crate::project::{assemble_task, find_apk, modules};
use crate::{Adb, process};
use ide_core::{CancellationToken, IdeError, IdeResult, IpcPath};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `deploy` built and installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeployResult {
    /// Project-relative directory of the built module.
    pub module: String,
    pub variant: String,
    pub apk: IpcPath,
    /// `applicationId` started on the device, when the module declares one.
    pub application_id: Option<String>,
}

/// Builds `variant` of the module (the first application module when `module` is
/// empty), installs the APK on `serial` and starts the app. Output lines stream
/// through `on_line`; cancelling kills the build / install.
pub fn deploy(
    project_root: &Path,
    adb: &Adb,
    serial: &str,
    module: &str,
    variant: &str,
    cancel: &CancellationToken,
    on_line: &mut dyn FnMut(String),
) -> IdeResult<DeployResult> {
    let app_modules = modules(project_root);
    let selected = app_modules.iter().find(|candidate| candidate.dir == module).or_else(|| app_modules.first());
    let module_dir = selected.map(|candidate| candidate.dir.clone()).unwrap_or_else(|| module.to_owned());
    let task = assemble_task(&module_dir, variant);
    on_line(format!("> {task}"));

    let mut cmd = gradle(project_root, &task);
    cmd.current_dir(project_root);
    let (ok, tail) = process::stream(cmd, cancel, &mut *on_line)?;
    if !ok {
        return Err(IdeError::unavailable(format!("{task} failed: {}", tail.join("\n"))));
    }

    let build_dir = if module_dir.is_empty() { project_root.to_path_buf() } else { project_root.join(&module_dir) };
    let apk = find_apk(&build_dir, variant)
        .ok_or_else(|| IdeError::unavailable(format!("{task} produced no {variant} APK in {}", build_dir.display())))?;
    on_line(format!("Installing {}", apk.display()));
    adb.install(Some(serial), &apk, true, cancel, &mut *on_line)?;

    let application_id = selected.and_then(|candidate| candidate.application_id.clone());
    if let Some(id) = application_id.as_deref() {
        let output = adb.launch(Some(serial), id, None, cancel)?;
        for line in output.lines().filter(|line| !line.trim().is_empty()) {
            on_line(line.to_owned());
        }
    }

    Ok(DeployResult { module: module_dir, variant: variant.to_owned(), apk: apk.into(), application_id })
}

/// The project's Gradle wrapper, else `gradle` from the search path.
fn gradle(project_root: &Path, task: &str) -> Command {
    let wrapper = project_root.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" });
    let program =
        if wrapper.is_file() { wrapper } else { PathBuf::from(if cfg!(windows) { "gradle.bat" } else { "gradle" }) };
    #[cfg(windows)]
    {
        // A `.bat` cannot be started directly without `cmd`.
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(program).arg(task).arg("--console=plain");
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(program);
        cmd.arg(task).arg("--console=plain");
        cmd
    }
}
