---
status: Active
---

# Block constructs and nested content for plugin syntax (#32)

## Task summary

Plugin syntax on `feat/plugin-prototypes` covers inline constructs with flat tokens. Many plugins need more:

- block syntax: directives, admonitions, definition lists;
- markdown inside their own syntax: a wikilink alias with emphasis, a directive label, a container body.

This expansion designs block and nested-content extension points in the style of micromark and validates them with a real plugin. It tracks wooorm/markdown-rs#32 and builds on `plans/32-plugin-prototypes.md`, decisions D1 to D25.

## Skills

- `spike-and-stabilize-generic`: process, with spike passes of 50 lines or fewer and stabilize per layer.
- `humanize`: prose rules and Vale for the findings doc, commit messages, and any #32 follow-up.
- `code-review-generic`: the review step inside each stabilize pass.

## Syntax categories

| Category | Examples | micromark mechanism | markdown-rs today |
|---|---|---|---|
| Inline, flat | mention, `[[page]]`, `:emoji:` | `text` construct | works: `TextConstruct` |
| Inline with nested text | text directive `:name[*label*]`, wikilink alias, spoiler, ruby, inline footnote `^[…]` | label chunks with `contentType: 'text'`, parsed later by `subtokenize` | missing |
| Inline delimiter runs | `==mark==`, `^sup^`, `~sub~`, `++ins++` | `attentionMarkers`, `insideSpan`, `resolveAll`, as in GFM strikethrough | missing |
| Block leaf | leaf directive `::name[label]`, TOC line, `$$` math | `flow` construct, with a `chunkText` label | missing |
| Fenced container | container directive `:::name`, markdown-it-container admonitions, Pandoc fenced divs | `concrete` `flow` construct whose body lines are `chunkDocument` with `contentType: 'document'` | missing |
| Prefix container | definition lists, `!!! note` admonitions, footnote-style definitions | `document` construct with `tokenize`, `continuation`, `exit`, and `containerState` | missing |

Sources: `micromark-util-subtokenize/dev/index.js`, `micromark-extension-directive/dev/lib/directive-container.js` and `factory-label.js`, `micromark-extension-gfm-footnote/dev/lib/syntax.js`, `micromark-extension-gfm-strikethrough/dev/lib/syntax.js`. micromark added `contentTypeDocument` in commit `0ed6e00` (2021-06-09, micromark 3.0.0-alpha.3). The container-directive body already used it.

## Constraints

Carried from the first prototype:

- no_std with alloc, `#![forbid(unsafe_code)]` on new crates, MSRV 1.56, edition 2018.
- Plugins never touch `EditMap`, resolvers, attempts, or `tokenize_state`. The core facade may.
- With no plugins registered, small documents must not get slower. The gate is callgrind instruction counts per size bin against 1.0.0.

From reconnaissance of `feat/plugin-prototypes` at 49d1b7b:

- `subtokenize` runs to a fixed point (`parser.rs:76-87`), so a chunk made while tokenizing text is parsed in the next pass. No built-in makes `Content::Text` from inside text, and none makes nested flow or document content.
- `Content::Flow` in `subtokenize` falls through to text in release builds (`subtokenize.rs:107-116`). `Content::Document` does not exist.
- Definitions are known before text only because `content::resolve` runs a content-first pass during the document flush (`content.rs:182-186`). Definitions inside a nested document parsed in a later pass would not be visible to outer text.
- `document::resolve` places container exits by counting `LineEnding` and `BlankLineEnding` exits (`document.rs:544-548`). A flow construct must emit real `LineEnding` events.
- `flow::after` hits `unreachable!` unless a flow construct ends at a line ending or the end of input (`flow.rs:270-281`).
- Flow constructs must reset `interrupt` and `concrete` when they end.
- Resolvers are not idempotent: heading, setext, list item, table. Nested document events must be spliced in after the outer document's resolvers ran.
- `Name::Data` cannot carry a link inside text: `partial_data` merging and the GFM autolink literal rewrite drop links (`gfm_autolink_literal.rs:726-728`). Nested chunks need their own void event name.
- `to_mdast::tail_pop` requires equal Enter and Exit names. A construct that starts with `Extension` and ends with `ExtensionContinuation` cannot be popped as is.
- `to_html` writes a construct's whole source slice, so nested content would be written twice, and a flow token spanning lines would include `> ` prefixes.
- `collect_tokens` ignores depth and construct index, so a construct nested in its own label would merge token lists.
- `document::start` tries a BOM and frontmatter. A nested document must start past both.

From the parse-speed session (#113 branch):

- Attempts do not roll back `tokenize_state`: `label_starts` and its watermark, attention sequences, and queued `EditMap` edits. Nested content must get a fresh tokenizer through `subtokenize`, never a recursive parse inside an attempt.
- Container memos assume `define_skip` positions for a line never change.
- A flow plugin that can fail after spanning lines adds a new revert source, which the MDX brace memo assumes away, and adds quadratic retries.
- `Tokenizer` must stay at or under 1000 bytes on the merged branch. The extension state moves behind one `Option<Box<..>>`, 8 bytes.
- 49d1b7b measured +0.3% to +1.6% on small documents with no plugins. More than half comes from `event::Name` carrying data (`Extension(u8, u16)`), which makes every name comparison slower. There are also `State::to_result`, the per-event extension check in `to_html`, and `Position::from_exit_event` no longer being inlined.

## Design

### Nested regions

A construct marks a region of its input as nested content, the way micromark enters a chunk with a `contentType`:

- `ConstructTokenizer::enter_content(kind)` and `exit_content()`. The kind is `Text` inside text and flow constructs, and `Document` inside flow constructs.
- The facade emits a region token around linked chunks with a new void event name, linked across line endings with `link_to`.
- `subtokenize` parses each region later with a fresh tokenizer, so attempts, `label_starts` and attention state never leak.
- `TextConstruct::to_mdast` and `FlowConstruct::to_mdast` receive the region's mdast children, one list per region, next to the construct's own tokens.
- Positions of nested nodes point into the original source, across container prefixes.

### Flow constructs

- `FlowConstruct` has the same `markers`, `step` and `to_mdast` shape as `TextConstruct`.
- Dispatch happens at the top of `flow::start`, behind an empty-list check, before the built-in fast paths. The facade strips up to 3 columns of indent first.
- Rules the facade enforces:
  - A lazy next line, or the end of input, looks like the end of input to the construct: `current()` is `None`, as in micromark's `nonLazyContinuation`.
  - The construct is concrete after its opening line. Containers cannot open inside it, so a `> ` line inside a fenced container belongs to its body.
  - `interrupt()` tells the construct that it would interrupt a paragraph.
  - Line endings are real `LineEnding` events. Inside a `Document` region they are linked chunks, so blank lines reach the nested document.
  - The facade resets `interrupt` and `concrete` when the construct ends.
- `ConstructTokenizer::line()` returns the rest of the current line without consuming it. A fenced construct checks for its closing fence before deciding that a line is body. This replaces `attempt`, which plugins must not use.

### Nested documents and parse order

- `Content::Document` maps to a new document start state that skips the BOM and frontmatter.
- The parser loop runs document regions to a fixed point before any text pass. Definitions and footnote definitions inside a container body are then known to all text, as in CommonMark, where definitions inside a block quote apply everywhere.
- With no flow constructs registered, the extra pass is skipped.

### Compilers

- `to_mdast`: the construct node becomes a parent on the tree stack. Region events compile into buffered children and are handed to the construct at its exit. Depth-aware walking replaces `extension_end`.
- `to_html`, the event path: nested content renders in place, and each of the construct's own tokens is written as its own source text. So `[[a|*b*]]` gives `[[a|<em>b</em>]]`, with no duplication and no container prefixes. Whether constructs get an HTML hook is already question 9 in the posted #32 comment.

### Prefix containers, designed only

This follows micromark's `document` constructs:

- `Container::Extension(u8)` in the document tokenizer.
- The plugin provides the first-line `start`, a per-line `continuation`, and an `exit`. The core keeps lazy continuation and blank-line handling.
- The resolvers that list prefix names (`content.rs:140-142`, `heading_setext.rs:200`, `list_item.rs:401-406`, `to_html.rs:1393-1403`, `util/infer.rs:66`) must accept a plugin prefix. Otherwise paragraphs split per line inside the container.

### Delimiter runs, designed only

This follows micromark's `attentionMarkers` and GFM strikethrough:

- A declarative registration: a byte, the allowed run sizes, and a node name.
- The built-in attention resolver pairs the runs, so flanking and nesting with `*` and `_` follow CommonMark.
- Plugins never write a resolver.

## Decisions and alternatives

### D26. How nested content is parsed

- Question: a construct needs part of its input parsed as markdown, for example a wikilink alias or a directive body. How?
- Alternatives:
  - Linked regions, parsed later by `subtokenize` with a fresh tokenizer. Chosen.
    - This is micromark's `contentType` mechanism, including `document` for container directives.
    - Positions stay correct across container prefixes. Definitions resolve, and attempts cannot leak state.
    - Costs: a region API in the facade, a void chunk name, `Content::Document`, depth-aware `collect_tokens`, and parent handling in `to_mdast`.
    - The construct finds its own end by scanning bytes. A `]]` inside a code span in the alias ends the wikilink early. micromark directive labels have the same limit.
  - Opener and closer tokens paired by a core resolver, as links are.
    - Inline parsing continues between the markers, so code spans take precedence as they do in links.
    - It exposes pairing and resolver semantics to plugins. It does not help block content.
  - Re-parse the slice at tree-build time, as markdown-it and markdown-it.rs do with recursive `tokenize` calls.
    - The smallest core change.
    - Multi-line slices contain container prefixes, and positions need remapping. Definitions are not shared. Recursion depth is unbounded on the stack, which no_std cannot recover from.
- Transcript: question 2.

### D27. When a flow construct may fail

- Question: a flow construct that fails after many lines is retried at the next line, which makes parsing quadratic. It is also a revert source the MDX brace memo does not expect.
- Alternatives:
  - Decide on the opening line. After the first line ending, `Nok` stops the parse with an error message naming the construct. Chosen. Fenced code, math, and micromark's container directive already behave this way. `line()` lookahead means a construct never needs to fail later.
  - A failing later line ends the construct before that line, like a lazy line. It is bounded to one line of rework and more forgiving. It needs an attempt per line and saved facade state.
  - Allow `Nok` anywhere, as built-in attempts do. Quadratic input becomes possible, and the MDX memo guard needs widening.
- Transcript: question 3.

### D28. When nested documents are parsed

- Alternatives:
  - A document-first loop in `parse` after the document flush, then the text passes. Recommended. Definitions inside bodies stay global, and the outer resolvers have already run.
  - During the outer document flush, as a resolver like `content::resolve`. Heading, setext, list item and table resolvers would wrap nested events a second time.
  - In the same passes as text. Outer text parsed in the same pass would miss definitions from the body.

### D29. Nesting depth

- Question: each nesting level adds one `subtokenize` pass over all events. Fences nest only when the outer fence is longer, so documents nest at most about the square root of twice the length. Label nesting is capped by the plugin, as micromark caps balanced brackets at 32.
- Alternatives:
  - No core cap. Plugins keep micromark's caps, and a pathological test measures deep fences and labels. Recommended, following the preference for exact fixes over limits.
  - A core depth cap that leaves deeper regions as literal text. This changes accepted output.
  - Expand nested regions without a full pass per level. An exact fix, but a larger change to `subtokenize`. It is the fallback if the measurement shows a problem.

### D30. Scratch memory for constructs

- This revisits D20, whose revisit condition now holds: a fence needs its opening size, and a label needs its bracket depth.
- Alternatives:
  - A small per-match scratch array, zeroed at construct start and kept in the boxed extension state. Recommended.
  - Encode counts in the `u16` state. No API change, but states become arithmetic.
  - A per-match state object allocated on each attempt. Friendly, but it allocates at every marker byte.

### D31. The `event::Name` cost

- Alternatives, to be measured with callgrind:
  - A unit `Name::Extension` with the construct and token id stored in `Event` padding. `Event` has about 6 spare bytes. There are 45 `Event { .. }` literals in 10 files, and the perf branch touches several of them.
  - A hand-written, inlined `PartialEq` that compares the tag byte first.
  - Keep the data, and move the per-event `to_html` check into the existing `match` arms.

### D32. Custom node fields for directives

- Question: `Custom` has one flat `attributes` map. mdast-util-directive nodes have a directive `name`, an `attributes` map, and a label paragraph marked with `data.directiveLabel`.
- Alternatives:
  - Add a `fields` map to `Custom` for node properties, such as the directive name or the wikilink target. `attributes` keeps HTML-like attributes such as `{#id .class k=v}`. A container label is the first child `Paragraph`, flagged by `fields.label`. Chosen.
  - Keep `Custom` as it is, and store the directive name and the label flag under keys that directive syntax cannot produce. No core change, but it is a hidden convention.
  - Let attribute values nest, closer to unist. More API for one plugin.
- Transcript: question 7.

### D33. Line endings and token values inside content

- Found in spike passes 1 and 2.
- Line endings inside content stay in the content, one linked chunk per line, as in paragraphs. Tokens around the content stay whole, so emphasis in `{{*a\nb*}}` can cross lines.
  - Alternative: split tokens at line endings as in D21. Nested events would then interleave with the split tokens, and every compiler would have to skip them.
- A content token's `value` is empty, and its markdown is in `children`. Other tokens' values leave content out, so they never contain a container prefix: the outer token of `{{*b*}}` has the value `{{}}`.
- `to_mdast` takes `Vec<Token>`, so a construct moves children into its node without cloning.
- Inside content, a construct can only `consume` and `exit`. The bytes are markdown, not construct tokens.

### D34. Line endings in flow constructs

- Found in spike pass 4.
- Flow tokens stay whole across lines, as `CodeFenced` does. Line endings are real `LineEnding` events, so `document::resolve` counts lines and places container exits. Inside content they are linked `LineEnding` chunks, so blank lines reach the nested document.
- Values leave out the container-prefix events inside a token, as well as content. So a fence inside `> ` has the value `:::\na\n:::`.
- A flow `consume()` of a line ending is deferred to the end of the step. The facade then marks the construct concrete, checks that the next line continues it, and only then consumes the line ending. If the next line is lazy or the end, the construct sees `current()` as `None` at the line ending and ends before it.
  - The first spike checked the next line before the construct had consumed anything. The document then read the next line while the construct was not concrete, and opened a block quote for a `> ` inside the body.
  - A single-line leaf never triggers the check.
- A flow construct's `Ok` requires the current byte to be a line ending or the end, with no `consume` in that step. After the first consumed line ending, `Nok` is a parse error (D27).
- Alternative: split tokens at line endings, as D21 did for text. A fenced body could then not end after its last line ending.

### D35. Core fixes for a document that starts mid-line

- Found in spike pass 4, with a container inside a block quote.
- `Tokenizer::account_for_potential_skip` treated only column 1 as a line start. A nested document starts after the outer `> `, so its first line never skipped its own list prefix. The spike also accepts the tokenizer's starting point as a line start.
- `document::flow_end` pushed the flow child up to the document's current point. With chunked input, that point has already skipped the next line's outer prefix, so the child received the `>`. The spike pushes up to `line_start`, the raw start of the next line, when not at the end.
- Both are equivalent at the top level, where no skips exist. The full suite passes, including CommonMark.

### D36. Attempts in constructs

- Question: micromark directives try optional parts, the label and attributes, with `effects.attempt`, and text directives let them span lines. How does a construct fall back?
- Alternatives:
  - `Step::Attempt { state, ok, nok }`, micromark’s `effects.attempt` over a sub-machine. Chosen.
    - The facade pushes a frame with the open-token depth, and runs `Tokenizer::attempt`, which reverts events and position on failure.
    - An attempt succeeds only with the tokens it opened closed, and closes no others. A rule broken inside it fails the attempt, not the construct (D44).
    - Content chunks stay off the tokenizer stack: a chunk is open when its enter is the last event. `Tokenizer::attempt` undoes by truncating events and the stack, which restores an open chunk from events but could not restore one popped from the stack.
    - The facade keeps the last chunk of the current content token, and each attempt saves it. After a failed attempt, that chunk drops a `next` link to a chunk the undo removed. Scanning back for it instead made each failed attempt cost as much as the match so far.
    - An attempt that fails, or succeeds without moving, counts toward the retry cap, so it cannot loop.
    - In flow, an attempt cannot consume a line ending. So reverts in flow stay within one line, and the rule that a flow construct fails only on its first line holds.
    - Facade memory is not undone by a failed attempt; a sub-machine initializes what it uses.
  - Lookahead only (`line()`): exact for single-line parts, but a text directive whose label fails after a line ending would lose `:name` too.
- It replaces `line()`: closing fences are attempts, as in micromark’s `directive-container.js`.
- Transcript: question 12.

### D37. Aligning the facade with micromark

- The developer’s direction during the spike: “aim to align design with micromark, avoid proliferating constructs”.
- One `Construct` trait. `text_constructs` and `flow_constructs` decide where it runs, as micromark extension keys do. A `previous(byte)` guard mirrors micromark’s `previous`.
- One `enter_content(name, ContentType)`, micromark’s `{contentType}`. The content type lives with the interned token name, as `contentType` lives on a micromark token. So there is one `Extension(u8, u16)` event variant, plus `ExtensionChunk` for linked chunks, micromark’s `chunkText` and `chunkDocument`.
- Removed, compared with 49d1b7b and the early spike: `ExtensionContinuation`, `ExtensionLineEnding`, `ExtensionContent`, `ExtensionDocument`, the reopen list, `enter_document`, and `line()`.
- A construct can enter its own tokens inside its content token, as micromark’s container puts `linePrefix` and the closing fence inside `directiveContainerContent`. Such a token starts before any content on its line, since after content it would split that content. Those tokens are interned as *in content*: their bytes are not content, the compilers skip them, and `to_mdast` does not receive them. Chunks of one content token stay linked across them.
- Internal state names: `TextBeforeConstruct`, `FlowBeforeConstruct`, `ExtensionStep`, `ExtensionNonLazy`, `ExtensionLazy`, `ExtensionAttemptOk`, `ExtensionAttemptNok`, `DocumentStartNested`.

### D38. Skips of chunks that continue a line

- The parse-speed memos assume a line’s skip never changes once its bytes are fed. A line-ending chunk in a nested document redefined its line’s skip, so a revert into that line would skip its content.
- `subtokenize` now only fills a missing skip, with `define_skip_if_missing`, for a chunk that continues the previous chunk’s line. Not defining it at all misaligned the per-line table and broke escapes.
- Paragraph chunks, which cross a line ending, still call `define_skip`.

### D39. Links inside spliced child events

- Found with the directive container: an indented container whose body has two lines panicked in `subtokenize` with “expected link”.
- `divide_events` re-indexes the links inside child events when it splices them into their parent. Its arithmetic assumed that consecutive linked events of a chain are two events apart (enter, exit, next enter). A `linePrefix` token between a line-ending chunk and the next chunk broke that.
- Fix: a child event lands at its chunk’s position, minus two per earlier chunk, plus its child index. That equals the old arithmetic for adjacent chains, and the suite passes.
- A chunk in which no child event starts is skipped, and its empty slice removes its enter and exit. The index arithmetic adds before it subtracts, since removed chunk events can outnumber child events (D44).

### D40. Prefix containers

- Question 14 asked for them in the spike, before stabilizing.
- Shape, like micromark’s `document` constructs:
  - `ParseOptions.document_constructs`, tried before the built-in containers at their markers, after up to 3 columns of indentation the facade skips.
  - The start ends in `Ok` with two tokens left open: the container token, and a `ContentType::Document` token for its body. The rest of the line is flow. The core keeps both open and exits them when the container closes, as it does for block quotes.
  - `Construct::continuation()` is the state that runs at the start of each later line, inside the core’s existing attempt. It enters its prefix tokens, which are tokens in content, and ends in `Ok` (continued) or `Nok`. Lazy lines and closing stay with the core.
  - `Container::Extension(construct, open token, content token)`, with one word of memory kept in `ContainerState.size`, like micromark’s `containerState`.
  - The compilers need nothing new: the body is a content token whose events are already flow.
- Core changes:
  - `content::resolve`, setext headings, and list grouping look past plugin tokens in content, the prefixes, so paragraphs and lists in a plugin container stay whole. Other plugin tokens stop them, so lists before and after a plugin container stay apart.
  - A text construct in a plugin container leaves the container’s prefix tokens out of its values, as it does for block quote prefixes. A nested instance of a construct leaves out its outer instance’s prefixes, since a match’s own tokens in content sit in its content.
  - A container’s own checks only see tokens opened in the current match (`extension_stack_len`), so an admonition inside an admonition works.
- Validated with `plugins/admonition`: `!!! note "Title"` and `???`/`???+` with bodies indented by four spaces. Titles are text content. Smoke cases: block quote, list, lazy line, fenced code, setext heading, nesting, several kinds, empty title.
- Divergence from Python-Markdown: a lazy line continues the innermost paragraph, as in CommonMark containers.

### D41. Delimiter runs

- Question 16 asked for them in the spike too.
- Shape, like micromark’s `attentionMarkers` with a strikethrough-style construct, but declarative, since plugins do not write resolvers:
  - `Construct::attention_sizes()`: when not empty, `text::before` tokenizes a run of the construct’s marker as an `AttentionSequence`, and `step` is not used.
  - The attention resolver pairs runs of equal, allowed size, with CommonMark flanking and no misnesting.
  - Emphasis markers can be used around plugin runs, as `attentionMarkers` allows: `*==a==*`.
  - A pair becomes `attention` > `attentionSequence`, `attentionText` (a text content token, whose events are already parsed), `attentionSequence`. `to_mdast` receives the content token’s children.
- Validated with `plugins/mark`, `==mark==` as `<mark>`, like micromark-extension-highlight-mark, including nesting, sizes, flanking, links, and block quotes.

### D42. Fidelity to micromark-extension-directive

- Question 15 chose porting its syntax tests.
- Method: stubs record each case of its `test/index.js` (main, 2026-09-27) under Node, generating `plugins/directive/tests/fixtures/micromark_cases.rs`. Its test HTML handlers are ported.
- Result: all 255 cases match, ignoring whitespace between tags and attribute order, which come from each compiler and from `Custom`’s sorted map.
- Fixes the port needed, all now in the core facade or the plugin:
  - `previous` is `None` after a character escape, as micromark checks for `characterEscape` (`\\::a`).
  - Attribute names can start with `-` or `_`.
  - An empty content token is dropped rather than rejected, since micromark only enters content when there is some.
  - Token boundaries can be between the virtual spaces of a tab, which line prefixes need.
  - Text content keeps its initial and final whitespace, like micromark’s `_contentTypeTextTrailing`. The wiki link plugin treats a whitespace-only alias as empty (D24).
  - `ConstructTokenizer::options()` exposes the parse options, like micromark’s `parser.constructs`, for the case with indented code turned off.
- Divergences: attribute order, and HTML line endings between blocks, which are compiler formatting, not parsing.

### D43. Cost without plugins

- Callgrind instructions against 1.0.0 (`1506572`), no plugins registered, a probe that runs one document through one function. Before and after the fieldless `Name` (question 19):

| Document | Config | `to_html` before | `to_html` now | `to_mdast` before | `to_mdast` now |
| --- | --- | --- | --- | --- | --- |
| tiny, 13 B | CommonMark | +1.62% | +0.82% | +1.03% | +0.37% |
| tiny | GFM | +1.47% | +0.79% | +1.16% | +0.60% |
| small, about 1 KB | CommonMark | +0.69% | +0.49% | +0.63% | +0.39% |
| small | GFM | +0.65% | +0.49% | +0.57% | +0.39% |
| `readme.md` | CommonMark | +0.68% | +0.32% | +0.53% | +0.14% |
| `readme.md` | GFM | +0.47% | +0.32% | +0.37% | +0.27% |

- Changes that stayed:
  - `Name::Extension` is fieldless, and `Event.extension: u16` holds the interned token name. The field fits in `Event` padding, which stays 80 bytes; `Name` is 1 byte again. The interned table records each token’s construct. The facade keeps the ids of open tokens itself, since the tokenizer stack only holds names. This touched 44 `Event` literals in 9 files, and the empty-token assertion in `Tokenizer::exit`, since nested plugin tokens now share a name.
  - The facade state lives in one `Option<Box<ExtensionState>>`, created when a construct runs. `Tokenizer` is 672 bytes against 664 on 1.0.0.
  - `divide_events` fixes links in one pass, finding an earlier event’s slice by stepping back from the current one: −470 instructions on the tiny document.
  - `exit_containers` no longer allocates a `Vec` per closed container.
- Reverted, each costing instructions on the tiny document: a compiler gate (+120), skipping the link pass (+158), and a hand-written `PartialEq` for `Name` (+436).
- Boxing `Custom` (question 20) shrinks `mdast::Node` to 152 bytes but slows `to_mdast` by up to 0.13%, so it was reverted too.
- `mdast::Node` stays 176 bytes against 152 on 1.0.0, because `Custom`, now the largest variant, has `fields`.
- The performance session’s differential found no difference from 1.0.0 over 1,001,467 inputs in 7 option sets.
- With one codegen unit and rustc 1.98.1, the same bins measure +0.34% to +0.94%. The default profile’s partitioning alone moves counts by about 1.5% (D45, D46).
- The review fixes (D44) cost up to 0.10 points on the small and `readme.md` documents, and nothing on the tiny document:
  - The `divide_events` loop costs about 1,150 instructions on small `to_html`. Two other loop shapes measured worse, by 311 and 1,382.
  - The list item lookback in `to_html` costs 523 on small `to_html` and 3,470 on `readme.md`. Returning early without plugin tokens saved 152 and 1,111 of that; `#[inline]` on the helpers cost more.
  - `content::resolve` borrows the name table once rather than at every line join, which saved 1,306.

### D44. Review fixes

- The sub-agent review (question 18) and its re-review found these, all fixed:
  - Critical: `divide_events` placed a child event in the wrong chunk when an earlier chunk had no event in it, which lost directive body lines (D39).
  - Its index arithmetic underflowed when removed chunk events outnumbered child events.
  - Resolver lookbacks skipped every plugin token, so the lists around an admonition merged (D40). The `to_html` list item lookback skipped no plugin prefix, so a tight item in a plugin container ended with a line ending.
  - Attempts could close tokens opened before them, lose an open chunk or keep a link to a removed one on undo, or loop forever (D36).
  - A token in content could start after content on its line and split it (D37).
  - A text construct’s values kept a plugin container’s prefix, and a nested instance of a construct kept its outer instance’s prefix (D40).
  - `own_text` scanned every excluded span for every token, now a binary search.
- Mutation checks found a failing test for nearly every fix; the retry fixes fail by hanging. No input reaches the `content::resolve` narrowing.
- The third round found no critical defect but a quadratic cost, now fixed: a 40 KB match took 1.6 s, now 11 ms.
- Choices:
  - A rule broken inside an attempt fails the attempt, like any `Nok`. Failing the whole construct instead needs a flag through every attempt frame, and micromark has no such rule to mirror.
  - The facade records the line of the last content byte, not a flag. A fourth `bool` in `ExtensionState` trips clippy’s `struct_excessive_bools`, and the chunk state cannot stand in, since an attempt closes the chunk mid-line.
  - `is_in_content` holds the check that both compilers, `content::resolve`, and `util::skip` repeated.
  - Chunks off the stack, rather than closing an open chunk before each attempt. Closing it made an attempt that continued content open a second chunk, which the success check counted as a token left open.
- Revisit if plugin authors want a broken rule reported instead of a silently failed attempt.

### D46. The 1% gate on short documents

- Question: in the performance session’s harness, GFM `to_html` on short documents costs +1.02% over 1.0.0, above the gate of about 1%. Should that change the code?
- Evidence, with both sides built in one run on one compiler:
  - My three documents measure +0.34% to +0.94%, and the 652 spec examples +0.71% to +0.91%.
  - A profile of GFM `to_html` over the spec examples shows no hot spot. About two thirds of the extra is in parsing, from the extension design: the `extension` field in every event, nested documents, and attention-construct lookups. The rest is the per-event plugin check in `to_html`.
  - Layout noise from changing one function is 0.2 to 0.6 points, more than the 0.02-point overshoot.
- Alternatives:
  - Accept and document, chosen.
  - One plugin check per event in `to_html`: up to 0.21 points better on documents, and up to 0.63 points worse on spec examples.
  - `#[inline]` on `attention_construct`: up to 0.10 points better on the documents, but up to 0.77 points worse on the spec examples.
  - A harness that averages over code layout, before judging any lever: more work, and not needed for this decision.
- The stable toolchain changed from rustc 1.95 to 1.98.1 during this work. Comparisons need both sides built in the same run, on one toolchain.
- Revisit if: a lever helps on every bin by more than the layout noise, or a harness that averages over layout makes smaller differences measurable.
- Transcript: questions 21 and 22.

### Nearby finding: fenced code at the end of a list item

- `- ```\n  a\n\n  b\n- c` renders a second `<ul>` inside the first on 1.0.0 (`1506572`) and on this branch. The unclosed fenced code runs to the end of the item, and the next item starts a nested list. This is not related to plugins. A candidate for a separate upstream issue.

### Nearby finding: list items with no line ending between them

- In `list_item::resolve`, `skip::opt(.., &[Name::ListItem])` skips every consecutive list item, not one. An unclosed fenced code keeps its item’s last line ending, so the next item follows with no event between them. With another marker on that item, the first list ends after the second, and `to_mdast` panics on `- ```\n1. a`, in 1.0.0 too. An unclosed container directive reaches the same panic, which is how fuzzing found it. A candidate for a separate upstream issue.
- Version 1.0.0 also panics on `[a]:[\n=\n=-\n-`, with the message `Cannot push to non-parent`.

## Clarifying questions and answers

1. Scope: "Block and nested plugin syntax comes in several kinds: inline with nested text, such as a directive label or wikilink alias; block leaf; fenced container with a nested document; prefix container, such as definition lists; delimiter runs, such as `==mark==`. Which should the prototype build and prove with a real plugin now? The rest get a written design in the plan and findings doc."
   Answer: Directives port (Recommended). Port micromark-extension-directive: `:name[label]`, `::name[label]` and `:::name` … `:::`. Prefix containers and delimiter runs are designed on paper.
2. Nesting: "A construct needs part of its input parsed as markdown, such as a wikilink alias or a directive label or body. How should nested content be parsed? This is the core mechanism, and it is hard to change later."
   Answer: Linked regions (Recommended).
3. Block fail: "A block construct that fails after spanning many lines gets retried at each following line, which makes parsing quadratic. It is also a revert that the parse-speed branch's MDX memo doesn't expect. When may a block construct fail?"
   Answer: Opening line only (Recommended).
4. Alias: "You flagged that a wikilink alias can't contain emphasis. Obsidian and landakram's remark wikilink plugin both keep the alias as plain text. Obsidian users have an open feature request asking for formatting there. What should the wiki_link plugin do?"
   Answer: Parse as markdown (Recommended). `[[a|*b*]]` renders `<a href=…><em>b</em></a>`, which diverges from Obsidian.
5. Plan: "The plan in plans/32-block-nested-syntax.md now reflects your answers: a directives port, linked regions, block constructs that fail only on the opening line, and a wikilink alias parsed as markdown. Approve it, and what should run first?"
   Answer: Approve, features first (Recommended). The `event::Name` cost fix and the boxed state land in the performance pass.
6. Branch: "Your posted #32 comment links to feat/plugin-prototypes, which is pushed. Where should the block and nested work go?"
   Answer: Same branch (Recommended). New commits on `feat/plugin-prototypes`.
7. Custom: "`Custom` has one flat `attributes` string map. A directive node in mdast-util-directive has a directive `name`, its own `attributes` map, and, for containers, a first-child label paragraph marked with `data.directiveLabel`. How should the directive plugin fit into `Custom`?"
   Answer: Add a fields map (Recommended).
8. Spike shape: "The nested-text spike works: `enter_content` marks a region, subtokenize parses it later, and `to_mdast` hands the construct a `Vec<Token>` whose content tokens carry parsed children. Is this shape right, and should I stabilize this layer before starting block constructs?"
   Answer: Shape OK, spike blocks first. All layers stabilize together after the block spike.
9. Line model: "Block constructs keep tokens whole across lines, as CodeFenced does, and leave container prefixes out of values by skipping prefix events. Text constructs still split every token at each line ending, the D21 design, with ExtensionContinuation/ExtensionLineEnding events and a reopen list. Should text constructs switch to the block model?"
   Answer: Unify on block model (Recommended). This supersedes the split-and-join choice in D21.
10. Trait: "The spike registers flow constructs with the same TextConstruct trait. micromark has one Construct type, and the extension key it's registered under (text, flow, document) says where it runs. How should markdown-rs name this?"
   Answer: One Construct trait (Recommended).
11. Next: "The core mechanism is spiked for nested text, block leaves and fenced containers with nested documents. What comes next?"
   Answer: Directives, then stabilize (Recommended).
12. Attempts: "micromark directives try their optional label `[…]` and attributes `{…}` with `effects.attempt`. If the label fails, `:name` still matches and the `[` stays text. Text-directive labels and attributes can span lines, so `line()` lookahead can't always check them ahead of time. Our facade has no attempt, and the parse-speed session's rule 4 assumes plugins never run attempts. How should the port handle optional parts?"
   Answer: Add Step::Attempt (Recommended).
13. Free-text direction, given during the spike: "aim to align design with micromark, avoid proliferating constructs". Applied as D37.
14. Stabilize: "The spike is complete … Confirm the shape and start stabilizing?"
   Answer: Spike prefix containers too. Applied as D40.
15. Fidelity: "How closely should the directives plugin be checked against micromark-extension-directive? …"
   Answer: Port its syntax tests (Recommended). Planned for stabilization.
16. Stabilize: "Every kind of block and nested plugin syntax now has a working spike … Start stabilizing? … Delimiter runs (`==mark==`) stay design-only."
   Answer: Spike delimiter runs too. Applied as D41.
17. Stabilize (again, after D41): "Every kind of plugin syntax now has a working spike and a plugin … Start stabilizing?"
   Answer: Confirm, stabilize (Recommended).
18. Review: "Stabilize checks are green … How should the code review of this diff run?"
   Answer: Sub-agent review (Recommended).
19. Name cost: "… A fieldless Name::Extension with the id in Event padding would remove it … What should happen with it?"
   Answer: Measure it on a spike (Recommended).
20. Node size: "mdast::Node is 176 bytes against 152 on 1.0.0 … How should that be handled?"
   Answer: Measure boxing Custom (Recommended).
21. The 1% gate: "One bin crosses your ~1% gate for short documents: GFM to_html on short documents … at +1.02%. … What should happen?"
   Answer: Profile, then decide (Recommended).
22. After profiling: "… The cost is spread across the extension design, with no hot spot. Both candidate fixes are inconsistent … What now?"
   Answer: Accept and document (Recommended).

## Assumptions (override at approval)

- The edge cases of directives follow micromark-extension-directive: an unclosed container runs to the end, a nested fence needs a longer outer fence, a lazy line closes the container, leaf and container labels cannot span lines, and text labels can.
- No core nesting cap (D29), per the preference for exact fixes over limits.
- The directive HTML handler renders containers and leaves as `<div>` and text directives as `<span>`, with the name as a class and sanitized attributes.
- The folded-in cleanups: box the extension state, and fix the `event::Name` cost (D31). Both are in code this task changes.

## First spike target

A nested text region in a text construct: a test construct `{{…}}` whose inside is parsed as text, so `{{*a*}}` gives a `Custom` node with an `Emphasis` child in `to_mdast`.

## Spike passes

1. Text regions: `enter_content(Text)`, the chunk name, linking across line endings, and `subtokenize` pickup.
2. `to_mdast` children per region, the parent on the stack, and depth-aware tokens.
3. Flow dispatch and the flow facade rules, checked with a single-line leaf construct.
4. `Content::Document`, the nested document start state, linked line endings, and the document-first loop. Check that `:::\n> *a*\n:::` gives `Custom > Blockquote > Paragraph > Emphasis`, and that a definition inside the body resolves outside.
5. The validation plugin crate and its tests.
6. `to_html` fallback, `event::Name` cost, boxed state, callgrind small bins, and pathological nesting.
7. Findings doc update.

## Files to modify or create

- `src/extension.rs`: region API, `line()`, scratch memory, `FlowConstruct`, flow rules, and depth-aware tokens.
- `src/event.rs`: `Content::Document`, the void chunk name, and the `Name` cost fix.
- `src/subtokenize.rs`, `src/parser.rs`: the document mapping and the document-first loop.
- `src/construct/flow.rs`, `src/construct/document.rs`, `src/state.rs`: flow dispatch and the nested document start.
- `src/tokenizer.rs`: the boxed extension state.
- `src/configuration.rs`: `ParseOptions.flow_constructs`, with `Debug` and the serde test.
- `src/to_mdast.rs`, `src/to_html.rs`: parent regions and the fallback.
- `plugins/wiki_link/`, a new plugin crate, `markdown_processor/` handler wiring, and `tests/extension.rs`.
- `plans/32-plugin-findings.md` and this plan.

## Verification

- CI loop, with outputs in `.agent-tmp/32-block-nested/`: `cargo fmt --check`, `cargo clippy --all-features --all-targets --workspace`, and `cargo test --all-features --workspace`.
- The CommonMark parity example stays unchanged.
- The no_std build for `thumbv7em-none-eabihf`.
- Callgrind small bins against 1.0.0 with no plugins. The parse-speed session offered to re-run its differential and bins.
- Pathological inputs: deeply nested fences and labels, and a flow construct that fails on its opening line at every line.
- Ported test cases from micromark-extension-directive for the validation plugin.

## Anticipated expansions

- Prefix containers, if chosen now or later.
- Delimiter runs through attention markers.
- A follow-up comment on #32, after the developer's go-ahead.

## Review notes

- Rounds 1 to 3, sub-agent, quick tier plus correctness probes and a fuzz harness: findings and fixes in D44.
- Advisory, open: nested attempts that retry after failing each get the retry cap, so their steps multiply with depth. A plugin like that loops forever on the previous version. A true bound needs a step budget per match.
- Advisory, pre-existing: fenced code is superlinear in its body size: 1.2 s for 160 KB on 1.0.0 and on this branch. Plugin flow constructs grow at the same rate. The perf branch makes fenced code linear, so a rebase onto it should fix both.

## Progress log

- D46: GFM `to_html` on short documents at +1.02% was profiled and accepted. Both levers were inconsistent across bins, and layout noise exceeds the overshoot. The findings doc now shows rustc 1.98.1 numbers from one run.
- Stabilize, part 3: review fixes (D44), in three sub-agent rounds, the last with no critical findings. Content chunks stay off the tokenizer stack, and the facade tracks the last chunk. Verified: fmt, clippy, 98 test binaries, `no_std` builds of all seven crates, fuzzing, and callgrind bins (D43).
- Stabilize, part 2: the performance pass (D43), with the fieldless `Name` kept and boxed `Custom` measured and reverted. Verified again: fmt, clippy (only the pre-existing `util/char.rs` warning), 98 test binaries, and `no_std` builds of all seven crates.
- Stabilize, part 1: spike code removed, 34 facade tests and plugin tests added (D42), clippy fixed without suppressions, and the facade state boxed (D43). Mutation checks show the tests catch the core fixes. fmt, clippy, all 98 test binaries, and `no_std` builds pass.

- Spike pass 7, delimiter runs (D41): `Construct::attention_sizes`, plugin runs in the attention tokenizer and resolver, and the mark plugin. The workspace suite passes. Not stabilized yet.
- Spike pass 6, prefix containers (D40): `document_constructs`, `Construct::continuation`, `Container::Extension`, resolver lookbacks past plugin prefixes, match-scoped token checks, and the admonitions plugin. The workspace suite passes. Not stabilized yet.
- Spike pass 5: the directives plugin, a port of micromark-extension-directive 4.0.0. It added `current_char()`, `memory()`, and `indent()` to the facade, `Custom.fields` (question 7), a markdown wikilink alias (question 4), and core fix D39.
- Facade rework after questions 9–13 (D36–D38): one `Construct` trait, one line model, `enter_content`, `Step::Attempt`, and tokens inside content. Three text tests changed with the line model, which lets tokens hold line endings.
- Spike passes 3 and 4: flow leaves and fences (D34), and nested documents with their definitions resolved everywhere (D35).
- Spike passes 1 and 2, nested text in text constructs: `enter_content`, linked per-line chunks, content-aware `collect_tokens`, and `to_mdast` children through `buffer`/`resume`. A smoke test shows `> a {{*b\n> c*}} d` giving `Custom > Emphasis > Text "b\nc"`. The workspace suite passes. Not stabilized yet (question 8).
