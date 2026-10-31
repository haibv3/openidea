//! `plugin.toml`: the plugin package manifest.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Component, Path, PathBuf};

/// Plugin API version implemented by this host (`openidea-api` in the manifest).
pub const API_VERSION: &str = "0.4";
pub const MANIFEST_FILE: &str = "plugin.toml";

/// Capabilities a plugin must declare to use the gated host functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// `workspace-root`, `read-file` (inside the project root only).
    ProjectRead,
    /// `active-file`.
    EditorRead,
    /// `insert-text`, `replace-selection`.
    EditorWrite,
    /// `write-file`, `create-dir`, `delete-file` inside the project root.
    ProjectWrite,
    /// `run-process`.
    Process,
    /// `http-fetch`.
    Net,
    /// `settings-get`.
    SettingsRead,
    /// `settings-set`.
    SettingsWrite,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::ProjectRead => "project-read",
            Permission::EditorRead => "editor-read",
            Permission::EditorWrite => "editor-write",
            Permission::ProjectWrite => "project-write",
            Permission::Process => "process",
            Permission::Net => "net",
            Permission::SettingsRead => "settings-read",
            Permission::SettingsWrite => "settings-write",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Release notes of this version (Settings › Plugins › What's New).
    #[serde(default)]
    pub change_notes: Option<String>,
    pub openidea_api: String,
    /// Component file relative to the package directory; absent for
    /// contribution-only plugins (themes, languages).
    #[serde(default)]
    pub wasm: Option<PathBuf>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub contributes: Contributions,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    #[serde(default)]
    pub languages: Vec<FileDecl>,
    #[serde(default)]
    pub themes: Vec<ThemeDecl>,
    #[serde(default)]
    pub tool_windows: Vec<ToolWindowDecl>,
    #[serde(default)]
    pub previews: Vec<PreviewDecl>,
}

/// Where a plugin's tool window docks in the workbench.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum ToolWindowAnchor {
    Left,
    Right,
    #[default]
    Bottom,
}

/// A docked view contributed by a plugin. It shows either a text file shipped in
/// the package (`content`) or the result of one of the plugin's commands
/// (`command`) — a report, a device list, build output… The window appears in
/// the stripe while the plugin is enabled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ToolWindowDecl {
    pub id: String,
    pub title: String,
    /// Text file inside the package (copied on install, at most 1 MiB shown).
    #[serde(default)]
    pub content: Option<PathBuf>,
    /// Command of this plugin whose returned text the window shows.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub anchor: ToolWindowAnchor,
    /// Icon name from the IDE's icon set; the plugin icon when absent.
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: Option<String>,
}

/// A contributed file, relative to the package directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDecl {
    pub path: PathBuf,
}

/// A contributed theme: the UI theme file plus an optional editor colour scheme
/// whose tokens are merged over it (IntelliJ's `.theme.json` + `.icls`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ThemeDecl {
    pub path: PathBuf,
    #[serde(default)]
    pub editor_scheme: Option<PathBuf>,
}

/// A file preview the plugin renders through its `render-preview` export
/// (Android Studio-style layout previews, generated docs…). Files matching an
/// entry get a preview pane; the IDE asks the plugin to render them. A file
/// matches when its extension is listed (if any are) and its project-relative
/// path matches one of the globs (if any are). The UI matches paths with the
/// same rules (see the frontend preview matcher) — keep both sides in step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PreviewDecl {
    pub id: String,
    pub title: String,
    /// File extensions (lowercase, without the dot), e.g. `["xml"]`.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Glob patterns against the path relative to the project root, with `/`
    /// separators; `**` spans directories (also none), `*`/`?` one segment.
    #[serde(default)]
    pub path_globs: Vec<String>,
}

impl PluginManifest {
    pub fn parse(src: &str) -> Result<Self, String> {
        let manifest: Self = toml::from_str(src).map_err(|e| e.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(MANIFEST_FILE);
        let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&src).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_plugin_id(&self.id)?;
        if self.name.trim().is_empty() {
            return Err("`name` must not be empty".into());
        }
        if !is_version(&self.version) {
            return Err(format!("`version` must look like 1.2.3, got {:?}", self.version));
        }
        if !api_compatible(&self.openidea_api) {
            return Err(format!(
                "unsupported openidea-api {:?} (this IDE implements {API_VERSION})",
                self.openidea_api
            ));
        }
        if let Some(wasm) = &self.wasm {
            check_relative(wasm, "wasm")?;
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.contributes.commands {
            validate_command_id(&c.id)?;
            if !seen.insert(c.id.as_str()) {
                return Err(format!("duplicate command id {:?}", c.id));
            }
            if c.title.trim().is_empty() {
                return Err(format!("command {:?} has an empty title", c.id));
            }
        }
        let theme_files = self.contributes.themes.iter().flat_map(|t| std::iter::once(&t.path).chain(&t.editor_scheme));
        for path in self.contributes.languages.iter().map(|f| &f.path).chain(theme_files) {
            check_relative(path, "contributes path")?;
        }
        let mut window_ids = std::collections::HashSet::new();
        for w in &self.contributes.tool_windows {
            validate_command_id(&w.id)?;
            if !window_ids.insert(w.id.as_str()) {
                return Err(format!("duplicate tool window id {:?}", w.id));
            }
            if w.title.trim().is_empty() {
                return Err(format!("tool window {:?} has an empty title", w.id));
            }
            match (&w.content, &w.command) {
                (Some(path), None) => check_relative(path, "tool window content")?,
                (None, Some(cmd)) => validate_command_id(cmd)?,
                (None, None) => return Err(format!("tool window {:?} needs `content` or `command`", w.id)),
                (Some(_), Some(_)) => {
                    return Err(format!("tool window {:?} cannot have both `content` and `command`", w.id));
                }
            }
        }
        let mut preview_ids = std::collections::HashSet::new();
        for p in &self.contributes.previews {
            validate_command_id(&p.id)?;
            if !preview_ids.insert(p.id.as_str()) {
                return Err(format!("duplicate preview id {:?}", p.id));
            }
            if p.title.trim().is_empty() {
                return Err(format!("preview {:?} has an empty title", p.id));
            }
            if p.extensions.is_empty() && p.path_globs.is_empty() {
                return Err(format!("preview {:?} needs `extensions` or `path-globs`", p.id));
            }
            for ext in &p.extensions {
                if ext.is_empty()
                    || ext.len() > 32
                    || !ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                {
                    return Err(format!(
                        "preview {:?} extension {ext:?} must be 1–32 lowercase letters or digits (no dot)",
                        p.id
                    ));
                }
            }
            for glob in &p.path_globs {
                let ok = !glob.is_empty()
                    && glob.len() <= 512
                    && !glob.starts_with('/')
                    && !glob.split(['/', '\\']).any(|seg| seg == ".." || seg.is_empty())
                    && glob.bytes().all(|b| b.is_ascii_alphanumeric() || b" /*?-_.".contains(&b));
                if !ok {
                    return Err(format!(
                        "preview {:?} path glob {glob:?} must be a relative pattern with '/', '*', '?' (no '..')",
                        p.id
                    ));
                }
            }
        }
        Ok(())
    }

    /// Every file of the package besides the manifest (what an install copies).
    pub fn files(&self) -> Vec<&Path> {
        let mut files: Vec<&Path> = self.wasm.iter().map(PathBuf::as_path).collect();
        files.extend(self.contributes.languages.iter().map(|f| f.path.as_path()));
        for t in &self.contributes.themes {
            files.push(t.path.as_path());
            files.extend(t.editor_scheme.as_deref());
        }
        files.extend(self.contributes.tool_windows.iter().filter_map(|w| w.content.as_deref()));
        files
    }

    pub fn has(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }
}

/// `[a-z0-9][a-z0-9_-]*`, at most 64 chars (no dots: it is one segment of an action id).
pub fn validate_plugin_id(id: &str) -> Result<(), String> {
    validate_ident(id, "plugin id", false)
}

/// `[a-z0-9][a-z0-9._-]*`, at most 64 chars.
pub fn validate_command_id(id: &str) -> Result<(), String> {
    validate_ident(id, "command id", true)
}

fn validate_ident(id: &str, what: &str, dots: bool) -> Result<(), String> {
    let ok_first = id.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let ok_rest =
        id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || (dots && c == '.'));
    if ok_first && ok_rest && id.len() <= 64 {
        Ok(())
    } else {
        let allowed = if dots { "a-z, 0-9, '.', '-', '_'" } else { "a-z, 0-9, '-', '_'" };
        Err(format!("invalid {what} {id:?}: use {allowed}, starting with a letter or digit (max 64)"))
    }
}

fn is_version(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Pre-1.0 the minor part is the breaking one: a plugin written for API `0.1`
/// runs on a `0.2` host (0.2 only adds imports), one written for `0.3` does not.
fn api_compatible(v: &str) -> bool {
    let version = |text: &str| {
        let core = text.split(['-', '+']).next().unwrap_or_default();
        let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
        Some((parts.next()??, parts.next().unwrap_or(Some(0))?))
    };
    match (version(v), version(API_VERSION)) {
        (Some((major, minor)), Some((host_major, host_minor))) => major == host_major && minor <= host_minor,
        _ => false,
    }
}

/// Package paths must stay inside the package directory.
fn check_relative(path: &Path, what: &str) -> Result<(), String> {
    let ok = !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)));
    if ok { Ok(()) } else { Err(format!("{what} {:?} must be a relative path inside the plugin", path.display())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
        id = "hello"
        name = "Hello"
        version = "0.1.0"
        description = "Says hello"
        authors = ["A"]
        openidea-api = "0.1"
        wasm = "dist/plugin.wasm"
        permissions = ["project-read", "editor-write"]

        [[contributes.commands]]
        id = "say.hi"
        title = "Say Hi"
        category = "Hello"

        [[contributes.themes]]
        path = "themes/dark.json"
        editor-scheme = "themes/dark.scheme.json"

        [[contributes.tool-windows]]
        id = "about"
        title = "Hello Notes"
        content = "docs/about.md"
        anchor = "bottom"

        [[contributes.previews]]
        id = "layout-xml"
        title = "Android Layout Preview"
        extensions = ["xml"]
        path-globs = ["**/res/layout/**"]
    "#;

    #[test]
    fn parses_full_manifest() {
        let m = PluginManifest::parse(FULL).unwrap();
        assert_eq!(m.id, "hello");
        assert_eq!(m.permissions, vec![Permission::ProjectRead, Permission::EditorWrite]);
        assert_eq!(m.contributes.commands[0].category.as_deref(), Some("Hello"));
        assert_eq!(
            m.files(),
            vec![
                Path::new("dist/plugin.wasm"),
                Path::new("themes/dark.json"),
                Path::new("themes/dark.scheme.json"),
                Path::new("docs/about.md")
            ]
        );
        assert!(m.has(Permission::EditorWrite) && !m.has(Permission::EditorRead));
        let w = &m.contributes.tool_windows[0];
        assert_eq!((w.id.as_str(), w.title.as_str(), w.anchor), ("about", "Hello Notes", ToolWindowAnchor::Bottom));
        assert_eq!(w.command, None);
        let p = &m.contributes.previews[0];
        assert_eq!(p.id, "layout-xml");
        assert_eq!(p.title, "Android Layout Preview");
        assert_eq!(p.extensions, ["xml"]);
        assert_eq!(p.path_globs, ["**/res/layout/**"]);
    }

    #[test]
    fn tool_windows_need_one_of_content_and_command() {
        let base = "id = \"t\"\nname = \"T\"\nversion = \"1.0.0\"\nopenidea-api = \"0.2\"\n";
        let window = |extra: &str| format!("{base}[[contributes.tool-windows]]\nid = \"w\"\ntitle = \"W\"\n{extra}");
        assert!(PluginManifest::parse(&window("content = \"docs/w.md\"")).is_ok());
        assert!(PluginManifest::parse(&window("command = \"run.w\"")).is_ok());
        assert!(PluginManifest::parse(&window("")).unwrap_err().contains("needs `content` or `command`"));
        assert!(
            PluginManifest::parse(&window("content = \"a.md\"\ncommand = \"x\""))
                .unwrap_err()
                .contains("both `content` and `command`")
        );
        assert!(PluginManifest::parse(&window("content = \"../out.md\"")).unwrap_err().contains("relative path"));
        assert!(PluginManifest::parse(&window("content = \"a.md\"\nanchor = \"top\"")).is_err());
        assert!(
            PluginManifest::parse(&format!(
                "{base}[[contributes.tool-windows]]\nid = \"w\"\ntitle = \" \"\ncontent = \"a.md\"\n"
            ))
            .unwrap_err()
            .contains("empty title")
        );
        assert!(
            PluginManifest::parse(&format!(
                "{base}[[contributes.tool-windows]]\nid = \"Bad Id\"\ntitle = \"W\"\ncontent = \"a.md\"\n"
            ))
            .unwrap_err()
            .contains("invalid command id")
        );
        let two = format!(
            "{base}[[contributes.tool-windows]]\nid = \"w\"\ntitle = \"W\"\ncontent = \"a.md\"\n\
             [[contributes.tool-windows]]\nid = \"w\"\ntitle = \"W2\"\ncontent = \"b.md\"\n"
        );
        assert!(PluginManifest::parse(&two).unwrap_err().contains("duplicate tool window id"));
    }

    #[test]
    fn minimal_contribution_only_manifest() {
        let m =
            PluginManifest::parse("id = \"t\"\nname = \"T\"\nversion = \"1.0.0\"\nopenidea-api = \"0.1.3\"").unwrap();
        assert!(m.wasm.is_none() && m.permissions.is_empty());
    }

    #[test]
    fn accepts_older_api_minors() {
        assert!(api_compatible("0.1"));
        assert!(api_compatible("0.1.4"));
        assert!(api_compatible("0.2"));
        assert!(api_compatible(API_VERSION));
        assert!(!api_compatible("0.5"));
        assert!(!api_compatible("1.0"));
        assert!(api_compatible("0")); // a missing minor means 0
        assert!(!api_compatible("x"));
    }

    #[test]
    fn preview_decls_are_validated() {
        let base = "id = \"t\"\nname = \"T\"\nversion = \"1.0.0\"\nopenidea-api = \"0.3\"\n";
        let preview = |extra: &str| format!("{base}[[contributes.previews]]\nid = \"p\"\ntitle = \"P\"\n{extra}");
        let untitled = |extra: &str| format!("{base}[[contributes.previews]]\nid = \"p\"\n{extra}");
        let no_id = |extra: &str| format!("{base}[[contributes.previews]]\n{extra}");
        assert!(PluginManifest::parse(&preview("extensions = [\"xml\"]")).is_ok());
        assert!(PluginManifest::parse(&preview("path-globs = [\"**/res/**\"]")).is_ok());
        assert!(err(&preview("")).contains("needs `extensions` or `path-globs`"));
        assert!(err(&preview("extensions = []")).contains("needs `extensions`"));
        assert!(err(&preview("extensions = [\"XML\"]")).contains("lowercase"));
        assert!(err(&preview("extensions = [\".xml\"]")).contains("lowercase"));
        assert!(err(&preview("extensions = [\"a b\"]")).contains("lowercase"));
        assert!(err(&preview("path-globs = [\"/abs/**\"]")).contains("pattern"));
        assert!(err(&preview("path-globs = [\"../escape\"]")).contains("pattern"));
        assert!(err(&preview("path-globs = [\"a//b\"]")).contains("pattern"));
        assert!(err(&preview("path-globs = [\"a$\"]")).contains("pattern"));
        assert!(err(&untitled("extensions = [\"xml\"]")).contains("title"));
        assert!(err(&untitled("title = \" \"\nextensions = [\"xml\"]")).contains("empty title"));
        assert!(err(&no_id("id = \"Bad Id\"\ntitle = \"P\"\nextensions = [\"xml\"]")).contains("invalid command id"));
        let two = format!(
            "{base}[[contributes.previews]]\nid = \"p\"\ntitle = \"P\"\nextensions = [\"xml\"]\n\
             [[contributes.previews]]\nid = \"p\"\ntitle = \"P2\"\nextensions = [\"kt\"]\n"
        );
        assert!(err(&two).contains("duplicate preview id"));
    }

    fn err(src: &str) -> String {
        PluginManifest::parse(src).unwrap_err()
    }

    #[test]
    fn rejects_invalid_manifests() {
        let base = |extra: &str, id: &str, api: &str| {
            format!("id = \"{id}\"\nname = \"N\"\nversion = \"1.0.0\"\nopenidea-api = \"{api}\"\n{extra}")
        };
        assert!(err(&base("", "Bad.Id", "0.1")).contains("invalid plugin id"));
        assert!(err(&base("", "ok", "0.5")).contains("unsupported openidea-api"));
        assert!(err(&base("", "ok", "0.10")).contains("unsupported openidea-api"));
        assert!(err(&base("permissions = [\"network\"]", "ok", "0.1")).contains("unknown variant"));
        assert!(err(&base("wasm = \"../evil.wasm\"", "ok", "0.1")).contains("relative path"));
        assert!(err(&base("wasm = \"/abs.wasm\"", "ok", "0.1")).contains("relative path"));
        assert!(err(&base("bogus = 1", "ok", "0.1")).contains("unknown field"));
        let dup =
            "[[contributes.commands]]\nid = \"a\"\ntitle = \"A\"\n[[contributes.commands]]\nid = \"a\"\ntitle = \"B\"";
        assert!(err(&base(dup, "ok", "0.1")).contains("duplicate command"));
        let traversal = "[[contributes.languages]]\npath = \"a/../../x.toml\"";
        assert!(err(&base(traversal, "ok", "0.1")).contains("relative path"));
        let scheme_escape = "[[contributes.themes]]\npath = \"t.json\"\neditor-scheme = \"../s.json\"";
        assert!(err(&base(scheme_escape, "ok", "0.1")).contains("relative path"));
        assert!(err("id = \"ok\"\nname = \"N\"\nversion = \"1.0\"\nopenidea-api = \"0.1\"").contains("version"));
        assert!(err("id = \"ok\"\nname = \" \"\nversion = \"1.0.0\"\nopenidea-api = \"0.1\"").contains("name"));
    }

    #[test]
    fn ident_rules() {
        assert!(validate_command_id("word-count.v2_x").is_ok());
        assert!(validate_command_id("-x").is_err());
        assert!(validate_command_id("").is_err());
        assert!(validate_command_id(&"a".repeat(65)).is_err());
        assert!(validate_plugin_id("a.b").is_err());
        assert!(is_version("1.2.3-beta.1+build"));
        assert!(!is_version("1.2.x"));
    }
}
