//! The IDE services behind the plugin host's API 0.2 imports: running programs,
//! HTTP requests and settings.
//!
//! The sandbox has already checked the plugin's permissions and kept paths
//! inside the project before a provider here is called; providers only refuse
//! what the shell itself cannot do (no project, no runtime, oversized body).

use crate::events::TauriSink;
use crate::state::AppState;
use ide_core::{EventSink, IdeEvent, output_encoding::decode_tool_output};
use ide_host::Host;
use ide_host::ops::project;
use ide_plugin::{Capabilities, HttpRequest, HttpResponse, ProcessRequest, ProcessResult};
use ide_workspace::settings::SettingsScope;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tauri::AppHandle;

/// Timeout a `run-process` call gets when it asks for none.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest a plugin may keep a process running.
const MAX_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest a plugin's HTTP request may take.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes read from a child process before its output is cut short.
const MAX_OUTPUT: usize = 4 << 20;
/// Bytes of a response body kept for the plugin.
const MAX_BODY: usize = 8 << 20;

/// One worker thread is enough: plugin calls are serialized per plugin by the
/// sandbox, and these runs are short.
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("plugin-caps")
        .enable_all()
        .build()
        .expect("plugin capability runtime")
});

/// Builds the providers for a plugin host.
pub fn build(app: &AppHandle, host: &Arc<Host>) -> Capabilities {
    let read_host = host.clone();
    let write_host = host.clone();
    let write_app = app.clone();
    Capabilities {
        process: Some(Arc::new(run_process)),
        http: Some(Arc::new(fetch)),
        settings_get: Some(Arc::new(move |client, key| read_setting(&read_host, client.as_deref(), &key))),
        settings_set: Some(Arc::new(move |client, key, json| {
            write_setting(&write_app, &write_host, client.as_deref().unwrap_or(""), &key, &json)
        })),
    }
}

fn cap(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n… output truncated");
    }
    text
}

/// Runs `req` in the project (or its `cwd`) and captures its output. No shell:
/// argv reaches the program exactly as the plugin passed it.
fn run_process(root: Option<String>, req: ProcessRequest) -> Result<ProcessResult, String> {
    let dir = req.cwd.clone().or(root).ok_or("no project is open")?;
    if !Path::new(&dir).is_dir() {
        return Err(format!("{dir}: no such directory"));
    }
    let timeout = req.timeout_ms.map_or(DEFAULT_TIMEOUT, |ms| Duration::from_millis(u64::from(ms))).min(MAX_TIMEOUT);
    let mut cmd = tokio::process::Command::new(&req.program);
    ide_core::process::prepare(cmd.as_std_mut());
    cmd.args(&req.args).current_dir(&dir).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let label = req.program.clone();
    RUNTIME.block_on(async move {
        let child = cmd.spawn().map_err(|e| format!("{label}: {e}"))?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => Ok(ProcessResult {
                exit_code: out.status.code().unwrap_or(-1),
                stdout: cap(decode_tool_output(&out.stdout), MAX_OUTPUT),
                stderr: cap(decode_tool_output(&out.stderr), MAX_OUTPUT),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(format!("{label}: {e}")),
            // Dropping the future kills the child (kill_on_drop); its partial
            // output is lost, so the plugin only learns that it ran too long.
            Err(_) => Ok(ProcessResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("{label}: killed after {} ms", timeout.as_millis()),
                timed_out: true,
            }),
        }
    })
}

/// Sends `req` through the IDE's proxy settings.
fn fetch(req: HttpRequest) -> Result<HttpResponse, String> {
    let client = ide_http::client::shared().map_err(|e| e.message)?;
    let url = req.url.clone();
    let mut builder = match req.method.as_deref().unwrap_or("GET") {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        "HEAD" => client.head(&url),
        other => return Err(format!("unsupported HTTP method {other:?}")),
    };
    for (name, value) in &req.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if let Some(body) = &req.body {
        builder = builder.body(body.clone());
    }
    RUNTIME.block_on(async move {
        let response = tokio::time::timeout(HTTP_TIMEOUT, builder.send())
            .await
            .map_err(|_| "the request timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let bytes = tokio::time::timeout(HTTP_TIMEOUT, response.bytes())
            .await
            .map_err(|_| "reading the response timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        let bytes = &bytes[..bytes.len().min(MAX_BODY)];
        // Binary bodies are not shown as text; the plugin gets its status and type.
        Ok(HttpResponse {
            status,
            body: if std::str::from_utf8(bytes).is_ok() {
                cap(String::from_utf8_lossy(bytes).into_owned(), MAX_BODY)
            } else {
                String::new()
            },
            content_type,
        })
    })
}

/// The effective value of a dotted settings key, as JSON text.
fn read_setting(host: &Host, window: Option<&str>, key: &str) -> Option<String> {
    let settings = serde_json::to_value(project::settings(host, window.unwrap_or(""))).ok()?;
    key.split('.').try_fold(&settings, |value, part| value.get(part)).map(ToString::to_string)
}

fn write_setting(app: &AppHandle, host: &Host, window: &str, key: &str, json: &str) -> Result<(), String> {
    let scoped = host.project(window).is_ok_and(|p| !p.light);
    let scope = if scoped { SettingsScope::Project } else { SettingsScope::Global };
    let applied = project::set_settings(host, window, scope, key, json).map_err(|e| e.message)?;
    // Tell the windows, exactly as Settings › apply does, so the change is visible.
    let event = IdeEvent::SettingsChanged { key: key.to_owned() };
    if applied.project_scope {
        let owner = AppState::owner_window(window);
        for label in [owner.to_owned(), crate::windows::settings_label(owner)] {
            let sink = TauriSink { app: app.clone(), window: Some(label) };
            sink.emit(event.clone());
            if let Some(root) = &applied.roots_changed {
                sink.emit(IdeEvent::ProjectRootsChanged { root: root.clone() });
            }
        }
    } else {
        TauriSink { app: app.clone(), window: None }.emit(event);
    }
    Ok(())
}
