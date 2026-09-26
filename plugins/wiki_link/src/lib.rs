//! Wiki links, as a syntax plugin: `[[Page]]` and `[[Page|alias]]`.
//!
//! ```markdown
//! See [[Home]] or [[Getting started|the guide]].
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
};
use markdown::{
    extension::{ConstructTokenizer, Step, TextConstruct, Token},
    mdast, normalize_uri,
};
use markdown_processor::{hast, visit::visit_mut, Plugin, Processor};

/// Name of the custom node this plugin makes.
const NAME: &str = "wikiLink";

/// The plugin: links go to `base` followed by the page name.
pub struct WikiLinks {
    base: String,
}

impl WikiLinks {
    /// Link pages to `base` followed by the page name, such as `/wiki/`.
    #[must_use]
    pub fn new(base: &str) -> Self {
        WikiLinks { base: base.into() }
    }
}

impl Plugin for WikiLinks {
    fn attach(self, processor: &mut Processor) {
        processor.add_syntax(WikiLinkSyntax);
        processor.add_mdast_transform(|tree| {
            unwrap_in_links(tree);
            Ok(())
        });

        let base = self.base;
        processor.add_hast_handler(NAME, move |node, children| {
            let target = node.attributes.get("target").map_or("", String::as_str);
            vec![hast::Node::Element(hast::Element {
                tag_name: "a".into(),
                properties: vec![(
                    "href".into(),
                    hast::PropertyValue::String(format!(
                        "{}{}",
                        base,
                        normalize_uri(target.trim())
                    )),
                )],
                children,
                position: node.position.clone(),
            })]
        });
    }
}

/// Turn wiki links inside links into their text: links cannot nest.
fn unwrap_in_links(tree: &mut mdast::Node) {
    visit_mut(tree, &mut |node| {
        if let mdast::Node::Link(_) | mdast::Node::LinkReference(_) = node {
            for child in node.children_mut().into_iter().flatten() {
                visit_mut(child, &mut |node| {
                    if let mdast::Node::Custom(custom) = node {
                        if custom.name == NAME {
                            *node = mdast::Node::Text(mdast::Text {
                                value: node.to_string(),
                                position: node.position().cloned(),
                            });
                        }
                    }
                });
            }
        }
    });
}

/// Whether `byte` can be part of a target or alias.
fn is_part_byte(byte: u8) -> bool {
    !matches!(byte, b'\n' | b'[' | b'|')
}

const START: u16 = 0;
const OPEN: u16 = 1;
/// In the target, before anything but whitespace.
const TARGET_START: u16 = 2;
const TARGET: u16 = 3;
const ALIAS_START: u16 = 4;
const ALIAS: u16 = 5;
const CLOSE: u16 = 6;
const CLOSE_SECOND: u16 = 7;

/// The syntax: `[[`, a target, optionally `|` and an alias, then `]]`, on one line.
struct WikiLinkSyntax;

impl TextConstruct for WikiLinkSyntax {
    fn markers(&self) -> &[u8] {
        b"["
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (START, Some(b'[')) => {
                t.enter("wikiLink");
                t.enter("wikiLinkMarker");
                t.consume();
                Step::Next(OPEN)
            }
            (OPEN, Some(b'[')) => {
                t.consume();
                t.exit("wikiLinkMarker");
                t.enter("wikiLinkTarget");
                Step::Next(TARGET_START)
            }
            (TARGET_START, Some(b' ' | b'\t')) => {
                t.consume();
                Step::Next(TARGET_START)
            }
            (TARGET_START, Some(b']')) => Step::Nok,
            (TARGET_START, Some(byte)) if is_part_byte(byte) => {
                t.consume();
                Step::Next(TARGET)
            }
            (TARGET, Some(b'|')) => {
                t.exit("wikiLinkTarget");
                t.enter("wikiLinkAliasMarker");
                t.consume();
                t.exit("wikiLinkAliasMarker");
                Step::Next(ALIAS_START)
            }
            (TARGET, Some(b']')) => {
                t.exit("wikiLinkTarget");
                Step::Retry(CLOSE)
            }
            (ALIAS_START, Some(b']')) => Step::Retry(CLOSE),
            (ALIAS_START, Some(byte)) if is_part_byte(byte) => {
                t.enter("wikiLinkAlias");
                Step::Retry(ALIAS)
            }
            (ALIAS, Some(b']')) => {
                t.exit("wikiLinkAlias");
                Step::Retry(CLOSE)
            }
            (TARGET | ALIAS, Some(byte)) if is_part_byte(byte) => {
                t.consume();
                Step::Next(state)
            }
            (CLOSE, Some(b']')) => {
                t.enter("wikiLinkMarker");
                t.consume();
                Step::Next(CLOSE_SECOND)
            }
            (CLOSE_SECOND, Some(b']')) => {
                t.consume();
                t.exit("wikiLinkMarker");
                t.exit("wikiLink");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: &[Token]) -> mdast::Node {
        let find = |name: &str| tokens.iter().find(|token| token.name == name);
        let target = find("wikiLinkTarget").expect("expected target");
        let label = find("wikiLinkAlias")
            .filter(|alias| !alias.value.trim().is_empty())
            .unwrap_or(target);
        let mut attributes = BTreeMap::new();
        attributes.insert("target".into(), target.value.clone().into_owned());

        mdast::Node::Custom(mdast::Custom {
            name: NAME.into(),
            attributes,
            children: vec![mdast::Node::Text(mdast::Text {
                value: label.value.clone().into_owned(),
                position: Some(label.position.clone()),
            })],
            ..mdast::Custom::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use markdown::{to_mdast, unist::Position, ParseOptions};

    #[test]
    fn makes_custom_node_with_positions() {
        let options = ParseOptions {
            text_constructs: vec![Box::new(WikiLinkSyntax)],
            ..ParseOptions::default()
        };
        let tree = to_mdast("a [[b|c]]", &options).unwrap();
        let node = &tree.children().unwrap()[0].children().unwrap()[1];

        let custom = match node {
            mdast::Node::Custom(custom) => custom,
            _ => panic!("expected custom node"),
        };
        assert_eq!(custom.name, "wikiLink");
        assert_eq!(
            custom.attributes.get("target").map(String::as_str),
            Some("b")
        );
        assert_eq!(
            custom.position,
            Some(Position::new(1, 3, 2, 1, 10, 9)),
            "should span the whole link"
        );
        assert_eq!(
            custom.children[0].position(),
            Some(&Position::new(1, 7, 6, 1, 8, 7)),
            "should give the label the alias position"
        );
    }
}
