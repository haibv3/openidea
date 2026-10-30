//! Kotlin scanner for Jetpack Compose `@Preview` functions.
//!
//! A small lexer (string-, char-, template- and comment-aware, including
//! nested block comments) feeds a pragmatic parser that finds every
//! `@Preview` annotation (stacked annotations → one frame each), then builds a
//! [`Node`] tree from the composable calls in the annotated function's body:
//! Column/Row/Box, Text, buttons, Material colors, a `Modifier` chain subset
//! and placeholders for everything it cannot understand.
//!
//! Pure Rust, no host calls: fully unit-testable natively.

use std::collections::HashMap;

use crate::layout::{FlexDir, Gravity, HAlign, Node, Size, Style, VAlign, Visibility};
use crate::resources::{color_css, theme_attr_css};

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
    /// `Arrangement.spacedBy(N.dp)`.
    Gap(f32),
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
fn parse_named_args(toks: &[TokPos], out: &mut HashMap<String, Val>) {
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
        let val = parse_val(&mut cur);
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
fn parse_val(cur: &mut Cursor) -> Val {
    match cur.peek().cloned() {
        Some(Tok::Punct('-')) => {
            cur.bump();
            match parse_val(cur) {
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
            parse_lambda(&cur.toks[start..end])
        }
        Some(Tok::Punct('(')) => {
            let (start, end) = cur.group_range('(', ')').unwrap_or((cur.pos, cur.pos));
            let mut inner = Cursor { toks: cur.toks[start..end].to_vec(), pos: 0 };
            parse_val(&mut inner)
        }
        Some(Tok::Ident(id)) => parse_ident_val(cur, &id),
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

/// `{ … }`: composable calls when present, else a leading scalar.
fn parse_lambda(toks: &[TokPos]) -> Val {
    let nodes = parse_nodes(toks);
    let scalar = if nodes.is_empty() {
        let mut cur = Cursor { toks: toks.to_vec(), pos: 0 };
        match parse_val(&mut cur) {
            Val::Num(n) => Some(n),
            Val::Dp(n) => Some(n),
            _ => None,
        }
    } else {
        None
    };
    Val::Lambda(nodes, scalar)
}

/// Parses an identifier-headed value: Modifier chains, Color, MaterialTheme,
/// enums, function calls, bare references.
fn parse_ident_val(cur: &mut Cursor, id: &str) -> Val {
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
                        args.push(parse_val(&mut inner));
                        if inner.peek() == Some(&Tok::Punct(',')) {
                            inner.pos += 1;
                        } else {
                            break;
                        }
                    }
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
                parse_named_args(&cur.toks[start..end], &mut args);
                // Arrangement.spacedBy(8.dp)
                if path.last().map(String::as_str) == Some("spacedBy") {
                    let gap = args.values().find_map(|v| match v {
                        Val::Dp(n) | Val::Num(n) => Some(*n),
                        _ => None,
                    });
                    return Val::Gap(gap.unwrap_or(0.0));
                }
                return Val::Call { name: path.join("."), args: Box::new(args) };
            }
            cur.pos = probe;
            Val::Ident(path.join("."))
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
// Preview discovery
// ---------------------------------------------------------------------------

/// One `@Preview` annotation's options plus the function it annotates.
#[derive(Debug, Clone)]
struct PreviewSpec {
    name: Option<String>,
    width_dp: Option<f32>,
    height_dp: Option<f32>,
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
pub fn scan(src: &str) -> Result<Vec<Frame>, String> {
    let toks = lex(src)?;
    let mut cur = Cursor { toks, pos: 0 };
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
            parse_named_args(&cur.toks[start..end], &mut args);
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
        frames.push(build_frame(&cur.toks, spec.fun_index, &caption, spec)?);
    }
    Ok(frames)
}

fn num_arg(args: &HashMap<String, Val>, key: &str) -> Option<f32> {
    match args.get(key) {
        Some(Val::Num(n)) | Some(Val::Dp(n)) => Some(*n),
        _ => None,
    }
}

fn build_frame(toks: &[TokPos], fun_index: usize, caption: &str, spec: &PreviewSpec) -> Result<Frame, String> {
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
    let root = body.map(|(start, end)| {
        let mut n = Node { kind: "Frame".into(), ..Default::default() };
        n.children = parse_nodes(&toks[start..end]);
        n
    });
    Ok(Frame {
        caption: caption.to_owned(),
        root,
        width: spec.width_dp,
        height: spec.height_dp,
        show_background: spec.show_background,
        bg_color: spec.background_color.clone(),
    })
}

// ---------------------------------------------------------------------------
// Body → nodes
// ---------------------------------------------------------------------------

const CONTROL_KEYWORDS: [&str; 9] = ["if", "else", "for", "while", "when", "return", "repeat", "try", "catch"];

/// Parses a sequence of composable calls into nodes.
fn parse_nodes(toks: &[TokPos]) -> Vec<Node> {
    let mut cur = Cursor { toks: toks.to_vec(), pos: 0 };
    let mut out = Vec::new();
    while cur.pos < cur.toks.len() {
        let before = cur.pos;
        if let Some(node) = parse_call(&mut cur) {
            out.push(node);
        }
        if cur.pos == before {
            // Always make progress (unbalanced groups, stray tokens, …).
            cur.bump();
        }
    }
    out
}

/// Parses one `Name(args) { children }` call; returns None for non-calls.
fn parse_call(cur: &mut Cursor) -> Option<Node> {
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
        parse_named_args(&cur.toks[start..end], &mut args);
    } else if cur.peek_at(1) == Some(&Tok::Punct('{')) {
        // Paren-less trailing-lambda call: `Column { … }`.
        cur.pos += 1;
    } else {
        cur.bump();
        return None;
    }
    // Trailing lambda = children (paren-less calls included).
    let mut children: Vec<Node> = Vec::new();
    if cur.peek() == Some(&Tok::Punct('{')) {
        if let Some((s, e)) = cur.group_range('{', '}') {
            children = parse_nodes(&cur.toks[s..e]);
        }
    }
    // content = { … } lambdas also supply children.
    if let Some(Val::Lambda(nodes, _)) = args.get("content") {
        children = nodes.clone();
    }
    Some(build_composable(&name, &args, children))
}

fn get_f32(args: &HashMap<String, Val>, key: &str) -> Option<f32> {
    match args.get(key) {
        Some(Val::Num(n)) | Some(Val::Dp(n)) => Some(*n),
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

/// `.padding(16.dp)` and the named variants `padding(start = 8.dp, top = 4.dp)`,
/// `padding(horizontal = 8.dp)`. Named argument names are not preserved by the
/// generic parser, so positional/named mixes are recovered by walking pairs.
fn apply_padding(style: &mut Style, op: &ModOp) {
    if let Some(Val::Dp(all)) = op.args.first() {
        if op.args.len() == 1 {
            style.padding.start += all;
            style.padding.end += all;
            style.padding.top += all;
            style.padding.bottom += all;
            return;
        }
    }
    let names = ["start", "end", "top", "bottom", "horizontal", "vertical", "left", "right"];
    let mut i = 0;
    while i < op.args.len() {
        let named = matches!(&op.args[i], Val::Ident(n) | Val::Unknown(n) if names.contains(&n.as_str()));
        if named {
            if let Some(Val::Dp(v) | Val::Num(v)) = op.args.get(i + 1) {
                let (Val::Ident(name) | Val::Unknown(name)) = &op.args[i] else { unreachable!() };
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
        i += 1;
    }
}

fn compose_alignment(path: &str) -> Gravity {
    let last = path.rsplit('.').next().unwrap_or(path);
    let (h, v) = match last {
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
    };
    Gravity { h: h.unwrap_or(HAlign::Start), v: v.unwrap_or(VAlign::Top) }
}

/// Builds the render node for one composable call.
fn build_composable(name: &str, args: &HashMap<String, Val>, children: Vec<Node>) -> Node {
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
                        "Center" | "SpaceBetween" | "SpaceEvenly" | "SpaceAround" => {
                            if column {
                                gravity.v = VAlign::Center;
                            } else {
                                gravity.h = HAlign::Center;
                            }
                        }
                        _ => {}
                    }
                }
                Some(Val::Gap(n)) => style.gap = Some(*n),
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
        "Box" => {
            node.kind = "FrameLayout".into();
            if let Some(Val::Ident(path)) = args.get("contentAlignment") {
                style.gravity = Some(compose_alignment(path));
            }
        }
        "Surface" => {
            node.kind = "Surface".into();
            style.bg_color = get_color(args, "color").or_else(|| Some(crate::resources::M3_SURFACE.to_owned()));
            if let Some(v) = args.get("shape") {
                if let Some(r) = shape_radius(v) {
                    style.radius = Some(r);
                }
            }
            style.elevation = get_f32(args, "shadowElevation").or_else(|| get_f32(args, "tonalElevation"));
        }
        "Card" | "ElevatedCard" | "OutlinedCard" => {
            node.kind = "CardView".into();
            style.radius = Some(12.0);
            style.bg_color =
                get_color(args, "containerColor").or_else(|| Some(crate::resources::M3_SURFACE.to_owned()));
            style.elevation = get_f32(args, "elevation");
            if let Some(v) = args.get("shape") {
                if let Some(r) = shape_radius(v) {
                    style.radius = Some(r);
                }
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
            style.max_lines = match args.get("maxLines") {
                Some(Val::Num(n)) => Some(*n as u32),
                _ => None,
            };
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
            if let Some(Val::Num(n)) = args.get("letterSpacing") {
                style.letter_spacing = Some(*n);
            }
        }
        "Button" | "OutlinedButton" | "ElevatedButton" | "TextButton" | "IconButton" | "FilledTonalButton" => {
            node.kind = name.into();
            if let Some(Val::Str(s)) = args.get("@0").or_else(|| args.get("text")) {
                node.text = Some(s.clone());
            }
            if let Some(Val::Color(c)) = args.get("containerColor") {
                style.bg_color = Some(c.clone());
            }
        }
        "Icon" => {
            node.kind = "Icon".into();
            node.sub = match args.get("@0").or_else(|| args.get("imageVector")) {
                Some(Val::Ident(path)) => Some(path.rsplit('.').next().unwrap_or(path).to_owned()),
                Some(Val::Unknown(t)) => Some(t.clone()),
                _ => Some("icon".into()),
            };
            style.fg = get_color(args, "tint");
        }
        "Image" => {
            node.kind = "ImageView".into();
            node.sub = match args.get("@0") {
                Some(Val::Ident(path)) => Some(path.rsplit('.').next().unwrap_or(path).to_owned()),
                _ => Some("image".into()),
            };
        }
        "Spacer" => node.kind = "Space".into(),
        "Divider" | "HorizontalDivider" | "VerticalDivider" => {
            node.kind = "Divider".into();
            style.vertical = name == "VerticalDivider";
            style.fg = get_color(args, "color");
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
            for i in 0..3 {
                let mut item = Node::placeholder(format!("item {}", i + 1));
                item.style.width = Some(Size::Match);
                item.style.height = Some(Size::Px(48.0));
                item.style.visibility = Visibility::Visible;
                node.children.push(item);
            }
            return node;
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
