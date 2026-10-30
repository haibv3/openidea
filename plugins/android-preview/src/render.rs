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
}

/// Builds the full standalone HTML document (toolbar + screens).
pub fn page(title: &str, screens: &[Screen]) -> String {
    let mut body = String::new();
    for s in screens {
        let w = s.width.unwrap_or(DEVICE_W);
        let h = s.height.unwrap_or(DEVICE_H);
        let bg = s.bg.as_deref().unwrap_or("var(--ap-window)");
        body.push_str("<section class=\"ap-frame\">");
        if let Some(c) = &s.caption {
            body.push_str(&format!("<div class=\"ap-caption\">{}</div>", esc(c)));
        }
        body.push_str(&format!(
            "<div class=\"ap-screen\" style=\"width:{}px;height:{}px;background:{};\">",
            fmt(w),
            fmt(h),
            esc(bg)
        ));
        for root in &s.roots {
            body.push_str(&view_in(root, true, None));
        }
        body.push_str("</div></section>");
    }
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>{title}</title><style>{CSS}</style></head>\
<body>\
<div class=\"ap-bar\" id=\"ap-bar\">\
<span class=\"ap-title\">{title}</span>\
<label>Device <select id=\"ap-device\">\
<option value=\"411x891\" selected>Pixel 5 · 411×891</option>\
<option value=\"360x800\">Small · 360×800</option>\
<option value=\"800x1280\">Tablet · 800×1280</option>\
</select></label>\
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
        body = body
    )
}

// ---------------------------------------------------------------------------
// View → HTML
// ---------------------------------------------------------------------------

/// Renders one node. `root` nodes stretch to fill the device screen; children
/// receive their parent so the positioning model can switch to absolute.
fn view_in(n: &Node, root: bool, parent: Option<&Node>) -> String {
    if n.style.visibility == Visibility::Gone || n.style.guideline.is_some() {
        return String::new();
    }
    let style = view_style(n, root, parent);
    let inner = content(n);
    let hidden = if n.style.visibility == Visibility::Invisible { "visibility:hidden;" } else { "" };
    format!("<div class=\"ap-view\" style=\"{style}{hidden}\">{inner}</div>")
}

fn content(n: &Node) -> String {
    let kids: String = n.children.iter().map(|c| view_in(c, false, Some(n))).collect();
    match n.kind.as_str() {
        "TextView" | "Button" | "OutlinedButton" | "TextButton" | "ElevatedButton" | "FilledTonalButton"
        | "IconButton" => {
            let text = n.text.clone().or_else(|| n.sub.clone()).unwrap_or_else(|| n.kind.to_owned());
            format!("<span class=\"ap-text\" style=\"{}\">{}</span>{kids}", text_style(&n.style), esc(&text))
        }
        "EditText" => {
            let shown = n.text.clone().or_else(|| n.hint.clone()).unwrap_or_else(|| "…".to_owned());
            let hint = if n.text.is_none() { "color:var(--ap-hint);font-style:italic;" } else { "" };
            format!("<span class=\"ap-text\" style=\"{}{hint}\">{}</span>{kids}", text_style(&n.style), esc(&shown))
        }
        "Icon" => {
            let label = n.sub.clone().unwrap_or_else(|| "icon".to_owned());
            format!("<span class=\"ap-icon\">{}</span>{kids}", esc(&label))
        }
        "ImageView" | "ImageButton" => {
            let label = n.sub.clone().unwrap_or_else(|| "image".to_owned());
            format!("<span class=\"ap-image\">🖼 {}{kids}</span>", esc(&label))
        }
        "CheckBox" => {
            let label = n.text.clone().unwrap_or_default();
            let checked = n.style.checked.unwrap_or(false);
            format!(
                "<label class=\"ap-check\"><input type=\"checkbox\"{} disabled><span>{}</span></label>{kids}",
                if checked { " checked" } else { "" },
                esc(&label)
            )
        }
        "RadioButton" => {
            let label = n.text.clone().unwrap_or_default();
            let checked = n.style.checked.unwrap_or(false);
            format!(
                "<label class=\"ap-check\"><input type=\"radio\"{} disabled><span>{}</span></label>{kids}",
                if checked { " checked" } else { "" },
                esc(&label)
            )
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
            format!("<span class=\"ap-empty\">&lt;{}&gt;</span>", esc(&n.kind))
        }
        _ => match &n.sub {
            Some(label) if n.kind == "Placeholder" => {
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

/// Shared box styles for every view (positioning, size, spacing, paint).
fn view_style(n: &Node, root: bool, parent: Option<&Node>) -> String {
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
        "ScrollView" | "NestedScrollView" | "LazyColumn" => {
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
            out.push_str("position:relative;");
        }
        _ => {}
    }

    // Size + positioning.
    let abs_child = parent.map(|p| uses_absolute_children(p.kind.as_str())).unwrap_or(false);
    if root {
        out.push_str("width:100%;height:100%;overflow:hidden;");
    } else if abs_child {
        out.push_str("position:absolute;");
        match parent.unwrap().kind.as_str() {
            "RelativeLayout" | "ConstraintLayout" => match &s.pos {
                Some(pos) => {
                    if let Some(l) = pos.left {
                        out.push_str(&format!("left:{}px;", fmt(l)));
                    }
                    if let Some(t) = pos.top {
                        out.push_str(&format!("top:{}px;", fmt(t)));
                    }
                    if let Some(r) = pos.right {
                        out.push_str(&format!("right:{}px;", fmt(r)));
                    }
                    if let Some(b) = pos.bottom {
                        out.push_str(&format!("bottom:{}px;", fmt(b)));
                    }
                }
                None => out.push_str("left:0;top:0;"),
            },
            _ => push_frame_anchor(&mut out, s.layout_gravity.unwrap_or_default(), s),
        }
        push_abs_size(&mut out, s);
    } else {
        push_flow_size(&mut out, s);
    }

    // Box paint.
    if let Some(bg) = &s.bg_color {
        out.push_str(&format!("background:{};", esc(bg)));
    } else if let Some(shape) = &s.shape {
        if let Some(fill) = &shape.fill {
            out.push_str(&format!("background:{};", esc(fill)));
        }
    }
    if let Some(r) = s.radius {
        out.push_str(&format!("border-radius:{}px;", fmt(r)));
    } else if let Some(shape) = &s.shape {
        let c = &shape.corners;
        out.push_str(&format!("border-radius:{}px {}px {}px {}px;", fmt(c[0]), fmt(c[1]), fmt(c[2]), fmt(c[3])));
    }
    if let Some((w, color)) = &s.border {
        out.push_str(&format!("border:{}px solid {};", fmt(*w), esc(color)));
    } else if let Some(shape) = &s.shape {
        if let Some((w, color)) = &shape.stroke {
            out.push_str(&format!("border:{}px solid {};", fmt(*w), esc(color)));
        }
    }
    if let Some(a) = s.alpha {
        out.push_str(&format!("opacity:{};", fmt(a)));
    }
    if let Some(e) = s.elevation {
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

    // Spacing.
    push_edges(&mut out, "margin", &s.margin);
    push_edges(&mut out, "padding", &s.padding);

    out
}

/// FrameLayout-style stacking anchor for one child.
fn push_frame_anchor(out: &mut String, g: Gravity, s: &Style) {
    let match_w = s.width == Some(Size::Match);
    let match_h = s.height == Some(Size::Match);
    let mut tx = false;
    let mut ty = false;
    if match_w {
        out.push_str("left:0;right:0;");
    } else {
        match g.h {
            HAlign::Center => {
                out.push_str("left:50%;");
                tx = true;
            }
            HAlign::End => out.push_str("right:0;"),
            _ => out.push_str("left:0;"),
        }
    }
    if match_h {
        out.push_str("top:0;bottom:0;");
    } else {
        match g.v {
            VAlign::Center => {
                out.push_str("top:50%;");
                ty = true;
            }
            VAlign::Bottom => out.push_str("bottom:0;"),
            _ => out.push_str("top:0;"),
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

fn push_abs_size(out: &mut String, s: &Style) {
    match s.width {
        Some(Size::Px(v)) => out.push_str(&format!("width:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("width:{}%;", fmt(p))),
        Some(Size::Wrap) => out.push_str("width:fit-content;"),
        Some(Size::Match) | None => {} // anchored by left/right (or inferred)
    }
    match s.height {
        Some(Size::Px(v)) => out.push_str(&format!("height:{}px;", fmt(v))),
        Some(Size::Percent(p)) => out.push_str(&format!("height:{}%;", fmt(p))),
        Some(Size::Wrap) => out.push_str("height:fit-content;"),
        Some(Size::Match) | None => {}
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

fn text_style(s: &Style) -> String {
    let mut out = String::new();
    let fg = s.fg.clone().or_else(|| s.tint.clone()).unwrap_or_else(|| "var(--ap-fg)".to_owned());
    out.push_str(&format!("color:{};", esc(&fg)));
    if let Some(fs) = s.font_size {
        out.push_str(&format!("font-size:{}px;", fmt(fs)));
    }
    if s.bold == Some(true) {
        out.push_str("font-weight:600;");
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
  --ap-outline:#CAC4D0; --ap-surface:#FEF7FF; --ap-chrome:#EEEBF5; }
body.ap-dark { --ap-window:#121212; --ap-fg:#E6E1E5; --ap-hint:#9A9A9A;
  --ap-surface:#1E1B1E; --ap-chrome:#262229; }
* { box-sizing:border-box; }
html,body { margin:0; }
body { font-family:Roboto,"Segoe UI",sans-serif; background:#B7B4BD; color:#1B1B1B; }
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
.ap-caption { font-size:12px; color:#33333f; }
.ap-screen { position:relative; display:flex; flex-direction:column; overflow:hidden;
  box-shadow:0 6px 24px rgba(0,0,0,.3); border-radius:8px; transform-origin:top left; }
.ap-view { min-width:0; min-height:0; }
.ap-text { display:block; }
.ap-icon { display:inline-flex; align-items:center; justify-content:center;
  width:24px; height:24px; border-radius:50%; background:rgba(103,80,164,.15);
  color:var(--ap-primary); font-size:11px; overflow:hidden; }
.ap-image { display:inline-flex; align-items:center; justify-content:center; gap:4px;
  min-width:48px; min-height:48px; background:rgba(0,0,0,.08); color:var(--ap-hint);
  border-radius:4px; font-size:11px; }
.ap-check { display:inline-flex; align-items:center; gap:8px; }
.ap-switch { display:inline-block; width:36px; height:16px; border-radius:8px;
  background:var(--ap-outline); position:relative; transition:background .15s; }
.ap-switch.ap-on { background:var(--ap-primary); }
.ap-switch .ap-knob { position:absolute; top:-3px; left:0; width:22px; height:22px;
  border-radius:50%; background:#FAFAFA; box-shadow:0 1px 3px rgba(0,0,0,.4);
  transition:left .15s; }
.ap-switch.ap-on .ap-knob { left:14px; }
.ap-seek { display:inline-block; width:160px; height:4px; margin:14px 0;
  border-radius:2px; background:var(--ap-outline); position:relative; }
.ap-seek-fill { position:absolute; inset:0 auto 0 0; border-radius:2px;
  background:var(--ap-primary); }
.ap-bar-track { display:inline-block; width:120px; height:6px; border-radius:3px;
  background:rgba(103,80,164,.25); position:relative; overflow:hidden; }
.ap-bar-fill { position:absolute; inset:0 auto 0 0; background:var(--ap-primary); }
.ap-spinner { display:inline-block; width:28px; height:28px; border-radius:50%;
  border:3px solid rgba(103,80,164,.25); border-top-color:var(--ap-primary);
  animation:ap-spin 0.9s linear infinite; }
@keyframes ap-spin { to { transform:rotate(360deg); } }
.ap-empty { display:inline-block; padding:6px 10px; border:1.5px dashed var(--ap-outline);
  border-radius:6px; color:var(--ap-hint); font-size:11px; }
"#;

const JS: &str = r#"
(function () {
  var device = document.getElementById('ap-device');
  var zoom = document.getElementById('ap-zoom');
  var dark = document.getElementById('ap-dark');
  function apply() {
    var parts = device.value.split('x');
    var w = parseInt(parts[0], 10), h = parseInt(parts[1], 10);
    document.querySelectorAll('.ap-screen').forEach(function (s) {
      s.style.width = w + 'px';
      s.style.height = h + 'px';
      s.style.zoom = zoom.value;
    });
  }
  device.addEventListener('change', apply);
  zoom.addEventListener('change', apply);
  dark.addEventListener('click', function () { document.body.classList.toggle('ap-dark'); });
  apply();
})();
"#;
