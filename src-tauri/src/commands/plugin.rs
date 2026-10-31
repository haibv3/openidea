//! WASM plugins (`ide-plugin`). Each window subscribes once with
//! `plugin_subscribe(channel)` and receives [`PluginEvent`]s addressed to it
//! (insert-text requests, notifications) plus broadcasts (list/theme changes).

use crate::state::AppState;
use ide_core::{CancellationToken, IdeError, IdeResult, RequestId};
use ide_host::Host;
use ide_plugin::{
    ActiveTheme, CatalogPlugin, Invocation, Limits, PluginEvent, PluginHost, PluginInfo, PluginsSnapshot,
    PreviewOutput, PreviewRequest, RegistryPlugin, RegistryProgress,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, Window};

/// How long `plugin_list` waits for the startup load before answering with what it has.
const INITIAL_LOAD_WAIT: Duration = Duration::from_secs(5);

/// Window label → its event channel.
static SUBSCRIBERS: LazyLock<Mutex<HashMap<String, Channel<PluginEvent>>>> = LazyLock::new(Default::default);

fn deliver(client: Option<&str>, event: PluginEvent) {
    let mut subs = SUBSCRIBERS.lock();
    match client {
        Some(label) => {
            if let Some(ch) = subs.get(label)
                && ch.send(event).is_err()
            {
                subs.remove(label);
            }
        }
        None => subs.retain(|_, ch| ch.send(event.clone()).is_ok()),
    }
}

/// Creates the host over `<app-data>/plugins` and loads installed plugins in the background.
pub fn create_host(app: &AppHandle, data_dir: &Path, ide: &Arc<Host>) -> IdeResult<Arc<PluginHost>> {
    let capabilities = crate::plugin_caps::build(app, ide);
    let host = Arc::new(PluginHost::new(data_dir.join("plugins"), Arc::new(deliver), capabilities, Limits::default())?);
    let loader = host.clone();
    std::thread::Builder::new().name("plugin-load".into()).spawn(move || loader.load_all())?;
    Ok(host)
}

fn join_err(e: impl std::fmt::Display) -> IdeError {
    IdeError::internal(e.to_string())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> IdeResult<T> + Send + 'static) -> IdeResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(join_err)?
}

/// Routes this window's plugin events to `on_event` (replaces an earlier subscription).
#[tauri::command]
#[specta::specta]
pub fn plugin_subscribe(window: Window, on_event: Channel<PluginEvent>) {
    SUBSCRIBERS.lock().insert(window.label().to_owned(), on_event);
}

/// Installed plugins plus the dev plugins of this window's project
/// (`<project>/.openidea/plugins/*`, rescanned on every call).
#[tauri::command]
#[specta::specta]
pub async fn plugin_list(window: Window, state: State<'_, AppState>) -> IdeResult<PluginsSnapshot> {
    let host = state.plugins.clone();
    let root = state.project(window.label()).ok().map(|p| p.root.clone());
    blocking(move || {
        // A window opened at startup would otherwise see no plugins and drop the selected theme.
        host.wait_initial_load(INITIAL_LOAD_WAIT);
        if let Some(root) = &root {
            host.sync_project(root);
        }
        Ok(host.list(root.as_deref()))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn plugin_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> IdeResult<()> {
    let host = state.plugins.clone();
    blocking(move || host.set_enabled(&id, enabled)).await
}

/// Installs the plugin package in `path` (a folder with `plugin.toml`).
#[tauri::command]
#[specta::specta]
pub async fn plugin_install_dir(state: State<'_, AppState>, path: PathBuf) -> IdeResult<PluginInfo> {
    let host = state.plugins.clone();
    blocking(move || host.install_from_dir(&path)).await
}

/// Plugin packages available locally for installation: `<resources>/plugins`
/// when bundled, plus the repository's sample `plugins/` in debug builds.
/// Published packages come from [`plugin_registry_index`].
#[tauri::command]
#[specta::specta]
pub async fn plugin_catalog(app: AppHandle) -> IdeResult<Vec<CatalogPlugin>> {
    let mut dirs: Vec<PathBuf> = app.path().resource_dir().map(|d| vec![d.join("plugins")]).unwrap_or_default();
    if cfg!(debug_assertions) {
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins"));
    }
    blocking(move || Ok(ide_plugin::scan_catalog(&dirs))).await
}

#[tauri::command]
#[specta::specta]
pub async fn plugin_uninstall(state: State<'_, AppState>, id: String) -> IdeResult<()> {
    let host = state.plugins.clone();
    blocking(move || host.uninstall(&id)).await
}

/// Re-reads a plugin from disk (after rebuilding a dev plugin).
#[tauri::command]
#[specta::specta]
pub async fn plugin_reload(state: State<'_, AppState>, id: String) -> IdeResult<PluginInfo> {
    let host = state.plugins.clone();
    blocking(move || host.reload(&id)).await
}

/// Runs a plugin command in the context of this window's project.
#[tauri::command]
#[specta::specta]
pub async fn plugin_run_command(
    window: Window,
    state: State<'_, AppState>,
    plugin_id: String,
    command_id: String,
    arg: Option<String>,
) -> IdeResult<Option<String>> {
    let host = state.plugins.clone();
    let project = state.project(window.label()).ok();
    let invocation = Invocation {
        client: Some(window.label().to_owned()),
        workspace_root: project.as_ref().map(|p| p.root.clone()),
        buffer_text: project.map(|p| {
            let documents = p.documents.clone();
            Arc::new(move |path: &Path| documents.text_of(path)) as ide_plugin::BufferText
        }),
    };
    blocking(move || host.run_command(&plugin_id, &command_id, arg.as_deref(), invocation)).await
}

/// What a plugin's `render-preview` produced (the UI knows the plugin and the
/// preview title from the plugin list).
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreviewRendering {
    pub plugin_id: String,
    pub html: String,
    /// The built-in server's URL for `html`: served over http it has its own
    /// CSP, so a plugin's preview scripts run (a `srcDoc` iframe inherits the
    /// app's `script-src 'self'`, which blocks them). `None` when the server
    /// could not start — the UI falls back to `srcDoc`.
    pub url: Option<String>,
    pub background: Option<String>,
}

/// The preview request paths: the absolute file path and its project-relative
/// form with `/` separators (a file outside the project falls back to its name).
fn preview_paths(root: Option<&Path>, path: &str) -> (String, String) {
    let name = || Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let relative = root
        .and_then(|root| Path::new(path).strip_prefix(root).ok())
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .filter(|rel| !rel.is_empty())
        .unwrap_or_else(name);
    (path.to_owned(), relative)
}

/// Renders a file preview with a plugin's `render-preview` export, in the
/// context of this window's project. The `content` is the editor's current text.
#[tauri::command]
#[specta::specta]
pub async fn plugin_render_preview(
    window: Window,
    state: State<'_, AppState>,
    plugin_id: String,
    decl_id: String,
    path: String,
    content: String,
) -> IdeResult<PluginPreviewRendering> {
    let host = state.plugins.clone();
    let project = state.project(window.label()).ok();
    let invocation = Invocation {
        client: Some(window.label().to_owned()),
        workspace_root: project.as_ref().map(|p| p.root.clone()),
        buffer_text: project.clone().map(|p| {
            let documents = p.documents.clone();
            Arc::new(move |path: &Path| documents.text_of(path)) as ide_plugin::BufferText
        }),
    };
    let root = project.as_ref().map(|p| p.root.clone());
    let (plugin_id, html, background) = blocking(move || {
        let (absolute, relative) = preview_paths(root.as_deref(), &path);
        let request = PreviewRequest { path: absolute, project_path: relative, decl: decl_id, content };
        let PreviewOutput { html, background } = host.render_preview(&plugin_id, request, invocation)?;
        Ok((plugin_id, html, background))
    })
    .await?;
    // Served over http the page carries no CSP of ours, so a plugin's preview
    // scripts run (a srcDoc iframe inherits `script-src 'self'` and dies).
    let url = state.web.serve_page(&html).await.ok();
    Ok(PluginPreviewRendering { plugin_id, html, url, background })
}

/// The text a plugin's tool window shows: its package `content` file.
#[tauri::command]
#[specta::specta]
pub async fn plugin_tool_window_content(
    state: State<'_, AppState>,
    plugin_id: String,
    window_id: String,
) -> IdeResult<String> {
    let host = state.plugins.clone();
    blocking(move || host.tool_window_content(&plugin_id, &window_id)).await
}

/// The UI reports the focused editor's file (`active-file` for plugins).
#[tauri::command]
#[specta::specta]
pub fn plugin_set_active_file(window: Window, state: State<'_, AppState>, path: Option<String>) {
    state.plugins.set_active_file(window.label(), path);
}

/// Selects a plugin theme; both `None` restores the built-in theme.
#[tauri::command]
#[specta::specta]
pub fn plugin_set_theme(
    state: State<'_, AppState>,
    plugin_id: Option<String>,
    theme_id: Option<String>,
) -> IdeResult<Option<ActiveTheme>> {
    match (plugin_id.as_deref(), theme_id.as_deref()) {
        (Some(p), Some(t)) => state.plugins.set_theme(Some((p, t))),
        _ => state.plugins.set_theme(None),
    }
}

/// The UI reports the selected plugin theme's resolved frame colour (`#rrggbb[aa]`),
/// which new windows open in before their page loads.
#[tauri::command]
#[specta::specta]
pub fn plugin_set_theme_frame(state: State<'_, AppState>, color: Option<String>) -> IdeResult<()> {
    state.plugins.set_theme_frame(color.as_deref())
}

fn network_err(e: impl std::fmt::Display) -> IdeError {
    IdeError::unavailable(format!("plugin registry: {e}"))
}

async fn fetch_index(url: &str) -> IdeResult<Vec<RegistryPlugin>> {
    let client = ide_http::client::shared().map_err(network_err)?;
    let text = client
        .get(url)
        .send()
        .await
        .map_err(network_err)?
        .error_for_status()
        .map_err(network_err)?
        .text()
        .await
        .map_err(network_err)?;
    ide_plugin::parse_index(&text)
}

/// Reads a response in chunks, reporting percent progress and honouring `token`.
async fn download(url: &str, token: &CancellationToken, on_progress: &Channel<RegistryProgress>) -> IdeResult<Vec<u8>> {
    let client = ide_http::client::shared().map_err(network_err)?;
    let mut response = client.get(url).send().await.map_err(network_err)?.error_for_status().map_err(network_err)?;
    let total = response.content_length();
    let mut bytes: Vec<u8> = Vec::new();
    let mut next_step = 0u64;
    loop {
        if token.is_cancelled() {
            return Err(IdeError::cancelled());
        }
        let Some(chunk) = response.chunk().await.map_err(network_err)? else { break };
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > ide_plugin::MAX_PACKAGE_BYTES {
            return Err(IdeError::invalid("the plugin package is too large"));
        }
        let downloaded = bytes.len() as u64;
        let step = match total {
            Some(total) if total > 0 => downloaded * 100 / total,
            _ => downloaded / (256 * 1024),
        };
        if step >= next_step {
            next_step = step + 1;
            let _ = on_progress.send(RegistryProgress::Downloading {
                downloaded: downloaded as f64,
                total: total.map(|total| total as f64),
            });
        }
    }
    Ok(bytes)
}

/// Packages published by the registry at `url` (`plugins.registryUrl`).
#[tauri::command]
#[specta::specta]
pub async fn plugin_registry_index(url: String) -> IdeResult<Vec<RegistryPlugin>> {
    ide_plugin::package_url_ok(&url)?;
    let plugins = fetch_index(&url).await?;
    Ok(plugins)
}

/// Downloads `plugin_id` from the registry at `url`, verifies its checksum and
/// installs it; progress streams through `on_progress`, cancel through `request`.
#[tauri::command]
#[specta::specta]
pub async fn plugin_registry_install(
    state: State<'_, AppState>,
    url: String,
    plugin_id: String,
    request: RequestId,
    on_progress: Channel<RegistryProgress>,
) -> IdeResult<PluginInfo> {
    ide_plugin::package_url_ok(&url)?;
    let token = state.host.begin_request(request);
    let host = state.plugins.clone();
    let scratch = state.host.data_dir.join("plugin-download");
    let result = async {
        let entry = fetch_index(&url)
            .await?
            .into_iter()
            .find(|plugin| plugin.id == plugin_id)
            .ok_or_else(|| IdeError::not_found(format!("{plugin_id} is not in the registry")))?;
        ide_plugin::package_url_ok(&entry.url)?;
        let bytes = download(&entry.url, &token, &on_progress).await?;
        if let Some(sha256) = entry.sha256.as_deref()
            && !ide_plugin::verify_sha256(&bytes, sha256)
        {
            return Err(IdeError::invalid("the package does not match its published checksum"));
        }
        let _ = on_progress.send(RegistryProgress::Installing);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        let dir = scratch.join(format!("{plugin_id}-{stamp}"));
        blocking(move || {
            let extracted = ide_plugin::extract_package(&bytes, &dir)?;
            let installed = host.install_from_dir(&extracted);
            let _ = std::fs::remove_dir_all(&dir);
            installed
        })
        .await
    }
    .await;
    state.host.end_request(request);
    result
}
