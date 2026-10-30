//! UI-facing types and the event stream.

use crate::manifest::{CommandDecl, Permission, ToolWindowAnchor};
use crate::theme::{ThemeDef, ThemeKind};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PluginSource {
    /// Installed under `<app-data>/plugins/<id>/`.
    Installed,
    /// Loaded from `<project>/.openidea/plugins/<dir>/` while that project is open.
    Dev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PluginStatus {
    /// Code loaded and activated.
    Active,
    /// No code; static contributions only.
    Static,
    Disabled,
    /// Failed to load, or trapped and was disabled for this session (see `error`).
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginLanguage {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginTheme {
    pub id: String,
    pub name: String,
    pub kind: ThemeKind,
}

/// A tool window a plugin docks in the workbench. Exactly one of `content` and
/// `command` is set: the window shows the package file, or the text the plugin's
/// command returns when the window asks for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolWindow {
    pub id: String,
    pub title: String,
    pub anchor: ToolWindowAnchor,
    /// Icon name from the IDE's icon set; the UI falls back to the plugin icon.
    pub icon: Option<String>,
    /// Text file relative to the package directory.
    pub content: Option<String>,
    /// Command of this plugin whose result the window shows.
    pub command: Option<String>,
}

/// A file preview a plugin renders through its `render-preview` export (see
/// `contributes.previews` in `plugin.toml`). Files matching the extensions and
/// path globs get a preview pane; the plugin renders their content as HTML.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreviewDecl {
    pub id: String,
    pub title: String,
    pub extensions: Vec<String>,
    pub path_globs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    pub change_notes: Option<String>,
    pub source: PluginSource,
    /// Package directory.
    pub path: ide_core::IpcPath,
    pub enabled: bool,
    pub status: PluginStatus,
    pub error: Option<String>,
    pub has_code: bool,
    pub permissions: Vec<Permission>,
    /// Commands contributed while enabled (manifest + registered at runtime).
    pub commands: Vec<CommandDecl>,
    pub languages: Vec<PluginLanguage>,
    pub themes: Vec<PluginTheme>,
    /// Tool windows the plugin docks while it is enabled.
    pub tool_windows: Vec<PluginToolWindow>,
    /// File previews the plugin renders while it is enabled.
    pub previews: Vec<PluginPreviewDecl>,
}

/// A plugin package available for installation from a local catalog
/// directory (the samples shipped with the IDE). Not installed by listing it.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    pub change_notes: Option<String>,
    /// Package directory; install with `install_from_dir(path)`.
    pub path: ide_core::IpcPath,
    pub has_code: bool,
    pub permissions: Vec<Permission>,
    pub commands: Vec<CommandDecl>,
    pub language_count: u32,
    pub theme_count: u32,
    /// Why the package cannot be installed as is (e.g. its WASM is not built).
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTheme {
    pub plugin_id: String,
    pub theme: ThemeDef,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginsSnapshot {
    pub plugins: Vec<PluginInfo>,
    pub active_theme: Option<ActiveTheme>,
}

/// Backend → UI notifications (Tauri event `ide://plugin`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginEvent {
    /// Plugins, their status or their commands changed: re-fetch `plugin_list`.
    CommandsChanged,
    #[serde(rename_all = "camelCase")]
    Notification { plugin_id: String, level: NotificationLevel, title: String, body: String },
    #[serde(rename_all = "camelCase")]
    InsertText { plugin_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    ReplaceSelection { plugin_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    ThemeChanged { theme: Option<ActiveTheme> },
}

/// Delivers events to the UI. `client` is the window that invoked the plugin
/// (`None` = broadcast, e.g. during activation).
pub trait PluginEventSink: Send + Sync {
    fn emit(&self, client: Option<&str>, event: PluginEvent);
}

impl<F: Fn(Option<&str>, PluginEvent) + Send + Sync> PluginEventSink for F {
    fn emit(&self, client: Option<&str>, event: PluginEvent) {
        self(client, event)
    }
}

pub type BufferText = Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>;

/// Where a command runs: the invoking window and its project.
#[derive(Clone, Default)]
pub struct Invocation {
    pub client: Option<String>,
    pub workspace_root: Option<PathBuf>,
    /// Unsaved editor contents win over disk for `read-file`.
    pub buffer_text: Option<BufferText>,
}

/// The IDE services a plugin may reach through the gated host functions. The
/// desktop shell supplies these; a `None` provider makes the API answer an
/// error instead of trapping, so a headless host stays usable.
#[derive(Clone, Default)]
pub struct Capabilities {
    /// Runs a program with argv in the project directory. `(root, request)`.
    pub process: Option<ProcessProvider>,
    /// Fetches an URL through the IDE's proxy settings.
    pub http: Option<HttpProvider>,
    /// Effective settings value as JSON text. `(window, key)`.
    pub settings_get: Option<SettingsGetProvider>,
    /// Stores a settings value. `(window, key, json)`.
    pub settings_set: Option<SettingsSetProvider>,
}

/// What `run-process` reaches: `(project root, request)`.
pub type ProcessProvider = Arc<dyn Fn(Option<String>, ProcessRequest) -> Result<ProcessResult, String> + Send + Sync>;
/// What `http-fetch` reaches.
pub type HttpProvider = Arc<dyn Fn(HttpRequest) -> Result<HttpResponse, String> + Send + Sync>;
/// What `settings-get` reaches: `(window, key)` → the effective value as JSON text.
pub type SettingsGetProvider = Arc<dyn Fn(Option<String>, String) -> Option<String> + Send + Sync>;
/// What `settings-set` reaches: `(window, key, json)`.
pub type SettingsSetProvider = Arc<dyn Fn(Option<String>, String, String) -> Result<(), String> + Send + Sync>;

/// `run-process` arguments (see `wit/plugin.wit`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    /// Resolved absolute working directory, or `None` for the project root.
    pub cwd: Option<String>,
    pub timeout_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessResult {
    /// -1 when the process was killed before it reported a status.
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// `http-fetch` arguments; the host enforces the URL policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// `render-preview` call input (see `wit/plugin.wit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRequest {
    /// Absolute path of the file being previewed.
    pub path: String,
    /// The path relative to the project root, with `/` separators.
    pub project_path: String,
    /// `id` of the `contributes.previews` entry that matched the file.
    pub decl: String,
    /// Current editor content, unsaved edits included.
    pub content: String,
}

/// What a plugin's `render-preview` produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewOutput {
    /// Self-contained HTML document for the sandboxed preview iframe.
    pub html: String,
    /// Suggested page background as a hex colour.
    pub background: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub content_type: Option<String>,
}

/// Per-call resource limits for plugin code.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Wasm fuel (≈ instructions) per host → plugin call.
    pub fuel: u64,
    /// Wall-clock limit per call (epoch interruption).
    pub timeout: Duration,
    /// Linear memory cap per plugin.
    pub memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 5_000_000_000, timeout: Duration::from_secs(5), memory_bytes: 256 << 20 }
    }
}
