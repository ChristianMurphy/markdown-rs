//! Wiki links: `[[Page]]` and `[[Page|alias]]`.
//!
//! ```markdown
//! See [[Home]] or [[Getting started|the guide]].
//! ```
//!
//! A link goes to a base followed by the page, trimmed and normalized.
//! The alias is markdown; without one, or with a blank one, the link shows
//! the page as written.
//! In `Processor`, a wiki link inside a link becomes its text, as links do
//! not nest; `to_html` has no tree to check, so there it stays a link.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use markdown::{
    extension::{
        construct, Construct, Ctx, HtmlEvent, HtmlWriter, Step, Text, Token, TokenSlice, TokenType,
    },
    mdast, normalize_uri, ParseOptions,
};
use markdown_processor::{visit::visit_mut, Plugin, Processor};

/// Name of the custom node this plugin makes.
pub const NAME: &str = "wikiLink";

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
        processor.add_text_construct(WikiLinkSyntax { base: self.base });
        processor.add_mdast_transform(|tree| {
            unwrap_in_links(tree);
            Ok(())
        });
    }
}

/// Parse options with wiki links to `base`, for `markdown::to_html_with_options`
/// and `markdown::to_mdast`.
#[must_use]
pub fn parse_options(base: &str) -> ParseOptions {
    ParseOptions {
        text_constructs: vec![construct(WikiLinkSyntax { base: base.into() })],
        ..ParseOptions::default()
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

/// Tokens of a wiki link.
#[derive(Clone, Copy, Debug, Eq, PartialEq, TokenType)]
pub enum WikiLinkToken {
    WikiLink,
    WikiLinkMarker,
    WikiLinkTarget,
    WikiLinkAliasMarker,
    #[token(content = text)]
    WikiLinkAlias,
}

/// Where a wiki link is.
#[derive(Clone, Copy, Debug)]
pub enum State {
    Start,
    Open,
    /// In the target, before anything but whitespace.
    TargetStart,
    Target,
    AliasStart,
    /// In an attempt: spaces and tabs up to `]`, a blank alias, which is no
    /// alias, as content cannot be blank.
    Blank,
    /// At the start of an alias that is not blank.
    AliasContent,
    Alias,
    Close,
    CloseSecond,
}

/// Whether `byte` can be part of a target or alias.
fn is_part_byte(byte: u8) -> bool {
    !matches!(byte, b'\n' | b'[' | b'|')
}

/// The syntax: `[[`, a target, optionally `|` and an alias, then `]]`, on
/// one line.
///
/// It needs no bound on its reach: a match stops at the next `[` or line
/// ending, so no byte is read by more than the matches at the two `[` of an
/// opener, and parsing stays linear.
/// `Reach::Pair` would lose links: in `[[[a]]`, the match at the first `[`
/// fails at the third, so the one at the second would not be tried.
pub struct WikiLinkSyntax {
    base: String,
}

impl WikiLinkSyntax {
    /// URL of the page `target`.
    fn url(&self, target: &str) -> String {
        format!("{}{}", self.base, normalize_uri(target.trim()))
    }
}

impl Construct for WikiLinkSyntax {
    type Place = Text;
    type State = State;
    type Token = WikiLinkToken;

    fn markers(&self) -> &[u8] {
        b"["
    }

    fn start(&self) -> State {
        State::Start
    }

    fn step(&self, state: State, t: Ctx<'_, '_, Self>) -> Step<Self> {
        use WikiLinkToken::{
            WikiLink, WikiLinkAlias, WikiLinkAliasMarker, WikiLinkMarker, WikiLinkTarget,
        };

        match (state, t.current()) {
            (State::Start, Some(b'[')) => t
                .enter(WikiLink)
                .enter(WikiLinkMarker)
                .consume()
                .next(State::Open),
            (State::Open, Some(b'[')) => t
                .consume()
                .exit()
                .enter(WikiLinkTarget)
                .next(State::TargetStart),
            (State::TargetStart, Some(b' ' | b'\t')) => t.consume().next(State::TargetStart),
            (State::TargetStart, Some(byte)) if byte != b']' && is_part_byte(byte) => {
                t.consume().next(State::Target)
            }
            (State::Target, Some(b'|')) => t
                .exit()
                .enter(WikiLinkAliasMarker)
                .consume()
                .exit()
                .next(State::AliasStart),
            (State::Target | State::Alias, Some(b']')) => t.exit().retry(State::Close),
            (State::AliasStart, Some(b']')) => t.retry(State::Close),
            (State::AliasStart, Some(b' ' | b'\t')) => {
                t.attempt(State::Blank, State::Close, State::AliasContent)
            }
            (State::Blank, Some(b' ' | b'\t')) => t.consume().next(State::Blank),
            (State::Blank, Some(b']')) => t.ok(),
            (State::AliasStart | State::AliasContent, Some(byte)) if is_part_byte(byte) => {
                t.enter(WikiLinkAlias).retry(State::Alias)
            }
            (State::Target | State::Alias, Some(byte)) if is_part_byte(byte) => {
                t.consume().next(state)
            }
            (State::Close, Some(b']')) => {
                t.enter(WikiLinkMarker).consume().next(State::CloseSecond)
            }
            (State::CloseSecond, Some(b']')) => t.consume().exit().exit().ok(),
            _ => t.nok(),
        }
    }

    fn to_mdast(&self, mut tokens: Vec<Token<'_, WikiLinkToken>>) -> mdast::Node {
        let mut children = tokens.take_children(WikiLinkToken::WikiLinkAlias);
        let target = tokens
            .token(WikiLinkToken::WikiLinkTarget)
            .expect("expected target");
        // An empty or whitespace-only alias falls back to the target.
        let alias: String = children.iter().map(ToString::to_string).collect();
        if alias.trim().is_empty() {
            children = vec![mdast::Node::Text(mdast::Text {
                value: target.value.to_string(),
                position: Some(target.position.clone()),
            })];
        }

        mdast::Node::Custom(mdast::Custom {
            name: NAME.into(),
            fields: vec![("target".into(), target.value.to_string())]
                .into_iter()
                .collect(),
            attributes: vec![("href".into(), self.url(&target.value))]
                .into_iter()
                .collect(),
            h_name: Some("a".into()),
            children,
            ..mdast::Custom::default()
        })
    }

    fn html(
        &self,
        event: HtmlEvent<'_, WikiLinkToken>,
        tokens: &[Token<'_, WikiLinkToken>],
        w: &mut HtmlWriter<'_>,
    ) {
        let target = || {
            tokens
                .token(WikiLinkToken::WikiLinkTarget)
                .expect("expected target")
        };
        let has_alias = || tokens.token(WikiLinkToken::WikiLinkAlias).is_some();

        match event {
            HtmlEvent::Enter(token) => match token.name {
                WikiLinkToken::WikiLink => {
                    w.tag("<a");
                    w.attr("href", &self.url(&target().value));
                    w.tag(">");
                    if !has_alias() {
                        w.text(&target().value);
                    }
                }
                // A blank alias falls back to the target.
                WikiLinkToken::WikiLinkAlias => w.buffer(),
                _ => {}
            },
            HtmlEvent::Exit(token) => match token.name {
                WikiLinkToken::WikiLink => w.tag("</a>"),
                WikiLinkToken::WikiLinkAlias => {
                    let alias = w.resume();
                    if alias.as_str().trim().is_empty() {
                        w.text(&target().value);
                    } else {
                        w.html(&alias);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdown::{to_mdast, unist::Position};

    #[test]
    fn makes_custom_node_with_positions() {
        let tree = to_mdast("a [[b|c]]", &parse_options("")).unwrap();
        let node = &tree.children().unwrap()[0].children().unwrap()[1];

        let custom = match node {
            mdast::Node::Custom(custom) => custom,
            _ => panic!("expected custom node"),
        };
        assert_eq!(custom.name, "wikiLink");
        assert_eq!(custom.fields.get("target").map(String::as_str), Some("b"));
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
