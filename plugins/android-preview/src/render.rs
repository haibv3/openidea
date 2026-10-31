//! Render node trees → the standalone HTML page shown in the preview pane.
//!
//! One page hosts a toolbar (device size, zoom, light/dark) and one "screen"
//! per preview frame. Views render with dp → px 1:1 inside the screen; flex
//! for LinearLayout, absolute positioning for Frame/Relative/Constraint
//! containers (positions precomputed by `layout.rs`). Pure string building —
//! unit-tested natively.

use crate::layout::{DEVICE_H, DEVICE_W, FlexDir, Gravity, HAlign, Node, Size, Style, VAlign, Visibility};

/// One renderable preview screen.
pub struct Screen {
    pub caption: Option<String>,
    pub roots: Vec<Node>,
    /// Initial device size (dp). Falls back to [`DEVICE_W`]/[`DEVICE_H`].
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Explicit screen background (Compose `@Preview(backgroundColor = …)`).
    pub bg: Option<String>,
    /// Layout roots fill the screen (XML: a root view is laid out against the
    /// device); Compose roots size themselves, like a `ComposeView` wrapping
    /// its content at the top start.
    pub stretch_roots: bool,
    /// Toolbar device this screen belongs to (`DEVICES` index); device
    /// switching shows/hides these variants instead of resizing them.
    pub device: Option<usize>,
}

/// Builds the full standalone HTML document (toolbar + screens).
pub fn page(title: &str, screens: &[Screen]) -> String {
    let mut body = String::new();
    for s in screens {
        let w = s.width.unwrap_or(DEVICE_W);
        let h = s.height.unwrap_or(DEVICE_H);
        let bg = s.bg.as_deref().unwrap_or("var(--ap-window)");
        // Device variants render as one frame each; only the selected device
        // is visible (they were each laid out for their own viewport).
        let (variant, hidden) = match s.device {
            Some(i) => (format!(" data-device=\"{i}\""), if i == 0 { "" } else { " hidden" }),
            None => (String::new(), ""),
        };
        body.push_str(&format!("<section class=\"ap-frame\"{variant}{hidden}>"));
        if let Some(c) = &s.caption {
            // Android Studio shows the explicit frame size in the caption.
            let size = match (s.width, s.height) {
                (Some(w), Some(h)) => format!(" · {}×{} dp", fmt(w), fmt(h)),
                _ => String::new(),
            };
            body.push_str(&format!("<div class=\"ap-caption\">{}{size}</div>", esc(c)));
        }
        // A `@Preview(widthDp/heightDp)` frame keeps that size: the toolbar
        // only resizes the axes the annotation left free. Device variants are
        // already the right size, so they are fixed too.
        let fixed = match (s.width.is_some(), s.height.is_some()) {
            (false, false) => "",
            (true, true) => " data-fixed-w data-fixed-h",
            (true, false) => " data-fixed-w",
            (false, true) => " data-fixed-h",
        };
        body.push_str(&format!(
            "<div class=\"ap-screen\"{fixed} style=\"width:{}px;height:{}px;background:{};\">",
            fmt(w),
            fmt(h),
            esc(bg)
        ));
        for root in &s.roots {
            body.push_str(&view_in(root, true, None, s.stretch_roots, None));
        }
        body.push_str("</div></section>");
    }
    let devices = crate::layout::DEVICES
        .iter()
        .enumerate()
        .map(|(i, (label, w, h))| {
            let selected = if i == 0 { " selected" } else { "" };
            format!("<option value=\"{i}\" data-w=\"{}\" data-h=\"{}\"{selected}>{}</option>", fmt(*w), fmt(*h), esc(label))
        })
        .collect::<String>();
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>{title}</title><style>{CSS}</style></head>\
<body>\
<div class=\"ap-bar\" id=\"ap-bar\">\
<span class=\"ap-title\">{title}</span>\
<label>Device <select id=\"ap-device\">{devices}</select></label>\
<label>Zoom <select id=\"ap-zoom\">\
<option value=\"0.5\">50%</option><option value=\"0.75\">75%</option>\
<option value=\"1\" selected>100%</option><option value=\"1.5\">150%</option>\
</select></label>\
<button id=\"ap-dark\" type=\"button\" title=\"Toggle dark surface\">◐ Dark</button>\
</div>\
<main class=\"ap-stage\">{body}</main>\
<script>{JS}</script>\
</body></html>",
        title = esc(title),
        body = body,
        devices = devices
    )
}


// ---------------------------------------------------------------------------
// View → HTML
// ---------------------------------------------------------------------------

/// Renders one node. `root` nodes honour their declared size (`stretch` = the
/// XML rule that a root view is measured against the device); children receive
/// their parent so the positioning model can switch to absolute.
fn view_in(n: &Node, root: bool, parent: Option<&Node>, stretch: bool, inherited: Option<&str>) -> String {
    if n.style.visibility == Visibility::Gone || n.style.guideline.is_some() {
        return String::new();
    }
    let style = view_style(n, root, parent, stretch);
    let inner = content(n, inherited);
    let hidden = if n.style.visibility == Visibility::Invisible { "visibility:hidden;" } else { "" };
    format!("<div class=\"ap-view\" style=\"{style}{hidden}\">{inner}</div>")
}

fn content(n: &Node, inherited: Option<&str>) -> String {
    // What the children inherit: this node's own content colour, else the one
    // it inherited itself.
    let own = effective_fg(n);
    let pass = own.as_deref().or(inherited);
    let kids: String = n.children.iter().map(|c| view_in(c, false, Some(n), false, pass)).collect();
    match n.kind.as_str() {
        "TextView" | "Button" | "OutlinedButton" | "TextButton" | "ElevatedButton" | "FilledTonalButton"
        | "IconButton" | "Chip" => {
            let icon = match &n.style.icon {
                Some(crate::layout::Image::Svg(svg)) => {
                    format!("<span class=\"ap-btn-icon\">{svg}</span>")
                }
                Some(crate::layout::Image::Uri(uri)) => {
                    format!("<span class=\"ap-btn-icon\" style=\"background-image:url('{uri}')\"></span>")
                }
                None => String::new(),
            };
            let label = n.text.clone().or_else(|| n.sub.clone());
            let text = match label {
                Some(text) => {
                    format!(
                        "<span class=\"ap-text\" style=\"{}\">{}</span>",
                        text_style(&n.style, &n.kind, inherited),
                        esc(&text)
                    )
                }
                // A label-less button (Compose `Button { Text(…) }`) shows its
                // content, not its kind.
                None if kids.is_empty() => format!(
                    "<span class=\"ap-text\" style=\"{}\">{}</span>",
                    text_style(&n.style, &n.kind, inherited),
                    esc(&n.kind)
                ),
                None => String::new(),
            };
            format!("{icon}{text}{kids}")
        }
        "EditText" => {
            let shown = n.text.clone().or_else(|| n.hint.clone()).unwrap_or_else(|| "…".to_owned());
            let hint = if n.text.is_none() { "color:var(--ap-hint);font-style:italic;" } else { "" };
            format!(
                "<span class=\"ap-text\" style=\"{}{hint}\">{}</span>{kids}",
                text_style(&n.style, &n.kind, inherited),
                esc(&shown)
            )
        }
        "Icon" => {
            let label = n.sub.clone().unwrap_or_else(|| "icon".to_owned());
            format!("<span class=\"ap-icon\">{}</span>{kids}", esc(&label))
        }
        "ImageView" | "ImageButton" => {
            match &n.style.src_image {
                // A `<vector>` renders inline so the pane needs no assets.
                Some(crate::layout::Image::Svg(svg)) => {
                    format!("<span class=\"ap-image ap-vector\">{svg}{kids}</span>")
                }
                // Bitmaps and shapes are painted on the box (view_style), the
                // content is only the children.
                Some(crate::layout::Image::Uri(_)) => kids,
                None => {
                    let label = n.sub.clone().unwrap_or_else(|| "image".to_owned());
                    format!(
                        "<span class=\"ap-image\"><span class=\"ap-image-glyph\">🖼</span>\
<span class=\"ap-image-label\">{}</span>{kids}</span>",
                        esc(&label)
                    )
                }
            }
        }
        "CheckBox" | "RadioButton" => {
            let checked = n.style.checked.unwrap_or(false);
            let mut box_class = if n.kind == "RadioButton" { "ap-box ap-radio" } else { "ap-box" };
            if checked {
                box_class = if n.kind == "RadioButton" { "ap-box ap-radio ap-on" } else { "ap-box ap-on" };
            }
            let label = n.text.clone().unwrap_or_default();
            let label = if label.is_empty() {
                String::new()
            } else {
                format!(
                    "<span class=\"ap-check-label\" style=\"{}\">{}</span>",
                    text_style(&n.style, &n.kind, inherited),
                    esc(&label)
                )
            };
            format!("<span class=\"ap-check\"><span class=\"{box_class}\"></span>{label}</span>{kids}")
        }
        "Switch" => {
            let on = n.style.checked.unwrap_or(false);
            format!(
                "<span class=\"ap-switch{}\"><span class=\"ap-knob\"></span></span>{kids}",
                if on { " ap-on" } else { "" }
            )
        }
        "SeekBar" => {
            let (v, max) = n.style.progress.unwrap_or((0.0, 100.0));
            let pct = if max > 0.0 { (v / max * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            format!("<span class=\"ap-seek\"><span class=\"ap-seek-fill\" style=\"width:{pct}%\"></span></span>{kids}")
        }
        "ProgressBar" if n.style.spinner => format!("<span class=\"ap-spinner\"></span>{kids}"),
        "ProgressBar" => {
            let (v, max) = n.style.progress.unwrap_or((0.0, 100.0));
            let pct = if max > 0.0 { (v / max * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            format!(
                "<span class=\"ap-bar-track\"><span class=\"ap-bar-fill\" style=\"width:{pct}%\"></span></span>{kids}"
            )
        }
        "Divider" => String::new(),
        "RecyclerView" | "ListView" | "GridView" | "LazyColumn" | "LazyRow" if kids.is_empty() => {
            // Empty list: Android Studio centres a dashed placeholder in the
            // whole list area, not a small tag stuck to the top edge.
            format!(
                "<span class=\"ap-list-empty\"><span class=\"ap-empty\">&lt;{}&gt;</span></span>",
                esc(&n.kind)
            )
        }
        _ => match &n.sub {
            Some(label) if n.kind == "Placeholder" || n.kind == "View" => {
                format!("<span class=\"ap-empty\">&lt;{}&gt;</span>", esc(label))
            }
            _ => kids,
        },
    }
}

/// Whether children of `kind` are laid out with absolute positioning.
fn uses_absolute_children(kind: &str) -> bool {
    matches!(kind, "FrameLayout" | "Surface" | "CardView" | "RelativeLayout" | "ConstraintLayout")
}

// ---------------------------------------------------------------------------
// Theme defaults (Material 3 / AppCompat)
// ---------------------------------------------------------------------------

use crate::layout::Edges;
use crate::resources::{
    M3_ON_PRIMARY, M3_ON_SURFACE, M3_OUTLINE, M3_PRIMARY, M3_PRIMARY_CONTAINER, M3_SURFACE, M3_SURFACE_VARIANT,
};

/// Default appearance a view kind gets from the app theme. Merged *below*
/// `style=` and explicit attributes, matching Android's precedence: only
/// fields the node left unset are filled in.
struct Defaults {
    bg: Option<&'static str>,
    fg: Option<&'static str>,
    radius: Option<f32>,
    border: Option<(f32, &'static str)>,
    padding: Edges,
    min_w: Option<f32>,
    min_h: Option<f32>,
    /// Centre the content box (buttons, icon buttons, images).
    center: bool,
    font_size: Option<f32>,
    font_weight: Option<&'static str>,
    elevation: Option<f32>,
}

const NO_DEFAULTS: Defaults = Defaults {
    bg: None,
    fg: None,
    radius: None,
    border: None,
    padding: Edges { start: 0.0, end: 0.0, top: 0.0, bottom: 0.0 },
    min_w: None,
    min_h: None,
    center: false,
    font_size: None,
    font_weight: None,
    elevation: None,
};

/// Material 3 button metrics: 40 dp tall, 24 dp side padding, 20 dp radius.
fn button_defaults(bg: Option<&'static str>, fg: Option<&'static str>) -> Defaults {
    Defaults {
        bg,
        fg,
        radius: Some(20.0),
        padding: Edges { start: 24.0, end: 24.0, top: 0.0, bottom: 0.0 },
        min_h: Some(40.0),
        center: true,
        font_size: Some(14.0),
        font_weight: Some("500"),
        ..NO_DEFAULTS
    }
}

fn widget_defaults(kind: &str) -> Defaults {
    match kind {
        "Button" => button_defaults(Some(M3_PRIMARY), Some(M3_ON_PRIMARY)),
        "OutlinedButton" => Defaults { border: Some((1.0, M3_OUTLINE)), ..button_defaults(None, Some(M3_PRIMARY)) },
        "TextButton" => button_defaults(None, Some(M3_PRIMARY)),
        "ElevatedButton" => {
            Defaults { bg: Some("#F7F2FA"), elevation: Some(1.0), ..button_defaults(None, Some(M3_PRIMARY)) }
        }
        "FilledTonalButton" => button_defaults(Some("#E8DEF8"), Some("#1D192B")),
        "IconButton" => Defaults { min_w: Some(48.0), min_h: Some(48.0), center: true, ..NO_DEFAULTS },
        "ImageButton" => Defaults { min_w: Some(48.0), min_h: Some(48.0), center: true, ..NO_DEFAULTS },
        "ImageView" => Defaults { min_w: Some(48.0), min_h: Some(48.0), center: true, ..NO_DEFAULTS },
        "FloatingActionButton" => Defaults {
            bg: Some(M3_PRIMARY_CONTAINER),
            fg: Some("#21005D"),
            radius: Some(16.0),
            min_w: Some(56.0),
            min_h: Some(56.0),
            center: true,
            elevation: Some(3.0),
            ..NO_DEFAULTS
        },
        "CheckBox" | "RadioButton" => {
            Defaults { fg: Some(M3_ON_SURFACE), min_h: Some(48.0), font_size: Some(14.0), ..NO_DEFAULTS }
        }
        "Switch" => Defaults { min_h: Some(32.0), ..NO_DEFAULTS },
        "EditText" => Defaults {
            fg: Some(M3_ON_SURFACE),
            padding: Edges { start: 16.0, end: 16.0, top: 8.0, bottom: 8.0 },
            min_h: Some(48.0),
            font_size: Some(16.0),
            ..NO_DEFAULTS
        },
        "SeekBar" => Defaults { min_h: Some(48.0), ..NO_DEFAULTS },
        "ProgressBar" => Defaults { ..NO_DEFAULTS },
        "CardView" => Defaults { bg: Some(M3_SURFACE), radius: Some(12.0), elevation: Some(1.0), ..NO_DEFAULTS },
        "Toolbar" => Defaults { bg: Some(M3_SURFACE), min_h: Some(56.0), ..NO_DEFAULTS },
        "Chip" => Defaults {
            bg: Some(M3_SURFACE_VARIANT),
            fg: Some(M3_ON_SURFACE),
            radius: Some(8.0),
            padding: Edges { start: 12.0, end: 12.0, top: 6.0, bottom: 6.0 },
            min_h: Some(32.0),
            center: true,
            font_size: Some(14.0),
            ..NO_DEFAULTS
        },
        _ => NO_DEFAULTS,
    }
}

/// The colour a node gives its content: explicit `textColor`/`tint`, else the
/// kind's theme colour. Children without their own colour inherit it, like
/// Android's `textColor` inheritance and Compose's `LocalContentColor`.
fn effective_fg(n: &Node) -> Option<String> {
    n.style.fg.clone().or_else(|| n.style.tint.clone()).or_else(|| widget_defaults(&n.kind).fg.map(str::to_owned))
}

/// Effective font size and weight: explicit attributes, else the kind's theme
/// text style. Emitted on the box so content that has no style of its own
/// inherits it (a Compose `Button { Text(…) }` label is `labelLarge`).
fn font_props(s: &Style, d: &Defaults) -> (Option<f32>, Option<&'static str>) {
    let weight = match s.bold {
        Some(true) => Some("600"),
        Some(false) => None,
        None => d.font_weight,
    };
    (s.font_size.or(d.font_size), weight)
}

/// The effective box padding: an explicit `padding`/`contentPadding` wins, an
/// untouched box takes the kind's default.
fn effective_padding(s: &Style, d: &Defaults) -> Edges {
    if s.padding == Edges::default() { d.padding } else { s.padding }
}

/// Shared box styles for every view (positioning, size, spacing, paint).
fn view_style(n: &Node, root: bool, parent: Option<&Node>, stretch: bool) -> String {
    let abs_child = parent.map(|p| uses_absolute_children(p.kind.as_str())).unwrap_or(false);
    let s = &n.style;
    let mut out = String::new();

    // Display / positioning model per container kind.
    match n.kind.as_str() {
        "LinearLayout" => {
            out.push_str("display:flex;");
            let col = !matches!(s.flex_dir, Some(FlexDir::Row));
            out.push_str(if col { "flex-direction:column;" } else { "flex-direction:row;" });
            if let Some(g) = s.gravity {
                if col {
                    out.push_str(&format!("justify-content:{};align-items:{};", justify(g.v), align_items(g.h)));
                } else {
                    out.push_str(&format!("justify-content:{};align-items:{};", justify_h(g.h), align_items_v(g.v)));
                }
            }
            if let Some(gap) = s.gap {
                out.push_str(&format!("gap:{}px;", fmt(gap)));
            }
        }
        "ScrollView" | "NestedScrollView" | "LazyColumn" | "RecyclerView" | "ListView" => {
            out.push_str("display:flex;flex-direction:column;overflow-y:auto;");
            if let Some(g) = s.gravity {
                out.push_str(&format!("align-items:{};", align_items(g.h)));
            }
        }
        "HorizontalScrollView" | "LazyRow" => {
            out.push_str("display:flex;flex-direction:row;overflow-x:auto;");
            if let Some(g) = s.gravity {
                out.push_str(&format!("align-items:{};", align_items_v(g.v)));
            }
        }
        "GridView" => {
            out.push_str(&format!(
                "display:grid;grid-template-columns:repeat({},1fr);overflow-y:auto;",
                s.grid_cols.unwrap_or(2)
            ));
        }
        k if uses_absolute_children(k) || !n.children.is_empty() => {
            if !abs_child {
                out.push_str("position:relative;");
            }
        }
        _ => {}
    }

    // Size + positioning.
    let d = widget_defaults(&n.kind);
    if root {
        out.push_str("overflow:hidden;");
        if stretch {
            // An XML layout root is measured against the device: it occupies
            // its declared size, `match_parent` by default.
            push_root_size(&mut out, "width", s.width);
            push_root_size(&mut out, "height", s.height);
        } else {
            // A Compose preview root wraps its content at the top start.
            push_flow_size(&mut out, s);
        }
    } else if abs_child {
        // Absolutely positioned boxes are placed against the padding box, while
        // Android positions children inside the parent's padding.
        let parent = parent.unwrap();
        let pad = effective_padding(&parent.style, &widget_defaults(&parent.kind));
        out.push_str("position:absolute;");
        match parent.kind.as_str() {
            "RelativeLayout" | "ConstraintLayout" => match &s.pos {
                Some(pos) => {
                    if let Some(l) = pos.left {
                        out.push_str(&format!("left:{}px;", fmt(l + pad.start)));
                    }
                    if let Some(t) = pos.top {
                        out.push_str(&format!("top:{}px;", fmt(t + pad.top)));
                    }
                    if let Some(r) = pos.right {
                        out.push_str(&format!("right:{}px;", fmt(r + pad.end)));
                    }
                    if let Some(b) = pos.bottom {
                        out.push_str(&format!("bottom:{}px;", fmt(b + pad.bottom)));
                    }
                }
                None => out.push_str(&format!("left:{}px;top:{}px;", fmt(pad.start), fmt(pad.top))),
            },
            _ => push_frame_anchor(&mut out, s.layout_gravity.unwrap_or_default(), s, &pad),
        }
        push_abs_size(&mut out, s);
    } else {
        push_flow_size(&mut out, s);
        // A flex child honours `layout_gravity` / Compose `Modifier.align` on the
        // cross axis (Android's LinearLayout gravity and Row/Column alignment).
        if let (Some(g), Some(parent)) = (s.layout_gravity, parent) {
            let align = if flex_column(parent) { align_items(g.h) } else { align_items_v(g.v) };
            out.push_str(&format!("align-self:{align};"));
        }
    }

    // `android:clipChildren="false"` lets children draw outside the box
    // (Android clips by default). Compose nodes leave the flag unset.
    if s.clip_children.unwrap_or(false) && !s.scroll_x && !s.scroll_y {
        out.push_str("overflow:hidden;");
    }

    // Theme minimums and content centring.
    if s.weight.is_none() {
        if let Some(w) = d.min_w {
            out.push_str(&format!("min-width:{}px;", fmt(w)));
        }
    }
    if let Some(h) = d.min_h {
        out.push_str(&format!("min-height:{}px;", fmt(h)));
    }
    if d.center {
        out.push_str("display:flex;align-items:center;justify-content:center;");
    }

    // Box paint: explicit attribute → style/shape → theme default.
    if let Some(bg) = &s.bg_color {
        out.push_str(&format!("background:{};", esc(bg)));
    } else if let Some(fill) = s.shape.as_ref().and_then(|sh| sh.fill.as_ref()) {
        out.push_str(&format!("background:{};", esc(fill)));
    } else if let Some(bg) = d.bg {
        out.push_str(&format!("background:{};", esc(bg)));
    }
    // Drawable backgrounds / ImageView bitmaps: `centerCrop` → cover, fit →
    // contain (or the scaleType we stored).
    let paint_uri = [s.bg_image.as_ref(), s.src_image.as_ref()].into_iter().flatten().find_map(|i| match i {
        crate::layout::Image::Uri(u) => Some(u),
        _ => None,
    });
    if let Some(uri) = paint_uri {
        let fit = match s.scale_type.unwrap_or("cover") {
            "cover" => "cover",
            "fill" => "100% 100%",
            _ => "contain",
        };
        out.push_str(&format!(
            "background-image:url('{uri}');background-size:{fit};background-position:center;background-repeat:no-repeat;"
        ));
    }
    if let Some(r) = s.radius {
        out.push_str(&format!("border-radius:{}px;", fmt(r)));
    } else if let Some(shape) = &s.shape {
        let c = &shape.corners;
        out.push_str(&format!("border-radius:{}px {}px {}px {}px;", fmt(c[0]), fmt(c[1]), fmt(c[2]), fmt(c[3])));
    } else if let Some(r) = d.radius {
        out.push_str(&format!("border-radius:{}px;", fmt(r)));
    }
    if let Some((w, color)) = &s.border {
        out.push_str(&format!("border:{}px solid {};", fmt(*w), esc(color)));
    } else if let Some((w, color)) = s.shape.as_ref().and_then(|sh| sh.stroke.as_ref()) {
        out.push_str(&format!("border:{}px solid {};", fmt(*w), esc(color)));
    } else if let Some((w, color)) = d.border {
        out.push_str(&format!("border:{}px solid {};", fmt(w), esc(color)));
    }
    if let Some(a) = s.alpha {
        out.push_str(&format!("opacity:{};", fmt(a)));
    }
    if let Some(e) = s.elevation.or(d.elevation) {
        if e > 0.0 {
            out.push_str(&format!("box-shadow:0 {}px {}px rgba(0,0,0,0.26);", fmt(e * 0.5), fmt(e)));
        }
    }
    if let Some(rot) = s.rotation {
        if rot != 0.0 {
            out.push_str(&format!("transform:rotate({}deg);", fmt(rot)));
        }
    }
    if let Some((dx, dy)) = s.offset {
        out.push_str(&format!("left:{}px;top:{}px;", fmt(dx), fmt(dy)));
    }
    if let Some(aspect) = s.aspect {
        out.push_str(&format!("aspect-ratio:{};", fmt(aspect)));
    }

    // Text defaults, inherited by content without a style of its own.
    let (font_size, font_weight) = font_props(s, &d);
    if let Some(fs) = font_size {
        out.push_str(&format!("font-size:{}px;", fmt(fs)));
    }
    if let Some(w) = font_weight {
        out.push_str(&format!("font-weight:{w};"));
    }

    // Spacing. RelativeLayout/ConstraintLayout offsets already include the
    // margins; Frame-style anchors take them from CSS.
    let solved_parent =
        parent.map(|p| matches!(p.kind.as_str(), "RelativeLayout" | "ConstraintLayout")).unwrap_or(false);
    if !solved_parent {
        push_edges(&mut out, "margin", &s.margin);
    }
    push_edges(&mut out, "padding", &effective_padding(s, &d));

    out
}

/// Root size: the declared value, `match_parent` when the layout says nothing.
fn push_root_size(out: &mut String, prop: &str, size: Option<Size>) {
    match size {
        Some(Size::Px(v)) => out.push_str(&format!("{prop}:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("{prop}:{}%;", fmt(p))),
        Some(Size::Wrap) => out.push_str(&format!("{prop}:fit-content;")),
        Some(Size::Match) | None => out.push_str(&format!("{prop}:100%;")),
    }
}

/// FrameLayout-style stacking anchor for one child inside a padded parent.
fn push_frame_anchor(out: &mut String, g: Gravity, s: &Style, pad: &Edges) {
    let match_w = s.width == Some(Size::Match);
    let match_h = s.height == Some(Size::Match);
    let (start, end, top, bottom) = (pad.start, pad.end, pad.top, pad.bottom);
    let mut tx = false;
    let mut ty = false;
    if match_w {
        out.push_str(&format!("left:{}px;right:{}px;", fmt(start), fmt(end)));
    } else {
        match g.h {
            HAlign::Center => {
                // Centre of the content box, in padding-box coordinates.
                out.push_str(&format!("left:calc(50% + {}px);", fmt((start - end) / 2.0)));
                tx = true;
            }
            HAlign::End => out.push_str(&format!("right:{}px;", fmt(end))),
            _ => out.push_str(&format!("left:{}px;", fmt(start))),
        }
    }
    if match_h {
        out.push_str(&format!("top:{}px;bottom:{}px;", fmt(top), fmt(bottom)));
    } else {
        match g.v {
            VAlign::Center => {
                out.push_str(&format!("top:calc(50% + {}px);", fmt((top - bottom) / 2.0)));
                ty = true;
            }
            VAlign::Bottom => out.push_str(&format!("bottom:{}px;", fmt(bottom))),
            _ => out.push_str(&format!("top:{}px;", fmt(top))),
        }
    }
    if tx && ty {
        out.push_str("transform:translate(-50%,-50%);");
    } else if tx {
        out.push_str("transform:translateX(-50%);");
    } else if ty {
        out.push_str("transform:translateY(-50%);");
    }
}

/// Whether a container stacks children vertically (the flex column case).
fn flex_column(node: &Node) -> bool {
    match node.kind.as_str() {
        "HorizontalScrollView" | "LazyRow" => false,
        "LinearLayout" => !matches!(node.style.flex_dir, Some(FlexDir::Row)),
        _ => true,
    }
}

fn push_abs_size(out: &mut String, s: &Style) {
    match s.width {
        // `0dp` is "match constraint": the anchors decide the width.
        Some(Size::Px(v)) if v > 0.0 => out.push_str(&format!("width:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("width:{}%;", fmt(p))),
        Some(Size::Wrap) => out.push_str("width:fit-content;"),
        Some(Size::Px(_)) | Some(Size::Match) | None => {} // anchored by left/right
    }
    match s.height {
        Some(Size::Px(v)) if v > 0.0 => out.push_str(&format!("height:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("height:{}%;", fmt(p))),
        Some(Size::Wrap) => out.push_str("height:fit-content;"),
        Some(Size::Px(_)) | Some(Size::Match) | None => {}
    }
}

fn push_flow_size(out: &mut String, s: &Style) {
    match s.width {
        Some(Size::Px(v)) => out.push_str(&format!("width:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("width:{}%;", fmt(p))),
        Some(Size::Match) => out.push_str("width:100%;"),
        Some(Size::Wrap) | None => out.push_str("width:fit-content;"),
    }
    match s.height {
        Some(Size::Px(v)) => out.push_str(&format!("height:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("height:{}%;", fmt(p))),
        Some(Size::Match) => out.push_str("height:100%;"),
        Some(Size::Wrap) | None => {}
    }
    // weight → grow along the flex main axis.
    if let Some(w) = s.weight {
        out.push_str(&format!("flex:{} 1 0;", fmt(w)));
    }
}

fn push_edges(out: &mut String, prop: &str, e: &crate::layout::Edges) {
    if e.start == 0.0 && e.end == 0.0 && e.top == 0.0 && e.bottom == 0.0 {
        return;
    }
    out.push_str(&format!("{prop}:{}px {}px {}px {}px;", fmt(e.top), fmt(e.end), fmt(e.bottom), fmt(e.start)));
}

fn text_style(s: &Style, kind: &str, inherited: Option<&str>) -> String {
    let d = widget_defaults(kind);
    let mut out = String::new();
    let fg =
        s.fg.clone()
            .or_else(|| s.tint.clone())
            .or_else(|| d.fg.map(str::to_owned))
            .or_else(|| inherited.map(str::to_owned))
            .unwrap_or_else(|| "var(--ap-fg)".to_owned());
    out.push_str(&format!("color:{};", esc(&fg)));
    let (font_size, font_weight) = font_props(s, &d);
    if let Some(fs) = font_size {
        out.push_str(&format!("font-size:{}px;", fmt(fs)));
    }
    if let Some(w) = font_weight {
        out.push_str(&format!("font-weight:{w};"));
    }
    if s.italic == Some(true) {
        out.push_str("font-style:italic;");
    }
    if let Some(fam) = s.font_family {
        out.push_str(&format!("font-family:{fam};"));
    }
    if s.all_caps {
        out.push_str("text-transform:uppercase;");
    }
    if let Some(ls) = s.letter_spacing {
        out.push_str(&format!("letter-spacing:{}em;", fmt(ls)));
    }
    if let Some(lh) = s.line_height {
        out.push_str(&format!("line-height:{}px;", fmt(lh)));
    }
    if let Some(g) = s.gravity {
        out.push_str(&format!("text-align:{};", text_align(g.h)));
    }
    if s.ellipsize {
        out.push_str("white-space:nowrap;overflow:hidden;text-overflow:ellipsis;");
    } else if let Some(max) = s.max_lines {
        out.push_str(&format!(
            "display:-webkit-box;-webkit-line-clamp:{max};-webkit-box-orient:vertical;overflow:hidden;"
        ));
    } else {
        out.push_str("white-space:pre-wrap;");
    }
    out
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn fmt(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn justify(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "flex-start",
        VAlign::Center => "center",
        VAlign::Bottom => "flex-end",
        VAlign::Fill => "space-between",
    }
}

fn align_items(h: HAlign) -> &'static str {
    match h {
        HAlign::Start => "flex-start",
        HAlign::Center => "center",
        HAlign::End => "flex-end",
        HAlign::Fill => "stretch",
    }
}

/// `justify-content` for a Row (horizontal main axis).
fn justify_h(h: HAlign) -> &'static str {
    match h {
        HAlign::Start => "flex-start",
        HAlign::Center => "center",
        HAlign::End => "flex-end",
        HAlign::Fill => "space-between",
    }
}

/// `align-items` for a Row (vertical cross axis).
fn align_items_v(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "flex-start",
        VAlign::Center => "center",
        VAlign::Bottom => "flex-end",
        VAlign::Fill => "stretch",
    }
}

fn text_align(h: HAlign) -> &'static str {
    match h {
        HAlign::Start => "left",
        HAlign::Center => "center",
        HAlign::End => "right",
        HAlign::Fill => "justify",
    }
}

const CSS: &str = r#"
:root { --ap-window:#FFFFFF; --ap-fg:#1B1B1B; --ap-hint:#7A7A7A; --ap-primary:#6750A4;
  --ap-on-primary:#FFFFFF; --ap-outline:#79747E; --ap-outline-variant:#CAC4D0;
  --ap-surface:#FEF7FF; --ap-surface-variant:#E7E0EC; --ap-chrome:#EEEBF5; }
body.ap-dark { --ap-window:#121212; --ap-fg:#E6E1E5; --ap-hint:#9A9A9A;
  --ap-surface:#1E1B1E; --ap-surface-variant:#49454F; --ap-chrome:#262229; }
* { box-sizing:border-box; }
html,body { margin:0; }
body { font-family:Roboto,"Segoe UI",sans-serif; background:#B7B4BD; color:#1B1B1B;
  font-size:14px; }
.ap-bar { display:flex; align-items:center; gap:16px; padding:6px 14px;
  background:var(--ap-chrome); border-bottom:1px solid rgba(0,0,0,.15);
  font-size:12px; position:sticky; top:0; z-index:10; color:var(--ap-fg); }
.ap-bar .ap-title { font-weight:600; margin-right:auto; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; }
.ap-bar select, .ap-bar button { font:inherit; padding:2px 6px;
  border:1px solid rgba(0,0,0,.2); border-radius:4px; background:var(--ap-surface);
  color:var(--ap-fg); cursor:pointer; }
.ap-stage { display:flex; flex-wrap:wrap; gap:24px; padding:24px; align-items:flex-start; }
.ap-frame { display:flex; flex-direction:column; gap:6px; }
.ap-frame[hidden] { display:none; }
.ap-caption { font-size:12px; color:#33333f; }
.ap-screen { position:relative; display:flex; flex-direction:column; overflow:hidden;
  box-shadow:0 6px 24px rgba(0,0,0,.3); border-radius:8px; transform-origin:top left;
  font-size:14px; }
.ap-view { min-width:0; min-height:0; }
.ap-text { display:block; }
.ap-icon { display:inline-flex; align-items:center; justify-content:center;
  width:24px; height:24px; border-radius:50%; background:rgba(103,80,164,.15);
  color:var(--ap-primary); font-size:11px; overflow:hidden; }
.ap-image { display:flex; flex-direction:column; align-items:center; justify-content:center;
  gap:2px; width:100%; height:100%; min-width:48px; min-height:48px;
  background:rgba(0,0,0,.08); color:var(--ap-hint); border-radius:4px;
  font-size:11px; overflow:hidden; }
.ap-vector svg, .ap-btn-icon svg { width:100%; height:100%; display:block; }
.ap-vector { display:block; width:100%; height:100%; }
.ap-btn-icon { display:inline-block; width:18px; height:18px; margin-right:8px;
  vertical-align:middle; background-size:contain; background-repeat:no-repeat;
  background-position:center; }
.ap-btn-icon svg { color:inherit; }
.ap-image-label { max-width:100%; overflow:hidden; text-overflow:ellipsis;
  white-space:nowrap; }
.ap-check { display:inline-flex; align-items:center; gap:8px; }
.ap-box { display:inline-block; flex:none; width:18px; height:18px;
  border:2px solid var(--ap-outline); border-radius:2px; position:relative; }
.ap-box.ap-on { background:var(--ap-primary); border-color:var(--ap-primary); }
.ap-box.ap-on::after { content:""; position:absolute; left:4px; top:0; width:5px; height:9px;
  border:solid #FFFFFF; border-width:0 2px 2px 0; transform:rotate(45deg); }
.ap-box.ap-radio { border-radius:50%; }
.ap-box.ap-radio.ap-on::after { left:3px; top:3px; width:8px; height:8px; border:0;
  border-radius:50%; background:#FFFFFF; transform:none; }
.ap-check-label { white-space:pre-wrap; }
.ap-switch { display:inline-block; width:52px; height:32px; border-radius:16px;
  background:var(--ap-outline-variant); position:relative; transition:background .15s; }
.ap-switch.ap-on { background:var(--ap-primary); }
.ap-switch .ap-knob { position:absolute; top:4px; left:4px; width:24px; height:24px;
  border-radius:50%; background:#FAFAFA; box-shadow:0 1px 3px rgba(0,0,0,.4);
  transition:left .15s; }
.ap-switch.ap-on .ap-knob { left:24px; }
.ap-seek { display:block; width:100%; min-width:160px; height:4px; margin:22px 0;
  border-radius:2px; background:var(--ap-outline-variant); position:relative; }
.ap-seek-fill { position:absolute; inset:0 auto 0 0; border-radius:2px;
  background:var(--ap-primary); }
.ap-bar-track { display:block; width:100%; min-width:120px; height:4px; border-radius:2px;
  background:rgba(103,80,164,.24); position:relative; overflow:hidden; }
.ap-bar-fill { position:absolute; inset:0 auto 0 0; background:var(--ap-primary); }
.ap-spinner { display:inline-block; width:48px; height:48px; border-radius:50%;
  border:4px solid rgba(103,80,164,.24); border-top-color:var(--ap-primary);
  animation:ap-spin 0.9s linear infinite; }
@keyframes ap-spin { to { transform:rotate(360deg); } }
.ap-empty { display:inline-block; padding:6px 10px; border:1.5px dashed var(--ap-outline);
  border-radius:6px; color:var(--ap-hint); font-size:11px; }
.ap-list-empty { display:flex; flex:1 1 auto; align-items:center; justify-content:center;
  width:100%; min-height:96px; }
"#;

const JS: &str = r#"
(function () {
  var device = document.getElementById('ap-device');
  var zoom = document.getElementById('ap-zoom');
  var dark = document.getElementById('ap-dark');
  function apply() {
    var opt = device.options[device.selectedIndex];
    var w = parseInt(opt.getAttribute('data-w'), 10);
    var h = parseInt(opt.getAttribute('data-h'), 10);
    // Device variants were laid out for their own size: show the one the
    // toolbar selects, hide the rest.
    document.querySelectorAll('.ap-frame[data-device]').forEach(function (f) {
      f.hidden = f.getAttribute('data-device') !== device.value;
    });
    // Plain screens just resize; data-fixed axes (and variants) keep theirs.
    document.querySelectorAll('.ap-screen').forEach(function (s) {
      if (!s.hasAttribute('data-fixed-w')) s.style.width = w + 'px';
      if (!s.hasAttribute('data-fixed-h')) s.style.height = h + 'px';
      s.style.zoom = zoom.value;
    });
  }
  device.addEventListener('change', apply);
  zoom.addEventListener('change', apply);
  dark.addEventListener('click', function () { document.body.classList.toggle('ap-dark'); });
  apply();
})();
"#;
