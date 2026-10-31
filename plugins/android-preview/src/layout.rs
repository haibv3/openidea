//! Android XML view tree → the renderable [`Node`] model.
//!
//! Parses a layout file (with resource resolution through [`Resources`] and
//! `include` expansion), applies `tools:` overrides on top of runtime
//! attributes, and resolves RelativeLayout / ConstraintLayout rules into
//! absolute positions. The output model is consumed by `render.rs`.
//!
//! Pure Rust, no host calls: fully unit-testable natively.

use std::collections::HashMap;
use std::rc::Rc;

use crate::resources::ResDrawable;

use crate::resources::{Resources, ShapeDrawable};
use crate::xml::{self, Element};

/// Resource tables plus the theme active at the current tree position.
/// `android:theme` on a view swaps the theme for that view and its children.
#[derive(Clone)]
pub struct Ctx<'a> {
    pub res: &'a Resources<'a>,
    /// `(attr name, value)` items of the active `android:theme` style chain.
    pub theme: Rc<Vec<(String, String)>>,
}

impl<'a> Ctx<'a> {
    fn color(&self, raw: &str) -> Option<String> {
        self.res.color_in_theme(raw, &self.theme, 0)
    }

    /// Same context under a different theme.
    fn with_theme(&self, theme_ref: &str) -> Ctx<'a> {
        Ctx { res: self.res, theme: Rc::new(self.res.theme_attrs(theme_ref)) }
    }
}

/// How a drawable renders on a view: inline SVG (`<vector>`) or a `data:` URI
/// (bitmap, or a URL-encoded SVG where CSS `url()` is needed).
#[derive(Debug, Clone, PartialEq)]
pub enum Image {
    Svg(String),
    Uri(String),
}

/// Preview device viewport (dp → px 1:1).
pub const DEVICE_W: f32 = 411.0;
pub const DEVICE_H: f32 = 891.0;

/// The devices the preview toolbar offers: label, width dp, height dp. The
/// first entry is the default. Relative/Constraint layouts are re-solved per
/// size (see `lib.rs::layout_screens`), so these must cover every toolbar
/// option in the order the toolbar lists them.
pub const DEVICES: &[(&str, f32, f32)] =
    &[("Pixel 5 · 411×891", 411.0, 891.0), ("Small · 360×800", 360.0, 800.0), ("Tablet · 800×1280", 800.0, 1280.0)];

/// How a view sizes itself along one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    Wrap,
    Match,
    Px(f32),
    /// Percent of the parent's inner size (0–100).
    Percent(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub start: f32,
    pub end: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Invisible,
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDir {
    Row,
    #[default]
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    #[default]
    Start,
    Center,
    End,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Gravity {
    pub h: HAlign,
    pub v: VAlign,
}

/// Which edge of a constraint target a view attaches to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    Parent,
    Id(String),
}

/// RelativeLayout rules of one child.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelRules {
    pub below: Option<String>,
    pub above: Option<String>,
    pub to_start_of: Option<String>,
    pub to_end_of: Option<String>,
    pub align_top: Option<String>,
    pub align_bottom: Option<String>,
    pub align_start: Option<String>,
    pub align_end: Option<String>,
    pub align_baseline: Option<String>,
    pub parent_start: bool,
    pub parent_end: bool,
    pub parent_top: bool,
    pub parent_bottom: bool,
    pub center_h: bool,
    pub center_v: bool,
    pub center: bool,
}

/// ConstraintLayout rules of one child (or a guideline).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConRules {
    pub top_to_top: Option<Anchor>,
    pub top_to_bottom: Option<Anchor>,
    pub bottom_to_top: Option<Anchor>,
    pub bottom_to_bottom: Option<Anchor>,
    pub start_to_start: Option<Anchor>,
    pub start_to_end: Option<Anchor>,
    pub end_to_start: Option<Anchor>,
    pub end_to_end: Option<Anchor>,
    pub h_bias: Option<f32>,
    pub v_bias: Option<f32>,
    pub width_percent: Option<f32>,
    pub height_percent: Option<f32>,
    /// `layout_dimensionRatio` as `(w, h)`.
    pub ratio: Option<(f32, f32)>,
    pub guide_begin: Option<f32>,
    pub guide_end: Option<f32>,
    pub guide_percent: Option<f32>,
    /// `layout_editor_absoluteX/Y`: the position the design tool pinned when
    /// the view has no constraints.
    pub editor_x: Option<f32>,
    pub editor_y: Option<f32>,
}

impl ConRules {
    pub fn is_guideline(&self) -> bool {
        self.guide_begin.is_some() || self.guide_end.is_some() || self.guide_percent.is_some()
    }
}

/// Absolute offsets for relative/constraint-resolved children. `right` /
/// `bottom` are distances from the parent's right/bottom edge. `left`+`right`
/// both set means "stretch between".
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pos {
    pub left: Option<f32>,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
}

/// The resolved, render-ready view node.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    /// Canonical kind (`TextView`, `LinearLayout`, `Placeholder`, …) — see
    /// `render.rs` for the understood set.
    pub kind: String,
    pub id: Option<String>,
    /// Main text (resolved through resources when possible).
    pub text: Option<String>,
    pub hint: Option<String>,
    /// Secondary payload: placeholder/drawable/expression label.
    pub sub: Option<String>,
    pub style: Style,
    pub children: Vec<Node>,
}

impl Node {
    pub fn placeholder(label: impl Into<String>) -> Node {
        Node { kind: "Placeholder".into(), sub: Some(label.into()), ..Default::default() }
    }
}

/// Resolved visual + layout style (all px after dp→px 1:1).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub width: Option<Size>,
    pub height: Option<Size>,
    pub margin: Edges,
    pub padding: Edges,
    pub bg_color: Option<String>,
    /// Background from a `<shape>` drawable.
    pub shape: Option<ShapeDrawable>,
    /// Unresolved `@drawable` name shown as a labelled block.
    pub drawable: Option<String>,
    /// `android:src`: inline vector SVG or a bitmap/SVG `data:` URI.
    pub src_image: Option<Image>,
    /// `android:background` when it is a drawable.
    pub bg_image: Option<Image>,
    /// `app:icon` (Material/custom buttons).
    pub icon: Option<Image>,
    /// `android:clipChildren` — XML views clip by default, Compose doesn't.
    pub clip_children: Option<bool>,
    /// `RecyclerView`/`ListView`/`GridView` sample item count (tools:itemCount).
    pub item_count: Option<u32>,
    pub fg: Option<String>,
    pub tint: Option<String>,
    pub font_size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub font_family: Option<&'static str>,
    pub all_caps: bool,
    /// Container content gravity (`android:gravity`).
    pub gravity: Option<Gravity>,
    /// The child's own gravity (`android:layout_gravity`).
    pub layout_gravity: Option<Gravity>,
    pub max_lines: Option<u32>,
    pub ellipsize: bool,
    /// In `em` units.
    pub letter_spacing: Option<f32>,
    pub line_height: Option<f32>,
    pub alpha: Option<f32>,
    pub elevation: Option<f32>,
    pub rotation: Option<f32>,
    pub visibility: Visibility,
    pub flex_dir: Option<FlexDir>,
    pub weight_sum: Option<f32>,
    pub weight: Option<f32>,
    pub pos: Option<Pos>,
    pub scroll_y: bool,
    pub scroll_x: bool,
    pub aspect: Option<f32>,
    /// `(value, max)`.
    pub progress: Option<(f32, f32)>,
    /// Circular spinner vs determinate bar.
    pub spinner: bool,
    pub scale_type: Option<&'static str>,
    pub grid_cols: Option<u32>,
    pub grid_row: Option<u32>,
    pub grid_col: Option<u32>,
    pub grid_row_span: Option<u32>,
    pub grid_col_span: Option<u32>,
    /// `Some(true)` = vertical guideline.
    pub guideline: Option<bool>,
    pub checked: Option<bool>,
    pub radius: Option<f32>,
    /// Flex gap (Compose `Arrangement.spacedBy`).
    pub gap: Option<f32>,
    /// `(width px, css color)`.
    pub border: Option<(f32, String)>,
    /// Relative `(x, y)` offset in px (Compose `Modifier.offset`).
    pub offset: Option<(f32, f32)>,
    /// Vertical divider (Compose `VerticalDivider`).
    pub vertical: bool,
    pub rel: RelRules,
    pub con: ConRules,
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// Builds the node tree for a layout XML document. Returns the roots (a
/// top-level `<merge>` yields its children directly). The `res` tables supply
/// `@string`/`@color`/… values, `@style` attributes, includes and shape
/// drawables. Malformed XML → `Err` with a line number.
pub fn build(content: &str, res: &Resources) -> Result<Vec<Node>, String> {
    let doc = xml::parse(content).map_err(|e| format!("XML error: {e}"))?;
    let Some(root) = doc.root else {
        return Err("no root element".into());
    };
    // `?attr/…` resolves against the theme of the root (or the default when
    // the layout declares none) — the same chain Android Studio applies.
    let theme = root.attr("android:theme").map(|t| res.theme_attrs(t)).unwrap_or_default();
    let ctx = Ctx { res, theme: Rc::new(theme) };
    let mut stack = Vec::new();
    if root.name_local() == "merge" {
        let mut out = Vec::new();
        for child in root.elements() {
            out.extend(build_element(child, &ctx, 0, &mut stack));
        }
        return Ok(out);
    }
    Ok(build_element(&root, &ctx, 0, &mut stack))
}

fn id_ref(raw: &str) -> String {
    raw.trim().trim_start_matches("@+id/").trim_start_matches("@id/").to_owned()
}

/// Maps a concrete widget class to the kind the renderer understands, so
/// `AppCompatButton`, `MaterialButton`, `MaterialCardView`, `SwitchCompat`…
/// preview like their base widget instead of an empty box.
fn canonical_kind(tag: &str) -> String {
    let simple = tag.rsplit([':', '.']).next().unwrap_or(tag);
    let kind = match simple {
        "AppCompatButton" | "MaterialButton" | "ButtonCompat" => "Button",
        "AppCompatTextView" | "MaterialTextView" => "TextView",
        "AppCompatImageView" | "ShapeableImageView" => "ImageView",
        "AppCompatImageButton" => "ImageButton",
        "AppCompatCheckBox" | "MaterialCheckBox" => "CheckBox",
        "AppCompatRadioButton" | "MaterialRadioButton" => "RadioButton",
        "SwitchCompat" | "SwitchMaterial" | "MaterialSwitch" => "Switch",
        "AppCompatEditText" | "MaterialEditText" | "TextInputEditText" => "EditText",
        "AppCompatAutoCompleteTextView" | "MaterialAutoCompleteTextView" | "AutoCompleteTextView" => "EditText",
        "MaterialCardView" => "CardView",
        "MaterialToolbar" | "Toolbar" => "Toolbar",
        "FloatingActionButton" | "ExtendedFloatingActionButton" => "FloatingActionButton",
        "Chip" => "Chip",
        "MaterialDivider" => "Divider",
        "LinearProgressIndicator" => "ProgressBar",
        "CircularProgressIndicator" => "ProgressBar",
        "MaterialSeekBar" => "SeekBar",
        "Space" => "Space",
        // Every built-in kind stays itself even under a qualified name
        // (`androidx.constraintlayout.widget.ConstraintLayout`).
        "LinearLayout"
        | "ConstraintLayout"
        | "FrameLayout"
        | "RelativeLayout"
        | "GridLayout"
        | "CoordinatorLayout"
        | "ScrollView"
        | "HorizontalScrollView"
        | "NestedScrollView"
        | "CardView"
        | "TabLayout"
        | "ViewPager"
        | "ViewPager2"
        | "RecyclerView"
        | "ListView"
        | "GridView"
        | "TextView"
        | "EditText"
        | "ImageView"
        | "ImageButton"
        | "Button"
        | "RadioButton"
        | "CheckBox"
        | "Switch"
        | "SeekBar"
        | "ProgressBar"
        | "Spinner"
        | "WebView"
        | "Space"
        | "View"
        | "Guideline"
        | "Divider"
        | "Toolbar"
        | "FloatingActionButton"
        | "VideoView"
        | "Surface"
        | "Box"
        | "Column"
        | "Row"
        | "LazyColumn"
        | "LazyRow"
        | "OutlinedButton"
        | "TextButton"
        | "ElevatedButton"
        | "FilledTonalButton"
        | "IconButton"
        | "Icon"
        | "Chip" => simple,
        // Anything custom (a dotted class name) maps by suffix:
        // `com.vinfast.…OneUIButton` previews like a Button, `…ImageView` like
        // an ImageView. Unqualified tags pass through unchanged.
        other if tag.contains('.') || tag.contains(':') => return suffix_kind(other),
        other => other,
    };
    kind.to_owned()
}

/// Custom view classes (`com.foo.BarButton`) → the closest built-in kind.
fn suffix_kind(simple: &str) -> String {
    // Order matters: `RadioButton`/`ImageButton`/`CheckBox` before `Button`.
    const RULES: [(&str, &str); 21] = [
        ("ImageButton", "ImageButton"),
        ("RadioButton", "RadioButton"),
        ("CheckBox", "CheckBox"),
        ("Button", "Button"),
        ("EditText", "EditText"),
        ("AutoCompleteTextView", "EditText"),
        ("TextView", "TextView"),
        ("ImageView", "ImageView"),
        ("RecyclerView", "RecyclerView"),
        ("ListView", "ListView"),
        ("GridView", "GridView"),
        ("ScrollView", "ScrollView"),
        ("SwitchCompat", "Switch"),
        ("Switch", "Switch"),
        ("SeekBar", "SeekBar"),
        ("ProgressBar", "ProgressBar"),
        ("Spinner", "Spinner"),
        ("CardView", "CardView"),
        ("Toolbar", "Toolbar"),
        ("FloatingActionButton", "FloatingActionButton"),
        ("Chip", "Chip"),
    ];
    for (suffix, kind) in RULES {
        if simple != suffix && simple.ends_with(suffix) {
            return kind.to_owned();
        }
    }
    "View".to_owned()
}

/// Builds one element into zero or more nodes (`include` of a `<merge>` can
/// produce several).
fn build_element(el: &Element, ctx: &Ctx, depth: usize, stack: &mut Vec<String>) -> Vec<Node> {
    if depth > 24 {
        return Vec::new(); // runaway nesting guard
    }
    match el.name_local() {
        "include" => build_include(el, ctx, depth, stack),
        "requestFocus" => Vec::new(),
        _ => vec![build_view(el, ctx, depth, stack)],
    }
}

fn build_include(el: &Element, ctx: &Ctx, depth: usize, stack: &mut Vec<String>) -> Vec<Node> {
    let Some(layout_attr) = el.attr("layout") else { return Vec::new() };
    let name = layout_attr.trim().trim_start_matches("@layout/").to_owned();
    let expansion = if depth + 1 > 8 || stack.iter().any(|s| *s == name) {
        // Cycle or too deep: degrade to a labelled placeholder.
        let mut n = Node::placeholder(format!("<include {name}> (not expanded)"));
        apply_common(el, &mut n, ctx);
        apply_tools(el, &mut n, ctx);
        vec![n]
    } else {
        match ctx.res.layout_xml(&name).and_then(|t| xml::parse(&t).ok().and_then(|d| d.root)) {
            Some(root) => {
                stack.push(name);
                let mut out = Vec::new();
                if root.name_local() == "merge" {
                    for child in root.elements() {
                        for mut node in build_element(child, ctx, depth + 1, stack) {
                            apply_include_overrides(el, &mut node, ctx);
                            out.push(node);
                        }
                    }
                } else {
                    for mut node in build_element(&root, ctx, depth + 1, stack) {
                        apply_include_overrides(el, &mut node, ctx);
                        out.push(node);
                    }
                }
                stack.pop();
                out
            }
            None => {
                let mut n = Node::placeholder(format!("<include {name}> (not found)"));
                apply_common(el, &mut n, ctx);
                apply_tools(el, &mut n, ctx);
                vec![n]
            }
        }
    };
    expansion
}

/// The `<include>` element's `android:layout_*`, `android:id`,
/// `android:visibility` and `tools:*` attributes override the included root's.
fn apply_include_overrides(el: &Element, node: &mut Node, ctx: &Ctx) {
    for (name, value) in &el.attrs {
        if name == "layout" {
            continue;
        }
        if let Some(rest) = name.strip_prefix("android:") {
            if rest.starts_with("layout_") || rest == "id" || rest == "visibility" {
                apply_android_attr(node, rest, value, ctx);
            }
        } else if let Some(local) = name.strip_prefix("tools:") {
            apply_tool(node, local, value, ctx);
        }
    }
}

fn build_view(el: &Element, ctx: &Ctx, depth: usize, stack: &mut Vec<String>) -> Node {
    // `android:theme` re-themes this view and everything below it.
    let child_ctx_storage;
    let ctx = match el.attr("android:theme") {
        Some(t) => {
            child_ctx_storage = ctx.with_theme(t);
            &child_ctx_storage
        }
        None => ctx,
    };
    let tag = el.name_local();
    let mut node = Node { kind: canonical_kind(tag), ..Default::default() };
    // A custom class with no matching widget shows its name, like AS does.
    if node.kind == "View" && tag.contains('.') {
        node.sub = Some(tag.rsplit('.').next().unwrap_or(tag).to_owned());
    }
    // XML views clip children unless told otherwise.
    node.style.clip_children = Some(true);
    if let Some(id) = el.attr("android:id") {
        node.id = Some(id_ref(id));
    }
    // `style="@style/X"` provides the base layer; element attributes win.
    if let Some(style_ref) = el.attr("style") {
        for (name, value) in ctx.res.style_attrs(style_ref) {
            if let Some(local) = name.strip_prefix("android:") {
                apply_android_attr(&mut node, local, &value, ctx);
            }
        }
    }
    apply_common(el, &mut node, ctx);
    apply_kind_attrs(el, &mut node, ctx);
    apply_tools(el, &mut node, ctx);
    // `tools:listitem`/`tools:itemCount`: preview a RecyclerView/ListView/
    // GridView with sample rows of the item layout, like Android Studio.
    if node.children.is_empty() && matches!(node.kind.as_str(), "RecyclerView" | "ListView" | "GridView") {
        if let Some(item_layout) = el.attr("tools:listitem") {
            let name = item_layout.trim().trim_start_matches("@layout/");
            if let Some(item_root) = ctx.res.layout_xml(name).and_then(|t| xml::parse(&t).ok().and_then(|d| d.root)) {
                let count = node.style.item_count.unwrap_or(5).clamp(1, 12);
                for _ in 0..count {
                    node.children.extend(build_element(&item_root, ctx, depth + 1, stack));
                }
            }
        }
    }
    // Defaults Android applies itself.
    match node.kind.as_str() {
        "LinearLayout" if node.style.flex_dir.is_none() => node.style.flex_dir = Some(FlexDir::Row),
        "ScrollView" | "NestedScrollView" => node.style.scroll_y = true,
        "HorizontalScrollView" => node.style.scroll_x = true,
        _ => {}
    }
    for child in el.elements() {
        node.children.extend(build_element(child, ctx, depth + 1, stack));
    }
    node
}

/// Applies the generic android:* attributes every view shares; unknown ones
/// are silently ignored. Shorthand `layout_margin` / `padding` are applied
/// before the per-edge variants so document order never matters.
fn apply_common(el: &Element, node: &mut Node, ctx: &Ctx) {
    const SHORTHANDS: [&str; 2] = ["layout_margin", "padding"];
    for pass in 0..2 {
        for (name, value) in &el.attrs {
            match name.split_once(':') {
                Some(("android", local)) => {
                    let shorthand = SHORTHANDS.contains(&local);
                    if (pass == 0) == shorthand {
                        apply_android_attr(node, local, value, ctx);
                    }
                }
                Some(("app", local)) if pass == 0 => apply_app_attr(node, local, value, ctx),
                _ => {} // xmlns:*, tools:* (later), unknown namespaces
            }
        }
    }
}

fn apply_android_attr(node: &mut Node, name: &str, value: &str, ctx: &Ctx) {
    let s = &mut node.style;
    match name {
        "layout_width" => s.width = parse_size(value, ctx),
        "layout_height" => s.height = parse_size(value, ctx),
        "layout_margin" => {
            let m = d(value, ctx);
            s.margin = Edges { start: m, end: m, top: m, bottom: m };
        }
        "layout_marginLeft" | "layout_marginStart" => s.margin.start = d(value, ctx),
        "layout_marginRight" | "layout_marginEnd" => s.margin.end = d(value, ctx),
        "layout_marginTop" => s.margin.top = d(value, ctx),
        "layout_marginBottom" => s.margin.bottom = d(value, ctx),
        "padding" => {
            let p = d(value, ctx);
            s.padding = Edges { start: p, end: p, top: p, bottom: p };
        }
        "paddingLeft" | "paddingStart" => s.padding.start = d(value, ctx),
        "paddingRight" | "paddingEnd" => s.padding.end = d(value, ctx),
        "paddingTop" => s.padding.top = d(value, ctx),
        "paddingBottom" => s.padding.bottom = d(value, ctx),
        "background" => apply_background(node, value, ctx),
        "alpha" => s.alpha = value.trim().parse().ok(),
        "elevation" => s.elevation = Some(d(value, ctx)),
        "rotation" => s.rotation = value.trim().parse().ok(),
        "visibility" => s.visibility = parse_visibility(value),
        "clipChildren" => s.clip_children = Some(ctx.res.boolean(value).unwrap_or(true)),
        "layout_gravity" => s.layout_gravity = Some(parse_gravity(value)),
        "gravity" => s.gravity = Some(parse_gravity(value)),
        "layout_weight" => s.weight = value.trim().parse().ok(),
        "weightSum" => s.weight_sum = value.trim().parse().ok(),
        "orientation" => {
            s.flex_dir = Some(match value.trim() {
                "horizontal" => FlexDir::Row,
                _ => FlexDir::Column,
            })
        }
        "text" => node.text = resolve_text(value, ctx),
        "hint" => node.hint = resolve_text(value, ctx),
        "textSize" => s.font_size = ctx.res.dimen(value, 0),
        "textColor" => s.fg = ctx.color(value),
        "textStyle" => {
            s.bold = Some(value.contains("bold"));
            s.italic = Some(value.contains("italic"));
        }
        "typeface" => {
            s.font_family = Some(match value.trim() {
                "monospace" => "monospace",
                "serif" => "serif",
                _ => "sans-serif",
            })
        }
        "textAllCaps" => s.all_caps = ctx.res.boolean(value).unwrap_or(false),
        "maxLines" => s.max_lines = value.trim().parse().ok(),
        "ellipsize" => s.ellipsize = matches!(value.trim(), "end" | "marquee" | "middle" | "start"),
        "letterSpacing" => s.letter_spacing = value.trim().parse().ok(),
        "lineSpacingExtra" => s.line_height = Some(s.font_size.unwrap_or(14.0) * 1.4 + d(value, ctx)),
        "lineSpacingMultiplier" => {
            s.line_height = Some(s.font_size.unwrap_or(14.0) * 1.4 * value.trim().parse().unwrap_or(1.0))
        }
        "src" => apply_src(node, value, ctx),
        "tint" => s.tint = ctx.color(value),
        "scaleType" => {
            s.scale_type = Some(match value.trim() {
                "fitXY" => "fill",
                "centerCrop" => "cover",
                _ => "contain",
            })
        }
        "progress" | "max" => {
            let (cur, max) = s.progress.take().unwrap_or((0.0, 100.0));
            s.progress = Some(match name {
                "progress" => (value.trim().parse().unwrap_or(cur), max),
                _ => (cur, value.trim().parse().unwrap_or(max)),
            });
            if name == "progress" {
                s.spinner = false;
            }
        }
        "indeterminate" => s.spinner = ctx.res.boolean(value).unwrap_or(true),
        "checked" => s.checked = ctx.res.boolean(value),
        // RelativeLayout rules
        "layout_below" => s.rel.below = Some(id_ref(value)),
        "layout_above" => s.rel.above = Some(id_ref(value)),
        "layout_toStartOf" | "layout_toLeftOf" => s.rel.to_start_of = Some(id_ref(value)),
        "layout_toEndOf" | "layout_toRightOf" => s.rel.to_end_of = Some(id_ref(value)),
        "layout_alignTop" => s.rel.align_top = Some(id_ref(value)),
        "layout_alignBottom" => s.rel.align_bottom = Some(id_ref(value)),
        "layout_alignStart" | "layout_alignLeft" => s.rel.align_start = Some(id_ref(value)),
        "layout_alignEnd" | "layout_alignRight" => s.rel.align_end = Some(id_ref(value)),
        "layout_alignBaseline" => s.rel.align_baseline = Some(id_ref(value)),
        "layout_alignParentStart" | "layout_alignParentLeft" => s.rel.parent_start = value == "true",
        "layout_alignParentEnd" | "layout_alignParentRight" => s.rel.parent_end = value == "true",
        "layout_alignParentTop" => s.rel.parent_top = value == "true",
        "layout_alignParentBottom" => s.rel.parent_bottom = value == "true",
        "layout_centerHorizontal" => s.rel.center_h = value == "true",
        "layout_centerVertical" => s.rel.center_v = value == "true",
        "layout_centerInParent" => s.rel.center = value == "true",
        // ConstraintLayout rules
        "layout_constraintTop_toTopOf" => s.con.top_to_top = Some(parse_anchor(value)),
        "layout_constraintTop_toBottomOf" => s.con.top_to_bottom = Some(parse_anchor(value)),
        "layout_constraintBottom_toTopOf" => s.con.bottom_to_top = Some(parse_anchor(value)),
        "layout_constraintBottom_toBottomOf" => s.con.bottom_to_bottom = Some(parse_anchor(value)),
        "layout_constraintStart_toStartOf" => s.con.start_to_start = Some(parse_anchor(value)),
        "layout_constraintStart_toEndOf" => s.con.start_to_end = Some(parse_anchor(value)),
        "layout_constraintEnd_toStartOf" => s.con.end_to_start = Some(parse_anchor(value)),
        "layout_constraintEnd_toEndOf" => s.con.end_to_end = Some(parse_anchor(value)),
        "layout_constraintHorizontal_bias" => s.con.h_bias = value.trim().parse().ok(),
        "layout_constraintVertical_bias" => s.con.v_bias = value.trim().parse().ok(),
        "layout_constraintWidth_percent" => s.con.width_percent = value.trim().parse().ok(),
        "layout_constraintHeight_percent" => s.con.height_percent = value.trim().parse().ok(),
        "layout_dimensionRatio" => s.con.ratio = parse_ratio(value),
        "layout_constraintGuide_begin" => s.con.guide_begin = Some(d(value, ctx)),
        "layout_constraintGuide_end" => s.con.guide_end = Some(d(value, ctx)),
        "layout_constraintGuide_percent" => s.con.guide_percent = value.trim().parse().ok(),
        "layout_editor_absoluteX" => s.con.editor_x = Some(d(value, ctx)),
        "layout_editor_absoluteY" => s.con.editor_y = Some(d(value, ctx)),
        // GridLayout
        "columnCount" => s.grid_cols = value.trim().parse().ok(),
        "rowCount" => {}
        "layout_row" => s.grid_row = value.trim().parse().ok(),
        "layout_column" => s.grid_col = value.trim().parse().ok(),
        "layout_rowSpan" => s.grid_row_span = value.trim().parse().ok(),
        "layout_columnSpan" => s.grid_col_span = value.trim().parse().ok(),
        _ => {}
    }
}

fn apply_app_attr(node: &mut Node, name: &str, value: &str, ctx: &Ctx) {
    // ConstraintLayout rules live in the `app:` namespace.
    if name.starts_with("layout_constraint") || name == "layout_dimensionRatio" || name.starts_with("layout_editor_") {
        return apply_android_attr(node, name, value, ctx);
    }
    let s = &mut node.style;
    match name {
        "cardCornerRadius" => s.radius = Some(d(value, ctx)),
        "cardElevation" => s.elevation = Some(d(value, ctx)),
        "cardBackgroundColor" => s.bg_color = ctx.color(value),
        "icon" => {
            let _ = apply_image_attr(node, value, ctx, |n, i| n.style.icon = Some(i));
        }
        "iconTint" => s.tint = ctx.color(value),
        "contentPadding" => {
            let p = d(value, ctx);
            s.padding = Edges { start: p, end: p, top: p, bottom: p };
        }
        "contentPaddingLeft" => s.padding.start = d(value, ctx),
        "contentPaddingRight" => s.padding.end = d(value, ctx),
        "contentPaddingTop" => s.padding.top = d(value, ctx),
        "contentPaddingBottom" => s.padding.bottom = d(value, ctx),
        _ => {} // other app:* namespaces ignored
    }
}

/// `tools:` attributes win over the runtime ones, exactly like Android Studio.
fn apply_tools(el: &Element, node: &mut Node, ctx: &Ctx) {
    for (name, value) in &el.attrs {
        if let Some(local) = name.strip_prefix("tools:") {
            apply_tool(node, local, value, ctx);
        }
    }
}

fn apply_tool(node: &mut Node, local: &str, value: &str, ctx: &Ctx) {
    match local {
        "text" => node.text = resolve_text(value, ctx),
        "hint" => node.hint = resolve_text(value, ctx),
        "src" => apply_src(node, value, ctx),
        "background" => apply_background(node, value, ctx),
        "textColor" => node.style.fg = ctx.color(value),
        "visibility" => node.style.visibility = parse_visibility(value),
        "itemCount" => node.style.item_count = value.trim().parse().ok(),
        "listitem" => {} // consumed by build_view when inflating sample rows
        _ => {}          // tools:ignore, tools:context, tools:targetApi, …
    }
}

fn parse_anchor(value: &str) -> Anchor {
    match value.trim() {
        "parent" => Anchor::Parent,
        v => Anchor::Id(id_ref(v)),
    }
}

fn parse_ratio(value: &str) -> Option<(f32, f32)> {
    let v = value.trim();
    let v = v.split(',').next_back().unwrap_or(v); // "H,16:9" form
    let (w, h) = v.split_once(':')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn resolve_text(value: &str, ctx: &Ctx) -> Option<String> {
    Some(ctx.res.text(value, 0).unwrap_or_else(|| value.to_owned()))
}

fn parse_visibility(value: &str) -> Visibility {
    match value.trim() {
        "invisible" => Visibility::Invisible,
        "gone" => Visibility::Gone,
        _ => Visibility::Visible,
    }
}

fn parse_size(value: &str, ctx: &Ctx) -> Option<Size> {
    match value.trim() {
        "match_parent" | "fill_parent" => Some(Size::Match),
        "wrap_content" => Some(Size::Wrap),
        _ => ctx.res.dimen(value, 0).map(Size::Px),
    }
}

fn d(value: &str, ctx: &Ctx) -> f32 {
    ctx.res.dimen(value, 0).unwrap_or(0.0)
}

fn apply_background(node: &mut Node, value: &str, ctx: &Ctx) {
    let s = &mut node.style;
    if let Some(color) = ctx.color(value) {
        s.bg_color = Some(color);
        return;
    }
    if let Some(name) = value.strip_prefix("@drawable/").or_else(|| value.strip_prefix("@mipmap/")) {
        match ctx.res.drawable(name) {
            Some(ResDrawable::Shape(shape)) => s.shape = Some(shape),
            Some(ResDrawable::Svg(svg)) => s.bg_image = Some(Image::Uri(svg_data_uri(&svg))),
            Some(ResDrawable::Image(uri)) => s.bg_image = Some(Image::Uri(uri)),
            None => s.drawable = Some(name.to_owned()),
        }
    }
}

/// `<svg>…</svg>` → `data:image/svg+xml` for CSS `url()`/`background-image`.
fn svg_data_uri(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len() + 32);
    out.push_str("data:image/svg+xml,");
    for c in svg.chars() {
        match c {
            '#' => out.push_str("%23"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '"' => out.push_str("%22"),
            '&' => out.push_str("%26"),
            _ => out.push(c),
        }
    }
    out
}

/// `android:src` / `app:icon` shared drawable resolution.
fn apply_image_attr(node: &mut Node, value: &str, ctx: &Ctx, set: impl Fn(&mut Node, Image)) -> bool {
    let Some(name) = value.strip_prefix("@drawable/").or_else(|| value.strip_prefix("@mipmap/")) else {
        return false;
    };
    match ctx.res.drawable(name) {
        Some(ResDrawable::Shape(shape)) => {
            node.style.shape = Some(shape);
            node.sub = None;
        }
        Some(ResDrawable::Svg(svg)) => set(node, Image::Svg(svg)),
        Some(ResDrawable::Image(uri)) => set(node, Image::Uri(uri)),
        None => node.sub = Some(name.to_owned()),
    }
    true
}

fn apply_src(node: &mut Node, value: &str, ctx: &Ctx) {
    if apply_image_attr(node, value, ctx, |n, i| n.style.src_image = Some(i)) {
        return;
    }
    // Text/URL sources: show as a labelled block or the literal.
    node.sub = resolve_text(value, ctx);
}

/// Kind-specific handling that plain attribute application can't express.
fn apply_kind_attrs(el: &Element, node: &mut Node, ctx: &Ctx) {
    match node.kind.as_str() {
        "ProgressBar" => {
            let explicit_bar = el.attr("android:progress").is_some()
                || el.attr("style").map(|st| st.contains("progressBarStyleHorizontal")).unwrap_or(false);
            node.style.spinner = match el.attr("android:indeterminate") {
                Some(v) => ctx.res.boolean(v).unwrap_or(true),
                None => !explicit_bar,
            };
            // Material progress indicators say which shape they are.
            match el.tag_local() {
                "CircularProgressIndicator" => node.style.spinner = true,
                "LinearProgressIndicator" => node.style.spinner = false,
                _ => {}
            }
        }
        "fragment" => {
            node.kind = "Placeholder".into();
            let name = el.attr("android:name").or_else(|| el.attr("class")).unwrap_or("fragment");
            node.sub = Some(name.rsplit(['.', ':']).next().unwrap_or(name).to_owned());
        }
        "Guideline" => {
            node.style.guideline = Some(el.attr("android:orientation").map(str::trim) != Some("horizontal"));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Gravity
// ---------------------------------------------------------------------------

pub fn parse_gravity(raw: &str) -> Gravity {
    let mut g = Gravity::default();
    for tok in raw.split(['|', ',']) {
        match tok.trim() {
            "left" | "start" => g.h = HAlign::Start,
            "right" | "end" => g.h = HAlign::End,
            "center_horizontal" => g.h = HAlign::Center,
            "fill_horizontal" => g.h = HAlign::Fill,
            "top" => g.v = VAlign::Top,
            "bottom" => g.v = VAlign::Bottom,
            "center_vertical" => g.v = VAlign::Center,
            "fill_vertical" => g.v = VAlign::Fill,
            "center" => {
                g.h = HAlign::Center;
                g.v = VAlign::Center;
            }
            "fill" => {
                g.h = HAlign::Fill;
                g.v = VAlign::Fill;
            }
            _ => {} // clip_*, fill_viewport etc.
        }
    }
    g
}

// ---------------------------------------------------------------------------
// Size estimation (used to position wrap_content children absolutely)
// ---------------------------------------------------------------------------

/// Estimates a node's rendered size inside a `(pw, ph)` parent box.
pub fn estimate_size(node: &Node, pw: f32, ph: f32) -> (f32, f32) {
    let s = &node.style;
    let w = match s.width {
        Some(Size::Px(v)) => Some(v),
        Some(Size::Percent(p)) => Some(p / 100.0 * pw),
        Some(Size::Match) => Some((pw - s.margin.start - s.margin.end).max(0.0)),
        _ => None,
    };
    let h = match s.height {
        Some(Size::Px(v)) => Some(v),
        Some(Size::Percent(p)) => Some(p / 100.0 * ph),
        Some(Size::Match) => Some((ph - s.margin.top - s.margin.bottom).max(0.0)),
        _ => None,
    };
    let (dw, dh) = wrap_estimate(node, 0);
    (w.unwrap_or(dw), h.unwrap_or(dh))
}

fn wrap_estimate(node: &Node, depth: u32) -> (f32, f32) {
    let s = &node.style;
    let font = s.font_size.unwrap_or(14.0);
    let pad_h = s.padding.start + s.padding.end;
    let pad_v = s.padding.top + s.padding.bottom;
    let text_w =
        |extra: f32| node.text.as_deref().map(|t| t.chars().count() as f32 * font * 0.62 + extra).unwrap_or(0.0);
    match node.kind.as_str() {
        "TextView" => (text_w(0.0) + pad_h, font * 1.4 + pad_v),
        "Button" | "OutlinedButton" | "TextButton" | "ElevatedButton" => {
            (text_w(32.0) + pad_h, (font * 1.4 + 20.0 + pad_v).max(36.0))
        }
        "EditText" => ((text_w(16.0) + pad_h).max(96.0), (font * 1.4 + 24.0 + pad_v).max(48.0)),
        "CheckBox" | "RadioButton" | "Switch" => ((24.0 + text_w(0.0)).max(32.0), 32.0 + pad_v),
        "ImageView" | "ImageButton" | "Icon" => (48.0 + pad_h, 48.0 + pad_v),
        "ProgressBar" if s.spinner => (32.0, 32.0),
        "ProgressBar" => (120.0, 8.0),
        "SeekBar" => (160.0, 32.0),
        "Space" | "Guideline" => (0.0, 0.0),
        "Divider" => (120.0, 1.0),
        "LinearLayout"
        | "ScrollView"
        | "HorizontalScrollView"
        | "NestedScrollView"
        | "CardView"
        | "FrameLayout"
        | "RelativeLayout"
        | "ConstraintLayout"
        | "GridLayout"
        | "Surface"
        | "Box"
        | "LazyColumn"
        | "LazyRow" => {
            if depth > 3 || node.children.is_empty() {
                return (pad_h.max(24.0), pad_v.max(24.0));
            }
            let vertical = match node.kind.as_str() {
                "HorizontalScrollView" | "LazyRow" | "Row" => false,
                "ScrollView" | "NestedScrollView" | "LazyColumn" | "Column" => true,
                _ => !matches!(s.flex_dir, Some(FlexDir::Row)),
            };
            let mut w = pad_h;
            let mut h = pad_v;
            if vertical {
                for c in &node.children {
                    if c.style.visibility == Visibility::Gone {
                        continue;
                    }
                    let (cw, ch) = wrap_estimate(c, depth + 1);
                    w = w.max(cw + c.style.margin.start + c.style.margin.end);
                    h += ch + c.style.margin.top + c.style.margin.bottom;
                }
            } else {
                for c in &node.children {
                    if c.style.visibility == Visibility::Gone {
                        continue;
                    }
                    let (cw, ch) = wrap_estimate(c, depth + 1);
                    h = h.max(ch + c.style.margin.top + c.style.margin.bottom);
                    w += cw + c.style.margin.start + c.style.margin.end;
                }
            }
            (w, h)
        }
        _ => (96.0 + pad_h, 48.0 + pad_v),
    }
}

// ---------------------------------------------------------------------------
// Position solving (RelativeLayout / ConstraintLayout / GridLayout)
// ---------------------------------------------------------------------------

/// Sibling geometry snapshot used while solving.
#[derive(Clone, Copy, Default)]
struct Geom {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Geom {
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// Computes absolute positions for every RelativeLayout / ConstraintLayout in
/// the tree (top-down; the root gets the device viewport).
pub fn resolve_positions(node: &mut Node, avail_w: f32, avail_h: f32) {
    // Children are positioned inside this node's inner box.
    let (w, h) = inner_size(node, avail_w, avail_h);
    let solved = match node.kind.as_str() {
        "RelativeLayout" => {
            solve_relative(node, w, h);
            true
        }
        "ConstraintLayout" => {
            solve_constraint(node, w, h);
            true
        }
        _ => false,
    };
    // Absolutely positioned children are invisible to CSS measurement, so a
    // `wrap_content` box would collapse to its padding. Pin the size the solver
    // (or the child estimates) produced.
    if uses_absolute_children(&node.kind) {
        let (sw, sh) = if solved { solved_extent(node, w, h) } else { wrap_extent(node) };
        if node.style.width == Some(Size::Wrap) {
            node.style.width = Some(Size::Px(sw.min(avail_w)));
        }
        if node.style.height == Some(Size::Wrap) {
            node.style.height = Some(Size::Px(sh.min(avail_h)));
        }
    }
    for child in &mut node.children {
        resolve_positions(child, w, h);
    }
}

/// Whether children of `kind` are absolutely positioned by the renderer.
fn uses_absolute_children(kind: &str) -> bool {
    matches!(kind, "FrameLayout" | "Surface" | "CardView" | "RelativeLayout" | "ConstraintLayout")
}

/// Outer size of a solved absolute container: the farthest child edge plus
/// the container's padding.
fn solved_extent(node: &Node, pw: f32, ph: f32) -> (f32, f32) {
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    for child in &node.children {
        if child.style.visibility == Visibility::Gone || child.style.guideline.is_some() {
            continue;
        }
        let (cw, ch) = estimate_size(child, pw, ph);
        let pos = child.style.pos.unwrap_or_default();
        let right = match (pos.left, pos.right) {
            (Some(l), Some(r)) => (pw - r).max(l),
            (Some(l), None) => l + cw,
            (None, Some(r)) => pw - r,
            (None, None) => cw,
        };
        let bottom = match (pos.top, pos.bottom) {
            (Some(t), Some(b)) => (ph - b).max(t),
            (Some(t), None) => t + ch,
            (None, Some(b)) => ph - b,
            (None, None) => ch,
        };
        w = w.max(right + child.style.margin.end);
        h = h.max(bottom + child.style.margin.bottom);
    }
    let s = &node.style;
    (w + s.padding.start + s.padding.end, h + s.padding.top + s.padding.bottom)
}

/// The inner content box of a node given the space available to it.
fn inner_size(node: &Node, avail_w: f32, avail_h: f32) -> (f32, f32) {
    let s = &node.style;
    let pad_w = s.padding.start + s.padding.end;
    let pad_h = s.padding.top + s.padding.bottom;
    // `wrap_content` boxes are as large as their content, so a centred child
    // (RelativeLayout `centerVertical`, a ConstraintLayout bias) lands inside
    // the content, not the whole device.
    let (wrap_w, wrap_h) = wrap_extent(node);
    let w = match s.width {
        Some(Size::Px(v)) => (v - pad_w).max(0.0),
        Some(Size::Percent(p)) => (p / 100.0 * avail_w - pad_w).max(0.0),
        Some(Size::Match) => (avail_w - s.margin.start - s.margin.end - pad_w).max(0.0),
        Some(Size::Wrap) => (wrap_w - pad_w).max(0.0).min(avail_w),
        None => avail_w,
    };
    let h = match s.height {
        Some(Size::Px(v)) => (v - pad_h).max(0.0),
        Some(Size::Percent(p)) => (p / 100.0 * avail_h - pad_h).max(0.0),
        Some(Size::Match) => (avail_h - s.margin.top - s.margin.bottom - pad_h).max(0.0),
        Some(Size::Wrap) => (wrap_h - pad_h).max(0.0).min(avail_h),
        None => avail_h,
    };
    (w, h)
}

/// Content-driven outer size of a node (padding included). Containers that
/// stack children on top of each other are as big as their largest child;
/// flow containers add their children along the main axis.
fn wrap_extent(node: &Node) -> (f32, f32) {
    let s = &node.style;
    if !matches!(node.kind.as_str(), "FrameLayout" | "Surface" | "CardView" | "RelativeLayout" | "ConstraintLayout") {
        return wrap_estimate(node, 0);
    }
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    for child in &node.children {
        if child.style.visibility == Visibility::Gone || child.style.guideline.is_some() {
            continue;
        }
        let (cw, ch) = wrap_estimate(child, 1);
        w = w.max(cw + child.style.margin.start + child.style.margin.end);
        h = h.max(ch + child.style.margin.top + child.style.margin.bottom);
    }
    (w + s.padding.start + s.padding.end, h + s.padding.top + s.padding.bottom)
}

fn id_map(children: &[Node]) -> HashMap<&str, usize> {
    children.iter().enumerate().filter_map(|(i, c)| c.id.as_deref().map(|id| (id, i))).collect()
}

fn solve_relative(node: &mut Node, pw: f32, ph: f32) {
    let n = node.children.len();
    if n == 0 {
        return;
    }
    let ids = id_map(&node.children);
    let mut geom = vec![Geom::default(); n];
    let mut pos = vec![Pos::default(); n];

    // Two passes: references to later siblings resolve in the second one.
    for _ in 0..2 {
        for i in 0..n {
            let (ew, eh) = estimate_size(&node.children[i], pw, ph);
            let style = &node.children[i].style;
            let m = style.margin;
            let r = &style.rel;
            let match_w = style.width == Some(Size::Match);
            let match_h = style.height == Some(Size::Match);
            let gone = style.visibility == Visibility::Gone;

            // Horizontal.
            let mut left = None;
            let mut right = None;
            if let Some(o) = r.to_end_of.as_deref().and_then(|id| ids.get(id)) {
                left = Some(geom[*o].right() + m.start);
            } else if let Some(o) = r.align_start.as_deref().and_then(|id| ids.get(id)) {
                left = Some(geom[*o].x + m.start);
            }
            if let Some(o) = r.to_start_of.as_deref().and_then(|id| ids.get(id)) {
                right = Some(pw - geom[*o].x + m.end);
            } else if let Some(o) = r.align_end.as_deref().and_then(|id| ids.get(id)) {
                right = Some(pw - geom[*o].right() + m.end);
            }
            if r.parent_start {
                left = Some(m.start);
            }
            if r.parent_end {
                right = Some(m.end);
            }
            if r.center_h || r.center {
                left = Some(((pw - ew - m.start - m.end) / 2.0 + m.start).max(0.0));
                right = None;
            }
            let (emit_l, emit_r) = if match_w {
                (Some(m.start), Some(m.end))
            } else if let Some(l) = left {
                (Some(l), None)
            } else if let Some(rt) = right {
                (None, Some(rt))
            } else {
                (Some(m.start), None)
            };
            let x = if emit_l.is_some() { emit_l.unwrap_or(0.0) } else { pw - emit_r.unwrap_or(0.0) - ew };

            // Vertical.
            let mut top = None;
            let mut bottom = None;
            if let Some(o) = r.below.as_deref().and_then(|id| ids.get(id)) {
                top = Some(geom[*o].bottom() + m.top);
            } else if let Some(o) = r.align_top.as_deref().and_then(|id| ids.get(id)) {
                top = Some(geom[*o].y + m.top);
            } else if let Some(o) = r.align_baseline.as_deref().and_then(|id| ids.get(id)) {
                // Text baselines coincide: both sit ~0.8em below their top.
                let target_font = node.children[*o].style.font_size.unwrap_or(14.0);
                top = Some(geom[*o].y + 0.8 * (target_font - style.font_size.unwrap_or(14.0)));
            }
            if let Some(o) = r.above.as_deref().and_then(|id| ids.get(id)) {
                bottom = Some(ph - geom[*o].y + m.bottom);
            } else if let Some(o) = r.align_bottom.as_deref().and_then(|id| ids.get(id)) {
                bottom = Some(ph - geom[*o].bottom() + m.bottom);
            }
            if r.parent_top {
                top = Some(m.top);
            }
            if r.parent_bottom {
                bottom = Some(m.bottom);
            }
            if r.center_v || r.center {
                top = Some(((ph - eh - m.top - m.bottom) / 2.0 + m.top).max(0.0));
                bottom = None;
            }
            let (emit_t, emit_b) = if match_h {
                (Some(m.top), Some(m.bottom))
            } else if let Some(t) = top {
                (Some(t), None)
            } else if let Some(b) = bottom {
                (None, Some(b))
            } else {
                (Some(m.top), None)
            };
            let y = if emit_t.is_some() { emit_t.unwrap_or(0.0) } else { ph - emit_b.unwrap_or(0.0) - eh };

            geom[i] = if gone { Geom { x, y, w: 0.0, h: 0.0 } } else { Geom { x, y, w: ew, h: eh } };
            pos[i] = Pos { left: emit_l, top: emit_t, right: emit_r, bottom: emit_b };
        }
    }
    for (child, p) in node.children.iter_mut().zip(pos) {
        child.style.pos = Some(p);
    }
}

fn anchor_edge(
    anchor: &Anchor,
    end_side: bool,
    ids: &HashMap<&str, usize>,
    geom: &[Geom],
    pw: f32,
    ph: f32,
    horizontal: bool,
) -> Option<f32> {
    Some(match anchor {
        Anchor::Parent => {
            if horizontal {
                if end_side { pw } else { 0.0 }
            } else if end_side {
                ph
            } else {
                0.0
            }
        }
        Anchor::Id(id) => {
            let g = geom[*ids.get(id.as_str())?];
            if horizontal {
                if end_side { g.right() } else { g.x }
            } else if end_side {
                g.bottom()
            } else {
                g.y
            }
        }
    })
}

fn solve_constraint(node: &mut Node, pw: f32, ph: f32) {
    let n = node.children.len();
    if n == 0 {
        return;
    }
    let ids = id_map(&node.children);
    let mut geom = vec![Geom::default(); n];
    let mut pos = vec![Pos::default(); n];
    // Solver-decided sizes written back into the model (ratio → px,
    // percent → %) so the renderer emits the constrained dimension.
    let mut size_over = vec![(None, None); n];

    // Guidelines first: they are anchors for the other children.
    for i in 0..n {
        let con = &node.children[i].style.con;
        if !con.is_guideline() || node.children[i].kind != "Guideline" {
            continue;
        }
        let vertical = node.children[i].style.guideline.unwrap_or(true);
        let x_or_y = if vertical {
            if let Some(p) = con.guide_percent {
                p * pw
            } else if let Some(e) = con.guide_end {
                pw - e
            } else {
                con.guide_begin.unwrap_or(0.0)
            }
        } else if let Some(p) = con.guide_percent {
            p * ph
        } else if let Some(e) = con.guide_end {
            ph - e
        } else {
            con.guide_begin.unwrap_or(0.0)
        };
        geom[i] = if vertical {
            Geom { x: x_or_y, y: 0.0, w: 0.0, h: ph }
        } else {
            Geom { x: 0.0, y: x_or_y, w: pw, h: x_or_y }
        };
        pos[i] = if vertical {
            Pos { left: Some(x_or_y), top: Some(0.0), right: None, bottom: Some(0.0) }
        } else {
            Pos { left: Some(0.0), top: Some(x_or_y), right: Some(0.0), bottom: None }
        };
    }

    // Two passes for forward references.
    for _ in 0..2 {
        for i in 0..n {
            let style = &node.children[i].style;
            if style.con.is_guideline() && node.children[i].kind == "Guideline" {
                continue;
            }
            if style.visibility == Visibility::Gone {
                continue;
            }
            let m = style.margin;
            let con = &style.con;

            // Width: fixed / percent / ratio needs the other axis. `0dp` is
            // match-constraint in ConstraintLayout.
            let (mut ew, mut eh) = estimate_size(&node.children[i], pw, ph);
            let mut ratio_w = false;
            let mut ratio_h = false;
            if let Some(ratio) = con.ratio {
                let w_match = matches!(style.width, Some(Size::Match) | Some(Size::Px(0.0)));
                let h_match = matches!(style.height, Some(Size::Match) | Some(Size::Px(0.0)));
                // "w:h": width = height * (w/h) when only the height is decided.
                if w_match && !h_match {
                    ew = eh * ratio.0 / ratio.1;
                    ratio_w = true;
                } else if h_match && !w_match {
                    eh = ew * ratio.1 / ratio.0;
                    ratio_h = true;
                }
            }
            if let Some(p) = con.width_percent {
                ew = p * pw;
                size_over[i].0 = Some(Size::Percent(p * 100.0));
            }
            if let Some(p) = con.height_percent {
                eh = p * ph;
                size_over[i].1 = Some(Size::Percent(p * 100.0));
            }
            if ratio_w {
                size_over[i].0 = Some(Size::Px(ew));
            }
            if ratio_h {
                size_over[i].1 = Some(Size::Px(eh));
            }
            let match_w = matches!(style.width, Some(Size::Match) | Some(Size::Px(0.0))) && !ratio_w;

            // Horizontal.
            let a = con
                .start_to_start
                .as_ref()
                .and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, true))
                .map(|v| v + m.start)
                .or_else(|| {
                    con.start_to_end
                        .as_ref()
                        .and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, true))
                        .map(|v| v + m.start)
                });
            let b = con
                .end_to_end
                .as_ref()
                .and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, true))
                .map(|v| v - m.end)
                .or_else(|| {
                    con.end_to_start
                        .as_ref()
                        .and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, true))
                        .map(|v| v - m.end)
                });
            let (emit_l, emit_r, x) = match (a, b) {
                (Some(a), Some(b)) if match_w => (Some(a), Some(pw - b), a),
                (Some(a), Some(b)) => {
                    let bias = con.h_bias.unwrap_or(0.5).clamp(0.0, 1.0);
                    let span = (b - a).max(0.0);
                    let l = a + (span - ew).max(0.0) * bias;
                    (Some(l), None, l)
                }
                (Some(a), None) => (Some(a), None, a),
                (None, Some(b)) => (None, Some(pw - b), b - ew),
                (None, None) => {
                    let l = con.editor_x.unwrap_or(m.start);
                    (Some(l), None, l)
                }
            };
            geom[i] = Geom { x, y: geom[i].y, w: ew, h: eh };
            pos[i].left = emit_l;
            pos[i].right = emit_r;

            // Vertical.
            let a = con
                .top_to_top
                .as_ref()
                .and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, false))
                .map(|v| v + m.top)
                .or_else(|| {
                    con.top_to_bottom
                        .as_ref()
                        .and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, false))
                        .map(|v| v + m.top)
                });
            let b = con
                .bottom_to_bottom
                .as_ref()
                .and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, false))
                .map(|v| v - m.bottom)
                .or_else(|| {
                    con.bottom_to_top
                        .as_ref()
                        .and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, false))
                        .map(|v| v - m.bottom)
                });
            let match_h = matches!(style.height, Some(Size::Match) | Some(Size::Px(0.0))) && !ratio_h;
            let (emit_t, emit_b, y) = match (a, b) {
                (Some(a), Some(b)) if match_h => (Some(a), Some(ph - b), a),
                (Some(a), Some(b)) => {
                    let bias = con.v_bias.unwrap_or(0.5).clamp(0.0, 1.0);
                    let span = (b - a).max(0.0);
                    let t = a + (span - eh).max(0.0) * bias;
                    (Some(t), None, t)
                }
                (Some(a), None) => (Some(a), None, a),
                (None, Some(b)) => (None, Some(ph - b), b - eh),
                (None, None) => {
                    let t = con.editor_y.unwrap_or(m.top);
                    (Some(t), None, t)
                }
            };
            geom[i] = Geom { x: geom[i].x, y, w: ew, h: eh };
            pos[i].top = emit_t;
            pos[i].bottom = emit_b;
        }
    }
    for (child, p) in node.children.iter_mut().zip(pos) {
        child.style.pos = Some(p);
    }
    for (child, (w_over, h_over)) in node.children.iter_mut().zip(size_over) {
        if let Some(w) = w_over {
            child.style.width = Some(w);
        }
        if let Some(h) = h_over {
            child.style.height = Some(h);
        }
    }
}

#[cfg(test)]
mod tests;
