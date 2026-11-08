# Formal assurance: no panics, bounded work, safe output

## Task summary

Show, as strongly as is worth the cost, that no input makes markdown-rs panic, run long, or write unsafe HTML, for the core and for constructs from plugins. Get there in steps that each help on their own, toward a possible full deductive proof later.

This is a separate effort from the syntax extensions of PR 270, built on top of them, and delivered as its own upstream series after PR 270.

## Goals and constraints, as the developer set them

| Topic | Choice |
|---|---|
| Strength | Layered: property fuzzing with oracles, restriction lints, bounded proofs with Kani, and deductive proofs, starting small, toward a possible full proof |
| Path | Incremental; each step pays off on its own, in engineering and for the community |
| Scope | The core parser and compilers; the extension runtime; a kit for plugin authors; the MDX boundary |
| Properties | No panics; linear work; HTML safety; equivalence of the two render paths; round-trip; positions; conformance |
| Oracles | micromark (JavaScript), cmark and cmark-gfm, and later a Lean reference |
| Continuous fuzzing | Prepare OSS-Fuzz, and ask wooorm, who would be its contact; meanwhile, time-boxed fuzzing in GitHub Actions |
| Done for panic-freedom | All four: no unjustified panic sites; Kani bounds met; a fuzzing budget clean; the extension runtime proven |
| Deductive tool | Chosen by a spike of Verus against Aeneas (to Lean) |
| Delivery | An upstream series after PR 270, one property or tool per commit; the library keeps MSRV 1.62, `no_std`, and the 1% callgrind gate; proof tools run in separate jobs on their own toolchains |
| Bindings | Python, Node, and WASM may come later: proofs must hold for the `no_std` core on every target, wasm32 included |

## Where the project stands

- About 37,800 lines of production code in `src/`, without tests. Explicit panic sites: 86 `unwrap()`, 90 `expect(…)`, 52 `unreachable!`, 2 `panic!`, 48 `assert!`, and 47 `debug_assert!`. About 340 index or slice expressions, which can panic out of bounds or, on a `str`, off a character boundary. Most `expect`s state invariants of the event stream ("expected tag"), not input checks.
- The hard Rust features are in one place: trait objects, `Box<dyn Any>`, and `RefCell` appear only in the extension runtime (`src/extension.rs`, about 3,500 lines) and in two MDX callback types. The rest is enums, structs, `Vec`s, slices, and a trampoline over a `match` on state names, with about 115 loops.
- Tests: about 2,300, 100% line coverage in CI (tarpaulin). The fuzz targets (libFuzzer, honggfuzz) call `to_html` and `to_mdast` with default, GFM, and MDX options, and check only that nothing panics. No construct is fuzzed in the repository; round 2 of PR 270 fuzzed the five ports on the fork.
- markdown-rs is not in OSS-Fuzz; cmark, pulldown-cmark, and md4c are. comrak had advisories for quadratic inputs (GHSL-2023-047 to 049).
- PR 270 already bounds constructs at run time: a step budget of 256 per byte reached, attempts 256 deep, 32 passes over content, `Reach`, and `assert_linear`, with no error from a late failure.

## Research

### Tools

| Tool | Proves | Code impact | Fit here | Adoption |
|---|---|---|---|---|
| Kani (bounded model checking) | No panic, overflow, or out-of-bounds, for all inputs up to a bound; unbounded with contracts, which are experimental | `cfg_attr` and `cfg(kani)` harnesses, no dependency, MSRV unchanged | Handles `dyn`, `Any`, `RefCell`; whole parser only up to small inputs | AWS; Rust std verification; high |
| bolero | One harness for fuzzers and Kani | Dev-dependency | Shares harnesses between steps 1 and 2 | Used by s2n-quic |
| Verus (SMT) | Panic-freedom and functional properties, unbounded | Attribute mode, erased by default, behind a feature; pins stable Rust | Core fits; the runtime needs a refactor or trusted wrappers; 7.5:1 proof-to-code reported; one proof covers 32- and 64-bit `usize` | Largest community; Amazon in production; Vest |
| Aeneas + Charon (to Lean) | The same, in Lean | None: proofs live in a separate Lean project | Core fits; runtime needs a refactor; proofs break when the code changes | Inria and Microsoft; SymCrypt, 16.7k lines of Rust at about 14:1 |
| Creusot (Why3) | The same | Erased attributes; nightly; edition-2024 dependency | Best at `&mut`; little `dyn` | Inria; "very experimental" |
| Flux (refinement types) | Bounds and panic-freedom; weak on functional properties | `cfg_attr`; nightly; git-only crate | No `dyn` | Small; Tock |
| hax (to F*, Lean via Aeneas) | The same | Dependency; nightly | No `&mut` returns | Shaky after its lead left |
| `no-panic` crate | No panic left after optimization, at link time | Attribute | Small leaf functions only | |
| Clippy restriction lints | Not a proof: every panic site visible, removed, or justified | Lints in CI | Whole crate | |
| cargo-mutants | Whether the tests catch changes | None | Per module, on diffs | Mature |

None of them verifies the glue of PyO3, napi-rs, or wasm-bindgen; a proof of the `no_std` core is what keeps parser panics from becoming a Python exception or a wasm trap. None covers running out of memory or stack.

### Literature

- Verified parsers prove soundness, completeness, and termination against a grammar: CompCert's validated LR(1) parser (Jourdan, Pottier, Leroy, ESOP 2012), CakeML, Vermillion (ITP 2019), CoStar (PLDI 2021), TRX (ESOP 2010), a packrat parser in PVS (CPP 2020), total parser combinators (ICFP 2010), Verbatim (CPP 2022).
- Formats prove round-trips and non-malleability: EverParse (USENIX Security 2019, in Windows), Narcissus (ICFP 2019), Vest (USENIX Security 2025, Verus, Rust), EverCBOR (2025, F* to safe Rust).
- Tree transforms prove semantic preservation by simulation: CompCert and CakeML passes.
- Markdown: lean-markdown (Lean 4, 2026, not peer-reviewed) proves totality, that leaf text is escaped, and well-formed HTML without raw HTML; conformance is tested only. NASA JPL's L4YAML (Lean 4, 2026) proves a YAML round-trip, with an audit that refuted one claim.
- Missing: a formal grammar or semantics of CommonMark, which is not context-free and is defined by procedures in prose; a verified HTML5 tokenizer; a sanitizer with a machine-checked proof of XSS-freedom; a mechanized worst-case time bound for a production parser; any work on plugins that cannot break their host.

## Properties

| Property | Needs a grammar? | Checked by |
|---|---|---|
| No panics, for all inputs | No | Lints, fuzzing, Kani, then the deductive tool |
| Termination and linear work: steps per byte bounded | No | Step counter; fuzzing that maximizes steps per byte; pathological tests; later a proof of the counter |
| HTML safety without dangerous options: no `<script>`, no event attributes, only safe URL protocols, output re-parses to the same tree | No; an HTML model | Fuzzing with an HTML5 parser as oracle; Kani on the encoder and URL checks |
| Well-formed output: balanced events, nested tags | No | Fuzzing; then proofs of the event stack |
| Positions: on character boundaries, nested, monotone | No | Fuzzing; then refinements or invariants |
| Equivalence of `to_html` and the `Processor` path | No | Differential fuzzing; later a simulation proof |
| Round-trip: `to_markdown`, then parse, gives the same mdast | A normal form | Fuzzing |
| Conformance with CommonMark and GFM | Yes, or an executable reference | Spec suites; differential fuzzing against micromark and cmark; later a Lean reference |
| Plugin isolation: a construct cannot unbalance events, move points back, or exceed its budget | No | The runtime's rules; Kani on the runtime; new territory for proofs |

## Decisions

### F1. Specs as plain predicates first

- Chosen: write invariants as ordinary Rust functions returning `bool` (events balanced, points monotone, offsets on boundaries, output escaped), used in `debug_assert!`, the fuzzers, and Kani. They read as documentation, need no tool, and are the easiest to accept upstream. Verus can lift them (`dual_spec`), and Aeneas can state them in Lean.
- Alternatives: rustc's experimental `core::contracts` (nightly); one tool's spec language from the start.

### F2. Property fuzzing with oracles

- Chosen: fuzz targets for each property above, over the core, GFM, MDX, test constructs, and the five ports, with bolero so the same harnesses later run under Kani. Differential targets against micromark (Node, in a CI job) and cmark-gfm, each with a list of known, justified differences. Steps per byte as a fuzzing objective, with every slow input kept as a regression test, as cmark's pathological tests do.
- Continuous: OSS-Fuzz project files and a request drafted for wooorm; meanwhile, time-boxed fuzzing on each PR and a longer nightly run in GitHub Actions.

### F3. Restriction lints

- Chosen: `clippy::unwrap_used`, `expect_used`, `indexing_slicing`, `string_slice`, `panic`, `unreachable`, and `arithmetic_side_effects` in the core, each remaining site with an `#[allow]` that states its invariant, or rewritten. Done when none is unjustified.
- The 1% callgrind gate applies: a rewrite that costs speed keeps the panic and its justification instead.

### F4. Kani for bounded proofs

- Chosen: Kani, through bolero harnesses where they fit:
  - leaf functions (UTF-8 and character helpers, position arithmetic, encoding, URL sanitizing, character references) for all inputs up to a length to measure;
  - the extension runtime (limits, attempts, rollback, ending a construct early) with contracts and `stub_verified`;
  - the whole parser for all inputs up to a small length.
- In an optional CI job with Kani's own toolchain. Bounds are recorded with each proof.

### F5. The deductive tool, by a spike

- Chosen: two days each for Verus and for Aeneas (to Lean) on the same two pieces, a leaf module and a tokenizer module, measured by proof effort, readability for contributors, code impact upstream, and how the proofs survive a small code change. Kani stays for bounded proofs either way.
- Alternatives: pick Verus now (Rust-native, largest community, stable toolchain, covers wasm32); pick Aeneas now (no code change, strongest scale, shares a logic with a Lean reference); Creusot (nightly, small proofs so far).

### F6. The extension runtime stays as it is

- Chosen: keep the type erasure (`Box<dyn Any>`) that lets constructs of different types live in one list, prove the runtime with Kani, and decide on any refactor for deductive proofs after the spike, as it touches the plugin API.

### F7. Linear work as a proven step counter

- Chosen: no tool proves time bounds for Rust, so state linear work as a counter: each step consumes a byte, or is charged against a budget of c times the input length. Fuzz the counter's maximum now; prove the counter later in the deductive tool.

### F8. The MDX boundary

- Chosen: treat MDX callbacks as trusted, but fuzz and bound what markdown-rs does with what they return (positions, lengths, errors), so markdown-rs itself does not panic around them.

### F9. A kit for plugin authors

- Chosen: what authors can run on their own constructs: a fuzz template with the oracles, a bolero/Kani harness template, and the `trace`, `assert_linear_with`, and `assert_same_html` helpers from PR 270, with a guide recipe.

### F10. A Lean reference, later

- Chosen, after the steps above: an executable reference of the spec's algorithms in Lean, possibly from lean-markdown, tested on the spec examples and used first as a differential oracle; proving the Rust equivalent to it is research-scale and comes only if the spike picks Aeneas and the rest has landed.

## Measures

- No unjustified panic sites in the core, enforced by lints in CI.
- Kani: leaf functions proven up to their recorded bounds; the runtime's limits and rollback proven; the whole parser proven up to its recorded bound.
- Fuzzing: a budget of CPU hours per release across all targets with no failures; every finding a regression test.
- The extension runtime proven by the deductive tool, after the spike.
- Each commit: the CI job clean, MSRV 1.62, and callgrind within 1% of its parent without constructs.

## Order of work, as an upstream series after PR 270

1. Invariant predicates (F1), with the property fuzzers and their oracles (F2), and a regression corpus.
2. Differential fuzzing against micromark and cmark-gfm, with the list of known differences; OSS-Fuzz files and the request for wooorm.
3. Restriction lints, module by module (F3).
4. Kani: leaf functions, then the runtime, then the whole parser up to a bound (F4); cargo-mutants on the core.
5. The kit for plugin authors (F9).
6. The spike (F5), then deductive proofs: leaf functions, the step counter (F7), and the runtime, module by module.
7. The Lean reference (F10), if the spike and the steps before it support it.

## Open questions

- The bounds for Kani: measured in step 4.
- The CPU budget for fuzzing per release.
- Whether wooorm wants OSS-Fuzz, and who besides him receives its reports.
- Node in CI for micromark, or a prebuilt corpus of micromark's outputs.

## Sources

Tools: Kani (model-checking.github.io/kani), Rust std verification (model-checking.github.io/verify-rust-std), bolero with Kani (aws.github.io/s2n-quic/dev-guide/kani.html), Verus (verus-lang.github.io/verus; amazon.science, "Developing provably correct Rust code with Verus"), Aeneas (github.com/AeneasVerif/aeneas; SymCrypt, arXiv 2609.15648), Creusot (github.com/creusot-rs/creusot), Flux (github.com/flux-rs/flux), hax (github.com/cryspen/hax), cargo-mutants (mutants.rs).

Literature: Jourdan, Pottier, Leroy, ESOP 2012; Vermillion, ITP 2019; CoStar, PLDI 2021; TRX, ESOP 2010; Blaudeau and Shankar, CPP 2020; Danielsson, ICFP 2010; Verbatim, CPP 2022; EverParse, USENIX Security 2019; Narcissus, ICFP 2019; Vest, USENIX Security 2025; EverCBOR, arXiv 2505.17335; lean-markdown (paulbutcher.com/lean-markdown.html); L4YAML (github.com/nasa-jpl/L4YAML); BEK, USENIX Security 2011; comrak advisories GHSL-2023-047 to 049.

Some claims about the newest work (2026 papers, lean-markdown) are as reported by the research and not checked line by line.
