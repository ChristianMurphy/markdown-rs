---
status: Active
---

# Plugin prototype findings for markdown-rs (#32)

## Summary

Issue #32 asks for custom plugins in markdown-rs. The discussion there treats syntax extensions as impossible in Rust.

A prototype on branch `feat/plugin-prototypes` shows that both kinds of plugin work in safe, `no_std` Rust:
- Tree transforms: plugins edit mdast (the markdown syntax tree) or hast (the HTML syntax tree) before the HTML is written.
- Syntax extensions: a plugin adds a construct that runs as a state machine inside markdown-rs's own tokenizer.

With no plugins registered, markdown-rs costs -0.56% to +1.31% instructions against 1.0.0. A first version cost +7% to +15%, and the fix is recorded below.

Two real plugins exercise the design. Each is its own crate that uses only public APIs:
- `gfm_alert` turns GitHub alert block quotes (`> [!NOTE]`) into alert boxes. It is a transform plugin.
- `wiki_link` adds `[[Page]]` and `[[Page|alias]]` links. It is a syntax plugin.

The new tree path renders HTML through mdast and hast. It matches `to_html` on 645 of the 652 CommonMark spec examples, apart from a final line ending. The other 7 differ because `to_mdast` already builds a wrong mdast for them.

### Recommendation

Adopt the plugin system in stages. Each stage stands on its own:
1. The tree path and transform plugins:
   - an mdast → hast → HTML pipeline;
   - a `Processor` with one `Plugin` trait;
   - a `Custom` mdast node for plugin nodes;
   - hast handlers.

   This adds no cost to the existing parser, and it answers most requests in the issue tracker: alerts, heading ids, link attributes, and custom HTML.
2. Syntax extensions: the `markdown::extension` interface. It works, and costs about 1% with no plugins registered. It adds more public API to the core crate, so it should come second.

### Open questions for the maintainer

- Where should the `Processor`, hast and the converters go: into the `markdown` crate, into one new crate, or into separate packages as in unified? The prototype uses one crate, `markdown_processor`.
- `mdast::Node::Custom` and `ParseOptions.text_constructs` break semver for `markdown` 1.x. Should they wait for 2.0, or hide behind a feature, or should `Node` become `#[non_exhaustive]`?
- `Custom` serializes as `{"type": "custom", "name": "wikiLink"}`. Should it instead serialize as `{"type": "wikiLink"}`, the unist shape, with a hand-written serde implementation?
- The tree path writes HTML in `to_html`'s style: void elements end in ` />`, and `&`, `<`, `>` and `"` are encoded. Should it follow `rehype-stringify` instead?
- Should `to_html`, which compiles from events, render plugin constructs? Today it writes their source as text. Rendering them would need an HTML hook on the construct, like micromark's `htmlExtensions`.

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

A syntax construct is a state machine over the input, in the style of micromark's `effects`:

```rust
impl TextConstruct for WikiLinkSyntax {
    fn markers(&self) -> &[u8] {
        b"["
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (START, Some(b'[')) => {
                t.enter("wikiLink");
                t.enter("wikiLinkMarker");
                t.consume();
                Step::Next(OPEN)
            }
            // ...
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: &[Token]) -> mdast::Node {
        // Build a `Custom` node from the token values.
    }
}
```

Properties of the construct interface:
- The core's private `Tokenizer` stays hidden. A construct can call only `current`, `consume`, `enter`, and `exit`.
- Constructs run before the built-in constructs at their marker bytes, in registration order.
- Constructs may span lines. Inside a block quote or list, the core closes tokens at each line ending and reopens them after the next line's prefix. Token text never contains `> ` or list indentation.
- The core enforces the construct rules. A construct that breaks one does not match, and its input stays plain text. A buggy plugin cannot hang or crash the parser.

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

| Document | Function | 1.0.0 | Prototype, first version | Prototype, fixed |
| --- | --- | --- | --- | --- |
| tiny | `to_html` | 102,491 | +8.5% | +1.31% |
| tiny | `to_mdast` | 106,475 | +6.7% | -0.56% |
| small | `to_html` | 1,734,079 | +15.1% | +0.98% |
| small | `to_mdast` | 1,783,824 | +13.9% | +0.20% |
| `readme.md` | `to_html` | 18,991,149 | +11.9% | +0.87% |
| `readme.md` | `to_mdast` | 19,361,314 | +11.0% | +0.17% |

The table shows CommonMark options. GFM options give the same picture.

A separate parse-speed harness measured the first version on its own corpus, across 5 option sets:
- documents under 256 bytes: +9.9% to +10.8%;
- documents of 256 bytes to 4 KB: +12.4% to +13.5%.

It found no output differences against 1.0.0 across 507,090 comparisons.

The same harness re-measured the fix without the last change, which stops allocating a marker list per parse. It again found no output differences, across 1,007,160 comparisons:
- documents under 256 bytes: `to_mdast` +0.84% to +1.28%, and `to_html` +1.48% to +1.79%;
- documents of 256 bytes to 4 KB: `to_mdast` -0.11% to +0.34%, and `to_html` +0.63% to +0.81%.

Three changes brought the overhead down, measured cumulatively with `to_html` on the 1 KB document:

| Step | What was slow | Fix | Overhead |
| --- | --- | --- | --- |
| First version | | | +15.1% |
| 1. Keep parser states free of data | `state::Name` gained `TextConstruct(u8, u16)`. With data in the enum, the derived `State` comparison in the tokenizer loop became a function call, and `state::call` stopped being inlined. | Unit variants `TextBeforeConstruct` and `TextConstruct`. Three `TokenizeState` fields hold the construct index and step. | +7.2% |
| 2. Keep events small | `event::Name` gained `Extension(u8, &'static str)`, so every `Event` grew from 80 to 104 bytes, and every copy of an event cost more. | Events hold `(u8, u16)`: the parser interns token names per parse, in `ParseState`. | +1.0% |
| 3. No setup per parse without plugins | Each parse copied the text marker list into a new `Vec`. | Without constructs, text uses the static marker list. On the 13-byte document, `to_html` went from +1.68% to +1.31%. | +1.0% |

The remaining cost in `to_html` is a check on each event for construct tokens.

Other costs:
- A registered construct that never matches adds +0.3% to +1.6%.
- The tree path, through the `Processor`, costs +9.2% to +13.0% over `to_html` on the same document. That is the price of building mdast and hast.

### `no_std`

`markdown`, `markdown_processor`, `gfm_alert` and `wiki_link` all build for `thumbv7em-none-eabihf`, a target without the standard library, with one exception.

`markdown` 1.0.0 itself does not build for that target. `src/util/gfm_tagfilter.rs` has an unused `extern crate std;`, and with that line removed, all four crates build. Each new crate declares `#![forbid(unsafe_code)]`.

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
| markdown-rs prototype | yes: a state machine inside the tokenizer, with multi-line support | `mdast::Node::Custom` (string fields, serde) | registration order, before the built-ins | yes | none |

A source scanner, as in markdown-it.rs, is easier to write. It would read container prefixes as content, so it cannot be correct inside block quotes and lists with markdown-rs's chunked tokenizer. So the prototype drives constructs from the tokenizer.

## API shapes compared

The prototype built the attacher design, and wrote the other two as compile-only sketches in `markdown_processor/examples/`.

| Shape | What users write | Trade-off |
| --- | --- | --- |
| Attacher, one `Plugin` trait (built) | `.plugin(WikiLinks::new("/wiki/"))` | one concept, like `.use()`; inline closure plugins need a `\|p: &mut Processor\|` annotation |
| A trait per stage (`per_stage.rs`) | `.mdast(AlertTransform).hast_handler("alert", alert_handler)` | small traits, but users wire each package's pieces themselves |
| Static generic chain (`static_chain.rs`) | `Processor(Base).with(Gfm).with(Alerts)` | no boxing for transforms; the type grows as `Processor<Chain<Chain<Base, Gfm>, Alerts>>`, and handlers still end up boxed |

## Public API added to `markdown`

- `pub mod extension`: `TextConstruct`, `ConstructTokenizer`, `Step` and `Token`.
- `ParseOptions.text_constructs: Vec<Box<dyn TextConstruct>>`: a new public field, which breaks struct literals.
- `mdast::Node::Custom(Custom)`: a new variant, which breaks exhaustive matches.
- Hidden re-exports: `normalize_uri`, `sanitize_with_protocols`, `SAFE_PROTOCOL_HREF` and `SAFE_PROTOCOL_SRC`, used by the tree path and by plugins.

## Known limits

- Syntax constructs cannot nest, and cannot parse their content as markdown. A wikilink alias cannot contain emphasis. Lifting this would reuse markdown-rs's linked-content mechanism, which is a larger design.
- Only text constructs, that is inline ones, exist. Block extensions are not designed.
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

## Where to look

- Branch `feat/plugin-prototypes`.
- Design decisions, with every alternative weighed and a "revisit if" for each: `plans/32-plugin-prototypes.md`, D1 to D25.
- Core crate: `src/extension.rs` holds the construct interface, `src/mdast.rs` has `Custom`, and the hooks are in `src/construct/text.rs`, `src/to_mdast.rs` and `src/to_html.rs`.
- The new crate `markdown_processor/`:
  - `processor.rs` has `Plugin` and `Processor`.
  - `mdast_util_to_hast.rs` is ported from mdxjs-rs.
  - `hast_util_to_html.rs` serializes hast.
  - `examples/` holds the parity report and the API sketches.
- Plugins: `plugins/gfm_alert/` and `plugins/wiki_link/`.
- Tests:
  - `tests/extension.rs` covers the construct interface, including multi-line tokens and every rule the core enforces.
  - Each crate has behavior tests.
  - The workspace passes 302 tests.
