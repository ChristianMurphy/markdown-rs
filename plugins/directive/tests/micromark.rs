//! Test cases of `micromark-extension-directive`, through the tree path,
//! with its test HTML handlers ported.

use directive::{Directives, CONTAINER, LEAF, TEXT};
use markdown::mdast;
use markdown_processor::{
    hast,
    hast_util_to_html::{hast_util_to_html, Options as HtmlOptions},
    mdast_util_to_hast::wrap,
    Processor,
};

include!("fixtures/micromark_cases.rs");

/// `html-void-elements`.
const VOID: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "command", "embed", "frame", "hr", "image",
    "img", "input", "keygen", "link", "meta", "param", "source", "track", "wbr",
];

fn raw(value: String) -> hast::Node {
    hast::Node::Raw(hast::Raw {
        value,
        position: None,
    })
}

/// Like micromark’s `encode`.
fn encode(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html(nodes: &[hast::Node]) -> String {
    hast_util_to_html(
        &hast::Node::Root(hast::Root {
            children: nodes.to_vec(),
            position: None,
        }),
        &HtmlOptions {
            allow_dangerous_html: true,
            allow_dangerous_protocol: false,
        },
    )
}

/// Label and content of a directive, like `d.label` and `d.content`:
/// `None` when empty.
fn split(
    node: &mdast::Custom,
    mut children: Vec<hast::Node>,
) -> (Option<Vec<hast::Node>>, Option<Vec<hast::Node>>) {
    let non_empty = |nodes: Vec<hast::Node>| if nodes.is_empty() { None } else { Some(nodes) };
    if node.name == CONTAINER {
        let label = if node.fields.contains_key("label") && !children.is_empty() {
            match children.remove(0) {
                hast::Node::Element(paragraph) => paragraph.children,
                _ => vec![],
            }
        } else {
            vec![]
        };
        (non_empty(label), non_empty(children))
    } else {
        (non_empty(children), None)
    }
}

fn attributes(node: &mdast::Custom, skip: &str) -> Vec<String> {
    node.attributes
        .iter()
        .filter(|(key, _)| key.as_str() != skip)
        .map(|(key, value)| format!("{}=\"{}\"", encode(key), encode(value)))
        .collect()
}

/// Content of a container on its own lines.
fn block(content: Vec<hast::Node>) -> Vec<hast::Node> {
    wrap(content, true)
}

fn abbr(node: &mdast::Custom, children: Vec<hast::Node>) -> Option<Vec<hast::Node>> {
    if node.name != TEXT {
        return None;
    }
    let (label, _) = split(node, children);
    let title = node.attributes.get("title").map_or(String::new(), |title| {
        format!(" title=\"{}\"", encode(title))
    });
    let mut out = vec![raw(format!("<abbr{}>", title))];
    out.extend(label.unwrap_or_default());
    out.push(raw("</abbr>".into()));
    Some(out)
}

fn youtube(node: &mdast::Custom, children: Vec<hast::Node>) -> Option<Vec<hast::Node>> {
    let v = node.attributes.get("v").filter(|v| !v.is_empty())?;
    let (label, content) = split(node, children);
    let mut list = vec![
        format!("src=\"https://www.youtube.com/embed/{}\"", encode(v)),
        "allowfullscreen".into(),
    ];
    if let Some(label) = label {
        list.push(format!("title=\"{}\"", encode(&html(&label))));
    }
    list.extend(attributes(node, "v"));
    let mut out = vec![raw(format!("<iframe {}>", list.join(" ")))];
    if let Some(content) = content {
        out.extend(block(content));
    }
    out.push(raw("</iframe>".into()));
    Some(out)
}

fn h(node: &mdast::Custom, children: Vec<hast::Node>) -> Option<Vec<hast::Node>> {
    let name = node.fields.get("name").cloned().unwrap_or_default();
    let (label, content) = split(node, children);
    let list = attributes(node, "");
    let mut out = vec![raw(if list.is_empty() {
        format!("<{}>", name)
    } else {
        format!("<{} {}>", name, list.join(" "))
    })];
    match (content, label) {
        (Some(content), _) if node.name == CONTAINER => out.extend(block(content)),
        (None, Some(label)) if node.name == CONTAINER => out.extend(block(label)),
        (_, Some(label)) => out.extend(label),
        _ => {}
    }
    if !VOID.contains(&name.as_str()) {
        out.push(raw(format!("</{}>", name)));
    }
    Some(out)
}

/// Render `input`, like `micromark(input, options(handlers))`.
fn render(name: &str, input: &str, handlers: &'static [&'static str]) -> String {
    let mut processor = Processor::new().plugin(Directives);
    processor.compile.allow_dangerous_html = true;
    // Like `{disable: {null: ['codeIndented']}}`.
    if name.contains("codeIndented disabled") {
        processor.parse.constructs.code_indented = false;
    }
    for kind in [TEXT, LEAF, CONTAINER] {
        processor.add_hast_handler(kind, move |node, children| {
            let name = node.fields.get("name").map_or("", String::as_str);
            let handled = if handlers.contains(&name) {
                match name {
                    "abbr" => abbr(node, children.clone()),
                    "youtube" => youtube(node, children.clone()),
                    // Captures the label, and renders nothing.
                    "x" => Some(vec![]),
                    _ => None,
                }
            } else {
                None
            };
            handled
                .or_else(|| {
                    if handlers.contains(&"*") {
                        h(node, children)
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        });
    }
    processor
        .process(input)
        .unwrap_or_else(|message| format!("ERROR {}", message))
}

/// HTML without whitespace between tags, and with sorted attributes:
/// micromark’s line endings come from its compiler, and `Custom` keeps
/// attributes sorted.
fn normalize(html: &str) -> String {
    let mut collapsed = String::new();
    let mut pending = String::new();
    let mut after_tag = false;
    for char in html.trim().chars() {
        if after_tag && char.is_whitespace() {
            pending.push(char);
            continue;
        }
        if char != '<' {
            collapsed.push_str(&pending);
        }
        pending.clear();
        collapsed.push(char);
        after_tag = char == '>';
    }

    let mut out = String::new();
    let mut rest = collapsed.as_str();
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let mut quote = None;
        let mut end = rest.len() - 1;
        for (index, char) in rest.char_indices() {
            match (quote, char) {
                (None, '"' | '\'') => quote = Some(char),
                (Some(open), _) if open == char => quote = None,
                (None, '>') => {
                    end = index;
                    break;
                }
                _ => {}
            }
        }
        let tag = &rest[1..end];
        if tag.starts_with('/') || tag.starts_with('!') {
            out.push('<');
            out.push_str(tag);
        } else {
            let mut parts = vec![];
            let mut part = String::new();
            let mut quote = None;
            for char in tag.chars() {
                match (quote, char) {
                    (None, '"' | '\'') => {
                        quote = Some(char);
                        part.push(char);
                    }
                    (Some(open), _) if open == char => {
                        quote = None;
                        part.push(char);
                    }
                    (None, ' ') => parts.push(std::mem::take(&mut part)),
                    _ => part.push(char),
                }
            }
            parts.push(part);
            let name = parts.remove(0);
            parts.retain(|part| !part.is_empty());
            parts.sort();
            out.push('<');
            out.push_str(&name);
            for part in parts {
                out.push(' ');
                out.push_str(&part);
            }
        }
        out.push('>');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn micromark_extension_directive() {
    let mut failures = vec![];
    for (name, input, handlers, expected) in CASES {
        let actual = render(name, input, handlers);
        if normalize(&actual) != normalize(expected) {
            failures.push(format!(
                "{}\n  input:    {:?}\n  expected: {:?}\n  actual:   {:?}",
                name, input, expected, actual
            ));
        }
    }
    println!("{} of {} cases differ", failures.len(), CASES.len());
    for failure in &failures {
        println!("{}", failure);
    }
    assert!(failures.is_empty(), "expected no differences");
}

#[test]
fn keeps_whitespace_around_a_label() {
    let processor = Processor::new().plugin(Directives);
    let tree = markdown::to_mdast("a :x[ b ] c", &processor.parse).unwrap();
    let paragraph = &tree.children().unwrap()[0];

    assert_eq!(
        paragraph.children().unwrap()[1].to_string(),
        " b ",
        "should keep initial and final whitespace, as the `x` handler test checks `d.label`"
    );
}
