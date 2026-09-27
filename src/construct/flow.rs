//! The flow content type.
//!
//! **Flow** represents the sections, such as headings and code, which are
//! parsed per line.
//! An example is HTML, which has a certain starting condition (such as
//! `<script>` on its own line), then continues for a while, until an end
//! condition is found (such as `</style>`).
//! If that line with an end condition is never found, that flow goes until
//! the end.
//!
//! The constructs found in flow are:
//!
//! * [Blank line][crate::construct::blank_line]
//! * [Code (indented)][crate::construct::code_indented]
//! * [Heading (atx)][crate::construct::heading_atx]
//! * [Heading (setext)][crate::construct::heading_setext]
//! * [HTML (flow)][crate::construct::html_flow]
//! * [MDX esm][crate::construct::mdx_esm]
//! * [MDX expression (flow)][crate::construct::mdx_expression_flow]
//! * [MDX JSX (flow)][crate::construct::mdx_jsx_flow]
//! * [Raw (flow)][crate::construct::raw_flow] (code (fenced), math (flow))
//! * [Thematic break][crate::construct::thematic_break]

use crate::event::Name;
use crate::state::{Name as StateName, State};
use crate::tokenizer::Tokenizer;
use core::convert::TryFrom;

/// Start of flow.
//
/// ```markdown
/// > | ## alpha
///     ^
/// > |     bravo
///     ^
/// > | ***
///     ^
/// ```
pub fn start(tokenizer: &mut Tokenizer) -> State {
    if tokenizer.parse_state.options.flow_constructs.is_empty() {
        start_builtin(tokenizer)
    } else {
        before_construct(tokenizer, 0)
    }
}

/// Before plugin constructs, trying the one at `index` and later.
pub fn before_construct(tokenizer: &mut Tokenizer, index: u8) -> State {
    let options = tokenizer.parse_state.options;
    let mut index = usize::from(index);

    if let Some(byte) = tokenizer.current {
        while index < options.flow_constructs.len() {
            // After indentation, which the construct skips first, a marker
            // is checked there.
            if matches!(byte, b'\t' | b' ')
                || (byte != b'\n'
                    && options.flow_constructs[index].markers().contains(&byte)
                    && options.flow_constructs[index].previous(tokenizer.previous))
            {
                crate::extension::ext_mut(tokenizer).next =
                    u8::try_from(index + 1).expect("expected fewer than 256 flow constructs");
                tokenizer.attempt(
                    State::Next(StateName::FlowAfter),
                    State::Next(StateName::FlowBeforeConstruct),
                );
                let global = u8::try_from(options.text_constructs.len() + index)
                    .expect("expected fewer than 256 constructs");
                return crate::extension::start(tokenizer, global);
            }
            index += 1;
        }
    }

    start_builtin(tokenizer)
}

/// Before plugin constructs, after one did not match.
pub fn before_construct_next(tokenizer: &mut Tokenizer) -> State {
    let next = crate::extension::ext(tokenizer).next;
    before_construct(tokenizer, next)
}

/// Start of flow, at built-in constructs.
pub fn start_builtin(tokenizer: &mut Tokenizer) -> State {
    match tokenizer.current {
        Some(b'#') => {
            tokenizer.attempt(
                State::Next(StateName::FlowAfter),
                State::Next(StateName::FlowBeforeContent),
            );
            State::Retry(StateName::HeadingAtxStart)
        }
        Some(b'$' | b'`' | b'~') => {
            tokenizer.attempt(
                State::Next(StateName::FlowAfter),
                State::Next(StateName::FlowBeforeContent),
            );
            State::Retry(StateName::RawFlowStart)
        }
        // Note: `-` is also used in setext heading underline so it’s not
        // included here.
        Some(b'*' | b'_') => {
            tokenizer.attempt(
                State::Next(StateName::FlowAfter),
                State::Next(StateName::FlowBeforeContent),
            );
            State::Retry(StateName::ThematicBreakStart)
        }
        Some(b'<') => {
            tokenizer.attempt(
                State::Next(StateName::FlowAfter),
                State::Next(StateName::FlowBeforeMdxJsx),
            );
            State::Retry(StateName::HtmlFlowStart)
        }
        Some(b'e' | b'i') => {
            tokenizer.attempt(
                State::Next(StateName::FlowAfter),
                State::Next(StateName::FlowBeforeContent),
            );
            State::Retry(StateName::MdxEsmStart)
        }
        Some(b'{') => {
            tokenizer.attempt(
                State::Next(StateName::FlowAfter),
                State::Next(StateName::FlowBeforeContent),
            );
            State::Retry(StateName::MdxExpressionFlowStart)
        }
        // Actual parsing: blank line? Indented code? Indented anything?
        // Tables, setext heading underlines, definitions, and Contents are
        // particularly weird.
        _ => State::Retry(StateName::FlowBlankLineBefore),
    }
}

/// At blank line.
///
/// ```markdown
/// > | ␠␠␊
///     ^
/// ```
pub fn blank_line_before(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowBlankLineAfter),
        State::Next(StateName::FlowBeforeCodeIndented),
    );
    State::Retry(StateName::BlankLineStart)
}

/// At code (indented).
///
/// ```markdown
/// > | ␠␠␠␠a
///     ^
/// ```
pub fn before_code_indented(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeRaw),
    );
    State::Retry(StateName::CodeIndentedStart)
}

/// At raw.
///
/// ````markdown
/// > | ```
///     ^
/// ````
pub fn before_raw(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeHtml),
    );
    State::Retry(StateName::RawFlowStart)
}

/// At html (flow).
///
/// ```markdown
/// > | <a>
///     ^
/// ```
pub fn before_html(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeMdxJsx),
    );
    State::Retry(StateName::HtmlFlowStart)
}

/// At mdx jsx (flow).
///
/// ```markdown
/// > | <A />
///     ^
/// ```
pub fn before_mdx_jsx(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeHeadingAtx),
    );
    State::Retry(StateName::MdxJsxFlowStart)
}

/// At heading (atx).
///
/// ```markdown
/// > | # a
///     ^
/// ```
pub fn before_heading_atx(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeHeadingSetext),
    );
    State::Retry(StateName::HeadingAtxStart)
}

/// At heading (setext).
///
/// ```markdown
///   | a
/// > | =
///     ^
/// ```
pub fn before_heading_setext(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeThematicBreak),
    );
    State::Retry(StateName::HeadingSetextStart)
}

/// At thematic break.
///
/// ```markdown
/// > | ***
///     ^
/// ```
pub fn before_thematic_break(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeMdxExpression),
    );
    State::Retry(StateName::ThematicBreakStart)
}

/// At MDX expression (flow).
///
/// ```markdown
/// > | {Math.PI}
///     ^
/// ```
pub fn before_mdx_expression(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeGfmTable),
    );
    State::Retry(StateName::MdxExpressionFlowStart)
}

/// At GFM table.
///
/// ```markdown
/// > | | a |
///     ^
/// ```
pub fn before_gfm_table(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(
        State::Next(StateName::FlowAfter),
        State::Next(StateName::FlowBeforeContent),
    );
    State::Retry(StateName::GfmTableStart)
}

/// At content.
///
/// ```markdown
/// > | a
///     ^
/// ```
pub fn before_content(tokenizer: &mut Tokenizer) -> State {
    tokenizer.attempt(State::Next(StateName::FlowAfter), State::Nok);
    State::Retry(StateName::ContentChunkStart)
}

/// After blank line.
///
/// ```markdown
/// > | ␠␠␊
///       ^
/// ```
pub fn blank_line_after(tokenizer: &mut Tokenizer) -> State {
    match tokenizer.current {
        None => State::Ok,
        Some(b'\n') => {
            tokenizer.enter(Name::BlankLineEnding);
            tokenizer.consume();
            tokenizer.exit(Name::BlankLineEnding);
            // Feel free to interrupt.
            tokenizer.interrupt = false;
            State::Next(StateName::FlowStart)
        }
        _ => unreachable!("expected eol/eof"),
    }
}

/// After flow.
///
/// ```markdown
/// > | # a␊
///        ^
/// ```
pub fn after(tokenizer: &mut Tokenizer) -> State {
    match tokenizer.current {
        None => State::Ok,
        Some(b'\n') => {
            tokenizer.enter(Name::LineEnding);
            tokenizer.consume();
            tokenizer.exit(Name::LineEnding);
            State::Next(StateName::FlowStart)
        }
        _ => unreachable!("expected eol/eof"),
    }
}
