# Writing OpenIDEA plugins

OpenIDEA plugins are sandboxed **WebAssembly components** (wasmtime, component
model, WASI preview 2), in the style of Zed extensions. A plugin can register
commands (they become IDE actions you can search for and bind to keys), read
project files, insert text into the editor, show notifications, dock a tool
window, contribute themes and languages, and render live file previews (like
Android Studio's layout preview). Plugins that only contribute themes,
languages, a tool window of package text or previews need no code at all
(previews always need code).

```
plugins/
├─ build.sh                 # builds Rust plugins into components
├─ hello/                   # Rust plugin: two commands and a tool window
├─ android-preview/         # Rust plugin: previews of Android layouts and @Preview composables
└─ language-toy/            # manifest-only: a language definition
```

The API is defined in [`wit/plugin.wit`](../wit/plugin.wit) (`openidea:plugin/plugin@0.1.0`).

## 1. The package

A plugin is a directory with a `plugin.toml`:

```toml
id = "hello"                    # [a-z0-9][a-z0-9_-]*, unique
name = "Hello"
version = "0.1.0"               # semver
description = "Inserts today's date and counts words."
authors = ["You <you@example.com>"]
homepage = "https://example.com" # optional
openidea-api = "0.2"                # API version this plugin targets
wasm = "dist/plugin.wasm"       # omit for contribution-only plugins
permissions = ["project-read", "editor-read", "editor-write"]

# Optional static contributions (active even before / without code):
[[contributes.commands]]
id = "insert-date"              # [a-z0-9][a-z0-9._-]*
title = "Hello: Insert Date"
category = "Hello"              # group in Search Everywhere / Keymap

[[contributes.themes]]
path = "themes/my-theme.json"

[[contributes.languages]]
path = "languages/my-lang.toml"

[[contributes.tool-windows]]
id = "notes"                    # [a-z0-9][a-z0-9._-]*
title = "My Notes"
content = "docs/notes.md"       # a file in the package, or…
command = "show-notes"          # …a command returning text (not both)
anchor = "bottom"               # left | right | bottom (default)
icon = "package"                # optional icon name
```

All paths are relative to the package directory and may not leave it.

### Permissions

| Permission       | Unlocks                                   |
| ---------------- | ----------------------------------------- |
| `project-read`   | `workspace-root`, `read-file`, `list-dir` |
| `project-write`  | `write-file`, `create-dir`, `delete-file` |
| `editor-read`    | `active-file`                             |
| `editor-write`   | `insert-text`, `replace-selection`        |
| `process`        | `run-process`                             |
| `net`            | `http-fetch`                              |
| `settings-read`  | `settings-get`                            |
| `settings-write` | `settings-set`                            |

`log`, `register-command`, `show-notification`, `register-language` and
`register-theme` are always available. Calling a gated function without its
permission **traps**: the plugin is stopped and the user is told which
permission is missing.

API 0.2 added the last five rows and the file-system writes. API 0.3 added
file previews (see _6. File previews_). The version in `openidea-api` is what
the plugin claims to target; the host accepts anything with the same major and
a minor it implements (so `0.1` plugins keep loading). The component-model
interface itself stays at `openidea:plugin@0.1.0` — new functions are added in
place, and a plugin only references the ones it calls.

## 2. Writing code (Rust)

`wasm32-wasip2` produces a component directly, so all you need is rustup:

```sh
rustup target add wasm32-wasip2
```

`Cargo.toml`:

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[workspace]              # keep it out of any parent workspace

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.63"

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"
```

`src/lib.rs`:

```rust
wit_bindgen::generate!({ world: "plugin", path: "path/to/openidea/wit" });

use openidea::plugin::host::{self, LogLevel, NotificationLevel};

struct MyPlugin;

impl Guest for MyPlugin {
    fn activate(_ctx: Context) -> Result<(), String> {
        host::register_command("shout", "My Plugin: Shout Selection", Some("My Plugin"));
        Ok(())
    }

    fn deactivate() {}

    fn run_command(id: String, arg: Option<String>) -> Result<Option<String>, String> {
        match id.as_str() {
            "shout" => {
                host::replace_selection(&arg.unwrap_or_default().to_uppercase());
                Ok(None)
            }
            _ => Err(format!("unknown command {id}")),
        }
    }
}

export!(MyPlugin);
```

Build (copy `build.sh` or run it with your plugin's path):

```sh
plugins/build.sh hello                 # → plugins/hello/dist/plugin.wasm
plugins/build.sh /path/to/my-plugin
```

Any language with component-model tooling works (e.g. `componentize-py`,
`jco componentize`, TinyGo): the only contract is the `plugin` world.

### Host functions

| Function                                 | Notes                                                                                                                                    |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `log(level, msg)`                        | Goes to the IDE log, tagged with the plugin id. `eprintln!` output is logged too and shown when the plugin crashes.                      |
| `register-command(id, title, category?)` | Becomes action `plugin.<plugin-id>.<id>`; call again to rename. Max 256 commands.                                                        |
| `show-notification(level, title, body)`  | Balloon in the invoking window.                                                                                                          |
| `workspace-root()`                       | Root of the project the command runs in.                                                                                                 |
| `active-file()`                          | Absolute path of the focused editor's file.                                                                                              |
| `read-file(path)`                        | Absolute or project-relative; `..`/symlinks escaping the root are rejected; UTF-8 only, ≤ 16 MiB. Unsaved editor contents win over disk. |
| `insert-text(text)`                      | Inserts at every cursor of the active editor (cursor ends after the text).                                                               |
| `replace-selection(text)`                | Replaces every selection and selects the new text.                                                                                       |
| `register-language(toml)`                | Same format as `languages/<id>.toml`.                                                                                                    |
| `register-theme(json)`                   | See _Themes_ below.                                                                                                                      |
| `list-dir(path)`                         | Entries of a directory in the project, sorted, max 2000 (`name`, `is-dir`, `size`).                                                      |
| `write-file(path, text)`                 | Creates or replaces a UTF-8 file (≤ 8 MiB); the parent directory must exist. Paths are relative to the root and may not escape it.       |
| `create-dir(path)`                       | Creates a directory with its parents, inside the project.                                                                                |
| `delete-file(path)`                      | Removes one file; never a directory.                                                                                                     |
| `run-process(request)`                   | `{program, args, cwd?, timeout-ms?}` → `{exit-code, stdout, stderr, timed-out}`. No shell: argv reaches the program as given.            |
| `http-fetch(request)`                    | `{url, method?, headers?, body?}` → `{status, body, content-type}`. https only (plain http for localhost in debug builds).               |
| `settings-get(key)`                      | The effective value of a dotted settings key (`"editor.fontFamily"`) as JSON text, or none when unset.                                   |
| `settings-set(key, json)`                | Stores JSON text in the project layer when a project is open, the global layer otherwise; the Settings UI updates.                       |

`run-process` policy: a bare `program` (no path separator) is looked up on the
IDE's search path; anything with a separator must be a file inside the project,
and so must `cwd`. The timeout defaults to 20 s and is capped at 60 s; each
output stream is cut at 512 KiB. `http-fetch` allows at most 32 headers and 4
MiB of body, and returns an empty body for non-UTF-8 responses.

`run-command` receives an optional string argument: `executeAction("plugin.hello.insert-date", "arg")`
from another feature, or nothing when run from Search Everywhere / a shortcut.
Returning `Err(msg)` shows _"<title> failed: msg"_; the plugin keeps running.

### Sandbox and limits

Each plugin gets its own wasmtime `Store`: no preopened directories, no
environment, no network, no inherited stdio — only clocks and randomness from
WASI, plus the host functions above. Every call into the plugin is bounded:

- **CPU:** a fuel budget (≈ 5·10⁹ wasm instructions) per call;
- **time:** 5 s wall clock per call (epoch interruption);
- **memory:** 256 MiB of linear memory.

A panic, trap, permission violation or exceeded limit stops the plugin and
marks it _Failed_ for the session (with the reason in Settings › Plugins);
toggling it off and on, or _Reload_, starts a fresh instance. The IDE itself is
never affected.

Everything else that can go wrong — a path outside the project, a program that
could not start, an insecure URL, a bad settings key — comes back as the
`error` of the call's `result`, so the plugin can handle it and stay running.
A plugin never sees the file system, the network or the environment directly:
the writes, programs and requests above are performed by the IDE, scoped to the
project and limited as documented.

## 3. Themes

```json
{
  "id": "harbor-night",
  "name": "Harbor Night",
  "kind": "dark",
  "tokens": { "--bg-app": "#14171c", "--accent": "#2b9e93", "--syn-keyword": "#e0a46e" }
}
```

Tokens are the CSS variables of `src/styles/tokens.css` (`--bg-*`, `--fg-*`,
`--border*`, `--accent`, `--syn-*`, `--vcs-*`, `--gutter-*`, …). Unlisted tokens
keep the values of the built-in base named by `kind`. Values must be plain CSS
values (no `url()`, `;`, braces); `//` and `/* */` comments are allowed. A
manifest theme entry may add `editor-scheme = "themes/x.scheme.json"` (editor
colours, merged over the theme). Pick a theme in Settings › Plugins › _Apply_.

Start from [`templates/theme-plugin`](../templates/theme-plugin); the full guide,
token reference and IntelliJ / VS Code porting tables are in
[`docs/plugins/themes.md`](../docs/plugins/themes.md).

## 4. Languages

`languages/<id>.toml` files use the IDE's own language format; see any file in
[`languages/`](../languages) (`id`, `name`, `extensions`, `filenames`,
`first-line`, `line-comment`, `block-comment`, `indent`, `ui-mode`,
`framework-ui-modes`, `language-servers`, `debugger`). `language-toy` is a
minimal example. `[[framework-ui-modes]]` entries (`ui-mode`, `root-markers`)
replace `ui-mode` for the files of a framework's projects: the first whose
root marker is in a directory above the file, up to the project folder, applies
(see [`languages/html.toml`](../languages/html.toml), Angular templates).
Markers are matched like a language server's `root-markers`.

## 5. Tool windows

`[[contributes.tool-windows]]` docks a window in the workbench — no code needed
if it points at a file:

```toml
[[contributes.tool-windows]]
id = "notes"
title = "My Notes"
content = "docs/notes.md"   # read from the package, shown as text
anchor = "right"
```

The window appears while the plugin is enabled, in the strip named by `anchor`
(`left`, `right`, the default `bottom`), and is closed like any built-in one.
`content` files are plain text up to 1 MiB, always read from the plugin's own
package directory; the view has a _Reload File_ button.

A window without `content` instead has `command`, and shows the text that
command returns: it runs when the window opens and again when the user presses
_Run Again_. A command window needs code, and the command must be declared in
`[[contributes.commands]]` (or registered with `register-command`) — a plugin
that answers it needs no extra permission beyond the ones its commands already
use. Exactly one of `content` and `command` is required.

The `hello` sample ships a content window (`docs/about.md`) you can open after
installing it.

## 6. File previews

`[[contributes.previews]]` entries give files a live preview pane, like
Android Studio's layout preview or IntelliJ's Markdown preview. Matching files
get an _Editor / Editor and Preview / Preview_ switch around the text editor;
while it is visible, the IDE renders the editor's current text through the
plugin — unsaved edits included, debounced while typing.

```toml
[[contributes.previews]]
id = "layout-xml"               # [a-z0-9][a-z0-9._-]*, unique in the plugin
title = "Android Layout Preview"
extensions = ["xml"]            # file extensions to match (lowercase, no dot)…
path-globs = ["**/res/layout/**"]  # …and/or globs against the project-relative path
```

A file matches an entry when its extension is listed (if any are) **and** its
project-relative path matches one of the globs (if any are). Globs use `/`
separators; `**` spans directories (including none), `*` and `?` stay within
one segment.

Rendering needs code that implements the optional `render-preview` export.
Because guest exports are resolved when the host loads a component, the export
lives in its own world — generate your bindings from `world: "plugin-preview"`
(a superset of `plugin`, so your component satisfies both):

```rust
wit_bindgen::generate!({ world: "plugin-preview", path: "../../wit" });

impl Guest for MyPlugin {
    // activate / deactivate / run-command as usual, plus:
    fn render_preview(request: PreviewRequest) -> Result<PreviewOutput, String> {
        Ok(PreviewOutput {
            html: format!("<h1>{}</h1>", escape(&request.content)),
            background: None, // or Some("#111111")
        })
    }
}
```

- `request.path` is the absolute file path, `request.project-path` the same
  path relative to the project with `/` separators, `request.decl` the `id` of
  the matching entry, and `request.content` the editor's current text.
- Return `html`: a **self-contained HTML document** (inline CSS/JS, no
  external assets) shown in a sandboxed iframe (`allow-scripts` only, opaque
  origin — no access to the IDE or the project). `background` suggests the
  pane's page colour. HTML over 8 MiB is rejected.
- Returning `Err(msg)` shows the message in the preview pane; the plugin keeps
  running, so this is the right answer for a malformed file.
- The call shares the per-call fuel and wall-clock budget with everything else
  (≈5·10⁹ wasm instructions, 5 s), and needs no extra permission — but use
  `read-file` / `list-dir` (`project-read`) to resolve references into the
  project (Android `@string/…`, `@color/…`, `@layout/…` includes): unsaved
  editor contents win over disk there too.
- Components without the export keep loading unchanged; they simply offer no
  preview. Use `host::log` for diagnostics; a trap disables the plugin like
  for any other call, so validate input and return errors instead.

[`plugins/android-preview`](../plugins/android-preview) is a complete example:
it previews Android XML layouts (device frame, common views and attributes,
`tools:` overrides, resource resolution) and Jetpack Compose `@Preview`
functions. Try it on [`fixtures/android-app`](../fixtures/android-app).

## 7. Installing and developing

- **Install:** Settings › Plugins › _Install Plugin from Disk…_ (or the action
  _Install Plugin from Disk…_) and pick the package folder, or install a package
  published by a registry (see _8. Publishing to a plugin registry_). The
  manifest and the files it references are copied to `<app-data>/plugins/<id>/`.
- **Dev plugins:** put (or symlink) the package in `<project>/.openidea/plugins/<name>/`.
  It loads when the project is opened, is listed with a _dev_ tag, and _Reload_
  (or the _Reload Plugins_ action) picks up a rebuilt `plugin.wasm` without
  restarting the IDE.
- Commands appear in Search Everywhere › Actions under their category, and can
  be bound in Settings › Keymap like any other action.

## 8. Testing

The host's integration tests (`crates/ide-plugin/tests/host.rs`) load the
checked-in `hello` and test-fixture components; rebuild them after changing the
WIT file:

```sh
plugins/build.sh hello
plugins/build.sh crates/ide-plugin/tests/fixtures/misbehave
plugins/build.sh crates/ide-plugin/tests/fixtures/preview
cargo test -p ide-plugin
```

## 9. Publishing to a plugin registry

Settings › Plugins › _Available_ browses an online repository: any static
`index.json` plus package archives (a git page, an object store, an intranet
server). Point **Plugin repository URL** at the index — stored as
`plugins.registryUrl` — and the tab lists what it publishes with
_Install_ / _Update_ buttons.

```json
{
  "plugins": [
    {
      "id": "hello",
      "name": "Hello",
      "version": "0.2.0",
      "description": "Inserts today's date.",
      "authors": ["You <you@example.com>"],
      "homepage": "https://example.com/hello",
      "changeNotes": "0.2.0 — adds Word Count.",
      "openideaApi": "0.1",
      "url": "https://example.com/downloads/hello-0.2.0.zip",
      "sha256": "3f7a…",
      "size": 48123
    }
  ]
}
```

- The ZIP holds one package directory (the archive root, or its single top-level
  folder) with `plugin.toml` and the files it references.
- `https` is required for the index and for every `url` (`http://localhost` is
  allowed in debug builds, so a local registry can be tested).
- `sha256` is optional but strongly advised: an entry without one installs with
  a warning, and a mismatched checksum is refused.
- Entries with an invalid `id`, no name/version or an unsafe url are dropped
  when the index is read, so one bad row cannot break the listing.
- Caps: 4 MiB index, 64 MiB package (compressed and unpacked), 2048 files;
  symlinks and paths escaping the package are ignored.
