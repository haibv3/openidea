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

    /// A dimension that may be a theme attribute (`?attr/actionBarSize`).
    fn dimen(&self, raw: &str) -> Option<f32> {
        self.res.dimen_in_theme(raw, &self.theme, 0)
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

/// One menu/tab entry (`app:menu`, `<TabItem>`, Material navigation bars).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuItem {
    pub label: String,
    pub icon: Option<Image>,
    pub checked: bool,
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

/// ConstraintLayout rules of one child (or a guideline/barrier).
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
    /// Which side the ratio derives: `true` = width (`"W,16:9"`), `false` =
    /// height (`"H,16:9"`). Absent means "whichever side is not decided".
    pub ratio_side: Option<bool>,
    /// `layout_constraintCircle`: `(target id, radius px, angle deg)`.
    pub circle: Option<(String, f32, f32)>,
    pub width_min: Option<f32>,
    pub width_max: Option<f32>,
    pub height_min: Option<f32>,
    pub height_max: Option<f32>,
    /// Margins that replace the normal ones when the anchor is `gone`.
    pub gone_start: Option<f32>,
    pub gone_end: Option<f32>,
    pub gone_top: Option<f32>,
    pub gone_bottom: Option<f32>,
    pub guide_begin: Option<f32>,
    pub guide_end: Option<f32>,
    pub guide_percent: Option<f32>,
    /// `layout_editor_absoluteX/Y`: the position the design tool pinned when
    /// the view has no constraints.
    pub editor_x: Option<f32>,
    pub editor_y: Option<f32>,
    /// `Barrier` helper: which edge of the referenced views it sits on.
    pub barrier_direction: Option<BarrierDir>,
    /// `constraint_referenced_ids` (Barrier / Group / Flow).
    pub referenced_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierDir {
    Start,
    End,
    Top,
    Bottom,
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
    /// Menu / tab entries (`app:menu`, `<TabItem>` children, Material bars).
    pub items: Vec<MenuItem>,
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
    /// `android:minWidth` / `minHeight` / `maxWidth` / `maxHeight` in px.
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    /// `android:enabled="false"` dims the view like Android does.
    pub enabled: Option<bool>,
    /// `android:translationX/Y` in px.
    pub translation: Option<(f32, f32)>,
    /// `android:scaleX/Y`.
    pub scale: Option<(f32, f32)>,
    /// Compound drawables (`android:drawableStart` …).
    pub drawable_start: Option<Image>,
    pub drawable_end: Option<Image>,
    pub drawable_padding: Option<f32>,
    /// Raw `lineSpacingExtra` / `lineSpacingMultiplier`, combined with the
    /// final font size once every attribute has been seen.
    pub line_spacing_extra: Option<f32>,
    pub line_spacing_mult: Option<f32>,
    /// `android:textFontWeight` (100–900).
    pub font_weight: Option<u32>,
    /// Progress/seek/switch paints.
    pub progress_tint: Option<String>,
    pub track_tint: Option<String>,
    pub thumb_tint: Option<String>,
    /// `TextInputLayout` with `app:boxBackgroundMode="outline"`.
    pub outlined: bool,
    /// `android:singleLine="true"`.
    pub single_line: bool,
    /// `android:foreground` when it resolves to a colour.
    pub foreground: Option<String>,
    /// Number of `RatingBar` stars and the shown rating.
    pub rating: Option<(f32, f32)>,
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
    // Theme precedence, as in the layout editor: the root's own `android:theme`,
    // then the activity named by `tools:context`, then the manifest's app theme.
    // `?attr/…` resolves against that chain, with the M3 palette as fallback.
    let theme_ref = root
        .attr("android:theme")
        .map(str::to_owned)
        .or_else(|| root.attr("tools:context").and_then(|c| res.activity_theme(c)).map(str::to_owned))
        .or_else(|| res.app_theme().map(str::to_owned));
    let theme = theme_ref.as_deref().map(|t| res.theme_attrs(t)).unwrap_or_default();
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

/// Widget class → renderer kind. Covers the platform widgets, their AppCompat
/// twins and the Material Components classes, so `MaterialButton`,
/// `TextInputLayout`, `LinearLayoutCompat` or `BottomNavigationView` preview as
/// the widget they are instead of an empty box. Anything not listed here is
/// either a Compose node kind (already canonical) or a custom view.
const KIND_MAP: &[(&str, &str)] = &[
    // Containers that stack their children.
    ("FrameLayout", "FrameLayout"),
    ("AbsoluteLayout", "FrameLayout"),
    ("CoordinatorLayout", "FrameLayout"),
    ("DrawerLayout", "FrameLayout"),
    ("SwipeRefreshLayout", "FrameLayout"),
    ("CollapsingToolbarLayout", "FrameLayout"),
    ("FragmentContainerView", "FrameLayout"),
    ("ComposeView", "FrameLayout"),
    ("MaterialButtonToggleGroup", "LinearLayout"),
    // Flow containers.
    ("LinearLayout", "LinearLayout"),
    ("LinearLayoutCompat", "LinearLayout"),
    ("RadioGroup", "LinearLayout"),
    ("TableLayout", "LinearLayout"),
    ("TableRow", "LinearLayout"),
    ("AppBarLayout", "LinearLayout"),
    ("TabHost", "LinearLayout"),
    ("TabWidget", "LinearLayout"),
    ("GridLayout", "GridLayout"),
    ("RelativeLayout", "RelativeLayout"),
    ("ConstraintLayout", "ConstraintLayout"),
    ("ScrollView", "ScrollView"),
    ("NestedScrollView", "NestedScrollView"),
    ("HorizontalScrollView", "HorizontalScrollView"),
    ("CardView", "CardView"),
    ("MaterialCardView", "CardView"),
    ("ChipGroup", "ChipGroup"),
    ("TextInputLayout", "TextInputLayout"),
    ("TabLayout", "TabLayout"),
    ("TabItem", "TabItem"),
    ("ViewPager", "ViewPager"),
    ("ViewPager2", "ViewPager"),
    ("RecyclerView", "RecyclerView"),
    ("ListView", "ListView"),
    ("ExpandableListView", "ListView"),
    ("GridView", "GridView"),
    ("Toolbar", "Toolbar"),
    ("MaterialToolbar", "Toolbar"),
    ("BottomAppBar", "BottomAppBar"),
    ("BottomNavigationView", "BottomNavigationView"),
    ("NavigationBarView", "BottomNavigationView"),
    ("NavigationRailView", "NavigationRailView"),
    ("NavigationView", "NavigationView"),
    ("SearchView", "SearchView"),
    ("androidx.appcompat.widget.SearchView", "SearchView"),
    // Text.
    ("TextView", "TextView"),
    ("AppCompatTextView", "TextView"),
    ("MaterialTextView", "TextView"),
    ("CheckedTextView", "TextView"),
    ("TextClock", "TextView"),
    ("DigitalClock", "TextView"),
    ("Chronometer", "TextView"),
    ("EditText", "EditText"),
    ("AppCompatEditText", "EditText"),
    ("MaterialEditText", "EditText"),
    ("TextInputEditText", "EditText"),
    ("AutoCompleteTextView", "EditText"),
    ("AppCompatAutoCompleteTextView", "EditText"),
    ("MaterialAutoCompleteTextView", "EditText"),
    ("MultiAutoCompleteTextView", "EditText"),
    // Buttons and inputs.
    ("Button", "Button"),
    ("AppCompatButton", "Button"),
    ("MaterialButton", "Button"),
    ("ButtonCompat", "Button"),
    ("ToggleButton", "Button"),
    ("Space", "Space"),
    ("ImageView", "ImageView"),
    ("AppCompatImageView", "ImageView"),
    ("ShapeableImageView", "ImageView"),
    ("ImageButton", "ImageButton"),
    ("AppCompatImageButton", "ImageButton"),
    ("CheckBox", "CheckBox"),
    ("AppCompatCheckBox", "CheckBox"),
    ("MaterialCheckBox", "CheckBox"),
    ("RadioButton", "RadioButton"),
    ("AppCompatRadioButton", "RadioButton"),
    ("MaterialRadioButton", "RadioButton"),
    ("Switch", "Switch"),
    ("SwitchCompat", "Switch"),
    ("SwitchMaterial", "Switch"),
    ("MaterialSwitch", "Switch"),
    ("SeekBar", "SeekBar"),
    ("AppCompatSeekBar", "SeekBar"),
    ("MaterialSeekBar", "SeekBar"),
    ("RatingBar", "RatingBar"),
    ("AppCompatRatingBar", "RatingBar"),
    ("ProgressBar", "ProgressBar"),
    ("LinearProgressIndicator", "ProgressBar"),
    ("CircularProgressIndicator", "ProgressBar"),
    ("Spinner", "Spinner"),
    ("AppCompatSpinner", "Spinner"),
    ("Chip", "Chip"),
    ("FloatingActionButton", "FloatingActionButton"),
    ("ExtendedFloatingActionButton", "FloatingActionButton"),
    ("MaterialDivider", "Divider"),
    ("Divider", "Divider"),
    ("View", "View"),
    ("SurfaceView", "SurfaceView"),
    ("TextureView", "SurfaceView"),
    ("VideoView", "SurfaceView"),
    ("WebView", "WebView"),
    ("Guideline", "Guideline"),
    ("Barrier", "Barrier"),
    ("Group", "Group"),
    ("Flow", "Flow"),
    ("CalendarView", "CalendarView"),
    ("DatePicker", "DatePicker"),
    ("TimePicker", "TimePicker"),
    ("NumberPicker", "NumberPicker"),
    // Compose node kinds are already canonical.
    ("Surface", "Surface"),
    ("Box", "Box"),
    ("Column", "Column"),
    ("Row", "Row"),
    ("LazyColumn", "LazyColumn"),
    ("LazyRow", "LazyRow"),
    ("OutlinedButton", "OutlinedButton"),
    ("TextButton", "TextButton"),
    ("ElevatedButton", "ElevatedButton"),
    ("FilledTonalButton", "FilledTonalButton"),
    ("IconButton", "IconButton"),
    ("Icon", "Icon"),
    ("Placeholder", "Placeholder"),
];

/// Maps a concrete widget class to the kind the renderer understands. A
/// qualified name whose simple name is unknown falls back to the suffix
/// heuristic (`com.vinfast.…OneUIButton` previews like a Button).
fn canonical_kind(tag: &str) -> String {
    let simple = tag.rsplit([':', '.']).next().unwrap_or(tag);
    if let Some((_, kind)) = KIND_MAP.iter().find(|(name, _)| *name == simple) {
        return (*kind).to_owned();
    }
    if tag.contains('.') || tag.contains(':') {
        return suffix_kind(simple);
    }
    simple.to_owned()
}

/// Custom view classes (`com.foo.BarButton`) → the closest built-in kind, by
/// name suffix. The declared superclass wins when the source is in the project
/// (see [`resolve_superclass_kind`]).
fn suffix_kind(simple: &str) -> String {
    // Order matters: the most specific suffix first.
    const RULES: &[(&str, &str)] = &[
        ("TextInputLayout", "TextInputLayout"),
        ("TextInputEditText", "EditText"),
        ("AutoCompleteTextView", "EditText"),
        ("BottomNavigationView", "BottomNavigationView"),
        ("NavigationRailView", "NavigationRailView"),
        ("NavigationView", "NavigationView"),
        ("FloatingActionButton", "FloatingActionButton"),
        ("RecyclerView", "RecyclerView"),
        ("ImageButton", "ImageButton"),
        ("RadioButton", "RadioButton"),
        ("ToggleButton", "Button"),
        ("CheckBox", "CheckBox"),
        ("RatingBar", "RatingBar"),
        ("SwitchCompat", "Switch"),
        ("SeekBar", "SeekBar"),
        ("ProgressBar", "ProgressBar"),
        ("ScrollView", "ScrollView"),
        ("GridLayout", "GridLayout"),
        ("ConstraintLayout", "ConstraintLayout"),
        ("RelativeLayout", "RelativeLayout"),
        ("LinearLayout", "LinearLayout"),
        ("FrameLayout", "FrameLayout"),
        ("ListView", "ListView"),
        ("GridView", "GridView"),
        ("CardView", "CardView"),
        ("ChipGroup", "ChipGroup"),
        ("TabLayout", "TabLayout"),
        ("Toolbar", "Toolbar"),
        ("Spinner", "Spinner"),
        ("Switch", "Switch"),
        ("Button", "Button"),
        ("EditText", "EditText"),
        ("TextView", "TextView"),
        ("ImageView", "ImageView"),
        ("WebView", "WebView"),
        ("Chip", "Chip"),
        ("Space", "Space"),
    ];
    for (suffix, kind) in RULES {
        if simple != *suffix && simple.ends_with(suffix) {
            return (*kind).to_owned();
        }
    }
    "View".to_owned()
}

/// Walks a custom view's superclass chain through the project sources and
/// returns the first ancestor that maps to a known widget kind. A view that
/// ultimately extends `View`/`ViewGroup` returns `None` (it stays a labelled
/// box, which is what Android Studio shows when it cannot inflate it either).
fn resolve_superclass_kind(fqn: &str, res: &Resources) -> Option<String> {
    let mut current = fqn.to_owned();
    for _ in 0..6 {
        let parent = res.class_parent(&current)?;
        let simple = parent.rsplit(['.', '$']).next().unwrap_or(parent.as_str());
        if matches!(simple, "View" | "ViewGroup" | "Object" | "Any") {
            return None;
        }
        let kind = canonical_kind(&parent);
        if kind != "View" {
            return Some(kind);
        }
        current = parent;
    }
    None
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
    // `<view class="com.foo.Bar"/>` is the escaped spelling of a custom tag.
    let class = if tag.eq_ignore_ascii_case("view") { el.attr("class").map(str::trim).unwrap_or(tag) } else { tag };
    let mut kind = canonical_kind(class);
    if kind == "View" && class.contains('.') {
        // A custom view previews like the widget it extends, which we can only
        // know by reading its superclass from the project's sources.
        kind = resolve_superclass_kind(class, ctx.res).unwrap_or(kind);
    }
    let mut node = Node { kind, ..Default::default() };
    // A custom class with no matching widget shows its name, like AS does.
    if node.kind == "View" && class.contains('.') {
        node.sub = Some(class.rsplit('.').next().unwrap_or(class).to_owned());
    }
    // XML views clip children unless told otherwise.
    node.style.clip_children = Some(true);
    if let Some(id) = el.attr("android:id") {
        node.id = Some(id_ref(id));
    }
    // `style="@style/X"` provides the base layer; element attributes win. The
    // style can also say which Material variant this is, because a library
    // style's items are not in the project's `values`.
    if let Some(style_ref) = el.attr("style") {
        apply_style_variant(&mut node, ctx.res, style_ref);
        for (name, value) in ctx.res.style_attrs(style_ref) {
            if let Some(local) = name.strip_prefix("android:") {
                apply_android_attr(&mut node, local, &value, ctx);
            }
        }
    }
    apply_common(el, &mut node, ctx);
    apply_kind_attrs(el, &mut node, ctx);
    apply_tools(el, &mut node, ctx);
    for child in el.elements() {
        node.children.extend(build_element(child, ctx, depth + 1, stack));
    }
    inflate_design_time(el, &mut node, depth, stack, ctx);
    finalize(&mut node, el);
    node
}

/// Runs the steps that need the built children: `tools:listitem` samples and
/// `tools:layout` expansion of `<fragment>` / `<ViewStub>`.
fn inflate_design_time(el: &Element, node: &mut Node, depth: usize, stack: &mut Vec<String>, ctx: &Ctx) {
    // `tools:listitem`/`tools:itemCount`: preview a RecyclerView/ListView/
    // GridView with sample rows of the item layout, like Android Studio.
    if node.children.is_empty() && matches!(node.kind.as_str(), "RecyclerView" | "ListView" | "GridView")
        && let Some(item_layout) = el.attr("tools:listitem")
    {
        let name = item_layout.trim().trim_start_matches("@layout/");
        if let Some(item_root) = ctx.res.layout_xml(name).and_then(|t| xml::parse(&t).ok().and_then(|d| d.root)) {
            let count = node.style.item_count.unwrap_or(5).clamp(1, 12);
            for _ in 0..count {
                node.children.extend(build_element(&item_root, ctx, depth + 1, stack));
            }
        }
    }
    // A `<fragment>`/`<ViewStub>` previews its `tools:layout`, which is exactly
    // what the attribute is for.
    if matches!(node.kind.as_str(), "Placeholder" | "ViewStub" | "View")
        && let Some(layout) = el.attr("tools:layout")
    {
        let name = layout.trim().trim_start_matches("@layout/");
        if let Some(root) = ctx.res.layout_xml(name).and_then(|t| xml::parse(&t).ok().and_then(|d| d.root)) {
            let mut inflated = build_element(&root, ctx, depth + 1, stack);
            if inflated.len() == 1 {
                let mut child = inflated.remove(0);
                // The host keeps its own id/size unless the tag overrides them.
                child.id = node.id.clone().or(child.id);
                child.style.width = node.style.width.or(child.style.width);
                child.style.height = node.style.height.or(child.style.height);
                child.style.margin = node.style.margin;
                *node = child;
            } else if !inflated.is_empty() {
                node.children = inflated;
            }
        }
    }
}

/// Attribute-order-independent fixups plus the defaults Android applies itself.
fn finalize(node: &mut Node, el: &Element) {
    let kind = node.kind.clone();
    let tag = el.tag_local().to_owned();
    {
        let s = &mut node.style;
        // Line spacing multiplies the final font size, which a later attribute
        // (or a style) may still have changed.
        let font = s.font_size.unwrap_or(14.0);
        let mult = s.line_spacing_mult.unwrap_or(1.0);
        if let Some(extra) = s.line_spacing_extra {
            s.line_height = Some(font * 1.2 * mult + extra);
        } else if s.line_spacing_mult.is_some() {
            s.line_height = Some(font * 1.2 * mult);
        }
        if s.single_line {
            s.max_lines = s.max_lines.or(Some(1));
            s.ellipsize = true;
        }
        if s.enabled == Some(false) && s.alpha.is_none() {
            s.alpha = Some(0.38);
        }
        match kind.as_str() {
            "LinearLayout" | "ChipGroup" | "TabLayout" | "BottomNavigationView" | "BottomAppBar" => {
                if s.flex_dir.is_none() {
                    // RadioGroup/TableLayout stack vertically by default; a
                    // LinearLayout is horizontal unless it says otherwise.
                    s.flex_dir = Some(match tag.as_str() {
                        "RadioGroup" | "TableLayout" | "AppBarLayout" => FlexDir::Column,
                        _ => FlexDir::Row,
                    });
                }
            }
            "NavigationView" | "NavigationRailView" => {
                if s.flex_dir.is_none() {
                    s.flex_dir = Some(FlexDir::Column);
                }
            }
            "ScrollView" | "NestedScrollView" | "RecyclerView" | "ListView" | "GridView" => s.scroll_y = true,
            "HorizontalScrollView" | "LazyRow" => s.scroll_x = true,
            _ => {}
        }
        // `app:layoutManager` decides how a RecyclerView lays out its items.
        if kind == "RecyclerView" {
            let manager = el.attr("app:layoutManager").or_else(|| el.attr("layoutManager")).unwrap_or("");
            if manager.contains("Grid") || manager.contains("StaggeredGrid") || s.grid_cols.unwrap_or(0) > 1 {
                node.kind = "GridView".into();
                s.grid_cols = Some(s.grid_cols.unwrap_or(2));
                s.scroll_x = false;
            } else if manager.contains("Linear") && el.attr("android:orientation").map(str::trim) == Some("horizontal") {
                node.kind = "LazyRow".into();
                s.scroll_y = false;
                s.scroll_x = true;
            }
        }
    }
}

/// A `style=` reference can change what the widget is: Material styles name the
/// variant (`Widget.Material3.Button.OutlinedButton`, `…CardView.Elevated`)
/// while their items live in the AAR, so the name is the only signal available.
fn apply_style_variant(node: &mut Node, res: &Resources, style_ref: &str) {
    let names = res.style_chain_names(style_ref);
    let any = |needle: &str| names.iter().any(|n| n.ends_with(needle));
    match node.kind.as_str() {
        "Button" => {
            if any(".IconButton") {
                node.kind = "IconButton".into();
            } else if any(".OutlinedButton") {
                node.kind = "OutlinedButton".into();
            } else if any(".TextButton") || any("Widget.AppCompat.Button.Borderless") {
                node.kind = "TextButton".into();
            } else if any(".TonalButton") {
                node.kind = "FilledTonalButton".into();
            } else if any(".ElevatedButton") {
                node.kind = "ElevatedButton".into();
            }
        }
        "CardView" => {
            if any(".CardView.Outlined") {
                node.style.outlined = true;
            } else if any(".CardView.Elevated") {
                node.style.elevation = node.style.elevation.or(Some(3.0));
            } else if any(".CardView.Filled") {
                node.style.bg_color =
                    node.style.bg_color.clone().or_else(|| Some(crate::resources::M3_SURFACE_VARIANT.to_owned()));
            }
        }
        _ => {}
    }
    if any(".TextInputLayout.OutlinedBox") || any(".OutlinedBox.Dense") {
        node.style.outlined = true;
    }
}

/// `android:textAppearance` / a `TextAppearance.*` style → the text properties.
/// Library appearances contribute their name only, so the M3/AppCompat type
/// scale is recognised by name as well as by items.
fn apply_text_appearance(node: &mut Node, res: &Resources, raw: &str, ctx: &Ctx) {
    for name in res.style_chain_names(raw) {
        // Material 3 styles are named `…Material3.TitleLarge`; the M2 table is
        // keyed by its own capitalisation, so normalise the M3 names.
        let normalized = if name.contains("Material3") || name.contains(".M3.") {
            let last = name.rsplit('.').next().unwrap_or(&name);
            let mut chars = last.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_lowercase(), chars.as_str()),
                None => last.to_owned(),
            }
        } else {
            name.clone()
        };
        if let Some((size, medium)) = text_appearance_scale(&normalized) {
            node.style.font_size = node.style.font_size.or(Some(size));
            if medium {
                node.style.bold = node.style.bold.or(Some(true));
            }
        }
    }
    for (name, value) in res.style_attrs(raw) {
        // Nested appearances are already flattened by the chain walk above.
        if let Some(local) = name.strip_prefix("android:")
            && local != "textAppearance"
        {
            apply_android_attr(node, local, &value, ctx);
        }
    }
}

/// `(size sp, medium weight)` for the well-known text appearance names.
fn text_appearance_scale(name: &str) -> Option<(f32, bool)> {
    let last = name.rsplit('.').next().unwrap_or(name);
    Some(match last {
        // Material 3 type scale.
        "displayLarge" => (57.0, false),
        "displayMedium" => (45.0, false),
        "displaySmall" => (36.0, false),
        "headlineLarge" => (32.0, false),
        "headlineMedium" => (28.0, false),
        "headlineSmall" => (24.0, false),
        "titleLarge" => (22.0, false),
        "titleMedium" => (16.0, true),
        "titleSmall" => (14.0, true),
        "bodyLarge" => (16.0, false),
        "bodyMedium" => (14.0, false),
        "bodySmall" => (12.0, false),
        "labelLarge" => (14.0, true),
        "labelMedium" => (12.0, true),
        "labelSmall" => (11.0, true),
        // Material 2 / AppCompat.
        "headline1" | "Display4" => (96.0, false),
        "headline2" | "Display3" => (60.0, false),
        "headline3" | "Display2" => (48.0, false),
        "headline4" | "Display1" => (34.0, false),
        "headline5" | "Title" | "LargeTitle" => (24.0, false),
        "headline6" | "TitleSmall" => (20.0, true),
        "subtitle1" => (16.0, false),
        "subtitle2" => (14.0, true),
        "body1" | "Large" | "Medium" | "Widget" => (16.0, false),
        "body2" | "Small" => (14.0, false),
        "caption" | "Caption" => (12.0, false),
        "overline" | "Overline" => (10.0, true),
        "button" | "Button" => (14.0, true),
        _ => return None,
    })
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
        "layout_marginHorizontal" => {
            let m = d(value, ctx);
            s.margin.start = m;
            s.margin.end = m;
        }
        "layout_marginVertical" => {
            let m = d(value, ctx);
            s.margin.top = m;
            s.margin.bottom = m;
        }
        "layout_marginLeft" | "layout_marginStart" => s.margin.start = d(value, ctx),
        "layout_marginRight" | "layout_marginEnd" => s.margin.end = d(value, ctx),
        "layout_marginTop" => s.margin.top = d(value, ctx),
        "layout_marginBottom" => s.margin.bottom = d(value, ctx),
        "padding" => {
            let p = d(value, ctx);
            s.padding = Edges { start: p, end: p, top: p, bottom: p };
        }
        "paddingHorizontal" => {
            let p = d(value, ctx);
            s.padding.start = p;
            s.padding.end = p;
        }
        "paddingVertical" => {
            let p = d(value, ctx);
            s.padding.top = p;
            s.padding.bottom = p;
        }
        "paddingLeft" | "paddingStart" => s.padding.start = d(value, ctx),
        "paddingRight" | "paddingEnd" => s.padding.end = d(value, ctx),
        "paddingTop" => s.padding.top = d(value, ctx),
        "paddingBottom" => s.padding.bottom = d(value, ctx),
        "background" => apply_background(node, value, ctx),
        "backgroundTint" => {
            // A tint repaints the background; for a plain colour that is the
            // colour itself, which is what Material buttons rely on.
            if let Some(c) = ctx.color(value) {
                s.bg_color = Some(c);
            }
        }
        "foreground" => {
            if let Some(c) = ctx.color(value) {
                s.foreground = Some(c);
            }
        }
        "minWidth" => s.min_width = ctx.dimen(value),
        "minHeight" => s.min_height = ctx.dimen(value),
        "maxWidth" => s.max_width = ctx.dimen(value),
        "maxHeight" => s.max_height = ctx.dimen(value),
        "enabled" => s.enabled = ctx.res.boolean(value),
        "translationX" => s.translation = Some((d(value, ctx), s.translation.map(|t| t.1).unwrap_or(0.0))),
        "translationY" => s.translation = Some((s.translation.map(|t| t.0).unwrap_or(0.0), d(value, ctx))),
        "scaleX" => s.scale = Some((value.trim().parse().unwrap_or(1.0), s.scale.map(|t| t.1).unwrap_or(1.0))),
        "scaleY" => s.scale = Some((s.scale.map(|t| t.0).unwrap_or(1.0), value.trim().parse().unwrap_or(1.0))),
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
        "textAppearance" => apply_text_appearance(node, ctx.res, value, ctx),
        "textSize" => s.font_size = ctx.dimen(value),
        "textColor" => s.fg = ctx.color(value),
        "textColorHint" => {
            if node.hint.is_some() && s.fg.is_none() {
                s.fg = ctx.color(value);
            }
        }
        "textStyle" => {
            s.bold = Some(value.contains("bold"));
            s.italic = Some(value.contains("italic"));
        }
        "textFontWeight" => s.font_weight = value.trim().parse().ok(),
        "typeface" => {
            s.font_family = Some(match value.trim() {
                "monospace" => "monospace",
                "serif" => "serif",
                _ => "sans-serif",
            })
        }
        "fontFamily" => {
            let (family, weight) = match value.trim() {
                "monospace" => ("monospace", None),
                "serif" => ("serif", None),
                "casual" | "cursive" => ("cursive", None),
                // `@font/…` names a font file the preview cannot embed.
                "sans-serif-medium" => ("sans-serif", Some(500)),
                "sans-serif-black" | "sans-serif-heavy" => ("sans-serif", Some(900)),
                "sans-serif-light" | "sans-serif-thin" => ("sans-serif", Some(300)),
                f if f.starts_with("sans-serif-condensed") => ("sans-serif", Some(400)),
                _ => ("sans-serif", None),
            };
            s.font_family = Some(family);
            if let Some(w) = weight {
                s.font_weight = Some(w);
            }
        }
        "textAllCaps" => s.all_caps = ctx.res.boolean(value).unwrap_or(false),
        "maxLines" => s.max_lines = value.trim().parse().ok(),
        "lines" => s.max_lines = value.trim().parse().ok(),
        "singleLine" => s.single_line = ctx.res.boolean(value).unwrap_or(false),
        "ellipsize" => s.ellipsize = matches!(value.trim(), "end" | "marquee" | "middle" | "start"),
        "letterSpacing" => s.letter_spacing = value.trim().parse().ok(),
        "lineHeight" => s.line_height = ctx.dimen(value),
        "lineSpacingExtra" => s.line_spacing_extra = ctx.dimen(value),
        "lineSpacingMultiplier" => s.line_spacing_mult = value.trim().parse().ok(),
        "textAlignment" => {
            let g = s.gravity.unwrap_or_default();
            s.gravity = Some(Gravity {
                h: match value.trim() {
                    "center" => HAlign::Center,
                    "textEnd" | "viewEnd" | "gravity" => HAlign::End,
                    _ => HAlign::Start,
                },
                v: g.v,
            });
        }
        "drawableStart" | "drawableLeft" => {
            if let Some(img) = drawable_image(value, ctx) {
                s.drawable_start = Some(img);
            }
        }
        "drawableEnd" | "drawableRight" => {
            if let Some(img) = drawable_image(value, ctx) {
                s.drawable_end = Some(img);
            }
        }
        "drawablePadding" => s.drawable_padding = ctx.dimen(value),
        "src" => apply_src(node, value, ctx),
        "tint" => s.tint = ctx.color(value),
        "buttonTint" => s.tint = ctx.color(value),
        "thumbTint" => s.thumb_tint = ctx.color(value),
        "trackTint" => s.track_tint = ctx.color(value),
        "progressTint" | "indeterminateTint" => s.progress_tint = ctx.color(value),
        "progressBackgroundTint" => s.track_tint = ctx.color(value),
        "numStars" => {
            let max = value.trim().parse().unwrap_or(5.0);
            s.rating = Some((s.rating.map(|r| r.0).unwrap_or(0.0), max));
        }
        "rating" => {
            let max = s.rating.map(|r| r.1).unwrap_or(5.0);
            s.rating = Some((value.trim().parse().unwrap_or(0.0), max));
        }
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
        "layout_constraintWidth_min" => s.con.width_min = ctx.dimen(value),
        "layout_constraintWidth_max" => s.con.width_max = ctx.dimen(value),
        "layout_constraintHeight_min" => s.con.height_min = ctx.dimen(value),
        "layout_constraintHeight_max" => s.con.height_max = ctx.dimen(value),
        "layout_constraintWidth_default" | "layout_constraintHeight_default" => {}
        "layout_constraintHorizontal_weight" | "layout_constraintVertical_weight"
        | "layout_constraintHorizontal_chainStyle" | "layout_constraintVertical_chainStyle" => {}
        "layout_dimensionRatio" => {
            let (ratio, side) = parse_ratio(value);
            s.con.ratio = ratio;
            s.con.ratio_side = side;
        }
        "layout_constraintCircle" => {
            let (_, r, a) = s.con.circle.take().unwrap_or_else(|| (String::new(), 0.0, 0.0));
            s.con.circle = Some((id_ref(value), r, a));
        }
        "layout_constraintCircleRadius" => {
            let (id, _, a) = s.con.circle.clone().unwrap_or_else(|| (String::new(), 0.0, 0.0));
            s.con.circle = Some((id, d(value, ctx), a));
        }
        "layout_constraintCircleAngle" => {
            let (id, r, _) = s.con.circle.clone().unwrap_or_else(|| (String::new(), 0.0, 0.0));
            s.con.circle = Some((id, r, value.trim().parse().unwrap_or(0.0)));
        }
        "layout_goneMarginStart" | "layout_goneMarginLeft" => s.con.gone_start = Some(d(value, ctx)),
        "layout_goneMarginEnd" | "layout_goneMarginRight" => s.con.gone_end = Some(d(value, ctx)),
        "layout_goneMarginTop" => s.con.gone_top = Some(d(value, ctx)),
        "layout_goneMarginBottom" => s.con.gone_bottom = Some(d(value, ctx)),
        "layout_constraintGuide_begin" => s.con.guide_begin = Some(d(value, ctx)),
        "layout_constraintGuide_end" => s.con.guide_end = Some(d(value, ctx)),
        "layout_constraintGuide_percent" => s.con.guide_percent = value.trim().parse().ok(),
        "layout_editor_absoluteX" => s.con.editor_x = Some(d(value, ctx)),
        "layout_editor_absoluteY" => s.con.editor_y = Some(d(value, ctx)),
        // ConstraintLayout helpers (Barrier / Group / Flow).
        "barrierDirection" => {
            s.con.barrier_direction = match value.trim() {
                "start" | "left" => Some(BarrierDir::Start),
                "end" | "right" => Some(BarrierDir::End),
                "top" => Some(BarrierDir::Top),
                "bottom" => Some(BarrierDir::Bottom),
                _ => None,
            }
        }
        "barrierAllowsGoneWidgets" => {}
        "constraint_referenced_ids" => {
            s.con.referenced_ids = value.split(',').map(|v| id_ref(v.trim())).filter(|v| !v.is_empty()).collect();
        }
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
    // ConstraintLayout rules and helpers live in the `app:` namespace.
    if name.starts_with("layout_constraint")
        || name.starts_with("layout_editor_")
        || name.starts_with("layout_gone")
        || name == "layout_dimensionRatio"
        || matches!(name, "barrierDirection" | "barrierAllowsGoneWidgets" | "constraint_referenced_ids")
    {
        return apply_android_attr(node, name, value, ctx);
    }
    let s = &mut node.style;
    match name {
        // CardView / MaterialCardView / any outlined box.
        "cardCornerRadius" | "cornerRadius" => s.radius = Some(d(value, ctx)),
        "cardElevation" | "elevation" => s.elevation = Some(d(value, ctx)),
        "cardBackgroundColor" | "cardForegroundColor" => s.bg_color = ctx.color(value),
        "cardStrokeColor" | "strokeColor" => {
            if let Some(c) = ctx.color(value) {
                set_border_color(s, c);
            }
        }
        "cardStrokeWidth" | "strokeWidth" | "borderWidth" => set_border_width(s, d(value, ctx)),
        // Images and icons.
        "srcCompat" | "src" => apply_src(node, value, ctx),
        "tint" => s.tint = ctx.color(value),
        "backgroundTint" => {
            if let Some(c) = ctx.color(value) {
                s.bg_color = Some(c);
            }
        }
        "icon" => {
            apply_image_attr(node, value, ctx, |n, i| n.style.icon = Some(i));
        }
        "iconTint" | "iconTintList" => s.tint = ctx.color(value),
        "iconPadding" => s.drawable_padding = Some(d(value, ctx)),
        "navigationIcon" => {
            if let Some(img) = drawable_image(value, ctx) {
                s.drawable_start = Some(img);
            }
        }
        // Material content padding (buttons, cards, chips).
        "contentPadding" => {
            let p = d(value, ctx);
            s.padding = Edges { start: p, end: p, top: p, bottom: p };
        }
        "contentPaddingLeft" | "contentPaddingStart" => s.padding.start = d(value, ctx),
        "contentPaddingRight" | "contentPaddingEnd" => s.padding.end = d(value, ctx),
        "contentPaddingTop" => s.padding.top = d(value, ctx),
        "contentPaddingBottom" => s.padding.bottom = d(value, ctx),
        // Toolbar / ActionBar.
        "title" => node.text = resolve_text(value, ctx),
        "subtitle" => node.hint = resolve_text(value, ctx),
        "titleTextColor" => s.fg = ctx.color(value),
        "menu" => node.items = menu_items(value, ctx),
        // TextInputLayout.
        "boxBackgroundMode" => s.outlined = value.trim() == "outline",
        "boxBackgroundColor" => s.bg_color = ctx.color(value),
        "boxStrokeColor" => {
            if let Some(c) = ctx.color(value) {
                set_border_color(s, c);
            }
        }
        "boxStrokeWidth" | "boxStrokeWidthFocused" => set_border_width(s, d(value, ctx)),
        "hintTextColor" | "errorTextColor" | "counterTextColor" | "placeholderTextColor" => {}
        "startIconDrawable" => {
            if let Some(img) = drawable_image(value, ctx) {
                s.drawable_start = Some(img);
            }
        }
        "endIconDrawable" => {
            if let Some(img) = drawable_image(value, ctx) {
                s.drawable_end = Some(img);
            }
        }
        // Chip.
        "chipBackgroundColor" => s.bg_color = ctx.color(value),
        "chipStrokeColor" => {
            if let Some(c) = ctx.color(value) {
                set_border_color(s, c);
            }
        }
        "chipStrokeWidth" => set_border_width(s, d(value, ctx)),
        "chipCornerRadius" => s.radius = Some(d(value, ctx)),
        "chipMinHeight" => s.min_height = Some(d(value, ctx)),
        "chipIcon" | "checkedIcon" | "closeIcon" => {
            if let Some(img) = drawable_image(value, ctx) {
                s.icon = Some(img);
            }
        }
        "chipIconTint" => s.tint = ctx.color(value),
        // Tabs.
        "tabIndicatorColor" => s.progress_tint = ctx.color(value),
        "tabIndicatorHeight" => {}
        "tabTextColor" | "tabSelectedTextColor" | "tabMode" | "tabGravity" => {}
        "tabText" => node.items.push(MenuItem { label: resolve_text(value, ctx).unwrap_or_default(), ..Default::default() }),
        // Navigation bars / menus.
        "itemIconTint" => s.tint = ctx.color(value),
        "itemTextColor" => s.fg = ctx.color(value),
        "itemBackground" => s.bg_color = ctx.color(value),
        // Progress indicators.
        "indicatorColor" => s.progress_tint = ctx.color(value),
        "trackColor" => s.track_tint = ctx.color(value),
        "trackThickness" | "indicatorSize" | "trackCornerRadius" | "trackStopIndicatorSize" => {}
        // FAB / ripple.
        "fabSize" | "rippleColor" | "shapeAppearance" | "shapeAppearanceOverlay" | "liftOnScroll"
        | "labelVisibilityMode" | "itemRippleColor" | "showAnimationBehavior" => {}
        _ => {} // other app:* namespaces ignored
    }
}

fn set_border_color(s: &mut Style, color: String) {
    let w = s.border.as_ref().map(|b| b.0).unwrap_or(1.0);
    s.border = Some((w, color));
}

fn set_border_width(s: &mut Style, w: f32) {
    let color = s
        .border
        .clone()
        .map(|b| b.1)
        .unwrap_or_else(|| crate::resources::M3_OUTLINE_VARIANT.to_owned());
    s.border = Some((w, color));
}

/// A drawable reference as something the renderer can paint inline.
fn drawable_image(value: &str, ctx: &Ctx) -> Option<Image> {
    let name = value.trim().strip_prefix("@drawable/").or_else(|| value.trim().strip_prefix("@mipmap/"))?;
    match ctx.res.drawable(name) {
        Some(ResDrawable::Svg(svg)) => Some(Image::Svg(svg)),
        Some(ResDrawable::Image(uri)) => Some(Image::Uri(uri)),
        _ => None,
    }
}

/// `app:menu="@menu/main"` → the menu's items, titles resolved.
fn menu_items(value: &str, ctx: &Ctx) -> Vec<MenuItem> {
    ctx.res
        .menu_items(value)
        .into_iter()
        .map(|item| MenuItem {
            label: item.title,
            icon: match item.icon {
                Some(ResDrawable::Svg(svg)) => Some(Image::Svg(svg)),
                Some(ResDrawable::Image(uri)) => Some(Image::Uri(uri)),
                _ => None,
            },
            checked: false,
        })
        .collect()
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
        "src" | "srcCompat" => apply_src(node, value, ctx),
        "background" => apply_background(node, value, ctx),
        "textColor" => node.style.fg = ctx.color(value),
        "textSize" => node.style.font_size = ctx.dimen(value),
        "visibility" => node.style.visibility = parse_visibility(value),
        "itemCount" => node.style.item_count = value.trim().parse().ok(),
        "listitem" | "layout" => {} // consumed while inflating samples / design-time layouts
        _ => {}                     // tools:ignore, tools:context, tools:targetApi, …
    }
}

fn parse_anchor(value: &str) -> Anchor {
    match value.trim() {
        "parent" => Anchor::Parent,
        v => Anchor::Id(id_ref(v)),
    }
}

/// `layout_dimensionRatio`: `"16:9"`, `"H,16:9"` (the height is derived from
/// the width) or `"W,16:9"` (the width is). Returns the ratio plus which side
/// it derives, `None` for the side when the prefix is absent.
fn parse_ratio(value: &str) -> (Option<(f32, f32)>, Option<bool>) {
    let v = value.trim();
    let (prefix, rest) = match v.split_once(',') {
        Some((p, r)) => (Some(p.trim().to_uppercase()), r.trim()),
        None => (None, v),
    };
    let side = match prefix.as_deref() {
        Some("W") => Some(true),
        Some("H") => Some(false),
        _ => None,
    };
    let Some((w, h)) = rest.split_once(':') else { return (None, side) };
    let ratio = match (w.trim().parse::<f32>().ok(), h.trim().parse::<f32>().ok()) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
        _ => None,
    };
    (ratio, side)
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
        _ => ctx.dimen(value).map(Size::Px),
    }
}

/// A dimension in px; theme attributes (`?attr/actionBarSize`) included, `0`
/// when the value cannot be resolved (Android would use its own default).
fn d(value: &str, ctx: &Ctx) -> f32 {
    ctx.dimen(value).unwrap_or(0.0)
}

fn apply_background(node: &mut Node, value: &str, ctx: &Ctx) {
    let s = &mut node.style;
    if let Some(color) = ctx.color(value) {
        s.bg_color = Some(color);
        return;
    }
    if let Some(name) = value.strip_prefix("@drawable/").or_else(|| value.strip_prefix("@mipmap/")) {
        match ctx.res.drawable(name) {
            Some(ResDrawable::Shape(mut shape)) => {
                // The drawable was parsed without a theme; `?attr/…` inside it
                // resolves against the view's theme, like inflation does.
                shape.resolve_theme(|v| ctx.color(v));
                s.shape = Some(shape);
            }
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
        Some(ResDrawable::Shape(mut shape)) => {
            shape.resolve_theme(|v| ctx.color(v));
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
    let tag = el.tag_local().to_owned();
    match node.kind.as_str() {
        "ProgressBar" => {
            let explicit_bar = el.attr("android:progress").is_some()
                || el.attr("style").map(|st| st.contains("progressBarStyleHorizontal")).unwrap_or(false);
            node.style.spinner = match el.attr("android:indeterminate") {
                Some(v) => ctx.res.boolean(v).unwrap_or(true),
                None => !explicit_bar,
            };
            // Material progress indicators say which shape they are.
            match tag.as_str() {
                "CircularProgressIndicator" => node.style.spinner = true,
                "LinearProgressIndicator" => node.style.spinner = false,
                _ => {}
            }
        }
        "RatingBar" => {
            if node.style.rating.is_none() {
                node.style.rating = Some((0.0, 5.0));
            }
        }
        "fragment" => {
            // `tools:layout` is inflated later; without it there is nothing to
            // draw, so the tag is labelled like Android Studio labels it.
            node.kind = "Placeholder".into();
            let name = el.attr("android:name").or_else(|| el.attr("class")).unwrap_or("fragment");
            node.sub = Some(name.rsplit(['.', ':']).next().unwrap_or(name).to_owned());
        }
        "ViewStub" => {
            // Invisible at runtime; the design view only shows it via tools:layout.
            if el.attr("tools:layout").is_none() {
                node.style.visibility = Visibility::Gone;
            }
        }
        "Guideline" => {
            node.style.guideline = Some(!matches!(el.local("orientation").map(str::trim), Some("horizontal")));
        }
        // ConstraintLayout helpers position other views; they draw nothing.
        "Barrier" | "Group" | "Flow" => {
            if node.kind == "Barrier" {
                node.style.guideline = Some(!matches!(
                    node.style.con.barrier_direction,
                    Some(BarrierDir::Top) | Some(BarrierDir::Bottom)
                ));
            } else {
                node.style.visibility = Visibility::Gone;
            }
        }
        "Toolbar" => {
            // A toolbar without an explicit title shows the app name, as the
            // platform does when the activity sets it.
            if node.text.is_none() {
                node.text = ctx.res.text("@string/app_name", 0);
            }
        }
        _ => {}
    }
    // Clocks show a plausible time in the preview instead of being blank.
    if node.kind == "TextView" && node.text.is_none() {
        match tag.as_str() {
            "TextClock" | "DigitalClock" => node.text = Some("12:00".to_owned()),
            "Chronometer" => node.text = Some("00:00".to_owned()),
            _ => {}
        }
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
        "Button" | "OutlinedButton" | "TextButton" | "ElevatedButton" | "FilledTonalButton" => {
            (text_w(32.0) + pad_h, (font * 1.4 + 20.0 + pad_v).max(36.0))
        }
        "Chip" => (text_w(24.0) + pad_h, (font * 1.4 + 12.0 + pad_v).max(32.0)),
        "IconButton" | "FloatingActionButton" => (56.0 + pad_h, 56.0 + pad_v),
        "EditText" | "TextInputLayout" | "SearchView" => {
            ((text_w(16.0) + pad_h).max(96.0), (font * 1.4 + 24.0 + pad_v).max(48.0))
        }
        "CheckBox" | "RadioButton" | "Switch" => ((24.0 + text_w(0.0)).max(32.0), 32.0 + pad_v),
        "ImageView" | "ImageButton" | "Icon" => (48.0 + pad_h, 48.0 + pad_v),
        "ProgressBar" if s.spinner => (32.0, 32.0),
        "ProgressBar" => (120.0, 8.0),
        "SeekBar" => (160.0, 32.0),
        "RatingBar" => (s.rating.map(|r| r.1).unwrap_or(5.0) * 24.0, 32.0),
        "Toolbar" => (text_w(0.0).max(160.0) + pad_h, 56.0 + pad_v),
        "TabItem" => (text_w(32.0) + pad_h, 48.0 + pad_v),
        "Space" | "Guideline" | "Barrier" | "Group" | "Flow" => (0.0, 0.0),
        "Divider" => (120.0, 1.0),
        "LinearLayout"
        | "ChipGroup"
        | "TabLayout"
        | "BottomNavigationView"
        | "BottomAppBar"
        | "NavigationView"
        | "NavigationRailView"
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
            Geom { x: 0.0, y: x_or_y, w: pw, h: 0.0 }
        };
        pos[i] = if vertical {
            Pos { left: Some(x_or_y), top: Some(0.0), right: None, bottom: Some(0.0) }
        } else {
            Pos { left: Some(0.0), top: Some(x_or_y), right: Some(0.0), bottom: None }
        };
    }

    // Two passes for forward references; barriers re-solve after each pass
    // because they anchor to widgets solved in it.
    for _ in 0..2 {
        solve_barriers(node, &ids, &mut geom, &mut pos, pw, ph);
        for i in 0..n {
            let style = &node.children[i].style;
            let kind = node.children[i].kind.clone();
            if kind == "Guideline" || kind == "Barrier" || kind == "Group" || kind == "Flow" {
                continue;
            }
            let con = &style.con;
            // A `gone` widget collapses to the point its constraints give it,
            // which is what the views anchored to it measure against.
            let gone = style.visibility == Visibility::Gone;
            let (mut ew, mut eh) = if gone { (0.0, 0.0) } else { estimate_size(&node.children[i], pw, ph) };

            // Circular positioning replaces every other constraint.
            if let Some((id, radius, angle)) = con.circle.clone()
                && let Some(&t) = ids.get(id.as_str())
                && !gone
            {
                let g = geom[t];
                let rad = angle.to_radians();
                let x = g.x + g.w / 2.0 + radius * rad.sin() - ew / 2.0;
                let y = g.y + g.h / 2.0 - radius * rad.cos() - eh / 2.0;
                geom[i] = Geom { x, y, w: ew, h: eh };
                pos[i] = Pos { left: Some(x), top: Some(y), right: None, bottom: None };
                continue;
            }

            // Margins of a `gone` anchor are replaced by the gone margins.
            let gone_anchor = |a: Option<&Anchor>| match a {
                Some(Anchor::Id(id)) => {
                    ids.get(id.as_str()).is_some_and(|&t| node.children[t].style.visibility == Visibility::Gone)
                }
                _ => false,
            };
            let start_gone = gone_anchor(con.start_to_start.as_ref().or(con.start_to_end.as_ref()));
            let end_gone = gone_anchor(con.end_to_end.as_ref().or(con.end_to_start.as_ref()));
            let top_gone = gone_anchor(con.top_to_top.as_ref().or(con.top_to_bottom.as_ref()));
            let bottom_gone = gone_anchor(con.bottom_to_bottom.as_ref().or(con.bottom_to_top.as_ref()));
            let m = if gone {
                Edges::default()
            } else {
                Edges {
                    start: if start_gone { con.gone_start.unwrap_or(0.0) } else { style.margin.start },
                    end: if end_gone { con.gone_end.unwrap_or(0.0) } else { style.margin.end },
                    top: if top_gone { con.gone_top.unwrap_or(0.0) } else { style.margin.top },
                    bottom: if bottom_gone { con.gone_bottom.unwrap_or(0.0) } else { style.margin.bottom },
                }
            };

            // Width: fixed / percent / ratio needs the other axis. `0dp` is
            // match-constraint in ConstraintLayout.
            let mut ratio_w = false;
            let mut ratio_h = false;
            let w_match = matches!(style.width, Some(Size::Match) | Some(Size::Px(0.0)));
            let h_match = matches!(style.height, Some(Size::Match) | Some(Size::Px(0.0)));
            if let Some(ratio) = con.ratio {
                // With both axes matched the ratio is derived from the span the
                // opposing anchors stretch across, not the placeholder estimate.
                let stretch_w = if w_match {
                    let l = con
                        .start_to_start
                        .as_ref()
                        .and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, true))
                        .or_else(|| {
                            con.start_to_end.as_ref().and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, true))
                        });
                    let r = con
                        .end_to_end
                        .as_ref()
                        .and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, true))
                        .or_else(|| {
                            con.end_to_start.as_ref().and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, true))
                        });
                    match (l, r) {
                        (Some(l), Some(r)) => Some((r - m.end - (l + m.start)).max(0.0)),
                        _ => None,
                    }
                } else {
                    None
                };
                let stretch_h = if h_match {
                    let t = con
                        .top_to_top
                        .as_ref()
                        .and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, false))
                        .or_else(|| {
                            con.top_to_bottom.as_ref().and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, false))
                        });
                    let b = con
                        .bottom_to_bottom
                        .as_ref()
                        .and_then(|x| anchor_edge(x, true, &ids, &geom, pw, ph, false))
                        .or_else(|| {
                            con.bottom_to_top.as_ref().and_then(|x| anchor_edge(x, false, &ids, &geom, pw, ph, false))
                        });
                    match (t, b) {
                        (Some(t), Some(b)) => Some((b - m.bottom - (t + m.top)).max(0.0)),
                        _ => None,
                    }
                } else {
                    None
                };
                // `"W,16:9"` derives the width, `"H,16:9"` the height; without a
                // prefix the undecided side is derived from the decided one.
                let derive_width = con.ratio_side.unwrap_or(w_match && !h_match);
                if derive_width {
                    ew = stretch_h.unwrap_or(eh) * ratio.0 / ratio.1;
                    ratio_w = true;
                } else {
                    eh = stretch_w.unwrap_or(ew) * ratio.1 / ratio.0;
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
            let clamped_w = clamp_dim(ew, con.width_min, con.width_max);
            let clamped_h = clamp_dim(eh, con.height_min, con.height_max);
            if (clamped_w - ew).abs() > f32::EPSILON && w_match {
                size_over[i].0 = Some(Size::Px(clamped_w));
            }
            if (clamped_h - eh).abs() > f32::EPSILON && h_match {
                size_over[i].1 = Some(Size::Px(clamped_h));
            }
            ew = clamped_w;
            eh = clamped_h;
            if ratio_w {
                size_over[i].0 = Some(Size::Px(ew));
            }
            if ratio_h {
                size_over[i].1 = Some(Size::Px(eh));
            }
            let match_w = w_match && !ratio_w;

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
                (Some(a), Some(b)) if match_w => (Some(a), Some((pw - b).max(0.0)), a),
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
            let match_h = h_match && !ratio_h;
            let (emit_t, emit_b, y) = match (a, b) {
                (Some(a), Some(b)) if match_h => (Some(a), Some((ph - b).max(0.0)), a),
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

/// `layout_constraintWidth_min`/`max` (`wrap` is 0, so it never clamps).
fn clamp_dim(v: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    let v = min.map(|m| v.max(m)).unwrap_or(v);
    max.filter(|m| *m > 0.0).map(|m| v.min(m)).unwrap_or(v)
}

/// A `Barrier` is a zero-size anchor on the extreme edge of the views it
/// references, shifted by its own margin.
fn solve_barriers(node: &Node, ids: &HashMap<&str, usize>, geom: &mut [Geom], pos: &mut [Pos], pw: f32, ph: f32) {
    for (i, child) in node.children.iter().enumerate() {
        if child.kind != "Barrier" {
            continue;
        }
        let Some(dir) = child.style.con.barrier_direction else { continue };
        let targets: Vec<usize> =
            child.style.con.referenced_ids.iter().filter_map(|id| ids.get(id.as_str()).copied()).collect();
        if targets.is_empty() {
            continue;
        }
        let m = child.style.margin;
        let (vertical, at) = match dir {
            BarrierDir::Start => (true, targets.iter().map(|&t| geom[t].x).fold(f32::MAX, f32::min) - m.start),
            BarrierDir::End => (true, targets.iter().map(|&t| geom[t].right()).fold(f32::MIN, f32::max) + m.end),
            BarrierDir::Top => (false, targets.iter().map(|&t| geom[t].y).fold(f32::MAX, f32::min) - m.top),
            BarrierDir::Bottom => (false, targets.iter().map(|&t| geom[t].bottom()).fold(f32::MIN, f32::max) + m.bottom),
        };
        geom[i] = if vertical { Geom { x: at, y: 0.0, w: 0.0, h: ph } } else { Geom { x: 0.0, y: at, w: pw, h: 0.0 } };
        pos[i] = if vertical {
            Pos { left: Some(at), top: Some(0.0), right: None, bottom: Some(0.0) }
        } else {
            Pos { left: Some(0.0), top: Some(at), right: Some(0.0), bottom: None }
        };
    }
}

#[cfg(test)]
mod tests;
