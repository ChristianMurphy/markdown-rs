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

use alloc::{collections::BTreeMap, format, string::String, vec, vec::Vec};
use markdown::{
    extension::{
        construct, Construct, Container, Ctx, HtmlEvent, HtmlWriter, Step, Token, TokenSlice,
        TokenType,
    },
    mdast, ParseOptions,
};
use markdown_processor::{Plugin, Processor};

/// Name of the custom node this plugin makes.
pub const NAME: &str = "admonition";

/// The plugin: `<div class="admonition note">`, or `<details>` when
/// collapsible.
pub struct Admonitions;

impl Plugin for Admonitions {
    fn attach(self, processor: &mut Processor) {
        processor.add_document_construct(AdmonitionSyntax);
        processor.add_hast_handler(NAME, to_hast);
    }
}

/// Parse options with admonitions, for `markdown::to_html_with_options` and
/// `markdown::to_mdast`.
#[must_use]
pub fn parse_options() -> ParseOptions {
    ParseOptions {
        document_constructs: vec![construct(AdmonitionSyntax)],
        ..ParseOptions::default()
    }
}

/// Put line endings between blocks, and around them, as
/// `mdast_util_to_hast` does for the children of a block quote.
fn wrap(nodes: Vec<hast::Node>) -> Vec<hast::Node> {
    let line_ending = || {
        hast::Node::Text(hast::Text {
            value: "\n".into(),
            position: None,
        })
    };
    let mut result = vec![line_ending()];
    for node in nodes {
        result.push(node);
        result.push(line_ending());
    }
    result
}

/// Whether an admonition is collapsible (`???`), and if so, open (`???+`).
fn collapsible(tokens: &[Token<'_, AdmonitionToken>]) -> Option<bool> {
    tokens
        .value(AdmonitionToken::AdmonitionMarker)
        .filter(|marker| marker.starts_with('?'))
        .map(|marker| marker.ends_with('+'))
}

/// Kinds of an admonition, such as `note`.
fn kinds<'a>(tokens: &'a [Token<'_, AdmonitionToken>]) -> impl Iterator<Item = &'a str> {
    tokens
        .iter()
        .filter(|token| token.name == AdmonitionToken::AdmonitionKind)
        .map(|token| &*token.value)
}

/// `note` as `Note`.
fn capitalize(kind: &str) -> String {
    let mut chars = kind.chars();
    chars.next().map_or_else(String::new, |first| {
        let mut title: String = first.to_uppercase().collect();
        title.push_str(chars.as_str());
        title
    })
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
        vec![hast::Node::Text(hast::Text {
            value: capitalize(kind).into(),
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
    let content = wrap(content);

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

fn element(
    tag_name: &'static str,
    class_names: Vec<String>,
    children: Vec<hast::Node>,
) -> hast::Element {
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

/// Columns a body line is indented by.
const INDENT_SIZE: u8 = 4;

fn is_kind_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

/// Tokens of an admonition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, TokenType)]
pub enum AdmonitionToken {
    Admonition,
    AdmonitionMarker,
    AdmonitionKind,
    AdmonitionTitle,
    AdmonitionTitleMarker,
    #[token(content = text)]
    AdmonitionTitleText,
    #[token(content = document)]
    AdmonitionBody,
    AdmonitionIndent,
}

/// Where an admonition is.
#[derive(Clone, Copy, Debug)]
pub enum State {
    Start,
    /// In the marker, with the marker and how many of it so far.
    Marker(u8, u8),
    Plus,
    KindBefore,
    Kind,
    AfterKind,
    Title,
    /// In an attempt: spaces and tabs up to `"`, a blank title, which is no
    /// title, as content cannot be blank.
    BlankTitle,
    /// At the start of a title that is not blank.
    TitleContent,
    TitleText,
    TitleClose,
    AfterTitle,
    /// At the end of the first line.
    Body,
    /// At the start of a later line.
    Continuation,
    /// In the indent of a later line, with the columns so far.
    Indent(u8),
}

/// `!!!` or `???` (`???+`), kinds, an optional `"title"`, then a body of
/// lines indented by four columns, and blank lines.
pub struct AdmonitionSyntax;

impl Construct for AdmonitionSyntax {
    type Place = Container;
    type State = State;
    type Token = AdmonitionToken;

    fn markers(&self) -> &[u8] {
        b"!?"
    }

    fn start(&self) -> State {
        State::Start
    }

    fn step(&self, state: State, t: Ctx<'_, '_, Self>) -> Step<Self> {
        use AdmonitionToken::{
            Admonition, AdmonitionBody, AdmonitionIndent, AdmonitionKind, AdmonitionMarker,
            AdmonitionTitle, AdmonitionTitleMarker, AdmonitionTitleText,
        };

        match (state, t.current()) {
            (State::Start, Some(byte @ (b'!' | b'?'))) => t
                .enter(Admonition)
                .enter(AdmonitionMarker)
                .consume()
                .next(State::Marker(byte, 1)),
            (State::Marker(marker, size), Some(byte)) if byte == marker && size < 3 => {
                t.consume().next(State::Marker(marker, size + 1))
            }
            (State::Marker(b'?', 3), Some(b'+')) => t.consume().next(State::Plus),
            (State::Marker(_, 3) | State::Plus, Some(b'\t' | b' ')) => {
                t.exit().consume().next(State::KindBefore)
            }
            (State::KindBefore | State::AfterKind, Some(b'\t' | b' ')) => t.consume().next(state),
            (State::KindBefore | State::AfterKind, Some(byte)) if is_kind_byte(byte) => {
                t.enter(AdmonitionKind).consume().next(State::Kind)
            }
            (State::Kind, Some(byte)) if is_kind_byte(byte) => t.consume().next(State::Kind),
            (State::Kind, _) => t.exit().retry(State::AfterKind),
            (State::AfterKind, Some(b'"')) => t
                .enter(AdmonitionTitle)
                .enter(AdmonitionTitleMarker)
                .consume()
                .exit()
                .next(State::Title),
            (State::AfterKind | State::AfterTitle, None | Some(b'\n')) => t.retry(State::Body),
            (State::Title, Some(b'"')) => t.retry(State::TitleClose),
            (State::Title, Some(b'\t' | b' ')) => {
                t.attempt(State::BlankTitle, State::TitleClose, State::TitleContent)
            }
            (State::BlankTitle, Some(b'\t' | b' ')) => t.consume().next(State::BlankTitle),
            (State::BlankTitle, Some(b'"')) => t.ok(),
            (State::Title | State::TitleContent, Some(byte)) if byte != b'\n' => t
                .enter(AdmonitionTitleText)
                .consume()
                .next(State::TitleText),
            (State::TitleText, Some(b'"')) => t.exit().retry(State::TitleClose),
            (State::TitleText, Some(byte)) if byte != b'\n' => t.consume().next(State::TitleText),
            (State::TitleClose, Some(b'"')) => t
                .enter(AdmonitionTitleMarker)
                .consume()
                .exit()
                .exit()
                .next(State::AfterTitle),
            (State::AfterTitle, Some(b'\t' | b' ')) => t.consume().next(State::AfterTitle),
            // The rest of the first line is flow: nothing here.
            (State::Body, _) => t.enter(AdmonitionBody).container(State::Continuation),
            // Later lines: blank, or indented; a tab is a column at a time.
            (State::Continuation, None | Some(b'\n')) => t.container(State::Continuation),
            (State::Continuation, Some(b'\t' | b' ')) => {
                t.enter(AdmonitionIndent).consume().next(State::Indent(1))
            }
            (State::Indent(columns), Some(b'\t' | b' ')) if columns < INDENT_SIZE => {
                t.consume().next(State::Indent(columns + 1))
            }
            (State::Indent(columns), current)
                if columns >= INDENT_SIZE || matches!(current, None | Some(b'\n')) =>
            {
                t.exit().container(State::Continuation)
            }
            _ => t.nok(),
        }
    }

    fn to_mdast(&self, mut tokens: Vec<Token<'_, AdmonitionToken>>) -> mdast::Node {
        let mut fields = BTreeMap::new();
        if let Some(open) = collapsible(&tokens) {
            fields.insert(
                "collapsible".into(),
                if open { "open" } else { "closed" }.into(),
            );
        }
        let kinds = kinds(&tokens).collect::<Vec<_>>();
        fields.insert(
            "kind".into(),
            kinds.first().copied().unwrap_or_default().into(),
        );
        let mut attributes = BTreeMap::new();
        attributes.insert("class".into(), kinds.join(" "));
        let title = tokens.take_children(AdmonitionToken::AdmonitionTitleText);
        let mut body = tokens.take_children(AdmonitionToken::AdmonitionBody);
        let mut children = vec![];
        // An empty title (`""`) means no title.
        if !title.is_empty() {
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

    fn html(
        &self,
        event: HtmlEvent<'_, AdmonitionToken>,
        tokens: &[Token<'_, AdmonitionToken>],
        w: &mut HtmlWriter<'_>,
    ) {
        let collapsible = collapsible(tokens);
        let kinds = || kinds(tokens);
        let has_title = tokens.token(AdmonitionToken::AdmonitionTitleText).is_some();
        let close_title = if collapsible.is_some() {
            "</summary>"
        } else {
            "</p>"
        };

        match event {
            HtmlEvent::Enter(token) if token.name == AdmonitionToken::Admonition => {
                let classes = kinds().collect::<Vec<_>>().join(" ");
                if let Some(open) = collapsible {
                    w.tag("<details");
                    w.attr("class", &classes);
                    if open {
                        w.attr("open", "");
                    }
                    w.tag(">\n<summary>");
                } else {
                    w.tag("<div");
                    w.attr("class", &format!("admonition {classes}"));
                    w.tag(">\n<p class=\"admonition-title\">");
                }
                if !has_title {
                    w.text(&capitalize(kinds().next().unwrap_or_default()));
                    w.tag(close_title);
                }
            }
            HtmlEvent::Exit(token) => match token.name {
                AdmonitionToken::AdmonitionTitleText => w.tag(close_title),
                AdmonitionToken::Admonition => {
                    w.line_ending_if_needed();
                    w.tag(if collapsible.is_some() {
                        "</details>"
                    } else {
                        "</div>"
                    });
                }
                _ => {}
            },
            _ => {}
        }
    }
}
