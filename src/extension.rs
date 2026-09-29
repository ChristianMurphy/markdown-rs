//! Syntax extensions: constructs to add to markdown.
//!
//! A construct is a state machine driven by the built-in tokenizer, like a
//! micromark construct, so it sees text without container prefixes such as
//! `> `, across lines.
//! Pass constructs in [`ParseOptions::text_constructs`][crate::ParseOptions].
//!
//! [`to_mdast()`][crate::to_mdast()] turns each match into a node with
//! [`Construct::to_mdast`].
//! HTML has no node to render, so [`to_html()`][crate::to_html()] writes
//! the source of a match as text.
//!
//! ## Examples
//!
//! ```
//! use markdown::{
//!     extension::{Construct, ConstructTokenizer, Step, Token},
//!     mdast::{Custom, Node},
//!     to_mdast, ParseOptions,
//! };
//!
//! /// `@` and a lowercase ASCII letter, such as `@a`.
//! struct Mention;
//!
//! impl Construct for Mention {
//!     fn markers(&self) -> &[u8] {
//!         b"@"
//!     }
//!
//!     fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
//!         match (state, tokenizer.current()) {
//!             (0, Some(b'@')) => {
//!                 tokenizer.enter("mention");
//!                 tokenizer.consume();
//!                 Step::Next(1)
//!             }
//!             (1, Some(b'a'..=b'z')) => {
//!                 tokenizer.enter("mentionName");
//!                 tokenizer.consume();
//!                 tokenizer.exit("mentionName");
//!                 tokenizer.exit("mention");
//!                 Step::Ok
//!             }
//!             _ => Step::Nok,
//!         }
//!     }
//!
//!     fn to_mdast(&self, tokens: Vec<Token>) -> Node {
//!         Node::Custom(Custom {
//!             name: "mention".into(),
//!             value: Some(tokens[1].value.to_string()),
//!             ..Custom::default()
//!         })
//!     }
//! }
//!
//! let options = ParseOptions {
//!     text_constructs: vec![Box::new(Mention)],
//!     ..ParseOptions::default()
//! };
//!
//! // `ToString` prefers the value of a custom node.
//! assert_eq!(to_mdast("hi @a, @1", &options)?.to_string(), "hi a, @1");
//! # Ok::<(), markdown::message::Message>(())
//! ```

use crate::event::{Event, Kind, Name};
use crate::mdast;
use crate::state::{Name as StateName, State};
use crate::tokenizer::{move_point_back, Tokenizer};
use crate::unist::Position;
use crate::util::slice::{Position as SlicePosition, Slice};
use crate::ParseOptions;
use alloc::{borrow::Cow, boxed::Box, string::String, vec, vec::Vec};
use core::{convert::TryFrom, str};

/// Most steps a construct can take in a row without consuming a byte.
const RETRY_MAX: u16 = 256;

/// A construct, such as a mention.
///
/// Constructs in [`ParseOptions::text_constructs`][crate::ParseOptions] run
/// in text.
/// They are tried in order, before built-ins, at their markers (never line
/// endings).
///
/// A construct that breaks one of these rules does not match, so its bytes
/// stay what they would otherwise be:
///
/// * every consumed byte is inside a token, and one token holds the others
/// * a token holds at least one byte
/// * `exit` closes the innermost open token
/// * `Next` comes after a `consume`, `Retry` does not
/// * after consuming a line ending, a step returns `Next`, and can first
///   `exit` tokens that end there
/// * `Ok` comes after at least one byte, with every token closed
/// * tokens start and end between characters, not inside one
///
/// A failed construct is tried again at its next marker, so keep lookahead
/// bounded, or parsing becomes quadratic.
pub trait Construct {
    /// Bytes this construct can start at.
    fn markers(&self) -> &[u8];

    /// Whether the construct can start after `previous`, the byte before it:
    /// `b'\n'` at the start of a line, after container prefixes, and `None`
    /// at the start or after a character escape.
    fn previous(&self, previous: Option<u8>) -> bool {
        let _ = previous;
        true
    }

    /// Take one step at state `state` (the first state is `0`).
    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step;

    /// Turn the tokens of one match into a node.
    ///
    /// `tokens[0]` is the outermost token, the first one entered.
    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node;
}

/// What to do after a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct Token<'a> {
    /// Name given to `enter`.
    pub name: &'static str,
    /// Source text, without container prefixes.
    pub value: Cow<'a, str>,
    /// Positional info.
    pub position: Position,
}

/// Index of a construct and name of a token, interned per parse: events of
/// constructs refer to it by index.
pub(crate) type TokenName = (u8, &'static str);

/// State of constructs in a tokenizer, boxed, so the tokenizer stays small
/// without them.
#[derive(Debug, Default)]
pub(crate) struct ExtensionState {
    /// Where the current construct started.
    start: usize,
    /// Number of events when the current construct started.
    events: usize,
    /// Steps the current construct took without consuming.
    retries: u16,
    /// Construct being tried.
    index: u8,
    /// State of the construct being tried.
    state: u16,
    /// Construct to try next at the current byte.
    pub(crate) next: u8,
    /// Memory of the current match.
    memory: [usize; 4],
    /// Interned names of the open tokens of the current match.
    open: Vec<u16>,
}

/// State of constructs in `tokenizer`, to read, once one started.
pub(crate) fn ext<'t>(tokenizer: &'t Tokenizer) -> &'t ExtensionState {
    tokenizer
        .tokenize_state
        .extension
        .as_deref()
        .expect("expected state of constructs")
}

/// State of constructs in `tokenizer`, created if needed.
pub(crate) fn ext_mut<'t>(tokenizer: &'t mut Tokenizer) -> &'t mut ExtensionState {
    tokenizer
        .tokenize_state
        .extension
        .get_or_insert_with(Box::default)
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

    /// Current character, or `None` at the end or inside a character.
    ///
    /// Line endings are `'\n'`, as with [`current`][Self::current].
    /// `consume` takes one byte: a character takes as many steps as it has
    /// bytes.
    pub fn current_char(&self) -> Option<char> {
        match self.current() {
            Some(b'\n') => Some('\n'),
            Some(_) if self.tokenizer.point.vs > 0 => Some(' '),
            Some(0x80..=0xBF) | None => None,
            Some(_) => {
                let bytes = self.tokenizer.parse_state.bytes;
                crate::util::char::after_index(bytes, self.tokenizer.point.index)
            }
        }
    }

    /// Parse options, such as which built-in constructs are on, like
    /// micromark’s `parser.constructs`.
    pub fn options(&self) -> &ParseOptions {
        self.tokenizer.parse_state.options
    }

    /// Memory of the current match, all zero at its start, such as the size
    /// of an opening fence: micromark keeps these in closures.
    pub fn memory(&mut self) -> &mut [usize; 4] {
        &mut ext_mut(self.tokenizer).memory
    }

    /// Consume the current byte, which must be inside a token.
    ///
    /// Does nothing at the end.
    pub fn consume(&mut self) {
        if self.current().is_none() {
            return;
        }

        if ext(self.tokenizer).open.is_empty() {
            self.broken = true;
            return;
        }

        let is_eol = self.tokenizer.current == Some(b'\n');

        if is_eol {
            self.tokenizer.enter(Name::LineEnding);
            self.tokenizer.consume();
            self.tokenizer.exit(Name::LineEnding);
        } else {
            self.tokenizer.consume();
        }

        self.line_ending = is_eol;
        self.consumed = true;
    }

    /// Start a token.
    pub fn enter(&mut self, name: &'static str) {
        let state = ext(self.tokenizer);
        // The first token starts the match, and holds the others.
        let can_start = self.tokenizer.events.len() == state.events || !state.open.is_empty();

        if self.line_ending || !can_start || !at_boundary(self.tokenizer) {
            self.broken = true;
            return;
        }

        match intern(self.tokenizer, self.index, name) {
            Some(id) => enter_token(self.tokenizer, id),
            None => self.broken = true,
        }
    }

    /// End the innermost open token, which must be called `name`.
    pub fn exit(&mut self, name: &'static str) {
        let names = self.tokenizer.parse_state.extension_names.borrow();
        let is_named = ext(self.tokenizer)
            .open
            .last()
            .map_or(false, |id| names[usize::from(*id)].1 == name);
        drop(names);

        if !is_named || !at_boundary(self.tokenizer) || self.top_is_empty() {
            self.broken = true;
            return;
        }

        exit_token(self.tokenizer);
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
}

/// Whether the tokenizer is between characters (not in a UTF-8 sequence);
/// a boundary can be in a tab, between its virtual spaces.
fn at_boundary(tokenizer: &Tokenizer) -> bool {
    let point = &tokenizer.point;
    point.vs > 0
        || tokenizer
            .parse_state
            .bytes
            .get(point.index)
            .map_or(true, |byte| !(0x80..0xC0).contains(byte))
}

/// Number of a token name in this parse, if it fits in an event.
fn intern(tokenizer: &Tokenizer, construct: u8, name: &'static str) -> Option<u16> {
    let mut names = tokenizer.parse_state.extension_names.borrow_mut();
    let index = names
        .iter()
        .position(|known| known.0 == construct && known.1 == name)
        .unwrap_or_else(|| {
            names.push((construct, name));
            names.len() - 1
        });
    u16::try_from(index).ok()
}

/// Enter a token of a construct.
fn enter_token(tokenizer: &mut Tokenizer, id: u16) {
    tokenizer.enter(Name::Extension);
    tokenizer
        .events
        .last_mut()
        .expect("expected event")
        .extension = id;
    ext_mut(tokenizer).open.push(id);
}

/// Exit the innermost open token of a construct.
fn exit_token(tokenizer: &mut Tokenizer) {
    let id = ext_mut(tokenizer).open.pop().expect("expected open token");
    tokenizer.exit(Name::Extension);
    tokenizer
        .events
        .last_mut()
        .expect("expected event")
        .extension = id;
}

/// Construct at `index`.
pub(crate) fn construct(options: &ParseOptions, index: u8) -> &dyn Construct {
    &*options.text_constructs[usize::from(index)]
}

/// Start trying construct `index`.
pub(crate) fn start(tokenizer: &mut Tokenizer, index: u8) -> State {
    let (start, events) = (tokenizer.point.index, tokenizer.events.len());
    let state = ext_mut(tokenizer);
    state.start = start;
    state.events = events;
    state.retries = 0;
    state.index = index;
    state.state = 0;
    state.memory = [0; 4];
    state.open.clear();
    State::Retry(StateName::ExtensionStep)
}

/// Run the current construct, enforcing the rules of [`Construct`].
pub(crate) fn step(tokenizer: &mut Tokenizer) -> State {
    let (index, state) = (ext(tokenizer).index, ext(tokenizer).state);
    let construct = construct(tokenizer.parse_state.options, index);
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
    let tokenize_state = ext(tokenizer);

    let step = match step {
        _ if broken => Step::Nok,
        Step::Next(_) if !consumed => Step::Nok,
        Step::Retry(_) if consumed || tokenize_state.retries >= RETRY_MAX => Step::Nok,
        Step::Ok
            if line_ending
                || tokenizer.point.index <= tokenize_state.start
                || !tokenize_state.open.is_empty() =>
        {
            Step::Nok
        }
        step => step,
    };

    match step {
        Step::Next(state) => {
            let tokenize_state = ext_mut(tokenizer);
            tokenize_state.retries = 0;
            tokenize_state.state = state;
            State::Next(StateName::ExtensionStep)
        }
        Step::Retry(state) => {
            let tokenize_state = ext_mut(tokenizer);
            tokenize_state.retries += 1;
            tokenize_state.state = state;
            State::Retry(StateName::ExtensionStep)
        }
        Step::Ok => State::Ok,
        Step::Nok => State::Nok,
    }
}

/// Index of the exit of the event entered at `index`.
fn balanced_exit(events: &[Event], mut index: usize) -> usize {
    let mut depth = 0;
    loop {
        match events[index].kind {
            Kind::Enter => depth += 1,
            Kind::Exit => depth -= 1,
        }
        if depth == 0 {
            return index;
        }
        index += 1;
    }
}

/// A match of a construct.
pub(crate) struct Match<'a> {
    pub tokens: Vec<Token<'a>>,
    /// Index of the last event of the match.
    pub end: usize,
}

/// Gather the tokens of the match that starts at `start`.
pub(crate) fn collect_tokens<'a>(
    events: &[Event],
    bytes: &'a [u8],
    names: &[TokenName],
    start: usize,
) -> Match<'a> {
    // Name, enter, and exit.
    let mut spans: Vec<(&'static str, usize, usize)> = vec![];
    let mut open = vec![];
    // Events left out of values: container prefixes.
    let mut excluded = vec![];
    let mut index = start;

    loop {
        let event = &events[index];
        match (&event.kind, &event.name) {
            (Kind::Enter, Name::Extension) => {
                spans.push((names[usize::from(event.extension)].1, index, index));
                open.push(spans.len() - 1);
            }
            (Kind::Exit, Name::Extension) => {
                let span = open.pop().expect("expected an open token");
                spans[span].2 = index;
                if open.is_empty() {
                    break;
                }
            }
            // Container prefixes.
            (Kind::Enter, name) if *name != Name::LineEnding => {
                let enter = index;
                index = balanced_exit(events, index);
                excluded.push((enter, index));
            }
            _ => {}
        }
        index += 1;
    }

    let tokens = spans
        .into_iter()
        .map(|(name, enter, exit)| Token {
            name,
            value: own_text(events, bytes, enter, exit, &excluded),
            position: Position {
                start: events[enter].point.to_unist(),
                end: events[exit].point.to_unist(),
            },
        })
        .collect();

    Match { tokens, end: index }
}

/// Source text from event `enter` to event `exit`, without the excluded
/// events inside.
fn own_text<'a>(
    events: &[Event],
    bytes: &'a [u8],
    enter: usize,
    exit: usize,
    excluded: &[(usize, usize)],
) -> Cow<'a, str> {
    // Like `to_html`, leaves out tabs partly used by prefixes.
    let text = |from: usize, to: usize| {
        let slice = Slice::from_position(
            bytes,
            &SlicePosition {
                start: &events[from].point,
                end: &events[to].point,
            },
        );
        str::from_utf8(slice.bytes).expect("expected tokens between characters")
    };
    let mut bounds = vec![enter];
    // Excluded spans are in order, and do not overlap.
    let first = excluded.partition_point(|(excluded_enter, _)| *excluded_enter <= enter);
    for (excluded_enter, excluded_exit) in &excluded[first..] {
        if *excluded_exit >= exit {
            break;
        }
        bounds.push(*excluded_enter);
        bounds.push(*excluded_exit);
    }
    bounds.push(exit);

    if bounds.len() == 2 {
        Cow::Borrowed(text(enter, exit))
    } else {
        Cow::Owned(
            bounds
                .chunks(2)
                .map(|pair| text(pair[0], pair[1]))
                .collect::<String>(),
        )
    }
}

/// Text markers: the built-in ones and those of constructs, except line
/// endings; empty without constructs.
pub(crate) fn text_markers(builtin: &[u8], constructs: &[Box<dyn Construct>]) -> Vec<u8> {
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
