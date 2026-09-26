//! Turn an HTML AST into a string of HTML.
//!
//! The output style matches [`markdown::to_html`] (void elements end in
//! ` />`, and `&`, `<`, `>`, `"` are encoded), so both paths can be compared.

use crate::hast;
use alloc::string::String;
use markdown::{sanitize_with_protocols, SAFE_PROTOCOL_HREF, SAFE_PROTOCOL_SRC};

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
                let value = match value {
                    hast::PropertyValue::Boolean(false) => continue,
                    hast::PropertyValue::Boolean(true) => String::new(),
                    hast::PropertyValue::String(x) => x.clone(),
                    hast::PropertyValue::CommaSeparated(x) => x.join(", "),
                    hast::PropertyValue::SpaceSeparated(x) => x.join(" "),
                };
                out.push(' ');
                attribute_name(name, out);
                out.push_str("=\"");
                match name.as_str() {
                    // `sanitize_with_protocols` also encodes.
                    "href" if !options.allow_dangerous_protocol => {
                        out.push_str(&sanitize_with_protocols(&value, &SAFE_PROTOCOL_HREF));
                    }
                    "src" if !options.allow_dangerous_protocol => {
                        out.push_str(&sanitize_with_protocols(&value, &SAFE_PROTOCOL_SRC));
                    }
                    _ => encode(&value, out),
                }
                out.push('"');
            }
            if VOIDS.contains(&element.tag_name.as_str()) {
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
    for char in value.chars() {
        match char {
            '\0' => out.push(char::REPLACEMENT_CHARACTER),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(char),
        }
    }
}
