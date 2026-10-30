//! IPC commands of `ide_host::ops::android` (the shared implementation).

use crate::state::AppState;
use ide_android::{AndroidSdk, DeployResult, Device, DeviceEntry, LogEntry, LogFilter};
use ide_core::{IdeResult, RequestId};
#[allow(unused_imports)]
pub use ide_host::ops::android::*;
use std::path::PathBuf;
use tauri::ipc::Channel;
use tauri::{State, Window};

/// The Android SDK the window's project uses, when one is installed.
#[tauri::command]
#[specta::specta]
pub async fn android_sdk(window: Window, state: State<'_, AppState>) -> IdeResult<Option<AndroidSdk>> {
    ide_host::ops::android::android_sdk(&state.host, window.label()).await
}

/// Connected devices and emulators of the SDK at `adb`.
#[tauri::command]
#[specta::specta]
pub async fn android_devices(state: State<'_, AppState>, adb: PathBuf, request: RequestId) -> IdeResult<Vec<Device>> {
    ide_host::ops::android::android_devices(&state.host, adb, request).await
}

/// `adb install [-r] <apk>`; output lines stream through `on_output`.
#[tauri::command]
#[specta::specta]
pub async fn android_install(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: Option<String>,
    apk: PathBuf,
    reinstall: bool,
    request: RequestId,
    on_output: Channel<String>,
) -> IdeResult<()> {
    ide_host::ops::android::android_install(
        &state.host,
        adb,
        serial,
        apk,
        reinstall,
        request,
        crate::commands::sender(on_output),
    )
    .await
}

/// Starts `package/activity` (its launcher activity when `activity` is empty).
#[tauri::command]
#[specta::specta]
pub async fn android_launch(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: Option<String>,
    package_name: String,
    activity: Option<String>,
    request: RequestId,
) -> IdeResult<String> {
    ide_host::ops::android::android_launch(&state.host, adb, serial, package_name, activity, request).await
}

/// Stops the app on the device and starts it again.
#[tauri::command]
#[specta::specta]
pub async fn android_restart(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: Option<String>,
    package_name: String,
    activity: Option<String>,
    request: RequestId,
) -> IdeResult<String> {
    ide_host::ops::android::android_restart(&state.host, adb, serial, package_name, activity, request).await
}

/// Forwards the running app's JDWP socket to a local port the debugger attaches to.
#[tauri::command]
#[specta::specta]
pub async fn android_jdwp_forward(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: String,
    package_name: String,
    request: RequestId,
) -> IdeResult<u16> {
    ide_host::ops::android::android_jdwp_forward(&state.host, adb, serial, package_name, request).await
}

/// Lists one directory of the device's own file system.
#[tauri::command]
#[specta::specta]
pub async fn android_ls(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: String,
    path: String,
    request: RequestId,
) -> IdeResult<Vec<DeviceEntry>> {
    ide_host::ops::android::android_ls(&state.host, adb, serial, path, request).await
}

/// Downloads one file of the device into `local`.
#[tauri::command]
#[specta::specta]
pub async fn android_pull(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: String,
    remote: String,
    local: PathBuf,
    request: RequestId,
) -> IdeResult<u64> {
    ide_host::ops::android::android_pull(&state.host, adb, serial, remote, local, request).await
}

/// Uploads the local file to the device at `remote`.
#[tauri::command]
#[specta::specta]
pub async fn android_push(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: String,
    local: PathBuf,
    remote: String,
    request: RequestId,
) -> IdeResult<()> {
    ide_host::ops::android::android_push(&state.host, adb, serial, local, remote, request).await
}

/// Streams logcat entries until the request is cancelled.
#[tauri::command]
#[specta::specta]
pub async fn android_logcat(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: Option<String>,
    filter: LogFilter,
    request: RequestId,
    on_log: Channel<LogEntry>,
) -> IdeResult<()> {
    ide_host::ops::android::android_logcat(&state.host, adb, serial, filter, request, crate::commands::sender(on_log))
        .await
}

/// The AVDs of the emulator installation at `emulator`.
#[tauri::command]
#[specta::specta]
pub async fn android_avds(state: State<'_, AppState>, emulator: PathBuf, request: RequestId) -> IdeResult<Vec<String>> {
    ide_host::ops::android::android_avds(&state.host, emulator, request).await
}

/// Launches an emulator in the background; it shows up as a device once booted.
#[tauri::command]
#[specta::specta]
pub async fn android_emulator_start(emulator: PathBuf, avd: String, cold_boot: bool) -> IdeResult<String> {
    ide_host::ops::android::android_emulator_start(emulator, avd, cold_boot).await
}

/// Asks the emulator behind `serial` to shut down.
#[tauri::command]
#[specta::specta]
pub async fn android_emulator_stop(
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: String,
    request: RequestId,
) -> IdeResult<()> {
    ide_host::ops::android::android_emulator_stop(&state.host, adb, serial, request).await
}

/// Builds the module, installs the APK on the device and starts the app.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn android_deploy(
    window: Window,
    state: State<'_, AppState>,
    adb: PathBuf,
    serial: String,
    module: String,
    variant: String,
    request: RequestId,
    on_output: Channel<String>,
) -> IdeResult<DeployResult> {
    ide_host::ops::android::android_deploy(
        &state.host,
        window.label(),
        adb,
        serial,
        module,
        variant,
        request,
        crate::commands::sender(on_output),
    )
    .await
}
