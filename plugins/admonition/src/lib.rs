//! Admonitions, as a container plugin, like the admonitions of
//! Python-Markdown.
//!
//! ```markdown
//! !!! note "Heads *up*"
//!     The body is indented by four spaces.
//!
//! ??? tip
//!     `???` makes it collapsible, `???+` open by default.
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::{boxed::Box, collections::BTreeMap, string::String, vec, vec::Vec};
use markdown::{
    extension::{Construct, ConstructTokenizer, ContentType, Step, Token},
    mdast,
};
use markdown_processor::{hast, mdast_util_to_hast::wrap, Plugin, Processor};

/// Name of the custom node this plugin makes.
pub const NAME: &str = "admonition";

/// The plugin: `<div class="admonition note">`, or `<details>` when
/// collapsible.
pub struct Admonitions;

impl Plugin for Admonitions {
    fn attach(self, processor: &mut Processor) {
        processor
            .parse
            .document_constructs
            .push(Box::new(AdmonitionSyntax));
        processor.add_hast_handler(NAME, to_hast);
    }
}

fn to_hast(node: &mdast::Custom, children: Vec<hast::Node>) -> Vec<hast::Node> {
    let classes = node.attributes.get("class").cloned().unwrap_or_default();
    let mut children = children.into_iter();
    // The title is the first child, or the capitalized kind.
    let title = if node.fields.contains_key("title") {
        match children.next() {
            Some(hast::Node::Element(paragraph)) => paragraph.children,
            _ => vec![],
        }
    } else {
        let kind = node.fields.get("kind").map_or("", String::as_str);
        let mut chars = kind.chars();
        let title = chars.next().map_or_else(String::new, |first| {
            let mut title: String = first.to_uppercase().collect();
            title.push_str(chars.as_str());
            title
        });
        vec![hast::Node::Text(hast::Text {
            value: title,
            position: None,
        })]
    };
    let collapsible = node.fields.get("collapsible").map(String::as_str);
    let heading = if collapsible.is_some() {
        element("summary", vec![], title)
    } else {
        element("p", vec!["admonition-title".into()], title)
    };
    let mut content = vec![hast::Node::Element(heading)];
    content.extend(children);
    let content = wrap(content, true);

    let element = if let Some(collapsible) = collapsible {
        let mut properties = vec![(
            "className".into(),
            hast::PropertyValue::SpaceSeparated(classes.split(' ').map(Into::into).collect()),
        )];
        if collapsible == "open" {
            properties.push(("open".into(), hast::PropertyValue::Boolean(true)));
        }
        hast::Element {
            properties,
            ..element("details", vec![], content)
        }
    } else {
        let mut class_names = vec![String::from("admonition")];
        class_names.extend(classes.split(' ').map(Into::into));
        element("div", class_names, content)
    };

    vec![hast::Node::Element(hast::Element {
        position: node.position.clone(),
        ..element
    })]
}

fn element(tag_name: &str, class_names: Vec<String>, children: Vec<hast::Node>) -> hast::Element {
    let properties = if class_names.is_empty() {
        vec![]
    } else {
        vec![(
            "className".into(),
            hast::PropertyValue::SpaceSeparated(class_names),
        )]
    };
    hast::Element {
        tag_name: tag_name.into(),
        properties,
        children,
        position: None,
    }
}

const MARKER: u16 = 1;
const PLUS: u16 = 2;
const KIND_BEFORE: u16 = 3;
const KIND: u16 = 4;
const AFTER_KIND: u16 = 5;
const TITLE: u16 = 6;
const TITLE_TEXT: u16 = 7;
const TITLE_CLOSE: u16 = 8;
const AFTER_TITLE: u16 = 9;
const BODY: u16 = 10;
const CONTINUATION: u16 = 20;
const INDENT: u16 = 21;

/// Columns a body line is indented by.
const INDENT_SIZE: usize = 4;

fn is_kind_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

/// `!!!` or `???` (`???+`), kinds, an optional `"title"`, then a body of
/// lines indented by four spaces, and blank lines.
struct AdmonitionSyntax;

impl Construct for AdmonitionSyntax {
    fn markers(&self) -> &[u8] {
        b"!?"
    }

    fn continuation(&self) -> u16 {
        CONTINUATION
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        // Memory: [0] marker, [1] marker size or indent columns, [2] title size.
        match (state, t.current()) {
            (0, Some(byte @ (b'!' | b'?'))) => {
                t.memory()[0] = usize::from(byte);
                t.memory()[1] = 1;
                t.enter(NAME);
                t.enter("admonitionMarker");
                t.consume();
                Step::Next(MARKER)
            }
            (MARKER, Some(byte)) if usize::from(byte) == t.memory()[0] && t.memory()[1] < 3 => {
                t.memory()[1] += 1;
                t.consume();
                Step::Next(MARKER)
            }
            (MARKER, Some(b'+')) if t.memory()[1] == 3 && t.memory()[0] == usize::from(b'?') => {
                t.consume();
                Step::Next(PLUS)
            }
            (MARKER | PLUS, Some(b'\t' | b' ')) if t.memory()[1] == 3 => {
                t.exit("admonitionMarker");
                t.consume();
                Step::Next(KIND_BEFORE)
            }
            (KIND_BEFORE | AFTER_KIND, Some(b'\t' | b' ')) => {
                t.consume();
                Step::Next(state)
            }
            (KIND_BEFORE | AFTER_KIND, Some(byte)) if is_kind_byte(byte) => {
                t.enter("admonitionKind");
                t.consume();
                Step::Next(KIND)
            }
            (KIND, Some(byte)) if is_kind_byte(byte) => {
                t.consume();
                Step::Next(KIND)
            }
            (KIND, _) => {
                t.exit("admonitionKind");
                Step::Retry(AFTER_KIND)
            }
            (AFTER_KIND, Some(b'"')) => {
                t.enter("admonitionTitle");
                t.enter("admonitionTitleMarker");
                t.consume();
                t.exit("admonitionTitleMarker");
                Step::Next(TITLE)
            }
            (AFTER_KIND | AFTER_TITLE, None | Some(b'\n')) => Step::Retry(BODY),
            (TITLE, Some(b'"')) => Step::Retry(TITLE_CLOSE),
            (TITLE, Some(byte)) if byte != b'\n' => {
                t.enter_content("admonitionTitleText", ContentType::Text);
                t.consume();
                Step::Next(TITLE_TEXT)
            }
            (TITLE_TEXT, Some(b'"')) => {
                t.exit("admonitionTitleText");
                Step::Retry(TITLE_CLOSE)
            }
            (TITLE_TEXT, Some(byte)) if byte != b'\n' => {
                t.consume();
                Step::Next(TITLE_TEXT)
            }
            (TITLE_CLOSE, _) => {
                t.enter("admonitionTitleMarker");
                t.consume();
                t.exit("admonitionTitleMarker");
                t.exit("admonitionTitle");
                Step::Next(AFTER_TITLE)
            }
            (AFTER_TITLE, Some(b'\t' | b' ')) => {
                t.consume();
                Step::Next(AFTER_TITLE)
            }
            // The rest of the line is flow: nothing here.
            (BODY, _) => {
                t.enter_content("admonitionBody", ContentType::Document);
                Step::Ok
            }
            // Later lines: blank, or indented.
            (CONTINUATION, Some(b'\n')) => Step::Ok,
            (CONTINUATION, Some(b'\t' | b' ')) => {
                t.memory()[1] = 0;
                t.enter("admonitionIndent");
                Step::Retry(INDENT)
            }
            (INDENT, Some(byte @ (b'\t' | b' '))) if t.memory()[1] < INDENT_SIZE => {
                t.memory()[1] += if byte == b'\t' { INDENT_SIZE } else { 1 };
                t.consume();
                Step::Next(INDENT)
            }
            (INDENT, current)
                if t.memory()[1] >= INDENT_SIZE || matches!(current, None | Some(b'\n')) =>
            {
                t.exit("admonitionIndent");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        let mut fields = BTreeMap::new();
        let mut kinds: Vec<String> = vec![];
        let mut title = None;
        let mut body = vec![];

        for token in tokens {
            match token.name {
                "admonitionMarker" if token.value.starts_with('?') => {
                    let open = if token.value.ends_with('+') {
                        "open"
                    } else {
                        "closed"
                    };
                    fields.insert("collapsible".into(), open.into());
                }
                "admonitionKind" => kinds.push(token.value.into_owned()),
                "admonitionTitle" => title = Some(vec![]),
                "admonitionTitleText" => title = Some(token.children),
                "admonitionBody" => body = token.children,
                _ => {}
            }
        }

        fields.insert("kind".into(), kinds.first().cloned().unwrap_or_default());
        let mut attributes = BTreeMap::new();
        attributes.insert("class".into(), kinds.join(" "));
        let mut children = vec![];
        // An empty title (`""`) means no title.
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            fields.insert("title".into(), "true".into());
            children.push(mdast::Node::Paragraph(mdast::Paragraph {
                children: title,
                position: None,
            }));
        }
        children.append(&mut body);

        mdast::Node::Custom(mdast::Custom {
            name: NAME.into(),
            fields,
            attributes,
            children,
            ..mdast::Custom::default()
        })
    }
}
