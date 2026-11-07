//! Directives, as a syntax plugin, ported from
//! [`micromark-extension-directive`](https://github.com/micromark/micromark-extension-directive).
//!
//! ```markdown
//! A :abbr[HTML]{title="HyperText Markup Language"} page.
//!
//! ::youtube[Video]{#dQw4w9WgXcQ}
//!
//! :::note[Heads up]
//! Nested *markdown*.
//! :::
//! ```
//!
//! Nodes follow `mdast-util-directive`: `textDirective`, `leafDirective`,
//! and `containerDirective`, with the name in `fields.name`, and the label
//! of a container as a first paragraph, flagged by `fields.label`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use markdown::{
    decode_named, decode_numeric,
    extension::{
        classify, construct, CharacterKind, Construct, Ctx, Flow, HtmlEvent, HtmlWriter, Reach,
        Step, Text, Token, TokenSlice, TokenType,
    },
    mdast, ParseOptions,
};
use markdown_processor::{Plugin, Processor};

/// Name of the text directive node.
pub const TEXT: &str = "textDirective";
/// Name of the leaf directive node.
pub const LEAF: &str = "leafDirective";
/// Name of the container directive node.
pub const CONTAINER: &str = "containerDirective";

/// The plugin: syntax, and HTML as `<span>` (text) or `<div>` (leaf,
/// container), with the name as a class.
pub struct Directives;

impl Plugin for Directives {
    fn attach(self, processor: &mut Processor) {
        processor.add_text_construct(DirectiveText);
        // Like micromark: containers before leaves, both at `:`.
        processor.add_flow_construct(DirectiveContainer);
        processor.add_flow_construct(DirectiveLeaf);

        processor.add_hast_handler(TEXT, |node, children| to_hast(node, "span", children));
        processor.add_hast_handler(LEAF, |node, children| to_hast(node, "div", children));
        processor.add_hast_handler(CONTAINER, |node, children| {
            to_hast(node, "div", wrap(children))
        });
    }
}

/// Parse options with directives, for `markdown::to_html_with_options` and
/// `markdown::to_mdast`.
#[must_use]
pub fn parse_options() -> ParseOptions {
    ParseOptions {
        text_constructs: vec![construct(DirectiveText)],
        flow_constructs: vec![construct(DirectiveContainer), construct(DirectiveLeaf)],
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

/// Whether an attribute is written: never event handlers or URLs, which
/// come from the author.
fn is_safe_attribute(key: &str) -> bool {
    matches!(key, "id" | "title" | "lang" | "dir") || key.starts_with("data-")
}

/// Element for a directive: sanitized attributes, the name as a class.
fn to_hast(
    node: &mdast::Custom,
    tag_name: &'static str,
    children: Vec<hast::Node>,
) -> Vec<hast::Node> {
    let mut class = node.fields.get("name").cloned().unwrap_or_default();
    let mut properties = vec![];
    for (key, value) in &node.attributes {
        match key.as_str() {
            "class" => {
                class.push(' ');
                class.push_str(value);
            }
            _ if is_safe_attribute(key) => {
                properties.push((
                    key.clone().into(),
                    hast::PropertyValue::String(value.clone()),
                ));
            }
            _ => {}
        }
    }
    properties.push((
        "className".into(),
        hast::PropertyValue::SpaceSeparated(class.split_whitespace().map(Into::into).collect()),
    ));

    vec![hast::Node::Element(hast::Element {
        tag_name: tag_name.into(),
        properties,
        children,
        position: node.position.clone(),
    })]
}

/// Like micromark’s `linkReferenceSizeMax`: a match reads at most this many
/// bytes, which keeps parsing linear, as labels nest.
const SIZE_MAX: usize = 999;
/// Like micromark’s `linkResourceDestinationBalanceMax`.
const LABEL_BALANCE_MAX: usize = 32;

/// Tokens of directives.
#[derive(Clone, Copy, Debug, Eq, PartialEq, TokenType)]
pub enum DirectiveToken {
    DirectiveText,
    DirectiveLeaf,
    DirectiveContainer,
    DirectiveMarker,
    DirectiveSequence,
    DirectiveContainerFence,
    DirectiveContainerSequence,
    #[token(content = document)]
    DirectiveContainerContent,
    LinePrefix,
    DirectiveName,
    DirectiveLabel,
    DirectiveLabelMarker,
    #[token(content = text)]
    DirectiveLabelString,
    DirectiveAttributes,
    DirectiveAttributesMarker,
    DirectiveAttribute,
    DirectiveAttributeId,
    DirectiveAttributeIdMarker,
    DirectiveAttributeIdValue,
    DirectiveAttributeClass,
    DirectiveAttributeClassMarker,
    DirectiveAttributeClassValue,
    DirectiveAttributeName,
    DirectiveAttributeInitializerMarker,
    DirectiveAttributeValueLiteral,
    DirectiveAttributeValue,
    DirectiveAttributeValueMarker,
    DirectiveAttributeValueData,
}

use DirectiveToken as T;

/// States of a name, like micromark’s factory.
#[derive(Clone, Copy, Debug)]
pub enum Name {
    Start,
    /// In a character, or after one.
    Char,
    Inside,
    AfterDash,
}

/// States of a label: `[`, text with balanced brackets, `]`.
#[derive(Clone, Copy, Debug)]
pub enum Label {
    Start,
    AfterStart,
    /// In the text, with the brackets open in it.
    Data(usize),
    Escape(usize),
    Close,
    Done,
}

/// States of attributes: `{`, `#id`, `.class`, `key`, `key=value`,
/// `key="value"`, `}`; markers are `#` or `.`, or a quote.
#[derive(Clone, Copy, Debug)]
pub enum Attributes {
    Start,
    Between,
    ShortcutStart(u8),
    ShortcutStartAfter(u8),
    Shortcut(u8),
    Name,
    NameAfter,
    ValueBefore,
    ValueUnquoted,
    ValueQuotedStart(u8),
    ValueQuotedBetween(u8),
    ValueQuoted(u8),
    ValueQuotedAfter,
    End,
    Done,
}

/// States of the three constructs, with the states they share; a container
/// carries the size of its opening sequence.
#[derive(Clone, Copy, Debug)]
pub enum State {
    Start,
    Name(Name),
    Label(Label),
    Attributes(Attributes),
    Nok,
    TextAfterMarker,
    TextAfterName,
    TextAfterLabel,
    TextDone,
    LeafSequence,
    LeafAfterSequence,
    LeafAfterName,
    LeafAfterLabel,
    LeafAfterAttributes,
    OpenSequence(usize),
    OpenAfterName(usize),
    OpenAfterLabel(usize),
    OpenAfterAttributes(usize),
    OpenEol(usize),
    ContentStart(usize),
    LineStart(usize),
    PrefixStart(usize),
    /// In the indent of a body line, with its columns so far.
    PrefixInside(usize, usize),
    Chunk(usize),
    Close(usize),
    CloseIndent(usize, usize),
    /// In a closing sequence, with the sizes of the opening and this one.
    CloseSequence(usize, usize),
    CloseAfter,
    AfterContent,
}

fn is_continuation(byte: Option<u8>) -> bool {
    matches!(byte, Some(0x80..=0xBF))
}

/// Whether `char` is neither whitespace nor punctuation.
fn is_other(char: char) -> bool {
    char != '\n' && classify(char) == CharacterKind::Other
}

/// Whether the current byte is whitespace between attributes.
fn is_attribute_whitespace<C: Construct>(t: &Ctx<'_, '_, C>, disallow_eol: bool) -> bool {
    match t.current() {
        Some(b'\t' | b' ') => true,
        Some(b'\n') => !disallow_eol,
        _ => false,
    }
}

/// A construct of this plugin, which shares the states of names, labels,
/// and attributes.
trait Directive: Construct<State = State, Token = DirectiveToken> {
    /// Whether labels and attributes stay on one line.
    const DISALLOW_EOL: bool;
}

/// Run a shared state, if `state` is one.
fn factory<'t, 'a, C: Directive>(
    state: State,
    t: Ctx<'t, 'a, C>,
) -> Result<Step<C>, Ctx<'t, 'a, C>> {
    match state {
        State::Name(state) => Ok(name(state, t)),
        State::Label(state) => Ok(label(state, t, C::DISALLOW_EOL)),
        State::Attributes(state) => Ok(attributes(state, t, C::DISALLOW_EOL)),
        State::Nok => Ok(t.nok()),
        _ => Err(t),
    }
}

/// Name: no whitespace or punctuation other than `-` and `_`, not ending in
/// them.
fn name<C: Directive>(state: Name, t: Ctx<'_, '_, C>) -> Step<C> {
    match state {
        Name::Start => match t.current_char() {
            Some(char) if is_other(char) => t
                .enter(T::DirectiveName)
                .consume()
                .next(State::Name(Name::Char)),
            _ => t.nok(),
        },
        // Rest of a character.
        Name::Char if is_continuation(t.current()) => t.consume().next(State::Name(Name::Char)),
        Name::Char => t.retry(State::Name(Name::Inside)),
        Name::Inside | Name::AfterDash => match t.current_char() {
            Some('-' | '_') => t.consume().next(State::Name(Name::AfterDash)),
            Some(char) if is_other(char) => t.consume().next(State::Name(Name::Char)),
            _ if matches!(state, Name::AfterDash) => t.nok(),
            _ => t.exit().ok(),
        },
    }
}

/// Label: `[`, text with balanced brackets, `]`.
fn label<C: Directive>(state: Label, t: Ctx<'_, '_, C>, disallow_eol: bool) -> Step<C> {
    match (state, t.current()) {
        (Label::Start, Some(b'[')) => t
            .enter(T::DirectiveLabel)
            .enter(T::DirectiveLabelMarker)
            .consume()
            .exit()
            .next(State::Label(Label::AfterStart)),
        (Label::AfterStart, Some(b']')) => t
            .enter(T::DirectiveLabelMarker)
            .consume()
            .exit()
            .exit()
            .next(State::Label(Label::Done)),
        (Label::AfterStart, _) => t
            .enter(T::DirectiveLabelString)
            .retry(State::Label(Label::Data(0))),
        (Label::Data(balance), Some(b'[')) if balance < LABEL_BALANCE_MAX => {
            t.consume().next(State::Label(Label::Data(balance + 1)))
        }
        (Label::Data(0), Some(b']')) => t.retry(State::Label(Label::Close)),
        (Label::Data(balance), Some(b']')) => {
            t.consume().next(State::Label(Label::Data(balance - 1)))
        }
        (Label::Data(_), Some(b'\n')) if disallow_eol => t.nok(),
        (Label::Data(balance), Some(b'\\')) => {
            t.consume().next(State::Label(Label::Escape(balance)))
        }
        (Label::Data(balance), Some(byte)) if byte != b'[' => {
            t.consume().next(State::Label(Label::Data(balance)))
        }
        (Label::Escape(balance), Some(b'[' | b'\\' | b']')) => {
            t.consume().next(State::Label(Label::Data(balance)))
        }
        (Label::Escape(balance), _) => t.retry(State::Label(Label::Data(balance))),
        (Label::Close, _) => t
            .exit()
            .enter(T::DirectiveLabelMarker)
            .consume()
            .exit()
            .exit()
            .next(State::Label(Label::Done)),
        (Label::Done, _) => t.ok(),
        _ => t.nok(),
    }
}

/// Attributes: `{`, `#id`, `.class`, `key`, `key=value`, `key="value"`,
/// `}`.
// One state machine, as in micromark's factory.
#[allow(clippy::too_many_lines)]
fn attributes<C: Directive>(state: Attributes, t: Ctx<'_, '_, C>, disallow_eol: bool) -> Step<C> {
    let current = t.current();
    let is_whitespace = is_attribute_whitespace(&t, disallow_eol);
    let next = |state| State::Attributes(state);

    match state {
        Attributes::Start => t
            .enter(T::DirectiveAttributes)
            .enter(T::DirectiveAttributesMarker)
            .consume()
            .exit()
            .next(next(Attributes::Between)),
        Attributes::Between => match current {
            Some(marker @ (b'#' | b'.')) => t.retry(next(Attributes::ShortcutStart(marker))),
            _ if is_whitespace => t.consume().next(next(Attributes::Between)),
            _ => match t.current_char() {
                Some(char) if is_other(char) || matches!(char, '-' | '_') => t
                    .enter(T::DirectiveAttribute)
                    .enter(T::DirectiveAttributeName)
                    .consume()
                    .next(next(Attributes::Name)),
                _ => t.retry(next(Attributes::End)),
            },
        },
        Attributes::ShortcutStart(marker) => {
            let (kind, kind_marker) = if marker == b'#' {
                (T::DirectiveAttributeId, T::DirectiveAttributeIdMarker)
            } else {
                (T::DirectiveAttributeClass, T::DirectiveAttributeClassMarker)
            };
            t.enter(T::DirectiveAttribute)
                .enter(kind)
                .enter(kind_marker)
                .consume()
                .exit()
                .next(next(Attributes::ShortcutStartAfter(marker)))
        }
        Attributes::ShortcutStartAfter(_)
            if matches!(
                current,
                None | Some(
                    b'"' | b'#'
                        | b'\''
                        | b'.'
                        | b'<'
                        | b'='
                        | b'>'
                        | b'`'
                        | b'}'
                        | b'\t'
                        | b'\n'
                        | b' '
                )
            ) =>
        {
            t.nok()
        }
        Attributes::Shortcut(_)
            if matches!(
                current,
                None | Some(b'"' | b'\'' | b'<' | b'=' | b'>' | b'`')
            ) =>
        {
            t.nok()
        }
        Attributes::ShortcutStartAfter(marker) => {
            let value = if marker == b'#' {
                T::DirectiveAttributeIdValue
            } else {
                T::DirectiveAttributeClassValue
            };
            t.enter(value)
                .consume()
                .next(next(Attributes::Shortcut(marker)))
        }
        Attributes::Shortcut(_)
            if matches!(current, Some(b'#' | b'.' | b'}' | b'\t' | b'\n' | b' ')) =>
        {
            t.exit().exit().exit().retry(next(Attributes::Between))
        }
        Attributes::Shortcut(marker) => t.consume().next(next(Attributes::Shortcut(marker))),
        Attributes::Name if is_continuation(current) => t.consume().next(next(Attributes::Name)),
        Attributes::Name => match t.current_char() {
            Some(char) if is_other(char) || matches!(char, '-' | '.' | ':' | '_') => {
                t.consume().next(next(Attributes::Name))
            }
            _ => t.exit().retry(next(Attributes::NameAfter)),
        },
        Attributes::NameAfter if is_whitespace => t.consume().next(next(Attributes::NameAfter)),
        Attributes::NameAfter if current == Some(b'=') => t
            .enter(T::DirectiveAttributeInitializerMarker)
            .consume()
            .exit()
            .next(next(Attributes::ValueBefore)),
        Attributes::NameAfter => t.exit().retry(next(Attributes::Between)),
        Attributes::ValueBefore => match current {
            None | Some(b'<' | b'=' | b'>' | b'`' | b'}') => t.nok(),
            Some(b'\n') if disallow_eol => t.nok(),
            Some(quote @ (b'"' | b'\'')) => t
                .enter(T::DirectiveAttributeValueLiteral)
                .enter(T::DirectiveAttributeValueMarker)
                .consume()
                .exit()
                .next(next(Attributes::ValueQuotedStart(quote))),
            _ if is_whitespace => t.consume().next(next(Attributes::ValueBefore)),
            _ => t
                .enter(T::DirectiveAttributeValue)
                .enter(T::DirectiveAttributeValueData)
                .consume()
                .next(next(Attributes::ValueUnquoted)),
        },
        Attributes::ValueUnquoted => match current {
            None | Some(b'"' | b'\'' | b'<' | b'=' | b'>' | b'`') => t.nok(),
            Some(b'}' | b'\t' | b'\n' | b' ') => {
                t.exit().exit().exit().retry(next(Attributes::Between))
            }
            _ => t.consume().next(next(Attributes::ValueUnquoted)),
        },
        Attributes::ValueQuotedStart(quote) if current == Some(quote) => t
            .enter(T::DirectiveAttributeValueMarker)
            .consume()
            .exit()
            .exit()
            .exit()
            .next(next(Attributes::ValueQuotedAfter)),
        Attributes::ValueQuotedStart(quote) => t
            .enter(T::DirectiveAttributeValue)
            .retry(next(Attributes::ValueQuotedBetween(quote))),
        Attributes::ValueQuotedBetween(quote) if current == Some(quote) => {
            t.exit().retry(next(Attributes::ValueQuotedStart(quote)))
        }
        Attributes::ValueQuotedBetween(quote) => match current {
            None => t.nok(),
            Some(b'\n') if disallow_eol => t.nok(),
            Some(b'\n') => t
                .consume()
                .next(next(Attributes::ValueQuotedBetween(quote))),
            _ => t
                .enter(T::DirectiveAttributeValueData)
                .consume()
                .next(next(Attributes::ValueQuoted(quote))),
        },
        Attributes::ValueQuoted(quote)
            if current == Some(quote) || matches!(current, None | Some(b'\n')) =>
        {
            t.exit().retry(next(Attributes::ValueQuotedBetween(quote)))
        }
        Attributes::ValueQuoted(quote) => t.consume().next(next(Attributes::ValueQuoted(quote))),
        Attributes::ValueQuotedAfter => match current {
            Some(b'}' | b'\t' | b'\n' | b' ') => t.retry(next(Attributes::Between)),
            _ => t.retry(next(Attributes::End)),
        },
        Attributes::End if current == Some(b'}') => t
            .enter(T::DirectiveAttributesMarker)
            .consume()
            .exit()
            .exit()
            .next(next(Attributes::Done)),
        Attributes::Done => t.ok(),
        Attributes::End => t.nok(),
    }
}

/// `:name[label]{attributes}`, in text.
pub struct DirectiveText;

impl Directive for DirectiveText {
    const DISALLOW_EOL: bool = false;
}

impl Construct for DirectiveText {
    type Place = Text;
    type State = State;
    type Token = DirectiveToken;

    fn markers(&self) -> &[u8] {
        b":"
    }

    fn start(&self) -> State {
        State::Start
    }

    fn reach(&self) -> Reach {
        Reach::Bytes(SIZE_MAX)
    }

    fn previous(&self, previous: Option<char>) -> bool {
        previous != Some(':')
    }

    fn step(&self, state: State, t: Ctx<'_, '_, Self>) -> Step<Self> {
        let t = match factory(state, t) {
            Ok(step) => return step,
            Err(t) => t,
        };

        match (state, t.current()) {
            (State::Start, Some(b':')) => t
                .enter(T::DirectiveText)
                .enter(T::DirectiveMarker)
                .consume()
                .exit()
                .next(State::TextAfterMarker),
            (State::TextAfterMarker, _) => {
                t.attempt(State::Name(Name::Start), State::TextAfterName, State::Nok)
            }
            (State::TextAfterName, Some(b'[')) => t.attempt(
                State::Label(Label::Start),
                State::TextAfterLabel,
                State::TextAfterLabel,
            ),
            (State::TextAfterName, Some(b':')) => t.nok(),
            (State::TextAfterName, _) => t.retry(State::TextAfterLabel),
            (State::TextAfterLabel, Some(b'{')) => t.attempt(
                State::Attributes(Attributes::Start),
                State::TextDone,
                State::TextDone,
            ),
            (State::TextAfterLabel | State::TextDone, _) => t.exit().ok(),
            _ => t.nok(),
        }
    }

    fn to_mdast(&self, tokens: Vec<Token<'_, DirectiveToken>>) -> mdast::Node {
        to_mdast(TEXT, tokens)
    }

    fn html(
        &self,
        event: HtmlEvent<'_, DirectiveToken>,
        tokens: &[Token<'_, DirectiveToken>],
        w: &mut HtmlWriter<'_>,
    ) {
        html(&event, tokens, w, T::DirectiveText, "span");
    }
}

/// `::name[label]{attributes}`, on its own line.
pub struct DirectiveLeaf;

impl Directive for DirectiveLeaf {
    const DISALLOW_EOL: bool = true;
}

impl Construct for DirectiveLeaf {
    type Place = Flow;
    type State = State;
    type Token = DirectiveToken;

    fn markers(&self) -> &[u8] {
        b":"
    }

    fn start(&self) -> State {
        State::Start
    }

    fn step(&self, state: State, t: Ctx<'_, '_, Self>) -> Step<Self> {
        let t = match factory(state, t) {
            Ok(step) => return step,
            Err(t) => t,
        };

        match (state, t.current()) {
            (State::Start, Some(b':')) => t
                .enter(T::DirectiveLeaf)
                .enter(T::DirectiveSequence)
                .consume()
                .next(State::LeafSequence),
            (State::LeafSequence, Some(b':')) => t.consume().exit().next(State::LeafAfterSequence),
            (State::LeafAfterSequence, _) => {
                t.attempt(State::Name(Name::Start), State::LeafAfterName, State::Nok)
            }
            (State::LeafAfterName, Some(b'[')) => t.attempt(
                State::Label(Label::Start),
                State::LeafAfterLabel,
                State::LeafAfterLabel,
            ),
            (State::LeafAfterName, _) => t.retry(State::LeafAfterLabel),
            (State::LeafAfterLabel, Some(b'{')) => t.attempt(
                State::Attributes(Attributes::Start),
                State::LeafAfterAttributes,
                State::LeafAfterAttributes,
            ),
            (State::LeafAfterLabel | State::LeafAfterAttributes, Some(b'\t' | b' ')) => {
                t.consume().next(State::LeafAfterAttributes)
            }
            (State::LeafAfterLabel | State::LeafAfterAttributes, None | Some(b'\n')) => {
                t.exit().ok()
            }
            _ => t.nok(),
        }
    }

    fn to_mdast(&self, tokens: Vec<Token<'_, DirectiveToken>>) -> mdast::Node {
        to_mdast(LEAF, tokens)
    }

    fn html(
        &self,
        event: HtmlEvent<'_, DirectiveToken>,
        tokens: &[Token<'_, DirectiveToken>],
        w: &mut HtmlWriter<'_>,
    ) {
        html(&event, tokens, w, T::DirectiveLeaf, "div");
    }
}

/// `:::name[label]{attributes}`, a body parsed as a document, and `:::`.
pub struct DirectiveContainer;

impl Directive for DirectiveContainer {
    const DISALLOW_EOL: bool = true;
}

impl Construct for DirectiveContainer {
    type Place = Flow;
    type State = State;
    type Token = DirectiveToken;

    fn markers(&self) -> &[u8] {
        b":"
    }

    fn start(&self) -> State {
        State::Start
    }

    fn step(&self, state: State, t: Ctx<'_, '_, Self>) -> Step<Self> {
        let t = match factory(state, t) {
            Ok(step) => return step,
            Err(t) => t,
        };

        match (state, t.current()) {
            // Opening fence: `:::`, a name, a label, and attributes.
            (State::Start, Some(b':')) => t
                .enter(T::DirectiveContainer)
                .enter(T::DirectiveContainerFence)
                .enter(T::DirectiveContainerSequence)
                .consume()
                .next(State::OpenSequence(1)),
            (State::OpenSequence(size), Some(b':')) => {
                t.consume().next(State::OpenSequence(size + 1))
            }
            (State::OpenSequence(size), _) if size < 3 => t.nok(),
            (State::OpenSequence(size), _) => t.exit().attempt(
                State::Name(Name::Start),
                State::OpenAfterName(size),
                State::Nok,
            ),
            (State::OpenAfterName(size), Some(b'[')) => t.attempt(
                State::Label(Label::Start),
                State::OpenAfterLabel(size),
                State::OpenAfterLabel(size),
            ),
            (State::OpenAfterName(size), _) => t.retry(State::OpenAfterLabel(size)),
            (State::OpenAfterLabel(size), Some(b'{')) => t.attempt(
                State::Attributes(Attributes::Start),
                State::OpenAfterAttributes(size),
                State::OpenAfterAttributes(size),
            ),
            (
                State::OpenAfterLabel(size) | State::OpenAfterAttributes(size),
                Some(b'\t' | b' '),
            ) => t.consume().next(State::OpenAfterAttributes(size)),
            (
                State::OpenAfterLabel(size) | State::OpenAfterAttributes(size),
                None | Some(b'\n'),
            ) => t.exit().retry(State::OpenEol(size)),
            (State::OpenEol(size), Some(b'\n')) => t.consume().next(State::ContentStart(size)),
            (State::OpenEol(_) | State::ContentStart(_), None)
            | (State::CloseAfter, None | Some(b'\n')) => t.exit().ok(),
            // Body, and closing fence, tried at each line.
            (State::ContentStart(size), _) => t
                .enter(T::DirectiveContainerContent)
                .retry(State::LineStart(size)),
            (State::LineStart(_) | State::Chunk(_), None) => t.retry(State::AfterContent),
            (State::LineStart(size), _) => t.attempt(
                State::Close(size),
                State::AfterContent,
                State::PrefixStart(size),
            ),
            // Up to the indent of the opening fence is not content.
            (State::PrefixStart(size), Some(b'\t' | b' ')) if t.indent() > 0 => t
                .enter(T::LinePrefix)
                .consume()
                .next(State::PrefixInside(size, 1)),
            (State::PrefixStart(size), _) => t.retry(State::Chunk(size)),
            (State::PrefixInside(size, columns), Some(b'\t' | b' ')) if columns < t.indent() => {
                t.consume().next(State::PrefixInside(size, columns + 1))
            }
            (State::PrefixInside(size, _), _) => t.exit().retry(State::Chunk(size)),
            (State::Chunk(size), Some(b'\n')) => t.consume().next(State::LineStart(size)),
            (State::Chunk(size), Some(_)) => t.consume().next(State::Chunk(size)),
            // Closing fence: up to 3 columns, or any without indented code,
            // `:::` or more, and spaces.
            (State::Close(size), Some(b'\t' | b' ')) => t
                .enter(T::DirectiveContainerFence)
                .consume()
                .next(State::CloseIndent(size, 1)),
            (State::Close(size), Some(b':')) => t
                .enter(T::DirectiveContainerFence)
                .retry(State::CloseSequence(size, 0)),
            (State::CloseIndent(size, columns), Some(b'\t' | b' '))
                if columns < 3 || !t.options().constructs.code_indented =>
            {
                t.consume().next(State::CloseIndent(size, columns + 1))
            }
            (State::CloseIndent(size, _), Some(b':')) => t.retry(State::CloseSequence(size, 0)),
            (State::CloseSequence(size, 0), Some(b':')) => t
                .enter(T::DirectiveContainerSequence)
                .consume()
                .next(State::CloseSequence(size, 1)),
            (State::CloseSequence(size, close), Some(b':')) => {
                t.consume().next(State::CloseSequence(size, close + 1))
            }
            (State::CloseSequence(size, close), _) if close >= size => {
                t.exit().retry(State::CloseAfter)
            }
            (State::CloseAfter, Some(b'\t' | b' ')) => t.consume().next(State::CloseAfter),
            (State::AfterContent, _) => t.exit().exit().ok(),
            _ => t.nok(),
        }
    }

    fn to_mdast(&self, tokens: Vec<Token<'_, DirectiveToken>>) -> mdast::Node {
        to_mdast(CONTAINER, tokens)
    }

    fn html(
        &self,
        event: HtmlEvent<'_, DirectiveToken>,
        tokens: &[Token<'_, DirectiveToken>],
        w: &mut HtmlWriter<'_>,
    ) {
        html(&event, tokens, w, T::DirectiveContainer, "div");
    }
}

/// Attributes of a directive.
fn attribute_map(tokens: &[Token<'_, DirectiveToken>]) -> BTreeMap<String, String> {
    let mut attributes: Vec<(String, String)> = vec![];

    for token in tokens {
        match token.name {
            T::DirectiveAttributeIdValue => attributes.push(("id".into(), decode(&token.value))),
            T::DirectiveAttributeClassValue => {
                attributes.push(("class".into(), decode(&token.value)));
            }
            T::DirectiveAttributeName => attributes.push((token.value.to_string(), String::new())),
            T::DirectiveAttributeValue => {
                if let Some(last) = attributes.last_mut() {
                    last.1 = decode(&token.value);
                }
            }
            _ => {}
        }
    }

    // Classes join, other attributes keep the last value.
    let mut cleaned: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in attributes {
        match cleaned.get_mut(&key) {
            Some(existing) if key == "class" => {
                existing.push(' ');
                existing.push_str(&value);
            }
            _ => {
                cleaned.insert(key, value);
            }
        }
    }

    cleaned
}

/// Node of a directive, like `mdast-util-directive`.
fn to_mdast(kind: &str, mut tokens: Vec<Token<'_, DirectiveToken>>) -> mdast::Node {
    let attributes = attribute_map(&tokens);
    let label = tokens
        .token(T::DirectiveLabel)
        .is_some()
        .then(|| tokens.take_children(T::DirectiveLabelString));
    let mut body = tokens.take_children(T::DirectiveContainerContent);
    let mut fields = BTreeMap::new();
    fields.insert(
        "name".into(),
        tokens.value(T::DirectiveName).unwrap_or_default().into(),
    );

    let children = if kind == CONTAINER {
        let mut children = vec![];
        if let Some(label) = label {
            fields.insert("label".into(), "true".into());
            children.push(mdast::Node::Paragraph(mdast::Paragraph {
                children: label,
                position: None,
            }));
        }
        children.append(&mut body);
        children
    } else {
        label.unwrap_or_default()
    };

    mdast::Node::Custom(mdast::Custom {
        name: kind.into(),
        fields,
        attributes,
        children,
        ..mdast::Custom::default()
    })
}

/// HTML of a directive in `to_html`, the same as `to_hast` writes: the
/// element with safe attributes and the name as a class, the label, and the
/// body of a container.
fn html(
    event: &HtmlEvent<'_, DirectiveToken>,
    tokens: &[Token<'_, DirectiveToken>],
    w: &mut HtmlWriter<'_>,
    kind: DirectiveToken,
    tag_name: &'static str,
) {
    match event {
        HtmlEvent::Enter(token) if token.name == kind => {
            let attributes = attribute_map(tokens);
            w.tag("<");
            w.tag(tag_name);
            for (key, value) in &attributes {
                if is_safe_attribute(key) {
                    w.attr(key, value);
                }
            }
            let mut class = String::from(tokens.value(T::DirectiveName).unwrap_or_default());
            if let Some(classes) = attributes.get("class") {
                class.push(' ');
                class.push_str(classes);
            }
            w.attr(
                "class",
                &class.split_whitespace().collect::<Vec<_>>().join(" "),
            );
            w.tag(">");
        }
        HtmlEvent::Exit(token) if token.name == kind => {
            if kind == T::DirectiveContainer {
                w.line_ending_if_needed();
            }
            w.tag("</");
            w.tag(tag_name);
            w.tag(">");
        }
        // The label of a container is its first paragraph.
        HtmlEvent::Enter(token)
            if kind == T::DirectiveContainer && token.name == T::DirectiveLabel =>
        {
            w.line_ending_if_needed();
            w.tag("<p>");
        }
        HtmlEvent::Exit(token)
            if kind == T::DirectiveContainer && token.name == T::DirectiveLabel =>
        {
            w.tag("</p>");
        }
        _ => {}
    }
}

/// Decode character references, like micromark’s `decodeLight`.
fn decode(value: &str) -> String {
    let mut result = String::new();
    let mut rest = value;

    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest[1..].find(';').and_then(|end| {
            let inner = &rest[1..=end];
            let value = if let Some(numeric) = inner.strip_prefix('#') {
                let (digits, radix) = match numeric.strip_prefix(['x', 'X'].as_ref()) {
                    Some(hex) if (1..=6).contains(&hex.len()) => (hex, 16),
                    None if (1..=7).contains(&numeric.len()) => (numeric, 10),
                    _ => return None,
                };
                if !digits.chars().all(|char| char.is_digit(radix)) {
                    return None;
                }
                decode_numeric(digits, radix)
            } else if (1..=31).contains(&inner.len())
                && inner.chars().all(|char| char.is_ascii_alphanumeric())
            {
                decode_named(inner, true)?
            } else {
                return None;
            };
            Some((value, end + 2))
        });

        if let Some((value, size)) = decoded {
            result.push_str(&value);
            rest = &rest[size..];
        } else {
            result.push('&');
            rest = &rest[1..];
        }
    }

    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_character_references() {
        assert_eq!(
            decode("a &amp; &#35; &#x41; &nope; & b"),
            "a & # A &nope; & b"
        );
    }
}
