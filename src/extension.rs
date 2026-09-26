//! Syntax extensions: constructs that plugins add to markdown.
//!
//! A construct is a state machine driven by the built-in tokenizer, so it
//! sees text without container prefixes such as `> `, across lines.

use crate::event::{Event, Kind, Name};
use crate::mdast;
use crate::state::{Name as StateName, State};
use crate::tokenizer::{move_point_back, Tokenizer};
use crate::unist::Position;
use crate::util::slice::{Position as SlicePosition, Slice};
use alloc::{borrow::Cow, boxed::Box, string::String, vec, vec::Vec};
use core::{convert::TryFrom, mem, str};

/// Most steps a construct can take in a row without consuming a byte.
const RETRY_MAX: u16 = 256;

/// A construct in text (phrasing content), such as a wiki link.
///
/// Constructs are tried in order, before built-ins, at their markers (never
/// line endings).
///
/// A construct that breaks one of these rules does not match, so its bytes
/// stay text:
///
/// * every consumed byte is inside a token, and one token holds the others
/// * `exit` closes the innermost open token
/// * `Next` comes after a `consume`, `Retry` does not
/// * after consuming a line ending, a step can only `exit` tokens that end
///   there, and returns `Next`
/// * `Ok` comes after at least one byte, with every token closed
/// * tokens start and end between characters, not inside one
///
/// A failed construct is tried again at its next marker: bound lookahead.
pub trait TextConstruct {
    /// Bytes this construct can start at.
    fn markers(&self) -> &[u8];

    /// Take one step at state `state` (the first state is `0`).
    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step;

    /// Turn the tokens of one match into a node.
    ///
    /// `tokens[0]` is the outermost token, the first one entered.
    fn to_mdast(&self, tokens: &[Token]) -> mdast::Node;
}

/// What to do after a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// Go to this state at the next byte (after `consume`).
    Next(u16),
    /// Go to this state at the current byte.
    Retry(u16),
    /// The construct matched.
    Ok,
    /// The construct did not match: its events are discarded.
    Nok,
}

/// Token of a construct: a name, the text it spans, and where.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token<'a> {
    /// Name given to `enter`.
    pub name: &'static str,
    /// Source text, without container prefixes.
    pub value: Cow<'a, str>,
    /// Positional info.
    pub position: Position,
}

/// The tokenizer, as a construct sees it.
pub struct ConstructTokenizer<'t, 'a> {
    tokenizer: &'t mut Tokenizer<'a>,
    index: u8,
    consumed: bool,
    /// Whether a line ending was consumed in this step.
    line_ending: bool,
    broken: bool,
}

impl ConstructTokenizer<'_, '_> {
    /// Current byte, or `None` at the end.
    ///
    /// Line endings (CR, LF, CR+LF) are all `b'\n'`.
    /// After `consume`, this is `None` until the next step brings the next
    /// byte.
    pub fn current(&self) -> Option<u8> {
        self.tokenizer.current
    }

    /// Consume the current byte, which must be inside a token.
    ///
    /// Does nothing at the end.
    pub fn consume(&mut self) {
        if self.tokenizer.current.is_none() {
            return;
        }

        // The next line’s prefix is skipped when the next step starts.
        if self.line_ending {
            self.broken = true;
            return;
        }

        self.reopen();

        if self.open_top().is_none() {
            self.broken = true;
            return;
        }

        if self.tokenizer.current == Some(b'\n') {
            // Split open tokens around it; empty ones restart after it.
            let mut reopen = vec![];
            while let Some(top) = self.open_top() {
                let (id, continues) = match top {
                    Name::Extension(_, id) => (id, false),
                    Name::ExtensionContinuation(_, id) => (id, true),
                    _ => unreachable!("expected a construct token"),
                };
                if self.top_is_empty() {
                    self.tokenizer.events.pop();
                    self.tokenizer.stack.pop();
                    reopen.push((id, continues));
                } else {
                    self.tokenizer.exit(top);
                    reopen.push((id, true));
                }
            }
            reopen.reverse();
            self.tokenizer.enter(Name::ExtensionLineEnding(self.index));
            self.tokenizer.consume();
            self.tokenizer.exit(Name::ExtensionLineEnding(self.index));
            self.tokenizer.tokenize_state.extension_reopen = reopen;
            self.line_ending = true;
        } else {
            self.tokenizer.consume();
        }

        self.consumed = true;
    }

    /// Start a token.
    pub fn enter(&mut self, name: &'static str) {
        if self.line_ending {
            self.broken = true;
            return;
        }

        self.reopen();

        let is_first =
            self.tokenizer.events.len() == self.tokenizer.tokenize_state.extension_events;
        let is_nested = self.open_top().is_some();

        match self.intern(name) {
            Some(id) if (is_first || is_nested) && self.at_boundary() => {
                self.tokenizer.enter(Name::Extension(self.index, id));
            }
            _ => self.broken = true,
        }
    }

    /// End the innermost open token, which must be called `name`.
    pub fn exit(&mut self, name: &'static str) {
        let id = self.intern(name);

        // Ended at the previous line ending; a token dropped there is empty.
        let reopen = &mut self.tokenizer.tokenize_state.extension_reopen;
        if let Some((open, continues)) = reopen.last() {
            if Some(*open) == id {
                if *continues {
                    reopen.pop();
                } else {
                    self.broken = true;
                }
                return;
            }
        }

        if self.line_ending {
            self.broken = true;
            return;
        }

        self.reopen();

        match self.open_top() {
            Some(top)
                if construct_token(&top).map(|(_, open)| open) == id
                    && self.at_boundary()
                    && !self.top_is_empty() =>
            {
                self.tokenizer.exit(top);
            }
            _ => self.broken = true,
        }
    }

    /// Reopen tokens closed at the previous line ending.
    fn reopen(&mut self) {
        for (id, continues) in mem::take(&mut self.tokenizer.tokenize_state.extension_reopen) {
            self.tokenizer.enter(if continues {
                Name::ExtensionContinuation(self.index, id)
            } else {
                Name::Extension(self.index, id)
            });
        }
    }

    /// Number of a token name in this parse, if it fits in an event.
    fn intern(&self, name: &'static str) -> Option<u16> {
        let mut names = self.tokenizer.parse_state.extension_names.borrow_mut();
        let index = names
            .iter()
            .position(|known| *known == name)
            .unwrap_or_else(|| {
                names.push(name);
                names.len() - 1
            });
        u16::try_from(index).ok()
    }

    /// Innermost open token of this construct.
    fn open_top(&self) -> Option<Name> {
        self.tokenizer
            .stack
            .last()
            .filter(|name| construct_token(name).map(|(index, _)| index) == Some(self.index))
            .cloned()
    }

    /// Whether the innermost open token has no bytes yet.
    fn top_is_empty(&self) -> bool {
        // Like enter points: before the CR of a CR+LF.
        let mut point = self.tokenizer.point.clone();
        move_point_back(self.tokenizer, &mut point);
        self.tokenizer.events.last().map_or(false, |event| {
            event.kind == Kind::Enter
                && event.point.index == point.index
                && event.point.vs == point.vs
        })
    }

    /// Whether the tokenizer is between characters.
    fn at_boundary(&self) -> bool {
        at_boundary(self.tokenizer)
    }
}

/// Whether the tokenizer is between characters (not in a UTF-8 sequence or
/// a tab).
fn at_boundary(tokenizer: &Tokenizer) -> bool {
    let point = &tokenizer.point;
    point.vs == 0
        && tokenizer
            .parse_state
            .bytes
            .get(point.index)
            .map_or(true, |byte| !(0x80..0xC0).contains(byte))
}

/// Start trying construct `index`.
pub(crate) fn start(tokenizer: &mut Tokenizer, index: u8) -> State {
    tokenizer.tokenize_state.extension_start = tokenizer.point.index;
    tokenizer.tokenize_state.extension_events = tokenizer.events.len();
    tokenizer.tokenize_state.extension_retries = 0;
    tokenizer.tokenize_state.extension_reopen.clear();
    tokenizer.tokenize_state.extension_index = index;
    tokenizer.tokenize_state.extension_state = 0;
    State::Retry(StateName::TextConstruct)
}

/// Run the current construct, enforcing the rules of [`TextConstruct`].
pub(crate) fn step(tokenizer: &mut Tokenizer) -> State {
    let index = tokenizer.tokenize_state.extension_index;
    let state = tokenizer.tokenize_state.extension_state;
    let parse_state = tokenizer.parse_state;
    let construct = &parse_state.options.text_constructs[usize::from(index)];
    let mut construct_tokenizer = ConstructTokenizer {
        tokenizer,
        index,
        consumed: false,
        line_ending: false,
        broken: false,
    };
    let step = construct.step(state, &mut construct_tokenizer);
    let ConstructTokenizer {
        tokenizer,
        consumed,
        line_ending,
        broken,
        ..
    } = construct_tokenizer;
    let tokenize_state = &tokenizer.tokenize_state;

    let step = match step {
        _ if broken => Step::Nok,
        Step::Next(_) if !consumed => Step::Nok,
        Step::Retry(_) | Step::Ok if line_ending => Step::Nok,
        Step::Retry(_) if consumed || tokenize_state.extension_retries >= RETRY_MAX => Step::Nok,
        // A match ends with a token exit, so `collect_tokens` stops at its end.
        Step::Ok
            if tokenizer.events.last().map_or(true, |event| {
                event.kind != Kind::Exit
                    || construct_token(&event.name).map(|(open, _)| open) != Some(index)
            }) || !tokenize_state.extension_reopen.is_empty()
                || tokenizer.point.index <= tokenize_state.extension_start
                || !at_boundary(tokenizer)
                || tokenizer
                    .stack
                    .last()
                    .and_then(construct_token)
                    .map(|(open, _)| open)
                    == Some(index) =>
        {
            Step::Nok
        }
        step => step,
    };

    match step {
        Step::Next(state) => {
            tokenizer.tokenize_state.extension_retries = 0;
            tokenizer.tokenize_state.extension_state = state;
            State::Next(StateName::TextConstruct)
        }
        Step::Retry(state) => {
            tokenizer.tokenize_state.extension_retries += 1;
            tokenizer.tokenize_state.extension_state = state;
            State::Retry(StateName::TextConstruct)
        }
        Step::Ok => State::Ok,
        Step::Nok => {
            tokenizer.tokenize_state.extension_reopen.clear();
            State::Nok
        }
    }
}

/// Construct index and name of a construct token or continuation.
fn construct_token(name: &Name) -> Option<(u8, u16)> {
    match name {
        Name::Extension(index, id) | Name::ExtensionContinuation(index, id) => Some((*index, *id)),
        _ => None,
    }
}

/// Whether `name` is an event of a construct.
pub(crate) fn is_extension(name: &Name) -> bool {
    matches!(
        name,
        Name::Extension(..) | Name::ExtensionContinuation(..) | Name::ExtensionLineEnding(_)
    )
}

/// Token of a match, as event indices.
struct Span {
    name: &'static str,
    /// Enter and exit of each fragment.
    fragments: Vec<(usize, usize)>,
    /// Enter and exit of the line ending between each fragment.
    line_endings: Vec<(usize, usize)>,
}

/// Gather the tokens of the match that starts at `start`.
///
/// Returns them, joined across line endings, with the index of the last
/// event of the match.
pub(crate) fn collect_tokens<'a>(
    events: &[Event],
    bytes: &'a [u8],
    names: &[&'static str],
    start: usize,
) -> (Vec<Token<'a>>, usize) {
    let mut spans: Vec<Span> = vec![];
    let mut open = vec![];
    let mut closed = vec![];
    let mut reopen: Vec<usize> = vec![];
    let mut line_ending = (0, 0);
    let mut index = start;

    loop {
        let event = &events[index];
        match (&event.kind, &event.name) {
            (Kind::Enter, Name::Extension(_, id)) => {
                spans.push(Span {
                    name: names[usize::from(*id)],
                    fragments: vec![(index, index)],
                    line_endings: vec![],
                });
                open.push(spans.len() - 1);
                closed.clear();
            }
            (Kind::Enter, Name::ExtensionContinuation(_, id)) => {
                let name = names[usize::from(*id)];
                let position = reopen
                    .iter()
                    .position(|span| spans[*span].name == name)
                    .expect("expected a token to continue");
                let span = reopen.remove(position);
                spans[span].fragments.push((index, index));
                spans[span].line_endings.push(line_ending);
                open.push(span);
                closed.clear();
            }
            (Kind::Exit, name) if construct_token(name).is_some() => {
                let span = open.pop().expect("expected an open token");
                spans[span]
                    .fragments
                    .last_mut()
                    .expect("expected a fragment")
                    .1 = index;
                closed.push(span);

                if open.is_empty()
                    && !matches!(
                        events.get(index + 1),
                        Some(Event {
                            name: Name::ExtensionLineEnding(_),
                            ..
                        })
                    )
                {
                    break;
                }
            }
            (Kind::Exit, Name::ExtensionLineEnding(_)) => {
                line_ending = (index - 1, index);
                reopen = closed.drain(..).rev().collect();
            }
            _ => {}
        }
        index += 1;
    }

    // Like `to_html`, leaves out tabs partly used by prefixes.
    let text = |(enter, exit): (usize, usize)| {
        let slice = Slice::from_position(
            bytes,
            &SlicePosition {
                start: &events[enter].point,
                end: &events[exit].point,
            },
        );
        str::from_utf8(slice.bytes).unwrap_or("")
    };
    let tokens = spans
        .into_iter()
        .map(
            |Span {
                 name,
                 fragments,
                 line_endings,
             }| {
                let value = if fragments.len() == 1 {
                    Cow::Borrowed(text(fragments[0]))
                } else {
                    let mut value = String::new();
                    for (index, fragment) in fragments.iter().enumerate() {
                        if index > 0 {
                            value.push_str(text(line_endings[index - 1]));
                        }
                        value.push_str(text(*fragment));
                    }
                    Cow::Owned(value)
                };
                Token {
                    name,
                    value,
                    position: Position {
                        start: events[fragments[0].0].point.to_unist(),
                        end: events[fragments[fragments.len() - 1].1].point.to_unist(),
                    },
                }
            },
        )
        .collect();

    (tokens, index)
}

/// Text markers: the built-in ones and those of registered constructs,
/// except line endings; empty without constructs.
pub(crate) fn text_markers(builtin: &[u8], constructs: &[Box<dyn TextConstruct>]) -> Vec<u8> {
    if constructs.is_empty() {
        return Vec::new();
    }

    let mut markers = builtin.to_vec();
    for construct in constructs {
        for byte in construct.markers() {
            if !matches!(byte, b'\n' | b'\r') && !markers.contains(byte) {
                markers.push(*byte);
            }
        }
    }
    markers
}
