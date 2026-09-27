//! Turn an HTML AST into a string of HTML.
//!
//! The output style matches [`markdown::to_html`] (void elements end in
//! ` />`, and `&`, `<`, `>`, `"` are encoded), so both paths can be compared.

use crate::hast;
use alloc::{borrow::Cow, string::String};
use markdown::{SAFE_PROTOCOL_HREF, SAFE_PROTOCOL_SRC};

/// HTML void elements: they have no closing tag.
const VOIDS: [&str; 13] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Configuration; the default is safe for untrusted input.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Whether to pass raw HTML through, instead of encoding it as text.
    pub allow_dangerous_html: bool,
    /// Whether to allow any protocol in `href` and `src`, instead of only the
    /// ones `to_html` allows (`SAFE_PROTOCOL_HREF`, `SAFE_PROTOCOL_SRC`).
    pub allow_dangerous_protocol: bool,
}

/// Serialize hast as HTML; expects URLs normalized but not yet encoded.
pub fn hast_util_to_html(node: &hast::Node, options: &Options) -> String {
    let mut out = String::new();
    one(node, &mut out, options);
    out
}

/// Serialize one node into `out`.
fn one(node: &hast::Node, out: &mut String, options: &Options) {
    match node {
        hast::Node::Root(root) => {
            for child in &root.children {
                one(child, out, options);
            }
        }
        hast::Node::Element(element) => {
            out.push('<');
            out.push_str(&element.tag_name);
            for (name, value) in &element.properties {
                let value: Cow<str> = match value {
                    hast::PropertyValue::Boolean(false) => continue,
                    hast::PropertyValue::Boolean(true) => "".into(),
                    hast::PropertyValue::String(x) => x.as_str().into(),
                    hast::PropertyValue::CommaSeparated(x) => x.join(", ").into(),
                    hast::PropertyValue::SpaceSeparated(x) => x.join(" ").into(),
                };
                out.push(' ');
                attribute_name(name, out);
                out.push_str("=\"");
                match name.as_ref() {
                    "href" if !options.allow_dangerous_protocol => {
                        if has_safe_protocol(&value, &SAFE_PROTOCOL_HREF) {
                            encode(&value, out);
                        }
                    }
                    "src" if !options.allow_dangerous_protocol => {
                        if has_safe_protocol(&value, &SAFE_PROTOCOL_SRC) {
                            encode(&value, out);
                        }
                    }
                    _ => encode(&value, out),
                }
                out.push('"');
            }
            if VOIDS.contains(&element.tag_name.as_ref()) {
                out.push_str(" />");
                return;
            }
            out.push('>');
            for child in &element.children {
                one(child, out, options);
            }
            out.push_str("</");
            out.push_str(&element.tag_name);
            out.push('>');
        }
        hast::Node::Text(text) => encode(&text.value, out),
        hast::Node::Raw(raw) => {
            if options.allow_dangerous_html {
                out.push_str(&raw.value.replace('\0', "\u{FFFD}"));
            } else {
                encode(&raw.value, out);
            }
        }
        hast::Node::Comment(comment) => {
            out.push_str("<!--");
            comment_value(&comment.value, out);
            out.push_str("-->");
        }
        hast::Node::Doctype(_) => out.push_str("<!doctype html>"),
    }
}

/// Write a comment value that cannot end the comment early.
fn comment_value(value: &str, out: &mut String) {
    let value = value.replace("-->", "--&gt;").replace("--!>", "--!&gt;");
    let value = if let Some(rest) = value.strip_prefix("->") {
        ["-&gt;", rest].concat()
    } else if let Some(rest) = value.strip_prefix('>') {
        ["&gt;", rest].concat()
    } else {
        value
    };
    if let Some(start) = value.strip_suffix("<!-") {
        out.push_str(start);
        out.push_str("&lt;!-");
    } else {
        out.push_str(&value);
    }
}

/// Map a hast property name (`className`, `dataFootnoteRef`) to an HTML
/// attribute name (`class`, `data-footnote-ref`): a subset of
/// `property-information`.
fn attribute_name(name: &str, out: &mut String) {
    match name {
        "className" => out.push_str("class"),
        "htmlFor" => out.push_str("for"),
        _ if name.starts_with("aria") => {
            out.push_str("aria-");
            out.push_str(&name[4..].to_ascii_lowercase());
        }
        _ if name.starts_with("data") => {
            out.push_str("data");
            for char in name[4..].chars() {
                if char.is_ascii_uppercase() {
                    out.push('-');
                }
                out.push(char.to_ascii_lowercase());
            }
        }
        _ => out.push_str(name),
    }
}

/// Encode `&`, `<`, `>`, `"`, and NUL, like `markdown::to_html` does.
fn encode(value: &str, out: &mut String) {
    let mut start = 0;
    for (index, byte) in value.bytes().enumerate() {
        let replacement = match byte {
            b'\0' => "\u{FFFD}",
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            _ => continue,
        };
        out.push_str(&value[start..index]);
        out.push_str(replacement);
        start = index + 1;
    }
    out.push_str(&value[start..]);
}

/// Whether a URL has no protocol, or one in `protocols`: the check of
/// `markdown::sanitize_with_protocols`, without normalizing again.
fn has_safe_protocol(value: &str, protocols: &[&str]) -> bool {
    let end = value.find(|char| matches!(char, '?' | '#' | '/'));
    match value.find(':') {
        Some(colon) if end.map_or(true, |end| colon < end) => {
            let protocol = &value[..colon];
            protocols
                .iter()
                .any(|known| known.eq_ignore_ascii_case(protocol))
        }
        _ => true,
    }
}
