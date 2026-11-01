//! Kotlin scanner for Jetpack Compose `@Preview` functions.
//!
//! A small lexer (string-, char-, template- and comment-aware, including
//! nested block comments) feeds a pragmatic parser that finds every
//! `@Preview` annotation (stacked annotations → one frame each), then builds a
//! [`Node`] tree from the composable calls in the annotated function's body:
//! Column/Row/Box, Text, buttons, Scaffold/TopAppBar and the rest of the
//! Material 3 set, Material colors, a `Modifier` chain subset, resource
//! references resolved through [`Resources`] when the caller supplies them,
//! and placeholders for everything it cannot understand.
//!
//! Pure Rust, no host calls: fully unit-testable natively.

use std::collections::HashMap;

use crate::layout::{Edges, FlexDir, Gravity, HAlign, Image, Node, Size, Style, VAlign};
use crate::resources::{
    M3_ERROR, M3_ON_SURFACE_VARIANT, M3_OUTLINE, M3_OUTLINE_VARIANT, M3_PRIMARY, M3_SURFACE, M3_SURFACE_VARIANT,
    ResDrawable, Resources, ShapeDrawable, color_css, theme_attr_css,
};

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    /// `0x…` literal kept as the exact u64 (f32 would lose color bits).
    Hex(u64),
    Str(String),
    Punct(char),
}

#[derive(Debug, Clone)]
struct TokPos {
    tok: Tok,
    line: usize,
}

fn lex(src: &str) -> Result<Vec<TokPos>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let start_line = line;
                let mut depth = 1; // Kotlin block comments nest
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if depth > 0 {
                    return Err(format!("unterminated block comment (started at line {start_line})"));
                }
            }
            '"' if chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') => {
                // Raw string ("""…"""), may span lines, no escapes.
                i += 3;
                let mut text = String::new();
                loop {
                    if i >= chars.len() {
                        return Err(format!("unterminated raw string (line {line})"));
                    }
                    if chars[i] == '"' && chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') {
                        i += 3;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                toks.push(TokPos { tok: Tok::Str(text), line });
            }
            '"' => {
                i += 1;
                let mut text = String::new();
                loop {
                    if i >= chars.len() {
                        return Err(format!("unterminated string (line {line})"));
                    }
                    match chars[i] {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\n' => return Err(format!("unterminated string (line {line})")),
                        '\\' => {
                            i += 1;
                            if i < chars.len() {
                                text.push(match chars[i] {
                                    'n' => '\n',
                                    't' => '\t',
                                    'r' => '\r',
                                    other => other,
                                });
                                i += 1;
                            }
                        }
                        other => {
                            text.push(other);
                            i += 1;
                        }
                    }
                }
                toks.push(TokPos { tok: Tok::Str(text), line });
            }
            '\'' => {
                // Char literal — skip its content.
                i += 1;
                if i < chars.len() && chars[i] == '\\' {
                    i += 2;
                } else if i < chars.len() {
                    i += 1;
                }
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let start = i;
                if c == '0' && matches!(chars.get(i + 1), Some('x') | Some('X')) {
                    i += 2;
                    while i < chars.len() && chars[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    let hex: String = chars[start + 2..i].iter().collect();
                    let value = u64::from_str_radix(&hex, 16).unwrap_or(0);
                    toks.push(TokPos { tok: Tok::Hex(value), line });
                } else {
                    let mut text = String::new();
                    while i < chars.len()
                        && (chars[i].is_ascii_digit()
                            || (chars[i] == '.' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == '_')))
                    {
                        text.push(chars[i]);
                        i += 1;
                    }
                    // Numeric suffixes: f / F / L / u.
                    if matches!(chars.get(i), Some('f') | Some('F') | Some('L') | Some('u')) {
                        i += 1;
                    }
                    let num = text.replace('_', "").parse::<f64>().unwrap_or(0.0);
                    toks.push(TokPos { tok: Tok::Num(num), line });
                }
            }
            c if c.is_alphabetic() || c == '_' || c == '`' => {
                if c == '`' {
                    i += 1;
                }
                let mut text = String::new();
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    text.push(chars[i]);
                    i += 1;
                }
                if c == '`' && i < chars.len() && chars[i] == '`' {
                    i += 1;
                }
                toks.push(TokPos { tok: Tok::Ident(text), line });
            }
            other => {
                toks.push(TokPos { tok: Tok::Punct(other), line });
                i += 1;
            }
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A parsed argument expression.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Num(f32),
    Str(String),
    Bool(bool),
    /// CSS color.
    Color(String),
    /// Enum-style path, e.g. `Alignment.CenterVertically`.
    Ident(String),
    /// `N.dp` / `N.sp`.
    Dp(f32),
    /// `Arrangement.spacedBy(N.dp)` plus its optional cross-axis alignment.
    Gap(f32, Option<String>),
    /// A drawable resolved through [`Resources`] (`painterResource`, a bare
    /// `R.drawable.x`): inline vector SVG or a bitmap `data:` URI.
    Paint(Image),
    /// A `<shape>` drawable resolved through [`Resources`].
    Shape(ShapeDrawable),
    Modifier(Vec<ModOp>),
    /// A lambda: parsed child nodes plus a leading scalar when the body is a
    /// plain number (`progress = { 0.5f }`).
    Lambda(Vec<Node>, Option<f32>),
    /// An unrecognized call; name + parsed args kept for best-effort use.
    Call {
        name: String,
        args: Box<HashMap<String, Val>>,
    },
    /// Unresolvable expression; carries display text.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
struct ModOp {
    name: String,
    args: Vec<Val>,
}

struct Cursor {
    toks: Vec<TokPos>,
    pos: usize,
}

impl Cursor {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_at(&self, off: usize) -> Option<&Tok> {
        self.toks.get(self.pos + off).map(|t| &t.tok)
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    /// Reads a balanced group's inner token range (cursor on the opener); the
    /// cursor ends just past the closer.
    fn group_range(&mut self, open: char, close: char) -> Option<(usize, usize)> {
        if self.peek() != Some(&Tok::Punct(open)) {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1i32;
        while let Some(t) = self.toks.get(self.pos) {
            match t.tok {
                Tok::Punct(c) if c == open => {
                    depth += 1;
                    self.pos += 1;
                }
                Tok::Punct(c) if c == close => {
                    depth -= 1;
                    self.pos += 1;
                    if depth <= 0 {
                        return Some((start, self.pos - 1));
                    }
                }
                _ => self.pos += 1,
            }
        }
        None
    }

    fn group_range_at(&self, from: usize, open: char, close: char) -> Result<(usize, usize), usize> {
        let mut probe = from;
        if self.toks.get(probe).map(|t| &t.tok) != Some(&Tok::Punct(open)) {
            return Err(self.toks.get(probe).map(|t| t.line).unwrap_or(0));
        }
        probe += 1;
        let start = probe;
        let mut depth = 1i32;
        let mut last_open = self.toks.get(from).map(|t| t.line).unwrap_or(0);
        while let Some(t) = self.toks.get(probe) {
            match t.tok {
                Tok::Punct(c) if c == open => {
                    depth += 1;
                    last_open = t.line;
                    probe += 1;
                }
                Tok::Punct(c) if c == close => {
                    depth -= 1;
                    probe += 1;
                    if depth <= 0 {
                        return Ok((start, probe - 1));
                    }
                }
                _ => probe += 1,
            }
        }
        Err(last_open)
    }
}

/// Parses `name = value` / positional values from a raw token slice.
fn parse_named_args(toks: &[TokPos], out: &mut HashMap<String, Val>, file: &File, scope: &Scope) {
    let mut cur = Cursor { toks: toks.to_vec(), pos: 0 };
    let mut positional = 0usize;
    while cur.pos < cur.toks.len() {
        let key = if let (Some(Tok::Ident(k)), Some(Tok::Punct('='))) = (cur.peek(), cur.peek_at(1)) {
            let k = k.clone();
            cur.pos += 2;
            Some(k)
        } else {
            None
        };
        if cur.pos >= cur.toks.len() {
            break;
        }
        let val = parse_val(&mut cur, file, scope);
        let named = key.is_some();
        out.insert(key.unwrap_or_else(|| format!("@{positional}")), val);
        if !named {
            positional += 1;
        }
        if cur.peek() == Some(&Tok::Punct(',')) {
            cur.pos += 1;
        }
    }
}

/// Parses one value expression from the cursor.
fn parse_val(cur: &mut Cursor, file: &File, scope: &Scope) -> Val {
    match cur.peek().cloned() {
        Some(Tok::Punct('-')) => {
            cur.bump();
            match parse_val(cur, file, scope) {
                Val::Num(n) => Val::Num(-n),
                Val::Dp(n) => Val::Dp(-n),
                other => other,
            }
        }
        Some(Tok::Num(n)) => {
            cur.bump();
            // 16.dp / 16.sp (suffix f/F/L already stripped by the lexer)
            if cur.peek() == Some(&Tok::Punct('.')) {
                if let Some(Tok::Ident(unit)) = cur.peek_at(1) {
                    if unit == "dp" || unit == "sp" {
                        cur.pos += 2;
                        return Val::Dp(n as f32);
                    }
                }
            }
            Val::Num(n as f32)
        }
        Some(Tok::Hex(v)) => {
            cur.bump();
            Val::Color(argb_css(v))
        }
        Some(Tok::Str(s)) => {
            cur.bump();
            Val::Str(s)
        }
        Some(Tok::Punct('{')) => {
            let (start, end) = cur.group_range('{', '}').unwrap_or((cur.pos, cur.pos));
            parse_lambda(&cur.toks[start..end], file, scope)
        }
        Some(Tok::Punct('(')) => {
            let (start, end) = cur.group_range('(', ')').unwrap_or((cur.pos, cur.pos));
            let mut inner = Cursor { toks: cur.toks[start..end].to_vec(), pos: 0 };
            parse_val(&mut inner, file, scope)
        }
        Some(Tok::Ident(id)) => parse_ident_val(cur, &id, file, scope),
        Some(other) => {
            cur.bump();
            Val::Unknown(tok_text(&other))
        }
        None => Val::Unknown(String::new()),
    }
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Str(s) => s.clone(),
        Tok::Num(n) => format!("{n}"),
        Tok::Hex(v) => format!("0x{v:X}"),
        Tok::Punct(c) => c.to_string(),
    }
}

/// `{ … }`: composable calls when present, else a leading scalar, else an
/// assignment block (`graphicsLayer { rotationZ = 45f }`) kept as a Call so
/// named modifier ops can still read their properties.
fn parse_lambda(toks: &[TokPos], file: &File, scope: &Scope) -> Val {
    let nodes = parse_nodes(toks, file, scope);
    if !nodes.is_empty() {
        return Val::Lambda(nodes, None);
    }
    let mut cur = Cursor { toks: toks.to_vec(), pos: 0 };
    match parse_val(&mut cur, file, scope) {
        Val::Num(n) => return Val::Lambda(Vec::new(), Some(n)),
        Val::Dp(n) => return Val::Lambda(Vec::new(), Some(n)),
        _ => {}
    }
    if has_assignment(toks) {
        let kept: Vec<TokPos> = toks.iter().filter(|t| t.tok != Tok::Punct(';')).cloned().collect();
        let mut args = HashMap::new();
        parse_named_args(&kept, &mut args, file, scope);
        return Val::Call { name: ASSIGN_BLOCK.to_owned(), args: Box::new(args) };
    }
    Val::Lambda(Vec::new(), None)
}

/// Marker name for a `{ key = value; … }` lambda parsed as assignments.
const ASSIGN_BLOCK: &str = "{}";

fn has_assignment(toks: &[TokPos]) -> bool {
    toks.windows(2).any(|w| matches!((&w[0].tok, &w[1].tok), (Tok::Ident(_), Tok::Punct('='))))
}

/// Parses an identifier-headed value: Modifier chains, Color, MaterialTheme,
/// enums, function calls, bare references.
fn parse_ident_val(cur: &mut Cursor, id: &str, file: &File, scope: &Scope) -> Val {
    match id {
        "true" => {
            cur.bump();
            Val::Bool(true)
        }
        "false" => {
            cur.bump();
            Val::Bool(false)
        }
        "Modifier" => {
            cur.bump();
            let mut ops = Vec::new();
            while cur.peek() == Some(&Tok::Punct('.')) {
                let Some(Tok::Ident(name)) = cur.peek_at(1).cloned() else { break };
                cur.pos += 2;
                let mut args = Vec::new();
                if cur.peek() == Some(&Tok::Punct('(')) {
                    let (start, end) = cur.group_range('(', ')').unwrap_or((cur.pos, cur.pos));
                    let mut inner = Cursor { toks: cur.toks[start..end].to_vec(), pos: 0 };
                    while inner.pos < inner.toks.len() {
                        // `name = value` → keep the name as an Ident marker so
                        // apply_* can recover named variants.
                        if let (Some(Tok::Ident(k)), Some(Tok::Punct('='))) = (inner.peek(), inner.peek_at(1)) {
                            args.push(Val::Ident(k.clone()));
                            inner.pos += 2;
                        }
                        args.push(parse_val(&mut inner, file, scope));
                        if inner.peek() == Some(&Tok::Punct(',')) {
                            inner.pos += 1;
                        } else {
                            break;
                        }
                    }
                } else if cur.peek() == Some(&Tok::Punct('{')) {
                    // Trailing-lambda op: `graphicsLayer { rotationZ = 45f }`.
                    let (start, end) = cur.group_range('{', '}').unwrap_or((cur.pos, cur.pos));
                    args.push(parse_lambda(&cur.toks[start..end], file, scope));
                }
                ops.push(ModOp { name, args });
            }
            Val::Modifier(ops)
        }
        "Color" => {
            cur.bump();
            if cur.peek() == Some(&Tok::Punct('(')) {
                let (start, _end) = cur.group_range('(', ')').unwrap_or((cur.pos, cur.pos));
                if let Some(Tok::Hex(v)) = cur.toks.get(start).map(|t| &t.tok) {
                    return Val::Color(argb_css(*v));
                }
                return Val::Unknown("Color(…)".to_owned());
            }
            let mut name = String::new();
            if cur.peek() == Some(&Tok::Punct('.')) {
                if let Some(Tok::Ident(n)) = cur.peek_at(1).cloned() {
                    name = n;
                    cur.pos += 2;
                }
            }
            // `Color.Unspecified` must not paint anything: keep it a non-colour
            // Ident so `get_color` skips it instead of resolving to transparent.
            if name == "Unspecified" {
                return Val::Ident("Color.Unspecified".to_owned());
            }
            Val::Color(named_color(&name).unwrap_or_else(|| "rgba(0,0,0,0)".into()))
        }
        "MaterialTheme" => {
            cur.bump();
            let mut path = vec![id.to_owned()];
            while cur.peek() == Some(&Tok::Punct('.')) {
                let Some(Tok::Ident(n)) = cur.peek_at(1).cloned() else { break };
                path.push(n);
                cur.pos += 2;
            }
            // `MaterialTheme.typography.titleLarge` and `…shapes.small` are not
            // colours; keep the path so the caller can use the text style.
            if matches!(path.get(1).map(String::as_str), Some("typography" | "shapes")) {
                return Val::Ident(path.join("."));
            }
            let attr = path.last().map(String::as_str).unwrap_or("");
            Val::Color(theme_attr_css(attr).unwrap_or("rgba(0,0,0,0)").to_owned())
        }
        _ => {
            // Possibly a call: Name(args…) or a dotted path / bare reference.
            let mut path = vec![id.to_owned()];
            let mut probe = cur.pos + 1;
            while cur.toks.get(probe).map(|t| &t.tok) == Some(&Tok::Punct('.')) {
                match cur.toks.get(probe + 1).map(|t| &t.tok) {
                    Some(Tok::Ident(n)) => {
                        path.push(n.clone());
                        probe += 2;
                    }
                    _ => break,
                }
            }
            if cur.toks.get(probe).map(|t| &t.tok) == Some(&Tok::Punct('(')) {
                cur.pos = probe;
                let (start, end) = cur.group_range('(', ')').unwrap_or((cur.pos, cur.pos));
                let mut args = HashMap::new();
                parse_named_args(&cur.toks[start..end], &mut args, file, scope);
                let last = path.last().map(String::as_str).unwrap_or("");
                // Arrangement.spacedBy(8.dp, Alignment.CenterVertically)
                if last == "spacedBy" {
                    let gap = args.values().find_map(|v| match v {
                        Val::Dp(n) | Val::Num(n) => Some(*n),
                        _ => None,
                    });
                    let align = args.values().find_map(|v| match v {
                        Val::Ident(p) => Some(p.clone()),
                        _ => None,
                    });
                    return Val::Gap(gap.unwrap_or(0.0), align);
                }
                if let Some(v) = resolve_resource_call(last, &args, file) {
                    return v;
                }
                return Val::Call { name: path.join("."), args: Box::new(args) };
            }
            cur.pos = probe;
            let joined = path.join(".");
            if let Some(v) = resolve_r_ident(&joined, file) {
                return v;
            }
            Val::Ident(joined)
        }
    }
}

/// `0xFF6750A4` → `#6750A4`; alpha < ff → rgba.
fn argb_css(v: u64) -> String {
    let argb = (v & 0xFFFF_FFFF) as u32;
    let a = ((argb >> 24) & 0xFF) as u8;
    let rgb = format!("#{:06X}", argb & 0xFFFFFF);
    if a == 255 {
        rgb
    } else {
        let r = (argb >> 16) & 0xFF;
        let g = (argb >> 8) & 0xFF;
        let b = argb & 0xFF;
        format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0)
    }
}

fn named_color(name: &str) -> Option<String> {
    color_css(match name {
        "Black" => "#FF000000",
        "White" => "#FFFFFFFF",
        "Red" => "#FFFF0000",
        "Green" => "#FF00FF00",
        "Blue" => "#FF0000FF",
        "Yellow" => "#FFFFFF00",
        "Cyan" => "#FF00FFFF",
        "Magenta" => "#FFFF00FF",
        "Gray" => "#FF888888",
        "LightGray" => "#FFCCCCCC",
        "DarkGray" => "#FF444444",
        "Transparent" => "#00000000",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Resource resolution (`stringResource(R.string.x)` and friends)
// ---------------------------------------------------------------------------

/// `stringResource` / `colorResource` / `dimensionResource` / `painterResource`
/// resolved against the resource tables; `None` keeps the call so the existing
/// `stringResource(…)` label fallback still shows when unresolvable.
fn resolve_resource_call(name: &str, args: &HashMap<String, Val>, file: &File) -> Option<Val> {
    // A bare `R.…` argument already resolved while it was parsed itself.
    let first = args.get("@0").or_else(|| args.get("id"));
    match name {
        "stringResource" => match first {
            Some(Val::Str(s)) => return Some(Val::Str(s.clone())),
            _ => {}
        },
        "colorResource" => match first {
            Some(Val::Color(c)) => return Some(Val::Color(c.clone())),
            _ => {}
        },
        "dimensionResource" => match first {
            Some(Val::Dp(n)) => return Some(Val::Dp(*n)),
            _ => {}
        },
        "painterResource" | "imageResource" => match first {
            Some(v @ (Val::Paint(_) | Val::Shape(_))) => return Some(v.clone()),
            _ => {}
        },
        _ => return None,
    }
    let res = file.res?;
    match name {
        "stringResource" => {
            let id = r_ref(args, "R.string.")?;
            res.text(&format!("@string/{id}"), 0).map(Val::Str)
        }
        "colorResource" => {
            let id = r_ref(args, "R.color.")?;
            res.color(&format!("@color/{id}"), 0).map(Val::Color)
        }
        "dimensionResource" => {
            let id = r_ref(args, "R.dimen.")?;
            res.dimen(&format!("@dimen/{id}"), 0).map(Val::Dp)
        }
        "painterResource" | "imageResource" => {
            let id = r_ref(args, "R.drawable.").or_else(|| r_ref(args, "R.mipmap."))?;
            resolve_drawable(res, &id)
        }
        _ => None,
    }
}

/// The resource name of an `R.<type>.<name>` reference among the arguments.
fn r_ref(args: &HashMap<String, Val>, prefix: &str) -> Option<String> {
    args.get("@0")
        .or_else(|| args.get("id"))
        .into_iter()
        .chain(args.values())
        .find_map(|v| match v {
            Val::Ident(p) => p.strip_prefix(prefix).map(|n| n.to_owned()),
            _ => None,
        })
}

/// A bare `R.string.x` / `R.color.x` / `R.dimen.x` / `R.drawable.x` used
/// directly as an argument resolves like its `*Resource(…)` wrapper.
fn resolve_r_ident(path: &str, file: &File) -> Option<Val> {
    let res = file.res?;
    if let Some(id) = path.strip_prefix("R.string.") {
        return res.text(&format!("@string/{id}"), 0).map(Val::Str);
    }
    if let Some(id) = path.strip_prefix("R.color.") {
        return res.color(&format!("@color/{id}"), 0).map(Val::Color);
    }
    if let Some(id) = path.strip_prefix("R.dimen.") {
        return res.dimen(&format!("@dimen/{id}"), 0).map(Val::Dp);
    }
    let id = path.strip_prefix("R.drawable.").or_else(|| path.strip_prefix("R.mipmap."))?;
    resolve_drawable(res, id)
}

fn resolve_drawable(res: &Resources, name: &str) -> Option<Val> {
    Some(match res.drawable(name)? {
        ResDrawable::Svg(svg) => Val::Paint(Image::Svg(svg)),
        ResDrawable::Image(uri) => Val::Paint(Image::Uri(uri)),
        ResDrawable::Shape(shape) => Val::Shape(shape),
    })
}

/// `MaterialTheme.typography.titleLarge` → `(22sp, medium)`; the M3 type
/// scale, so text without an explicit `fontSize` matches the design system.
fn typography(path: &str) -> Option<(f32, bool)> {
    let name = path.rsplit('.').next().unwrap_or(path);
    Some(match name {
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
        _ => return None,
    })
}

/// `RoundedCornerShape(8.dp)` → 8; `CircleShape` → 999.
fn shape_radius(val: &Val) -> Option<f32> {
    match val {
        Val::Ident(path) => {
            if path.rsplit('.').next() == Some("CircleShape") {
                Some(999.0)
            } else {
                None
            }
        }
        Val::Call { name, args } if name.ends_with("RoundedCornerShape") => args.values().find_map(|v| match v {
            Val::Dp(n) | Val::Num(n) => Some(*n),
            _ => None,
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// File scope: composables that can be inlined into a preview
// ---------------------------------------------------------------------------

/// One composable function declared in the previewed file.
#[derive(Debug, Clone)]
struct FunDef {
    /// `(name, default value)` per parameter: a preview renders the defaults
    /// for arguments nobody passes, like the compiler would.
    params: Vec<(String, Option<Val>)>,
    /// Token range of the body (`{ … }`), absolute in the file.
    body: Option<(usize, usize)>,
}

/// The previewed file: its tokens, every top-level function and (optionally)
/// the resource tables that resolve `stringResource`/`painterResource`/`R.…`.
/// A `@Preview` body that calls another composable of the same file is
/// rendered by inlining that body, the way the preview tooling actually
/// composes it.
struct File<'a, 'r> {
    toks: Vec<TokPos>,
    funs: HashMap<String, FunDef>,
    res: Option<&'a Resources<'r>>,
}

/// Parameters in scope while building nodes of an inlined function.
struct Scope {
    params: HashMap<String, Val>,
    depth: usize,
    /// Inside a button/chip content: the container box already carries the
    /// label typography (render emits it), so `Text` leaves its size unset and
    /// inherits — what `ProvideTextStyle` does in Compose.
    inherited_text: bool,
}

impl Scope {
    fn root() -> Self {
        Self { params: HashMap::new(), depth: 0, inherited_text: false }
    }

    /// Same scope inside label-providing container content.
    fn with_inherited_text(&self) -> Self {
        Self { params: self.params.clone(), depth: self.depth, inherited_text: true }
    }
}

/// How deep a call chain of composables is followed before it degrades to a
/// placeholder (guards against recursive composables).
const MAX_INLINE_DEPTH: usize = 4;

/// Indexes every top-level `fun` in the file with its parameter names,
/// defaults and body range. Local functions, lambdas and interfaces are
/// skipped.
fn index_functions(file: &File) -> HashMap<String, FunDef> {
    let toks = &file.toks;
    let mut funs = HashMap::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i].tok {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => depth -= 1,
            Tok::Ident(id) if depth == 0 && id == "fun" => {
                // `fun [<T>] name ( params ) [: Type] { body }`
                let mut name_at = i + 1;
                if toks.get(name_at).map(|t| &t.tok) == Some(&Tok::Punct('<')) {
                    while name_at < toks.len() && toks[name_at].tok != Tok::Punct('>') {
                        name_at += 1;
                    }
                    name_at += 1;
                }
                let Some(Tok::Ident(name)) = toks.get(name_at).map(|t| t.tok.clone()) else {
                    i += 1;
                    continue;
                };
                let mut paren = name_at + 1;
                while toks.get(paren).map(|t| &t.tok) == Some(&Tok::Punct('<')) {
                    while paren < toks.len() && toks[paren].tok != Tok::Punct('>') {
                        paren += 1;
                    }
                    paren += 1;
                }
                if let Some(def) = fun_def(toks, paren, file) {
                    funs.entry(name).or_insert(def);
                }
                i = paren;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    funs
}

/// Parses `( params ) [: Type] { body }` starting at the opening paren.
fn fun_def(toks: &[TokPos], open: usize, file: &File) -> Option<FunDef> {
    if toks.get(open).map(|t| &t.tok) != Some(&Tok::Punct('(')) {
        return None;
    }
    let mut depth = 1i32;
    let mut pos = open + 1;
    let params_start = pos;
    while depth > 0 {
        match toks.get(pos)?.tok {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') => depth -= 1,
            _ => {}
        }
        pos += 1;
    }
    let params = param_defs(&toks[params_start..pos - 1], file);
    // Skip the optional return type, up to the body or an expression body.
    while let Some(t) = toks.get(pos) {
        match t.tok {
            Tok::Punct('{') | Tok::Punct('=') | Tok::Punct(';') => break,
            _ => pos += 1,
        }
    }
    if toks.get(pos).map(|t| &t.tok) != Some(&Tok::Punct('{')) {
        return Some(FunDef { params, body: None });
    }
    let start = pos + 1;
    let mut depth = 1i32;
    pos += 1;
    while depth > 0 {
        match toks.get(pos)?.tok {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => depth -= 1,
            _ => {}
        }
        pos += 1;
    }
    Some(FunDef { params, body: Some((start, pos - 1)) })
}

/// `title: String = "Hi", onDone: () -> Unit` → `[("title", Some(Str("Hi"))),
/// ("onDone", None)]`. Parameters split on depth-0 commas; the `->` of a
/// function type must not unbalance the split (depth clamps at 0).
fn param_defs(toks: &[TokPos], file: &File) -> Vec<(String, Option<Val>)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for i in 0..=toks.len() {
        let mut boundary = i == toks.len();
        if let Some(t) = toks.get(i).map(|t| &t.tok) {
            match t {
                Tok::Punct(',') if depth == 0 => boundary = true,
                Tok::Punct('(') | Tok::Punct('[') | Tok::Punct('<') => depth += 1,
                Tok::Punct(')') | Tok::Punct(']') | Tok::Punct('>') => depth = (depth - 1).max(0),
                _ => {}
            }
        }
        if boundary {
            if let Some(def) = param_def(&toks[start..i], file) {
                out.push(def);
            }
            start = i + 1;
        }
    }
    out
}

/// One `name: Type [= default]` parameter segment.
fn param_def(toks: &[TokPos], file: &File) -> Option<(String, Option<Val>)> {
    let mut name = None;
    let mut default_at = None;
    let mut depth = 0i32;
    for i in 0..toks.len() {
        match &toks[i].tok {
            Tok::Punct('(') | Tok::Punct('[') | Tok::Punct('<') => depth += 1,
            Tok::Punct(')') | Tok::Punct(']') | Tok::Punct('>') => depth = (depth - 1).max(0),
            Tok::Ident(n)
                if name.is_none() && depth == 0 && toks.get(i + 1).map(|t| &t.tok) == Some(&Tok::Punct(':')) =>
            {
                name = Some(n.clone());
            }
            Tok::Punct('=')
                if name.is_some()
                    && default_at.is_none()
                    && depth == 0
                    && toks.get(i + 1).map(|t| &t.tok) != Some(&Tok::Punct('=')) =>
            {
                default_at = Some(i + 1);
            }
            _ => {}
        }
    }
    let name = name?;
    let default = default_at.map(|at| {
        let mut cur = Cursor { toks: toks[at..].to_vec(), pos: 0 };
        parse_val(&mut cur, file, &Scope::root())
    });
    Some((name, default))
}

/// Replaces parameters by the arguments a caller passed.
fn substitute(v: &Val, params: &HashMap<String, Val>) -> Val {
    match v {
        Val::Ident(name) if params.contains_key(name) => params[name].clone(),
        Val::Modifier(ops) => Val::Modifier(
            ops.iter()
                .map(|op| ModOp {
                    name: op.name.clone(),
                    args: op.args.iter().map(|a| substitute(a, params)).collect(),
                })
                .collect(),
        ),
        Val::Call { name, args } => Val::Call {
            name: name.clone(),
            args: Box::new(args.iter().map(|(k, a)| (k.clone(), substitute(a, params))).collect()),
        },
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Preview discovery
// ---------------------------------------------------------------------------

/// One `@Preview` annotation's options plus the function it annotates.
#[derive(Debug, Clone)]
struct PreviewSpec {
    name: Option<String>,
    width_dp: Option<f32>,
    height_dp: Option<f32>,
    /// `device = "spec:width=…dp,height=…dp,orientation=…"` raw value.
    device: Option<String>,
    show_background: bool,
    background_color: Option<String>,
    fun_index: usize,
    fun_name: String,
}

/// One rendered preview frame.
#[derive(Debug, Clone)]
pub struct Frame {
    pub caption: String,
    pub root: Option<Node>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub show_background: bool,
    pub bg_color: Option<String>,
}

/// Scans the file for `@Preview` functions. Multiple `@Preview` annotations on
/// one function produce one frame each. A file without any `@Preview` returns
/// an empty list (the caller shows a friendly empty state, not an error).
#[cfg(test)]
pub fn scan(src: &str) -> Result<Vec<Frame>, String> {
    scan_with(src, None)
}

/// Like [`scan`], with the resource tables that resolve `stringResource(…)`,
/// `painterResource(…)`, bare `R.…` references and `@Preview` values.
pub fn scan_with(src: &str, res: Option<&Resources>) -> Result<Vec<Frame>, String> {
    let toks = lex(src)?;
    let mut file = File { toks, funs: HashMap::new(), res };
    let funs = index_functions(&file);
    file.funs = funs;
    let mut cur = Cursor { toks: file.toks.clone(), pos: 0 };
    let mut specs: Vec<PreviewSpec> = Vec::new();

    while cur.pos < cur.toks.len() {
        if cur.peek() != Some(&Tok::Punct('@')) {
            cur.bump();
            continue;
        }
        cur.bump(); // consume '@'
        // Fully-qualified or bare annotation name; only `…Preview` interests us.
        let mut last = String::new();
        while let Some(t) = cur.peek() {
            match t {
                Tok::Ident(id) => {
                    last = id.clone();
                    cur.bump();
                }
                Tok::Punct('.') => cur.bump(),
                _ => break,
            }
        }
        if last != "Preview" {
            continue;
        }
        let mut args: HashMap<String, Val> = HashMap::new();
        if cur.peek() == Some(&Tok::Punct('(')) {
            let Some((start, end)) = cur.group_range('(', ')') else {
                return Err("unbalanced @Preview arguments".into());
            };
            parse_named_args(&cur.toks[start..end], &mut args, &file, &Scope::root());
        }
        // Find the annotated function: annotations / modifiers may intervene.
        let mut probe = cur.pos;
        let mut fun_index = None;
        let mut fun_name = String::new();
        while let Some(t) = cur.toks.get(probe) {
            match &t.tok {
                Tok::Ident(id) => match id.as_str() {
                    "fun" => {
                        if let Some(Tok::Ident(name)) = cur.toks.get(probe + 1).map(|t| &t.tok) {
                            fun_index = Some(probe + 1);
                            fun_name = name.clone();
                        }
                        break;
                    }
                    "class" | "object" | "interface" | "val" | "var" => break,
                    _ => probe += 1, // private/internal/override/…
                },
                Tok::Punct(';') => break,
                Tok::Punct('@') => {
                    // Skip the following annotation: name is `ident ('.' ident)*`
                    // (never greedy across spaces, so `fun name` survives).
                    probe += 1;
                    if matches!(cur.toks.get(probe).map(|t| &t.tok), Some(Tok::Ident(_))) {
                        probe += 1;
                    }
                    while cur.toks.get(probe).map(|t| &t.tok) == Some(&Tok::Punct('.')) {
                        probe += 1;
                        if matches!(cur.toks.get(probe).map(|t| &t.tok), Some(Tok::Ident(_))) {
                            probe += 1;
                        }
                    }
                    if cur.toks.get(probe).map(|t| &t.tok) == Some(&Tok::Punct('(')) {
                        probe += 1;
                        let mut depth = 1i32;
                        while depth > 0 && probe < cur.toks.len() {
                            match cur.toks[probe].tok {
                                Tok::Punct('(') => depth += 1,
                                Tok::Punct(')') => depth -= 1,
                                _ => {}
                            }
                            probe += 1;
                        }
                    }
                }
                _ => break,
            }
        }
        let Some(fun_index) = fun_index else { continue };
        specs.push(PreviewSpec {
            name: match args.get("name") {
                Some(Val::Str(s)) => Some(s.clone()),
                _ => None,
            },
            width_dp: num_arg(&args, "widthDp"),
            height_dp: num_arg(&args, "heightDp"),
            device: match args.get("device") {
                Some(Val::Str(s)) => Some(s.clone()),
                _ => None,
            },
            show_background: matches!(args.get("showBackground"), Some(Val::Bool(true))),
            background_color: match args.get("backgroundColor") {
                Some(Val::Color(c)) => Some(c.clone()),
                _ => None,
            },
            fun_index,
            fun_name,
        });
    }

    // Multi-preview functions without explicit names get numbered captions.
    let mut per_fun: HashMap<&str, usize> = HashMap::new();
    let total_per_fun: HashMap<&str, usize> = specs.iter().fold(HashMap::new(), |mut m, s| {
        *m.entry(s.fun_name.as_str()).or_insert(0) += 1;
        m
    });
    let mut frames = Vec::new();
    for spec in &specs {
        let idx = per_fun.entry(spec.fun_name.as_str()).and_modify(|c| *c += 1).or_insert(0);
        let caption = match (&spec.name, *idx, total_per_fun.get(spec.fun_name.as_str()).copied().unwrap_or(1)) {
            (Some(n), _, _) => n.clone(),
            (None, i, total) if total > 1 => format!("{} · {}", spec.fun_name, i + 1),
            (None, _, _) => spec.fun_name.clone(),
        };
        frames.push(build_frame(&file, spec.fun_index, &caption, spec)?);
    }
    Ok(frames)
}

fn num_arg(args: &HashMap<String, Val>, key: &str) -> Option<f32> {
    match args.get(key) {
        Some(Val::Num(n)) | Some(Val::Dp(n)) => Some(*n),
        _ => None,
    }
}

/// `spec:width=411dp,height=891dp,dpi=420,orientation=landscape` → `(w, h)`;
/// landscape swaps the axes. Named device IDs and `uiMode`/`locale`-style
/// options carry no size and are ignored.
fn device_spec_size(spec: &str) -> Option<(f32, f32)> {
    let rest = spec.strip_prefix("spec:")?;
    let (mut w, mut h, mut landscape) = (None, None, false);
    for part in rest.split(',') {
        let Some((key, value)) = part.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "width" => w = value.trim_end_matches("dp").trim().parse().ok(),
            "height" => h = value.trim_end_matches("dp").trim().parse().ok(),
            "orientation" => landscape = value == "landscape",
            _ => {} // dpi, density…: nothing to size from
        }
    }
    let (w, h) = (w?, h?);
    Some(if landscape { (h, w) } else { (w, h) })
}

fn build_frame(file: &File, fun_index: usize, caption: &str, spec: &PreviewSpec) -> Result<Frame, String> {
    let toks = &file.toks;
    // fun name ( … ) [: Type] { body }
    let mut pos = fun_index;
    while matches!(toks.get(pos).map(|t| &t.tok), Some(Tok::Ident(_))) {
        pos += 1;
    }
    let body = if toks.get(pos).map(|t| &t.tok) == Some(&Tok::Punct('(')) {
        let mut depth = 1i32;
        pos += 1;
        while depth > 0 && pos < toks.len() {
            match toks[pos].tok {
                Tok::Punct('(') => depth += 1,
                Tok::Punct(')') => depth -= 1,
                _ => {}
            }
            pos += 1;
        }
        // Skip the optional return type (and generics) up to '{' or '='.
        let mut angle = 0i32;
        while let Some(t) = toks.get(pos) {
            match t.tok {
                Tok::Punct('<') => angle += 1,
                Tok::Punct('>') => angle -= 1,
                Tok::Punct('{') => break,
                Tok::Punct('=') | Tok::Punct(';') => break,
                _ if angle > 0 => {}
                _ => {}
            }
            pos += 1;
        }
        if toks.get(pos).map(|t| &t.tok) == Some(&Tok::Punct('{')) {
            let cursor = Cursor { toks: toks.to_vec(), pos };
            match cursor.group_range_at(pos, '{', '}') {
                Ok((start, end)) => Some((start, end)),
                Err(line) => return Err(format!("unbalanced braces in `{}` (opened at line {})", spec.fun_name, line)),
            }
        } else {
            None // expression body / abstract — nothing to render
        }
    } else {
        return Err(format!(
            "cannot find the body of `{}` (line {})",
            spec.fun_name,
            toks.get(fun_index).map(|t| t.line).unwrap_or(0)
        ));
    };
    // The preview function's own parameter defaults stand in for the arguments
    // nobody passes: a runnable `@Preview` is callable with no arguments, so
    // `fun Foo(title: String = "Hi")` renders "Hi", not an unresolved label.
    let root_scope = match file.funs.get(&spec.fun_name) {
        Some(def) => Scope {
            params: def.params.iter().filter_map(|(n, d)| d.as_ref().map(|v| (n.clone(), v.clone()))).collect(),
            depth: 0,
            inherited_text: false,
        },
        None => Scope::root(),
    };
    let root = body.map(|(start, end)| {
        let mut n = Node { kind: "Frame".into(), ..Default::default() };
        n.children = parse_nodes(&file.toks[start..end], &file, &root_scope);
        n
    });
    // Explicit `widthDp`/`heightDp` win over the `device = "spec:…"` size.
    let (device_w, device_h) = spec.device.as_deref().and_then(device_spec_size).unzip();
    Ok(Frame {
        caption: caption.to_owned(),
        root,
        width: spec.width_dp.or(device_w),
        height: spec.height_dp.or(device_h),
        show_background: spec.show_background,
        bg_color: spec.background_color.clone(),
    })
}

// ---------------------------------------------------------------------------
// Body → nodes
// ---------------------------------------------------------------------------

const CONTROL_KEYWORDS: [&str; 9] = ["if", "else", "for", "while", "when", "return", "repeat", "try", "catch"];

/// Parses a sequence of composable calls into nodes.
fn parse_nodes(toks: &[TokPos], file: &File, scope: &Scope) -> Vec<Node> {
    let mut cur = Cursor { toks: toks.to_vec(), pos: 0 };
    let mut out = Vec::new();
    while cur.pos < cur.toks.len() {
        let before = cur.pos;
        if let Some(nodes) = parse_call(&mut cur, file, scope) {
            out.extend(nodes);
        }
        if cur.pos == before {
            // Always make progress (unbalanced groups, stray tokens, …).
            cur.bump();
        }
    }
    out
}

/// Containers whose content inherits the container's label typography instead
/// of Compose's default `bodyLarge` (what `ProvideTextStyle(labelLarge)` does).
const LABEL_CONTAINERS: [&str; 6] =
    ["Button", "OutlinedButton", "TextButton", "ElevatedButton", "FilledTonalButton", "IconButton"];

/// Parses one `Name(args) { children }` call; returns None for non-calls. A
/// call to a composable of the same file expands to its body's nodes.
fn parse_call(cur: &mut Cursor, file: &File, scope: &Scope) -> Option<Vec<Node>> {
    let Tok::Ident(name) = cur.peek()? else {
        cur.bump();
        return None;
    };
    let name = name.clone();
    if CONTROL_KEYWORDS.contains(&name.as_str()) {
        // if (cond) { … } / when (x) { … } — skip entirely.
        cur.bump();
        if cur.peek() == Some(&Tok::Punct('(')) {
            cur.group_range('(', ')');
        }
        if cur.peek() == Some(&Tok::Punct('{')) {
            cur.group_range('{', '}');
        }
        return None;
    }
    let mut args: HashMap<String, Val> = HashMap::new();
    if cur.peek_at(1) == Some(&Tok::Punct('(')) {
        cur.pos += 1; // land ON '(' — group_range expects the cursor there
        let (start, end) = cur.group_range('(', ')')?;
        parse_named_args(&cur.toks[start..end], &mut args, file, scope);
    } else if cur.peek_at(1) == Some(&Tok::Punct('{')) {
        // Paren-less trailing-lambda call: `Column { … }`.
        cur.pos += 1;
    } else {
        cur.bump();
        return None;
    }
    // Trailing lambda = children (paren-less calls included).
    let label_scope;
    let child_scope = if LABEL_CONTAINERS.contains(&name.as_str()) {
        label_scope = scope.with_inherited_text();
        &label_scope
    } else {
        scope
    };
    let mut children: Vec<Node> = Vec::new();
    if cur.peek() == Some(&Tok::Punct('{')) {
        if let Some((s, e)) = cur.group_range('{', '}') {
            children = parse_nodes(&cur.toks[s..e], file, child_scope);
        }
    }
    // content = { … } lambdas also supply children.
    if let Some(Val::Lambda(nodes, _)) = args.get("content") {
        children = nodes.clone();
    }
    Some(build_nodes(&name, &args, children, file, scope))
}

/// Composables that only wrap content: their children splice into the parent,
/// preserving order (`MaterialTheme { … }` paints no box of its own).
const TRANSPARENT: [&str; 9] = [
    "MaterialTheme",
    "CompositionLocalProvider",
    "ProvideTextStyle",
    "BoxWithConstraints",
    "key",
    "AnimatedVisibility",
    "Crossfade",
    "SwipeRefresh",
    "PullToRefreshBox",
];

/// Side effects and state holders: never part of the rendered tree. Every
/// `remember*` (rememberSaveable, rememberScrollState, …) is ignored too.
const IGNORED: [&str; 9] = [
    "LaunchedEffect",
    "SideEffect",
    "DisposableEffect",
    "LifecycleEventEffect",
    "mutableStateOf",
    "mutableStateListOf",
    "derivedStateOf",
    "collectAsState",
    "collectAsStateWithLifecycle",
];

/// Hard cap for `items(n)` expansion so a huge list cannot explode the tree.
const MAX_ITEM_REPEAT: usize = 12;

/// Builds the nodes for one call: a known composable of this file is inlined,
/// wrappers splice or vanish, anything else becomes a single node (or a
/// placeholder).
fn build_nodes(name: &str, args: &HashMap<String, Val>, children: Vec<Node>, file: &File, scope: &Scope) -> Vec<Node> {
    let bound: HashMap<String, Val> = args.iter().map(|(k, v)| (k.clone(), substitute(v, &scope.params))).collect();
    if let Some(def) = file.funs.get(name) {
        if let (Some((start, end)), true) = (def.body, scope.depth < MAX_INLINE_DEPTH) {
            let mut params = HashMap::new();
            for (i, (param, default)) in def.params.iter().enumerate() {
                // An argument the caller left out falls back to the default.
                if let Some(v) = bound.get(param).or_else(|| bound.get(&format!("@{i}"))).or(default.as_ref()) {
                    params.insert(param.clone(), v.clone());
                }
            }
            let inner = Scope { params, depth: scope.depth + 1, inherited_text: scope.inherited_text };
            return parse_nodes(&file.toks[start..end], file, &inner);
        }
    }
    if IGNORED.contains(&name) || name.starts_with("remember") {
        return Vec::new();
    }
    match name {
        // Lazy-list item slots: `item` contributes its body once.
        "item" | "itemIndexed" | "stickyHeader" => return children,
        // `items(5)` repeats the body (capped); an unresolvable collection
        // shows three samples, like the tooling's placeholder rows.
        "items" | "itemsIndexed" => {
            let count = match bound.get("@0") {
                Some(Val::Num(n)) | Some(Val::Dp(n)) => (*n as usize).clamp(1, MAX_ITEM_REPEAT),
                _ => 3,
            };
            return children.iter().cycle().take(children.len() * count).cloned().collect();
        }
        _ => {}
    }
    if TRANSPARENT.contains(&name) {
        return children;
    }
    vec![build_composable(name, &bound, children, scope.inherited_text)]
}

fn get_f32(args: &HashMap<String, Val>, key: &str) -> Option<f32> {
    match args.get(key) {
        Some(Val::Num(n)) | Some(Val::Dp(n)) => Some(*n),
        _ => None,
    }
}

fn val_f32(v: &Val) -> Option<f32> {
    match v {
        Val::Num(n) | Val::Dp(n) => Some(*n),
        _ => None,
    }
}

fn get_color(args: &HashMap<String, Val>, key: &str) -> Option<String> {
    match args.get(key) {
        Some(Val::Color(c)) => Some(c.clone()),
        _ => None,
    }
}

fn apply_modifier(style: &mut Style, ops: &[ModOp]) {
    for op in ops {
        match op.name.as_str() {
            "padding" => apply_padding(style, op),
            "fillMaxWidth" => {
                style.width = Some(match op.args.first() {
                    Some(Val::Num(f)) => Size::Percent(f * 100.0),
                    _ => Size::Match,
                })
            }
            "fillMaxHeight" => {
                style.height = Some(match op.args.first() {
                    Some(Val::Num(f)) => Size::Percent(f * 100.0),
                    _ => Size::Match,
                })
            }
            "fillMaxSize" => {
                let v = match op.args.first() {
                    Some(Val::Num(f)) => Size::Percent(f * 100.0),
                    _ => Size::Match,
                };
                style.width = Some(v);
                style.height = Some(v);
            }
            "width" | "requiredWidth" => {
                if let Some(Val::Dp(n)) = op.args.first() {
                    style.width = Some(Size::Px(*n));
                }
            }
            "height" | "requiredHeight" => {
                if let Some(Val::Dp(n)) = op.args.first() {
                    style.height = Some(Size::Px(*n));
                }
            }
            "size" | "requiredSize" => {
                if let Some(Val::Dp(n)) = op.args.first() {
                    style.width = Some(Size::Px(*n));
                    style.height = Some(Size::Px(*n));
                }
            }
            "background" => {
                if let Some(Val::Color(c)) = op.args.first() {
                    style.bg_color = Some(c.clone());
                }
                for a in op.args.iter().skip(1) {
                    if let Some(r) = shape_radius(a) {
                        style.radius = Some(r);
                    }
                }
            }
            "weight" => {
                if let Some(Val::Num(f)) = op.args.first() {
                    style.weight = Some(*f);
                }
            }
            "sizeIn" | "requiredSizeIn" | "defaultMinSize" | "widthIn" | "heightIn" => {
                apply_size_in(&op.name, style, op);
            }
            "rotate" => {
                if let Some(n) = op.args.first().and_then(val_f32) {
                    style.rotation = Some(n);
                }
            }
            "graphicsLayer" => {
                // Best effort: the paren form arrives as (Ident, value) marker
                // pairs, the trailing-lambda form as one assignment block.
                let (mut rotation, mut alpha) = (None, None);
                let mut i = 0;
                while i + 1 < op.args.len() {
                    if let Val::Ident(key) = &op.args[i] {
                        match key.as_str() {
                            "rotationZ" => rotation = rotation.or_else(|| val_f32(&op.args[i + 1])),
                            "alpha" => alpha = alpha.or_else(|| val_f32(&op.args[i + 1])),
                            _ => {}
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if let Some(Val::Call { args, .. }) =
                    op.args.iter().find(|a| matches!(a, Val::Call { name, .. } if name == ASSIGN_BLOCK))
                {
                    rotation = rotation.or_else(|| get_f32(args, "rotationZ"));
                    alpha = alpha.or_else(|| get_f32(args, "alpha"));
                }
                if let Some(r) = rotation {
                    style.rotation = Some(r);
                }
                if let Some(a) = alpha {
                    style.alpha = Some(a);
                }
            }
            "align" => {
                if let Some(Val::Ident(path)) = op.args.first() {
                    style.layout_gravity = Some(compose_alignment(path));
                }
            }
            "clip" => {
                for a in &op.args {
                    if let Some(r) = shape_radius(a) {
                        style.radius = Some(r);
                    }
                }
            }
            "border" => {
                let width = match op.args.first() {
                    Some(Val::Dp(n)) => *n,
                    _ => 1.0,
                };
                let color = op
                    .args
                    .iter()
                    .find_map(|a| match a {
                        Val::Color(c) => Some(c.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "rgba(0,0,0,0.3)".into());
                style.border = Some((width, color));
                for a in op.args.iter().skip(2) {
                    if let Some(r) = shape_radius(a) {
                        style.radius = Some(r);
                    }
                }
            }
            "shadow" => {
                style.elevation = op.args.first().and_then(|a| match a {
                    Val::Dp(n) | Val::Num(n) => Some(*n),
                    _ => None,
                });
            }
            "alpha" => {
                style.alpha = op.args.first().and_then(|a| match a {
                    Val::Num(n) => Some(*n),
                    _ => None,
                });
            }
            "aspectRatio" => {
                style.aspect = op.args.first().and_then(|a| match a {
                    Val::Num(n) => Some(*n),
                    _ => None,
                });
            }
            "offset" => {
                let get = |a: Option<&Val>| match a {
                    Some(Val::Dp(n)) | Some(Val::Num(n)) => Some(*n),
                    _ => None,
                };
                style.offset = Some((get(op.args.first()).unwrap_or(0.0), get(op.args.get(1)).unwrap_or(0.0)));
            }
            // clickable, testTag, semantics, navigationBarsPadding, … ignored
            _ => {}
        }
    }
}

/// `.padding(16.dp)`, the positional overloads `padding(8.dp, 4.dp)`
/// (horizontal, vertical) and `padding(start, top, end, bottom)`, and the
/// named variants `padding(start = 8.dp, horizontal = 8.dp)`. Named argument
/// names arrive as Ident markers before their value, so pairs are walked in
/// order and whatever is left over is positional.
fn apply_padding(style: &mut Style, op: &ModOp) {
    let names = ["start", "end", "top", "bottom", "horizontal", "vertical", "left", "right"];
    let mut positional: Vec<f32> = Vec::new();
    let mut i = 0;
    while i < op.args.len() {
        if let Some(Val::Ident(name) | Val::Unknown(name)) = op.args.get(i) {
            if names.contains(&name.as_str()) {
                if let Some(v) = op.args.get(i + 1).and_then(val_f32) {
                    match name.as_str() {
                        "start" | "left" => style.padding.start += v,
                        "end" | "right" => style.padding.end += v,
                        "top" => style.padding.top += v,
                        "bottom" => style.padding.bottom += v,
                        "horizontal" => {
                            style.padding.start += v;
                            style.padding.end += v;
                        }
                        "vertical" => {
                            style.padding.top += v;
                            style.padding.bottom += v;
                        }
                        _ => {}
                    }
                    i += 2;
                    continue;
                }
            }
        } else if let Some(v) = op.args.get(i).and_then(val_f32) {
            positional.push(v);
        }
        i += 1;
    }
    match positional.as_slice() {
        [all] => {
            style.padding.start += *all;
            style.padding.end += *all;
            style.padding.top += *all;
            style.padding.bottom += *all;
        }
        [h, v] => {
            style.padding.start += *h;
            style.padding.end += *h;
            style.padding.top += *v;
            style.padding.bottom += *v;
        }
        [start, top, end, bottom] => {
            style.padding.start += *start;
            style.padding.top += *top;
            style.padding.end += *end;
            style.padding.bottom += *bottom;
        }
        _ => {} // 3 positional values: no Compose overload, ignore
    }
}

/// `sizeIn`/`widthIn`/`heightIn`/`requiredSizeIn`/`defaultMinSize`: a min/max
/// constraint becomes a fixed size only while the axis has no explicit size
/// yet — the closest static answer to "at least / at most this big".
fn apply_size_in(op_name: &str, style: &mut Style, op: &ModOp) {
    let mut i = 0;
    while i + 1 < op.args.len() {
        let key = match &op.args[i] {
            Val::Ident(k) | Val::Unknown(k) => k.as_str(),
            _ => {
                i += 1;
                continue;
            }
        };
        let Some(v) = val_f32(&op.args[i + 1]) else {
            i += 1;
            continue;
        };
        let horizontal = match key {
            "minWidth" | "maxWidth" => Some(true),
            "minHeight" | "maxHeight" => Some(false),
            // widthIn/heightIn name their bounds just `min`/`max`.
            "min" | "max" => Some(op_name.starts_with("width")),
            _ => None,
        };
        match horizontal {
            Some(true) if style.width.is_none() => style.width = Some(Size::Px(v)),
            Some(false) if style.height.is_none() => style.height = Some(Size::Px(v)),
            _ => {}
        }
        i += 2;
    }
}

/// The non-default components of an `Alignment.…` path, so callers can merge
/// only the axis they care about.
fn alignment_parts(path: &str) -> (Option<HAlign>, Option<VAlign>) {
    let last = path.rsplit('.').next().unwrap_or(path);
    match last {
        "Center" => (Some(HAlign::Center), Some(VAlign::Center)),
        "CenterStart" | "Start" => (Some(HAlign::Start), Some(VAlign::Center)),
        "CenterEnd" | "End" => (Some(HAlign::End), Some(VAlign::Center)),
        "CenterHorizontally" => (Some(HAlign::Center), None),
        "CenterVertically" => (None, Some(VAlign::Center)),
        "TopStart" => (Some(HAlign::Start), Some(VAlign::Top)),
        "TopCenter" => (Some(HAlign::Center), Some(VAlign::Top)),
        "TopEnd" => (Some(HAlign::End), Some(VAlign::Top)),
        "BottomStart" => (Some(HAlign::Start), Some(VAlign::Bottom)),
        "BottomCenter" => (Some(HAlign::Center), Some(VAlign::Bottom)),
        "BottomEnd" => (Some(HAlign::End), Some(VAlign::Bottom)),
        _ => (None, None),
    }
}

fn compose_alignment(path: &str) -> Gravity {
    let (h, v) = alignment_parts(path);
    Gravity { h: h.unwrap_or(HAlign::Start), v: v.unwrap_or(VAlign::Top) }
}

/// Nodes of a `name = { … }` lambda argument.
fn lambda_nodes(args: &HashMap<String, Val>, key: &str) -> Vec<Node> {
    match args.get(key) {
        Some(Val::Lambda(nodes, _)) => nodes.clone(),
        _ => Vec::new(),
    }
}

/// The first literal text a lambda renders (`title = { Text("Hi") }`).
fn lambda_text(args: &HashMap<String, Val>, key: &str) -> Option<String> {
    first_text(&lambda_nodes(args, key))
}

fn first_text(nodes: &[Node]) -> Option<String> {
    for n in nodes {
        if n.kind == "TextView" && n.text.is_some() {
            return n.text.clone();
        }
        if let Some(t) = first_text(&n.children) {
            return Some(t);
        }
    }
    None
}

/// `BorderStroke(1.dp, Color.Red)` → `(width, css color)`.
fn border_stroke(v: &Val) -> Option<(f32, String)> {
    match v {
        Val::Call { name, args } if name.ends_with("BorderStroke") => {
            let w = args.get("@0").and_then(val_f32).unwrap_or(1.0);
            let color = args
                .get("color")
                .or_else(|| args.get("@1"))
                .and_then(|c| match c {
                    Val::Color(c) => Some(c.clone()),
                    _ => None,
                })
                .or_else(|| args.values().find_map(|v| match v {
                    Val::Color(c) => Some(c.clone()),
                    _ => None,
                }))?;
            Some((w, color))
        }
        _ => None,
    }
}

/// `PaddingValues(…)` → edges; supports the all / (horizontal, vertical) /
/// (start, top, end, bottom) positional overloads and the named form.
fn padding_values(v: &Val) -> Option<Edges> {
    let Val::Call { name, args } = v else { return None };
    if !name.ends_with("PaddingValues") {
        return None;
    }
    let named = |key: &str| args.get(key).and_then(val_f32);
    let mut e = Edges::default();
    if let Some(v) = named("all") {
        e.start += v;
        e.end += v;
        e.top += v;
        e.bottom += v;
    }
    for (key, horizontal) in [("horizontal", true), ("vertical", false)] {
        if let Some(v) = named(key) {
            if horizontal {
                e.start += v;
                e.end += v;
            } else {
                e.top += v;
                e.bottom += v;
            }
        }
    }
    if let Some(v) = named("start").or_else(|| named("left")) {
        e.start += v;
    }
    if let Some(v) = named("end").or_else(|| named("right")) {
        e.end += v;
    }
    if let Some(v) = named("top") {
        e.top += v;
    }
    if let Some(v) = named("bottom") {
        e.bottom += v;
    }
    match (named("@0"), named("@1"), named("@2"), named("@3")) {
        (Some(a), None, None, None) => {
            e.start += a;
            e.end += a;
            e.top += a;
            e.bottom += a;
        }
        (Some(h), Some(v), None, None) => {
            e.start += h;
            e.end += h;
            e.top += v;
            e.bottom += v;
        }
        (Some(s), Some(t), Some(en), Some(b)) => {
            e.start += s;
            e.top += t;
            e.end += en;
            e.bottom += b;
        }
        _ => {}
    }
    Some(e)
}

/// A colour inside a `…Defaults.…Colors(containerColor = …)` style call.
fn call_color(v: &Val, key: &str) -> Option<String> {
    match v {
        Val::Call { args, .. } => match args.get(key) {
            Some(Val::Color(c)) => Some(c.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A number inside a `ButtonDefaults.elevation(defaultElevation = …)` call.
fn call_f32(v: &Val, key: &str) -> Option<f32> {
    match v {
        Val::Call { args, .. } => args.get(key).or_else(|| args.get("@0")).and_then(val_f32),
        _ => val_f32(v),
    }
}

/// Builds the render node for one composable call.
fn build_composable(name: &str, args: &HashMap<String, Val>, mut children: Vec<Node>, inherited_text: bool) -> Node {
    let mut node = Node::default();
    let mut style = Style::default();
    match name {
        "Column" | "Row" => {
            node.kind = "LinearLayout".into();
            let column = name == "Column";
            style.flex_dir = Some(if column { FlexDir::Column } else { FlexDir::Row });
            // Main axis = arrangement, cross axis = alignment; both may be
            // present and must merge instead of overwriting each other.
            let mut gravity = Gravity::default();
            match args.get(if column { "verticalArrangement" } else { "horizontalArrangement" }) {
                Some(Val::Ident(path)) | Some(Val::Unknown(path)) => {
                    let last = path.rsplit('.').next().unwrap_or(path);
                    match last {
                        "End" => {
                            if column {
                                gravity.v = VAlign::Bottom;
                            } else {
                                gravity.h = HAlign::End;
                            }
                        }
                        "SpaceBetween" | "SpaceEvenly" | "SpaceAround" => {
                            // `justify-content:space-between` — the closest CSS
                            // distribution; centring would move the content.
                            if column {
                                gravity.v = VAlign::Fill;
                            } else {
                                gravity.h = HAlign::Fill;
                            }
                        }
                        "Center" => {
                            if column {
                                gravity.v = VAlign::Center;
                            } else {
                                gravity.h = HAlign::Center;
                            }
                        }
                        _ => {}
                    }
                }
                Some(Val::Gap(n, align)) => {
                    style.gap = Some(*n);
                    // spacedBy's second argument aligns on the cross axis.
                    if let Some(path) = align {
                        let (h, v) = alignment_parts(path);
                        if column {
                            if let Some(h) = h {
                                gravity.h = h;
                            }
                        } else if let Some(v) = v {
                            gravity.v = v;
                        }
                    }
                }
                _ => {}
            }
            if let Some(Val::Ident(path)) = args.get(if column { "horizontalAlignment" } else { "verticalAlignment" }) {
                let g = compose_alignment(path);
                if column {
                    gravity.h = g.h;
                } else {
                    gravity.v = g.v;
                }
            }
            style.gravity = Some(gravity);
        }
        // Flow layouts approximate to a plain Row/Column.
        "FlowRow" | "FlowColumn" => {
            node.kind = "LinearLayout".into();
            style.flex_dir = Some(if name == "FlowRow" { FlexDir::Row } else { FlexDir::Column });
        }
        "Box" => {
            node.kind = "FrameLayout".into();
            if let Some(Val::Ident(path)) = args.get("contentAlignment") {
                style.gravity = Some(compose_alignment(path));
            }
        }
        "Surface" => {
            node.kind = "Surface".into();
            style.bg_color = get_color(args, "color").or_else(|| Some(M3_SURFACE.to_owned()));
            if let Some(v) = args.get("shape") {
                if let Some(r) = shape_radius(v) {
                    style.radius = Some(r);
                }
            }
            style.elevation = get_f32(args, "shadowElevation").or_else(|| get_f32(args, "tonalElevation"));
            if let Some(b) = args.get("border").and_then(border_stroke) {
                style.border = Some(b);
            }
        }
        "Card" | "ElevatedCard" | "OutlinedCard" => {
            node.kind = "CardView".into();
            style.radius = Some(12.0);
            style.bg_color = get_color(args, "containerColor")
                .or_else(|| args.get("colors").and_then(|v| call_color(v, "containerColor")))
                .or_else(|| Some(M3_SURFACE.to_owned()));
            style.elevation = get_f32(args, "shadowElevation")
                .or_else(|| get_f32(args, "tonalElevation"))
                .or_else(|| get_f32(args, "elevation"))
                .or_else(|| args.get("elevation").and_then(|v| call_f32(v, "defaultElevation")));
            if let Some(v) = args.get("shape") {
                if let Some(r) = shape_radius(v) {
                    style.radius = Some(r);
                }
            }
            style.border = args.get("border").and_then(border_stroke);
            if name == "OutlinedCard" && style.border.is_none() {
                // An outlined card shows its hairline even without an argument.
                style.border = Some((1.0, M3_OUTLINE_VARIANT.to_owned()));
            }
        }
        "Text" => {
            node.kind = "TextView".into();
            match args.get("@0").or_else(|| args.get("text")) {
                Some(Val::Str(s)) => node.text = Some(s.clone()),
                Some(Val::Ident(p)) | Some(Val::Unknown(p)) => node.sub = Some(p.clone()),
                Some(Val::Call { name, .. }) => node.sub = Some(format!("{name}(…)")),
                _ => {}
            }
            style.font_size = get_f32(args, "fontSize");
            if let Some(Val::Ident(path)) = args.get("style") {
                if let Some((size, medium)) = typography(path) {
                    style.font_size = style.font_size.or(Some(size));
                    if medium {
                        style.bold = Some(true);
                    }
                }
            }
            // Compose's default is `bodyLarge`; inside a button the container's
            // label typography applies instead (left unset so it inherits).
            if style.font_size.is_none() && !inherited_text {
                style.font_size = Some(16.0);
            }
            style.fg = get_color(args, "color");
            if let Some(Val::Ident(path)) = args.get("fontWeight") {
                let last = path.rsplit('.').next().unwrap_or(path);
                style.bold = Some(matches!(
                    last,
                    "Bold" | "Black" | "ExtraBold" | "ExtraBoldW900" | "SemiBold" | "W600" | "W700" | "W800" | "W900"
                ));
            }
            if let Some(Val::Ident(path)) = args.get("fontStyle") {
                style.italic = Some(path.ends_with("Italic"));
            }
            if let Some(Val::Ident(path)) = args.get("fontFamily") {
                style.font_family = match path.rsplit('.').next().unwrap_or(path) {
                    "Monospace" => Some("monospace"),
                    "Serif" | "Cursive" => Some("serif"),
                    "SansSerif" => Some("sans-serif"),
                    _ => None,
                };
            }
            style.max_lines = match args.get("maxLines") {
                Some(Val::Num(n)) => Some(*n as u32),
                _ => None,
            };
            if let Some(Val::Ident(path)) = args.get("overflow") {
                if path.ends_with("Ellipsis") {
                    style.ellipsize = true;
                }
            }
            // No wrapping (or a single line) clips with an ellipsis in CSS.
            if matches!(args.get("softWrap"), Some(Val::Bool(false))) || style.max_lines == Some(1) {
                style.ellipsize = true;
            }
            style.line_height = get_f32(args, "lineHeight");
            if let Some(Val::Ident(path)) = args.get("textAlign") {
                style.gravity = Some(Gravity {
                    h: match path.rsplit('.').next().unwrap_or(path) {
                        "Center" => HAlign::Center,
                        "End" | "Right" => HAlign::End,
                        _ => HAlign::Start,
                    },
                    v: VAlign::Top,
                });
            }
            style.letter_spacing = get_f32(args, "letterSpacing");
        }
        "Button" | "OutlinedButton" | "ElevatedButton" | "TextButton" | "IconButton" | "FilledTonalButton" => {
            node.kind = name.into();
            if let Some(Val::Str(s)) = args.get("@0").or_else(|| args.get("text")) {
                node.text = Some(s.clone());
            }
            style.bg_color = get_color(args, "containerColor");
            style.fg = get_color(args, "contentColor");
            if let Some(colors) = args.get("colors") {
                style.bg_color = style.bg_color.or_else(|| call_color(colors, "containerColor"));
                style.fg = style.fg.or_else(|| call_color(colors, "contentColor"));
            }
            if let Some(p) = args.get("contentPadding").and_then(padding_values) {
                style.padding = p;
            }
            if let Some(v) = args.get("shape") {
                if let Some(r) = shape_radius(v) {
                    style.radius = Some(r);
                }
            }
            if matches!(args.get("enabled"), Some(Val::Bool(false))) {
                // Compose dims disabled buttons to 38% content alpha.
                style.alpha = Some(0.38);
            }
            if let Some(b) = args.get("border").and_then(border_stroke) {
                style.border = Some(b);
            }
            if let Some(v) = args.get("elevation") {
                style.elevation = call_f32(v, "defaultElevation");
            }
        }
        "FloatingActionButton" | "SmallFloatingActionButton" | "LargeFloatingActionButton"
        | "ExtendedFloatingActionButton" => {
            node.kind = "FloatingActionButton".into();
            match name {
                "SmallFloatingActionButton" => {
                    style.width = Some(Size::Px(40.0));
                    style.height = Some(Size::Px(40.0));
                    style.radius = Some(12.0);
                }
                "LargeFloatingActionButton" => {
                    style.width = Some(Size::Px(96.0));
                    style.height = Some(Size::Px(96.0));
                    style.radius = Some(28.0);
                }
                _ => {}
            }
            style.bg_color = get_color(args, "containerColor");
            style.fg = get_color(args, "contentColor");
            if let Some(v) = args.get("shape") {
                if let Some(r) = shape_radius(v) {
                    style.radius = Some(r);
                }
            }
            // An extended FAB carries its content in `icon`/`text` lambdas.
            let mut kids = lambda_nodes(args, "icon");
            kids.extend(std::mem::take(&mut children));
            kids.extend(lambda_nodes(args, "text"));
            children = kids;
        }
        "Chip" | "AssistChip" | "FilterChip" | "InputChip" | "SuggestionChip" => {
            node.kind = "Chip".into();
            node.text = lambda_text(args, "label");
            style.checked = match args.get("checked").or_else(|| args.get("selected")) {
                Some(Val::Bool(b)) => Some(*b),
                _ => None,
            };
            style.bg_color = get_color(args, "containerColor");
            if let Some(colors) = args.get("colors") {
                style.bg_color = style.bg_color.or_else(|| call_color(colors, "containerColor"));
                style.fg = call_color(colors, "labelColor");
            }
            if let Some(b) = args.get("border").and_then(border_stroke) {
                style.border = Some(b);
            }
            let mut kids = lambda_nodes(args, "leadingIcon");
            kids.extend(std::mem::take(&mut children));
            children = kids;
        }
        "TextField" | "OutlinedTextField" => {
            node.kind = "EditText".into();
            node.text = match args.get("value") {
                Some(Val::Str(s)) => Some(s.clone()),
                _ => None,
            };
            node.hint = lambda_text(args, "placeholder").or_else(|| lambda_text(args, "label"));
            style.radius = Some(4.0);
            if name == "TextField" {
                style.bg_color = get_color(args, "containerColor").or_else(|| Some(M3_SURFACE_VARIANT.to_owned()));
            } else {
                style.border = get_color(args, "borderColor")
                    .map(|c| (1.0, c))
                    .or_else(|| Some((1.0, M3_OUTLINE.to_owned())));
            }
            // M3 text fields are 56dp tall unless the modifier says otherwise.
            if style.height.is_none() {
                style.height = Some(Size::Px(56.0));
            }
        }
        "Scaffold" => {
            node.kind = "FrameLayout".into();
            style.bg_color = get_color(args, "containerColor").or_else(|| Some(M3_SURFACE.to_owned()));
            let top = lambda_nodes(args, "topBar");
            let bottom = lambda_nodes(args, "bottomBar");
            let fab = lambda_nodes(args, "floatingActionButton");
            let has_top = !top.is_empty();
            let has_bottom = !bottom.is_empty();
            let mut kids = Vec::with_capacity(top.len() + children.len() + bottom.len() + fab.len());
            for mut n in top {
                n.style.width = Some(Size::Match);
                kids.push(n);
            }
            // Content is inset by the bars, like ScaffoldPaddingValues does.
            for mut n in std::mem::take(&mut children) {
                n.style.width = Some(Size::Match);
                if has_top {
                    n.style.margin.top = 64.0;
                }
                if has_bottom {
                    n.style.margin.bottom = 80.0;
                }
                kids.push(n);
            }
            for mut n in bottom {
                n.style.width = Some(Size::Match);
                n.style.layout_gravity = Some(Gravity { h: HAlign::Start, v: VAlign::Bottom });
                kids.push(n);
            }
            for mut n in fab {
                n.style.layout_gravity = Some(Gravity { h: HAlign::End, v: VAlign::Bottom });
                n.style.margin.end += 16.0;
                n.style.margin.bottom += 16.0;
                kids.push(n);
            }
            children = kids;
        }
        "TopAppBar" | "CenterAlignedTopAppBar" | "MediumTopAppBar" | "LargeTopAppBar" => {
            node.kind = "Toolbar".into();
            node.text = lambda_text(args, "title");
            node.hint = lambda_text(args, "subtitle");
            style.width = Some(Size::Match);
            style.height = Some(Size::Px(64.0));
            style.bg_color =
                args.get("colors").and_then(|v| call_color(v, "containerColor")).or_else(|| Some(M3_SURFACE.to_owned()));
            style.padding = Edges { start: 4.0, end: 16.0, top: 0.0, bottom: 0.0 };
            style.gravity = Some(Gravity {
                h: if name == "CenterAlignedTopAppBar" { HAlign::Center } else { HAlign::Start },
                v: VAlign::Center,
            });
            let mut kids = lambda_nodes(args, "navigationIcon");
            kids.extend(lambda_nodes(args, "actions"));
            kids.extend(std::mem::take(&mut children));
            children = kids;
        }
        "NavigationBar" => {
            node.kind = "LinearLayout".into();
            style.flex_dir = Some(FlexDir::Row);
            style.width = Some(Size::Match);
            style.height = Some(Size::Px(80.0));
            style.bg_color = get_color(args, "containerColor").or_else(|| Some(M3_SURFACE.to_owned()));
        }
        "NavigationBarItem" => {
            node.kind = "LinearLayout".into();
            style.flex_dir = Some(FlexDir::Column);
            style.weight = Some(1.0);
            style.height = Some(Size::Match);
            style.gravity = Some(Gravity { h: HAlign::Center, v: VAlign::Center });
            let selected = matches!(args.get("selected"), Some(Val::Bool(true)));
            let mut label = lambda_nodes(args, "label");
            for n in &mut label {
                if n.kind == "TextView" {
                    // M3 navigation labels are 12sp regardless of the Text default.
                    n.style.font_size = Some(12.0);
                    if selected && n.style.fg.is_none() {
                        n.style.fg = Some(M3_PRIMARY.to_owned());
                    }
                }
            }
            let mut kids = lambda_nodes(args, "icon");
            kids.extend(std::mem::take(&mut children));
            kids.extend(label);
            children = kids;
        }
        "TabRow" | "ScrollableTabRow" => {
            node.kind = "LinearLayout".into();
            style.flex_dir = Some(FlexDir::Row);
            style.width = Some(Size::Match);
            style.bg_color = get_color(args, "containerColor").or_else(|| Some(M3_SURFACE.to_owned()));
            if name == "TabRow" {
                // Fixed tabs divide the row equally; scrollable ones wrap.
                for c in &mut children {
                    if c.style.weight.is_none() {
                        c.style.weight = Some(1.0);
                    }
                }
            }
        }
        "Tab" => {
            node.kind = "TextView".into();
            let selected = matches!(args.get("selected"), Some(Val::Bool(true)));
            node.text = lambda_text(args, "text");
            if node.text.is_none() && !children.is_empty() {
                if let Some(t) = first_text(&children) {
                    node.text = Some(t);
                    children = Vec::new();
                }
            }
            style.font_size = Some(14.0);
            style.bold = Some(selected);
            style.fg = Some(if selected { M3_PRIMARY } else { M3_ON_SURFACE_VARIANT }.to_owned());
            style.height = Some(Size::Px(48.0));
            style.gravity = Some(Gravity { h: HAlign::Center, v: VAlign::Center });
        }
        "Badge" => {
            node.kind = "TextView".into();
            node.text = first_text(&children).or_else(|| match args.get("@0") {
                Some(Val::Str(s)) => Some(s.clone()),
                _ => None,
            });
            if node.text.is_some() {
                children = Vec::new();
            }
            style.bg_color = get_color(args, "containerColor").or_else(|| Some(M3_ERROR.to_owned()));
            style.fg = Some("#FFFFFF".to_owned());
            style.radius = Some(999.0);
            style.font_size = Some(12.0);
            style.gravity = Some(Gravity { h: HAlign::Center, v: VAlign::Center });
            style.padding = Edges { start: 4.0, end: 4.0, top: 0.0, bottom: 0.0 };
        }
        "ListItem" => {
            node.kind = "LinearLayout".into();
            style.flex_dir = Some(FlexDir::Row);
            style.width = Some(Size::Match);
            style.gravity = Some(Gravity { h: HAlign::Start, v: VAlign::Center });
            style.padding = Edges { start: 16.0, end: 16.0, top: 8.0, bottom: 8.0 };
            style.bg_color = args.get("colors").and_then(|v| call_color(v, "containerColor"));
            // Headline and supporting text stack in a centre column.
            let mut center = Node { kind: "LinearLayout".into(), ..Default::default() };
            center.style.flex_dir = Some(FlexDir::Column);
            center.style.weight = Some(1.0);
            center.children = lambda_nodes(args, "headlineContent");
            center.children.extend(lambda_nodes(args, "supportingContent"));
            let mut kids = lambda_nodes(args, "leadingContent");
            kids.push(center);
            kids.extend(lambda_nodes(args, "trailingContent"));
            kids.extend(std::mem::take(&mut children));
            children = kids;
        }
        "DropdownMenuItem" => {
            node.kind = "LinearLayout".into();
            style.flex_dir = Some(FlexDir::Row);
            style.width = Some(Size::Match);
            style.height = Some(Size::Px(48.0));
            style.gravity = Some(Gravity { h: HAlign::Start, v: VAlign::Center });
            style.padding = Edges { start: 12.0, end: 12.0, top: 0.0, bottom: 0.0 };
            let mut kids = lambda_nodes(args, "leadingIcon");
            kids.extend(lambda_nodes(args, "text"));
            kids.extend(lambda_nodes(args, "trailingIcon"));
            kids.extend(std::mem::take(&mut children));
            children = kids;
        }
        "AlertDialog" | "BasicAlertDialog" => {
            node.kind = "CardView".into();
            style.width = Some(Size::Px(320.0));
            style.radius = Some(28.0);
            style.bg_color = get_color(args, "containerColor").or_else(|| Some(M3_SURFACE.to_owned()));
            style.padding = Edges { start: 24.0, end: 24.0, top: 24.0, bottom: 24.0 };
            let mut column = Node { kind: "LinearLayout".into(), ..Default::default() };
            column.style.flex_dir = Some(FlexDir::Column);
            column.style.width = Some(Size::Match);
            column.style.gap = Some(16.0);
            for key in ["icon", "title", "text"] {
                column.children.extend(lambda_nodes(args, key));
            }
            let mut buttons = lambda_nodes(args, "dismissButton");
            buttons.extend(lambda_nodes(args, "confirmButton"));
            if !buttons.is_empty() {
                let mut row = Node { kind: "LinearLayout".into(), ..Default::default() };
                row.style.flex_dir = Some(FlexDir::Row);
                row.style.width = Some(Size::Match);
                row.style.gap = Some(8.0);
                row.style.gravity = Some(Gravity { h: HAlign::End, v: VAlign::Center });
                row.children = buttons;
                column.children.push(row);
            }
            column.children.extend(std::mem::take(&mut children));
            children = vec![column];
        }
        "Icon" => {
            node.kind = "Icon".into();
            match args.get("@0").or_else(|| args.get("imageVector")).or_else(|| args.get("painter")) {
                Some(Val::Paint(img)) => style.src_image = Some(img.clone()),
                Some(Val::Ident(path)) => {
                    node.sub = Some(path.rsplit('.').next().unwrap_or(path).to_owned());
                }
                Some(Val::Unknown(t)) => node.sub = Some(t.clone()),
                _ => node.sub = Some("icon".into()),
            }
            style.fg = get_color(args, "tint");
        }
        "Image" => {
            node.kind = "ImageView".into();
            match args.get("painter").or_else(|| args.get("@0")) {
                Some(Val::Paint(img)) => style.src_image = Some(img.clone()),
                Some(Val::Shape(shape)) => style.shape = Some(shape.clone()),
                Some(Val::Ident(path)) => node.sub = Some(path.rsplit('.').next().unwrap_or(path).to_owned()),
                _ => node.sub = Some("image".into()),
            }
        }
        "Spacer" => node.kind = "Space".into(),
        "Divider" | "HorizontalDivider" | "VerticalDivider" => {
            node.kind = "Divider".into();
            let vertical = name == "VerticalDivider";
            style.vertical = vertical;
            let thickness = get_f32(args, "thickness").unwrap_or(1.0);
            style.bg_color = get_color(args, "color").or_else(|| Some(M3_OUTLINE_VARIANT.to_owned()));
            // Material dividers span the container across their short axis.
            if vertical {
                style.width = Some(Size::Px(thickness));
                style.height = Some(Size::Match);
            } else {
                style.width = Some(Size::Match);
                style.height = Some(Size::Px(thickness));
            }
        }
        "Checkbox" | "Switch" | "RadioButton" => {
            node.kind = if name == "Checkbox" { "CheckBox".into() } else { name.into() };
            style.checked = match args.get("checked") {
                Some(Val::Bool(b)) => Some(*b),
                _ => Some(false),
            };
        }
        "Slider" => {
            node.kind = "SeekBar".into();
            let v = get_f32(args, "value").unwrap_or(0.5);
            style.progress = Some(((v * 100.0).round(), 100.0));
        }
        "CircularProgressIndicator" => {
            node.kind = "ProgressBar".into();
            style.spinner = true;
        }
        "LinearProgressIndicator" => {
            node.kind = "ProgressBar".into();
            style.spinner = false;
            let v = match args.get("progress").or_else(|| args.get("value")) {
                Some(Val::Num(n)) | Some(Val::Dp(n)) => Some(*n),
                Some(Val::Lambda(_, scalar)) => *scalar,
                _ => None,
            }
            .unwrap_or(0.25);
            style.progress = Some(((v * 100.0).round(), 100.0));
        }
        "LazyColumn" | "LazyRow" | "LazyVerticalGrid" => {
            node.kind = if name == "LazyRow" { "LazyRow".into() } else { "LazyColumn".into() };
            style.flex_dir = Some(if name == "LazyRow" { FlexDir::Row } else { FlexDir::Column });
            if children.is_empty() {
                // No item calls at all: `items(list)` cannot be expanded
                // without the data, so show the placeholder rows.
                for i in 0..3 {
                    let mut item = Node::placeholder(format!("item {}", i + 1));
                    item.style.width = Some(Size::Match);
                    item.style.height = Some(Size::Px(48.0));
                    node.children.push(item);
                }
                node.style = style;
                return node;
            }
        }
        other => {
            node.kind = "Placeholder".into();
            node.sub = Some(other.to_owned());
        }
    }
    if let Some(Val::Modifier(ops)) = args.get("modifier") {
        apply_modifier(&mut style, ops);
    }
    node.style = style;
    node.children = children;
    node
}

#[cfg(test)]
mod tests;
