---
status: Active
---

# Plugin system prototypes for markdown-rs (#32)

## Context

Issue wooorm/markdown-rs#32 asks for custom plugins. wooorm's position in that thread:
- Plugins, meaning AST transforms, are welcome. The intended shape is mdast → hast → html, as in JS.
- Syntax extensions are "impossible in Rust" and out of scope.

Users keep asking for both kinds:
- Wikilinks: #62 and psxvoid in #32.
- Alerts: #159, #168, #180.
- Heading ids, rel/target on links, custom HTML output: #76, #128, #160, #183.
- Rendering mdast to HTML: #156.

The goal is evidence, not a merge-ready feature:
- A survey of how Rust plugin systems and pluggable parsers work.
- Working prototypes of a unified-style API: one real transformer plugin, GFM alerts, and one real syntax plugin, wikilinks.
- Measured risks.
- A findings doc the developer can later bring to #32.

Tracking: issue #32.

## Skills

- `spike-and-stabilize-generic`: governs the process: spike passes of 50 lines or fewer, then stabilize per layer.
- `humanize`: prose rules and Vale lint for the findings doc and commit messages.
- `code-review-generic`: the review step inside each stabilize pass.

## Design decisions and alternatives

This section records each design choice, the alternatives weighed, and the evidence behind the pick, so other maintainers can judge the choices for themselves. The prototype chose one option per decision. A choice stays open to challenge wherever its "Revisit if" condition holds. Each entry ends with a pointer to the verbatim question in the next section.

Terms used below:
- `no_std`: builds without the Rust standard library. Only `alloc` is available.
- WebAssembly (WASM): a sandboxed binary format that a host program can load plugins from at run time.
- Abstract syntax tree (AST): the general name for trees like mdast and hast.
- Speedy Web Compiler (SWC): the Rust JavaScript compiler behind Next.js. Its plugin history is the main prior art here.
- GitHub Flavored Markdown (GFM): tables, strikethrough, footnotes, task lists and autolink literals.
- JavaScript XML (JSX): the HTML-like syntax that MDX, markdown with JSX, compiles to.
- mdast: the markdown syntax tree.
- hast: the HTML syntax tree.
- `Processor`: the new pipeline type in `markdown_processor`.
- Plugin: anything that configures a `Processor`.
- Syntax extension: new markdown syntax in the tokenizer.
- Transform: a function that edits a finished tree.

### D1. Where the `no_std` line sits

- Question: which code must build without the Rust standard library?
- Alternatives:
  - Everything `no_std`, chosen:
    - Covers the plugin API and every reference plugin.
    - Plugins that need std-only crates, such as syntax highlighting with `syntect`, cannot be reference plugins.
  - Core `no_std`, plugins free:
    - The API stays `no_std`, and each plugin crate may opt into std.
    - Gives the most freedom to plugin authors. A plugin set is then `no_std` only if every plugin is.
  - std behind a `plugins` feature:
    - Simpler types become available, such as `HashMap`, `Box<dyn Error>` and `thread_local!`.
    - `no_std` users lose plugins entirely.
- Evidence: the `markdown` crate is `#![no_std]` with `alloc`. SWC routes diagnostics through thread-local globals, which need std. A `no_std` design must pass context explicitly instead.
- Revisit if: a common plugin, such as a highlighter, cannot exist without std.
- Transcript: Q1.

### D2. How plugins are packaged and loaded

- Question: are plugins compiled into the application, or loaded at run time?
- Alternatives:
  - Compiled-in crates, with a serializable boundary kept for later WASM, chosen:
    - A plugin is an ordinary crate that depends on `markdown` and runs in the same process.
    - It needs no runtime and no `unsafe`.
    - Plugin crates track the `markdown` major version.
  - Compiled-in only: the same as the choice, without the constraint that the tree types stay serializable.
  - Run-time loading first:
    - Covers WASM hosts such as wasmtime and extism, or native dynamic libraries through `abi_stable`.
    - Command-line users could add plugins without recompiling.
    - The tree becomes a versioned wire format. Dynamic libraries need `unsafe`, and Rust has no stable application binary interface (ABI) between compiler versions.
  - Link-time registration, with `linkme` or `inventory`:
    - Plugins would register themselves.
    - Rejected: `linkme` has no documented wasm support, and a crate that is never referenced is not linked in.
    - Rejected: `inventory` runs code before `main`, which fails on most embedded targets.
- Evidence:
  - SWC's first wasm plugin boundary serialized its AST with `rkyv`. Every AST change broke existing plugins and forced lockstep upgrades.
  - SWC moved to self-describing Concise Binary Object Representation (CBOR) in Nov 2025. Plugins still break when a field is removed or changes type.
  - `wasmi` runs without std, so a future WASM host could stay `no_std`.
- Revisit if: users need plugins in a prebuilt binary, such as a command-line tool.
- Transcript: Q2.

### D3. How a transformed tree becomes HTML

- Question: `to_html` compiles straight from parser events, so a tree transform cannot affect it. What path turns a transformed tree into HTML?
- Alternatives:
  - mdast → hast → HTML, chosen:
    - Mirrors `remark-rehype` + `rehype-stringify`, and allows both mdast and hast plugins.
    - Reuses the mdxjs-rs converter, about 1,300 lines of implementation.
    - Adds the most code.
  - mdast → HTML directly:
    - One compiler, the one issue #156 asks for.
    - Leaves no HTML-tree stage for plugins such as heading ids or link `rel`.
  - mdast only: plugins transform the tree, and rendering is left to users. This proves the API shape only.
- Evidence: wooorm proposed the mdast → hast → HTML design in #32. mdxjs-rs already has `mdast_util_to_hast`.
- Revisit if: pass 8 measures parity and performance and shows the tree path diverges from `to_html`, or costs too much.
- Transcript: Q3.

### D4. Which plugins the prototype builds

- Question: which real plugins exercise the design?
- Alternatives for the syntax plugin:
  - Wikilinks `[[Page|alias]]`, chosen:
    - Inline syntax, and the most requested, in #62 and #32.
    - It competes with the built-in link syntax for `[`.
  - A block `:::` container: tests the harder block layer, with nesting and resolvers.
  - Both: covers both tokenizer layers at about twice the effort.
- Alternatives for the transformer:
  - GFM alerts, chosen: requested in #159, #168 and #180, and needs an mdast transform plus custom HTML.
  - Heading ids: a port of rehype-slug, hast-only, requested in #76.
  - Both.
- Evidence: GFM alerts need no new syntax. The blockquote syntax already exists, so alerts fit a transform.
- Revisit if: the wikilinks work shows block-level extensions need a separate design. It likely does.
- Transcript: Q4 and Q6.

### D5. The plugin API shape

- Question: what does a user write to add a plugin, and what does a plugin author implement?
- Alternatives:
  - One attacher trait, chosen:
    - `Plugin::attach(self, &mut Processor)`, and a plugin may register syntax, transforms and handlers.
    - Users learn one concept, `.plugin(x)`, like unified's `.use()` or Bevy's `Plugin`.
  - A trait per stage:
    - `SyntaxExtension`, `MdastTransform` and `HastTransform`, each added with its own method.
    - Each trait is small, but a package like wikilinks asks users to add three pieces in the right order: syntax, transform and handler.
  - A static generic chain:
    - `Processor<Chain<A, B>>`, as in tower's `ServiceBuilder` or SWC's tuple `Pass` impls.
    - No `Box<dyn>`, but long types in compiler errors.
    - The chain cannot be stored in `Options`, and syntax hooks still need `dyn` inside the tokenizer.
- Evidence:
  - Mature Rust hosts compose statically inside and use boxed trait objects at the extension boundary: SWC's `Box<dyn Pass>`, rustc's lint passes, and Bevy's `add_plugins`.
  - Pass 7 writes the alternatives as sketches for side-by-side comparison.
- Revisit if: maintainers find a sketch easier to read than the attacher version.
- Transcript: Q5.

### D6. How plugin-defined nodes are represented

- Question: plugins create nodes the core does not know, such as `wikiLink`. `mdast::Node` is a closed enum with no `data` field. How are these nodes stored?
- Alternatives:
  - A data-driven `Node::Custom { name, attributes, value, children, position }`, chosen:
    - Keeps `Clone`, `PartialEq` and serde, so the node survives a future WASM boundary (D2).
    - Plugin fields are untyped strings.
  - A typed trait object:
    - `Box<dyn CustomNode>`, downcast through `core::any::Any`, which is the markdown-it.rs approach.
    - Plugin authors get typed fields.
    - Loses the derived traits and serde, which blocks D2.
  - A generic node parameter:
    - `mdast::Node<E>` with an application-defined extension enum. Fully typed.
    - The parameter spreads into every signature, and plugins from different crates must be merged into one enum.
- Evidence: markdown-it.rs stores `Box<dyn NodeValue>`, downcast through a `Downcast` supertrait without `unsafe`. Adding a variant to `mdast::Node` breaks semver for `markdown` 1.x.
- Revisit if: plugin authors need typed fields more than a serializable tree.
- Transcript: Q7.

### D7. Plugin ownership: `attach(self)`

- Question: clippy's `needless_pass_by_value` flagged `Processor::plugin`, because it took the plugin by value while `attach(&self)` only borrowed it. How should ownership work?
- Alternatives:
  - `attach(self)` consumes the plugin, chosen:
    - A plugin moves its options into the closures it registers, without cloning. Inline closure plugins become `FnOnce`.
    - A plugin value is used up.
    - `Box<dyn Plugin>` can no longer call `attach`. A configuration-driven plugin list would store `Box<dyn FnOnce(&mut Processor)>` instead.
  - Keep `attach(&self)` and suppress the lint:
    - Plugins stay reusable, and `dyn Plugin` stays callable.
    - Needs a lint suppression, and authors clone options.
  - Take `&impl Plugin`, clippy's own suggestion: call sites become `.plugin(&Gfm)`.
  - Store attached plugins, as Bevy does: gives the ownership a use, but the prototype has no use for stored plugins yet.
- Revisit if: configuration-driven loading, like unified-engine's config files, becomes a goal.
- Transcript: Q10.

### D8. Inline closure plugins need a type annotation

- Question: `.plugin(|p| ..)` fails with E0282. A closure passed through a trait bound cannot infer its argument type, so callers write `|p: &mut Processor|`. Should the API work around it?
- Alternatives:
  - Keep one `.plugin` method and accept the annotation, chosen: struct plugins and transform closures are unaffected.
  - Add `.plugin_fn(|p| ..)`: closures infer, but users face two ways to add a plugin, where unified has one.
- Revisit if: inline plugins turn out to be the common case.
- Transcript: Q9.

### D9. Where URL protocol checks live

- Question: the tree path rendered `[a](javascript:alert(1))` as a live link, while `to_html` drops unsafe protocols by default. Where should the check run?
- Alternatives:
  - In the HTML serializer, chosen:
    - Checks `href` and `src` against `to_html`'s safe-protocol lists, unless `allow_dangerous_protocol` is set.
    - Covers URLs that plugins add.
  - In mdast → hast: mirrors `to_html` exactly, but URLs created by plugins go unchecked.
  - Record only: leaves the processor unsafe for untrusted input.
- Evidence:
  - micromark checks protocols in its HTML compiler, at `packages/micromark/dev/lib/compile.js:803-816`.
  - `mdast-util-to-hast` 13.2.1 only normalizes URLs, and `hast-util-to-html` has no protocol logic.
  - In unified, protocol checks are opt-in through `rehype-sanitize`, whose GitHub schema uses the same lists.
  - react-markdown checks every URL attribute by default, in a final tree pass before output.
- Revisit if: the serializer should cover every URL attribute, as react-markdown does, rather than `href` and `src` only.
- Transcript: Q11.

### D10. How custom nodes become HTML

- Question: a plugin that adds `Custom` nodes must say what HTML they produce. What does it register?
- Alternatives:
  - A handler that receives the already-converted children, chosen:
    - Signature: `Fn(&mdast::Custom, Vec<hast::Node>) -> Vec<hast::Node>`.
    - No converter state becomes public API.
    - Over WASM, it is one pure call per node.
    - A handler cannot convert only some of its children, or read link definitions.
  - A context object, as in `mdast-util-to-hast`:
    - Its readme documents `State` as public, including `all`, `one`, and the footnote and definition maps.
    - In Rust the context could be opaque, with methods only. Those methods still become `markdown` 1.x API.
    - Over WASM, each `state.all` is a host round trip.
  - Data only, like `data.hName` / `hProperties` / `hChildren`:
    - No closures, and the most WASM-friendly.
    - The least flexible, and `Custom` grows HTML-shaped fields.
- Evidence: micromark shows the cost of public internals. Its public `TokenizeContext` type carries `_gfmTableDynamicInterruptHack`, documented as an "Internal boolean shared with `micromark-extension-gfm-table`" and marked "next major: remove".
- Revisit if: a plugin needs selective conversion. An opaque context could be added then.
- Transcript: Q12.

### D11. How `Custom` stores plugin fields

- Alternatives:
  - `BTreeMap<String, String>`, chosen: unique keys, serialized as a JSON object like `mdast-util-directive`'s `attributes` (`Record<string, string>`). Keys come out sorted.
  - `Vec<(String, String)>`: keeps insertion order and matches the Rust-side precedents (hast properties, MDX JSX attributes). The JSON is an array of pairs, and duplicate keys are possible.
- Transcript: Q13.

### D12. How the tree path serializes HTML

- Alternatives:
  - Match `markdown::to_html`, chosen: ` />` on void elements, encoding of `&`, `<`, `>` and `"`, and `disabled=""`. Output from both paths can then be compared directly, and users see the same style from either.
  - Match `rehype-stringify`: closer to unified, but the output differs from `to_html` on the same input.
- Related: the mdxjs-rs converter wrote HTML-encoded URLs into the tree, which is correct for JSX output. With HTML output, those URLs were encoded twice. The converter now stores URLs normalized but unencoded, as `mdast-util-to-hast` does with `normalizeUri`.

### D13. Syntax extensions as a state machine, not a scanner

- Question: how does a syntax extension read input?
- Alternatives:
  - Steps in the existing tokenizer state machine, chosen and implemented in pass 5:
    - The same model as micromark's `effects.consume` / `enter` / `exit`.
    - Correct inside containers.
  - A scanner over the source bytes, like markdown-it.rs's `InlineRule::run(state) -> Option<(Node, usize)>`: easier to write.
- Evidence: `subtokenize` (`src/subtokenize.rs:78-115`) feeds text to the tokenizer in chunks. Line prefixes such as a blockquote's `> ` are skipped between chunks. A scanner over `bytes[pos..]` would read those prefixes as content.
- Result: a wikilink inside a block quote, `> a [[x]]`, links correctly. So does a test construct on the second line of a block quote.
- Revisit if: the interface proves too hard to use, as D20 describes. A scanner could sit on top of it for single-line constructs.

### D14. GFM alert matching rules

- Question: which block quotes become alerts? GitHub documents only the basic syntax.
- Rules, with the alternatives considered:
  - Nesting:
    - Top-level block quotes only, chosen. GitHub's docs say "Alerts cannot be nested within other elements".
    - Any block quote: more forgiving, but diverges from GitHub.
  - Text after the marker on the same line, `> [!NOTE] a`:
    - Not an alert, chosen. The marker must be followed by a line ending, a hard break, or the end of the paragraph. Ordinary quotes that start with `[!X]` stay quotes.
    - An alert with the text as content: more forgiving, and may diverge from GitHub.
  - Marker case:
    - Case-insensitive, chosen. `[!note]` matches, and the class uses the lowercased kind, as rehype-github-alerts does.
    - Uppercase only: matches every documented example, and lowercase markers stay text.
  - Empty alert, `> [!WARNING]` alone:
    - The alert with its title only, chosen. The marker alone decides.
    - Keep the block quote: avoids empty boxes, but the meaning of the marker then depends on what follows.
- Evidence:
  - GitHub docs, "Basic writing and formatting syntax", Alerts section.
  - rehype-github-alerts readme: titles Note, Tip, Important, Warning and Caution, and its examples put a hard break after the marker.
- Unverified: GitHub's own behavior for same-line text and lowercase markers. The rules above follow recollection, not a test against github.com.
- The marker line may end with any line ending, `\r\n`, `\n` or `\r`, because `to_mdast` keeps line endings in text as written.
- Accepted gaps:
  - No octicon SVG in the title.
  - No `dir="auto"`.
  - The trimmed first text node keeps its original position.
- Revisit if: a comparison against github.com output shows a difference.
- Transcript: Q14 to Q16.

### D15. Public `wrap` helper for handlers

- Question: alert HTML needs line endings between block children, like every built-in block handler. Handlers receive already-converted children and no converter state (D10). How do they get the same formatting?
- Alternatives:
  - Make the converter's stateless `wrap(nodes, loose)` public, chosen: it is the equivalent of JS `state.wrap`, and adds one free function to the API.
  - Each plugin reimplements it: duplicated knowledge across crates, and output can drift from the built-in handlers.
- Consequence: D10's pre-converted signature needs stateless helpers exported next to it. Other helpers may follow, such as JS `state.patch` for positions.

### D16. Wikilinks versus CommonMark reference links

- Question: today `[[Home]]` with a definition `[Home]: /x` renders `[<a href="/x">Home</a>]`. When both readings apply, which wins?
- Alternatives:
  - The wikilink wins, chosen:
    - Plugin constructs run before built-ins for the same byte, so `[[` is claimed first. This matches Obsidian.
    - It changes output for documents that relied on `[[label]]` resolving to a bracketed reference link.
  - The reference wins: preserves existing output, but the construct needs read access to parsed definitions, which widens the public tokenizer interface.
- Evidence: `to_html` on `main` renders `[[Home]]` with a definition as `[<a href="/x">Home</a>]`, and `[[a](b)]` as `[<a href="b">a</a>]`.
- Revisit if: users need wikilinks mixed with reference-style links that use the same labels.
- Transcript: Q18.

### D17. Wikilink href

- Alternatives:
  - Base plus the normalized target, chosen: `WikiLinks::new("/wiki/")` gives `/wiki/My%20Page#Intro`. The `#` fragment is kept, URL normalization matches other links, and there is one option.
  - Base plus a slug, like `/wiki/my-page`: friendlier URLs, but the slug rule is a new policy.
  - A caller-supplied function, like remark-wiki-link's `pageResolver` and `hrefTemplate`: the most flexible, and it still needs a default.
- Revisit if: a real site needs slugs. A caller function could be added without breaking the base-only constructor.
- Transcript: Q19.

### D18. Empty wikilink alias

- Alternatives:
  - `[[Page|]]` renders like `[[Page]]`, chosen: forgiving, and the pipe is ignored.
  - Not a wikilink: strict, and the typo stays visible.
- Unverified: Obsidian's behavior.
- Transcript: Q20.

### D19. Syntax extensions and the event-based `to_html`

- Question: syntax plugins live in `ParseOptions`, which `to_html_with_options` also reads. Only the tree path knows how to render a plugin node. What does `to_html` emit for a construct's bytes?
- Alternatives:
  - The construct's source text, HTML-encoded, chosen: no content is lost, and the output is safe. The extension has no effect on the event path.
  - An error when a text construct is registered: explicit, but it breaks a call that otherwise works.
  - Nothing, which was the plan's original assumption: the simplest, but it silently loses text.
- Revisit if: the event path should render extensions too. That would need an HTML hook on the construct, like micromark's `htmlExtensions`.
- Transcript: Q21.

### D20. The construct interface a plugin author sees

- Question: what does a syntax plugin implement, and what does the core expose?
- Shape, implemented:
  - `markdown::extension::TextConstruct` has three methods:
    - `markers() -> &[u8]`: the bytes the construct can start at.
    - `step(state: u16, &mut ConstructTokenizer) -> Step`: one step of the state machine.
    - `to_mdast(&[Token]) -> mdast::Node`: turns one match into a node.
  - `ConstructTokenizer` exposes only `current`, `consume`, `enter(&'static str)` and `exit(&'static str)`.
  - `Step` is `Next(u16)`, `Retry(u16)`, `Ok` or `Nok`.
  - The core's private `Tokenizer`, `Event`, and the built-in states stay hidden.
- Alternatives:
  - Expose the real `Tokenizer`: full power, including `attempt`, `check` and the shared scratch fields. It makes every tokenizer internal public API, the micromark `TokenizeContext` problem from D10.
  - Give each match a small scratch value, such as a counter: friendlier for constructs that count markers, like `$$` math. It widens the interface before any feedback.
  - Let constructs define their own state enum through an associated type: typed states, but the trait could no longer be stored as `Box<dyn TextConstruct>`.
- Findings from the wikilink plugin:
  - Numbered states read like micromark's `effects`. Any memory must be encoded as extra states, for example "only whitespace so far" versus "seen content".
  - Construct indices are `u8`, so at most 255 constructs.
  - Nested constructs are not possible. A construct cannot attempt another construct, or parse its content as markdown, so a wikilink alias cannot contain emphasis.
  - Token values come from direct byte slices. They ignore virtual spaces from tabs, which the built-in `Slice` handles.
- Revisit if: a second syntax plugin, such as math or a block construct, needs `attempt`, scratch storage, or nested content.

### D21. Plugin constructs that span lines

- Question: a construct may consume line endings. Inside a container, the tokenizer skips each new line's prefix, `> ` or list indentation, but token text is sliced from the source. So a token that crosses a line would include the prefix. The layer 3 review reproduced this: `> {{a\n> b}}` gave `{{a\n> b}}`. How is this made correct?
- Requirement: plugins must be able to handle multi-line constructs.
- Alternatives:
  - Single-line constructs: a line ending looks like the end of input to a construct.
    - The smallest fix.
    - Rejected, because it rules out the requirement.
  - Automatic split and join, chosen:
    - Authors `consume()` line endings like any byte.
    - The facade closes the construct's open tokens before a line ending and emits a construct line-ending event.
    - It reopens the same tokens at the next step, after the skipped prefix.
    - The compilers treat the fragments and line endings as one match, and join each token's fragments.
    - `Token.value` becomes a `Cow<str>`, copying only for multi-line tokens.
  - Explicit line endings, micromark style: authors exit their tokens, call `line_ending()`, and re-enter them.
    - Closer to how micromark's own constructs are written.
    - Every plugin must get containers right.
    - An outer token that spans lines still needs the same joining in the compilers.
- Evidence:
  - micromark gets prefix-free token text in two ways. First, `micromark-util-subtokenize` writes prefix-free chunks into the text tokenizer (`dev/index.js:200-214`), and `sliceSerialize` reads from those chunks (`create-tokenizer.js:609` `sliceChunks`). Second, constructs such as code (text) emit explicit `lineEnding` tokens (`code-text.js:191-195`).
  - markdown-rs tokens are byte offsets into the source. The built-ins never let a text token cross a line ending.
- Revisit if: constructs need their content parsed as markdown. That would reuse markdown-rs's linked-content mechanism, a larger redesign.

### D22. Guardrails on plugin constructs

- Question: a buggy construct can hang or panic on user input in release builds. For example, `Ok` without progress loops forever, an unbalanced `exit` panics, and a token ending mid-UTF-8 panics in `to_html`. Who enforces the rules?
- Alternatives:
  - The facade enforces them, chosen. A violation turns the step into `Nok`, so the bytes stay text.
  - Document the rules and keep debug assertions: the smallest option, but production input can still hang or crash through a plugin.
- Rules enforced, as revised after the second layer 3 review:
  - Every consumed byte is inside a token, and one outermost token holds the others.
  - `consume` at the end of input does nothing.
  - `exit` must match the construct's innermost open token.
  - `Next` requires a `consume` in that step, and `Retry` must not follow one. At most 256 `Retry` steps run in a row.
  - After consuming a line ending, the step may only `exit` tokens that end there, and must return `Next`. The tokenizer skips the next line's container prefix only when the next step starts.
  - `Ok` requires the last event to be the exit of one of this construct's tokens, so a match never ends with a line ending. It also requires progress since the construct started, no open tokens, and a character boundary.
  - `Ok` does not require a `consume` in the same step, so a construct can end just before a line ending. After `Ok`, the attempt machinery treats the current byte as consumed and hands it to the next state.
  - Closing a token that stayed empty up to a line ending breaks the construct, just like an empty token on one line.
  - Token boundaries must fall on character boundaries.
  - Line-ending bytes are never markers.
  - A token that is still empty at a line ending is dropped, and starts after it rather than breaking the construct. This makes `{{`-then-newline constructs possible, with LF and CR+LF behaving the same.
- Plugin-side: the trait docs tell authors to bound lookahead. A multi-line construct that scans to the end of a paragraph before failing is quadratic, because it is retried at every marker.
- Transcript: Q24.

### D23. Wikilinks inside link text

- Question: `[x [[a]] y](z)` produced nested `<a>` elements. Built-in links deactivate outer label starts, and a construct cannot.
- Alternatives:
  - The plugin also registers an mdast transform that turns wikilinks inside links into their text, chosen. The HTML is valid, and it shows one plugin combining syntax, transform and handler.
  - Leave the nesting, and record it as a limit of constructs.
- Transcript: Q25.

### D24. Whitespace-only wikilink alias

- Alternatives:
  - `[[a| ]]` renders like `[[a]]`, chosen. This is consistent with D18, the empty alias rule.
  - Link with a single-space label.
- Transcript: Q26.

### D25. Cost of the extension point when no plugins are registered

- Question: callgrind showed that the extension point added +7% to +15% instructions to every parse with no plugins registered. The parse-speed session measured the same thing independently: +10% to +13.5% on its small-document bins. How is that removed?
- Causes, from per-function callgrind profiles on a document of about 1 KB:
  - `state::Name` gained data-carrying variants, so `State`'s `PartialEq` became an out-of-line call in the tokenizer loop, and `state::call` stopped being inlined.
  - `event::Name` gained a `&'static str`, so every `Event` grew from 80 to 104 bytes, and `memcpy` rose with it.
- Alternatives, with measured `to_html` overhead on the ~1 KB document:
  - State fix only: unit-only state variants, with the construct index and step kept in `TokenizeState`. +7.2%, and the API is unchanged.
  - Numeric event ids only: +11.2%.
  - Both, with names interned per parse in a `RefCell` in `ParseState`, chosen. +1.0%, and the API is unchanged.
  - Both, with constructs declaring their token names through a `tokens()` method: the same cost, and no `RefCell`, but a trait method every plugin must implement.
- How unified compares: micromark states are functions, and token types are engine-interned strings, so openness costs nothing extra there. markdown-rs's closed enums buy speed, so plugin data must sit beside the hot paths, not inside them.
- Result on the branch, against `main`:
  - `to_mdast`: -0.56% to +0.28%.
  - `to_html`: +0.75% to +1.31%. The rest is the per-event extension check in the compiler.
- A follow-up, from the parse-speed session's per-document observation: without constructs, `text_markers` returns an empty `Vec`, and text uses the built-in `MARKERS`. This saves one allocation per parse, and moved tiny-document `to_html` from +1.68% to +1.31%.
  - A registered construct that never matches adds +0.3% to +1.3%.
- Revisit if: the remaining `to_html` cost matters. The per-event `is_extension` check could move into the existing `match` arms.
- Transcript: Q31.

## Clarifying questions and answers

1. no_std boundary: "markdown-rs is #![no_std] + alloc and forbids unsafe code. A plugin API can keep those guarantees everywhere, or relax them for plugins that want std. Which boundary should the design hold?"
   Answer: Everything no_std. The plugin API and every reference plugin stay no_std + no unsafe.
2. Packaging: "Plugins need a way to reach the processor. Rust offers compile-time composition of crates, or runtime loading through WASM or dynamic libraries. What should the design target?"
   Answer: Compiled-in, leave room for WASM. Keep the plugin boundary serializable (mdast via serde), so a separate WASM host crate can be added later.
3. Output path: "markdown-rs compiles HTML straight from parser events, so an mdast transform has no effect on to_html today. Which path should the prototype build?"
   Answer: mdast → hast → html (Recommended).
4. Syntax demo: "Which syntax should the syntax-plugin prototype implement?"
   Answer: Wikilinks [[Page|alias]] (Recommended).
5. API shape: "Which shape should the prototype build end to end? The other shapes get short sketches against the same plugin."
   Answer: Attacher trait (Recommended). Selected preview: `Processor::new().plugin(WikiLinks::new("/wiki/")).plugin(GfmAlerts).process(..)`, where `impl Plugin` has `fn attach(&self, p: &mut Processor)` calling `p.add_syntax(..)`, `p.add_hast_handler("wikiLink", ..)` and `p.add_mdast_transform(..)`.
6. Transformer: "The real transformer plugin should prove the mdast → hast → html path end to end. Which plugin should it be?"
   Answer: GFM alerts (Recommended).
7. Custom nodes: "Plugins create nodes the core does not know. Which representation should the prototype use?"
   Answer: Data-driven Custom variant (Recommended). This is `mdast::Node::Custom { name, attributes, value, children, position }`.
8. Deliverable: "Where should the results land?"
   Answer: Local branch + findings doc (Recommended). Nothing is pushed or posted without a separate go-ahead.

9. Spike shape, after passes 1 and 2: "Is this shape right to stabilize?" (inline closure plugins need `|p: &mut Processor|`, E0282 without it).
   Answer: Confirm, stabilize now (Recommended). The closure annotation is recorded as a finding, with no workaround method.
10. Clippy `needless_pass_by_value` on `Processor::plugin`: "Which fix?"
    Answer: attach(self) consumes (Recommended). `Plugin::attach(self, &mut Processor)`, and the closure blanket impl is `FnOnce`.
11. Review found that `javascript:` URLs pass through the tree path: "Where should protocol checks live?"
    Answer: In the serializer (Recommended). Checked against the ecosystem:
    - micromark `compile.js:803-816,1132` checks at compile time.
    - `mdast-util-to-hast` 13.2.1 only calls `normalizeUri`, and `hast-util-to-html` has no protocol logic.
    - `rehype-sanitize` is opt-in, and its schema uses the same lists.
    - react-markdown runs `defaultUrlTransform` over all `html-url-attributes` in a final hast pass.
12. Handler API for `Custom` nodes, after comparing `mdast-util-to-hast`'s public `State` handlers and `data.hName`. Its readme `### State` documents `all`, `one`, and the footnote and definition maps. micromark's `TokenizeContext` carries leaked `_gfm*` internals marked "next major: remove".
    Answer: Pre-converted children (Recommended). The signature is `Fn(&mdast::Custom, Vec<hast::Node>) -> Vec<hast::Node>`.
13. `Custom` attributes collection.
    Answer: `BTreeMap<String, String>` (Recommended). It serializes as a JSON object, like `mdast-util-directive`'s `attributes`.
14. Alerts, text on the marker line: "`> [!NOTE] Some text`: how should the plugin treat it?"
    Answer: Not an alert (Recommended).
15. Alerts, marker case: "Should the marker match case-insensitively?"
    Answer: Case-insensitive (Recommended).
16. Alerts, empty alert: "Should `> [!NOTE]` alone still become an alert box?"
    Answer: Alert with title only (Recommended).
17. Layer 2 shape: "Is this shape right to stabilize?", which includes the public `wrap`.
    Answer: Confirm, stabilize now (Recommended).
18. Wikilinks versus references: "Which should win when both could apply?"
    Answer: Wikilink wins (Recommended).
19. Wikilink href: "How should `[[My Page#Intro]]` become an href?"
    Answer: Base + normalized target (Recommended).
20. Empty alias: "How should `[[Page|]]` parse?"
    Answer: Show the target (Recommended).
21. Event-based `to_html` with a registered construct: "What should the wikilink's bytes become?"
    Answer: Source text, encoded (Recommended).

Assumptions stated at the pass 5 checkpoint:
- A newline or an empty target means not a wikilink.
- `[[a]b]]` is not a wikilink.
- The target is trimmed for the href and displayed as written.
- Embeds (`![[x]]`) are out of scope.
22. Layer 3 shape: "Is this shape right to stabilize?" The interface has numbered states and no scratch storage.
    Answer: Confirm, stabilize now (Recommended).
23. Layer 3 review: "How should the review run?"
    Answer: Sub-agent, thorough (Recommended).
24. Construct guardrails: "Should the facade enforce the rules?"
    Answer: Enforce in the facade (Recommended).
25. Wikilinks inside link text: "What should the plugin do?"
    Answer: Unwrap inside links (Recommended).
26. Whitespace-only alias: "How should it behave?"
    Answer: Treat as no alias (Recommended).
27. Multi-line constructs: the developer asked for more depth several times, then stated the requirement: "I need plugins able to handle multiline constructs".
28. Line endings in constructs: "Which way should plugin authors deal with line endings?"
    Answer: Automatic (Recommended).
29. Review abstraction proposal for the wikilink part-byte rule, which appeared in three guards.
    Answer: Extract `is_part_byte` (Recommended).
30. Review abstraction proposal for the construct-token or-pattern, repeated at five sites.
    Answer: Extract `construct_token` (Recommended).
31. Extension overhead fix: "How should the prototype fix it?" The developer asked for more depth and for a comparison with unified/remark first.
    Answer: Both fixes, interned ids (Recommended).
32. Review of the marker-allocation follow-up: "How should its review run?"
    Answer: Skip, commit (Recommended).
33. The unused `extern crate std;` in `src/util/gfm_tagfilter.rs`, found on `main`: "Where should that fix go?"
    Answer: Separate local branch (Recommended), `fix/gfm-tagfilter-no-std`.
34. Wrap-up: "What now?"
    Answer: Complete, unwind WIP (Recommended).

## Assumptions (override at approval)

- Branch `feat/plugin-prototypes`, created from `origin/main` after a fetch. This plan is copied to `plans/32-plugin-prototypes.md` on that branch, and the findings go to `plans/32-plugin-findings.md`. Both stay local.
- Prototype quality, not merge quality. Tests cover each behavior the findings rely on. There is no public-API polish, no docs pass on every item, and no `to_markdown` support for `Custom`.
- `to_html` / `to_html_with_options` keep compiling from events and stay unchanged. The tree pipeline is a new, separate entry point, `Processor`. Extension events are ignored by the event-based `to_html`. That gap is recorded as a finding and not fixed.
- New crates declare `#![no_std]` and `#![forbid(unsafe_code)]` and use edition 2018, matching `mdast_util_to_markdown`. Adding `#![forbid(unsafe_code)]` to the existing `markdown` crate is outside this task. The research found the guarantee is currently a readme policy only, so it is noted in the findings.
- Plugin-side ordering: syntax extensions run before the built-in constructs for their marker byte, in registration order. Transforms run in registration order, first added runs first, matching unified's `.use()` order.
- The mdast → hast port reuses `wooorm/mdxjs-rs` `src/mdast_util_to_hast.rs` and `src/hast.rs`. The MDX node arms are dropped. That is copied code, not spike code, so the 50-line spike cap applies to the new logic around it.

## Research summary (feeds the findings doc)

Source: the deep-research run, with 105 agents, 23 sources, and 20 claims confirmed at 3-0. Refuted claims are excluded.

- Composition: mature hosts use static composition inside and `Box<dyn Trait>` at the extension boundary.
  - Examples: SWC `Pass` with tuple impls and `Box<dyn Pass>`, rustc combined lint passes, and bevy's marker-generic `add_plugins` that stores plugins boxed.
  - A single object-safe `&mut root` method is the common transform shape.
- Registration: `linkme`, `inventory` and `ctor` do not fit.
  - `linkme` has no documented wasm support, and a crate that is never referenced is not linked.
  - `inventory` runs code before main, and embedded targets largely fail.
  - Explicit `.plugin(x)` registration is the portable choice.
- Visitors: `syn` and SWC `VisitMut` show the idiom, and the known hazard is an override that forgets to recurse.
  - Safe Rust cannot hand a visitor `&mut node` plus its parent. oxc_traverse does it with raw-pointer unsafe code.
  - unified-style ancestor access needs a safe substitute: index paths, parent-first callbacks, or collect-then-mutate.
- Ordering: tower runs the first-added layer first, and axum runs the last-added first. markdown-it.rs uses named `before`/`after` constraints plus a topological sort.
- WASM boundary: SWC's rkyv AST forced lockstep plugin upgrades. SWC moved to self-describing CBOR in Nov 2025, and plugins still break when a field is removed or changes type.
  - Keeping mdast serde-friendly, which is why `Custom` is data-driven, keeps that door open. It also makes mdast a versioned wire format.
- markdown-it.rs is the closest prior art for syntax plugins. It declares `#![forbid(unsafe_code)]`, and its latest release, 0.6.1, is from 2024-07.
  - `InlineRule` has `const MARKER: char`, `fn run(state: &mut InlineState) -> Option<(Node, usize)>`, and `check`. It is a zero-sized type with associated fns, and `run` slices `state.src[pos..pos_max]`.
  - Custom nodes are `Box<dyn NodeValue>`, downcast through a `Downcast` supertrait with no unsafe. Rendering lives on the node as `render(&self, node, fmt: &mut dyn Renderer)`.
  - Ordering uses `before(mark)`, `after(mark)`, `before_all` and `require` on `Ruler`, with a topological sort.
- pulldown-cmark: `Event`/`Tag` are closed enums with no syntax-extension API. Transforms are iterator adapters.
- comrak 0.55: closed `NodeValue` enum on an arena of `RefCell` nodes. Its hooks are render adapters, `SyntaxHighlighterAdapter` and `HeadingAdapter` (PR #266, by the author of #32), plus per-node-type overrides of the HTML formatter. No custom node kinds or syntax were found, and there is no no_std evidence. The exact adapter signatures are unverified.
- rushdown 0.18: goldmark-style. `parser_extension(|p| p.add_inline_parser(Constructor::new, options, PRIORITY_EMPHASIS + 100))` returns `impl ParserExtension`. Ordering is a numeric priority, unlike markdown-it.rs's named `before`/`after`. It has a `no-std` feature, which limits entities to numeric and predefined ones. The trait bodies and any `unsafe` use are unverified.
- WASM hosts: wasmi supports no_std. A future WASM plugin host could therefore stay no_std. Extism builds on wasmtime and appears std-only.
- Why the syntax facade is a state machine rather than a markdown-it-style scanner: `subtokenize` (`src/subtokenize.rs:78-115`) feeds text to the tokenizer chunk by chunk through linked events. A construct therefore sees one logical stream while byte indexes skip container prefixes. A scanner over `bytes[pos..]` would read the `> ` of a following blockquote line. The scanner shape is easier to write, so the findings doc compares the two.

## Design under test

The plugin crates use only the public API. That proves how plugins compile together across crate boundaries.

```rust
// markdown_processor (new workspace crate)
pub trait Plugin {
    fn attach(&self, processor: &mut Processor);
}

impl Processor {
    pub fn new() -> Self;
    pub fn plugin(mut self, plugin: impl Plugin) -> Self; // attaches immediately
    pub fn add_syntax(&mut self, construct: impl markdown::extension::TextConstruct + 'static);
    pub fn add_mdast_transform(&mut self, f: impl Fn(&mut mdast::Node) -> Result<(), Message> + 'static);
    pub fn add_hast_transform(&mut self, f: impl Fn(&mut hast::Node) -> Result<(), Message> + 'static);
    pub fn add_hast_handler(&mut self, name: &str, f: impl Fn(&mut ToHast, &mdast::Custom) -> Vec<hast::Node> + 'static);
    pub fn process(&self, input: &str) -> Result<String, Message>; // parse → mdast transforms → to_hast → hast transforms → to_html
}
```

Syntax extension point in the `markdown` crate, micromark-faithful. Constructs are a state machine driven by the existing tokenizer:
- `state::Name::Extension(u8, u16)` and `event::Name::Extension(&'static str)`. Both stay closed enums with one open variant each.
- `text::before` tries registered constructs for the current byte before the built-ins, falling back to the built-in path.
- The text marker set becomes the built-in markers plus the extension markers, computed once per parse. The `&'static [u8]` field becomes `&'a [u8]`.
- A public facade, `markdown::extension::{TextConstruct, ConstructTokenizer, Step}`, exposes `current`, `consume`, `enter`, `exit`, and `Next(u16) / Retry(u16) / Ok / Nok`. The private `Tokenizer` and `event::Event` stay unexported.
- `TextConstruct::to_mdast(&self, tokens: &[Token], bytes: &[u8]) -> mdast::Node` turns the construct's outer token into a node inside `to_mdast::compile`.
- `ParseOptions` gets a `text_constructs` field in the style of the MDX callbacks: boxed, `#[serde(skip)]`, and printed by the hand-written `Debug` impl.

## Spike passes (each 50 lines or fewer of new logic, then stabilize)

1. Tree HTML path: add a minimal `hast_util_to_html` over the ported `mdast_util_to_hast`. It handles elements, text escaping, void elements, comments, doctype and `Raw`. Check: `# a *b*` → `<h1>a <em>b</em></h1>`.
2. Processor + attacher: the `Plugin` trait, `Processor`, the mdast and hast transform lists, and `visit_mut` pre-order over `children_mut()`. Check: a closure transform uppercases text and the HTML reflects it.
3. Custom nodes + handlers: `mdast::Node::Custom`. Wire it through `Debug`, `ToString`, `children`, `position` and serde. Add the handler registry in to_hast. Check: a hand-built `Custom` renders through a handler.
4. GFM alerts plugin crate: `plugins/gfm_alert`. It turns a `Blockquote` whose first text starts `[!NOTE|TIP|IMPORTANT|WARNING|CAUTION]` into `Custom("alert")`. The handler emits GitHub's `div.markdown-alert.markdown-alert-note` + `p.markdown-alert-title`. Check: GitHub's documented alert examples.
5. Tokenizer extension point: `Name::Extension` dispatch in `state::call`, dynamic markers, and the `text::before` hook. Check: a throwaway extension that claims `@` produces an extension event.
6. Wikilinks plugin crate: `plugins/wiki_link`. The state machine handles `[[target]]` and `[[target|alias]]`, then `to_mdast` produces `Custom("wikiLink")` and the hast handler produces `<a href="/wiki/target">`. Check: basic, alias, empty, unclosed, and newline inside. Also check nesting in a blockquote, adjacency to `[text](url)`, and the conflict with a defined `[target]: url`.
7. Comparison sketches: in `markdown_processor/examples/`, the alerts plugin written as per-stage traits and as a static generic `Chain<A, B>`. These are for the findings doc's side-by-side and are not stabilized beyond compiling.
8. Risk measurements:
   - A CommonMark spec parity harness that parses `commonmark-data.txt` examples and compares event-path vs tree-path HTML. Record the mismatch count by category.
   - A criterion bench, `to_html` vs `Processor::process`, on `readme.md`.
   - A serde JSON round-trip of an mdast containing `Custom`, as the WASM-room check.
   - `cargo build --target thumbv7em-none-eabihf` for `markdown`, `markdown_processor` and both plugins, as the no_std check. This needs `rustup target add thumbv7em-none-eabihf`.

9. Survey gap-fill for the findings tables, capped at 4 fetches: rushdown's parser-extension trait and comrak's `Plugins` / adapter signatures. Mark anything unretrieved as unverified.

Stabilize after passes 2, 4 and 6, the natural layers:
1. Tighten types.
2. Add one test per behavior.
3. Run the full check loop.
4. Code review, with the developer choosing the path.
5. WIP commit.
6. Progress log.

## Reusable code

- `src/lib.rs:160` `to_mdast`: the parse entry for the Processor.
- `src/mdast.rs:354-483` `children_mut()` / `position()`: the base for `visit_mut`.
- `src/configuration.rs:1211-1253`: the boxed-callback precedent for `ParseOptions`, with the `#[serde(skip)]` field and the hand-written `Debug`/`Default`.
- `src/construct/text.rs:35,64-168`: the marker list and the byte dispatch the hook slots into.
- `src/construct/partial_data.rs:24-65`: data stops at `tokenize_state.markers`.
- `src/state.rs:473` `call`: add the `Extension` arm, dispatched through `tokenizer.parse_state.options`.
- `src/to_mdast.rs:228,262,340`: enter/exit dispatch to extend for `event::Name::Extension`.
- `src/util/sanitize_uri.rs` `sanitize`: URL safety in the wikilink and alert handlers.
- mdxjs-rs `src/mdast_util_to_hast.rs` and `src/hast.rs`: the port source, about 1,300 implementation lines plus about 2,800 test lines.
- `tests/commonmark.rs` options: `allow_dangerous_html` and `allow_dangerous_protocol` for the parity harness.

## Files to modify or create

- `Cargo.toml`: add workspace members.
- `src/lib.rs`: add `pub mod extension`.
- `src/extension.rs`: new facade.
- `src/configuration.rs`: `ParseOptions.text_constructs`, `Debug`, `Default`.
- `src/state.rs`, `src/event.rs`, `src/tokenizer.rs`, `src/construct/text.rs`, `src/construct/partial_data.rs`, `src/parser.rs`: extension dispatch and markers.
- `src/to_mdast.rs`: compile extension tokens.
- `src/mdast.rs`: the `Custom` node.
- `markdown_processor/` (new crate): `hast.rs`, `mdast_util_to_hast.rs` (port), `hast_util_to_html.rs`, `processor.rs`, `visit.rs`, tests, examples, benches.
- `plugins/gfm_alert/` and `plugins/wiki_link/` (new crates): plugin + tests.
- `tests/`: a new syntax-extension test alongside the existing construct tests.
- `plans/32-plugin-prototypes.md` and `plans/32-plugin-findings.md`.

Cross-cutting: this writes under `src/`, `tests/`, the root `Cargo.toml`, and new `markdown_processor/`, `plugins/` and `plans/` directories. The `src/` changes add a public module, a new `mdast::Node` variant, and a new `ParseOptions` field. All three are semver-breaking for `markdown` 1.x, and the findings doc records them.

## Verification

- Full CI loop, each output saved under `.agent-tmp/32-plugin-prototypes/`:
  - `cargo fmt --check`
  - `cargo clippy --all-features --all-targets --workspace`
  - `cargo test --all-features --workspace`
- Existing suites stay green. The baseline is recorded on `main` before the first edit, so any failure can be attributed.
- End to end: `Processor::new().plugin(WikiLinks::new("/wiki/")).plugin(GfmAlerts).process("> [!NOTE]\n> See [[Home|home]]")` gives the expected HTML, checked in a test in each plugin crate.
- no_std: the `thumbv7em-none-eabihf` build for all four crates.
- No unsafe: `#![forbid(unsafe_code)]` on the new crates, plus `grep -rn "unsafe" src/` showing only doc comments.
- Fuzz targets untouched; `cargo test` includes `tests/fuzz.rs`.

## Findings doc outline (`plans/32-plugin-findings.md`)

1. Recommendation and open questions for wooorm, first.
2. Survey table: plugin mechanisms × ergonomics, compile cost, coupling, no_std, unsafe, wasm.
3. Parser table: markdown-it.rs, comrak, pulldown-cmark, rushdown, markdown-rs prototype.
4. API side-by-side: attacher vs per-stage vs static chain, from pass 7.
5. Measured risks: parity mismatches, bench numbers, public-API surface added, and semver breaks.
6. Unresolved: ancestor access, `to_html` ignoring extensions, the `Custom` serde shape (`"type":"custom"` vs the unist `"type":"wikiLink"`), and `forbid(unsafe_code)` on the core crate.

## Review notes

Layer 1 review (processor), quick tier, sub-agent:
- Fixed, critical: URLs were HTML-encoded twice, because the port used `sanitize`. The port now stores `normalize` output.
- Fixed, critical: dangerous protocols passed through. The serializer now checks `href`/`src` unless `allow_dangerous_protocol` is set.
- Fixed, advisory:
  - `Gfm` no longer resets other constructs.
  - NUL now becomes U+FFFD.
  - Comment values can no longer close the comment.
  - The docs say MDX is dropped.
  - The new crate declares `rust-version = "1.56"`.
  - Two tests are sharpened so their assertions can fail.
- Deferred, advisory: `visit_mut` has no skip or exit control. A transform that wraps `Text` in `Emphasis{Text}` recurses forever. This is a findings item next to ancestor access, the unist-util-visit `SKIP`/`EXIT` equivalent.
- Deferred, advisory: GFM footnotes differ from `to_html`. They have no `user-content-` clobber prefix, and the id is `#fn-1` rather than `fn-1`. Both come from the mdxjs-rs port; the parity harness in pass 8 measures them.
- Deferred, advisory: the serializer protocol-checks only `href` and `src`. react-markdown covers every `html-url-attributes` entry.
- The re-review found no critical findings. Its advisory items are fixed:
  - comment edges `>`, `->` and a trailing `<!-` are escaped;
  - raw HTML NUL becomes U+FFFD when dangerous HTML is allowed;
  - the export is named `normalize_uri`, after micromark's `normalizeUri`.

Layer 2 review (`Custom` nodes and alerts), quick tier, sub-agent:
- Fixed, critical: alerts never triggered in files with Windows line endings. `to_mdast` keeps `\r\n` in text nodes. The marker may now be followed by `\r\n`, `\n` or `\r`.
- Fixed, advisory:
  - An unknown `kind` no longer panics; it falls back to `note`.
  - The node is named `gfmAlert` to avoid clashes with other plugins.
  - The alert `div` keeps the source position.
  - An empty text node left before inline content is removed. Positions whose start moved are cleared.
  - `transform_custom` converts children only when they are used. Converting registers footnote calls, so a dropped subtree used to leave dangling footnotes.
  - A serde test was moved so it no longer steals a doc comment.
- Recorded, parity: the tree path always emits `\n` between blocks, through `wrap`, while `to_html` reuses the document's first line ending. Pass 8 measures this.
- The re-review found no critical findings. Its advisory is fixed: the empty-text test could not fail, so `to_alert` now has unit tests on the tree itself. Integration tests cover the `div` position and the unknown-kind fallback.

Layer 3 review (syntax extensions), thorough tier, sub-agent:
- Critical, latent, fixed by D21: token text was sliced from the source, so a construct token that crosses a line inside a container kept the prefix. Reproduced: `> {{a\n> b}}` gave `{{a\n> b}}` in mdast, and `&gt; b` in HTML. Tokens are now split at line endings and joined again.
- Advisory, fixed by D22: a buggy construct could hang or panic on user input in release builds. The facade now enforces the construct rules, and a violation becomes `Nok`.
- Advisory, fixed: `to_html` found the outer token by name through `Position::from_exit_event`, which loses text when an inner token reuses the outer name. It now records the outer enter index.
- Advisory, fixed by D23 and D24: wikilinks nested inside link text, and whitespace-only aliases.
- Advisory, fixed: dead `\r` guards, which the tokenizer never delivers. The review's abstraction proposal became `is_part_byte`, the developer's choice.
- Advisory, found by probing, fix pending: when a list-item continuation prefix ends partway through a tab, `to_mdast` kept the tab and `to_html` dropped it. The built-in code (text) drops it, so `collect_tokens` must slice through `Slice`, like `to_html` does.
- Second review, critical, fixed: a construct call made after consuming a line ending, in the same step, ran before the next line's prefix was skipped.
  - An `Ok` there made `collect_tokens` read past the match: a panic, or a lost later node.
  - An `enter` there started before the `> `.
  - Fixed by the line-ending rule in D22.
- Second review, advisory, fixed:
  - The empty-token check disagreed between LF and CR+LF. It now compares against the moved-back enter point.
  - Two outermost tokens made `to_mdast` call the construct twice. That is now rejected.
  - The guardrail tests could not tell a rejected construct from a matched one through `to_html`. They now assert through `to_mdast`.
- Third review, critical, fixed: a match could still end on a line ending one step later, when a construct consumed `\n`, returned `Next`, and then `Ok`. The single invariant "`Ok` needs the last event to be this construct's token exit" now covers the same-step and next-step paths. Tests were mutation-checked: they fail with the guard disabled.
- Fourth review: no critical or advisory findings. It traced that `collect_tokens` always stops inside a match, for every input.
- Third review, advisory, fixed:
  - Closing a dropped, empty token breaks the construct.
  - The `enter` guard test now fails without the guard.
  - `current()` docs say it is `None` after `consume` until the next step.
- Second review, abstraction, the developer's choice: `construct_token` replaces the or-pattern for "first fragment or continuation" at four sites.
- Overhead-fix review (D25), thorough tier:
  - No stale reads of the `extension_*` fields, no `RefCell` conflicts, and both enums are cheap again.
  - Critical, fixed: `examples/parity.rs` embedded the gitignored `commonmark-data.txt` through `include_str!`, which would break CI, because CI compiles examples. It now reads the file at runtime.
  - Advisory, fixed:
    - A few trimmed comments were the only record of a rule a future edit could undo. They are back as one-liners.
    - A misleading doc line is fixed.
    - The empty-code regression case was added to `tree_html.rs`.
    - The static-chain sketch gained `parse` and `hast` hooks for a fair comparison.
- `event::Name` now has data-carrying variants; see D25 for the measured cost and the fix. Every `Event` may be larger, even with no plugins, so this needs a benchmark against the previous WIP commit. If it regresses, a numeric token id keeps `Name` narrow.

## Progress log

Baseline on `main` (1506572), before any edit:
- fmt passes.
- clippy passes, with one existing warning: `implicit_saturating_sub` at `src/util/char.rs:44`.
- Tests: 198 passed, 0 failed.
- Known failure: on a fresh lockfile, `swc_common` 8.1.1 does not compile with serde 1.0.228, because `serde::__private` was renamed `__private228`. That blocks `clippy --all-targets` and every `markdown` test target.
- Local workaround: a patched copy in the scratchpad, applied only through `cargo {clippy,test} --config patch.crates-io...`, as in `.agent-tmp/32-plugin-prototypes/check.sh`. No tracked file changes.

Layer 1 (passes 1 and 2), processor, stabilized:
- `markdown_processor` crate:
  - `hast` is ported from mdxjs-rs, without the MDX nodes and with `Raw`.
  - `mdast_util_to_hast` is ported, with 32 of its tests.
  - `hast_util_to_html` has `Options { allow_dangerous_html, allow_dangerous_protocol }`.
  - `Processor` has `Plugin::attach(self)`, an `FnOnce` blanket impl, mdast and hast transform lists, and the `Gfm` plugin.
  - `visit_mut` walks mdast in preorder.
- `markdown`: hidden exports `normalize_uri`, `sanitize_with_protocols`, `SAFE_PROTOCOL_HREF` and `SAFE_PROTOCOL_SRC`.
- Covered:
  - plugin order;
  - struct and closure plugins;
  - parser configuration;
  - transform errors stopping the pipeline;
  - hast transforms;
  - raw HTML and protocol safety, on and off;
  - attribute mapping, encoding and comment escaping;
  - parity with `to_html` for 17 CommonMark and URL cases, plus 4 dangerous-mode cases.
- Verified: `cargo fmt --check`, `cargo clippy --all-features --all-targets --workspace`, and `cargo test --all-features --workspace` pass, with 249 tests passed and 0 failed.
- Spike note: the pass 1 serializer was about 75 lines, above the 50-line spike cap. It is logged here, not split after the fact.
- Findings so far:
  - Inline closure plugins need a `|p: &mut Processor|` annotation (E0282).
  - `attach(self)` removes `dyn Plugin` attach.
  - mdxjs-rs's `sanitize` in to-hast is correct for JSX, which decodes entities, but encodes twice when the output is HTML.

Layer 2 (passes 3 and 4), `Custom` nodes and GFM alerts, stabilized:
- `markdown`: `mdast::Node::Custom(Custom { name, attributes: BTreeMap<String, String>, value, children, position })`, wired into every accessor, with serde as `{"type":"custom",...}`. This is a semver-breaking new variant.
- `markdown_processor`:
  - `Handler` / `Handlers` types and `mdast_util_to_hast_with_handlers`.
  - The handler receives children already converted. The fallback matches `defaultUnknownHandler`: text of a value, or a `div` of the children.
  - `Processor::add_hast_handler`.
  - `wrap` is public.
- `plugins/gfm_alert`: the `GfmAlert` plugin. Its rules are in D14.
- Covered:
  - `Custom` accessors, `Debug` and `ToString`;
  - the serde JSON shape and round trip;
  - handler use and replacement, and the `div` and text fallbacks;
  - each alert kind;
  - case-insensitive markers;
  - hard break, CRLF and CR after the marker;
  - same-line text rejected;
  - empty alert;
  - nested quotes and list items rejected;
  - unknown kinds rejected, or defaulted when another plugin builds the node;
  - block content kept;
  - tree shape and positions after trimming;
  - the `div` position.
- Verified: `cargo fmt --check`, `cargo clippy --all-features --all-targets --workspace`, and `cargo test --all-features --workspace` pass, with 270 tests passed and 0 failed.
- Spike note: pass 4's plugin was about 90 lines of logic, above the 50-line cap. It is logged, not split.
- Findings so far:
  - The pre-converted handler signature pushes stateless helpers like `wrap` into the public API.
  - Plugins must handle every line-ending form themselves, because mdast text keeps them as written.

Layer 3 (passes 5 and 6), syntax extensions and wikilinks, stabilized in WIP commit f2f43d8:
- `markdown` crate:
  - Public `extension` module: `TextConstruct`, `ConstructTokenizer`, `Step` and `Token` (with a `Cow` value).
  - `ParseOptions.text_constructs`.
  - `state::Name::{TextBeforeConstruct, TextConstruct}` and `event::Name::{Extension, ExtensionContinuation, ExtensionLineEnding}`.
  - Text markers per parse, and the construct chain in `text::before`.
  - Line-ending splitting and joining (D21), and the enforced rules (D22).
  - `to_mdast` and `to_html` hooks.
- `markdown_processor`: `Processor::add_syntax`.
- `plugins/wiki_link`: the `WikiLinks` plugin, with its syntax, an mdast transform that unwraps wikilinks inside links, and a hast handler.
- Covered:
  - A construct makes a node from its tokens and positions.
  - A failed construct falls back to text.
  - Constructs work in containers, run before built-ins, and are tried in order.
  - `to_html` writes source text.
  - Multi-line tokens join without container prefixes, in block quotes, list items (including tab prefixes) and at top level.
  - Every broken-construct case keeps text: `Ok` or `Next` without progress, endless `Retry`, consuming outside a token, a mismatched exit, an open token at `Ok`, two outermost tokens, an empty token, a token inside a character, and ending on a line ending in the same step or one step later.
  - Line endings are never markers.
  - Empty tokens start after a line ending, with LF and CR+LF alike.
  - Consuming at the end of input does nothing.
  - Every decided wikilink rule.
- Verified: `cargo fmt --check`, `cargo clippy --all-features --all-targets --workspace`, and `cargo test --all-features --workspace` pass, with 302 tests passed and 0 failed.
- Review: thorough tier, four rounds, with three critical findings fixed.
- Spike note: passes 5 and 6 were about 150 core lines plus a 140-line plugin, above the 50-line cap. They were split by verb, tokenize (5a) and compile (5b), but each part is still larger than the cap.

Passes 7 and 8, comparison sketches and measurements:
- Sketches in `markdown_processor/examples/`, compile-only:
  - `per_stage.rs`: users must wire the alerts transform and handler separately.
  - `static_chain.rs`: the chain type grows as `Processor<Chain<Chain<Base, Alerts>, Nothing>>`, and handlers still end up boxed, because the converter takes `Handlers`.
- CommonMark parity, from `examples/parity.rs` over the 652 spec examples:
  - The event path, `to_html`, matches the spec on all 652.
  - The tree path ends without the final line ending that `to_html` keeps when the input has one, on 650 of 652.
  - Ignoring that, 7 examples differ. The mdast is already wrong in all 7, so none is the tree path's fault:
    - `to_mdast` drops the virtual spaces of indented code under a tab prefix: 3 examples.
    - `to_mdast` keeps code (text) padding line endings (`"\nfoo\n"`): 3 examples.
    - `to_mdast` drops a nested image's alt (`"foo "`): 1 example.
  - The port also appended `\n` to empty code. That is fixed; it was 4 more examples.
- Instructions, by callgrind with no plugins registered, against `main`:
  - Layer 3 as first built: +7% to +15%.
  - After D25: `to_mdast` -0.24% to +0.28%, and `to_html` +0.74% to +1.68%.
  - Layer 2: within ±0.3% of `main`.
  - The processor's tree path costs +9% to +13% over `to_html`.
  - Wall-clock criterion runs were thrown away: the load average was about 15 on 12 cores, and repeated runs of unchanged code differed by up to 2×.
- `no_std`, on `thumbv7em-none-eabihf`:
  - `markdown` at `main` (1506572) does not build, because `src/util/gfm_tagfilter.rs` has an unused `extern crate std;`. This is a known issue, outside this task.
  - With that one line removed in a scratch copy, `markdown`, `markdown_processor`, `gfm_alert` and `wiki_link` all build.

Pass 9 and wrap-up:
- `plans/32-plugin-findings.md` is written for a maintainer who was not in this session. It passes Vale with no errors.
- Follow-up to D25: text markers no longer allocate per parse without constructs. Final no-plugin cost against `main` is -0.56% to +1.31%. Its review was skipped, as the developer chose.
- The `no_std` fix goes on a separate local branch, `fix/gfm-tagfilter-no-std`, created from `main`.
- The WIP commits were unwound into staged changes on `feat/plugin-prototypes`, for the developer's own commits. Nothing was pushed or posted.
