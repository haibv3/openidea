//! Minimal namespace-agnostic XML parser for Android resource and layout files.
//!
//! Supports elements, attributes (single/double quoted), text, comments, XML
//! declarations, processing instructions, DOCTYPE (skipped) and CDATA. Entity
//! references are decoded. Malformed input yields an error carrying the line
//! number of the problem — good enough to point at in an editor.
//!
//! Pure Rust, no host calls: fully unit-testable natively.

/// A parsed element.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Tag name as written, namespace prefix included (`android.widget.Button`-style
    /// class names and `androidx.*` tags are left to callers).
    pub name: String,
    /// Attributes in document order; names keep their prefix (`android:text`).
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Child>,
    /// 1-based line of the `<`.
    pub line: usize,
}

/// Content inside an element.
#[derive(Debug, Clone, PartialEq)]
pub enum Child {
    Element(Element),
    Text(String),
}

/// A parsed document: the single root element, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub root: Option<Element>,
}

/// A parse failure with a 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (line {})", self.message, self.line)
    }
}

impl std::error::Error for ParseError {}

impl Element {
    /// Attribute by exact prefixed name (`android:text`, `layout`, `style`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// Attribute by local name ignoring any prefix: matches `android:text`,
    /// `tools:text` and plain `text` alike. Later duplicates win.
    pub fn local(&self, name: &str) -> Option<&str> {
        self.attrs.iter().rev().find(|(n, _)| n.rsplit(':').next() == Some(name)).map(|(_, v)| v.as_str())
    }

    /// The tag name without its namespace prefix.
    pub fn name_local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    /// The tag name after its last `.` or `:`: `androidx.cardview.widget.CardView`
    /// and `android.widget.Button` become `CardView` and `Button`.
    pub fn tag_local(&self) -> &str {
        let n = self.name.as_str();
        n.rsplit([':', '.']).next().unwrap_or(n)
    }

    /// Child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Child::Element(e) => Some(e),
            Child::Text(_) => None,
        })
    }

    /// All text inside the element, descendants included, entities decoded and
    /// whitespace-trimmed at the ends (used by `res/values` parsers).
    pub fn text_content(&self) -> String {
        fn walk(e: &Element, out: &mut String) {
            for c in &e.children {
                match c {
                    Child::Element(inner) => walk(inner, out),
                    Child::Text(t) => out.push_str(t),
                }
            }
        }
        let mut out = String::new();
        walk(self, &mut out);
        out.trim().to_owned()
    }

    /// Direct text children only, trimmed (values files put their payload there).
    pub fn direct_text(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            if let Child::Text(t) = c {
                out.push_str(t);
            }
        }
        out.trim().to_owned()
    }
}

pub fn parse(src: &str) -> Result<Document, ParseError> {
    Parser::new(src).parse_document()
}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    _src: std::marker::PhantomData<&'a str>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        let mut chars: Vec<char> = src.chars().collect();
        if chars.first() == Some(&'\u{feff}') {
            chars.remove(0); // BOM
        }
        Parser { chars, pos: 0, line: 1, _src: std::marker::PhantomData }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if let Some('\n') = c {
            self.line += 1;
        }
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.starts_with(s) {
            for _ in s.chars() {
                self.bump();
            }
            true
        } else {
            false
        }
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError { message: message.into(), line: self.line })
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }

    /// Skips whitespace, comments, XML declarations, processing instructions
    /// and DOCTYPE. Returns when positioned at `<name…`, `</` or end of input.
    fn skip_misc(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_ws();
            if self.starts_with("<!--") {
                let line = self.line;
                self.eat("<!--");
                loop {
                    if self.pos >= self.chars.len() {
                        return self.err_at(line, "unterminated comment");
                    }
                    if self.starts_with("-->") {
                        self.eat("-->");
                        break;
                    }
                    self.bump();
                }
            } else if self.starts_with("<![CDATA[") {
                // CDATA is content: leave it for the caller when inside an
                // element; at the top level it is malformed.
                return Ok(());
            } else if self.starts_with("<?") {
                self.skip_until("?>", "unterminated processing instruction")?;
            } else if self.starts_with("<!DOCTYPE") || self.starts_with("<!doctype") {
                self.skip_until(">", "unterminated DOCTYPE")?;
            } else {
                return Ok(());
            }
        }
    }

    fn skip_until(&mut self, end: &str, message: &str) -> Result<(), ParseError> {
        loop {
            if self.pos >= self.chars.len() {
                return self.err(message);
            }
            if self.starts_with(end) {
                for _ in end.chars() {
                    self.bump();
                }
                return Ok(());
            }
            self.bump();
        }
    }

    fn err_at<T>(&self, line: usize, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError { message: message.into(), line })
    }

    fn parse_document(&mut self) -> Result<Document, ParseError> {
        self.skip_misc()?;
        if self.peek().is_none() {
            return Ok(Document { root: None });
        }
        if !self.starts_with("<") || self.starts_with("</") {
            return self.err("expected an element");
        }
        let root = self.parse_element()?;
        // Only whitespace/comments may trail the root.
        self.skip_misc()?;
        if self.pos < self.chars.len() {
            return self.err("unexpected content after the root element");
        }
        Ok(Document { root: Some(root) })
    }

    fn parse_element(&mut self) -> Result<Element, ParseError> {
        let line = self.line;
        if !self.eat("<") {
            return self.err("expected '<'");
        }
        let name = self.parse_name("in a tag name")?;
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return self.err_at(line, format!("unclosed <{name}>")),
                Some('/') => {
                    self.bump();
                    if self.peek() != Some('>') {
                        return self.err("expected '>' after '/'");
                    }
                    self.bump();
                    return Ok(Element { name, attrs, children: Vec::new(), line });
                }
                Some('>') => {
                    self.bump();
                    break;
                }
                Some(c) if is_name_start(c) => {
                    let attr = self.parse_name("in an attribute name")?;
                    self.skip_ws();
                    if self.peek() != Some('=') {
                        return self.err(format!("expected '=' after attribute `{attr}`"));
                    }
                    self.bump();
                    self.skip_ws();
                    let value = self.parse_attr_value()?;
                    // Last duplicate wins.
                    if let Some(slot) = attrs.iter_mut().find(|(n, _)| *n == attr) {
                        slot.1 = value;
                    } else {
                        attrs.push((attr, value));
                    }
                }
                Some(_) => return self.err(format!("unexpected character in <{name}>")),
            }
        }
        // Children until the matching end tag.
        let mut children = Vec::new();
        loop {
            if self.pos >= self.chars.len() {
                return self.err_at(line, format!("unclosed <{name}>"));
            }
            if self.starts_with("</") {
                let close_line = self.line;
                self.eat("</");
                let close = self.parse_name("in an end tag")?;
                if close != name {
                    return self
                        .err_at(close_line, format!("</{close}> does not match <{name}> opened at line {line}"));
                }
                self.skip_ws();
                if self.peek() != Some('>') {
                    return self.err("expected '>' at the end of the end tag");
                }
                self.bump();
                return Ok(Element { name, attrs, children, line });
            }
            if self.starts_with("<![CDATA[") {
                self.eat("<![CDATA[");
                let mut text = String::new();
                loop {
                    if self.pos >= self.chars.len() {
                        return self.err("unterminated CDATA section");
                    }
                    if self.starts_with("]]>") {
                        self.eat("]]>");
                        break;
                    }
                    text.push(self.bump().unwrap());
                }
                children.push(Child::Text(text));
                continue;
            }
            if self.starts_with("<!--") || self.starts_with("<?") {
                self.skip_misc()?;
                continue;
            }
            if self.peek() == Some('<') {
                children.push(Child::Element(self.parse_element()?));
                continue;
            }
            // Text run until the next '<'.
            let mut text = String::new();
            while let Some(c) = self.peek() {
                if c == '<' {
                    break;
                }
                text.push(c);
                self.bump();
            }
            children.push(Child::Text(decode_entities(&text)));
        }
    }

    fn parse_name(&mut self, context: &str) -> Result<String, ParseError> {
        let mut name = String::new();
        match self.peek() {
            Some(c) if is_name_start(c) => name.push(c),
            _ => return self.err(format!("expected a name {context}")),
        }
        self.bump();
        while let Some(c) = self.peek() {
            if is_name_char(c) {
                name.push(c);
                self.bump();
            } else {
                break;
            }
        }
        Ok(name)
    }

    fn parse_attr_value(&mut self) -> Result<String, ParseError> {
        let quote = match self.peek() {
            Some(q @ ('"' | '\'')) => q,
            _ => return self.err("expected a quoted attribute value"),
        };
        self.bump();
        let mut value = String::new();
        loop {
            match self.bump() {
                None => return self.err("unterminated attribute value"),
                Some(c) if c == quote => break,
                Some(c) => value.push(c),
            }
        }
        Ok(decode_entities(&value))
    }
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.')
}

/// Decodes `&lt; &gt; &amp; &quot; &apos; &#nn; &#xhh;`. Unknown entities are
/// kept verbatim (lenient, like a browser's recovery mode).
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after.find(';').unwrap_or(after.len());
        let ent = &after[..end];
        let decoded = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                if let Some(num) = ent.strip_prefix('#') {
                    let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    code.and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[end.min(after.len())..];
                rest = rest.strip_prefix(';').unwrap_or(rest);
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(src: &str) -> Element {
        parse(src).unwrap().root.unwrap()
    }

    #[test]
    fn parses_nested_elements_with_attributes() {
        let doc = root(
            r#"<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:orientation="vertical" android:layout_width="match_parent">
    <TextView android:id="@+id/title" android:text='Hi there' />
    <Button android:text="OK" />
</LinearLayout>"#,
        );
        assert_eq!(doc.name, "LinearLayout");
        assert_eq!(doc.attr("android:orientation"), Some("vertical"));
        assert_eq!(doc.children.len(), 5); // text, TextView, text, Button, text
        let tv = doc.elements().next().unwrap();
        assert_eq!(tv.name, "TextView");
        assert_eq!(tv.attr("android:id"), Some("@+id/title"));
        assert_eq!(tv.attr("android:text"), Some("Hi there"));
        assert_eq!(tv.line, 4);
    }

    #[test]
    fn ignores_comments_and_keeps_positions() {
        let doc = root("<a><!-- <fake child> -->\n<b/><!-- multi\nline --></a>");
        assert_eq!(doc.name, "a");
        assert_eq!(doc.elements().count(), 1);
        assert_eq!(doc.elements().next().unwrap().line, 2);
    }

    #[test]
    fn parses_cdata_as_text() {
        let doc = root("<a><![CDATA[if (a < b && c > d) {}]]></a>");
        assert_eq!(doc.direct_text(), "if (a < b && c > d) {}");
    }

    #[test]
    fn decodes_entities() {
        let doc = root(r#"<a t="&lt;x&gt; &amp; &quot;q&quot; &#65;&#x42;">&amp;ok&lt;/a&gt;</a>"#);
        assert_eq!(doc.attr("t"), Some("<x> & \"q\" AB"));
        assert_eq!(doc.direct_text(), "&ok</a>");
    }

    #[test]
    fn keeps_unknown_entities_verbatim() {
        assert_eq!(decode_entities("&nbsp; &wrong; &#zz;"), "&nbsp; &wrong; &#zz;");
    }

    #[test]
    fn namespace_local_helpers() {
        let doc = root(r#"<androidx.cardview.widget.CardView app:cardCornerRadius="8dp" card="x"/>"#);
        assert_eq!(doc.tag_local(), "CardView");
        assert_eq!(doc.local("cardCornerRadius"), Some("8dp"));
        assert_eq!(doc.local("card"), Some("x"));
        assert_eq!(doc.attr("card"), Some("x"));
        assert_eq!(doc.attr("app:cardCornerRadius"), Some("8dp"));
        let plain = root("<Button/>");
        assert_eq!(plain.tag_local(), "Button");
    }

    #[test]
    fn text_content_joins_descendants() {
        let doc = root("<string>Hello <b>World</b>!</string>");
        assert_eq!(doc.text_content(), "Hello World!");
        assert_eq!(doc.direct_text(), "Hello !");
    }

    #[test]
    fn empty_input_has_no_root() {
        assert!(parse("").unwrap().root.is_none());
        assert!(parse("  <!-- nothing -->\n").unwrap().root.is_none());
    }

    #[test]
    fn malformed_reports_line() {
        let e = parse("<a>\n  <b>\n</a>").unwrap_err();
        assert!(e.message.contains("does not match"), "{e}");
        assert_eq!(e.line, 3);

        let e = parse("<a></b>").unwrap_err();
        assert!(e.message.contains("does not match"), "{e}");

        let e = parse("<a attr noquotes/>").unwrap_err();
        assert!(e.message.contains("expected '='"), "{e}");

        let e = parse("<a attr=x />").unwrap_err();
        assert!(e.message.contains("quoted attribute value"), "{e}");

        let e = parse("plain text no root").unwrap_err();
        assert!(e.message.contains("expected an element"), "{e}");

        let e = parse("<a/><b/>").unwrap_err();
        assert!(e.message.contains("after the root"), "{e}");

        let e = parse("<a>text").unwrap_err();
        assert!(e.message.contains("unclosed <a>"), "{e}");

        let e = parse("<!-- oops").unwrap_err();
        assert!(e.message.contains("comment"), "{e}");

        let e = parse("<a>&bad</a>");
        assert!(e.is_ok()); // stray & is lenient, kept verbatim
    }

    #[test]
    fn stray_ampersand_is_lenient() {
        let doc = root("<a>Tom & Jerry</a>");
        assert_eq!(doc.direct_text(), "Tom & Jerry");
    }

    #[test]
    fn doctype_and_processing_instructions_skipped() {
        let doc = root("<?xml version=\"1.0\"?><!DOCTYPE foo SYSTEM \"x.dtd\"><a/>");
        assert_eq!(doc.name, "a");
    }

    #[test]
    fn duplicate_attributes_last_wins() {
        let doc = root(r#"<a x="1" x="2"/>"#);
        assert_eq!(doc.attr("x"), Some("2"));
    }
}
