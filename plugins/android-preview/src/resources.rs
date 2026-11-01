//! Android resource resolution over a pluggable source.
//!
//! [`ResourceSource`] abstracts "read a file / list a directory below the
//! project's `res/` root". The plugin wires it to the host (`read-file` /
//! `list-dir`); tests use an in-memory map. `Resources` scans `res/values*`
//! once per render and resolves `@string/x`, `@color/x`, `@dimen/x`,
//! `@style/x`, `@bool/x`, `@integer/x`, `@android:color/…`, `#hex` literals
//! and `?attr/…` theme references.
//!
//! Pure Rust, no host calls: fully unit-testable natively.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::xml;

/// Where resource files come from. Paths are relative to the `res/` root with
/// `/` separators (`values/strings.xml`, `layout/activity_main.xml`); `dir`
/// for [`ResourceSource::list`] is `""` for the root itself.
pub trait ResourceSource {
    fn read(&self, path: &str) -> Option<String>;
    fn list(&self, dir: &str) -> Vec<String>;
    /// Raw bytes of a file (images). Text-only sources leave the default.
    fn read_bytes(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    /// A UTF-8 file outside `res/`, project-relative (`app/src/main/AndroidManifest.xml`).
    /// Needed for the app theme and for custom view superclasses; sources without
    /// project access leave the default and those lookups degrade gracefully.
    fn read_project(&self, _path: &str) -> Option<String> {
        None
    }
    /// Entries of a project-relative directory (names only).
    fn list_project(&self, _dir: &str) -> Vec<String> {
        Vec::new()
    }
    /// Gradle module the previewed file belongs to (`app`), `""` for the root.
    fn module(&self) -> &str {
        ""
    }
    /// `values*/*.xml` contents, least authoritative first so a later file
    /// overrides an earlier one. Multi-root sources (module res plus merged
    /// library resources) order the roots so the app's own values come last.
    fn values_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut values_dirs: Vec<(u32, String)> =
            self.list("").into_iter().filter_map(|entry| values_dir_version(&entry).map(|v| (v, entry))).collect();
        values_dirs.sort();
        for (_, entry) in values_dirs {
            for file in self.list(&entry) {
                if file.ends_with(".xml") {
                    if let Some(text) = self.read(&format!("{entry}/{file}")) {
                        out.push(text);
                    }
                }
            }
        }
        out
    }
}

/// A resolved `@drawable`/`@mipmap`: a parsed `<shape>`, an inline SVG built
/// from a `<vector>`, or a `data:` URI wrapping a bitmap file.
#[derive(Debug, Clone, PartialEq)]
pub enum ResDrawable {
    Shape(ShapeDrawable),
    Svg(String),
    Image(String),
}

/// How a `<gradient>` is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    Sweep,
}

/// A `<gradient>` inside a `<shape>`: colours in document order (start,
/// optional centre, end) plus the Android angle in degrees (0 = left→right,
/// 90 = bottom→top).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gradient {
    pub kind: GradientKind,
    pub angle: f32,
    pub colors: Vec<String>,
    /// Radial centre/radius as fractions of the drawable box.
    pub center: Option<(f32, f32)>,
    pub radius: Option<f32>,
}

/// One `<item>` of a `res/menu` file, title already resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub title: String,
    pub icon: Option<ResDrawable>,
}

/// An in-memory source for tests. Directory listings are derived from the
/// registered file paths, so a test only needs to register files (extra empty
/// directories can be declared with [`MemorySource::dir`]).
#[cfg(any(test, not(target_arch = "wasm32")))]
pub struct MemorySource {
    pub files: HashMap<String, String>,
    dirs: HashMap<String, Vec<String>>,
    project: HashMap<String, String>,
    module: String,
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl MemorySource {
    pub fn new() -> Self {
        Self { files: HashMap::new(), dirs: HashMap::new(), project: HashMap::new(), module: String::new() }
    }

    #[cfg(test)]
    pub fn file(mut self, path: &str, content: &str) -> Self {
        self.files.insert(path.to_owned(), content.to_owned());
        self
    }

    #[cfg(test)]
    pub fn dir(mut self, dir: &str, entries: &[&str]) -> Self {
        self.dirs.insert(dir.to_owned(), entries.iter().map(|s| s.to_string()).collect());
        self
    }

    /// A file outside `res/` (manifest, Kotlin/Java sources), project-relative.
    #[cfg(test)]
    pub fn project_file(mut self, path: &str, content: &str) -> Self {
        self.project.insert(path.to_owned(), content.to_owned());
        self
    }

    #[cfg(test)]
    pub fn module(mut self, module: &str) -> Self {
        self.module = module.to_owned();
        self
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl Default for MemorySource {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl ResourceSource for MemorySource {
    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn read_project(&self, path: &str) -> Option<String> {
        self.project.get(path).cloned()
    }

    fn list_project(&self, dir: &str) -> Vec<String> {
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        let mut out: Vec<String> = Vec::new();
        for path in self.project.keys() {
            if let Some(rest) = path.strip_prefix(prefix.as_str()) {
                let entry = rest.split('/').next().unwrap_or(rest);
                if !entry.is_empty() && !out.iter().any(|e| e == entry) {
                    out.push(entry.to_owned());
                }
            }
        }
        out.sort();
        out
    }

    fn module(&self) -> &str {
        &self.module
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let mut out: Vec<String> = self.dirs.get(dir).cloned().unwrap_or_default();
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        for path in self.files.keys() {
            if let Some(rest) = path.strip_prefix(prefix.as_str()) {
                let entry = rest.split('/').next().unwrap_or(rest);
                if !entry.is_empty() && !out.iter().any(|e| e == entry) {
                    out.push(entry.to_owned());
                }
            }
        }
        out.sort();
        out
    }
}

/// M3-like light palette used for `?attr/…` defaults and Material widgets.
pub const M3_PRIMARY: &str = "#6750A4";
pub const M3_ON_PRIMARY: &str = "#FFFFFF";
pub const M3_PRIMARY_CONTAINER: &str = "#EADDFF";
pub const M3_SECONDARY: &str = "#625B71";
pub const M3_SURFACE: &str = "#FEF7FF";
pub const M3_ON_SURFACE: &str = "#1D1B20";
pub const M3_SURFACE_VARIANT: &str = "#E7E0EC";
pub const M3_ON_SURFACE_VARIANT: &str = "#49454F";
pub const M3_OUTLINE: &str = "#79747E";
pub const M3_OUTLINE_VARIANT: &str = "#CAC4D0";
pub const M3_ERROR: &str = "#B3261E";

/// `?attr/…` / `?android:attr/…` → a default light-theme color.
pub fn theme_attr_css(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix('?').unwrap_or(name).rsplit('/').next().unwrap_or(name);
    Some(match name {
        // Material 3 Compose colorScheme names (MaterialTheme.colorScheme.*).
        "primary" | "colorPrimary" | "colorAccent" | "colorControlActivated" => M3_PRIMARY,
        "onPrimary" | "colorOnPrimary" => M3_ON_PRIMARY,
        "primaryContainer" | "colorPrimaryContainer" => M3_PRIMARY_CONTAINER,
        "onPrimaryContainer" | "colorOnPrimaryContainer" => "#21005D",
        "secondary" | "colorSecondary" => M3_SECONDARY,
        "onSecondary" | "colorOnSecondary" => "#FFFFFF",
        "secondaryContainer" | "colorSecondaryContainer" => "#E8DEF8",
        "onSecondaryContainer" | "colorOnSecondaryContainer" => "#1D192B",
        "tertiary" | "colorTertiary" => "#7D5260",
        "onTertiary" | "colorOnTertiary" => "#FFFFFF",
        "tertiaryContainer" | "colorTertiaryContainer" => "#FFD8E4",
        "onTertiaryContainer" => "#31111D",
        "error" | "colorError" => M3_ERROR,
        "onError" | "colorOnError" => "#FFFFFF",
        "errorContainer" => "#F9DEDC",
        "onErrorContainer" => "#410E0B",
        "background" | "colorBackground" | "colorBackgroundFloating" | "surface" | "colorSurface" => M3_SURFACE,
        "onSurface" | "colorOnSurface" | "textColorPrimary" => M3_ON_SURFACE,
        "onBackground" | "colorOnBackground" => M3_ON_SURFACE,
        "surfaceVariant" | "colorSurfaceVariant" | "surfaceContainer" | "colorSurfaceContainer" => M3_SURFACE_VARIANT,
        "onSurfaceVariant"
        | "colorOnSurfaceVariant"
        | "textColorSecondary"
        | "textColorTertiary"
        | "colorControlNormal" => M3_ON_SURFACE_VARIANT,
        "outline" | "colorOutline" | "textColorHint" => M3_OUTLINE,
        "outlineVariant" | "colorOutlineVariant" => M3_OUTLINE_VARIANT,
        "colorPrimaryDark" | "colorPrimaryVariant" => "#4F378B",
        "colorSecondaryVariant" => "#332D41",
        "colorForeground" | "android:colorForeground" => M3_ON_SURFACE,
        "colorButtonNormal" | "colorSwitchThumbNormal" => M3_SURFACE_VARIANT,
        "colorEdgeEffect" | "colorControlHighlight" => "rgba(103,80,164,0.12)",
        "colorLink" | "textColorLink" => M3_PRIMARY,
        "editTextColor" => M3_ON_SURFACE,
        "windowBackground" => M3_SURFACE,
        "statusBarColor" | "navigationBarColor" => M3_SURFACE,
        "colorOnSurfaceInverse" | "inverseOnSurface" => "#F2F2F2",
        "colorSurfaceInverse" | "inverseSurface" => "#303030",
        "surfaceTint" => M3_PRIMARY,
        "surfaceDim" => "#DED8E1",
        "surfaceBright" => M3_SURFACE,
        "surfaceContainerLowest" => "#FFFFFF",
        "surfaceContainerLow" => "#F7F2FA",
        "surfaceContainerHigh" => "#ECE6F0",
        "surfaceContainerHighest" => "#E6E0E9",
        "dividerHorizontal" | "dividerVertical" => "rgba(0,0,0,0.12)",
        "selectableItemBackground" | "selectableItemBackgroundBorderless" => "transparent",
        "textColorPrimaryInverse" => "#FFFFFF",
        _ => return None,
    })
}

/// A small built-in `@android:color/…` palette (values are `#AARRGGBB`).
pub fn android_color(name: &str) -> Option<&'static str> {
    Some(match name {
        "black" => "#FF000000",
        "white" => "#FFFFFFFF",
        "transparent" => "#00000000",
        "background_dark" => "#FF000000",
        "background_light" => "#FFFFFFFF",
        "darker_gray" => "#FFAAAAAA",
        "lighter_gray" => "#FFCCCCCC",
        "holo_blue_bright" => "#FF00DDFF",
        "holo_blue_light" => "#FF33B5E5",
        "holo_blue_dark" => "#FF0099CC",
        "holo_green_light" => "#FF99CC00",
        "holo_green_dark" => "#FF669900",
        "holo_red_light" => "#FFFF4444",
        "holo_red_dark" => "#FFCC0000",
        "holo_orange_light" => "#FFFFBB33",
        "holo_orange_dark" => "#FFFF8800",
        "holo_purple" => "#FFAA66CC",
        "primary_text_light" => "#DE000000",
        "secondary_text_light" => "#8A000000",
        "tertiary_text_light" => "#4D000000",
        "primary_text_dark" => "#FFFFFFFF",
        "secondary_text_dark" => "#B3FFFFFF",
        "tertiary_text_dark" => "#80FFFFFF",
        _ => return None,
    })
}

/// `?attr/actionBarSize` and friends → the platform default in px. Layouts use
/// these for toolbar/list heights; without them the value is dropped entirely.
pub fn theme_attr_dimen(name: &str) -> Option<f32> {
    let name = name.strip_prefix('?').unwrap_or(name).rsplit('/').next().unwrap_or(name);
    Some(match name {
        "actionBarSize" => 56.0,
        "alertDialogTheme" => 0.0,
        "listPreferredItemHeight" => 64.0,
        "listPreferredItemHeightSmall" => 48.0,
        "listPreferredItemHeightLarge" => 80.0,
        "listPreferredItemPaddingLeft" | "listPreferredItemPaddingRight" => 16.0,
        "listPreferredItemPaddingStart" | "listPreferredItemPaddingEnd" => 16.0,
        "dropdownListPreferredItemHeight" => 48.0,
        "textSize" => 14.0,
        "statusBarHeight" => 24.0,
        "navigationBarHeight" => 48.0,
        "toolbarStyle" => 56.0,
        "mtrl_toolbar_default_height" => 56.0,
        "bottomAppBarHeight" | "mtrl_bottomappbar_height" => 80.0,
        "chipMinHeight" | "mtrl_chip_min_height" => 32.0,
        "textInputEditTextSize" => 16.0,
        "dialogMinWidthMajor" | "dialogMinWidthMinor" => 0.0,
        _ => return None,
    })
}

/// A small built-in `@android:dimen/…` table.
pub fn android_dimen(name: &str) -> Option<f32> {
    Some(match name {
        "status_bar_height" => 24.0,
        "navigation_bar_height" => 48.0,
        "app_icon_size" => 48.0,
        "notification_large_icon_width" | "notification_large_icon_height" => 64.0,
        "system_app_widget_background_radius" => 28.0,
        "text_size_large" => 22.0,
        "text_size_medium" => 18.0,
        "text_size_small" => 14.0,
        "dialog_min_width_major" | "dialog_min_width_minor" => 0.0,
        "default_gap" => 8.0,
        _ => return None,
    })
}

/// SVG `<path>` elements for a `<vector>` tree (groups flattened; fill and
/// stroke colours resolved through the resource tables).
fn vector_paths(el: &xml::Element, out: &mut String, res: &Resources) {
    for child in el.elements() {
        match child.name_local() {
            "path" => {
                let Some(d) = child.attr("android:pathData") else { continue };
                let fill = child
                    .attr("android:fillColor")
                    .and_then(|v| res.color(v, 0))
                    .unwrap_or_else(|| "#FF000000".to_owned());
                let mut extra = String::new();
                if let Some(a) = child.attr("android:fillAlpha").and_then(|v| v.trim().parse::<f32>().ok()) {
                    extra.push_str(&format!(" fill-opacity=\"{a}\""));
                }
                if let Some(sc) = child.attr("android:strokeColor").and_then(|v| res.color(v, 0)) {
                    let sw =
                        child.attr("android:strokeWidth").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0);
                    out.push_str(&format!(
                        "<path d=\"{}\" fill=\"{fill}\" stroke=\"{sc}\" stroke-width=\"{sw}\"{extra}/>",
                        xml_escape(d)
                    ));
                } else {
                    out.push_str(&format!("<path d=\"{}\" fill=\"{fill}\"{extra}/>", xml_escape(d)));
                }
            }
            "group" | "clip-path" => vector_paths(child, out, res),
            _ => {}
        }
    }
}

/// Minimal `&` `<` escaping for attribute values.
fn xml_escape(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

/// `data:` URI for a bitmap, by magic bytes (ext as fallback for the MIME).
fn data_uri(bytes: &[u8], ext: &str) -> Option<String> {
    let mime = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"\xFF\xD8") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => return None,
        }
    };
    Some(format!("data:{mime};base64,{}", base64_encode(bytes)))
}

/// RFC 4648 base64.
pub fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = b0 << 16 | b1 << 8 | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Built-in `@android:string/…` subset.
fn android_string(name: &str) -> Option<&'static str> {
    Some(match name {
        "ok" => "OK",
        "cancel" => "Cancel",
        "yes" => "Yes",
        "no" => "No",
        "done" => "Done",
        "save" => "Save",
        "delete" => "Delete",
        "search" => "Search",
        _ => return None,
    })
}

/// Parses `#rgb`, `#argb`, `#rrggbb`, `#aarrggbb` into a CSS color: opaque
/// values stay `#rrggbb`, translucent ones become `rgba(r, g, b, a)`.
pub fn color_css(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix('#')?;
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (a, rgb): (u8, String) = match hex.len() {
        3 => (255, hex.chars().flat_map(|c| [c, c]).collect()),
        4 => {
            let a = u8::from_str_radix(&hex[..1].repeat(2), 16).ok()?;
            (a, hex[1..].chars().flat_map(|c| [c, c]).collect())
        }
        6 => (255, hex.to_owned()),
        8 => {
            let a = u8::from_str_radix(&hex[..2], 16).ok()?;
            (a, hex[2..].to_owned())
        }
        _ => return None,
    };
    let r = u8::from_str_radix(&rgb[0..2], 16).ok()?;
    let g = u8::from_str_radix(&rgb[2..4], 16).ok()?;
    let b = u8::from_str_radix(&rgb[4..6], 16).ok()?;
    if a == 255 { Some(format!("#{rgb:0>6}")) } else { Some(format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0)) }
}

/// Parses a literal dimension into px (dp/sp/px 1:1 for the preview viewport).
pub fn dimen_px(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    let split = raw.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(raw.len());
    let (num, unit) = raw.split_at(split);
    let n: f32 = num.parse().ok()?;
    Some(match unit {
        "" | "dp" | "dip" | "sp" | "px" => n,
        "pt" => n * 96.0 / 72.0,
        "in" => n * 160.0,
        "mm" => n * 160.0 / 25.4,
        _ => return None,
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapeDrawable {
    pub fill: Option<String>,
    /// Painted instead of `fill` when the shape declares a `<gradient>`.
    pub gradient: Option<Gradient>,
    /// `[top-left, top-right, bottom-right, bottom-left]` in px.
    pub corners: [f32; 4],
    pub stroke: Option<(f32, String)>,
    /// `[left, top, right, bottom]` in px.
    pub padding: [f32; 4],
    /// `<size>` intrinsic dimensions in px, when declared.
    pub size: Option<(f32, f32)>,
}

impl ShapeDrawable {
    /// Resolves every `?attr/…` colour through a theme (drawables are parsed
    /// without one; the calling view supplies the theme it is inflated with).
    pub fn resolve_theme(&mut self, resolve: impl Fn(&str) -> Option<String>) {
        let fix = |slot: &mut Option<String>, resolve: &dyn Fn(&str) -> Option<String>| {
            if let Some(v) = slot.take() {
                *slot = Some(if v.starts_with('?') { resolve(&v).unwrap_or(v) } else { v });
            }
        };
        fix(&mut self.fill, &resolve);
        if let Some(stroke) = &mut self.stroke
            && stroke.1.starts_with('?')
        {
            if let Some(c) = resolve(&stroke.1) {
                stroke.1 = c;
            }
        }
        if let Some(g) = &mut self.gradient {
            for c in &mut g.colors {
                if c.starts_with('?')
                    && let Some(v) = resolve(c)
                {
                    *c = v;
                }
            }
        }
    }
}

struct StyleDef {
    parent: Option<String>,
    items: Vec<(String, String)>,
}

/// Themes declared in the module's `AndroidManifest.xml`. Android Studio
/// previews against the app theme (or the activity's when the layout names one
/// through `tools:context`), which is what makes `?attr/colorPrimary` show the
/// app's brand colour instead of the M3 fallback.
#[derive(Debug, Clone, Default)]
pub struct ManifestThemes {
    pub package: String,
    pub application: Option<String>,
    /// `activity android:name` → `android:theme`.
    pub activities: Vec<(String, String)>,
}

/// Resolved resource tables for one render call.
pub struct Resources<'a> {
    source: &'a dyn ResourceSource,
    strings: HashMap<String, String>,
    colors: HashMap<String, String>,
    dimens: HashMap<String, String>,
    bools: HashMap<String, bool>,
    integers: HashMap<String, i64>,
    styles: HashMap<String, StyleDef>,
    layout_dirs: Vec<String>,
    drawable_dirs: Vec<String>,
    mipmap_dirs: Vec<String>,
    color_dirs: Vec<String>,
    menu_dirs: Vec<String>,
    manifest: ManifestThemes,
    /// Class → superclass cache for custom view resolution (project sources).
    parents: RefCell<HashMap<String, Option<String>>>,
}

impl<'a> Resources<'a> {
    /// Scans the `res/` root: every `values*/​*.xml` is parsed into the tables;
    /// `layout*` and `drawable*` directories are remembered for includes and
    /// shape lookups. Missing/unreadable files are tolerated (they resolve to
    /// "unresolved reference" later).
    pub fn new(source: &'a dyn ResourceSource) -> Self {
        let mut res = Resources {
            source,
            strings: HashMap::new(),
            colors: HashMap::new(),
            dimens: HashMap::new(),
            bools: HashMap::new(),
            integers: HashMap::new(),
            styles: HashMap::new(),
            layout_dirs: Vec::new(),
            drawable_dirs: Vec::new(),
            mipmap_dirs: Vec::new(),
            color_dirs: Vec::new(),
            menu_dirs: Vec::new(),
            manifest: ManifestThemes::default(),
            parents: RefCell::new(HashMap::new()),
        };
        // `values_files` orders the roots: merged library resources first,
        // the module's own `values/` last so it wins on ties. Only the
        // configurations the default preview shows (`values` plus API-level
        // variants; `values-night`, `values-fr`… are filtered by the source).
        for text in source.values_files() {
            res.load_values(&text);
        }
        for entry in source.list("") {
            if values_dir_version(&entry).is_some() {
                continue;
            }
            if entry.starts_with("layout") {
                res.layout_dirs.push(entry);
            } else if entry.starts_with("drawable") {
                res.drawable_dirs.push(entry);
            } else if entry.starts_with("mipmap") {
                res.mipmap_dirs.push(entry);
            } else if entry.starts_with("color") {
                res.color_dirs.push(entry);
            } else if entry.starts_with("menu") {
                res.menu_dirs.push(entry);
            }
        }
        // `layout` itself first, so unqualified includes win over -land/-sw600dp.
        res.layout_dirs.sort_by_key(|d| (d != "layout", d.clone()));
        res.drawable_dirs.sort_by_key(|d| (d != "drawable", d.clone()));
        res.color_dirs.sort_by_key(|d| (d != "color", d.clone()));
        res.menu_dirs.sort_by_key(|d| (d != "menu", d.clone()));
        res.manifest = res.read_manifest();
        res
    }

    /// `AndroidManifest.xml` of the module (or the project root): the app theme
    /// and each activity's theme.
    fn read_manifest(&self) -> ManifestThemes {
        let module = self.source.module();
        let prefix = if module.is_empty() { String::new() } else { format!("{module}/") };
        let candidates = [
            format!("{prefix}src/main/AndroidManifest.xml"),
            format!("{prefix}AndroidManifest.xml"),
            "src/main/AndroidManifest.xml".to_owned(),
            "AndroidManifest.xml".to_owned(),
        ];
        let Some(text) = candidates.iter().find_map(|p| self.source.read_project(p)) else {
            return ManifestThemes::default();
        };
        parse_manifest(&text)
    }

    /// The application theme reference (`@style/Theme.App`), if declared.
    pub fn app_theme(&self) -> Option<&str> {
        self.manifest.application.as_deref()
    }

    /// Theme of the activity named by `tools:context` (`.MainActivity`,
    /// `com.x.MainActivity` or `MainActivity`).
    pub fn activity_theme(&self, context: &str) -> Option<&str> {
        let want = context.trim_start_matches('.');
        let simple = want.rsplit('.').next().unwrap_or(want);
        self.manifest
            .activities
            .iter()
            .find(|(name, _)| {
                let name = name.trim_start_matches('.');
                name == want || name.rsplit('.').next() == Some(simple)
            })
            .map(|(_, theme)| theme.as_str())
    }

    /// Loads one `res/values*` file.
    pub fn load_values(&mut self, xml_text: &str) {
        let Ok(doc) = xml::parse(xml_text) else { return };
        let Some(root) = doc.root else { return };
        for el in root.elements() {
            let name = match el.attr("name") {
                Some(n) => n.to_owned(),
                None => continue,
            };
            match el.name_local() {
                "string" => {
                    self.strings.insert(name, decode_escapes(&el.text_content()));
                }
                "color" => {
                    self.colors.insert(name, el.direct_text());
                }
                "dimen" => {
                    self.dimens.insert(name, el.direct_text());
                }
                "bool" => {
                    if let Some(v) = parse_bool(&el.direct_text()) {
                        self.bools.insert(name, v);
                    }
                }
                "integer" => {
                    if let Ok(v) = el.direct_text().trim().parse::<i64>() {
                        self.integers.insert(name, v);
                    }
                }
                "style" => {
                    let parent = el.attr("parent").map(|p| style_ref_name(p));
                    let items = el
                        .elements()
                        .filter(|it| it.name_local() == "item")
                        .filter_map(|it| it.attr("name").map(|n| (n.to_owned(), it.text_content())))
                        .collect();
                    self.styles.insert(name, StyleDef { parent, items });
                }
                _ => {}
            }
        }
    }

    /// `@string/x`, `@android:string/ok` or a literal.
    pub fn text(&self, raw: &str, depth: usize) -> Option<String> {
        if raw == "@null" || raw == "@empty" {
            return Some(String::new());
        }
        if let Some(rest) = raw.strip_prefix("@android:string/") {
            return android_string(rest).map(str::to_owned);
        }
        if let Some(name) = raw.strip_prefix("@string/") {
            if depth > 8 {
                return None;
            }
            let v = self.strings.get(name)?.clone();
            // A string resource may itself be a reference.
            return if v.starts_with('@') { self.text(&v, depth + 1) } else { Some(v) };
        }
        if raw.starts_with('@') || raw.starts_with('?') {
            return None;
        }
        Some(raw.to_owned())
    }

    /// `#hex`, `@color/x`, `@android:color/white`, `?attr/colorPrimary`.
    /// A `@color/x` that is a state list (`res/color/x.xml`) resolves to its
    /// default state, the one Android shows when a view is enabled and unfocused.
    pub fn color(&self, raw: &str, depth: usize) -> Option<String> {
        let raw = raw.trim();
        if raw == "@null" {
            return None;
        }
        if raw.starts_with('#') {
            return color_css(raw);
        }
        if raw.starts_with('?') {
            return theme_attr_css(raw).map(str::to_owned);
        }
        if let Some(rest) = raw.strip_prefix("@android:color/") {
            return android_color(rest).map(color_from_aarrggbb);
        }
        if let Some(name) = raw.strip_prefix("@color/") {
            if depth > 8 {
                return None;
            }
            if let Some(v) = self.colors.get(name) {
                let v = v.clone();
                return self.color(&v, depth + 1);
            }
            return self.color_state_list(name, depth + 1);
        }
        None
    }

    /// `res/color/<name>.xml`: a `<selector>` (or a bare `<color>` alias file).
    fn color_state_list(&self, name: &str, depth: usize) -> Option<String> {
        if depth > 8 {
            return None;
        }
        let text = self.color_dirs.iter().find_map(|d| self.source.read(&format!("{d}/{name}.xml")))?;
        let doc = xml::parse(&text).ok()?;
        let root = doc.root?;
        if root.name_local() == "color" {
            return self.color(&root.direct_text(), depth + 1);
        }
        let items: Vec<&xml::Element> = root.elements().filter(|e| e.name_local() == "item").collect();
        // The default state has no `android:state_*` filter; otherwise the first
        // item (usually `state_pressed`) is the closest thing to a default.
        let chosen = items
            .iter()
            .find(|e| !e.attrs.iter().any(|(n, _)| n.contains("state_")))
            .or_else(|| items.first())?;
        if let Some(shape) = chosen.elements().find(|e| e.name_local() == "shape") {
            return parse_shape_with(&reserialize(shape), Some(self)).and_then(|s| s.fill);
        }
        let value = chosen.local("color")?;
        self.color(value, depth + 1)
    }

    /// `14dp`, `@dimen/x` → px.
    pub fn dimen(&self, raw: &str, depth: usize) -> Option<f32> {
        let raw = raw.trim();
        if let Some(name) = raw.strip_prefix("@dimen/") {
            if depth > 8 {
                return None;
            }
            let v = self.dimens.get(name)?.clone();
            return self.dimen(&v, depth + 1);
        }
        if let Some(name) = raw.strip_prefix("@android:dimen/") {
            return android_dimen(name);
        }
        dimen_px(raw)
    }

    /// `dimen` plus theme attributes (`?attr/actionBarSize`, or a dimen the
    /// active theme defines itself).
    pub fn dimen_in_theme(&self, raw: &str, theme: &[(String, String)], depth: usize) -> Option<f32> {
        let raw = raw.trim();
        if raw.starts_with('?') {
            let name = raw.rsplit('/').next().unwrap_or(raw).trim_start_matches("attr/");
            if depth <= 8
                && let Some(v) = theme.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone())
            {
                return self.dimen_in_theme(&v, theme, depth + 1);
            }
            return theme_attr_dimen(raw);
        }
        self.dimen(raw, depth)
    }

    pub fn boolean(&self, raw: &str) -> Option<bool> {
        if let Some(name) = raw.strip_prefix("@bool/") {
            return self.bools.get(name).copied();
        }
        parse_bool(raw)
    }

    #[cfg(test)]
    pub fn integer(&self, raw: &str) -> Option<i64> {
        if let Some(name) = raw.strip_prefix("@integer/") {
            return self.integers.get(name).copied();
        }
        raw.trim().parse().ok()
    }

    /// Whether a layout name is available for `include` (`res/layout*/name.xml`).
    #[cfg(test)]
    pub fn has_layout(&self, name: &str) -> bool {
        self.layout_dirs.iter().any(|d| self.source.read(&format!("{d}/{name}.xml")).is_some())
    }

    /// Reads and returns the text of `res/layout*/<name>.xml` (for includes).
    pub fn layout_xml(&self, name: &str) -> Option<String> {
        self.layout_dirs.iter().find_map(|d| self.source.read(&format!("{d}/{name}.xml")))
    }

    /// Parses `res/drawable*/<name>.xml` when it resolves to a `<shape>`,
    /// looking through `<selector>`/`<ripple>`/`<layer-list>`/`<inset>` wrappers.
    #[cfg(test)]
    pub fn drawable_shape(&self, name: &str) -> Option<ShapeDrawable> {
        match self.drawable(name) {
            Some(ResDrawable::Shape(shape)) => Some(shape),
            _ => None,
        }
    }

    /// `?attr/x` looked up in the active theme (the `android:theme` style
    /// chain); unknown attributes fall back to the built-in M3 palette.
    pub fn color_in_theme(&self, raw: &str, theme: &[(String, String)], depth: usize) -> Option<String> {
        let raw = raw.trim();
        if raw.starts_with('?') {
            let name = raw.rsplit('/').next().unwrap_or(raw).trim_start_matches("attr/");
            if depth <= 8 {
                if let Some(v) = theme.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()) {
                    return self.color_in_theme(&v, theme, depth + 1);
                }
            }
            return theme_attr_css(raw).map(str::to_owned);
        }
        self.color(raw, depth)
    }

    /// All items of a theme (`android:` and custom attributes alike) —
    /// `style_attrs` on the theme's style chain.
    pub fn theme_attrs(&self, raw: &str) -> Vec<(String, String)> {
        self.style_attrs(raw)
    }

    /// `@drawable/x` / `@mipmap/x` → shape, vector (as inline SVG) or bitmap
    /// (as a `data:` URI). Searches `drawable*` then `mipmap*` directories.
    pub fn drawable(&self, name: &str) -> Option<ResDrawable> {
        self.drawable_at(name, 0)
    }

    fn drawable_at(&self, name: &str, depth: usize) -> Option<ResDrawable> {
        if depth > 4 {
            return None;
        }
        for dirs in [&self.drawable_dirs, &self.mipmap_dirs] {
            for dir in dirs {
                if let Some(text) = self.source.read(&format!("{dir}/{name}.xml"))
                    && let Some(root) = xml::parse(&text).ok().and_then(|d| d.root)
                    && let Some(out) = self.drawable_element(&root, depth)
                {
                    return Some(out);
                }
                for ext in ["png", "webp", "jpg", "jpeg", "gif"] {
                    if let Some(bytes) = self.source.read_bytes(&format!("{dir}/{name}.{ext}"))
                        && let Some(uri) = data_uri(&bytes, ext)
                    {
                        return Some(ResDrawable::Image(uri));
                    }
                }
            }
        }
        None
    }

    /// One drawable element. Wrappers are unwrapped the way Android inflates
    /// them: exactly one drawable comes out, so a preview shows the default
    /// state of a selector and the topmost layer of a layer list.
    fn drawable_element(&self, root: &xml::Element, depth: usize) -> Option<ResDrawable> {
        if depth > 4 {
            return None;
        }
        match root.name_local() {
            "vector" => self.vector_svg(root).map(ResDrawable::Svg),
            "shape" => parse_shape_element(root, Some(self)).map(ResDrawable::Shape),
            "ripple" | "selector" | "level-list" | "transition" | "animated-vector" | "animation-list" => {
                self.default_item(root, depth)
            }
            "layer-list" => {
                let items: Vec<&xml::Element> = root.elements().filter(|e| e.name_local() == "item").collect();
                items.iter().rev().find_map(|it| self.item_drawable(it, depth + 1))
            }
            "bitmap" | "scale" | "clip" | "rotate" | "inset" => {
                let mut out = root.local("drawable").and_then(|d| self.drawable_ref(d, depth + 1))
                    .or_else(|| root.elements().find_map(|e| self.drawable_element(e, depth + 1)));
                if root.name_local() == "inset"
                    && let Some(ResDrawable::Shape(shape)) = &mut out
                {
                    for (attr, idx) in [("insetLeft", 0), ("insetTop", 1), ("insetRight", 2), ("insetBottom", 3)] {
                        if let Some(v) = root.local(attr).and_then(dimen_px) {
                            shape.padding[idx] += v;
                        }
                    }
                }
                out
            }
            "adaptive-icon" => {
                let layer = |tag: &str| {
                    root.elements()
                        .find(|e| e.name_local() == tag)
                        .and_then(|e| self.item_drawable(e, depth + 1))
                };
                // The foreground carries the recognisable artwork; a colour-only
                // background still beats nothing.
                layer("foreground").or_else(|| layer("background"))
            }
            _ => None,
        }
    }

    /// The drawable of the default `<item>` (the one without state filters).
    fn default_item(&self, root: &xml::Element, depth: usize) -> Option<ResDrawable> {
        let items: Vec<&xml::Element> = root.elements().filter(|e| e.name_local() == "item").collect();
        let chosen = items
            .iter()
            .find(|e| !e.attrs.iter().any(|(n, _)| n.contains("state_")))
            .or_else(|| items.first())?;
        self.item_drawable(chosen, depth)
    }

    fn item_drawable(&self, item: &xml::Element, depth: usize) -> Option<ResDrawable> {
        if let Some(d) = item.local("drawable") {
            return self.drawable_ref(d, depth + 1);
        }
        item.elements().find_map(|e| self.drawable_element(e, depth + 1))
    }

    /// A drawable reference: `@drawable/x`, `@mipmap/x`, or a bare colour, which
    /// Android accepts anywhere a drawable is expected (it paints a solid box).
    pub fn drawable_ref(&self, raw: &str, depth: usize) -> Option<ResDrawable> {
        let raw = raw.trim();
        if let Some(name) = raw.strip_prefix("@drawable/").or_else(|| raw.strip_prefix("@mipmap/")) {
            return self.drawable_at(name, depth);
        }
        if raw == "@null" {
            return None;
        }
        self.color(raw, depth)
            .map(|c| ResDrawable::Shape(ShapeDrawable { fill: Some(c), ..Default::default() }))
    }

    /// Items of `res/menu/<name>.xml` (`app:menu` on toolbars, bottom bars and
    /// navigation views), titles resolved and submenus flattened.
    pub fn menu_items(&self, raw: &str) -> Vec<MenuItem> {
        let name = raw.trim().trim_start_matches("@menu/");
        let mut out = Vec::new();
        if let Some(text) = self.menu_dirs.iter().find_map(|d| self.source.read(&format!("{d}/{name}.xml")))
            && let Some(root) = xml::parse(&text).ok().and_then(|d| d.root)
        {
            self.menu_walk(&root, &mut out, 0);
        }
        out
    }

    fn menu_walk(&self, el: &xml::Element, out: &mut Vec<MenuItem>, depth: usize) {
        if depth > 4 || out.len() >= 12 {
            return;
        }
        for child in el.elements() {
            match child.name_local() {
                "group" => self.menu_walk(child, out, depth + 1),
                "item" => {
                    if let Some(sub) = child.elements().find(|e| e.name_local() == "menu") {
                        self.menu_walk(sub, out, depth + 1);
                        continue;
                    }
                    let title = child
                        .local("title")
                        .map(|t| self.text(t, 0).unwrap_or_else(|| t.to_owned()))
                        .unwrap_or_default();
                    let icon = child.local("icon").and_then(|i| self.drawable_ref(i, 0));
                    if !title.is_empty() || icon.is_some() {
                        out.push(MenuItem { title, icon });
                    }
                }
                _ => {}
            }
        }
    }

    /// Superclass of a project class (`com.foo.MyView`), read from the module's
    /// Kotlin/Java sources. Library classes have no source here, so they return
    /// `None` and fall back to the name-suffix heuristic.
    pub fn class_parent(&self, fqn: &str) -> Option<String> {
        if let Some(hit) = self.parents.borrow().get(fqn) {
            return hit.clone();
        }
        let found = self.find_class_source(fqn).and_then(|(text, simple)| parse_class_parent(&text, &simple));
        self.parents.borrow_mut().insert(fqn.to_owned(), found.clone());
        found
    }

    /// Locates the source declaring `fqn`: its conventional path first, then any
    /// file of the package directory that mentions the class.
    fn find_class_source(&self, fqn: &str) -> Option<(String, String)> {
        let simple = fqn.rsplit('.').next()?.to_owned();
        let rel = fqn.replace('.', "/");
        let pkg_dir = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("").to_owned();
        const SRC_DIRS: [&str; 4] = ["src/main/java", "src/main/kotlin", "src/java", "src/kotlin"];
        let mut roots: Vec<String> = Vec::new();
        let module = self.source.module();
        if !module.is_empty() {
            roots.push(module.to_owned());
        }
        roots.push(String::new());
        for entry in self.source.list_project("") {
            if !roots.contains(&entry) {
                roots.push(entry);
            }
        }
        for root in roots.into_iter().take(24) {
            let prefix = if root.is_empty() { String::new() } else { format!("{root}/") };
            for src in SRC_DIRS {
                for ext in ["kt", "java"] {
                    if let Some(text) = self.source.read_project(&format!("{prefix}{src}/{rel}.{ext}")) {
                        return Some((text, simple.clone()));
                    }
                }
                if pkg_dir.is_empty() {
                    continue;
                }
                let dir = format!("{prefix}{src}/{pkg_dir}");
                for entry in self.source.list_project(&dir).into_iter().take(32) {
                    if !entry.ends_with(".kt") && !entry.ends_with(".java") {
                        continue;
                    }
                    if let Some(text) = self.source.read_project(&format!("{dir}/{entry}"))
                        && mentions_class(&text, &simple)
                    {
                        return Some((text, simple.clone()));
                    }
                }
            }
        }
        None
    }

    /// `<vector>` → an inline SVG (paths and groups only; fills resolved).
    fn vector_svg(&self, root: &xml::Element) -> Option<String> {
        if root.name_local() != "vector" {
            return None;
        }
        let vw = root.attr("android:viewportWidth").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(24.0);
        let vh = root.attr("android:viewportHeight").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(24.0);
        let mut body = String::new();
        vector_paths(root, &mut body, self);
        if body.is_empty() {
            return None;
        }
        Some(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {vw} {vh}\">{body}</svg>"))
    }

    /// Merged `android:` items of `@style/X` (ancestors first, child overrides
    /// keeping the ancestor's slot order).
    pub fn style_attrs(&self, raw: &str) -> Vec<(String, String)> {
        let mut chain = Vec::new();
        self.collect_style_chain(&style_ref_name(raw), 0, &mut chain);
        let mut out: Vec<(String, String)> = Vec::new();
        for (name, value) in chain {
            if let Some(slot) = out.iter_mut().find(|(n, _)| *n == name) {
                slot.1 = value;
            } else {
                out.push((name, value));
            }
        }
        out
    }

    fn collect_style_chain(&self, name: &str, depth: usize, out: &mut Vec<(String, String)>) {
        if depth > 8 {
            return;
        }
        let Some(style) = self.styles.get(name) else { return };
        // Explicit parent wins over the implicit dotted-name parent.
        let parent = style.parent.clone().or_else(|| name.rsplit_once('.').map(|(p, _)| p.to_owned()));
        if let Some(parent) = parent {
            if !parent.is_empty() {
                self.collect_style_chain(&parent, depth + 1, out);
            }
        }
        out.extend(style.items.iter().cloned());
    }

    /// Style names of `@style/X`, ancestors first and `X` last. Library styles
    /// contribute their name even though their items live in an AAR, which is
    /// how `Widget.Material3.Button.OutlinedButton` can be recognised without
    /// the Material resources being available.
    pub fn style_chain_names(&self, raw: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_style_names(&style_ref_name(raw), 0, &mut out);
        out
    }

    fn collect_style_names(&self, name: &str, depth: usize, out: &mut Vec<String>) {
        if depth > 8 || name.is_empty() {
            return;
        }
        let parent = match self.styles.get(name) {
            Some(style) => style.parent.clone().or_else(|| name.rsplit_once('.').map(|(p, _)| p.to_owned())),
            None => name.rsplit_once('.').map(|(p, _)| p.to_owned()),
        };
        if let Some(parent) = parent
            && !parent.is_empty()
        {
            self.collect_style_names(&parent, depth + 1, out);
        }
        out.push(name.to_owned());
    }
}

/// Themes of `AndroidManifest.xml`: `<application android:theme>` plus every
/// `<activity android:name android:theme>`.
pub fn parse_manifest(xml_text: &str) -> ManifestThemes {
    let mut out = ManifestThemes::default();
    let Ok(doc) = xml::parse(xml_text) else { return out };
    let Some(root) = doc.root else { return out };
    if root.name_local() != "manifest" {
        return out;
    }
    out.package = root.local("package").unwrap_or_default().to_owned();
    for child in root.elements() {
        if child.name_local() != "application" {
            continue;
        }
        if let Some(theme) = child.local("theme") {
            out.application = Some(theme.to_owned());
        }
        for inner in child.elements() {
            if !matches!(inner.name_local(), "activity" | "activity-alias") {
                continue;
            }
            let (Some(name), Some(theme)) = (inner.local("name"), inner.local("theme")) else { continue };
            out.activities.push((expand_class_name(name, &out.package), theme.to_owned()));
        }
    }
    out
}

/// `.MainActivity` → `com.example.MainActivity` (the manifest's shorthand).
fn expand_class_name(name: &str, package: &str) -> String {
    if name.starts_with('.') {
        format!("{package}{name}")
    } else if !name.contains('.') && !package.is_empty() {
        format!("{package}.{name}")
    } else {
        name.to_owned()
    }
}

/// Decodes the escapes Android applies to `<string>` values (`\n`, `\t`,
/// `\\`, `\'`, `\"`, `\uXXXX`) and strips a surrounding pair of quotes.
pub fn decode_escapes(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let next = chars[i + 1];
        match next {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'u' if i + 5 < chars.len() => {
                let hex: String = chars[i + 2..i + 6].iter().collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => {
                        out.push(c);
                        i += 6;
                        continue;
                    }
                    None => out.push(next),
                }
            }
            other => out.push(other),
        }
        i += 2;
    }
    let trimmed = out.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_owned()
    } else {
        out
    }
}

/// Whether `text` declares `class <simple>` (a cheap filter before parsing).
fn mentions_class(text: &str, simple: &str) -> bool {
    text.contains(&format!("class {simple}"))
}

/// The superclass of `class <simple>` in a Kotlin or Java source file, as a
/// fully qualified name (imports and the file's package resolve a bare name).
/// Returns `None` when the class has no declared superclass — which for a
/// custom view means the source was not found in this project at all.
pub fn parse_class_parent(text: &str, simple: &str) -> Option<String> {
    let needle = format!("class {simple}");
    // Word boundary: `class Button` must not match `class ButtonBar`.
    let at = text.match_indices(&needle).find_map(|(i, _)| {
        let next = text[i + needle.len()..].chars().next();
        match next {
            Some(c) if c.is_alphanumeric() || c == '_' => None,
            _ => Some(i),
        }
    })?;
    let after = &text[at + needle.len()..];
    let imports = imports_of(text);
    let package = text.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("package ")?.trim_end_matches(|c| c == ';' || c == '{').trim().split(';').next().map(str::to_owned)
    });
    let parent = kotlin_supertype(after).or_else(|| java_supertype(after))?;
    let parent = parent.trim().trim_end_matches('{').trim();
    if parent.is_empty() {
        return None;
    }
    Some(qualify(parent, &imports, package.as_deref()))
}

/// `(...)  : Base(args), Iface { }` → `Base(args)` (Kotlin).
fn kotlin_supertype(after: &str) -> Option<String> {
    let chars: Vec<char> = after.chars().collect();
    let mut i = 0;
    let mut depth = 0i32;
    while i < chars.len() {
        match chars[i] {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            '{' if depth <= 0 => return None,
            ':' if depth <= 0 => {
                let rest: String = chars[i + 1..].iter().collect();
                return first_supertype(&rest);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `... extends Base<Iface> implements X {` → `Base` (Java).
fn java_supertype(after: &str) -> Option<String> {
    let end = after.find('{').unwrap_or(after.len());
    let head = &after[..end];
    let at = head.find(" extends ")?;
    let rest = &head[at + " extends ".len()..];
    let token: String = rest
        .trim()
        .chars()
        .take_while(|c| !matches!(c, '<' | ',' | ' ' | '\t' | '\n'))
        .collect();
    if token.is_empty() { None } else { Some(token) }
}

/// The first entry of a Kotlin supertype list, without generics or constructor
/// arguments (`Base(args)` → `Base`).
fn first_supertype(rest: &str) -> Option<String> {
    let mut depth = 0i32;
    let mut out = String::new();
    for c in rest.chars() {
        match c {
            '<' | '(' | '[' => {
                depth += 1;
                continue;
            }
            '>' | ')' | ']' => {
                depth -= 1;
                continue;
            }
            ',' | '{' if depth <= 0 => break,
            _ => {}
        }
        if depth <= 0 {
            out.push(c);
        }
    }
    let out = out.trim().to_owned();
    if out.is_empty() { None } else { Some(out) }
}

/// `import a.b.C` / `import a.b.C as D` / `import a.b.*` → simple name → FQN.
fn imports_of(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("import ") else { continue };
        let rest = rest.trim_end_matches(';').trim();
        let (path, alias) = match rest.split_once(" as ") {
            Some((p, a)) => (p.trim(), a.trim()),
            None => (rest, ""),
        };
        if path.ends_with(".*") {
            continue;
        }
        let key = if alias.is_empty() { path.rsplit('.').next().unwrap_or(path) } else { alias };
        map.insert(key.to_owned(), path.to_owned());
    }
    map
}

/// Resolves a bare superclass name through the file's imports, else its package.
fn qualify(name: &str, imports: &HashMap<String, String>, package: Option<&str>) -> String {
    if name.contains('.') {
        return name.to_owned();
    }
    if let Some(fqn) = imports.get(name) {
        return fqn.clone();
    }
    match package {
        Some(p) if !p.is_empty() => format!("{p}.{name}"),
        _ => name.to_owned(),
    }
}

fn color_from_aarrggbb(v: &str) -> String {
    color_css(v).unwrap_or_else(|| "#000000".to_owned())
}

/// API level of a `values` directory the default preview applies, `None` for
/// other configurations (`values-night`, `values-fr`, `values-w600dp`, …).
pub fn values_dir_version(dir: &str) -> Option<u32> {
    let mut parts = dir.split('-');
    if parts.next() != Some("values") {
        return None;
    }
    let mut version = 0;
    for qualifier in parts {
        let level = qualifier.strip_prefix('v')?;
        if level.is_empty() || !level.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        version = version.max(level.parse().ok()?);
    }
    Some(version)
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// `"@style/AppTheme.Red"` / `"@android:style/Theme"` / `"AppTheme.Red"` → the style name.
fn style_ref_name(raw: &str) -> String {
    raw.trim().trim_start_matches("@android:style/").trim_start_matches("@style/").to_owned()
}

/// Parses a shape drawable XML subset: `<shape>` with `<solid>`, `<gradient>`,
/// `<corners>`, `<stroke>`, `<padding>` and `<size>`. Wrapper elements
/// (`<ripple>`, `<layer-list>`, `<inset>`, `<selector>`) yield the first nested
/// shape they contain. `@color/…` references resolve through `res`; `?attr/…`
/// is kept verbatim for the caller to resolve against the inflating theme
/// ([`ShapeDrawable::resolve_theme`]).
pub fn parse_shape_with(xml_text: &str, res: Option<&Resources>) -> Option<ShapeDrawable> {
    let doc = xml::parse(xml_text).ok()?;
    let root = doc.root?;
    parse_shape_element(&root, res)
}

/// [`parse_shape_with`] for an already-parsed element.
pub fn parse_shape_element(root: &xml::Element, res: Option<&Resources>) -> Option<ShapeDrawable> {
    let color_of = |v: &str| -> Option<String> {
        if v.trim().starts_with('?') {
            return Some(v.trim().to_owned());
        }
        match res {
            Some(r) => r.color(v, 0),
            None => color_css(v),
        }
    };
    match root.name_local() {
        "shape" => {}
        "ripple" | "layer-list" | "inset" | "selector" | "level-list" => {
            // Fall back to the first nested shape: `res`-backed callers resolve
            // wrappers properly (`Resources::drawable_element`), this is for
            // standalone parsing.
            return root.elements().find_map(|el| {
                if el.name_local() == "shape" {
                    parse_shape_element(el, res)
                } else {
                    el.elements().find_map(|inner| parse_shape_element(inner, res))
                }
            });
        }
        _ => return None,
    }
    let mut shape = ShapeDrawable { corners: [0.0; 4], padding: [0.0; 4], ..Default::default() };
    for el in root.elements() {
        match el.name_local() {
            "solid" => {
                if let Some(c) = el.local("color").and_then(|c| color_of(c)) {
                    shape.fill = Some(c);
                }
            }
            "corners" => {
                if let Some(r) = el.local("radius").and_then(dimen_px) {
                    shape.corners = [r; 4];
                }
                for (attr, idx) in
                    [("topLeftRadius", 0), ("topRightRadius", 1), ("bottomRightRadius", 2), ("bottomLeftRadius", 3)]
                {
                    if let Some(v) = el.local(attr).and_then(dimen_px) {
                        shape.corners[idx] = v;
                    }
                }
            }
            "stroke" => {
                let w = el.local("width").and_then(dimen_px).unwrap_or(1.0);
                let c = el.local("color").and_then(|c| color_of(c)).unwrap_or_else(|| "#000000".to_owned());
                shape.stroke = Some((w, c));
            }
            "padding" => {
                shape.padding[0] = el.local("left").and_then(dimen_px).unwrap_or(0.0);
                shape.padding[1] = el.local("top").and_then(dimen_px).unwrap_or(0.0);
                shape.padding[2] = el.local("right").and_then(dimen_px).unwrap_or(0.0);
                shape.padding[3] = el.local("bottom").and_then(dimen_px).unwrap_or(0.0);
            }
            "size" => {
                let w = el.local("width").and_then(dimen_px);
                let h = el.local("height").and_then(dimen_px);
                if w.is_some() || h.is_some() {
                    shape.size = Some((w.unwrap_or(0.0), h.unwrap_or(0.0)));
                }
            }
            "gradient" => {
                let mut colors = Vec::new();
                for attr in ["startColor", "centerColor", "endColor"] {
                    if let Some(c) = el.local(attr).and_then(|c| color_of(c)) {
                        colors.push(c);
                    }
                }
                if colors.len() >= 2 {
                    shape.gradient = Some(Gradient {
                        kind: match el.local("type").map(str::trim) {
                            Some("radial") => GradientKind::Radial,
                            Some("sweep") => GradientKind::Sweep,
                            _ => GradientKind::Linear,
                        },
                        angle: el.local("angle").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0),
                        colors,
                        center: match (
                            el.local("centerX").and_then(|v| v.trim().parse::<f32>().ok()),
                            el.local("centerY").and_then(|v| v.trim().parse::<f32>().ok()),
                        ) {
                            (Some(x), Some(y)) => Some((x, y)),
                            _ => None,
                        },
                        radius: el.local("gradientRadius").and_then(dimen_px),
                    });
                } else if shape.fill.is_none() {
                    shape.fill = colors.into_iter().next();
                }
            }
            _ => {}
        }
    }
    Some(shape)
}

/// Serializes a single element back to XML so it can go through `parse_shape`.
fn reserialize(el: &xml::Element) -> String {
    let mut out = format!("<{}", el.name);
    for (n, v) in &el.attrs {
        out.push_str(&format!(" {n}=\"{}\"", v.replace('"', "&quot;")));
    }
    out.push('>');
    for c in &el.children {
        match c {
            xml::Child::Element(e) => out.push_str(&reserialize(e)),
            xml::Child::Text(t) => out.push_str(t),
        }
    }
    out.push_str(&format!("</{}>", el.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(src: MemorySource) -> Resources<'static> {
        Resources::new(Box::leak(Box::new(src)))
    }

    fn values_res() -> Resources<'static> {
        // Leaked for convenience in tests; fine for the test binary.
        let src: &'static MemorySource = Box::leak(Box::new(
            MemorySource::new()
                .dir("", &["values", "values-night", "layout", "drawable"])
                .dir("values", &["strings.xml", "colors.xml", "dimens.xml", "styles.xml"])
                .dir("layout", &["activity_main.xml", "item_row.xml"])
                .dir("drawable", &["bg_card.xml", "photo.jpg"])
                .file("layout/activity_main.xml", "<LinearLayout/>")
                .file("layout/item_row.xml", "<TextView/>")
                .file(
                    "values/strings.xml",
                    r#"<resources>
                        <string name="app_name">My App</string>
                        <string name="greeting">Hello, %1$s!</string>
                        <string name="chained">@string/app_name</string>
                        <string name="rich">Hi <b>there</b></string>
                    </resources>"#,
                )
                .file(
                    "values/colors.xml",
                    r#"<resources>
                        <color name="brand">#0F0</color>
                        <color name="brand_opaque">#FF00FF00</color>
                        <color name="scrim">#80000000</color>
                        <color name="alias">@color/brand_opaque</color>
                        <color name="themish">?attr/colorPrimary</color>
                    </resources>"#,
                )
                .file(
                    "values/dimens.xml",
                    r#"<resources>
                        <dimen name="space_sm">8dp</dimen>
                        <dimen name="space_md">@dimen/space_sm</dimen>
                        <dimen name="text_md">14sp</dimen>
                        <dimen name="point">12pt</dimen>
                    </resources>"#,
                )
                .file(
                    "values/styles.xml",
                    r#"<resources>
                        <style name="Base" parent="">
                            <item name="android:textSize">14sp</item>
                            <item name="android:textColor">#111111</item>
                        </style>
                        <style name="Base.Red">
                            <item name="android:textColor">#CC0000</item>
                            <item name="android:background">@color/brand_opaque</item>
                        </style>
                        <style name="Title" parent="Base.Red">
                            <item name="android:textStyle">bold</item>
                        </style>
                    </resources>"#,
                )
                .file(
                    "drawable/bg_card.xml",
                    r##"<shape xmlns:android="http://schemas.android.com/apk/res/android">
                        <solid android:color="#FFEEDD"/>
                        <corners android:radius="12dp" android:topLeftRadius="2dp"/>
                        <stroke android:width="1dp" android:color="#FF0000"/>
                        <padding android:left="4dp" android:top="8dp"/>
                    </shape>"##,
                ),
        ));
        Resources::new(src)
    }

    #[test]
    fn color_literals_normalize() {
        assert_eq!(color_css("#0F0").as_deref(), Some("#00FF00"));
        assert_eq!(color_css("#FF00FF00").as_deref(), Some("#00FF00"));
        assert_eq!(color_css("#80000000").as_deref(), Some("rgba(0,0,0,0.502)"));
        assert_eq!(color_css("#30F").as_deref(), Some("#3300FF")); // argb
        assert_eq!(color_css("#GGGGGG"), None);
        assert_eq!(color_css("red"), None);
    }

    #[test]
    fn dimen_literals() {
        assert_eq!(dimen_px("14dp"), Some(14.0));
        assert_eq!(dimen_px("14sp"), Some(14.0));
        assert_eq!(dimen_px("0dip"), Some(0.0));
        assert_eq!(dimen_px("12pt"), Some(16.0));
        assert_eq!(dimen_px("1.5dp"), Some(1.5));
        assert_eq!(dimen_px("2mm"), Some(2.0 * 160.0 / 25.4));
        assert_eq!(dimen_px("banana"), None);
    }

    #[test]
    fn resolves_strings() {
        let r = values_res();
        assert_eq!(r.text("@string/app_name", 0).as_deref(), Some("My App"));
        assert_eq!(r.text("@string/chained", 0).as_deref(), Some("My App"));
        assert_eq!(r.text("@string/greeting", 0).as_deref(), Some("Hello, %1$s!"));
        assert_eq!(r.text("@string/rich", 0).as_deref(), Some("Hi there"));
        assert_eq!(r.text("@android:string/ok", 0).as_deref(), Some("OK"));
        assert_eq!(r.text("Literal text", 0).as_deref(), Some("Literal text"));
        assert_eq!(r.text("@string/missing", 0), None);
        assert_eq!(r.text("@color/brand", 0), None);
    }

    #[test]
    fn resolves_colors() {
        let r = values_res();
        assert_eq!(r.color("#FF0000", 0).as_deref(), Some("#FF0000"));
        assert_eq!(r.color("@color/brand", 0).as_deref(), Some("#00FF00"));
        assert_eq!(r.color("@color/alias", 0).as_deref(), Some("#00FF00"));
        assert_eq!(r.color("@color/scrims", 0), None);
        assert_eq!(r.color("@color/scrim", 0).as_deref(), Some("rgba(0,0,0,0.502)"));
        assert_eq!(r.color("@android:color/white", 0).as_deref(), Some("#FFFFFF"));
        assert_eq!(r.color("@android:color/transparent", 0).as_deref(), Some("rgba(0,0,0,0.000)"));
        assert_eq!(r.color("?attr/colorPrimary", 0).as_deref(), Some(M3_PRIMARY));
        assert_eq!(r.color("?android:attr/textColorSecondary", 0).as_deref(), Some(M3_ON_SURFACE_VARIANT));
        assert_eq!(r.color("@color/themish", 0).as_deref(), Some(M3_PRIMARY));
    }

    #[test]
    fn resolves_dimens() {
        let r = values_res();
        assert_eq!(r.dimen("@dimen/space_sm", 0), Some(8.0));
        assert_eq!(r.dimen("@dimen/space_md", 0), Some(8.0));
        assert_eq!(r.dimen("@dimen/text_md", 0), Some(14.0));
        assert_eq!(r.dimen("@dimen/point", 0), Some(16.0));
        assert_eq!(r.dimen("24dp", 0), Some(24.0));
        assert_eq!(r.dimen("@dimen/nope", 0), None);
    }

    #[test]
    fn resolves_bools_and_integers() {
        let src = MemorySource::new().file(
            "values/x.xml",
            r#"<resources><bool name="on">true</bool><integer name="n">42</integer></resources>"#,
        );
        let r = Resources::new(&src);
        assert_eq!(r.boolean("@bool/on"), Some(true));
        assert_eq!(r.boolean("false"), Some(false));
        assert_eq!(r.integer("@integer/n"), Some(42));
    }

    #[test]
    fn style_chain_merges_with_overrides() {
        let r = values_res();
        let attrs = r.style_attrs("@style/Title");
        let get = |name: &str| attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        // From Base (grandparent):
        assert_eq!(get("android:textSize"), Some("14sp"));
        // Overridden in Base.Red:
        assert_eq!(get("android:textColor"), Some("#CC0000"));
        // Own item:
        assert_eq!(get("android:textStyle"), Some("bold"));
        // Attribute from the child-most style wins; order is ancestors-first.
        assert_eq!(attrs[0].0, "android:textSize");
    }

    #[test]
    fn explicit_parent_wins_over_dotted_name() {
        let src = MemorySource::new().file(
            "values/x.xml",
            r#"<resources>
                <style name="A"><item name="android:textColor">#111111</item></style>
                <style name="B"><item name="android:textColor">#222222</item></style>
                <style name="A.B" parent="B"><item name="android:textSize">16sp</item></style>
            </resources>"#,
        );
        let r = Resources::new(&src);
        let attrs = r.style_attrs("@style/A.B");
        let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        // parent="B", not the dotted A:
        assert_eq!(get("android:textColor"), Some("#222222"));
        assert_eq!(get("android:textSize"), Some("16sp"));
    }

    #[test]
    fn layout_and_shape_lookups() {
        let r = values_res();
        assert!(r.has_layout("activity_main"));
        assert!(!r.has_layout("nope"));
        assert!(r.layout_xml("item_row").is_some());
        let shape = r.drawable_shape("bg_card").unwrap();
        assert_eq!(shape.fill.as_deref(), Some("#FFEEDD"));
        assert_eq!(shape.corners[0], 2.0); // topLeft overrides radius
        assert_eq!(shape.corners[1], 12.0);
        assert_eq!(shape.stroke, Some((1.0, "#FF0000".to_owned())));
        assert_eq!(shape.padding[0], 4.0);
        assert_eq!(shape.padding[1], 8.0);
        assert_eq!(r.drawable_shape("photo"), None); // not xml
    }

    #[test]
    fn malformed_values_file_is_tolerated() {
        let src = MemorySource::new().dir("values", &["bad.xml"]).file("values/bad.xml", "<resources><string name='x'");
        let mut r = Resources::new(&src);
        r.load_values("<resources><string name=\"y\">ok</string></resources>");
        assert_eq!(r.text("@string/y", 0).as_deref(), Some("ok"));
        assert_eq!(r.text("@string/x", 0), None);
    }

    #[test]
    fn theme_attr_defaults() {
        assert_eq!(theme_attr_css("?attr/colorPrimary"), Some(M3_PRIMARY));
        assert_eq!(theme_attr_css("?android:attr/textColorPrimary"), Some(M3_ON_SURFACE));
        assert_eq!(theme_attr_css("?colorOnSurfaceVariant"), Some(M3_ON_SURFACE_VARIANT));
        assert_eq!(theme_attr_css("?attr/nopeNope"), None);
        assert_eq!(theme_attr_dimen("?attr/actionBarSize"), Some(56.0));
        assert_eq!(android_dimen("status_bar_height"), Some(24.0));
    }

    #[test]
    fn manifest_theme_and_activity_themes() {
        let src = MemorySource::new()
            .module("app")
            .project_file(
                "app/src/main/AndroidManifest.xml",
                r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.example.app">
                    <application android:theme="@style/Theme.App">
                        <activity android:name=".MainActivity" android:theme="@style/Theme.Main"/>
                        <activity android:name="com.example.app.OtherActivity"/>
                    </application>
                </manifest>"#,
            )
            .file(
                "values/themes.xml",
                r#"<resources>
                    <style name="Theme.App" parent="Theme.Material3.DayNight.NoActionBar">
                        <item name="colorPrimary">#FF123456</item>
                    </style>
                    <style name="Theme.Main" parent="Theme.App">
                        <item name="colorPrimary">#FF654321</item>
                    </style>
                </resources>"#,
            );
        let r = res(src);
        assert_eq!(r.app_theme(), Some("@style/Theme.App"));
        assert_eq!(r.activity_theme(".MainActivity"), Some("@style/Theme.Main"));
        assert_eq!(r.activity_theme("com.example.app.MainActivity"), Some("@style/Theme.Main"));
        assert_eq!(r.activity_theme("OtherActivity"), None);
        let get = |theme: &str, name: &str| {
            r.theme_attrs(theme).into_iter().find(|(n, _)| n == name).map(|(_, v)| v)
        };
        assert_eq!(get("@style/Theme.App", "colorPrimary").as_deref(), Some("#FF123456"));
        assert_eq!(get("@style/Theme.Main", "colorPrimary").as_deref(), Some("#FF654321"));
    }

    #[test]
    fn color_state_lists_use_the_default_state() {
        let src = MemorySource::new()
            .file(
                "color/text_primary.xml",
                r##"<selector>
                    <item android:state_pressed="true" android:color="#FFFF0000"/>
                    <item android:color="#FF222222"/>
                </selector>"##,
            )
            .file(
                "values/colors.xml",
                r##"<resources><color name="alias">@color/text_primary</color></resources>"##,
            );
        let r = res(src);
        assert_eq!(r.color("@color/text_primary", 0).as_deref(), Some("#222222"));
        assert_eq!(r.color("@color/alias", 0).as_deref(), Some("#222222"));
    }

    #[test]
    fn selectors_gradients_and_adaptive_icons_resolve() {
        let src = MemorySource::new()
            .file(
                "drawable/btn.xml",
                r##"<selector>
                    <item android:state_enabled="false" android:drawable="@drawable/bg_grad"/>
                    <item android:drawable="@drawable/bg_grad"/>
                </selector>"##,
            )
            .file(
                "drawable/bg_grad.xml",
                r##"<shape>
                    <gradient android:startColor="#FF0000" android:centerColor="#00FF00"
                        android:endColor="#0000FF" android:angle="90" android:type="linear"/>
                    <corners android:radius="8dp"/>
                </shape>"##,
            )
            .file(
                "drawable/ic_launcher.xml",
                r##"<adaptive-icon>
                    <background android:drawable="@color/ic_bg"/>
                    <foreground android:drawable="@drawable/bg_grad"/>
                </adaptive-icon>"##,
            )
            .file(
                "drawable/inset_bg.xml",
                r##"<inset android:insetLeft="4dp" android:drawable="@drawable/bg_grad"/>"##,
            )
            .file("values/colors.xml", r##"<resources><color name="ic_bg">#FF00FF00</color></resources>"##);
        let r = res(src);
        // `<selector>` → default item → shape, gradient preserved.
        let shape = r.drawable_shape("btn").expect("shape through selector");
        let g = shape.gradient.as_ref().expect("gradient");
        assert_eq!(g.colors, ["#FF0000", "#00FF00", "#0000FF"]);
        assert_eq!(g.kind, GradientKind::Linear);
        assert_eq!(g.angle, 90.0);
        assert_eq!(shape.corners, [8.0; 4]);
        // `<inset>` adds padding around the drawable it wraps.
        let inset = r.drawable_shape("inset_bg").expect("inset");
        assert_eq!(inset.padding[0], 4.0);
        // Adaptive icons unwrap to the foreground artwork.
        let icon = r.drawable("ic_launcher").expect("adaptive icon");
        assert!(matches!(icon, ResDrawable::Shape(s) if s.gradient.is_some()));
    }

    #[test]
    fn menu_resources_resolve_titles_and_icons() {
        let src = MemorySource::new()
            .file(
                "values/strings.xml",
                r#"<resources><string name="save">Save</string></resources>"#,
            )
            .file(
                "menu/main.xml",
                r##"<menu xmlns:android="http://schemas.android.com/apk/res/android">
                    <item android:id="@+id/save" android:title="@string/save" android:icon="@drawable/ic_add"/>
                    <group><item android:title="Delete"/></group>
                    <item android:title="More"><menu><item android:title="Settings"/></menu></item>
                </menu>"##,
            )
            .file(
                "drawable/ic_add.xml",
                r##"<vector android:viewportWidth="24" android:viewportHeight="24"><path android:pathData="M0,0h24v24h-24z" android:fillColor="#FF000000"/></vector>"##,
            );
        let r = res(src);
        let items = r.menu_items("@menu/main");
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Save", "Delete", "Settings"]);
        assert!(matches!(items[0].icon, Some(ResDrawable::Svg(_))));
    }

    #[test]
    fn custom_class_superclasses_come_from_project_sources() {
        let src = MemorySource::new()
            .module("app")
            .project_file(
                "app/src/main/java/com/example/ui/BrandButton.kt",
                r#"package com.example.ui

import androidx.appcompat.widget.AppCompatButton

class BrandButton(context: Context) : AppCompatButton(context) { }
"#,
            )
            .project_file(
                "app/src/main/java/com/example/ui/Widgets.kt",
                r#"package com.example.ui

class Panel : androidx.constraintlayout.widget.ConstraintLayout { }
"#,
            )
            .project_file(
                "app/src/main/java/com/example/ui/NoteRow.java",
                r#"package com.example.ui;

import android.widget.LinearLayout;

public class NoteRow extends LinearLayout implements Runnable {
    public void run() { }
}
"#,
            );
        let r = res(src);
        assert_eq!(
            r.class_parent("com.example.ui.BrandButton").as_deref(),
            Some("androidx.appcompat.widget.AppCompatButton")
        );
        // Declared in a differently-named file of the same package.
        assert_eq!(
            r.class_parent("com.example.ui.Panel").as_deref(),
            Some("androidx.constraintlayout.widget.ConstraintLayout")
        );
        assert_eq!(r.class_parent("com.example.ui.NoteRow").as_deref(), Some("android.widget.LinearLayout"));
        assert_eq!(r.class_parent("com.example.ui.Missing"), None);
    }

    #[test]
    fn class_parent_parsing_handles_generics_and_java() {
        let kotlin = "package a.b\nimport android.widget.FrameLayout\nclass Panel<T>(c: Context) : FrameLayout(c), Runnable { }";
        assert_eq!(parse_class_parent(kotlin, "Panel").as_deref(), Some("android.widget.FrameLayout"));
        let java = "package a.b;\nimport android.widget.FrameLayout;\npublic class Panel extends FrameLayout implements Runnable { }";
        assert_eq!(parse_class_parent(java, "Panel").as_deref(), Some("android.widget.FrameLayout"));
        // A different class with the same prefix must not match.
        assert_eq!(parse_class_parent("class PanelRow : View { }", "Panel"), None);
    }

    #[test]
    fn android_string_escapes_decode() {
        assert_eq!(decode_escapes(r"Line\nNext\tTab \\ \'q\'"), "Line\nNext\tTab \\ 'q'");
        assert_eq!(decode_escapes("\"quoted\""), "quoted");
        assert_eq!(decode_escapes("plain"), "plain");
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn only_default_configuration_values_are_loaded() {
        // Night and locale variants must not override the base tables: the
        // preview shows the default light configuration.
        assert_eq!(values_dir_version("values"), Some(0));
        assert_eq!(values_dir_version("values-v21"), Some(21));
        assert_eq!(values_dir_version("values-v21-v31"), Some(31));
        assert_eq!(values_dir_version("values-night"), None);
        assert_eq!(values_dir_version("values-night-v31"), None);
        assert_eq!(values_dir_version("values-vi"), None);
        assert_eq!(values_dir_version("values-w600dp"), None);
        assert_eq!(values_dir_version("layout"), None);
    }

    #[test]
    fn newer_api_values_win_and_night_is_ignored() {
        let src = MemorySource::new()
            .file("values/colors.xml", r##"<resources><color name="accent">#FF111111</color></resources>"##)
            .file("values-v21/colors.xml", r##"<resources><color name="accent">#FF222222</color></resources>"##)
            .file("values-night/colors.xml", r##"<resources><color name="accent">#FFFFFFFF</color></resources>"##);
        let res = Resources::new(&src);
        assert_eq!(res.color("@color/accent", 0).as_deref(), Some("#222222"));
    }
}
