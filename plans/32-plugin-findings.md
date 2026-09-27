---
status: Active
---

# Plugin prototype findings for markdown-rs (#32)

## Summary

Issue #32 asks for custom plugins in markdown-rs. The discussion there treats syntax extensions as impossible in Rust.

A prototype on branch `feat/plugin-prototypes` shows that both kinds of plugin work in safe, `no_std` Rust:
- Tree transforms: plugins edit mdast (the markdown syntax tree) or hast (the HTML syntax tree) before the HTML is written.
- Syntax extensions: a plugin adds a construct that runs as a state machine inside markdown-rs's own tokenizer, like a micromark construct. Constructs can be inline, block, or container syntax, and can have markdown inside them.

With no plugins registered, markdown-rs costs +0.21% to +0.82% instructions against 1.0.0.

Five plugins exercise the design. Each is its own crate that uses only public APIs:
- `gfm_alert` turns GitHub alert block quotes (`> [!NOTE]`) into alert boxes. It is a transform plugin.
- `wiki_link` adds `[[Page]]` and `[[Page|*alias*]]` links, with markdown in the alias.
- `directive` ports micromark-extension-directive: `:name[label]{attributes}`, `::name` on its own line, and `:::name` containers. It matches all 255 test cases of the JavaScript package.
- `admonition` adds `!!! note "Title"` blocks with an indented body, a container like a block quote.
- `mark` adds `==highlight==`, paired like GitHub Flavored Markdown (GFM) strikethrough.

The new tree path renders HTML through mdast and hast. It matches `to_html` on 645 of the 652 CommonMark spec examples, apart from a final line ending. The other 7 differ because `to_mdast` already builds a wrong mdast for them.

### Recommendation

Adopt the plugin system in stages. Each stage stands on its own:
1. The tree path and transform plugins:
   - an mdast → hast → HTML pipeline;
   - a `Processor` with one `Plugin` trait;
   - a `Custom` mdast node for plugin nodes;
   - hast handlers.

   This adds no cost to the existing parser, and it answers most requests in the issue tracker: alerts, heading ids, link attributes, and custom HTML.
2. Syntax extensions: the `markdown::extension` interface. It covers inline, block, container, and delimiter syntax, with nested markdown, and costs under 1% with no plugins registered. It adds more public API to the core crate, and touches the tokenizer, so it should come second.

### Open questions for the maintainer

- Where should the `Processor`, hast and the converters go: into the `markdown` crate, into one new crate, or into separate packages as in unified? The prototype uses one crate, `markdown_processor`.
- `mdast::Node::Custom` and `ParseOptions.text_constructs` break semver for `markdown` 1.x. Should they wait for 2.0, or hide behind a feature, or should `Node` become `#[non_exhaustive]`?
- `Custom` serializes as `{"type": "custom", "name": "wikiLink"}`. Should it instead serialize as `{"type": "wikiLink"}`, the unist shape, with a hand-written serde implementation?
- The tree path writes HTML in `to_html`'s style: void elements end in ` />`, and `&`, `<`, `>` and `"` are encoded. Should it follow `rehype-stringify` instead?
- Should `to_html`, which compiles from events, render plugin constructs? Today it writes their own text as text, and renders the markdown inside them. Rendering them would need an HTML hook on the construct, like micromark's `htmlExtensions`.
- Constructs are numbered-state machines with one `step` function, where micromark constructs are closures. Is that acceptable to plugin authors? The directive port is about 900 lines against micromark-extension-directive's 1,100.
- A block construct can fail only on its first line: after that, failing is a parse error. It keeps block parsing linear. Is an error the right signal for a plugin bug?
- `Custom` stores attributes in a sorted map, so attribute order is lost. Should it keep source order, as `mdast-util-directive` does?

## What the API looks like

A user attaches plugins to a processor:

```rust
use markdown_processor::{Gfm, Processor};

let html = Processor::new()
    .plugin(Gfm)
    .plugin(gfm_alert::GfmAlert)
    .plugin(wiki_link::WikiLinks::new("/wiki/"))
    .process("> [!NOTE]\n> See [[Home]].")?;
```

A plugin receives the processor and registers syntax, transforms, and handlers, like a unified attacher:

```rust
impl Plugin for WikiLinks {
    fn attach(self, processor: &mut Processor) {
        processor.add_syntax(WikiLinkSyntax);
        processor.add_mdast_transform(|tree| {
            unwrap_in_links(tree);
            Ok(())
        });
        let base = self.base;
        processor.add_hast_handler("wikiLink", move |node, children| {
            // Build `<a href>` from the node's `target` attribute.
        });
    }
}
```

A syntax construct is a state machine over the input, in the style of micromark's `effects`. Where the plugin registers it decides where it runs, like the keys of a micromark extension:

```rust
impl Plugin for Directives {
    fn attach(self, processor: &mut Processor) {
        processor.add_syntax(DirectiveText); // micromark `text`
        processor.parse.flow_constructs.push(Box::new(DirectiveContainer)); // `flow`
        processor.parse.flow_constructs.push(Box::new(DirectiveLeaf));
        // ... hast handlers
    }
}

impl Construct for DirectiveText {
    fn markers(&self) -> &[u8] {
        b":"
    }

    fn previous(&self, previous: Option<u8>) -> bool {
        previous != Some(b':')
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (0, Some(b':')) => {
                t.enter("directiveText");
                t.enter("directiveMarker");
                t.consume();
                t.exit("directiveMarker");
                Step::Next(1)
            }
            // Like `effects.attempt(label, afterLabel, afterLabel)`.
            (2, Some(b'[')) => Step::Attempt { state: LABEL, ok: 3, nok: 3 },
            // ...
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        // A `Custom` node from token values, and the label's children.
    }
}
```

Inside the label, the construct marks its content, like micromark's `contentType`:

```rust
t.enter_content("directiveLabelString", ContentType::Text);
// consume bytes up to the balanced `]`...
t.exit("directiveLabelString");
```

The core parses that content later, with a fresh tokenizer, and `to_mdast` receives its children on the token.

| Syntax | micromark | The prototype | Plugin |
| --- | --- | --- | --- |
| Inline | `text` construct | `text_constructs` | `wiki_link` |
| Markdown inside a construct | chunks with `contentType: 'text'` | `enter_content(name, ContentType::Text)` | directive labels, wikilink aliases |
| Optional parts | `effects.attempt` | `Step::Attempt { state, ok, nok }` | directive labels and attributes |
| Block on its own line | `flow` construct | `flow_constructs` | `::name` |
| Fenced container | `flow` construct with `contentType: 'document'` chunks | `enter_content(name, ContentType::Document)` | `:::name` |
| Container with a prefix per line | `document` construct with `continuation` | `document_constructs` and `Construct::continuation` | `!!! note` |
| Delimiter runs | `attentionMarkers`, and a resolver | `Construct::attention_sizes`, paired by the core | `==mark==` |

Properties of the construct interface:
- The core's private `Tokenizer` stays hidden. A construct calls `current`, `current_char`, `consume`, `enter`, `enter_content`, and `exit`, and can read its indentation, a small `memory`, and the parse options.
- Constructs run before the built-in constructs at their marker bytes, in registration order.
- Constructs see input without container prefixes: `> ` or list indentation never shows up in a token value or in nested content.
- The core enforces the construct rules. A construct that breaks one does not match, and its input stays what it would otherwise be. A buggy plugin cannot hang or crash the parser.
- A block construct decides on its first line. A lazy line or the end of input ends it, as with fenced code.
- Plugins never write resolvers or touch the event list: delimiter runs are declared, and the core pairs them.

## Measurements

### CommonMark parity of the tree path

The parity example runs all 652 CommonMark spec examples through `to_html` and through the tree path, with dangerous HTML and protocols allowed on both.

| Comparison | Examples that differ |
| --- | --- |
| `to_html` against the spec | 0 |
| Tree path against `to_html` | 650 |
| The same, ignoring a final line ending | 7 |

The 650 come from one difference. `to_html` ends with a line ending when the input does. The tree path never does.

In all 7 remaining examples, the mdast from `to_mdast` is already wrong:
- Indented code under a tab prefix loses its virtual spaces, in 3 examples. The value is `"foo"`, where the spec expects `"  foo"`.
- Code (text) keeps its padding line endings, in 3 examples. The value is `"\nfoo\n"`, where the spec expects `"foo"`.
- A nested image loses its alt text, in 1 example. The alt is `"foo "`, where the spec expects `"foo bar"`.

`to_html` avoids these because it compiles from parser events, not from mdast.

### Instruction counts

Callgrind counted instructions inside one call. Wall-clock benchmarks were discarded, because the machine's load average was about 15 on 12 cores.

The documents:
- tiny: 13 bytes.
- small: the first 1 KB of `readme.md`.
- `readme.md`: 11,511 bytes.

With no plugins registered, against 1.0.0:

| Document | Function | 1.0.0 | Prototype |
| --- | --- | --- | --- |
| tiny | `to_html` | 102,491 | +0.82% |
| tiny | `to_mdast` | 106,475 | +0.38% |
| small | `to_html` | 1,734,079 | +0.59% |
| small | `to_mdast` | 1,783,824 | +0.45% |
| `readme.md` | `to_html` | 18,991,149 | +0.40% |
| `readme.md` | `to_mdast` | 19,361,314 | +0.21% |

The table shows CommonMark options. GFM options give the same picture.

The first version cost +7% to +15%. These changes brought it down:

| What was slow | Fix |
| --- | --- |
| `state::Name` gained data variants. The derived `State` comparison in the tokenizer loop became a function call, and `state::call` stopped being inlined. | Unit variants for plugin states. The construct index and step live beside the enum. |
| `event::Name` gained data, first `(u8, &'static str)`, then `(u8, u16)`. Events grew from 80 to 104 bytes in the first case. In the second, `Name` grew from 1 to 4 bytes, and every name comparison and copy cost more. | A fieldless `Name::Extension`. The interned token name is in `Event.extension`, a `u16` in padding, so `Event` stays 80 bytes. This took tiny `to_html` from +1.62% to +0.82%. |
| Each parse copied the text marker list into a new `Vec`. | Without constructs, text uses the static marker list. |
| Plugin state in the tokenizer. | One `Option<Box<..>>`, created when a construct runs: `Tokenizer` is 672 bytes against 664. |

Other costs:
- A registered construct that never matches adds +0.3% to +1.6%.
- The tree path, through the `Processor`, costs +9.2% to +13.0% over `to_html` on the same document. That is the price of building mdast and hast.
- `mdast::Node` is 176 bytes against 152, because `Custom` is the largest variant. Boxing it measured slower in `to_mdast`.

### Fidelity to micromark-extension-directive

The `directive` plugin runs all 255 test cases of micromark-extension-directive's `test/index.js`. The cases were extracted by running that file with `micromark` stubbed, and its test HTML handlers are ported. All 255 match, compared after collapsing whitespace between tags and sorting attributes in a tag. Neither of those is a parsing difference: the whitespace comes from each compiler's line endings, and the order from `Custom`'s sorted attribute map.

### `no_std`

`markdown`, `markdown_processor`, and the five plugin crates all build for `thumbv7em-none-eabihf`, a target without the standard library, with one exception.

`markdown` 1.0.0 itself does not build for that target. `src/util/gfm_tagfilter.rs` has an unused `extern crate std;` (upstream PR #221 removes it), and with that line removed, all seven crates build. Each new crate declares `#![forbid(unsafe_code)]`.

## Survey: how Rust plugin systems work

Sources: a deep-research pass with 23 sources and 20 claims confirmed three votes to zero, plus targeted reading of docs and code. "Unverified" marks claims no source confirmed.

| Mechanism | Examples | Ergonomics | Cost | `no_std` and no `unsafe` | Fit here |
| --- | --- | --- | --- | --- | --- |
| Trait objects, `Box<dyn Trait>` | SWC `Box<dyn Pass>`, rustc lint passes | one concept, easy to store in lists | one virtual call per hook | yes | the prototype's choice |
| Static composition through generics and tuples | tower `ServiceBuilder`, SWC tuple `Pass` impls, bevy `add_plugins` | long types in compiler errors | monomorphized, and slower to compile | yes | only inside a host: handlers and syntax still need `dyn` (see `examples/static_chain.rs`) |
| Closures and function pointers | markdown-it.rs rule payloads, SWC `fn_pass` | light for one-off hooks | as trait objects | yes | accepted as plugins (`FnOnce(&mut Processor)`) |
| Link-time registration | `linkme`, `inventory`, `ctor` | plugins register themselves | none at run time | `inventory` runs code before `main` | rejected: `linkme` has no documented wasm support, and an unreferenced crate is not linked |
| Dynamic libraries | `libloading`, `abi_stable` | load at run time | FFI | needs `unsafe`, and Rust has no stable ABI | rejected |
| WebAssembly hosts | SWC (wasmer), extism (wasmtime), wasmi | sandboxed, any language | serialization per call | wasmi supports `no_std` | possible later: mdast is serde-friendly, so it can cross a wasm boundary |

Lessons from the sources:
- Mature hosts compose statically inside and box at the extension boundary.
- Once an AST crosses a binary boundary, it becomes a versioned wire format. SWC's layout-exact `rkyv` boundary broke every plugin on every AST change. The self-describing CBOR that replaced it in Nov 2025 still breaks plugins when a field is removed.
- Registration order changes behavior. tower runs the first-added layer first, axum the last-added first, and markdown-it.rs uses named `before` and `after` constraints.
- Safe Rust cannot give a visitor `&mut node` plus access to its parent. oxc gets that through raw-pointer `unsafe` code.

## Survey: pluggable markdown parsers in Rust

| Parser | Syntax extensions | Custom nodes | Ordering | `no_std` | `unsafe` |
| --- | --- | --- | --- | --- | --- |
| markdown-it.rs 0.6.1 | yes: inline and block rules over the source (`run(state) -> Option<(Node, usize)>`) | `Box<dyn NodeValue>`, downcast without `unsafe` | named `before`/`after` constraints, sorted topologically | not found | `#![forbid(unsafe_code)]` |
| rushdown 0.18 | yes: `add_inline_parser(constructor, options, priority)` | unverified | numeric priority | `no-std` feature | unverified |
| comrak 0.55 | no: render adapters and per-node formatter overrides only | no, a closed `NodeValue` enum | none | no evidence | unverified |
| pulldown-cmark | no: iterator adapters over a closed `Event` enum | no | none | unverified: yes, with `hashbrown` | unverified: only in an opt-in SIMD feature |
| markdown-rs prototype | yes: state machines inside the tokenizer, for inline, block, container, and delimiter syntax, with nested markdown | `mdast::Node::Custom` (string fields, serde) | registration order, before the built-ins | yes | none |

A source scanner, as in markdown-it.rs, is easier to write, but it would read `> ` and list indentation as content. So the prototype drives constructs from the tokenizer, and, like micromark, marks nested content to parse later.

## API shapes compared

The prototype built the attacher design, and wrote the other two as compile-only sketches in `markdown_processor/examples/`.

| Shape | What users write | Trade-off |
| --- | --- | --- |
| Attacher, one `Plugin` trait (built) | `.plugin(WikiLinks::new("/wiki/"))` | one concept, like `.use()`; inline closure plugins need a `\|p: &mut Processor\|` annotation |
| A trait per stage (`per_stage.rs`) | `.mdast(AlertTransform).hast_handler("alert", alert_handler)` | small traits, but users wire each package's pieces themselves |
| Static generic chain (`static_chain.rs`) | `Processor(Base).with(Gfm).with(Alerts)` | no boxing for transforms; the type grows as `Processor<Chain<Chain<Base, Gfm>, Alerts>>`, and handlers still end up boxed |

## Public API added to `markdown`

- `pub mod extension`: `Construct`, `ConstructTokenizer`, `ContentType`, `Step`, and `Token`.
- `ParseOptions.text_constructs`, `flow_constructs`, and `document_constructs`, each a `Vec<Box<dyn Construct>>`: new public fields, which break struct literals.
- `mdast::Node::Custom(Custom)`: a new variant, which breaks exhaustive matches. `Custom` has `name`, `fields`, `attributes`, `value`, `children`, and `position`.
- Hidden re-exports: `normalize_uri`, `sanitize_with_protocols`, `SAFE_PROTOCOL_HREF`, `SAFE_PROTOCOL_SRC`, and `classify_character`, used by the tree path and by plugins.

## Known limits

- A construct finds the end of its content by scanning bytes. A `]]` inside a code span in a wikilink alias ends the alias early, as it does for micromark directive labels.
- A container keeps one word of memory across lines, like micromark's `containerState`; a match has four words.
- Delimiter runs pair only with the same size on both sides, as GFM strikethrough does.
- A lazy line continues the innermost paragraph in a plugin container, as in block quotes. Python-Markdown admonitions work differently.
- At most 255 constructs, and 65,536 distinct token names per parse.
- Transforms get `visit_mut` without skip, exit, or ancestor access. A transform that wraps text in a node that contains text recurses forever.
- The tree path:
  - It omits the final line ending.
  - GFM footnote ids differ from `to_html`: there is no `user-content-` prefix, and a stray `#` appears in `id="#fn-1"`. Both come from the mdxjs-rs port.
  - Only `href` and `src` are checked for dangerous protocols. react-markdown checks every URL attribute.
- Unrelated problems found on `main` along the way:
  - The 3 `to_mdast` bugs described under CommonMark parity.
  - The unused `extern crate std;` that stops `markdown` from building for a `no_std` target.
  - A fresh `Cargo.lock` breaks the dev dependencies, because `swc_common` 8.1.1 does not compile with serde 1.0.228.
  - An unclosed fenced code block at the end of a list item makes the next item a nested list: `- ```\n  a\n\n  b\n- c`.

## Where to look

- Branch `feat/plugin-prototypes`.
- Design decisions, with every alternative weighed: `plans/32-plugin-prototypes.md`, D1 to D25, and, for block constructs and nested content, `plans/32-block-nested-syntax.md`, D26 to D43.
- Core crate: `src/extension.rs` holds the construct interface, `src/mdast.rs` has `Custom`. The hooks are in `src/construct/{text,flow,document,attention}.rs`, `src/parser.rs`, `src/subtokenize.rs`, `src/to_mdast.rs`, and `src/to_html.rs`.
- The new crate `markdown_processor/`:
  - `processor.rs` has `Plugin` and `Processor`.
  - `mdast_util_to_hast.rs` is ported from mdxjs-rs.
  - `hast_util_to_html.rs` serializes hast.
  - `examples/` holds the parity report and the API sketches.
- Plugins: `plugins/gfm_alert/`, `plugins/wiki_link/`, `plugins/directive/`, `plugins/admonition/`, and `plugins/mark/`.
- Tests:
  - `tests/extension.rs` covers the construct interface: nested content, block and container constructs, attempts, delimiter runs, and every rule the core enforces.
  - `plugins/directive/tests/micromark.rs` runs the 255 ported micromark-extension-directive cases.
  - Each crate has behavior tests.
  - The workspace passes 330 tests.
