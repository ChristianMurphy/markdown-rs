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

use alloc::{boxed::Box, collections::BTreeMap, string::String, vec, vec::Vec};
use core::convert::TryFrom;
use markdown::{
    classify_character, decode_named, decode_numeric,
    extension::{Construct, ConstructTokenizer, ContentType, Step, Token},
    mdast, CharacterKind,
};
use markdown_processor::{hast, mdast_util_to_hast::wrap, Plugin, Processor};

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
        processor.add_syntax(DirectiveText);
        // Like micromark: containers before leaves, both at `:`.
        processor
            .parse
            .flow_constructs
            .push(Box::new(DirectiveContainer));
        processor
            .parse
            .flow_constructs
            .push(Box::new(DirectiveLeaf));

        processor.add_hast_handler(TEXT, |node, children| to_hast(node, "span", children));
        processor.add_hast_handler(LEAF, |node, children| to_hast(node, "div", children));
        processor.add_hast_handler(CONTAINER, |node, children| {
            to_hast(node, "div", wrap(children, true))
        });
    }
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
            // Never event handlers or URLs: they come from the author.
            "id" | "title" | "lang" | "dir" => {
                properties.push((
                    key.clone().into(),
                    hast::PropertyValue::String(value.clone()),
                ));
            }
            _ if key.starts_with("data-") => {
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

// States shared by the three constructs, like micromark’s factories: each is
// entered through `Step::Attempt`, and ends in `Ok` or `Nok`.
const NAME: u16 = 100;
const NAME_CHAR: u16 = 101;
const NAME_INSIDE: u16 = 102;
const NAME_AFTER_DASH: u16 = 103;

const LABEL: u16 = 200;
const LABEL_AFTER_START: u16 = 201;
const LABEL_DATA: u16 = 202;
const LABEL_ESCAPE: u16 = 203;
const LABEL_CLOSE: u16 = 204;

const ATTRIBUTES: u16 = 300;
const BETWEEN: u16 = 301;
const SHORTCUT_START: u16 = 302;
const SHORTCUT_START_AFTER: u16 = 303;
const SHORTCUT: u16 = 304;
const ATTRIBUTE_NAME: u16 = 305;
const ATTRIBUTE_NAME_AFTER: u16 = 306;
const VALUE_BEFORE: u16 = 307;
const VALUE_UNQUOTED: u16 = 308;
const VALUE_QUOTED_START: u16 = 309;
const VALUE_QUOTED_BETWEEN: u16 = 310;
const VALUE_QUOTED: u16 = 311;
const VALUE_QUOTED_AFTER: u16 = 312;
const ATTRIBUTES_END: u16 = 313;

const NOK: u16 = 99;

// Memory slots.
const SIZE_OPEN: usize = 0;
const BALANCE: usize = 1;
const SIZE: usize = 2;
const MARKER: usize = 3;
/// Line prefix columns in a container, when the label is done.
const PREFIX: usize = 1;
/// Size of a closing sequence, when the label is done.
const SIZE_CLOSE: usize = 2;

/// Like micromark’s `linkReferenceSizeMax`.
const LABEL_SIZE_MAX: usize = 999;
/// Like micromark’s `linkResourceDestinationBalanceMax`.
const LABEL_BALANCE_MAX: usize = 32;

fn is_space(byte: Option<u8>) -> bool {
    matches!(byte, Some(b'\t' | b' '))
}

fn is_space_or_eol(byte: Option<u8>) -> bool {
    matches!(byte, Some(b'\t' | b'\n' | b' '))
}

fn is_continuation(byte: Option<u8>) -> bool {
    matches!(byte, Some(0x80..=0xBF))
}

/// Whether `char` is neither whitespace nor punctuation.
fn is_other(char: char) -> bool {
    char != '\n' && classify_character(char) == CharacterKind::Other
}

/// Run a shared state, if `state` is one.
fn factory(state: u16, t: &mut ConstructTokenizer, disallow_eol: bool) -> Option<Step> {
    Some(match state {
        NAME..=NAME_AFTER_DASH => name(state, t),
        LABEL..=LABEL_CLOSE => label(state, t, disallow_eol),
        ATTRIBUTES..=ATTRIBUTES_END => attributes(state, t, disallow_eol),
        NOK => Step::Nok,
        _ => return None,
    })
}

/// Name: no whitespace or punctuation other than `-` and `_`, not ending in
/// them.
fn name(state: u16, t: &mut ConstructTokenizer) -> Step {
    match state {
        NAME => match t.current_char() {
            Some(char) if is_other(char) => {
                t.enter("directiveName");
                t.consume();
                Step::Next(NAME_CHAR)
            }
            _ => Step::Nok,
        },
        // Rest of a character.
        NAME_CHAR if is_continuation(t.current()) => {
            t.consume();
            Step::Next(NAME_CHAR)
        }
        NAME_CHAR => Step::Retry(NAME_INSIDE),
        _ => match t.current_char() {
            Some('-' | '_') => {
                t.consume();
                Step::Next(NAME_AFTER_DASH)
            }
            Some(char) if is_other(char) => {
                t.consume();
                Step::Next(NAME_CHAR)
            }
            _ => {
                t.exit("directiveName");
                if state == NAME_AFTER_DASH {
                    Step::Nok
                } else {
                    Step::Ok
                }
            }
        },
    }
}

/// Label: `[`, text with balanced brackets, `]`.
fn label(state: u16, t: &mut ConstructTokenizer, disallow_eol: bool) -> Step {
    match (state, t.current()) {
        (LABEL, Some(b'[')) => {
            t.memory()[BALANCE] = 0;
            t.memory()[SIZE] = 0;
            t.enter("directiveLabel");
            t.enter("directiveLabelMarker");
            t.consume();
            t.exit("directiveLabelMarker");
            Step::Next(LABEL_AFTER_START)
        }
        (LABEL_AFTER_START, Some(b']')) => {
            t.enter("directiveLabelMarker");
            t.consume();
            t.exit("directiveLabelMarker");
            t.exit("directiveLabel");
            Step::Ok
        }
        (LABEL_AFTER_START, _) => {
            t.enter_content("directiveLabelString", ContentType::Text);
            Step::Retry(LABEL_DATA)
        }
        (LABEL_DATA, None) => Step::Nok,
        (LABEL_DATA, _) if t.memory()[SIZE] > LABEL_SIZE_MAX => Step::Nok,
        (LABEL_DATA, Some(b'[')) => {
            t.memory()[BALANCE] += 1;
            if t.memory()[BALANCE] > LABEL_BALANCE_MAX {
                return Step::Nok;
            }
            t.consume();
            Step::Next(LABEL_DATA)
        }
        (LABEL_DATA, Some(b']')) if t.memory()[BALANCE] == 0 => Step::Retry(LABEL_CLOSE),
        (LABEL_DATA, Some(b']')) => {
            t.memory()[BALANCE] -= 1;
            t.consume();
            Step::Next(LABEL_DATA)
        }
        (LABEL_DATA, Some(b'\n')) if disallow_eol => Step::Nok,
        (LABEL_DATA, Some(byte)) => {
            t.consume();
            Step::Next(if byte == b'\\' {
                LABEL_ESCAPE
            } else {
                LABEL_DATA
            })
        }
        (LABEL_ESCAPE, Some(b'[' | b'\\' | b']')) => {
            t.memory()[SIZE] += 1;
            t.consume();
            Step::Next(LABEL_DATA)
        }
        (LABEL_ESCAPE, _) => Step::Retry(LABEL_DATA),
        (LABEL_CLOSE, _) => {
            t.exit("directiveLabelString");
            t.enter("directiveLabelMarker");
            t.consume();
            t.exit("directiveLabelMarker");
            t.exit("directiveLabel");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

/// Attributes: `{`, `#id`, `.class`, `key`, `key=value`, `key="value"`,
/// `}`.
fn attributes(state: u16, t: &mut ConstructTokenizer, disallow_eol: bool) -> Step {
    match state {
        SHORTCUT_START..=SHORTCUT => shortcut(state, t),
        ATTRIBUTE_NAME..=ATTRIBUTE_NAME_AFTER => attribute_name(state, t, disallow_eol),
        VALUE_BEFORE..=VALUE_QUOTED_AFTER => value(state, t, disallow_eol),
        _ => between(state, t, disallow_eol),
    }
}

/// Whether the current byte is whitespace between attributes.
fn is_attribute_whitespace(t: &ConstructTokenizer, disallow_eol: bool) -> bool {
    if disallow_eol {
        is_space(t.current())
    } else {
        is_space_or_eol(t.current())
    }
}

/// The braces, and what is between attributes.
fn between(state: u16, t: &mut ConstructTokenizer, disallow_eol: bool) -> Step {
    match (state, t.current()) {
        (ATTRIBUTES, _) => {
            t.enter("directiveAttributes");
            t.enter("directiveAttributesMarker");
            t.consume();
            t.exit("directiveAttributesMarker");
            Step::Next(BETWEEN)
        }
        (BETWEEN, Some(byte @ (b'#' | b'.'))) => {
            t.memory()[MARKER] = usize::from(byte);
            Step::Retry(SHORTCUT_START)
        }
        (BETWEEN, _) if is_attribute_whitespace(t, disallow_eol) => {
            t.consume();
            Step::Next(BETWEEN)
        }
        (BETWEEN, _) => match t.current_char() {
            Some(char) if is_other(char) || matches!(char, '-' | '_') => {
                t.enter("directiveAttribute");
                t.enter("directiveAttributeName");
                t.consume();
                Step::Next(ATTRIBUTE_NAME)
            }
            _ => Step::Retry(ATTRIBUTES_END),
        },
        (ATTRIBUTES_END, Some(b'}')) => {
            t.enter("directiveAttributesMarker");
            t.consume();
            t.exit("directiveAttributesMarker");
            t.exit("directiveAttributes");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

/// `#id` and `.class`.
fn shortcut(state: u16, t: &mut ConstructTokenizer) -> Step {
    let is_id = t.memory()[MARKER] == usize::from(b'#');
    let (kind, kind_marker, kind_value) = if is_id {
        (
            "directiveAttributeId",
            "directiveAttributeIdMarker",
            "directiveAttributeIdValue",
        )
    } else {
        (
            "directiveAttributeClass",
            "directiveAttributeClassMarker",
            "directiveAttributeClassValue",
        )
    };

    match (state, t.current()) {
        (SHORTCUT_START, _) => {
            t.enter("directiveAttribute");
            t.enter(kind);
            t.enter(kind_marker);
            t.consume();
            t.exit(kind_marker);
            Step::Next(SHORTCUT_START_AFTER)
        }
        (
            SHORTCUT_START_AFTER,
            None
            | Some(
                b'"' | b'#' | b'\'' | b'.' | b'<' | b'=' | b'>' | b'`' | b'}' | b'\t' | b'\n'
                | b' ',
            ),
        )
        | (SHORTCUT, None | Some(b'"' | b'\'' | b'<' | b'=' | b'>' | b'`')) => Step::Nok,
        (SHORTCUT_START_AFTER, _) => {
            t.enter(kind_value);
            t.consume();
            Step::Next(SHORTCUT)
        }
        (SHORTCUT, Some(b'#' | b'.' | b'}' | b'\t' | b'\n' | b' ')) => {
            t.exit(kind_value);
            t.exit(kind);
            t.exit("directiveAttribute");
            Step::Retry(BETWEEN)
        }
        _ => {
            t.consume();
            Step::Next(SHORTCUT)
        }
    }
}

/// `key`, maybe followed by `=`.
fn attribute_name(state: u16, t: &mut ConstructTokenizer, disallow_eol: bool) -> Step {
    let current = t.current();

    match state {
        ATTRIBUTE_NAME if is_continuation(current) => {
            t.consume();
            Step::Next(ATTRIBUTE_NAME)
        }
        ATTRIBUTE_NAME => match t.current_char() {
            Some(char) if is_other(char) || matches!(char, '-' | '.' | ':' | '_') => {
                t.consume();
                Step::Next(ATTRIBUTE_NAME)
            }
            _ => {
                t.exit("directiveAttributeName");
                Step::Retry(ATTRIBUTE_NAME_AFTER)
            }
        },
        ATTRIBUTE_NAME_AFTER if is_attribute_whitespace(t, disallow_eol) => {
            t.consume();
            Step::Next(ATTRIBUTE_NAME_AFTER)
        }
        ATTRIBUTE_NAME_AFTER if current == Some(b'=') => {
            t.enter("directiveAttributeInitializerMarker");
            t.consume();
            t.exit("directiveAttributeInitializerMarker");
            Step::Next(VALUE_BEFORE)
        }
        _ => {
            t.exit("directiveAttribute");
            Step::Retry(BETWEEN)
        }
    }
}

/// `value`, `"value"`, or `'value'`, after `=`.
fn value(state: u16, t: &mut ConstructTokenizer, disallow_eol: bool) -> Step {
    let current = t.current();
    let is_whitespace = is_attribute_whitespace(t, disallow_eol);
    let marker = u8::try_from(t.memory()[MARKER]).ok();
    let is_marker = current.is_some() && current == marker;

    match state {
        VALUE_BEFORE => match current {
            None | Some(b'<' | b'=' | b'>' | b'`' | b'}') => Step::Nok,
            Some(b'\n') if disallow_eol => Step::Nok,
            Some(byte @ (b'"' | b'\'')) => {
                t.memory()[MARKER] = usize::from(byte);
                t.enter("directiveAttributeValueLiteral");
                t.enter("directiveAttributeValueMarker");
                t.consume();
                t.exit("directiveAttributeValueMarker");
                Step::Next(VALUE_QUOTED_START)
            }
            _ if is_whitespace => {
                t.consume();
                Step::Next(VALUE_BEFORE)
            }
            _ => {
                t.enter("directiveAttributeValue");
                t.enter("directiveAttributeValueData");
                t.consume();
                Step::Next(VALUE_UNQUOTED)
            }
        },
        VALUE_UNQUOTED => match current {
            None | Some(b'"' | b'\'' | b'<' | b'=' | b'>' | b'`') => Step::Nok,
            Some(b'}' | b'\t' | b'\n' | b' ') => {
                t.exit("directiveAttributeValueData");
                t.exit("directiveAttributeValue");
                t.exit("directiveAttribute");
                Step::Retry(BETWEEN)
            }
            _ => {
                t.consume();
                Step::Next(VALUE_UNQUOTED)
            }
        },
        VALUE_QUOTED_START if is_marker => {
            t.enter("directiveAttributeValueMarker");
            t.consume();
            t.exit("directiveAttributeValueMarker");
            t.exit("directiveAttributeValueLiteral");
            t.exit("directiveAttribute");
            Step::Next(VALUE_QUOTED_AFTER)
        }
        VALUE_QUOTED_START => {
            t.enter("directiveAttributeValue");
            Step::Retry(VALUE_QUOTED_BETWEEN)
        }
        VALUE_QUOTED_BETWEEN if is_marker => {
            t.exit("directiveAttributeValue");
            Step::Retry(VALUE_QUOTED_START)
        }
        VALUE_QUOTED_BETWEEN => match current {
            None => Step::Nok,
            Some(b'\n') if disallow_eol => Step::Nok,
            Some(b'\n') => {
                t.consume();
                Step::Next(VALUE_QUOTED_BETWEEN)
            }
            _ => {
                t.enter("directiveAttributeValueData");
                t.consume();
                Step::Next(VALUE_QUOTED)
            }
        },
        VALUE_QUOTED if is_marker || matches!(current, None | Some(b'\n')) => {
            t.exit("directiveAttributeValueData");
            Step::Retry(VALUE_QUOTED_BETWEEN)
        }
        VALUE_QUOTED => {
            t.consume();
            Step::Next(VALUE_QUOTED)
        }
        // After a quoted value.
        _ => match current {
            Some(b'}' | b'\t' | b'\n' | b' ') => Step::Retry(BETWEEN),
            _ => Step::Retry(ATTRIBUTES_END),
        },
    }
}

/// `:name[label]{attributes}`, in text.
struct DirectiveText;

impl Construct for DirectiveText {
    fn markers(&self) -> &[u8] {
        b":"
    }

    fn previous(&self, previous: Option<u8>) -> bool {
        previous != Some(b':')
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        if let Some(step) = factory(state, t, false) {
            return step;
        }

        match (state, t.current()) {
            (0, Some(b':')) => {
                t.enter("directiveText");
                t.enter("directiveMarker");
                t.consume();
                t.exit("directiveMarker");
                Step::Next(1)
            }
            (1, _) => Step::Attempt {
                state: NAME,
                ok: 2,
                nok: NOK,
            },
            (2, Some(b':')) => Step::Nok,
            (2, Some(b'[')) => Step::Attempt {
                state: LABEL,
                ok: 3,
                nok: 3,
            },
            (2, _) => Step::Retry(3),
            (3, Some(b'{')) => Step::Attempt {
                state: ATTRIBUTES,
                ok: 4,
                nok: 4,
            },
            (3 | 4, _) => {
                t.exit("directiveText");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        to_mdast(TEXT, tokens)
    }
}

/// `::name[label]{attributes}`, on its own line.
struct DirectiveLeaf;

impl Construct for DirectiveLeaf {
    fn markers(&self) -> &[u8] {
        b":"
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        if let Some(step) = factory(state, t, true) {
            return step;
        }

        match (state, t.current()) {
            (0, Some(b':')) => {
                t.enter("directiveLeaf");
                t.enter("directiveSequence");
                t.consume();
                Step::Next(1)
            }
            (1, Some(b':')) => {
                t.consume();
                t.exit("directiveSequence");
                Step::Next(2)
            }
            (2, _) => Step::Attempt {
                state: NAME,
                ok: 3,
                nok: NOK,
            },
            (3, Some(b'[')) => Step::Attempt {
                state: LABEL,
                ok: 4,
                nok: 4,
            },
            (3, _) => Step::Retry(4),
            (4, Some(b'{')) => Step::Attempt {
                state: ATTRIBUTES,
                ok: 5,
                nok: 5,
            },
            (4 | 5, Some(b'\t' | b' ')) => {
                t.consume();
                Step::Next(5)
            }
            (4 | 5, None | Some(b'\n')) => {
                t.exit("directiveLeaf");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        to_mdast(LEAF, tokens)
    }
}

// Container states.
const CONTENT_START: u16 = 10;
const LINE_START: u16 = 11;
const PREFIX_START: u16 = 12;
const PREFIX_INSIDE: u16 = 13;
const CHUNK: u16 = 14;
const CLOSE: u16 = 20;
const CLOSE_INDENT: u16 = 21;
const CLOSE_SEQUENCE: u16 = 22;
const CLOSE_AFTER: u16 = 23;
const AFTER_CONTENT: u16 = 30;

/// `:::name[label]{attributes}`, a body parsed as a document, and `:::`.
struct DirectiveContainer;

impl Construct for DirectiveContainer {
    fn markers(&self) -> &[u8] {
        b":"
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        if let Some(step) = factory(state, t, true) {
            return step;
        }

        match state {
            CONTENT_START..=AFTER_CONTENT => container_content(state, t),
            _ => container_open(state, t),
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        to_mdast(CONTAINER, tokens)
    }
}

/// Opening fence of a container: `:::`, a name, a label, and attributes.
fn container_open(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0, Some(b':')) => {
            t.enter("directiveContainer");
            t.enter("directiveContainerFence");
            t.enter("directiveContainerSequence");
            t.consume();
            t.memory()[SIZE_OPEN] = 1;
            Step::Next(1)
        }
        (1, Some(b':')) => {
            t.consume();
            t.memory()[SIZE_OPEN] += 1;
            Step::Next(1)
        }
        (1, _) if t.memory()[SIZE_OPEN] < 3 => Step::Nok,
        (1, _) => {
            t.exit("directiveContainerSequence");
            Step::Attempt {
                state: NAME,
                ok: 2,
                nok: NOK,
            }
        }
        (2, Some(b'[')) => Step::Attempt {
            state: LABEL,
            ok: 3,
            nok: 3,
        },
        (2, _) => Step::Retry(3),
        (3, Some(b'{')) => Step::Attempt {
            state: ATTRIBUTES,
            ok: 4,
            nok: 4,
        },
        (3 | 4, Some(b'\t' | b' ')) => {
            t.consume();
            Step::Next(4)
        }
        (3 | 4, None | Some(b'\n')) => {
            t.exit("directiveContainerFence");
            Step::Retry(5)
        }
        (5, Some(b'\n')) => {
            t.consume();
            Step::Next(CONTENT_START)
        }
        (5, None) => {
            t.exit("directiveContainer");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

/// Body of a container, and its closing fence, tried at each line.
fn container_content(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (CONTENT_START, None) => {
            t.exit("directiveContainer");
            Step::Ok
        }
        (CONTENT_START, _) => {
            t.enter_content("directiveContainerContent", ContentType::Document);
            Step::Retry(LINE_START)
        }
        (LINE_START | CHUNK, None) => Step::Retry(AFTER_CONTENT),
        (LINE_START, _) => Step::Attempt {
            state: CLOSE,
            ok: AFTER_CONTENT,
            nok: PREFIX_START,
        },
        // Up to the indent of the opening fence is not content.
        (PREFIX_START, Some(b'\t' | b' ')) if t.indent() > 0 => {
            t.memory()[PREFIX] = 1;
            t.enter("linePrefix");
            t.consume();
            Step::Next(PREFIX_INSIDE)
        }
        (PREFIX_START, _) => Step::Retry(CHUNK),
        (PREFIX_INSIDE, Some(b'\t' | b' ')) if t.memory()[PREFIX] < t.indent() => {
            t.memory()[PREFIX] += 1;
            t.consume();
            Step::Next(PREFIX_INSIDE)
        }
        (PREFIX_INSIDE, _) => {
            t.exit("linePrefix");
            Step::Retry(CHUNK)
        }
        (CHUNK, Some(b'\n')) => {
            t.consume();
            Step::Next(LINE_START)
        }
        (CHUNK, Some(_)) => {
            t.consume();
            Step::Next(CHUNK)
        }
        // Closing fence, tried at each line: indent, `:::` or more, spaces.
        (CLOSE, Some(b'\t' | b' ')) => {
            t.memory()[PREFIX] = 1;
            t.enter("directiveContainerFence");
            t.consume();
            Step::Next(CLOSE_INDENT)
        }
        (CLOSE, Some(b':')) => {
            t.enter("directiveContainerFence");
            Step::Retry(CLOSE_SEQUENCE)
        }
        // Up to 3 columns, or any, without indented code.
        (CLOSE_INDENT, Some(b'\t' | b' '))
            if t.memory()[PREFIX] < 3 || !t.options().constructs.code_indented =>
        {
            t.memory()[PREFIX] += 1;
            t.consume();
            Step::Next(CLOSE_INDENT)
        }
        (CLOSE_INDENT, Some(b':')) => Step::Retry(CLOSE_SEQUENCE),
        (CLOSE_SEQUENCE, Some(b':')) => {
            if t.memory()[SIZE_CLOSE] == 0 {
                t.enter("directiveContainerSequence");
            }
            t.memory()[SIZE_CLOSE] += 1;
            t.consume();
            Step::Next(CLOSE_SEQUENCE)
        }
        (CLOSE_SEQUENCE, _) => {
            let is_long_enough = t.memory()[SIZE_CLOSE] >= t.memory()[SIZE_OPEN];
            t.memory()[SIZE_CLOSE] = 0;
            if !is_long_enough {
                return Step::Nok;
            }
            t.exit("directiveContainerSequence");
            Step::Retry(CLOSE_AFTER)
        }
        (CLOSE_AFTER, Some(b'\t' | b' ')) => {
            t.consume();
            Step::Next(CLOSE_AFTER)
        }
        (CLOSE_AFTER, None | Some(b'\n')) => {
            t.exit("directiveContainerFence");
            Step::Ok
        }
        (AFTER_CONTENT, _) => {
            t.exit("directiveContainerContent");
            t.exit("directiveContainer");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

/// Node of a directive, like `mdast-util-directive`.
fn to_mdast(kind: &str, tokens: Vec<Token>) -> mdast::Node {
    let mut fields = BTreeMap::new();
    let mut attributes: Vec<(String, String)> = vec![];
    let mut label = None;
    let mut body = vec![];

    for token in tokens {
        match token.name {
            "directiveName" => {
                fields.insert("name".into(), token.value.into_owned());
            }
            "directiveLabel" => label = Some(vec![]),
            "directiveLabelString" => label = Some(token.children),
            "directiveAttributeIdValue" => attributes.push(("id".into(), decode(&token.value))),
            "directiveAttributeClassValue" => {
                attributes.push(("class".into(), decode(&token.value)));
            }
            "directiveAttributeName" => attributes.push((token.value.into_owned(), String::new())),
            "directiveAttributeValue" => {
                if let Some(last) = attributes.last_mut() {
                    last.1 = decode(&token.value);
                }
            }
            "directiveContainerContent" => body = token.children,
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
        attributes: cleaned,
        children,
        ..mdast::Custom::default()
    })
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
