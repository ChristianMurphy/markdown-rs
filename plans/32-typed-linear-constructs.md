---
status: Proposed
---

# Typed, linear-time constructs (issue 32, PR 270)

## Task summary

PR 270 in wooorm/markdown-rs adds syntax extensions: a `Construct` is a state machine that the built-in tokenizer drives. Review and adopter feedback on 0906fe3 raised three points. The developer then set two goals for the next revision:

- Safe: parsing stays linear in the size of the document wherever the API can guarantee it.
- Type safe: mistakes a plugin author can make are compile errors where possible, not silent `Nok`s.

This plan revises the `Construct` API toward both goals and folds in the review points. It builds on `plans/32-plugin-prototypes.md` (D1 to D25, D45) and `plans/32-block-nested-syntax.md` (D26 to D46). It revisits D20, D22, D30, D37 and D41, whose revisit conditions now hold.

## Goals and constraints, as the developer set them

| Topic | Choice | What follows |
|---|---|---|
| Main goal | A balance of upstream merge, safety on hostile input, and adopter ergonomics | A medium-sized change is fine, if each part earns its surface |
| Threat model | Input is hostile; construct authors are trusted, like any dependency | Common patterns must be linear; a buggy construct may be slow on its own input |
| Author friction | Some, with safe defaults | A simple construct stays about as short as today |
| Cost with constructs | Up to about 5% over 0906fe3 | Allows a downcast per step and per-construct slots; with no constructs, the 1% gate stays |
| `no_std`, `forbid(unsafe_code)` | Fixed | Type erasure goes through `Box<dyn Any>` |
| MSRV | Stay at 1.62, as PR 258 declares | The typed API builds on 1.62 and needs no generic associated types; a raise to 1.65 was made and then dropped (D62) |
| Proc macros | Allowed, opt-in | An optional `markdown_derive` crate, behind a `derive` feature, with `#[derive(TokenType)]` only |
| One `Construct` trait | Keep one trait, with the place in the type | Place-specific methods exist only where valid |
| Rule broken at run time | `Nok`; an assertion naming the rule only with `test-util` | Apps' debug builds and fuzzers never panic on hostile input; authors' tests fail with a reason |
| New fields on `mdast::Custom` | Add plainly, no `#[non_exhaustive]` | Like the other mdast nodes; literals with `..Default::default()` keep compiling |
| Branches | The series is built on `ccr-30c42fe4-xadkx0`, on top of `feat/syntax-extensions` | Pushed to `feat/syntax-extensions` with `--force-with-lease` only after the developer approves it |
| Default reach in text | Unbounded; `Bytes` and `Pair` opt in | Docs and a test helper steer authors |
| Step limit | Keep 256 steps per byte | Matches stay as fuzzed on 0906fe3 |
| HTML in `to_html` | micromark-style handlers | D55 |
| HTML in the tree path | `hName` and `hProperties` like unified, with URL checks; no `hChildren` | D56 |
| Line ending before a flow match | Fix in the core | D57 |
| Delivery | Revise PR 270 fully | The first release has the final API; review starts over |

## Feedback on 0906fe3

| From | Point | Where |
|---|---|---|
| Murderlon (Codex review) | With GFM, a text construct at `=` that consumes `=` and the line ending, then fails, makes `www.b.c` a link in `"> =\n>www.b.c\n"`. Rollback should restore the line-skip state too. | D53 |
| hakanserce, gap 1 | `previous` gets the last byte of a UTF-8 sequence (`Some(169)` before `é`), so a construct cannot check for Unicode punctuation. | D52 |
| hakanserce, gap 2 | `to_html` writes a flow match as its bare source; a construct cannot choose its HTML. | D55, D56, D57 |

## Where parsing can be quadratic today

The limit of 256 steps per byte makes each match linear in its reach. The cost over all matches is what matters:

- **Text constructs: quadratic.** A failed construct is tried again at its next marker. A construct that scans to the end of the paragraph for `]]` costs O(n²) on `[[a [[a [[a …`. D22 only documents this.
- **Flow constructs: linear.** They are tried once per line, and decide on their first line (D27).
- **Containers: linear.** `continuation` runs once per container per line, and reads at most that line.
- **Content passes: a constant factor**, at most 32 (`CONTENT_MAX`).

So the guarantee needs work in text only.

## Design

### Shape, before and after

Before, in 0906fe3:

```rust
impl Construct for Mention {
    fn markers(&self) -> &[u8] { b"@" }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (0, Some(b'@')) => { t.enter("mention"); t.consume(); Step::Next(1) }
            (1, Some(b'a'..=b'z')) => {
                t.enter("mentionName"); t.consume(); t.exit("mentionName");
                t.exit("mention");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        // `tokens[1]` is `mentionName`, by position.
        Node::Custom(Custom { name: "mention".into(), value: Some(tokens[1].value.to_string()), ..Custom::default() })
    }
}
```

After, with the optional derive:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, TokenType)]
enum T {
    #[token(name = "mention")]
    Mention,
    #[token(name = "mentionName")]
    Name,
}

#[derive(Clone, Copy)]
enum S { Start, Name }

impl Construct for Mention {
    type Place = Text;
    type State = S;
    type Token = T;
    const MARKERS: &'static [u8] = b"@";
    const START: S = S::Start;
    // `REACH` defaults to `Reach::Unbounded`; this construct reads 2 bytes.
    const REACH: Reach = Reach::Bytes(2);

    fn step(&self, state: S, t: Ctx<'_, '_, Self>) -> Step<Self> {
        match (state, t.current()) {
            (S::Start, Some(b'@')) => t.enter(T::Mention).consume().next(S::Name),
            (S::Name, Some(b'a'..=b'z')) => t.enter(T::Name).consume().exit().exit().ok(),
            _ => t.nok(),
        }
    }

    fn to_mdast(&self, tokens: Vec<Token<'_, T>>) -> Node {
        let name = tokens.iter().find(|token| token.name == T::Name);
        // …
    }
}

let options = ParseOptions {
    text_constructs: vec![construct(Mention)],
    ..ParseOptions::default()
};
```

The pieces follow.

### D47. Typed states that carry data

- Question: how does a construct name its states and keep what it has seen, such as the size of an opening fence?
- Chosen: `type State: Clone + 'static`, with `const START: Self::State`. States are an enum. Data lives in variants, such as `S::Fence { size: usize }`. This replaces `memory()` and its four words.
- Storing it: the trait has associated types, so it is not object safe; this was D20's objection. `ParseOptions` stores a sealed, object-safe `ErasedConstruct` instead. `extension::construct(c)` wraps any `Construct` in it. The trait is sealed, so a plugin cannot write its own and skip the checks.
- Where states live: one slot per construct per parse, a `Box<dyn Any>` holding that construct's `Option<State>` and its attempt frames. It is made the first time the construct runs, and reused for every later match. So there is no allocation at each marker byte, which was D30's objection. A state type with no data allocates nothing. Each step does one `downcast_mut`, a `TypeId` comparison, inside the 5% budget.
- Attempts: `attempt(state, ok, nok)` carries states, so a failed attempt goes on in `nok` with the data the construct gave at the time of the attempt. The rule "a failed attempt does not undo memory" (D36) goes away.
- Containers: the first match of a container ends with `t.open_container(continuation_state)` instead of `ok()`. That state is kept with the container, in place of word 0 of `memory` in `ContainerState.size` (D40). Each later line starts `step` there. `Construct::continuation()` goes away.
- Alternatives:
  - `type State: Copy` converted to and from `u16`: no allocation, but data still needs `memory`.
  - Keep `u16` and type tokens only.
  - A state object allocated per match (D30): rejected again, for the allocation at every marker.
- Revisit if: the downcast or the slot costs more than 5% on documents with constructs.

### D48. Typed token names, with an optional derive

- Question: how does a construct name its tokens, and how does `to_mdast` find them?
- Chosen: `type Token: TokenType`, a `Copy + Eq + 'static` enum. `TokenType` has `const ALL`, `fn name(self) -> &'static str`, and `fn content(self) -> Option<ContentType>`, which defaults to `None`.
  - A content token is a property of its type, so `enter` and `enter_content` become one `enter(token)`.
  - `to_mdast` gets `Vec<Token<'_, Self::Token>>`, so it matches on the enum, not on positions or strings.
  - Names are interned once, from `ALL`, when a parse starts. Too many names becomes an error up front (`too-many-construct-tokens`), next to `too-many-constructs`. That removes the untested gap of a parse passing 65,536 names.
- The derive: a new `markdown_derive` crate, re-exported behind a `derive` feature of `markdown`. `#[derive(TokenType)]` writes `ALL`, `name` (from `#[token(name = "…")]`, or the variant name in camel case) and `content` (from `#[token(content = text)]`). Without the feature, authors write the impl by hand, in about ten lines. It uses `syn` 2 and `quote`, which build on 1.62 (D62, D63).
- Alternatives:
  - Keep `&'static str` and check names at `exit`, as 0906fe3 does.
  - A `macro_rules!` macro in the main crate: no extra crate, but harder to read in rustdoc.
  - A `states!` macro that writes micromark-style state functions into the state enum and the dispatch in `step`: rejected. A typed `match` in `step` stays the one way to write a construct, so `markdown_derive` holds only `TokenType`.

### D49. One trait, the place in the type

- Question: how do place-specific rules become compile errors without splitting the trait (D37)?
- Chosen: `type Place: Place`, one of `Text`, `Flow` or `Container`, which are zero-sized marker types. `step` gets a `Ctx<'_, '_, Self>` by value, typed by `Self::Place`, and returns `Step<Self>`, whose fields are private. A `Step` can only be made through `Ctx`:
  - Every place: `enter(token)`, `exit()`, `consume() -> Consumed`, `retry(state)`, `attempt(state, ok, nok)`, `nok()`.
  - On `Consumed`: `exit()` and `next(state)`.
  - `Text` and `Flow`: `ok()`. `Text` also has `ok()` on `Consumed`.
  - `Container`: `open_container(state)` on its first line, `ok()` on later lines.
- Registration checks the place: `text_constructs` takes `C: Construct<Place = Text>`, and so on. A construct in the wrong list does not compile.
- Rules that become compile errors:
  - `Next` comes after a `consume`; `Retry` and `Attempt` do not.
  - One `consume` per step.
  - `exit` closes the innermost open token: it takes no name.
  - After consuming, only exits, then `Next` (or `Ok` in text).
  - A container is opened only by a container, and only a container has a continuation.
  - A construct runs only in the place it was written for.
- Rules checked once, when a parse starts: a text construct has no `Document` content token; markers are not line endings, and outside text not spaces or tabs.
- Rules that stay checks during a parse, because they depend on input: every consumed byte inside a token, all tokens closed at `Ok`, an attempt closing only its own tokens, the flow and container line rules, the content rules, and character boundaries. A broken rule is `Nok`, as D22 chose. With the `test-util` feature, it also panics with the name of the rule, so an author's tests fail with a reason. Without the feature, debug builds behave like release builds, since hostile input can break these rules too.
- Alternatives:
  - Split `TextConstruct`, `FlowConstruct` and `ContainerConstruct`: also compile errors, but three traits to document, and further from micromark.
  - Keep one untyped trait and run-time checks, as D37 has.

### D50. Reach: unbounded by default, two opt-ins

- Question: how is parsing kept linear when a text construct fails and is tried again at the next marker?
- Chosen: `const REACH: Reach`, defaulting to `Reach::Unbounded`, which is today's behavior. The opt-ins:
  - `Reach::Bytes(n)`: a match reads at most `n` bytes from its start, failed attempts included; past that, it fails. For mentions, emoji shortcodes, `:name` and the like. Work is at most 256 × `n` steps per marker, so linear. `n` has no cap: the author picks the constant.
  - `Reach::Pair`: for constructs that scan to a closer. See D51.
- In flow and containers, `REACH` limits the first line, or the prefix of a continuation line. Both are linear anyway.
- The facade enforces it: a match already keeps `furthest` (`ExtensionState::furthest`). The check is one comparison next to the step budget. `STEP_MAX` stays 256.
- Steering authors:
  - The `Construct` rustdoc and the examples show both opt-ins, with the `[[a ` case.
  - A test helper, `markdown::extension::assert_linear(construct)`, behind a `test-util` feature. It parses adversarial inputs built from the construct's markers (each marker repeated, with and without spaces, and inside block quotes and lists) at several sizes. It fails if the construct steps at size 2n are more than about twice those at n. It counts steps, not time, so it is deterministic. The counter exists only with the feature.
- Alternatives:
  - `Pair` or `Bytes` as the default: linear by default, but a construct that nests would silently lose matches, or a long span would silently fail.
  - One work budget for the whole parse: whether a construct matches late in a document would depend on text before it.

### D51. Pairs: a failed scan rules out later starts

- Question: how does a construct that scans for a closer stay linear, without a length bound?
- The case: `[[a [[a [[a …` with no `]]`. Each `[[` scans to the end of the paragraph and fails.
- Chosen: with `Reach::Pair`, a failed match tells the facade that no start of this construct before the furthest byte it read can match either. The facade then skips this construct's markers up to that byte.
  - Each byte is scanned by at most one failed match of this construct, so the total is linear. This is the trick cmark uses for runs of backticks.
  - It is the author's promise, and it holds for a construct that stops at its first closer. A construct that nests, counting `[[` against `]]`, must use `Bytes`, or `[[a [[b]]` loses `[[b]]`. The rustdoc says so, and `assert_linear` has a case for it.
  - A construct that breaks the promise only loses its own matches: its markers stay text.
  - The skip is per construct and per tokenizer. Content parsed in a later pass gets a fresh tokenizer, so the skip never crosses into nested content.
- Delimiter runs (D41) are already linear, because the attention resolver pairs them. `attention_sizes` becomes `Reach::Attention(&[usize])`, accepted only with `Place = Text`. A construct is either a run or a state machine, never half of each.
- Alternatives:
  - An opener and closer given as data, with the text between them parsed first and wrapped by a resolver, like links. This cannot work for raw content like `$x$` or `{{ x }}`, and it is a second way to write a construct.
  - Skip automatically after any failure: not sound for constructs that nest.
- Revisit if: an adopter needs nesting without a length bound.

### D52. `previous` as a character

- Chosen: `fn previous(&self, previous: Option<char>) -> bool`. `'\n'` at the start of a line, and `None` at the start or after a character escape, as now.
- `util::char::before_index` already decodes the character before a byte, so no new decoding code is needed.
- It answers hakanserce's gap 1, and matches micromark.

### D53. Rolling back a failed match

- Murderlon's case: `"> =\n>www.b.c\n"` with GFM and a text construct at `=` that consumes `=` and the line ending, then fails. `www.b.c` becomes a link. Without the construct, it is text.
- Cause, found by tracing `previous` at the autolink check: not the rollback. When the tokenizer skips the container prefix of a later line, `move_to` moves over its bytes one at a time, and each becomes `previous`, so `www` sees `>`. A replay after a failed attempt reaches the next chunk without moving over the prefix, so it sees the line ending. 1.0.0 is inconsistent on its own: `> a\n>www.b.c` gives no link while `> a\n> www.b.c` and `>www.b.c` do, and micromark and cmark-gfm see a line ending there.
- Chosen: `move_to` keeps `previous`, a fix of 1.0.0 as its own upstream PR, stacked under PR 270 like PRs 230 and 258.
- Test: a differential test in the text constructs commit. A construct that consumes a random number of bytes, line endings included, then fails, against no construct, over random documents of lines with container prefixes, with three option sets. It found 10 differences in 180,000 runs before the fix, and none in 720,000 after.

### D54. Errors for authors

- A broken rule during a parse: `Nok`, and a panic naming the rule only with `test-util` (D49).
- A bad registration, such as a text construct with a `Document` token or a marker that is a line ending: an error from the parse, before any input is read.
- A flow construct failing after its first line stays the parse error `flow-construct-late-failure`.

### D55. HTML handlers in `to_html`, like micromark

- Question: how does a construct choose its HTML in `to_html`, which compiles events without a tree?
- Chosen: micromark's `htmlExtension`: hooks on the enter and exit of the construct's tokens.
  - `fn html(&self, event: HtmlEvent<'_, Self::Token>, w: &mut HtmlWriter)`. `HtmlEvent` is `Enter(token)` or `Exit(token)`, with the token's own text and position, like the `Token` that `to_mdast` gets.
  - The core compiles content tokens between their enter and exit, so a handler never sees child HTML. It writes only the markup around content, such as `<span class="mention">` at enter and `</span>` at exit.
  - The default writes the match's own text, escaped, which is today's behavior.
- `HtmlWriter`, kept small:
  - `text(&str)`: escaped text.
  - `tag(&str)`: raw markup. It ignores `allow_dangerous_html`, as a micromark extension's `this.tag` does; authors are trusted (threat model).
  - `url(&str, protocols)`: an escaped URL, checked like link URLs unless `allow_dangerous_protocol`, the same as `to_html` checks a link's `href`.
  - `line_ending_if_needed()`, and `buffer()` and `resume() -> String` to capture content, for constructs that need it as an attribute, like micromark's `this.buffer` and `this.resume`.
  - `options()`: the compile options.
- Why handlers and not something smaller: the developer chose the most flexible option, closest to micromark. A shape enum or an element per token were the smaller options weighed.
- A cost to document: a construct that renders through both paths writes its HTML twice, once here and once in the tree path (D56). This is also true of micromark and mdast-util-to-hast in JavaScript.
- Alternatives:
  - Processor only: no core API, and plain `to_html` keeps writing source.
  - A block shape enum (`Source`, `Content`, `Paragraph`, `Drop`): small, but too coarse for inline elements.
  - An element per token, with the core writing tags: type safe and always escaped, but less flexible.

### D56. `hName` and `hProperties` in the tree path

- Question: can a construct choose its HTML in `Processor` without writing a hast handler?
- Chosen, like mdast-util-to-hast's `data.hName` and `data.hProperties`:
  - `Custom` gets `h_name: Option<String>`. With it, the node becomes that element instead of the text or `div` fallback.
  - `Custom.attributes`, ignored today, becomes the element's properties when `h_name` is set.
  - URL properties (`href`, `src`, `cite`, `longdesc`, as in hast-util-sanitize) go through the protocol check unless `allow_dangerous_protocol`. Other properties are the plugin's responsibility, as in unified.
  - As built: the check lives in `hast_util_to_html`, the only place that has `allow_dangerous_protocol`, so it applies to every element, not only custom ones. `cite` and `longDesc` allow `http` and `https`, like `src`.
  - As built: `attributes` keep HTML names, and become hast property names: `class` becomes `className`, split at whitespace; `for`, `http-equiv` and `accept-charset` get their hast names; `data-*` and `aria-*` become camel case. The serializer turns them back.
  - As built: with `h_name`, a node with a value becomes the element with the value as its text, as `applyData` does in mdast-util-to-hast. In JSON, the field is `hName`.
  - No `hChildren`. `markdown::mdast::Custom` cannot hold hast children, since `hast` depends on `markdown`. A construct that needs custom children registers a hast handler. Weighed and rejected: an `h_children` of raw HTML, written only with `allow_dangerous_html`, and a side table in `markdown_processor`.
- A handler for the node's name still wins over these fields.

### D57. A line ending before a flow match in `to_html`

- A flow match is a block, so `to_html` calls `line_ending_if_needed` before it, as it does before other blocks.
- This fixes the known gap in PR 270's description. `to_html` output changes only where a flow construct matches.

### D58. Documentation

- Question: how do authors learn the API, choose between its options, and look up exact behavior?
- Chosen: rustdoc only, in layers, all doctested, so no snippet can fall behind the code.
  - Reference: each public item gets one paragraph and an example. The rules of `Construct` are split into three groups: enforced by types (each with a `compile_fail` example), checked while parsing (`Nok`, and a panic with `test-util`), and advice.
  - Tutorial: a doc-only module, `markdown::extension::guide`, like clap's `_tutorial`. It builds one mention construct in steps: tokens, states, `to_mdast`, `REACH`, HTML, then `assert_linear`.
  - Recipes: doc-only modules under `guide`, one per pattern, each a complete construct with tests. They are general, but cover the patterns of the first adopter (CompMD, see below):
    1. A mention: `Reach::Bytes`, and `previous` as a `char` to start only after white space or punctuation.
    2. A raw span between delimiters, like `{{ path }}`: `Reach::Pair`, one line, an escape, and how code spans take priority.
    3. A span with flanking rules, like `$x$`: `Reach::Pair`, a closer that needs a non-space before it and no digit after it, and stopping at a delimiter that has white space before it.
    4. An inline directive with a label that holds markdown and an optional suffix right after it, like `:name[label]{attrs}`: a text content token, and an attempt for the suffix. `Reach::Bytes(999)` with bracket counting, as D61 says.
    5. A delimiter run, like `==mark==`: `Reach::Attention`.
    6. A one-line block, like `:: name args`: a flow construct that interrupts a paragraph, is never a lazy line, works in block quotes and lists, and has an escape.
    7. A fenced container with a markdown body, like `::: name` and `:::`: a flow construct with document content.
    8. A prefix container, like `!!! note`: `open_container` and its continuation state.
    9. Rendering three ways: an HTML handler in `to_html`, `h_name` with properties, and a hast handler, for the same node.
    10. A transform that changes a node after parsing, such as a span alone in its paragraph becoming a block, with `Processor::add_mdast_transform`.
    11. Testing a construct: `assert_linear`, rule panics with `test-util`, and adversarial input.
  - Choosing: two short tables at the top of `guide`. "Which reach?": `Bytes` when the length is bounded, `Pair` when the construct stops at its first closer, `Attention` for runs, `Unbounded` with the risk stated. "Which HTML route?": a handler for `to_html`, `h_name` for a plain element in `Processor`, and a hast handler for anything else.
  - Background, in the `extension` module docs: why linear time and why typed; a table from micromark to markdown-rs (`effects.consume` to `consume()`, `effects.attempt` to `attempt`, `containerState` to the container state, `htmlExtension` to `html`); and the threat model.
  - The readme: the Extensions section links to `guide`. The Security section says that construct authors are trusted, that `Reach::Unbounded` can be quadratic on hostile input, and that `HtmlWriter::tag` ignores `allow_dangerous_html`.
  - A glossary at the top of `guide` defines marker, match, place, token, content, state and reach. The docs use only those words for those ideas.
  - Prose follows the same plain-language rules and Vale check as the plans.
- Alternatives:
  - `examples/` with runnable plugins: real code, but the explanation lives in comments.
  - A `docs/` folder in Markdown: easy to read on GitHub, but snippets are not compiled, and wooorm's projects do not use one.
- Docs ship with the code: each commit in "Order of work" brings the reference, tutorial and recipe parts for what it adds.

### D59. Stability

- Chosen: normal semver from the first release. A breaking change to `extension` is a major version.
- So this revision is the last cheap chance to change the API. Before release, every public item is checked:
  - Enums that may grow are `#[non_exhaustive]`: `Reach`, `ContentType`, `HtmlEvent`, the place markers' sealed trait.
  - Structs with private fields have constructors: `Step`, `Ctx`, `Consumed`, `HtmlWriter`.
  - Sealed traits stay sealed: `ErasedConstruct` and `Place`.
  - Nothing public exists only for tests, apart from what `test-util` adds.
- Alternatives: say in the docs that `extension` may change in minor releases; or gate it behind an `unstable-extensions` feature.

### D60. Telling adopters

- Chosen: one message on PR 270 when the revision is pushed, in reply to hakanserce, drafted for the developer to post.
- Contents:
  - Which of their gaps the revision closes: `previous` as a `char` (gap 1), and HTML handlers, `h_name` and the line ending before a flow match (gap 2).
  - A table from 0906fe3 to the new API: `u16` states to a state enum; `memory()` to data in states; `&'static str` names to `TokenType`; `enter_content` to a token's content; `continuation()` to `open_container`; `attention_sizes` to `Reach::Attention`; `Step::Next` to `consume().next(…)`; `ConstructTokenizer` to `Ctx`.
  - Recommended practice: declare a reach, run `assert_linear` in tests, render with handlers or `h_name` rather than raw `tag` writes, and use `Processor` where the tree path is already in use.
  - Links to the recipes that match their patterns: 2 and 3 for interpolation and math, 4 for inline components, 6 for component lines, 9 and 10 for rendering and block interpolation.
  - The stability promise from D59.
  - Three questions about CompMD's syntax, so the API serves formats like theirs:
    1. Component lines are close to generic directives (micromark-extension-directive, MDC). Why named closers (`::: /card`) and one-line forms instead of directive fences: the plain-renderer contract (§14), parsing a line at a time without lookahead, or something else?
    2. Was MDX ever an option, for example with a restricted expression language, or did running code in documents and the changes to CommonMark rule it out?
    3. The fork emits raw nodes, and the processor builds containers and slots after parsing. With constructs, the parser could build them. Would they rather keep that step in their processor?
  - What CompMD's docs already suggest, so the questions build on it: MDX runs code and has no plain-renderer contract, and directives (MDC, Comark) have "no types, no spec and no degradation contract" (`existing-specs-comparison.md`). The spec says "Processors don't add syntax" (`compmd-spec-v1.md`, §2).
- The recipes stay general. CompMD's source (github.com/comp-markdown/compmd, at 37a0e06) was read to find the patterns, not ported.
- The fork itself (comp-markdown/markdown-rs, branch `compmd`) was not read: it is a submodule. Its `ParseOptions.definitions`, which seeds definitions when a fallback or a slice is parsed alone, is out of scope. With PR 270, a label is a content token in the same parse and already sees every definition (`src/parser.rs:93-101`). Seeding helps only parsing a slice alone, for incremental editing, and waits until that need is raised upstream.

### D61. Labels that nest, as in directives

- Question: a directive label, `:name[label]{attrs}`, matches its brackets and holds markdown. `Pair` is not sound for it: in `:a[ :b[x]`, the outer start fails but the inner one should match. How does it stay linear?
- Chosen: `Reach::Bytes(999)` with bracket counting, as micromark-extension-directive caps a label at `linkReferenceSizeMax`, the same 999 that CommonMark sets for link labels. The recipe also shows how to skip backtick runs inside the cap, for authors whose labels must let code spans bind tighter.
- Later, if an adopter needs labels of any length: `Reach::Label`, where the construct matches only its opener, the core pairs it with `]` in the same stack as link and image openers, and the construct then reads its suffix. `Reach` is `#[non_exhaustive]` (D59), so this can come in a minor release.
- Alternatives:
  - `Reach::Label` in PR 270: any length, and code spans bind tighter by design, but large work in link pairing, the most delicate part of the parser and one that runs without constructs.
  - `Pair`, documenting that `:a[ :b[x]` loses `:b[x]`.
  - `Unbounded`: correct output, but quadratic on `:a[` repeated.

### D62. The minimum Rust version stays 1.62

- The plan raised it to 1.65 for generic associated types and `let`-`else`, on the expectation that the typed API needed them. Built, it does not: the text constructs commit builds on 1.62, and nothing uses generic associated types. `let`-`else` came only from clippy, which suggests it once the minimum is 1.65.
- Chosen: no raise. PR 258's 1.62 stays, and the `let`-`else` rewrites go back to `match`. If a later step needs a newer compiler, it raises the minimum with that reason.

### As built: text constructs

- `markers`, `start` and `reach` are methods, not associated consts, so a construct can be configured at run time, as the tests' constructs are.
- `Consumed` can `enter` as well as `exit`: micromark constructs often consume a marker and enter the next token in one step. After a line ending, that is the rule `enter-after-line-ending`.
- Rule names, which `test-util` panics with: `consume-outside-token`, `token-after-match`, `empty-token`, `exit-without-token`, `enter-after-line-ending`, `ok-after-line-ending`, `ok-without-bytes`, `ok-with-open-tokens`, `token-inside-character`. Retrying 256 times in a row and reading past the reach are limits, not rules: they do not match, and do not panic.
- `Reach::Bytes(n)`: a step at a byte past `n` fails; a step at the end does not, as there is nothing to read.
- `previous` is the character before: ASCII from the tokenizer's `previous`, which covers line endings and the virtual spaces of tabs, and other characters decoded from the bytes.

### As built: the later commits

- `TokenType::content` replaces `enter_content`: a token is content wherever it is entered.
- Flow: `Flow` place. `Consumed::ok` is not there for flow, so a flow construct cannot end after a consumed byte. Rules: `ok-before-line-end`, `exit-after-line-ending`, `line-ending-in-attempt`. A match that ends at a limit after its first line goes through the same path as any failure there, so it is `flow-construct-late-failure`; the first port returned early and missed that, and a test now covers it.
- Containers: `Container` place. A container ends its first line, and each later line it continues on, with `container(state)`, which only a `Container` has; `state` checks the next line and is kept with the container, replacing `continuation()` and word 0 of `memory`. Rules: `line-ending-in-container`, `consume-in-container-content`, `container-tokens`, `ok-in-container`, `container-in-attempt`.
- Content kinds by place are rules checked when a token is entered: `document-content-in-text` and `text-content-in-container`. A check of each token type before parsing rejected plugins that share one token enum across places, so the developer chose the rules.
- Delimiter runs: `Reach::Attention(sizes)` and the token type `RunToken`. Parsing returns `attention-construct` before it starts for a run whose `Token` is not `RunToken`, or outside text.
- Attempts: `attempt(state, ok, nok)` with typed states; `nok` goes on with the data given when the attempt started.
- Limits (the step budget, the reach, the depth of content and of attempts) end a match without a panic; only broken rules panic with `test-util`.

### D63. The derive comes after the delimiter runs

- Question: where in the series does the derive go? The plan put it right after text constructs, but `TokenType::content` arrives with text content, and `ContentType::Document` with document content.
- Chosen: one commit after the delimiter runs, before the `Processor` methods, written once against the final `TokenType`. `markdown` depends on `markdown_derive` at an exact version, so the trait and the derive release together. The earlier commits keep hand-written impls, which stay the way without the `derive` feature.
- Also chosen: a token missing from a hand-written `ALL` breaks the rule `token-not-in-all`, in the text constructs commit, instead of an `expect` that panicked in every build.
- The developer weighed it for the long term: once PR 270 lands, the place of a commit in the series matters little; what lasts is a trait settled before the first release, and a derive that ships in lockstep with it.
- Alternatives:
  - Right after text constructs, as planned, with `content = text` added in the content commit and `content = document` in the document commit: three commits change the derive, and no one releases the states between them.
  - A separate PR after PR 270: a smaller PR 270, but safe only if the trait is frozen there, since a change for the derive later would break released impls.
  - A `macro_rules!` macro in `markdown`: no second crate to version and no `syn`, and it also lists variants once, but it reads less like Rust, and attributes per variant are clumsier.
- `syn` 2.0.106, `quote` 1.0.41, and `proc-macro2` 1.0.103 build on 1.62, and a crate that derives `TokenType` builds and runs there, so `markdown_derive` declares 1.62, like `markdown`. The 1.62 check copies `markdown_derive` too.
- `#[token(crate = "...")]` on the enum gives the path to `markdown` for a renamed dependency, as serde's `crate` attribute does.

### D64. `assert_linear` takes `ParseOptions`

- Question: D50 gave `assert_linear` one construct. What should it take?
- Chosen: `assert_linear(&ParseOptions)`. It checks every construct in the options together, with the options a plugin really uses: GFM, MDX, constructs in all three places, and constructs that interact. One construct is `ParseOptions { text_constructs: vec![construct(x)], ..Default::default() }`.
- As built:
  - Documents come from the ASCII markers of all constructs, each alone and all together: a marker, twice, and before a letter, repeated with nothing, spaces, or line endings between, and in a block quote and in a list item, at 128, 256, and 512 repeats.
  - It panics if a doubling more than triples the steps, plus a slack of 1,024 steps for small counts. A linear construct doubles; a quadratic one about quadruples.
  - Steps are counted in `ParseState`, only with `test-util`. `parser::parse` is split into a check of the options, a new state, and the passes of the tokenizer, so the count is read when parsing returns an error, such as `flow-construct-late-failure`.
  - The skip of `Reach::Pair` starts at the furthest byte a failed match read: a start at that byte is still tried, since the failure may come from it.
- Alternatives:
  - One construct, as D50 had it: shortest to call, but blind to interactions between a plugin's constructs and with GFM or MDX.
  - Both, as `assert_linear` and `assert_linear_with`: two functions to document and keep in step.

### D65. Text events and URL kinds for HTML handlers

- Question: D55 gave handlers `Enter(token)` and `Exit(token)`. Who writes the bytes a step consumed into a token, such as the `@` and the name of a mention, and how does a handler say which protocols a URL may have?
- Chosen: a third event, `HtmlEvent::Text(token, value)`, for the bytes in a token outside the tokens in it, without container prefixes, with the innermost open token. The default handler writes it escaped, so a construct without a handler is written as before; a handler writes or drops it.
- Chosen: `HtmlWriter::url(value, Url::Href | Url::Src)`, the two lists `to_html` uses for links and images, with `allow_any_img_src` for `Src`. The protocol lists stay hidden; a construct that needs others checks the URL itself.
- As built:
  - A match becomes steps when it starts, which the core gives the handler up to each content token, and after it. The core writes content between the enter and exit of its token, and leaves out tokens inside content, as `to_mdast` does.
  - `tag` is left out in the `alt` of an image, like built-in markup and micromark's `this.tag`.
  - `resume` without a buffer panics, naming the mistake.
  - D57 covers containers too: a block match gets a line ending, unless one comes before it. Without it, a flow match ran on after `<blockquote>`, `<li>`, and `</ul>`; elsewhere the line ending before a block already came from the source.
- Alternatives:
  - `Enter` and `Exit` only, with the core writing the text: a handler could not drop markers, so a wiki link would show its brackets.
  - `Enter` and `Exit` only, with handlers writing `token.value`: the value of a token includes the tokens in it, so writing it at each level repeats text, and the default could not write today's output.
  - A protocol list in `url`: flexible, but it makes `SAFE_PROTOCOL_HREF` and `SAFE_PROTOCOL_SRC` public API.

### D66. HTML handlers get the tokens of the match

- Question: porting the wiki link plugin found that a handler, `&self` with one event at a time, cannot use a later token at an earlier event. A wiki link writes `<a href>` at its start, then its alias or, without one, its target, and learns whether it has an alias only later. A directive's `{attrs}` come after its label. micromark handlers have `this.setData` and `getData` for this.
- Chosen: `fn html(&self, event, tokens: &[Token<T>], w)`. Every call gets all the tokens of the match, as `to_mdast` gets them but without children: read-only, with no state for an author to manage across events.
- As built: the typed tokens are made from the match's tokens for each call, borrowing their values, which costs one `Vec` per event, and only where a construct matches.
- Alternatives:
  - Per-match state, as an associated `type HtmlState: Default` passed as `&mut`, like `setData`: the most flexible, but one more associated type on every construct, and state to reason about across events.
  - No change: constructs that need later tokens render only through `Processor`, and as source in `to_html`.
- Also found in the port, and fixed in the docs of `Reach::Pair`: the promise also fails for a construct that fails at a marker inside its opener. If a target cannot hold `[`, the match at the first `[` of `[[[a]]` fails at the third, so the match at the second is skipped. A construct that stops at each marker is linear without `Pair`, as the wiki link is; a test shows the lost match and the linearity.

### D67. A container can hold a title on its first line

- Question: porting admonitions (`!!! warning "Custom *title*"` and an indented body) found that a container could not hold a markdown title: `text-content-in-container` forbade text content in a container, and `consume-in-container-content` forbade consuming while any content was open.
- Chosen: a container can enter text content on its first line, before its document content. `consume-in-container-content` applies to document content only, `container-tokens` needs the content open at `container()` to be document content, and `text-content-in-container` stays for later lines, whose tokens are prefixes in the document.
- Evidence: the admonition port passes its tests (nesting, lists, lazy lines, setext headings, tabs, linearity, the same HTML in both paths), and 100,000 fuzzed documents gave no broken rule, and the same HTML in both paths but where 1.0.0 differs, at unclosed code at the end of a container, which PR 268 fixes.
- Alternatives:
  - Keep the rules: the title would be a plain token that `to_mdast` parses with `markdown::to_mdast` apart, without positions or plugin syntax, and `to_html` writes as text.
  - Allow text content on any line: nothing needs it, and text content in the prefix of a later line is untested.

### D68. What the directive port changed

- Question: porting directives against micromark-extension-directive's 255 test cases found three limits of PR 270. The developer chose each fix.
- `classify` is exported: `markdown::extension::{classify, CharacterKind}`, next to `Ctx::current_char` and `previous: Option<char>`, like micromark's `classifyCharacter`. A directive's name is defined by it; without it, a plugin misread non-ASCII punctuation (`:꙳` became a directive). In the text constructs commit.
- Text content can start or end with blank lines, or be only whitespace, but not have a blank line between lines with other bytes. In the text content commit.
  - `blank-line-in-content` guards a real panic: built-in text code (`partial_space_or_tab_eol`) asserts that text never has a blank line, and never ends right after a line ending, as a paragraph's text never does.
  - So the core leaves a final line ending of text content, and any blank lines after it, out of the text it parses, as the last line ending of a paragraph is not its text. In flow, where line endings of content are line endings of flow, the content ends before them and they stay line endings. A failed attempt restores the new state.
  - Evidence: 600,000 random documents with a text construct and 200,000 with a flow construct whose content starts or ends with blank lines set off no assertion, where the first try at this, with line endings in chunks of their own, set off 42.
  - Three of micromark's cases end a label with a line ending, which it keeps; here that line ending is left out, and a backslash before it is no hard break, as at the end of a paragraph. The port's test lists them.
- A token in document content, such as a line prefix that strips the indent of an opening fence, can start or end inside a tab, as the indent of fenced code can, and as container prefixes already could. The rest of the tab is content, as after `>`. In the document content commit.
- Alternatives the developer weighed: exporting `classify` at the crate root, or not at all; keeping `blank-line-in-content` with the port working around it, or forbidding content that ends with a line ending; letting any token of a flow construct split a tab, or none.

### D69. `previous` at the start of text

- Found while writing recipe 2 of the guide. The docs said `previous` is `'\n'` at the start of a line, and `None` "at the start", which reads as the start of the document. The code gives `None` at the start of text, the first line of each paragraph or heading, in containers too, and `'\n'` on its later lines, as micromark does: its text tokenizer starts with `previous` as `null`. After a character escape it is `None` too, so a construct cannot tell an escape from the start of text. An escaped marker is never tried, so a construct rarely needs to.
- Chosen: keep the behavior, which matches micromark, and fix the docs in the text constructs, flow, and containers commits. The text constructs commit adds a test: `None` at the start of a paragraph, a heading, a block quote, and a list item, and not on a later line.
- Alternative: `'\n'` at the start of text too, so every line start looks the same. It would differ from micromark, and `None` would then mean an escape or the start of the document.

### As built: the public API check

- Each public item of `extension` was checked against D59. Changed: `Debug` for `Step`, `Ctx`, and `Consumed`, which shows no fields, and `#[non_exhaustive]` on `RunToken`, so runs can gain a token in a minor release.
- Already held: `Reach`, `ContentType`, `HtmlEvent`, `Url`, and `Token` are `#[non_exhaustive]`; `Place` is sealed, and `PlaceKind` is hidden; `Step`, `Ctx`, `Consumed`, and `HtmlWriter` have private fields, so they can gain fields; only `assert_linear` exists for tests, behind `test-util`.

### As built: the guide

- One docs commit, last in PR 270, as the developer chose, rather than docs in each commit.
- `extension::guide`: the words, the tables for place, reach, and HTML route, the table from micromark, and security. `extension::guide::tutorial` builds a mention in three steps. `extension::guide::recipes` has recipes 1 to 8 and 11; `markdown_processor::guide` has 9 and 10. The readme links the guide, and its Security section says what D58 asks.
- Every example is a doctest. Recipe 11 runs with and without `test-util`, and recipes 7, 8, and 11 pass with `test-util`, so they break no rule.
- Differences from D58:
  - Recipe 2 needs no `previous` check for `\{{`: the escape leaves one `{`, which cannot open (D69).
  - Recipe 4 skips backtick runs with an attempt, as D61 says. A run that fails has a size no later run has, so few fail within 999 bytes. On `:x[` repeated, time doubles when input doubles, and is about 1.4 seconds for 48 KB in a release build; the recipe says the cap is also the cost.
  - Recipe 10 turns an image alone in its paragraph into a `<figure>`: a transform works the same on a construct's node, matched by its `name`.

## Not changed

- `Processor::add_text_construct`, `add_flow_construct` and `add_document_construct` keep their names. They take a `Construct` of the matching place and wrap it with `construct()`.
- Positions, content tokens, nested documents and the core fixes from D26 to D44.
- `STEP_MAX`, `ATTEMPT_MAX` and `CONTENT_MAX`.

## Tests

- Every test of a construct in 0906fe3, ported to the new API with the same input and output, where its rule did not become a compile error.
- For each rule that becomes a compile error, a `compile_fail` doctest.
- For each rule that stays a check, a test in release mode that it is `Nok`, and one in debug mode that the assertion names it.
- `assert_linear` on each `Reach`, on the fork's plugins, and on a construct that is quadratic on purpose, which it must catch.
- `Reach::Pair` against a construct that nests, which documents the lost match.
- D52: `previous` before `—`, `é`, and an escape.
- D53: Murderlon's case, before and after.
- D55: a mention as `<span>`, a wiki link with a checked `href`, a component line as `<p>`, and `buffer` and `resume`. Each case is also run through `Processor` with D56's fields, to show both paths.
- D56: `h_name` with properties, a `javascript:` URL with and without `allow_dangerous_protocol`, and a handler that overrides the fields.
- D57: a flow match between paragraphs, in a block quote, and in a container.
- The fuzz targets in PR 270's description, against the new API: containers at `|` like block quotes, a run at `~` like GFM strikethrough, and random step scripts. Debug assertions on, with no panic or hang.

## Performance

- Without constructs: the same instructions as on PR 262, measured with callgrind as before.
- With constructs: within about 5% of 0906fe3, on the mention, wiki link and fenced container from the tests, on short documents and on `readme.md`.
- `Reach::Pair` on `[[a ` × n: linear steps, measured by `assert_linear`.

## Order of work, as a revision of PR 270

PR 270 is revised in place, so the first release has the final API. The series is built on `ccr-30c42fe4-xadkx0`, on top of `feat/syntax-extensions`, and reaches PR 270 only after the developer approves it. The commit series keeps its shape: each commit adds one feature, with the typed API from the start, and passes the checks on its own.

1. Stack on PR 230, which fixes CI on current stable, PR 258, which declares 1.62, and the fix for `previous` across container prefixes (D53), like PRs 259 to 262. There is no version raise (D62).
2. hast, the custom mdast node with `h_name` (D56), mdast_util_to_hast, and markdown_processor, as in PRs 259 to 262.
3. Text constructs: `Construct`, `Place`, typed states and tokens, `Ctx`, `Reach`, `previous` as a `char`, the outer rollback (D47 to D54).
4. The optional `markdown_derive` crate, placed after the delimiter runs and the pairs (D63).
5. Text content, attempts, flow constructs (with D57), document content, containers (with `open_container`), and delimiter runs as `Reach::Attention`, as PR 270's commits are now.
6. `Reach::Pair` and `assert_linear`, after the delimiter runs and before the derive (D64).
7. HTML handlers (D55, D57, D65), after the derive.
8. The `Processor` methods.
9. Port the fork's plugins (wiki link, admonition, mark, directives) as a check on the API, and rewrite PR 270's description.
10. Check each public item against D59, and draft the message to adopters (D60).

## How the work follows this plan

- One step of "Order of work" per commit. The commit body cites the D-numbers it carries out.
- Tests first: for each step, the cases from "Tests" are written before the code, including `compile_fail` doctests and step counts.
- A deviation stops the work. If the code forces a change to a decision, such as a borrow that cannot work or a cost over budget, the developer decides, and the answer is recorded here as a new D-number before the work goes on.
- Checks on every commit: `cargo fmt --check`; all tests; clippy diagnostics as on `main`; `cargo check` on 1.62, with default and all features; the `no_std` build for `thumbv7em-none-eabihf`; output equal to PR 262 without constructs on the test suite and the generated documents. Callgrind (1% without constructs, 5% with) and the fuzz targets run on the commits that change parsing.
- A review of each commit against this plan before the developer sees it.
- Commit messages name issues and PRs without `#`, such as `Refs: issue 32` or `PR 270`, so GitHub does not autolink them and add them to a timeline on every push.
- Commits going upstream are authored by the developer, in the Problem, Goal, Changes, Notes and `Refs` style of PR 270, with no co-author, sign-off, or session trailers, and no tool or model names. PR 270's description keeps its one disclosure line, as the developer wrote it.

### Environment

- CI resolves a fresh lockfile on the latest stable. On 1.0.0 that breaks the dev-dependencies (`swc_common` 8.1.1 imports `serde::__private`, which `serde` 1.0.220 removed) and clippy (`manual_assert_eq` in 1.98 and `redundant_else` in 1.99 are errors under `deny(clippy::pedantic)`). PR 230 fixes both, with `swc_core` 32, and PR 270's copy of it also clears the warnings clippy 1.99 reports. Each commit runs the CI job as `main.yml` does: `cargo fmt --check`, then `cargo clippy --all-features --all-targets --workspace` with no diagnostics, then `cargo test --all-features --workspace`, on a fresh lockfile.
- `cargo +1.62` cannot resolve the workspace: newer dev-dependencies need a newer cargo. The 1.62 check copies the `markdown` library, and `markdown_derive` for its `derive` feature, without dev-dependencies, resolves it with `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`, pins `unicode-id` 0.3.6 (and `serde` 1.0.180 for `--all-features`), and writes the lockfile as version 3. The four new crates are checked the same way, as a workspace of their own.
- `#![deny(clippy::pedantic)]` stops clippy at the library's errors, so test code is linted only once the library has none. With no diagnostics at all, that holds on every commit.
- The four crates of the tree path declare no `rust-version`; they built on 1.65 when checked. `markdown_derive` declares 1.62, since the `derive` feature of `markdown` uses it (D63).

### Progress

| Step | Decisions | Commit | Tests | Status |
|---|---|---|---|---|
| 1. Base: PRs 230 and 258, the prefix fix | D53, D62 | the CI fix, the 1.62 minimum, the prefix fix | CI job clean on each commit; the prefix fix's tests fail on 1.0.0 and pass with it | Done |
| 2. hast, mdast, processor crates | D56 | the hast, custom node, mdast_util_to_hast and processor commits | `cite` and `longDesc` checks; `h_name` with `Debug` and serde; elements, properties and value with `h_name`; a handler before `h_name`; through `Processor`, with a `javascript:` `href` emptied | Done |
| 3. Text constructs | D47 to D54 | the text constructs commit | 28 tests in both modes (with and without `test-util`), `compile_fail` examples, the differential test; CI job clean; builds on 1.62; callgrind −0.12% to +0.64% without constructs | Done |
| 4. `markdown_derive` | D48, D63 | the derive commit, after the delimiter runs; the `token-not-in-all` rule in the text constructs commit | the derived impl, a parse with derived content tokens, a renamed crate, an enum without variants, a `compile_fail` example for each of six errors, each checked to fail with its own message; the rule in both modes, failing without the fix; builds and runs on 1.62 | Done |
| 5. Content, attempts, flow, containers, runs | D51 | text content, attempts, flow constructs, document content, containers, delimiter runs | 110 extension tests in both modes at the tip; CI job clean at each commit; builds on 1.62; output equal to each parent on 600,000 random documents; callgrind within +0.27% of each parent; `\|` containers like `>` on 12,000,000 fuzzed documents; runs at `~` like strikethrough on 2,400,000 | Done |
| 6. `Reach::Pair`, `assert_linear` | D50, D51, D64 | the pairs commit, after the delimiter runs | the same trees with `Pair` as unbounded for a construct that stops at its first closer, on fixed inputs and 2,000 random documents; a nesting construct loses a match; `assert_linear` passes pairs, bytes, runs, flow, containers, GFM, and all together, and catches an unbounded construct alone, nesting, and among others; with the skip off, it fails `Pair`; output equal to the parent on 600,000 random documents; callgrind at most +0.021% | Done |
| 7. HTML handlers | D55, D57, D65 | the HTML handlers commit, after the derive | a mention as a `span`, in a block quote and an image `alt`; a link with a checked URL; content in a buffer; a match in content; flow matches at the start of a block quote, in list items, after a list, and in a container, which fail without D57; `resume` without a buffer; the same HTML through `to_html` and `Processor`; output equal to the parent without constructs on 600,000 documents, and with constructs on 200,000 but for line endings; callgrind −0.07% to +0.002% | Done |
| 8. `Processor` methods | | the `Processor` commit | each method takes a construct of its place; tests through `process` | Done |
| 9. Plugin ports, description | D66, D67, D68 | `feat/plugin-ports`, outside PR 270: mark, wiki_link, gfm_alert, admonition, directive | each through `to_html` and `Processor`, with the same HTML, and `assert_linear`; 100,000 fuzzed admonition documents; 252 of micromark-extension-directive's 255 cases, and 3 listed differences | Ports done; wiki_link found D66, admonition D67, directive D68; description to do |
| 10. Public API check | D59, D69 | fixes in the text constructs and delimiter runs commits (D59), and the text constructs, flow, and containers commits (D69) | `Debug` of `Step`, `Ctx`, and `Consumed`; `previous` at the start of text; CI job clean on each commit; builds on 1.62 | Done |
| 11. Guide | D58 | the docs commit, last in PR 270 | every recipe and tutorial step as a doctest, in both modes; rustdoc and clippy with no warnings; builds on 1.62 | Done |
| 12. Message to adopters | D60 | | | To do |

## Settled questions

- The other stacked PRs: PR 230's branch carries the same CI fix as PR 270's copy. PRs 258 to 262 stay as they are, and PR 270 is rebased once they land.

- `hChildren`: left out (D56).
- MSRV: stays 1.62 (D62).
- Rule assertions: only with `test-util` (D49).
- `mdast::Custom`: `h_name` added plainly, without `#[non_exhaustive]`.
- Names: `markdown::extension::assert_linear` behind a `test-util` feature, and `markdown_derive` behind a `derive` feature.
- No `states!` macro (D48).
- Seeded definitions: not now (D60).

## Found on the way

- Definition lookups scan a `Vec<String>` once per reference (`src/construct/label_end.rs:268` and `:280`), so a document with d definitions and r references costs O(r × d) comparisons. This is in 1.0.0 too.
  - Already fixed upstream in PR 244 ("Improve performance of definition lookups", cf1c34b, part of the issue 227 series). It sorts and deduplicates definitions once per pass in `parser.rs`, uses `binary_search` in `label_end.rs`, and in `to_html.rs` sorts definitions and indexes footnote calls in a `BTreeMap` past 16. No new PR is needed.
  - It collides with PR 270: PR 270's loop for documents inside constructs (`src/parser.rs`, "Documents inside constructs first") appends nested definitions without sorting. Whichever lands second must sort and deduplicate there too, or `binary_search` misses definitions from container bodies. The revision adds a test for it: a reference in outer text to a definition inside a fenced container.
- Fuzzing `|` containers against `>` block quotes found two things.
  - A container's state must take the indentation of later lines itself, a column at a time, as micromark's block quote does. The docs did not say so, and a construct that took only spaces before `|` ended a container where `>` goes on. The containers commit now says it in the docs of `Container` and `Ctx::current`, with a test that fails for a construct that takes only spaces.
  - 1.0.0 nests lists wrongly after an item that ends in unclosed code, such as `1. ```\n\n+ b`: `to_html` writes `<ol><li>…</li><ul>…</ol></ul>` and `to_mdast` panics. Constructs neither cause nor change it: 13,385 of the 12,000,000 documents panic with and without the container. The developer's upstream PR 267 fixes it, and no change is needed here.
- Cargo leaves a workspace's path out of its fingerprints, so two worktrees at different commits must not share a target directory: a build reused a library from another commit. Each measurement uses its own target directory, or builds from a fresh checkout.
