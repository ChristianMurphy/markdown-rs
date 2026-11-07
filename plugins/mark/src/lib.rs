//! Highlighted text, as a delimiter run, like
//! `micromark-extension-highlight-mark`: `==mark==` is `<mark>mark</mark>`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::vec::Vec;
use markdown::{
    extension::{
        construct, Construct, Ctx, HtmlEvent, HtmlWriter, Reach, RunToken, Step, Text, Token,
        TokenSlice,
    },
    mdast, ParseOptions,
};
use markdown_processor::{Plugin, Processor};

/// Name of the custom node this plugin makes.
pub const NAME: &str = "mark";

/// The plugin: `==` pairs like `~~` in GFM strikethrough.
pub struct Mark;

impl Plugin for Mark {
    fn attach(self, processor: &mut Processor) {
        processor.add_text_construct(MarkSyntax);
    }
}

/// Parse options with marks, for `markdown::to_html_with_options` and
/// `markdown::to_mdast`.
#[must_use]
pub fn parse_options() -> ParseOptions {
    ParseOptions {
        text_constructs: alloc::vec![construct(MarkSyntax)],
        ..ParseOptions::default()
    }
}

/// `==`, paired by the attention resolver.
pub struct MarkSyntax;

impl Construct for MarkSyntax {
    type Place = Text;
    type State = ();
    type Token = RunToken;

    fn markers(&self) -> &[u8] {
        b"="
    }

    fn start(&self) {}

    fn reach(&self) -> Reach {
        Reach::Attention(&[2])
    }

    fn step(&self, (): (), t: Ctx<'_, '_, Self>) -> Step<Self> {
        // A delimiter run is paired by the resolver, never stepped.
        t.nok()
    }

    fn to_mdast(&self, mut tokens: Vec<Token<'_, RunToken>>) -> mdast::Node {
        let children = tokens.take_children(RunToken::Text);
        mdast::Node::Custom(mdast::Custom {
            name: NAME.into(),
            h_name: Some("mark".into()),
            children,
            ..mdast::Custom::default()
        })
    }

    fn html(
        &self,
        event: HtmlEvent<'_, RunToken>,
        _: &[Token<'_, RunToken>],
        w: &mut HtmlWriter<'_>,
    ) {
        match event {
            HtmlEvent::Enter(token) if token.name == RunToken::Run => w.tag("<mark>"),
            HtmlEvent::Exit(token) if token.name == RunToken::Run => w.tag("</mark>"),
            // The `==` sequences are not written.
            _ => {}
        }
    }
}
