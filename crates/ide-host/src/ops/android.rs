//! Android toolchain: SDK discovery, devices, installs, launches, logcat and the
//! device file system.
use crate::Host;
use crate::ops::Channel;
use crate::rpc::Dispatcher;
use ide_android::{Adb, AndroidSdk, DeployResult, Device, DeviceEntry, LogEntry, LogFilter};
use ide_core::{IdeError, IdeResult, RequestId};
use std::path::PathBuf;

fn join_err(e: impl std::fmt::Display) -> IdeError {
    IdeError::internal(e.to_string())
}

/// The Android SDK the window's project uses, when one is installed.
pub async fn android_sdk(host: &Host, window: &str) -> IdeResult<Option<AndroidSdk>> {
    let root = host.project(window).ok().map(|p| p.root.clone());
    tokio::task::spawn_blocking(move || ide_android::discover(root.as_deref())).await.map_err(join_err)
}

/// Connected devices and emulators of the SDK at `adb`.
pub async fn android_devices(host: &Host, adb: PathBuf, request: RequestId) -> IdeResult<Vec<Device>> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || Adb::new(adb).devices(&cancel)).await.map_err(join_err);
    host.end_request(request);
    result?
}

/// `adb install [-r] <apk>`; output lines stream through `on_output`.
pub async fn android_install(
    host: &Host,
    adb: PathBuf,
    serial: Option<String>,
    apk: PathBuf,
    reinstall: bool,
    request: RequestId,
    on_output: Channel<String>,
) -> IdeResult<()> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || {
        Adb::new(adb).install(serial.as_deref(), &apk, reinstall, &cancel, &mut |line| {
            let _ = on_output.send(line);
        })
    })
    .await
    .map_err(join_err);
    host.end_request(request);
    result?
}

/// Starts `package/activity` (its launcher activity when `activity` is empty).
pub async fn android_launch(
    host: &Host,
    adb: PathBuf,
    serial: Option<String>,
    package_name: String,
    activity: Option<String>,
    request: RequestId,
) -> IdeResult<String> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || {
        Adb::new(adb).launch(serial.as_deref(), &package_name, activity.as_deref(), &cancel)
    })
    .await
    .map_err(join_err);
    host.end_request(request);
    result?
}

/// Stops the app (`am force-stop`) and starts it again.
pub async fn android_restart(
    host: &Host,
    adb: PathBuf,
    serial: Option<String>,
    package_name: String,
    activity: Option<String>,
    request: RequestId,
) -> IdeResult<String> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || {
        let adb = Adb::new(adb);
        adb.force_stop(serial.as_deref(), &package_name, &cancel)?;
        adb.launch(serial.as_deref(), &package_name, activity.as_deref(), &cancel)
    })
    .await
    .map_err(join_err);
    host.end_request(request);
    result?
}

/// Lists one directory of the device's own file system.
pub async fn android_ls(
    host: &Host,
    adb: PathBuf,
    serial: String,
    path: String,
    request: RequestId,
) -> IdeResult<Vec<DeviceEntry>> {
    let cancel = host.begin_request(request);
    let result =
        tokio::task::spawn_blocking(move || Adb::new(adb).list_dir(&serial, &path, &cancel)).await.map_err(join_err);
    host.end_request(request);
    result?
}

/// `adb pull <remote> <local>` — the size written.
pub async fn android_pull(
    host: &Host,
    adb: PathBuf,
    serial: String,
    remote: String,
    local: PathBuf,
    request: RequestId,
) -> IdeResult<u64> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || Adb::new(adb).pull(&serial, &remote, &local, &cancel))
        .await
        .map_err(join_err);
    host.end_request(request);
    result?
}

/// `adb push <local> <remote>`.
pub async fn android_push(
    host: &Host,
    adb: PathBuf,
    serial: String,
    local: PathBuf,
    remote: String,
    request: RequestId,
) -> IdeResult<()> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || Adb::new(adb).push(&serial, &local, &remote, &cancel))
        .await
        .map_err(join_err);
    host.end_request(request);
    result?
}

/// Forwards the running app's JDWP socket to a local port for the debugger.
pub async fn android_jdwp_forward(
    host: &Host,
    adb: PathBuf,
    serial: String,
    package_name: String,
    request: RequestId,
) -> IdeResult<u16> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || Adb::new(adb).forward_jdwp(&serial, &package_name, &cancel))
        .await
        .map_err(join_err);
    host.end_request(request);
    result?
}

/// Streams logcat entries until the request is cancelled.
pub async fn android_logcat(
    host: &Host,
    adb: PathBuf,
    serial: Option<String>,
    filter: LogFilter,
    request: RequestId,
    on_log: Channel<LogEntry>,
) -> IdeResult<()> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || {
        Adb::new(adb).logcat(serial.as_deref(), &filter, &cancel, &mut |entry| {
            let _ = on_log.send(entry);
        })
    })
    .await
    .map_err(join_err);
    host.end_request(request);
    result?
}

/// Builds the module, installs the APK on the device and starts the app.
#[allow(clippy::too_many_arguments)]
pub async fn android_deploy(
    host: &Host,
    window: &str,
    adb: PathBuf,
    serial: String,
    module: String,
    variant: String,
    request: RequestId,
    on_output: Channel<String>,
) -> IdeResult<DeployResult> {
    let root = host.project(window)?.root.clone();
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || {
        ide_android::deploy(&root, &Adb::new(adb), &serial, &module, &variant, &cancel, &mut |line| {
            let _ = on_output.send(line);
        })
    })
    .await
    .map_err(join_err);
    host.end_request(request);
    result?
}

/// The AVDs of the emulator installation at `emulator`.
pub async fn android_avds(host: &Host, emulator: PathBuf, request: RequestId) -> IdeResult<Vec<String>> {
    let cancel = host.begin_request(request);
    let result =
        tokio::task::spawn_blocking(move || ide_android::list_avds(&emulator, &cancel)).await.map_err(join_err);
    host.end_request(request);
    result?
}

/// Launches an emulator in the background; it shows up as a device once booted.
pub async fn android_emulator_start(emulator: PathBuf, avd: String, cold_boot: bool) -> IdeResult<String> {
    tokio::task::spawn_blocking(move || ide_android::start_emulator(&emulator, &avd, cold_boot))
        .await
        .map_err(join_err)?
}

/// Asks the emulator behind `serial` to shut down.
pub async fn android_emulator_stop(host: &Host, adb: PathBuf, serial: String, request: RequestId) -> IdeResult<()> {
    let cancel = host.begin_request(request);
    let result = tokio::task::spawn_blocking(move || Adb::new(adb).emu_kill(&serial, &cancel)).await.map_err(join_err);
    host.end_request(request);
    result?
}

/// Commands of this module the remote server answers.
pub(crate) fn register(d: &mut Dispatcher) {
    d.add("android_sdk", |c| async move { android_sdk(&c.host, &c.window).await });
    d.add("android_avds", |c| async move { android_avds(&c.host, c.arg("emulator")?, c.arg("request")?).await });
    d.add("android_emulator_start", |c| async move {
        android_emulator_start(c.arg("emulator")?, c.arg("avd")?, c.arg("coldBoot")?).await
    });
    d.add("android_emulator_stop", |c| async move {
        android_emulator_stop(&c.host, c.arg("adb")?, c.arg("serial")?, c.arg("request")?).await
    });
    d.add("android_devices", |c| async move { android_devices(&c.host, c.arg("adb")?, c.arg("request")?).await });
    d.add("android_install", |c| async move {
        android_install(
            &c.host,
            c.arg("adb")?,
            c.arg("serial")?,
            c.arg("apk")?,
            c.arg("reinstall")?,
            c.arg("request")?,
            c.sender("onOutput")?,
        )
        .await
    });
    d.add("android_launch", |c| async move {
        android_launch(
            &c.host,
            c.arg("adb")?,
            c.arg("serial")?,
            c.arg("packageName")?,
            c.arg("activity")?,
            c.arg("request")?,
        )
        .await
    });
    d.add("android_restart", |c| async move {
        android_restart(
            &c.host,
            c.arg("adb")?,
            c.arg("serial")?,
            c.arg("packageName")?,
            c.arg("activity")?,
            c.arg("request")?,
        )
        .await
    });
    d.add("android_jdwp_forward", |c| async move {
        android_jdwp_forward(&c.host, c.arg("adb")?, c.arg("serial")?, c.arg("packageName")?, c.arg("request")?).await
    });
    d.add("android_ls", |c| async move {
        android_ls(&c.host, c.arg("adb")?, c.arg("serial")?, c.arg("path")?, c.arg("request")?).await
    });
    d.add("android_pull", |c| async move {
        android_pull(&c.host, c.arg("adb")?, c.arg("serial")?, c.arg("remote")?, c.arg("local")?, c.arg("request")?)
            .await
    });
    d.add("android_push", |c| async move {
        android_push(&c.host, c.arg("adb")?, c.arg("serial")?, c.arg("local")?, c.arg("remote")?, c.arg("request")?)
            .await
    });
    d.add("android_logcat", |c| async move {
        android_logcat(
            &c.host,
            c.arg("adb")?,
            c.arg("serial")?,
            c.arg("filter")?,
            c.arg("request")?,
            c.sender("onLog")?,
        )
        .await
    });
    d.add("android_deploy", |c| async move {
        android_deploy(
            &c.host,
            &c.window,
            c.arg("adb")?,
            c.arg("serial")?,
            c.arg("module")?,
            c.arg("variant")?,
            c.arg("request")?,
            c.sender("onOutput")?,
        )
        .await
    });
}
