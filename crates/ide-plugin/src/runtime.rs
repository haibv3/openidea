//! Wasmtime glue: one shared engine, one `Store` per plugin, host imports.

use crate::manifest::{API_VERSION, CommandDecl, Permission, ToolWindowDecl, validate_command_id};
use crate::sandbox;
use crate::theme::ThemeDef;
use crate::types::{
    Capabilities, HttpRequest, Invocation, Limits, NotificationLevel, PluginEvent, PluginEventSink, PreviewOutput,
    PreviewRequest, ProcessRequest,
};
use ide_language::LanguageConfig;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable, Val};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({
        world: "plugin",
        path: "../../wit",
        imports: { default: trappable },
    });
}

use bindings::openidea::plugin::host::{self as wit_host};
use bindings::openidea::plugin::types::{self as wit_types, LogLevel};

/// client → path of its focused editor.
pub(crate) type ActiveFileFn = Arc<dyn Fn(Option<&str>) -> Option<String> + Send + Sync>;

const EPOCH_TICK: Duration = Duration::from_millis(10);
const MAX_TEXT: usize = 1 << 20;
const MAX_TITLE: usize = 256;
const MAX_COMMANDS: usize = 256;
const STDERR_CAPACITY: usize = 256 << 10;
/// argv length accepted by `run-process`.
const MAX_ARGS: usize = 32;
const MAX_PROGRAM: usize = 4096;
const DEFAULT_TIMEOUT_MS: u32 = 20_000;
const MAX_TIMEOUT_MS: u32 = 60_000;
/// Each stream `run-process` returns.
const MAX_PROCESS_OUTPUT: usize = 512 << 10;
const MAX_HTTP_BODY: usize = 4 << 20;
const MAX_HEADERS: usize = 32;
const MAX_SETTINGS_KEY: usize = 256;
/// Longest file name `list-dir` hands back.
const MAX_PATH: usize = 4096;
/// Largest HTML document `render-preview` may return.
const MAX_PREVIEW_HTML: usize = 8 << 20;

/// Shared compilation engine plus an epoch ticker thread for wall-clock limits.
pub(crate) struct PluginEngine {
    engine: Engine,
    linker: Linker<StoreData>,
}

impl PluginEngine {
    pub fn new() -> Result<Self, String> {
        let mut config = Config::new();
        config.wasm_component_model(true).consume_fuel(true).epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| format!("wasm engine: {e}"))?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
        bindings::Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s).map_err(|e| e.to_string())?;

        let weak = engine.weak();
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || {
                while let Some(engine) = weak.upgrade() {
                    engine.increment_epoch();
                    drop(engine);
                    std::thread::sleep(EPOCH_TICK);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { engine, linker })
    }

    pub fn compile(&self, wasm: &Path) -> Result<Component, String> {
        let bytes = std::fs::read(wasm).map_err(|e| format!("{}: {e}", wasm.display()))?;
        Component::new(&self.engine, &bytes)
            .map_err(|e| format!("{}: not a valid plugin component: {e}", wasm.display()))
    }
}

/// What a plugin contributed (manifest files + runtime registrations).
#[derive(Default, Clone)]
pub(crate) struct Registrations {
    pub commands: Vec<CommandDecl>,
    pub languages: Vec<LanguageConfig>,
    pub themes: Vec<ThemeDef>,
    /// Declared in the manifest; a plugin cannot add windows at runtime (0.2).
    pub tool_windows: Vec<ToolWindowDecl>,
}

impl Registrations {
    pub fn add_tool_window(&mut self, window: ToolWindowDecl) {
        match self.tool_windows.iter_mut().find(|w| w.id == window.id) {
            Some(existing) => *existing = window,
            None => self.tool_windows.push(window),
        }
    }

    pub fn add_command(&mut self, cmd: CommandDecl) -> bool {
        match self.commands.iter_mut().find(|c| c.id == cmd.id) {
            Some(existing) if *existing == cmd => false,
            Some(existing) => {
                *existing = cmd;
                true
            }
            None => {
                self.commands.push(cmd);
                true
            }
        }
    }

    pub fn add_language(&mut self, cfg: LanguageConfig) {
        self.languages.retain(|l| l.id != cfg.id);
        self.languages.push(cfg);
    }

    pub fn add_theme(&mut self, theme: ThemeDef) {
        self.themes.retain(|t| t.id != theme.id);
        self.themes.push(theme);
    }
}

pub(crate) fn parse_language(src: &str) -> Result<LanguageConfig, String> {
    let cfg: LanguageConfig = toml::from_str(src).map_err(|e| format!("invalid language definition: {e}"))?;
    if cfg.id.as_str().is_empty() || cfg.name.trim().is_empty() {
        return Err("language `id` and `name` must not be empty".into());
    }
    Ok(cfg)
}

/// Side effects of one call the host must act on afterwards.
#[derive(Default)]
pub(crate) struct CallEffects {
    pub commands_changed: bool,
    pub themes_changed: Vec<String>,
}

pub(crate) struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    stderr: MemoryOutputPipe,
    stderr_seen: usize,
    plugin_id: String,
    permissions: Vec<Permission>,
    capabilities: Capabilities,
    registrations: Arc<Mutex<Registrations>>,
    sink: Arc<dyn PluginEventSink>,
    invocation: Invocation,
    active_file: ActiveFileFn,
    effects: CallEffects,
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl StoreData {
    fn require(&self, p: Permission) -> wasmtime::Result<()> {
        if self.permissions.contains(&p) {
            Ok(())
        } else {
            Err(wasmtime::Error::msg(format!(
                "permission denied: add \"{}\" to `permissions` in plugin.toml",
                p.as_str()
            )))
        }
    }

    fn emit(&self, event: PluginEvent) {
        self.sink.emit(self.invocation.client.as_deref(), event);
    }

    /// New guest stderr output since the last call (panic messages, eprintln!).
    fn drain_stderr(&mut self) -> String {
        let all = self.stderr.contents();
        let new = String::from_utf8_lossy(&all[self.stderr_seen.min(all.len())..]).into_owned();
        self.stderr_seen = all.len();
        new
    }

    /// The project the current command runs in, for the file-system imports.
    fn project_root(&self) -> Result<std::path::PathBuf, String> {
        self.invocation.workspace_root.clone().ok_or_else(|| "no project is open".into())
    }

    /// Applies the `run-process` policy: a bare program name is resolved on the
    /// IDE's search path, anything with a separator has to be inside the
    /// project, and so has the working directory.
    fn plan_process(&self, req: &wit_types::ProcessRequest) -> Result<ProcessRequest, String> {
        let program = req.program.trim();
        if program.is_empty() {
            return Err("program must not be empty".into());
        }
        if program.len() > MAX_PROGRAM {
            return Err(format!("program is longer than {MAX_PROGRAM} characters"));
        }
        if req.args.len() > MAX_ARGS {
            return Err(format!("too many arguments (max {MAX_ARGS})"));
        }
        for arg in &req.args {
            if arg.len() > MAX_PROGRAM {
                return Err(format!("an argument is longer than {MAX_PROGRAM} characters"));
            }
        }
        let bare = !program.contains('/') && !program.contains('\\') && !std::path::Path::new(program).is_absolute();
        let program = if bare {
            program.to_owned()
        } else {
            let real = sandbox::resolve_in_root(&self.project_root()?, program)?;
            real.to_string_lossy().into_owned()
        };
        let cwd = match &req.cwd {
            Some(cwd) => {
                let real = sandbox::resolve_in_root(&self.project_root()?, cwd)?;
                Some(real.to_string_lossy().into_owned())
            }
            None => None,
        };
        let timeout_ms = req.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).clamp(1, MAX_TIMEOUT_MS);
        Ok(ProcessRequest { program, args: req.args.clone(), cwd, timeout_ms: Some(timeout_ms) })
    }
}

fn clip(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

impl wit_types::Host for StoreData {}

impl wit_host::Host for StoreData {
    fn log(&mut self, level: LogLevel, message: String) -> wasmtime::Result<()> {
        let id = &self.plugin_id;
        let message = clip(message, MAX_TEXT);
        match level {
            LogLevel::Trace => tracing::trace!(plugin = %id, "{message}"),
            LogLevel::Debug => tracing::debug!(plugin = %id, "{message}"),
            LogLevel::Info => tracing::info!(plugin = %id, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin = %id, "{message}"),
            LogLevel::Error => tracing::error!(plugin = %id, "{message}"),
        }
        Ok(())
    }

    fn register_command(&mut self, id: String, title: String, category: Option<String>) -> wasmtime::Result<()> {
        validate_command_id(&id).map_err(wasmtime::Error::msg)?;
        let title = clip(title, MAX_TITLE);
        if title.trim().is_empty() {
            return Err(wasmtime::Error::msg(format!("command {id:?} has an empty title")));
        }
        let cmd = CommandDecl { id, title, category: category.map(|c| clip(c, MAX_TITLE)).filter(|c| !c.is_empty()) };
        let mut regs = self.registrations.lock();
        if regs.commands.len() >= MAX_COMMANDS && !regs.commands.iter().any(|c| c.id == cmd.id) {
            return Err(wasmtime::Error::msg(format!("too many commands (max {MAX_COMMANDS})")));
        }
        if regs.add_command(cmd) {
            self.effects.commands_changed = true;
        }
        Ok(())
    }

    fn show_notification(
        &mut self,
        level: wit_types::NotificationLevel,
        title: String,
        body: String,
    ) -> wasmtime::Result<()> {
        let level = match level {
            wit_types::NotificationLevel::Info => NotificationLevel::Info,
            wit_types::NotificationLevel::Warning => NotificationLevel::Warning,
            wit_types::NotificationLevel::Error => NotificationLevel::Error,
        };
        self.emit(PluginEvent::Notification {
            plugin_id: self.plugin_id.clone(),
            level,
            title: clip(title, MAX_TITLE),
            body: clip(body, 16 << 10),
        });
        Ok(())
    }

    fn workspace_root(&mut self) -> wasmtime::Result<Option<String>> {
        self.require(Permission::ProjectRead)?;
        Ok(self.invocation.workspace_root.as_ref().map(|p| p.to_string_lossy().into_owned()))
    }

    fn active_file(&mut self) -> wasmtime::Result<Option<String>> {
        self.require(Permission::EditorRead)?;
        Ok((self.active_file)(self.invocation.client.as_deref()))
    }

    fn read_file(&mut self, path: String) -> wasmtime::Result<Result<String, String>> {
        self.require(Permission::ProjectRead)?;
        let Some(root) = self.invocation.workspace_root.clone() else {
            return Ok(Err("no project is open".into()));
        };
        let real = match sandbox::resolve_in_root(&root, &path) {
            Ok(real) => real,
            Err(e) => return Ok(Err(e)),
        };
        if let Some(text) = self.invocation.buffer_text.as_ref().and_then(|f| f(&real)) {
            return Ok(Ok(text));
        }
        Ok(sandbox::read_text(&real))
    }


    fn read_file_bytes(&mut self, path: String) -> wasmtime::Result<Result<Vec<u8>, String>> {
        self.require(Permission::ProjectRead)?;
        let Some(root) = self.invocation.workspace_root.clone() else {
            return Ok(Err("no project is open".into()));
        };
        let real = match sandbox::resolve_in_root(&root, &path) {
            Ok(real) => real,
            Err(e) => return Ok(Err(e)),
        };
        Ok(sandbox::read_bytes(&real))
    }
    fn insert_text(&mut self, text: String) -> wasmtime::Result<()> {
        self.require(Permission::EditorWrite)?;
        self.emit(PluginEvent::InsertText { plugin_id: self.plugin_id.clone(), text: clip(text, MAX_TEXT) });
        Ok(())
    }

    fn replace_selection(&mut self, text: String) -> wasmtime::Result<()> {
        self.require(Permission::EditorWrite)?;
        self.emit(PluginEvent::ReplaceSelection { plugin_id: self.plugin_id.clone(), text: clip(text, MAX_TEXT) });
        Ok(())
    }

    fn register_language(&mut self, definition: String) -> wasmtime::Result<Result<(), String>> {
        Ok(parse_language(&definition).map(|cfg| {
            self.registrations.lock().add_language(cfg);
            self.effects.commands_changed = true;
        }))
    }

    fn register_theme(&mut self, definition: String) -> wasmtime::Result<Result<(), String>> {
        Ok(ThemeDef::parse(&definition).map(|theme| {
            self.effects.themes_changed.push(theme.id.clone());
            self.registrations.lock().add_theme(theme);
            self.effects.commands_changed = true;
        }))
    }

    fn list_dir(&mut self, path: String) -> wasmtime::Result<Result<Vec<wit_types::DirEntry>, String>> {
        self.require(Permission::ProjectRead)?;
        Ok(self.read_dir(&path))
    }

    fn write_file(&mut self, path: String, text: String) -> wasmtime::Result<Result<(), String>> {
        self.require(Permission::ProjectWrite)?;
        Ok(self.write(&path, &text))
    }

    fn create_dir(&mut self, path: String) -> wasmtime::Result<Result<(), String>> {
        self.require(Permission::ProjectWrite)?;
        Ok(self.make_dir(&path))
    }

    fn delete_file(&mut self, path: String) -> wasmtime::Result<Result<(), String>> {
        self.require(Permission::ProjectWrite)?;
        Ok(self.remove(&path))
    }

    fn run_process(
        &mut self,
        request: wit_types::ProcessRequest,
    ) -> wasmtime::Result<Result<wit_types::ProcessResult, String>> {
        self.require(Permission::Process)?;
        Ok(self.execute(request))
    }

    fn http_fetch(
        &mut self,
        request: wit_types::HttpRequest,
    ) -> wasmtime::Result<Result<wit_types::HttpResponse, String>> {
        self.require(Permission::Net)?;
        Ok(self.fetch(request))
    }

    fn settings_get(&mut self, key: String) -> wasmtime::Result<Result<Option<String>, String>> {
        self.require(Permission::SettingsRead)?;
        Ok(self.read_setting(key))
    }

    fn settings_set(&mut self, key: String, json: String) -> wasmtime::Result<Result<(), String>> {
        self.require(Permission::SettingsWrite)?;
        Ok(self.write_setting(key, json))
    }
}

const HTTP_METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// Errors from these host calls are reported to the plugin as `result::error`
/// (it stays usable); only a missing permission traps.
impl StoreData {
    fn read_dir(&self, path: &str) -> Result<Vec<wit_types::DirEntry>, String> {
        let real = sandbox::resolve_in_root(&self.project_root()?, path)?;
        Ok(sandbox::list_dir(&real)?
            .into_iter()
            .map(|e| wit_types::DirEntry { name: clip(e.name, MAX_PATH), is_dir: e.is_dir, size: e.size })
            .collect())
    }

    fn write(&self, path: &str, text: &str) -> Result<(), String> {
        let real = sandbox::resolve_new_in_root(&self.project_root()?, path)?;
        sandbox::write_text(&real, text)
    }

    fn make_dir(&self, path: &str) -> Result<(), String> {
        let real = sandbox::resolve_new_in_root(&self.project_root()?, path)?;
        sandbox::create_dir(&real)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        let real = sandbox::resolve_in_root(&self.project_root()?, path)?;
        sandbox::delete_file(&real)
    }

    fn execute(&self, request: wit_types::ProcessRequest) -> Result<wit_types::ProcessResult, String> {
        let Some(provider) = self.capabilities.process.clone() else {
            return Err("this IDE build does not let plugins run programs".into());
        };
        let planned = self.plan_process(&request)?;
        let root = self.invocation.workspace_root.as_ref().map(|p| p.to_string_lossy().into_owned());
        let out = provider(root, planned)?;
        Ok(wit_types::ProcessResult {
            exit_code: out.exit_code,
            stdout: clip(out.stdout, MAX_PROCESS_OUTPUT),
            stderr: clip(out.stderr, MAX_PROCESS_OUTPUT),
            timed_out: out.timed_out,
        })
    }

    fn fetch(&self, request: wit_types::HttpRequest) -> Result<wit_types::HttpResponse, String> {
        let Some(provider) = self.capabilities.http.clone() else {
            return Err("this IDE build does not let plugins make requests".into());
        };
        let url = request.url.trim();
        crate::registry::package_url_ok(url).map_err(|e| e.to_string())?;
        let method = match request.method.as_deref().map(str::trim) {
            None | Some("") => "GET".to_owned(),
            Some(m) if HTTP_METHODS.iter().any(|allowed| m.eq_ignore_ascii_case(allowed)) => m.to_ascii_uppercase(),
            Some(m) => return Err(format!("unsupported HTTP method {m:?}")),
        };
        if request.headers.len() > MAX_HEADERS {
            return Err(format!("too many headers (max {MAX_HEADERS})"));
        }
        let body = match request.body {
            Some(b) if b.len() > MAX_HTTP_BODY => {
                return Err(format!("request body is larger than {} MiB", MAX_HTTP_BODY >> 20));
            }
            other => other,
        };
        let headers =
            request.headers.into_iter().map(|(name, value)| (clip(name, MAX_TITLE), clip(value, MAX_TEXT))).collect();
        let out = provider(HttpRequest { url: url.to_owned(), method: Some(method), headers, body })?;
        Ok(wit_types::HttpResponse {
            status: out.status,
            body: clip(out.body, MAX_HTTP_BODY),
            content_type: out.content_type.map(|c| clip(c, MAX_TITLE)),
        })
    }

    fn read_setting(&self, key: String) -> Result<Option<String>, String> {
        let Some(provider) = self.capabilities.settings_get.clone() else {
            return Err("this IDE build exposes no settings to plugins".into());
        };
        Ok(provider(self.invocation.client.clone(), settings_key(key)?))
    }

    fn write_setting(&self, key: String, json: String) -> Result<(), String> {
        let Some(provider) = self.capabilities.settings_set.clone() else {
            return Err("this IDE build exposes no settings to plugins".into());
        };
        if serde_json::from_str::<serde_json::Value>(&json).is_err() {
            return Err("the value is not valid JSON".into());
        }
        provider(self.invocation.client.clone(), settings_key(key)?, json)
    }
}

fn settings_key(key: String) -> Result<String, String> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err("settings key must not be empty".into());
    }
    if key.len() > MAX_SETTINGS_KEY {
        return Err(format!("settings key is longer than {MAX_SETTINGS_KEY} characters"));
    }
    Ok(key)
}

/// A call failed: either the plugin returned an error (it stays usable) or it
/// trapped / ran out of budget (the instance is poisoned and must be dropped).
#[derive(Debug)]
pub(crate) enum CallError {
    Guest(String),
    Trap(String),
}

pub(crate) struct RuntimeSetup {
    pub plugin_id: String,
    pub version: String,
    pub permissions: Vec<Permission>,
    pub capabilities: Capabilities,
    pub registrations: Arc<Mutex<Registrations>>,
    pub sink: Arc<dyn PluginEventSink>,
    pub active_file: ActiveFileFn,
    pub limits: Limits,
}

pub(crate) struct Runtime {
    store: Store<StoreData>,
    plugin: bindings::Plugin,
    /// The raw instance, for optional exports the `plugin` world doesn't declare.
    instance: wasmtime::component::Instance,
    /// Whether the component exports `render-preview` (probed at instantiation).
    has_preview: bool,
    limits: Limits,
    version: String,
}

impl Runtime {
    pub fn instantiate(engine: &PluginEngine, component: &Component, setup: RuntimeSetup) -> Result<Self, String> {
        let stderr = MemoryOutputPipe::new(STDERR_CAPACITY);
        // No preopened dirs, env, args or network: host imports are the only way out.
        let wasi = WasiCtx::builder().stderr(stderr.clone()).build();
        let limits = StoreLimitsBuilder::new()
            .memory_size(setup.limits.memory_bytes)
            .instances(32)
            .tables(64)
            .memories(8)
            .trap_on_grow_failure(true)
            .build();
        let data = StoreData {
            wasi,
            table: ResourceTable::new(),
            limits,
            stderr,
            stderr_seen: 0,
            plugin_id: setup.plugin_id,
            permissions: setup.permissions,
            capabilities: setup.capabilities,
            registrations: setup.registrations,
            sink: setup.sink,
            invocation: Invocation::default(),
            active_file: setup.active_file,
            effects: CallEffects::default(),
        };
        let mut store = Store::new(&engine.engine, data);
        store.limiter(|d| &mut d.limits);
        store.epoch_deadline_trap();
        arm(&mut store, &setup.limits).map_err(|e| e.to_string())?;
        // Instantiate through `InstancePre` and wrap the raw instance: the typed
        // `Plugin` bindings resolve the world's exports eagerly, while optional
        // exports such as `render-preview` are looked up on the instance below,
        // so components without them keep loading.
        let pre = engine.linker.instantiate_pre(component).map_err(|e| describe_trap(&e, &setup.limits))?;
        let instance = pre.instantiate(&mut store).map_err(|e| describe_trap(&e, &setup.limits))?;
        let plugin = bindings::Plugin::new(&mut store, &instance).map_err(|e| describe_trap(&e, &setup.limits))?;
        let has_preview = instance.get_func(&mut store, "render-preview").is_some();
        Ok(Self { store, plugin, instance, has_preview, limits: setup.limits, version: setup.version })
    }

    pub fn activate(&mut self) -> Result<CallEffects, CallError> {
        let data = self.store.data();
        let ctx = wit_types::Context {
            plugin_id: data.plugin_id.clone(),
            plugin_version: self.version.clone(),
            api_version: API_VERSION.into(),
            permissions: data.permissions.iter().map(|p| p.as_str().to_owned()).collect(),
        };
        self.call(Invocation::default(), |plugin, _instance, store| plugin.call_activate(store, &ctx))
            .and_then(|(r, fx)| r.map(|()| fx).map_err(CallError::Guest))
    }

    pub fn run_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
        invocation: Invocation,
    ) -> Result<(Option<String>, CallEffects), CallError> {
        let (result, fx) = self.call(invocation, |plugin, _instance, store| plugin.call_run_command(store, id, arg))?;
        result.map(|v| (v, fx)).map_err(CallError::Guest)
    }

    /// Calls the optional `render-preview` export (see `world plugin-preview`
    /// in `wit/plugin.wit`). The export is looked up by name and called through
    /// untyped values, so components without it keep loading.
    pub fn render_preview(
        &mut self,
        request: &PreviewRequest,
        invocation: Invocation,
    ) -> Result<(PreviewOutput, CallEffects), CallError> {
        if !self.has_preview {
            return Err(CallError::Guest("plugin has no `render-preview` export (no preview support)".into()));
        }
        let (value, fx) = self.call(invocation, |_plugin, instance, store| {
            let func = instance
                .get_func(&mut *store, "render-preview")
                .ok_or_else(|| wasmtime::Error::msg("the component has no export `render-preview`"))?;
            let request = preview_request_val(request);
            let mut results = [Val::Result(Ok(None))];
            func.call(&mut *store, &[request], &mut results)?;
            Ok(results.into_iter().next().unwrap())
        })?;
        decode_preview_output(value).map(|output| (output, fx)).map_err(CallError::Guest)
    }

    /// Best effort; errors are logged.
    pub fn deactivate(&mut self) {
        if let Err(CallError::Trap(e) | CallError::Guest(e)) =
            self.call(Invocation::default(), |plugin, _instance, store| plugin.call_deactivate(store))
        {
            tracing::warn!(plugin = %self.store.data().plugin_id, "deactivate failed: {e}");
        }
    }

    fn call<R>(
        &mut self,
        invocation: Invocation,
        f: impl FnOnce(&bindings::Plugin, &wasmtime::component::Instance, &mut Store<StoreData>) -> wasmtime::Result<R>,
    ) -> Result<(R, CallEffects), CallError> {
        arm(&mut self.store, &self.limits).map_err(|e| CallError::Trap(e.to_string()))?;
        self.store.data_mut().invocation = invocation;
        let result = f(&self.plugin, &self.instance, &mut self.store);
        let data = self.store.data_mut();
        data.invocation = Invocation::default();
        let effects = std::mem::take(&mut data.effects);
        let stderr = data.drain_stderr();
        if !stderr.trim().is_empty() {
            tracing::info!(plugin = %data.plugin_id, "stderr: {}", stderr.trim_end());
        }
        match result {
            Ok(r) => Ok((r, effects)),
            Err(e) => {
                let mut msg = describe_trap(&e, &self.limits);
                // Typically the panic message: "panicked at src/lib.rs:1:2:\nboom".
                let detail: Vec<&str> =
                    stderr.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("note:")).collect();
                if !detail.is_empty() {
                    let tail = detail[detail.len().saturating_sub(4)..].join("\n");
                    msg = format!("{msg}\n{}", clip(tail, 1024));
                }
                Err(CallError::Trap(msg))
            }
        }
    }
}

fn preview_request_val(request: &PreviewRequest) -> Val {
    Val::Record(vec![
        ("path".into(), Val::String(request.path.clone())),
        ("project-path".into(), Val::String(request.project_path.clone())),
        ("decl".into(), Val::String(request.decl.clone())),
        ("content".into(), Val::String(request.content.clone())),
    ])
}

fn decode_preview_output(value: Val) -> Result<PreviewOutput, String> {
    let payload = match value {
        Val::Result(Ok(payload)) => payload.ok_or("render-preview returned ok without a value")?,
        Val::Result(Err(err)) => {
            let err = *err.ok_or("render-preview failed without a message")?;
            return match err {
                Val::String(message) => Err(message),
                other => Err(format!("render-preview failed: {other:?}")),
            };
        }
        other => return Err(format!("render-preview returned {other:?}, expected a result")),
    };
    let Val::Record(fields) = *payload else {
        return Err(format!("render-preview returned {payload:?}, expected a preview-output record"));
    };
    let mut html = None;
    let mut background = None;
    for (name, value) in fields {
        match name.as_str() {
            "html" => match value {
                Val::String(text) => html = Some(text),
                other => return Err(format!("preview-output `html` is {other:?}, expected a string")),
            },
            "background" => match value {
                Val::Option(None) => {}
                Val::Option(Some(color)) => match *color {
                    Val::String(color) => background = Some(color),
                    other => return Err(format!("preview-output `background` is {other:?}, expected a string")),
                },
                other => return Err(format!("preview-output `background` is {other:?}, expected an option")),
            },
            _ => {}
        }
    }
    let html = html.ok_or("preview-output has no `html` field")?;
    if html.len() > MAX_PREVIEW_HTML {
        return Err(format!("preview HTML is larger than {} MiB", MAX_PREVIEW_HTML >> 20));
    }
    if let Some(color) = &background
        && !crate::is_hex_color(color)
    {
        return Err(format!("preview-output `background` is not a hex colour: {color:?}"));
    }
    Ok(PreviewOutput { html, background })
}

fn arm(store: &mut Store<StoreData>, limits: &Limits) -> wasmtime::Result<()> {
    store.set_fuel(limits.fuel)?;
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    Ok(())
}

fn describe_trap(e: &wasmtime::Error, limits: &Limits) -> String {
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::OutOfFuel) => format!("exceeded its CPU budget ({} fuel per call)", limits.fuel),
        Some(wasmtime::Trap::Interrupt) => format!("timed out after {:?}", limits.timeout),
        Some(wasmtime::Trap::UnreachableCodeReached) => "crashed (panic / unreachable)".to_owned(),
        Some(trap) => format!("trapped: {trap}"),
        None => {
            let root = e.root_cause().to_string();
            if root.contains("memory") {
                format!("{root} (memory limit {} MiB)", limits.memory_bytes >> 20)
            } else {
                root
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::wit_host::Host;
    use super::*;
    use crate::types::{HttpResponse, ProcessProvider, ProcessResult};
    use std::path::PathBuf;

    /// A `StoreData` with fake IDE services: exercises the host imports without
    /// loading a component.
    fn data(permissions: &[Permission], caps: Capabilities, root: Option<PathBuf>) -> StoreData {
        let stderr = MemoryOutputPipe::new(STDERR_CAPACITY);
        StoreData {
            wasi: WasiCtx::builder().stderr(stderr.clone()).build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new().build(),
            stderr,
            stderr_seen: 0,
            plugin_id: "test".into(),
            permissions: permissions.to_vec(),
            capabilities: caps,
            registrations: Arc::new(Mutex::new(Registrations::default())),
            sink: Arc::new(|_: Option<&str>, _| {}),
            invocation: Invocation { client: Some("main".into()), workspace_root: root, buffer_text: None },
            active_file: Arc::new(|_| None),
            effects: CallEffects::default(),
        }
    }

    fn project() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "hi").unwrap();
        (tmp, root.canonicalize().unwrap())
    }

    /// A host call the plugin is not allowed to make: traps, so the instance dies.
    macro_rules! denied {
        ($call:expr) => {
            match $call {
                Err(e) => assert!(e.to_string().contains("permission denied"), "{e}"),
                Ok(_) => panic!("expected the call to be refused"),
            }
        };
    }

    /// A host call that fails in a way the plugin can handle.
    macro_rules! refused {
        ($call:expr, $pattern:expr) => {{
            let message = $call.unwrap().unwrap_err();
            assert!(message.contains($pattern), "{message}");
        }};
    }

    #[test]
    fn ungranted_permissions_trap() {
        let (_tmp, root) = project();
        let mut d = data(&[], Capabilities::default(), Some(root));
        denied!(d.list_dir("src".into()));
        denied!(d.write_file("src/b.txt".into(), "x".into()));
        denied!(d.create_dir("src/dir".into()));
        denied!(d.delete_file("src/a.txt".into()));
        denied!(d.run_process(wit_types::ProcessRequest {
            program: "tool".into(),
            args: Vec::new(),
            cwd: None,
            timeout_ms: None,
        }));
        denied!(d.http_fetch(http("https://example.com", None)));
        denied!(d.settings_get("appearance".into()));
        denied!(d.settings_set("appearance.theme".into(), "\"harbor\"".into()));
    }

    #[test]
    fn file_host_calls_stay_in_the_project() {
        let (_tmp, root) = project();
        let mut d =
            data(&[Permission::ProjectRead, Permission::ProjectWrite], Capabilities::default(), Some(root.clone()));

        let names: Vec<String> = d.list_dir("src".into()).unwrap().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["a.txt"]);
        refused!(d.list_dir("..".into()), "outside the project root");

        d.write_file("src/gen.kt".into(), "fun main() {}".into()).unwrap().unwrap();
        assert_eq!(d.read_file("src/gen.kt".into()).unwrap().unwrap(), "fun main() {}");
        refused!(d.write_file("../escape.kt".into(), "x".into()), "outside the project root");
        assert!(!root.join("../escape.kt").exists());
        refused!(d.write_file("out/deep/f.txt".into(), "x".into()), "no such directory");

        d.create_dir("out/deep".into()).unwrap().unwrap();
        d.write_file("out/deep/f.txt".into(), "x".into()).unwrap().unwrap();
        assert!(root.join("out/deep/f.txt").is_file());
        refused!(d.delete_file("out".into()), "directory");
        d.delete_file("out/deep/f.txt".into()).unwrap().unwrap();
        assert!(!root.join("out/deep/f.txt").exists());
        refused!(d.delete_file("out/deep/f.txt".into()), "no such file");

        // Without a project there is nothing to write to, but the plugin lives on.
        let mut headless = data(&[Permission::ProjectWrite], Capabilities::default(), None);
        refused!(headless.write_file("a.txt".into(), "x".into()), "no project is open");
    }

    fn process(program: &str, cwd: Option<&str>, timeout_ms: Option<u32>) -> wit_types::ProcessRequest {
        wit_types::ProcessRequest { program: program.into(), args: Vec::new(), cwd: cwd.map(str::to_owned), timeout_ms }
    }

    type Seen = Arc<Mutex<Vec<(Option<String>, ProcessRequest)>>>;

    fn echo_process(seen: &Seen) -> ProcessProvider {
        let seen = seen.clone();
        Arc::new(move |client, request| {
            seen.lock().push((client, request));
            Ok(ProcessResult { exit_code: 0, stdout: "done".into(), stderr: String::new(), timed_out: false })
        })
    }

    #[test]
    fn process_host_calls_are_planned_by_the_host() {
        let (_tmp, root) = project();
        let seen: Seen = Arc::default();
        let caps = Capabilities { process: Some(echo_process(&seen)), ..Capabilities::default() };
        let mut d = data(&[Permission::Process], caps, Some(root.clone()));

        let out = d.run_process(process(" gradlew ", None, None)).unwrap().unwrap();
        assert_eq!(out.stdout, "done");
        let (client, planned) = seen.lock().pop().unwrap();
        assert_eq!(client.as_deref(), Some(root.to_string_lossy().as_ref()));
        assert_eq!(planned.program, "gradlew");
        assert_eq!(planned.cwd, None);
        assert_eq!(planned.timeout_ms, Some(DEFAULT_TIMEOUT_MS));

        d.run_process(process("tool", Some("src"), Some(600_000))).unwrap().unwrap();
        let (_, planned) = seen.lock().pop().unwrap();
        assert_eq!(planned.cwd.as_deref(), Some(root.join("src").to_string_lossy().as_ref()));
        assert_eq!(planned.timeout_ms, Some(MAX_TIMEOUT_MS));

        // A program with a separator runs from the project, as an absolute path.
        d.run_process(process("src/a.txt", None, Some(1))).unwrap().unwrap();
        let (_, planned) = seen.lock().pop().unwrap();
        assert_eq!(planned.program, root.join("src/a.txt").to_string_lossy());
        assert_eq!(planned.timeout_ms, Some(1));

        refused!(d.run_process(process("../tool", None, None)), "outside the project root");
        refused!(d.run_process(process("", None, None)), "empty");
        refused!(
            d.run_process(wit_types::ProcessRequest {
                program: "tool".into(),
                args: vec!["x".to_owned(); MAX_ARGS + 1],
                cwd: None,
                timeout_ms: None,
            }),
            "too many arguments"
        );

        let mut unserved = data(&[Permission::Process], Capabilities::default(), Some(root));
        refused!(unserved.run_process(process("tool", None, None)), "run programs");
    }

    fn http(url: &str, method: Option<&str>) -> wit_types::HttpRequest {
        wit_types::HttpRequest { url: url.into(), method: method.map(str::to_owned), headers: Vec::new(), body: None }
    }

    #[test]
    fn http_host_calls_follow_the_url_policy() {
        let (_tmp, root) = project();
        let seen: Arc<Mutex<Vec<HttpRequest>>> = Arc::default();
        let provider = {
            let seen = seen.clone();
            Arc::new(move |request: HttpRequest| {
                seen.lock().push(request);
                Ok(HttpResponse { status: 200, body: "[]".into(), content_type: Some("application/json".into()) })
            })
        };
        let caps = Capabilities { http: Some(provider), ..Capabilities::default() };
        let mut d = data(&[Permission::Net], caps, Some(root.clone()));

        let res = d.http_fetch(http(" https://example.com/index.json ", Some("post"))).unwrap().unwrap();
        assert_eq!((res.status, res.body.as_str()), (200, "[]"));
        assert_eq!(res.content_type.as_deref(), Some("application/json"));
        assert_eq!(seen.lock().pop().unwrap().url, "https://example.com/index.json");

        d.http_fetch(http("https://example.com", None)).unwrap().unwrap();
        assert_eq!(seen.lock().pop().unwrap().method.as_deref(), Some("GET"));
        d.http_fetch(http("https://example.com", Some("PATCH"))).unwrap().unwrap();
        assert_eq!(seen.lock().pop().unwrap().method.as_deref(), Some("PATCH"));

        refused!(d.http_fetch(http("http://example.com/index.json", None)), "insecure");
        refused!(d.http_fetch(http("ftp://example.com", None)), "insecure");
        refused!(d.http_fetch(http("https://example.com", Some("TRACE"))), "unsupported HTTP method");
        refused!(
            d.http_fetch(wit_types::HttpRequest {
                url: "https://example.com".into(),
                method: None,
                headers: vec![("x".into(), "y".into()); MAX_HEADERS + 1],
                body: None,
            }),
            "too many headers"
        );

        let mut unserved = data(&[Permission::Net], Capabilities::default(), Some(root));
        refused!(unserved.http_fetch(http("https://example.com", None)), "make requests");
    }

    #[test]
    fn settings_host_calls_reach_the_ide_settings() {
        let (_tmp, root) = project();
        let caps =
            Capabilities {
                settings_get: Some(Arc::new(|client: Option<String>, key: String| {
                    if key == "missing" { None } else { Some(format!("{}|{}", client?, key)) }
                })),
                settings_set: Some(Arc::new(|_client: Option<String>, key: String, json: String| {
                    if key.contains(' ') {
                        return Err("bad key".into());
                    }
                    assert_eq!(json, "2");
                    Ok(())
                })),
                ..Capabilities::default()
            };
        let mut d = data(&[Permission::SettingsRead, Permission::SettingsWrite], caps, Some(root.clone()));
        assert_eq!(
            d.settings_get(" appearance.theme ".into()).unwrap().unwrap().as_deref(),
            Some("main|appearance.theme")
        );
        assert_eq!(d.settings_get("missing".into()).unwrap().unwrap(), None);
        d.settings_set("editor.tabSize".into(), "2".into()).unwrap().unwrap();
        refused!(d.settings_set("editor.tabSize".into(), "two".into()), "not valid JSON");
        refused!(d.settings_set(String::new(), "2".into()), "empty");
        refused!(d.settings_set("bad key".into(), "2".into()), "bad key");

        let mut unserved = data(&[Permission::SettingsRead], Capabilities::default(), Some(root));
        refused!(unserved.settings_get("editor".into()), "no settings");
    }
}
