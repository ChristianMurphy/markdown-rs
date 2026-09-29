//! Syntax extensions: constructs to add to markdown.
//!
//! A construct is a state machine driven by the built-in tokenizer, like a
//! micromark construct, so it sees text without container prefixes such as
//! `> `, across lines.
//! Pass constructs in `text_constructs` and `flow_constructs` of
//! [`ParseOptions`].
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

use crate::construct::partial_space_or_tab::space_or_tab_min_max;
use crate::event::{Content, Event, Kind, Link, Name, Point};
use crate::mdast;
use crate::message;
use crate::state::{Name as StateName, State};
use crate::subtokenize::link_to;
use crate::tokenizer::{move_point_back, Tokenizer};
use crate::unist::Position;
use crate::util::constant::TAB_SIZE;
use crate::util::slice::{Position as SlicePosition, Slice};
use crate::ParseOptions;
use alloc::{borrow::Cow, boxed::Box, string::String, vec, vec::Vec};
use core::{convert::TryFrom, str};

/// Most steps a match takes for each byte from its start through the furthest
/// point it reached.
const STEP_MAX: usize = 256;

/// Most attempts a construct can be in at once.
const ATTEMPT_MAX: usize = 256;

/// Most levels of content around content, built-in levels included: each
/// level is parsed once more.
const CONTENT_MAX: usize = 32;

/// A construct, such as a mention.
///
/// Constructs in [`ParseOptions::text_constructs`][crate::ParseOptions] run
/// in text, and those in `flow_constructs` at the start of a line in flow,
/// after up to 3 columns of indentation.
/// They are tried in order, before built-ins, at their markers, which are
/// never line endings, and in flow never spaces or tabs, which are
/// indentation.
///
/// A construct that breaks one of these rules does not match, so its bytes
/// stay what they would otherwise be (in an attempt, the attempt fails):
///
/// * every consumed byte is inside a token, and one token holds the others
/// * a token holds at least one byte; an empty content token is dropped
/// * `exit` closes the innermost open token; an attempt closes only tokens
///   it opened
/// * `Next` comes after a `consume`, `Retry` and `Attempt` do not
/// * after consuming a line ending, a step returns `Next`; in text, it can
///   first `exit` tokens that end there
/// * in flow, an attempt does not consume a line ending
/// * the first byte of a match is not content
/// * every line of content has a byte other than a space or tab, like a
///   paragraph
/// * inside content, a token holds no content, and starts before the
///   content of its line, like a line prefix
/// * `Ok` comes after at least one byte, with every token closed; in flow,
///   at a line ending or the end, in a step that did not consume; in an
///   attempt, with the tokens it opened closed
/// * tokens start and end between characters, not inside one; a tab is one
///   character, but a line can start inside it, after container prefixes
///
/// A match takes at most 256 steps for each byte from its start through the
/// furthest point it reached, failed attempts included, and moving past a
/// container prefix counts a step for each of its bytes; past that, it does
/// not match.
/// Attempts nest at most 256 deep; a deeper attempt fails.
/// Content nests at most 32 deep, counting the content of built-in
/// constructs, such as the text of a paragraph; deeper content does not
/// match.
/// A failed construct is tried again at its next marker, so keep lookahead
/// bounded, or parsing becomes quadratic.
/// A flow construct decides on its first line: after it consumed a line
/// ending, `Nok` is a parse error.
pub trait Construct {
    /// Bytes this construct can start at.
    fn markers(&self) -> &[u8];

    /// Whether the construct can start after `previous`, the byte before it:
    /// `b'\n'` at the start of a line, after container prefixes and before
    /// the indentation of a flow construct, and `None` at the start or after
    /// a character escape.
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
    /// Try `state` at the current byte: if it reaches `Ok`, go to `ok` where
    /// the attempt stopped; if it reaches `Nok`, undo it and go to `nok` at
    /// the byte where it started.
    Attempt {
        /// State to try.
        state: u16,
        /// State after success.
        ok: u16,
        /// State after failure.
        nok: u16,
    },
    /// The construct (or attempt) matched.
    Ok,
    /// The construct (or attempt) did not match: its events are discarded.
    Nok,
}

/// Kind of markdown inside a content token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContentType {
    /// Phrasing, such as a label.
    Text,
}

/// Token of a construct: a name, the text it spans, and where.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Token<'a> {
    /// Name given to `enter`.
    pub name: &'static str,
    /// Source text, without container prefixes and content (empty for a
    /// content token).
    pub value: Cow<'a, str>,
    /// Content of a content token, parsed as markdown.
    pub children: Vec<mdast::Node>,
    /// Positional info.
    pub position: Position,
}

/// What an interned token name is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    /// A token of the construct.
    Token,
    /// A token whose inside is parsed as markdown.
    Content(Content),
    /// A token of the construct inside a content token, such as a line
    /// prefix: its bytes are not content, and compilers skip it.
    InContent,
}

/// Index of a construct, name of a token, and what it is, interned per
/// parse: events of constructs refer to it by index.
pub(crate) type TokenName = (u8, &'static str, TokenKind);

/// State of constructs in a tokenizer, boxed, so the tokenizer stays small
/// without them.
#[derive(Debug, Default)]
pub(crate) struct ExtensionState {
    /// Where the current construct started.
    start: usize,
    /// Number of events when the current construct started.
    events: usize,
    /// Steps the current match took, where a step that moved past more than
    /// one byte, such as a container prefix, counts each byte.
    steps: usize,
    /// Where the last step of the current match was.
    at: usize,
    /// Furthest byte a step of the current match was at.
    furthest: usize,
    /// Construct being tried.
    index: u8,
    /// State of the construct being tried.
    state: u16,
    /// Construct to try next at the current byte.
    pub(crate) next: u8,
    /// Attempts the current construct is in.
    attempts: Vec<AttemptFrame>,
    /// Memory of the current match.
    memory: [usize; 4],
    /// Interned names of the open tokens of the current match.
    open: Vec<u16>,
    /// Whether to keep the initial and final whitespace of this text, which
    /// is content of a construct, like micromark’s `_contentTypeTextTrailing`.
    pub(crate) keep_whitespace: bool,
    /// Index in `open` of the current content token, if it is open.
    content_at: usize,
    /// Line of the last byte of the open content, `0` if none.
    content_line: usize,
    /// Whether the current line of the open content has only spaces and tabs.
    content_blank: bool,
    /// Enter of the last chunk of the current content token.
    last_chunk: Option<usize>,
    /// Where the current flow construct is in its lines.
    line: FlowLine,
    /// Columns of indentation before the current flow construct.
    indent: usize,
}

/// Where a flow construct is in its lines.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FlowLine {
    /// On its first line, where it can fail without an error.
    #[default]
    First,
    /// Past a line ending, so it matches or errors.
    Committed,
    /// At a line ending before a lazy line or the end, where it ends.
    Lazy,
}

/// An attempt a construct is in, and what undoing it restores.
#[derive(Debug)]
struct AttemptFrame {
    /// State after success.
    ok: u16,
    /// State after failure.
    nok: u16,
    /// Open tokens of the match when the attempt started.
    open_len: usize,
    /// Start of the line when the attempt started, which the tokenizer does
    /// not restore.
    line_start: Point,
    /// Line of the last byte of content when the attempt started.
    content_line: usize,
    /// Whether the line of content was blank when the attempt started.
    content_blank: bool,
    /// Last chunk when the attempt started.
    last_chunk: Option<usize>,
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

/// Where a construct runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Place {
    Text,
    Flow,
}

/// The tokenizer, as a construct sees it.
pub struct ConstructTokenizer<'t, 'a> {
    tokenizer: &'t mut Tokenizer<'a>,
    index: u8,
    place: Place,
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
    /// In flow, a line ending before a lazy line or the end is `None`: the
    /// construct ends there.
    pub fn current(&self) -> Option<u8> {
        if self.place == Place::Flow
            && self.tokenizer.current == Some(b'\n')
            && ext(self.tokenizer).line == FlowLine::Lazy
        {
            None
        } else {
            self.tokenizer.current
        }
    }

    /// Current character, or `None` at the end or inside a character.
    ///
    /// Line endings are `'\n'`, as with [`current`][Self::current].
    /// `consume` takes one byte, or one column of a tab: a character takes
    /// as many steps as it has bytes or columns.
    pub fn current_char(&self) -> Option<char> {
        match self.current() {
            Some(b'\n') => Some('\n'),
            Some(_) if self.tokenizer.point.vs > 0 && !self.tokenizer.at_line_start() => None,
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

    /// Columns of indentation (0 to 3) before a flow construct, which it
    /// starts after, like micromark’s `linePrefix`; `0` in text.
    pub fn indent(&self) -> usize {
        ext(self.tokenizer).indent
    }

    /// Memory of the current match, all zero at its start, such as the size
    /// of an opening fence: micromark keeps these in closures.
    ///
    /// A failed attempt does not undo changes.
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
        let in_content = is_content_top(self.tokenizer);

        if in_content && is_eol && ext(self.tokenizer).content_blank {
            self.broken = true;
            return;
        }

        // In flow, the line ending is consumed after the step, once the next
        // line is known to continue the construct.
        if self.place == Place::Flow && is_eol {
            if !ext(self.tokenizer).attempts.is_empty() {
                self.broken = true;
                return;
            }
        } else if in_content {
            // Content goes into linked chunks, one per line, parsed later.
            if !is_chunk_open(self.tokenizer) {
                let content = content_of_top(self.tokenizer);
                enter_chunk(self.tokenizer, Name::ExtensionChunk, content);
                self.tokenizer.stack.pop();
            }
            let (line, is_blank) = (
                self.tokenizer.point.line,
                matches!(self.tokenizer.current, Some(b'\t' | b' ')),
            );
            let state = ext_mut(self.tokenizer);
            state.content_line = line;
            if !is_blank {
                state.content_blank = is_eol;
            }
            self.tokenizer.consume();
            if is_eol {
                exit_chunk(self.tokenizer);
            }
        } else if is_eol {
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
        self.enter_impl(name, None);
    }

    /// Start a token whose inside is parsed as markdown later, like
    /// micromark’s `contentType`.
    ///
    /// Bytes consumed directly in it are content; tokens entered in it,
    /// such as a line prefix or a closing fence, are not, and are not given
    /// to [`Construct::to_mdast`].
    pub fn enter_content(&mut self, name: &'static str, content: ContentType) {
        self.enter_impl(
            name,
            Some(match content {
                ContentType::Text => Content::Text,
            }),
        );
    }

    fn enter_impl(&mut self, name: &'static str, content: Option<Content>) {
        let state = ext(self.tokenizer);
        let inside_content = state.open.get(state.content_at).map_or(false, |id| {
            matches!(
                self.tokenizer.parse_state.extension_names.borrow()[usize::from(*id)].2,
                TokenKind::Content(_)
            )
        });
        let is_content = content.is_some();
        let kind = match content {
            Some(content) => TokenKind::Content(content),
            None if inside_content => TokenKind::InContent,
            None => TokenKind::Token,
        };
        // The first token starts the match, and holds the others.
        let can_start = self.tokenizer.events.len() == state.events || !state.open.is_empty();
        // Content smaller than its match cannot match again forever.
        let is_first_byte = is_content && self.tokenizer.point.index == state.start;
        let is_too_deep = is_content && self.tokenizer.parse_state.content_depth >= CONTENT_MAX;
        // A token in content comes before any content on its line, such as
        // a line prefix: after content, it would split that content.
        let is_after_content =
            kind == TokenKind::InContent && state.content_line == self.tokenizer.point.line;

        if self.line_ending
            || (inside_content && is_content)
            || !can_start
            || is_first_byte
            || is_too_deep
            || is_after_content
            || !at_boundary(self.tokenizer)
        {
            self.broken = true;
            return;
        }

        match intern(self.tokenizer, self.index, name, kind) {
            Some(id) => {
                let content_at = ext(self.tokenizer).open.len();
                enter_token(self.tokenizer, id);
                if is_content {
                    let state = ext_mut(self.tokenizer);
                    state.content_at = content_at;
                    state.content_line = 0;
                    state.content_blank = true;
                    state.last_chunk = None;
                }
            }
            None => self.broken = true,
        }
    }

    /// End the innermost open token, which must be called `name`.
    pub fn exit(&mut self, name: &'static str) {
        let state = ext(self.tokenizer);
        let names = self.tokenizer.parse_state.extension_names.borrow();
        let (is_named, is_content) = state.open.last().map_or((false, false), |id| {
            let (_, known, kind) = &names[usize::from(*id)];
            (*known == name, matches!(kind, TokenKind::Content(_)))
        });
        drop(names);
        // An attempt cannot close tokens opened before it.
        let is_before_attempt = state
            .attempts
            .last()
            .map_or(false, |frame| state.open.len() <= frame.open_len);

        // In flow, the line ending is consumed after the step.
        if (self.line_ending && self.place == Place::Flow)
            || !is_named
            || is_before_attempt
            || !at_boundary(self.tokenizer)
        {
            self.broken = true;
            return;
        }

        if is_chunk_open(self.tokenizer) {
            exit_chunk(self.tokenizer);
        }

        if !self.top_is_empty() {
            if is_content && ext(self.tokenizer).content_blank {
                self.broken = true;
            } else {
                exit_token(self.tokenizer);
            }
        } else if is_content {
            // Empty content is no content.
            self.tokenizer.events.pop();
            self.tokenizer.stack.pop();
            ext_mut(self.tokenizer).open.pop();
        } else {
            self.broken = true;
        }
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

/// Whether the tokenizer is between characters: not in a UTF-8 sequence, and
/// not in a tab, unless a line starts there after container prefixes.
fn at_boundary(tokenizer: &Tokenizer) -> bool {
    let point = &tokenizer.point;
    (point.vs == 0 || tokenizer.at_line_start())
        && tokenizer
            .parse_state
            .bytes
            .get(point.index)
            .map_or(true, |byte| !(0x80..0xC0).contains(byte))
}

/// Number of a token name in this parse, if it fits in an event.
fn intern(
    tokenizer: &Tokenizer,
    construct: u8,
    name: &'static str,
    kind: TokenKind,
) -> Option<u16> {
    let mut names = tokenizer.parse_state.extension_names.borrow_mut();
    let index = names
        .iter()
        .position(|known| known.0 == construct && known.1 == name && known.2 == kind)
        .unwrap_or_else(|| {
            names.push((construct, name, kind));
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

/// Whether the innermost open token is a content token (or its chunk).
fn is_content_top(tokenizer: &Tokenizer) -> bool {
    match tokenizer.stack.last() {
        Some(Name::Extension) => ext(tokenizer).open.last().map_or(false, |id| {
            matches!(
                tokenizer.parse_state.extension_names.borrow()[usize::from(*id)].2,
                TokenKind::Content(_)
            )
        }),
        _ => false,
    }
}

/// Kind of content in the innermost open content token.
fn content_of_top(tokenizer: &Tokenizer) -> Content {
    let id = *ext(tokenizer).open.last().expect("expected content token");
    match &tokenizer.parse_state.extension_names.borrow()[usize::from(id)].2 {
        TokenKind::Content(content) => content.clone(),
        _ => unreachable!("expected content token"),
    }
}

/// Enter a chunk of content, linked to the previous chunk of the same
/// content token, across tokens in it.
fn enter_chunk(tokenizer: &mut Tokenizer, name: Name, content: Content) {
    tokenizer.enter_link(
        name,
        Link {
            previous: None,
            next: None,
            content,
        },
    );
    let current = tokenizer.events.len() - 1;
    if let Some(previous) = ext_mut(tokenizer).last_chunk.replace(current) {
        link_to(&mut tokenizer.events, previous, current);
    }
}

/// Whether a chunk of content is open: its enter is the last event.
///
/// Chunks stay off the tokenizer stack, so undoing an attempt, which
/// truncates events and the stack, also restores an open chunk.
fn is_chunk_open(tokenizer: &Tokenizer) -> bool {
    tokenizer.events.last().map_or(false, |event| {
        event.kind == Kind::Enter && event.name == Name::ExtensionChunk
    })
}

/// Exit the open chunk.
fn exit_chunk(tokenizer: &mut Tokenizer) {
    tokenizer.stack.push(Name::ExtensionChunk);
    tokenizer.exit(Name::ExtensionChunk);
}

/// Construct at `index`: text constructs, then flow.
pub(crate) fn construct(options: &ParseOptions, index: u8) -> (&dyn Construct, Place) {
    let index = usize::from(index);
    match options.text_constructs.get(index) {
        Some(construct) => (&**construct, Place::Text),
        None => (
            &*options.flow_constructs[index - options.text_constructs.len()],
            Place::Flow,
        ),
    }
}

/// Start trying construct `index`.
pub(crate) fn start(tokenizer: &mut Tokenizer, index: u8) -> State {
    let column = tokenizer.point.column;
    let state = ext_mut(tokenizer);
    state.index = index;
    state.state = 0;
    state.memory = [0; 4];
    state.open.clear();
    state.last_chunk = None;
    state.line = FlowLine::First;
    state.indent = column;

    if construct(tokenizer.parse_state.options, index).1 == Place::Flow
        && matches!(tokenizer.current, Some(b'\t' | b' '))
    {
        tokenizer.attempt(State::Next(StateName::ExtensionIndentAfter), State::Nok);
        return State::Retry(space_or_tab_min_max(tokenizer, 0, TAB_SIZE - 1));
    }

    indent_after(tokenizer)
}

/// After the indentation of a flow construct, at its marker.
pub(crate) fn indent_after(tokenizer: &mut Tokenizer) -> State {
    let options = tokenizer.parse_state.options;
    let (start, column) = (tokenizer.point.index, tokenizer.point.column);
    let events = tokenizer.events.len();
    let state = ext_mut(tokenizer);
    let (construct, place) = construct(options, state.index);
    state.indent = column - state.indent;
    state.start = start;
    state.events = events;
    state.steps = 0;
    state.at = start;
    state.furthest = start;

    match tokenizer.current {
        // In flow, spaces and tabs are indentation.
        Some(b'\t' | b' ') if place == Place::Flow => State::Nok,
        Some(byte) if byte != b'\n' && construct.markers().contains(&byte) => {
            State::Retry(StateName::ExtensionStep)
        }
        _ => State::Nok,
    }
}

/// After a flow line ending that continues the construct: consume it.
///
/// Line endings in flow are real, so containers can count lines; in content,
/// they are linked chunks.
pub(crate) fn at_non_lazy(tokenizer: &mut Tokenizer) -> State {
    // The end right after this line ending, the last byte, is like a lazy
    // line.
    if tokenizer.point.index + 1 == tokenizer.parse_state.bytes.len() {
        return at_lazy(tokenizer);
    }

    if is_content_top(tokenizer) {
        let content = content_of_top(tokenizer);
        if is_chunk_open(tokenizer) {
            exit_chunk(tokenizer);
        }
        enter_chunk(tokenizer, Name::LineEnding, content);
        let line = tokenizer.point.line;
        let state = ext_mut(tokenizer);
        state.content_line = line;
        state.content_blank = true;
    } else {
        tokenizer.enter(Name::LineEnding);
    }
    tokenizer.consume();
    tokenizer.exit(Name::LineEnding);
    State::Next(StateName::ExtensionStep)
}

/// After a flow line ending followed by a lazy line or the end.
pub(crate) fn at_lazy(tokenizer: &mut Tokenizer) -> State {
    ext_mut(tokenizer).line = FlowLine::Lazy;
    State::Retry(StateName::ExtensionStep)
}

/// After an attempt of a construct that succeeded.
pub(crate) fn attempt_ok(tokenizer: &mut Tokenizer) -> State {
    let state = ext_mut(tokenizer);
    state.state = state.attempts.pop().expect("expected attempt").ok;
    State::Retry(StateName::ExtensionStep)
}

/// After an attempt of a construct that failed, and was undone.
pub(crate) fn attempt_nok(tokenizer: &mut Tokenizer) -> State {
    let state = ext_mut(tokenizer);
    let frame = state.attempts.pop().expect("expected attempt");
    state.open.truncate(frame.open_len);
    state.content_line = frame.content_line;
    state.content_blank = frame.content_blank;
    state.last_chunk = frame.last_chunk;
    state.state = frame.nok;
    tokenizer.line_start = frame.line_start;

    // The undo removed chunks the attempt opened, but not the link to them.
    if let Some(chunk) = frame.last_chunk {
        let events_len = tokenizer.events.len();
        let link = tokenizer.events[chunk]
            .link
            .as_mut()
            .expect("expected link");
        if link.next.map_or(false, |next| next >= events_len) {
            link.next = None;
        }
    }

    State::Retry(StateName::ExtensionStep)
}

/// Run the current construct, enforcing the rules of [`Construct`].
pub(crate) fn step(tokenizer: &mut Tokenizer) -> State {
    let point = tokenizer.point.index;
    let tokenize_state = ext_mut(tokenizer);
    tokenize_state.steps += point.saturating_sub(tokenize_state.at).max(1);
    tokenize_state.at = point;
    tokenize_state.furthest = tokenize_state.furthest.max(point);
    // Work is linear in the bytes a match reaches, whatever its attempts do.
    let budget = STEP_MAX.saturating_mul(tokenize_state.furthest - tokenize_state.start + 1);
    let index = tokenize_state.index;
    let place = construct(tokenizer.parse_state.options, index).1;
    let is_flow = place == Place::Flow;
    let (step, line_ending) = if ext(tokenizer).steps > budget {
        (Step::Nok, false)
    } else {
        run(tokenizer, index, place)
    };
    let in_attempt = !ext(tokenizer).attempts.is_empty();

    match step {
        Step::Next(state) => {
            ext_mut(tokenizer).state = state;

            // Past its first line, a flow construct matches or errors, and
            // containers must not start in the next line while it is checked.
            if is_flow && line_ending {
                ext_mut(tokenizer).line = FlowLine::Committed;
                tokenizer.concrete = true;
                tokenizer.check(
                    State::Next(StateName::ExtensionNonLazy),
                    State::Next(StateName::ExtensionLazy),
                );
                return State::Retry(StateName::NonLazyContinuationStart);
            }

            State::Next(StateName::ExtensionStep)
        }
        Step::Retry(state) => {
            ext_mut(tokenizer).state = state;
            State::Retry(StateName::ExtensionStep)
        }
        Step::Attempt { state, ok, nok } => {
            let line_start = tokenizer.line_start.clone();
            let tokenize_state = ext_mut(tokenizer);
            tokenize_state.state = state;
            let frame = AttemptFrame {
                ok,
                nok,
                open_len: tokenize_state.open.len(),
                line_start,
                content_line: tokenize_state.content_line,
                content_blank: tokenize_state.content_blank,
                last_chunk: tokenize_state.last_chunk,
            };
            tokenize_state.attempts.push(frame);
            tokenizer.attempt(
                State::Next(StateName::ExtensionAttemptOk),
                State::Next(StateName::ExtensionAttemptNok),
            );
            State::Retry(StateName::ExtensionStep)
        }
        Step::Ok => {
            if is_flow && !in_attempt {
                tokenizer.interrupt = false;
                tokenizer.concrete = false;
            }
            State::Ok
        }
        Step::Nok => {
            if in_attempt {
                return State::Nok;
            }
            if ext(tokenizer).line != FlowLine::First {
                return State::Error(message::Message {
                    place: Some(Box::new(message::Place::Point(tokenizer.point.to_unist()))),
                    reason: "Unexpected failure of a flow construct after its first line".into(),
                    rule_id: Box::new("flow-construct-late-failure".into()),
                    source: Box::new("markdown-rs".into()),
                });
            }
            State::Nok
        }
    }
}

/// Take a step of construct `index`, and turn a step that breaks a rule of
/// [`Construct`] into `Nok`; also whether it consumed a line ending.
fn run(tokenizer: &mut Tokenizer, index: u8, place: Place) -> (Step, bool) {
    let state = ext(tokenizer).state;
    let construct = construct(tokenizer.parse_state.options, index).0;
    let mut construct_tokenizer = ConstructTokenizer {
        tokenizer,
        index,
        place,
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
    let attempt = tokenize_state.attempts.last();

    let step = match step {
        _ if broken => Step::Nok,
        Step::Next(_) if !consumed => Step::Nok,
        Step::Retry(_) | Step::Attempt { .. } if consumed => Step::Nok,
        Step::Attempt { .. } if tokenize_state.attempts.len() >= ATTEMPT_MAX => Step::Nok,
        Step::Ok if line_ending => Step::Nok,
        // An attempt ends with the tokens it opened closed.
        Step::Ok if attempt.map_or(false, |frame| tokenize_state.open.len() != frame.open_len) => {
            Step::Nok
        }
        Step::Ok
            if attempt.is_none()
                && (tokenizer.point.index <= tokenize_state.start
                    || !tokenize_state.open.is_empty()
                    // A flow construct ends at a line ending or the end.
                    || (place == Place::Flow
                        && (consumed || !matches!(tokenizer.current, None | Some(b'\n'))))) =>
        {
            Step::Nok
        }
        step => step,
    };

    (step, line_ending)
}

/// Whether `event` is of a token of a construct in content, such as a line
/// prefix.
pub(crate) fn is_in_content(names: &[TokenName], event: &Event) -> bool {
    event.name == Name::Extension && names[usize::from(event.extension)].2 == TokenKind::InContent
}

/// Index of the exit of the event entered at `index`.
pub(crate) fn balanced_exit(events: &[Event], mut index: usize) -> usize {
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

/// A match of a construct, as event indices.
pub(crate) struct Match<'a> {
    pub tokens: Vec<Token<'a>>,
    /// Content tokens: token, enter, and exit.
    pub contents: Vec<(usize, usize, usize)>,
    /// Events left out of values: content, and container prefixes.
    pub excluded: Vec<(usize, usize)>,
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
    // Name, enter, exit, and whether content.
    let mut spans: Vec<(&'static str, usize, usize, bool)> = vec![];
    let mut open = vec![];
    let mut contents = vec![];
    let mut excluded = vec![];
    let mut index = start;
    // The match’s own tokens in content are inside its content tokens, which
    // are skipped, so tokens in content here are another match’s.
    let is_own = |event: &Event| names[usize::from(event.extension)].2 != TokenKind::InContent;

    loop {
        let event = &events[index];
        match (&event.kind, &event.name) {
            (Kind::Enter, Name::Extension) if is_own(event) => {
                let (_, name, kind) = &names[usize::from(event.extension)];
                let is_content = matches!(kind, TokenKind::Content(_));
                spans.push((name, index, index, is_content));
                open.push(spans.len() - 1);

                if is_content {
                    let enter = index;
                    index = balanced_exit(events, index);
                    contents.push((spans.len() - 1, enter, index));
                    excluded.push((enter, index));
                    // Handle the exit.
                    continue;
                }
            }
            (Kind::Exit, Name::Extension) if is_own(event) => {
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
        .map(|(name, enter, exit, is_content)| Token {
            name,
            value: if is_content {
                Cow::Borrowed("")
            } else {
                own_text(events, bytes, enter, exit, &excluded)
            },
            children: vec![],
            position: Position {
                start: events[enter].point.to_unist(),
                end: events[exit].point.to_unist(),
            },
        })
        .collect();

    Match {
        tokens,
        contents,
        excluded,
        end: index,
    }
}

/// Source text from event `enter` to event `exit`, without the excluded
/// events inside.
pub(crate) fn own_text<'a>(
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
