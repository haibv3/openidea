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

use std::collections::HashMap;

use crate::xml;

/// Where resource files come from. Paths are relative to the `res/` root with
/// `/` separators (`values/strings.xml`, `layout/activity_main.xml`); `dir`
/// for [`ResourceSource::list`] is `""` for the root itself.
pub trait ResourceSource {
    fn read(&self, path: &str) -> Option<String>;
    fn list(&self, dir: &str) -> Vec<String>;
}

/// An in-memory source for tests. Directory listings are derived from the
/// registered file paths, so a test only needs to register files (extra empty
/// directories can be declared with [`MemorySource::dir`]).
pub struct MemorySource {
    pub files: HashMap<String, String>,
    dirs: HashMap<String, Vec<String>>,
}

impl MemorySource {
    pub fn new() -> Self {
        Self { files: HashMap::new(), dirs: HashMap::new() }
    }

    pub fn file(mut self, path: &str, content: &str) -> Self {
        self.files.insert(path.to_owned(), content.to_owned());
        self
    }

    pub fn dir(mut self, dir: &str, entries: &[&str]) -> Self {
        self.dirs.insert(dir.to_owned(), entries.iter().map(|s| s.to_string()).collect());
        self
    }
}

impl Default for MemorySource {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceSource for MemorySource {
    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
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
        "surfaceTint" => M3_PRIMARY,
        "inverseSurface" => "#303030",
        "inverseOnSurface" => "#F2F2F2",
        "surfaceDim" => "#DED8E1",
        "surfaceBright" => M3_SURFACE,
        "surfaceContainerLowest" => "#FFFFFF",
        "surfaceContainerLow" => "#F7F2FA",
        "surfaceContainerHigh" => "#ECE6F0",
        "surfaceContainerHighest" => "#E6E0E9",
        "colorControlHighlight" | "dividerHorizontal" | "dividerVertical" => "rgba(0,0,0,0.12)",
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
    /// `[top-left, top-right, bottom-right, bottom-left]` in px.
    pub corners: [f32; 4],
    pub stroke: Option<(f32, String)>,
    /// `[left, top, right, bottom]` in px.
    pub padding: [f32; 4],
}

struct StyleDef {
    parent: Option<String>,
    items: Vec<(String, String)>,
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
        };
        for entry in source.list("") {
            if entry.starts_with("values") {
                for file in source.list(&entry) {
                    if file.ends_with(".xml") {
                        if let Some(text) = source.read(&format!("{entry}/{file}")) {
                            res.load_values(&text);
                        }
                    }
                }
            } else if entry.starts_with("layout") {
                res.layout_dirs.push(entry);
            } else if entry.starts_with("drawable") {
                res.drawable_dirs.push(entry);
            }
        }
        res
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
                    self.strings.insert(name, el.text_content());
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
    pub fn color(&self, raw: &str, depth: usize) -> Option<String> {
        let raw = raw.trim();
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
            let v = self.colors.get(name)?.clone();
            return self.color(&v, depth + 1);
        }
        None
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
        dimen_px(raw)
    }

    pub fn boolean(&self, raw: &str) -> Option<bool> {
        if let Some(name) = raw.strip_prefix("@bool/") {
            return self.bools.get(name).copied();
        }
        parse_bool(raw)
    }

    pub fn integer(&self, raw: &str) -> Option<i64> {
        if let Some(name) = raw.strip_prefix("@integer/") {
            return self.integers.get(name).copied();
        }
        raw.trim().parse().ok()
    }

    /// Whether a layout name is available for `include` (`res/layout*/name.xml`).
    pub fn has_layout(&self, name: &str) -> bool {
        self.layout_dirs.iter().any(|d| self.source.read(&format!("{d}/{name}.xml")).is_some())
    }

    /// Reads and returns the text of `res/layout*/<name>.xml` (for includes).
    pub fn layout_xml(&self, name: &str) -> Option<String> {
        self.layout_dirs.iter().find_map(|d| self.source.read(&format!("{d}/{name}.xml")))
    }

    /// Parses `res/drawable*/<name>.xml` when it is a `<shape>` (or `<ripple>`,
    /// `<layer-list>` — approximated by their first solid fill).
    pub fn drawable_shape(&self, name: &str) -> Option<ShapeDrawable> {
        for dir in &self.drawable_dirs {
            let Some(text) = self.source.read(&format!("{dir}/{name}.xml")) else { continue };
            if let Some(shape) = parse_shape(&text) {
                return Some(shape);
            }
        }
        None
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
}

fn color_from_aarrggbb(v: &str) -> String {
    color_css(v).unwrap_or_else(|| "#000000".to_owned())
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

/// Parses a shape drawable XML subset: `<shape>` with `<solid>`, `<corners>`,
/// `<stroke>`, `<padding>`; `<gradient>` degrades to its start color.
pub fn parse_shape(xml_text: &str) -> Option<ShapeDrawable> {
    let doc = xml::parse(xml_text).ok()?;
    let root = doc.root?;
    match root.name_local() {
        "shape" | "ripple" | "layer-list" | "inset" => {}
        _ => return None,
    }
    let mut shape = ShapeDrawable { corners: [0.0; 4], padding: [0.0; 4], ..Default::default() };
    fn corners_from(el: &xml::Element, out: &mut [f32; 4]) {
        if let Some(r) = el.local("radius").and_then(dimen_px) {
            *out = [r; 4];
        }
        for (attr, idx) in
            [("topLeftRadius", 0), ("topRightRadius", 1), ("bottomRightRadius", 2), ("bottomLeftRadius", 3)]
        {
            if let Some(v) = el.local(attr).and_then(dimen_px) {
                out[idx] = v;
            }
        }
    }
    match root.name_local() {
        "shape" => {
            if let Some(solid) = root.elements().find(|e| e.name_local() == "solid") {
                if let Some(c) = solid.local("color") {
                    shape.fill = color_css(c).or_else(|| Some(c.to_owned()).filter(|_| c.starts_with('?')));
                }
            }
            for el in root.elements() {
                match el.name_local() {
                    "corners" => corners_from(el, &mut shape.corners),
                    "stroke" => {
                        let w = el.local("width").and_then(dimen_px).unwrap_or(1.0);
                        let c = el
                            .local("color")
                            .map(|c| color_css(c).unwrap_or_else(|| "#000000".to_owned()))
                            .unwrap_or_else(|| "#000000".to_owned());
                        shape.stroke = Some((w, c));
                    }
                    "padding" => {
                        shape.padding[0] = el.local("left").and_then(dimen_px).unwrap_or(0.0);
                        shape.padding[1] = el.local("top").and_then(dimen_px).unwrap_or(0.0);
                        shape.padding[2] = el.local("right").and_then(dimen_px).unwrap_or(0.0);
                        shape.padding[3] = el.local("bottom").and_then(dimen_px).unwrap_or(0.0);
                    }
                    "gradient" => {
                        if shape.fill.is_none() {
                            shape.fill = el
                                .local("startColor")
                                .and_then(|c| color_css(c).or(Some(c.to_owned()).filter(|_| c.starts_with('?'))));
                        }
                    }
                    _ => {}
                }
            }
        }
        "ripple" => {
            shape.fill = None;
            for el in root.elements() {
                if el.name_local() == "item" {
                    for inner in el.elements() {
                        if inner.name_local() == "shape" {
                            if let Some(sub) = parse_shape(&reserialize(inner)) {
                                return Some(sub);
                            }
                        }
                    }
                }
            }
        }
        // layer-list / inset: use the first drawable child if any.
        _ => {
            for el in root.elements() {
                for inner in el.elements() {
                    if inner.name_local() == "shape" {
                        if let Some(sub) = parse_shape(&reserialize(inner)) {
                            return Some(sub);
                        }
                    }
                }
            }
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
    }
}
