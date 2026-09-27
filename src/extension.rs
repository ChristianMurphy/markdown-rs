//! Syntax extensions: constructs that plugins add to markdown.
//!
//! A construct is a state machine driven by the built-in tokenizer, like a
//! micromark construct, so it sees text without container prefixes such as
//! `> `, across lines.

use crate::construct::partial_space_or_tab::space_or_tab_min_max;
use crate::event::{Content, Event, Kind, Link, Name};
use crate::mdast;
use crate::message;
use crate::state::{Name as StateName, State};
use crate::subtokenize::link_to;
use crate::tokenizer::{move_point_back, Container, Tokenizer};
use crate::unist::Position;
use crate::util::constant::TAB_SIZE;
use crate::util::slice::{Position as SlicePosition, Slice};
use crate::ParseOptions;
use alloc::{borrow::Cow, boxed::Box, string::String, vec, vec::Vec};
use core::{convert::TryFrom, str};

/// Most steps a construct can take in a row without consuming a byte.
const RETRY_MAX: u16 = 256;

/// A construct, such as a wiki link or a directive.
///
/// Constructs in [`ParseOptions::text_constructs`][crate::ParseOptions]
/// run in text, and those in `flow_constructs` at the start of a line in
/// flow.
/// They are tried in order, before built-ins, at their markers (never line
/// endings).
///
/// A construct that breaks one of these rules does not match, so its bytes
/// stay what they would otherwise be (in an attempt, the attempt fails):
///
/// * every consumed byte is inside a token, and one token holds the others
/// * `exit` closes the innermost open token; an attempt closes only tokens
///   it opened
/// * `Next` comes after a `consume`, `Retry` and `Attempt` do not
/// * after consuming a line ending, a step returns `Next`; in text it can
///   first `exit` tokens that end there
/// * inside content, a token holds no content, and starts before the
///   content of its line, like a line prefix
/// * `Ok` comes after at least one byte, with every token closed; in flow,
///   at a line ending or the end
/// * tokens start and end between characters, not inside one
///
/// A failed construct is tried again at its next marker: bound lookahead.
/// A flow construct decides on its first line: after it consumed a line
/// ending, `Nok` is a parse error.
pub trait Construct {
    /// Bytes this construct can start at.
    fn markers(&self) -> &[u8];

    /// Whether the construct can start after `previous`, the byte before it,
    /// which is `None` at the start, or after a character escape.
    fn previous(&self, previous: Option<u8>) -> bool {
        let _ = previous;
        true
    }

    /// Take one step at state `state` (the first state is `0`).
    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step;

    /// For a delimiter run in text, like `==` in `==mark==`: the sizes of a
    /// run that can pair, with the same size on both sides.
    ///
    /// When not empty, `step` is not used: the core tokenizes runs of the
    /// markers and pairs them with the rules of emphasis and GFM
    /// strikethrough, like micromark’s `attentionMarkers`.
    /// A pair has an `attention` token, with `attentionSequence` tokens
    /// around an `attentionText` content token.
    fn attention_sizes(&self) -> &[usize] {
        &[]
    }

    /// For a container, in `document_constructs`: the state that checks, at
    /// the start of each later line, whether the container continues, like
    /// micromark’s `continuation`.
    ///
    /// It ends in `Ok` (after its prefix, if any) or `Nok`.
    fn continuation(&self) -> u16 {
        0
    }

    /// Turn the tokens of one match into a node.
    ///
    /// `tokens[0]` is the outermost token, the first one entered.
    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node;
}

/// What to do after a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// Go to this state at the next byte (after `consume`).
    Next(u16),
    /// Go to this state at the current byte.
    Retry(u16),
    /// Try `state` at the current byte: if it reaches `Ok`, go to `ok`; if
    /// it reaches `Nok`, undo it and go to `nok`.
    ///
    /// In flow, an attempt cannot consume a line ending. A failed attempt
    /// counts as a step without consuming a byte.
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
pub enum ContentType {
    /// Phrasing, such as a label.
    Text,
    /// Blocks, such as the body of a container (flow constructs only).
    Document,
}

/// Token of a construct: a name, the text it spans, and where.
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// State of plugin constructs in a tokenizer, boxed, so the tokenizer stays
/// small without plugins.
#[derive(Debug)]
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
    /// Whether the line after the current flow line ending continues the
    /// construct: `0` if unknown, `1` if so, `2` if lazy or the end.
    line: u8,
    /// Whether the current flow construct consumed a line ending.
    committed: bool,
    /// Attempts the current construct is in.
    attempts: Vec<AttemptFrame>,
    /// Memory of the current match.
    memory: [usize; 4],
    /// Columns of indentation before the current flow construct.
    indent: usize,
    /// Whether this text is content of a construct, whose initial and final
    /// whitespace is kept, like micromark’s `_contentTypeTextTrailing`.
    pub(crate) text: bool,
    /// Whether the current container checks its continuation.
    continuation: bool,
    /// Open tokens when the current match started.
    stack_len: usize,
    /// Interned names of the open tokens of the current match.
    open: Vec<u16>,
    /// Line of the last byte of the open content, `0` if none.
    content_line: usize,
    /// Enter of the last chunk of the current content token.
    last_chunk: Option<usize>,
}

/// State without plugin constructs.
static EMPTY: ExtensionState = ExtensionState {
    start: 0,
    events: 0,
    retries: 0,
    index: 0,
    state: 0,
    next: 0,
    line: 0,
    committed: false,
    attempts: Vec::new(),
    memory: [0; 4],
    indent: 0,
    text: false,
    continuation: false,
    stack_len: 0,
    open: Vec::new(),
    content_line: 0,
    last_chunk: None,
};

/// State of plugin constructs in `tokenizer`, to read.
pub(crate) fn ext<'t>(tokenizer: &'t Tokenizer) -> &'t ExtensionState {
    tokenizer
        .tokenize_state
        .extension
        .as_deref()
        .unwrap_or(&EMPTY)
}

/// State of plugin constructs in `tokenizer`, created if needed.
pub(crate) fn ext_mut<'t>(tokenizer: &'t mut Tokenizer) -> &'t mut ExtensionState {
    tokenizer.tokenize_state.extension.get_or_insert_with(|| {
        Box::new(ExtensionState {
            attempts: Vec::new(),
            open: Vec::new(),
            ..EMPTY
        })
    })
}

/// An attempt a construct is in.
#[derive(Debug)]
pub(crate) struct AttemptFrame {
    ok: u16,
    nok: u16,
    /// Open tokens when the attempt started.
    stack_len: usize,
    /// Open tokens of the match when the attempt started.
    open_len: usize,
    /// Where the attempt started.
    start: (usize, usize),
    /// Line of the last byte of content when the attempt started.
    content_line: usize,
    /// Steps without progress when the attempt started.
    retries: u16,
    /// Last chunk when the attempt started.
    last_chunk: Option<usize>,
}

/// Where a construct runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Place {
    Text,
    Flow,
    /// A container.
    Document,
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
            && ext(self.tokenizer).line == 2
        {
            None
        } else {
            self.tokenizer.current
        }
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

    /// Columns of indentation (0 to 3) before a flow construct, which it
    /// starts after, like micromark’s `linePrefix`.
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

        // The next line’s prefix is skipped when the next step starts.
        if self.line_ending {
            self.broken = true;
            return;
        }

        let is_eol = self.tokenizer.current == Some(b'\n');
        let in_content = self.in_content();

        // A container consumes its prefix only: the rest is flow, and its
        // document content comes from flow.
        if (!in_content && ext(self.tokenizer).open.is_empty())
            || (self.place == Place::Document
                && (is_eol || (in_content && content_of_top(self.tokenizer) == Content::Document)))
        {
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
            ext_mut(self.tokenizer).content_line = self.tokenizer.point.line;
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
                ContentType::Document => Content::Document,
            }),
        );
    }

    fn enter_impl(&mut self, name: &'static str, content: Option<Content>) {
        let inside_content = self.inside_content();
        let is_content = content.is_some();
        let is_document_in_text = content == Some(Content::Document) && self.place == Place::Text;
        let state = ext(self.tokenizer);
        let kind = match content {
            Some(content) => TokenKind::Content(content),
            None if inside_content => TokenKind::InContent,
            None => TokenKind::Token,
        };
        let can_start = (self.tokenizer.events.len() == state.events && kind == TokenKind::Token)
            || !state.open.is_empty()
            || (state.continuation && kind == TokenKind::InContent);
        // A token in content comes before any content on its line, such as
        // a line prefix: after content, it would split that content.
        let is_after_content = kind == TokenKind::InContent
            && !state.continuation
            && state.content_line == self.tokenizer.point.line;

        if self.line_ending
            || (inside_content && is_content)
            || is_document_in_text
            || !can_start
            || is_after_content
            || !self.at_boundary()
        {
            self.broken = true;
            return;
        }

        match self.intern(name, kind) {
            Some(id) => {
                if is_chunk_open(self.tokenizer) {
                    exit_chunk(self.tokenizer);
                }
                enter_token(self.tokenizer, id);
                if is_content {
                    let state = ext_mut(self.tokenizer);
                    state.content_line = 0;
                    state.last_chunk = None;
                }
            }
            None => self.broken = true,
        }
    }

    /// End the innermost open token, which must be called `name`.
    pub fn exit(&mut self, name: &'static str) {
        let state = ext(self.tokenizer);
        let top = state.open.last().copied();
        // An attempt cannot close tokens opened before it.
        let is_before_attempt = state
            .attempts
            .last()
            .map_or(false, |frame| state.open.len() <= frame.open_len);
        let names = self.tokenizer.parse_state.extension_names.borrow();
        let (is_named, is_content) = top.map_or((false, false), |id| {
            let (_, known, kind) = &names[usize::from(id)];
            (*known == name, matches!(kind, TokenKind::Content(_)))
        });
        drop(names);

        // In flow, the line ending is consumed after the step.
        if (self.line_ending && self.place == Place::Flow)
            || !is_named
            || is_before_attempt
            || !self.at_boundary()
        {
            self.broken = true;
            return;
        }

        let is_chunk_open = is_chunk_open(self.tokenizer);
        if is_chunk_open {
            exit_chunk(self.tokenizer);
        }

        if is_chunk_open || !self.top_is_empty() {
            exit_token(self.tokenizer);
        } else if is_content {
            // Empty content is no content.
            self.tokenizer.events.pop();
            self.tokenizer.stack.pop();
            ext_mut(self.tokenizer).open.pop();
        } else {
            self.broken = true;
        }
    }

    /// Number of a token name in this parse, if it fits in an event.
    fn intern(&self, name: &'static str, kind: TokenKind) -> Option<u16> {
        intern(self.tokenizer, self.index, name, kind)
    }

    /// Whether bytes consumed now are content: the innermost open token is a
    /// content token.
    fn in_content(&self) -> bool {
        is_content_top(self.tokenizer)
    }

    /// Whether a content token is open, maybe with tokens in it: in this
    /// match, or the content of a container that checks its continuation.
    fn inside_content(&self) -> bool {
        let state = ext(self.tokenizer);
        let names = self.tokenizer.parse_state.extension_names.borrow();
        state.continuation
            || state
                .open
                .iter()
                .any(|id| matches!(names[usize::from(*id)].2, TokenKind::Content(_)))
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
pub(crate) fn intern(
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

/// Text construct for delimiter runs of `marker`, and its index.
pub(crate) fn attention_construct(
    options: &ParseOptions,
    marker: u8,
) -> Option<(u8, &dyn Construct)> {
    if options.text_constructs.is_empty() {
        return None;
    }

    options
        .text_constructs
        .iter()
        .position(|construct| {
            !construct.attention_sizes().is_empty() && construct.markers().contains(&marker)
        })
        .map(|index| {
            (
                u8::try_from(index).expect("expected fewer than 256 constructs"),
                &*options.text_constructs[index],
            )
        })
}

/// Construct at `index`: text constructs, then flow, then document.
pub(crate) fn construct(options: &ParseOptions, index: u8) -> (&dyn Construct, Place) {
    let mut index = usize::from(index);
    if index < options.text_constructs.len() {
        return (&*options.text_constructs[index], Place::Text);
    }
    index -= options.text_constructs.len();
    if index < options.flow_constructs.len() {
        return (&*options.flow_constructs[index], Place::Flow);
    }
    index -= options.flow_constructs.len();
    (&*options.document_constructs[index], Place::Document)
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

/// Start trying construct `index`.
pub(crate) fn start(tokenizer: &mut Tokenizer, index: u8) -> State {
    let (start, column, stack_len) = (
        tokenizer.point.index,
        tokenizer.point.column,
        tokenizer.stack.len(),
    );
    let events = tokenizer.events.len();
    let state = ext_mut(tokenizer);
    state.start = start;
    state.events = events;
    state.retries = 0;
    state.index = index;
    state.state = 0;
    state.line = 0;
    state.committed = false;
    state.attempts.clear();
    state.memory = [0; 4];
    state.indent = column;
    state.continuation = false;
    state.stack_len = stack_len;
    state.open.clear();
    state.last_chunk = None;

    if construct(tokenizer.parse_state.options, index).1 != Place::Text
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
    let (construct, _) = construct(options, state.index);
    state.indent = column - state.indent;
    state.start = start;
    state.events = events;

    match tokenizer.current {
        Some(byte) if byte != b'\n' && construct.markers().contains(&byte) => {
            State::Retry(StateName::ExtensionStep)
        }
        _ => State::Nok,
    }
}

/// At the start of a line, check whether a plugin container continues.
pub(crate) fn continuation(tokenizer: &mut Tokenizer) -> State {
    let document = &tokenizer.tokenize_state;
    let container = &document.document_container_stack[document.document_continued];
    let index = match container.kind {
        Container::Extension(index, ..) => index,
        _ => unreachable!("expected plugin container"),
    };
    // One word of memory lasts as long as the container, like micromark’s
    // `containerState`.
    let size = container.size;
    let (construct, _) = construct(tokenizer.parse_state.options, index);
    let (start, stack_len) = (tokenizer.point.index, tokenizer.stack.len());
    let events = tokenizer.events.len();
    let state = ext_mut(tokenizer);
    state.index = index;
    state.state = construct.continuation();
    state.retries = 0;
    state.attempts.clear();
    state.memory = [size, 0, 0, 0];
    state.continuation = true;
    state.stack_len = stack_len;
    state.open.clear();
    state.last_chunk = None;
    state.events = events;
    state.start = start;
    State::Retry(StateName::ExtensionStep)
}

/// After a flow line ending that continues the construct: consume it.
///
/// Line endings in flow are real, so containers can count lines; in content,
/// they are linked chunks.
pub(crate) fn at_non_lazy(tokenizer: &mut Tokenizer) -> State {
    if is_content_top(tokenizer) {
        let content = content_of_top(tokenizer);
        if is_chunk_open(tokenizer) {
            exit_chunk(tokenizer);
        }
        enter_chunk(tokenizer, Name::LineEnding, content);
    } else {
        tokenizer.enter(Name::LineEnding);
    }
    tokenizer.consume();
    tokenizer.exit(Name::LineEnding);
    ext_mut(tokenizer).committed = true;
    State::Next(StateName::ExtensionStep)
}

/// After a flow line ending followed by a lazy line or the end.
pub(crate) fn at_lazy(tokenizer: &mut Tokenizer) -> State {
    ext_mut(tokenizer).line = 2;
    State::Retry(StateName::ExtensionStep)
}

/// After an attempt of a construct that succeeded.
pub(crate) fn attempt_ok(tokenizer: &mut Tokenizer) -> State {
    let point = (tokenizer.point.index, tokenizer.point.vs);
    let state = ext_mut(tokenizer);
    let frame = state.attempts.pop().expect("expected attempt");
    // Without progress, it counts as a retry.
    if point != frame.start {
        state.retries = 0;
    }
    state.state = frame.ok;
    State::Retry(StateName::ExtensionStep)
}

/// After an attempt of a construct that failed, and was undone.
pub(crate) fn attempt_nok(tokenizer: &mut Tokenizer) -> State {
    let state = ext_mut(tokenizer);
    let frame = state.attempts.pop().expect("expected attempt");
    state.open.truncate(frame.open_len);
    state.content_line = frame.content_line;
    // A failed attempt makes no progress.
    state.retries = frame.retries;
    state.last_chunk = frame.last_chunk;
    state.state = frame.nok;

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
    let index = ext_mut(tokenizer).index;
    let state = ext_mut(tokenizer).state;
    let (construct, place) = construct(tokenizer.parse_state.options, index);
    let is_flow = place == Place::Flow;
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
        Step::Retry(_) | Step::Attempt { .. } | Step::Ok if line_ending => Step::Nok,
        Step::Retry(_) | Step::Attempt { .. }
            if consumed || tokenize_state.retries >= RETRY_MAX =>
        {
            Step::Nok
        }
        // An attempt ends with the tokens it opened closed.
        Step::Ok if attempt.map_or(false, |frame| tokenizer.stack.len() != frame.stack_len) => {
            Step::Nok
        }
        // A continuation ends with the tokens it opened closed.
        Step::Ok
            if attempt.is_none()
                && tokenize_state.continuation
                && (tokenizer.stack.len() != tokenize_state.stack_len
                    || !at_boundary(tokenizer)) =>
        {
            Step::Nok
        }
        // A container starts with its open token and its content token open.
        Step::Ok
            if attempt.is_none()
                && place == Place::Document
                && !tokenize_state.continuation
                && container_tokens(tokenizer).is_none() =>
        {
            Step::Nok
        }
        // A match ends with the exit of its outermost token.
        Step::Ok
            if attempt.is_none()
                && place != Place::Document
                && (tokenizer.events.last().map_or(true, |event| {
                    event.kind != Kind::Exit
                        || event.name != Name::Extension
                        || tokenizer.parse_state.extension_names.borrow()
                            [usize::from(event.extension)]
                        .0 != index
                }) || tokenizer.point.index <= tokenize_state.start
                    || !at_boundary(tokenizer)
                    || (is_flow
                        && (consumed || !matches!(tokenizer.current, None | Some(b'\n'))))
                    || !tokenize_state.open.is_empty()) =>
        {
            Step::Nok
        }
        step => step,
    };

    let in_attempt = !ext_mut(tokenizer).attempts.is_empty();

    match step {
        Step::Next(state) => {
            ext_mut(tokenizer).line = 0;
            ext_mut(tokenizer).retries = 0;
            ext_mut(tokenizer).state = state;

            // Containers must not start in the next line while it is checked.
            if is_flow && line_ending {
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
            ext_mut(tokenizer).retries += 1;
            ext_mut(tokenizer).state = state;
            State::Retry(StateName::ExtensionStep)
        }
        Step::Attempt { state, ok, nok } => {
            ext_mut(tokenizer).retries += 1;
            ext_mut(tokenizer).state = state;
            let stack_len = tokenizer.stack.len();
            let start = (tokenizer.point.index, tokenizer.point.vs);
            let state = ext_mut(tokenizer);
            let open_len = state.open.len();
            let content_line = state.content_line;
            let retries = state.retries;
            let last_chunk = state.last_chunk;
            state.attempts.push(AttemptFrame {
                ok,
                nok,
                stack_len,
                open_len,
                start,
                content_line,
                retries,
                last_chunk,
            });
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
                ext_mut(tokenizer).committed = false;
            }
            if place == Place::Document && !in_attempt {
                if ext_mut(tokenizer).continuation {
                    ext_mut(tokenizer).continuation = false;
                } else {
                    let (open, content) =
                        container_tokens(tokenizer).expect("expected container tokens");
                    let size = ext_mut(tokenizer).memory[0];
                    let state = &mut tokenizer.tokenize_state;
                    let container = &mut state.document_container_stack[state.document_continued];
                    container.kind = Container::Extension(index, open, content);
                    container.size = size;
                }
            }
            State::Ok
        }
        Step::Nok => {
            if in_attempt {
                return State::Nok;
            }
            ext_mut(tokenizer).continuation = false;
            if is_flow {
                tokenizer.concrete = false;
            }
            if ext_mut(tokenizer).committed {
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

/// Open token and content token of a container that just started, if they
/// are the only tokens it left open, the content token last.
fn container_tokens(tokenizer: &Tokenizer) -> Option<(u16, u16)> {
    let names = tokenizer.parse_state.extension_names.borrow();
    let state = ext(tokenizer);
    if tokenizer.stack.len() != state.stack_len + 2 {
        return None;
    }
    match state.open[..] {
        [open, content]
            if names[usize::from(open)].2 == TokenKind::Token
                && names[usize::from(content)].2 == TokenKind::Content(Content::Document) =>
        {
            Some((open, content))
        }
        _ => None,
    }
}

/// Whether `event` is of a plugin token in content, such as a line prefix
/// of a plugin container.
pub(crate) fn is_in_content(names: &[(u8, &'static str, TokenKind)], event: &Event) -> bool {
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
    /// Content tokens: token, enter, and exit, and whether the content is a
    /// document.
    pub contents: Vec<(usize, usize, usize, bool)>,
    /// Events left out of values: content, and container prefixes.
    pub excluded: Vec<(usize, usize)>,
    /// Index of the last event of the match.
    pub end: usize,
}

/// Gather the tokens of the match that starts at `start`.
pub(crate) fn collect_tokens<'a>(
    events: &[Event],
    bytes: &'a [u8],
    names: &[(u8, &'static str, TokenKind)],
    start: usize,
) -> Match<'a> {
    // Name, enter, exit, whether content.
    let mut spans: Vec<(&'static str, usize, usize, bool)> = vec![];
    let mut open = vec![];
    let mut contents = vec![];
    let mut excluded = vec![];
    let mut index = start;
    let construct = names[usize::from(events[start].extension)].0;
    // The match’s own tokens in content are inside its content tokens, which
    // are skipped, so tokens in content here are another match’s, such as the
    // line prefix of an outer instance of the same construct.
    let is_own = |event: &Event| {
        let (index, _, kind) = &names[usize::from(event.extension)];
        *index == construct && *kind != TokenKind::InContent
    };

    loop {
        let event = &events[index];
        match (&event.kind, &event.name) {
            (Kind::Enter, Name::Extension) if is_own(event) => {
                let (_, name, kind) = &names[usize::from(event.extension)];
                let is_content = matches!(kind, TokenKind::Content(_));
                spans.push((name, index, index, is_content));
                open.push(spans.len() - 1);

                if let TokenKind::Content(content) = kind {
                    let enter = index;
                    index = balanced_exit(events, index);
                    contents.push((spans.len() - 1, enter, index, *content == Content::Document));
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
            // Container prefixes, also of plugin containers.
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
        str::from_utf8(slice.bytes).unwrap_or("")
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

/// Text markers: the built-in ones and those of registered constructs,
/// except line endings; empty without constructs.
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
