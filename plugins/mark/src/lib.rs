//! Highlighted text, as a delimiter-run plugin, like
//! `micromark-extension-highlight-mark`: `==mark==` is `<mark>mark</mark>`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::{vec, vec::Vec};
use markdown::{
    extension::{Construct, ConstructTokenizer, Step, Token},
    mdast,
};
use markdown_processor::{hast, Plugin, Processor};

/// Name of the custom node this plugin makes.
pub const NAME: &str = "mark";

/// The plugin.
pub struct Mark;

impl Plugin for Mark {
    fn attach(self, processor: &mut Processor) {
        processor.add_syntax(MarkSyntax);
        processor.add_hast_handler(NAME, |node, children| {
            vec![hast::Node::Element(hast::Element {
                tag_name: "mark".into(),
                properties: vec![],
                children,
                position: node.position.clone(),
            })]
        });
    }
}

/// `==`, paired by the core like strikethrough `~~`.
struct MarkSyntax;

impl Construct for MarkSyntax {
    fn markers(&self) -> &[u8] {
        b"="
    }

    fn attention_sizes(&self) -> &[usize] {
        &[2]
    }

    fn step(&self, _state: u16, _tokenizer: &mut ConstructTokenizer) -> Step {
        Step::Nok
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        let children = tokens
            .into_iter()
            .find(|token| token.name == "attentionText")
            .map(|token| token.children)
            .unwrap_or_default();
        mdast::Node::Custom(mdast::Custom {
            name: NAME.into(),
            children,
            ..mdast::Custom::default()
        })
    }
}
