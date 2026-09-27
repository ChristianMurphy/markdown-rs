---
status: Active
---

# markdown-rs parse speed: superlinear fixes and SIMD fit

## Context

markdown-rs is slow and some inputs scale quadratically or worse. A third-party project measured 120 to 160 ns/byte, against 2 to 5 ns/byte for pulldown-cmark on the same documents ([amiss PR #1044](https://github.com/HardMax71/amiss/pull/1044)). Upstream issue [#113](https://github.com/wooorm/markdown-rs/issues/113) reports quadratic MDX parsing. Upstream PRs [#197](https://github.com/wooorm/markdown-rs/pull/197) and [#217](https://github.com/wooorm/markdown-rs/pull/217) target one cause, the edit map, and neither is merged.

Goal: parse faster and remove superlinear paths while keeping CommonMark 0.31.2, GFM, and MDX output identical. An opt-in lookahead or work limit is a last resort and stays off by default. Build several prototypes and measure which ones fit.

Small documents are the common case. A superlinear fix must not make small documents slower to help rare large ones. See "Small-document rule".

Baseline: `upstream/main`, `origin/main`, and tag `1.0.0` all point at `1506572`. The shared checkout at `markdown-rs/` currently has `feat/plugin-prototypes` checked out by another active session, with uncommitted changes that include `src/mdast.rs` and `Cargo.toml`. Installed: Rust 1.95 stable and nightly, valgrind with callgrind, cargo-fuzz. Not installed: perf, and the wasm32 and aarch64 targets.

This file doubles as the research report. It draws on read-only research passes over the source, local micromark, cmark, and pulldown-cmark sources, crates.io, and GitHub, plus design passes that weighed alternative fixes. No timing has been run yet.

## Research findings

### Where the constant factor goes (source reading, unmeasured)

- Every byte passes through `push_impl`, `byte_action`, the `call()` match in `src/state.rs:473`, the state function, then `consume` and `move_one`, which calls `byte_action` a second time (`src/tokenizer.rs:448-475`, `:691-782`).
- A paragraph byte is walked about four or five times: document flow, a GFM table head-row attempt when GFM is on, the content chunk, the paragraph, and the text data.
- Text data stops on a fixed marker set of 16 bytes (`src/construct/text.rs:35-52`). With default CommonMark options, 7 of them, `$ { ~ H h W w`, can never start a construct, yet each still costs a Data exit, a failed attempt, a Data enter, and a merge edit.
- `partial_data::inside` runs `markers.contains()`, a slice scan, on every byte.
- Attention `classify` does a linear `PUNCTUATION.contains` over 9,369 entries (`src/util/unicode.rs:18`) for each non-ASCII neighbor. `util/char.rs:43-58` calls `from_utf8_lossy`, which allocates when the 4-byte window splits a code point.

### Superlinear inventory

N is input bytes, S attention sequences, M edits, D definitions, R references, d nesting depth.

| Id | Site | Input family | Cost today |
|---|---|---|---|
| A2a | `util/edit_map.rs:123-148` linear scan per add | Any multi-line document | O(M²), M near line count |
| A2b | `edit_map.rs:134-137` `add_before` moves the buffer | `"*".repeat(n)+"a"+"*".repeat(n)` | O(n²) |
| A1c | `construct/attention.rs:261` clones `stack`, `:177` compares it | `"![".repeat(d)+"*a ".repeat(s)+"](b)".repeat(d)` | O(S·d) memory, O(S²·d) time |
| A1b | `attention.rs:419-432` `Vec::remove` | `"a*".repeat(n)` | O(S²) moves |
| A1a | `attention.rs:160-212` walk back plus the between loop at `:323-328` | `"a_ ".repeat(k)` | O(S²) |
| B3 | `list_item.rs:320-326` blank check per container | `"- ".repeat(d)+"a\n"+" ".repeat(2d)+"b"` | O(d·W) per line |
| B4 | `document.rs:94-131` continuation on blank lines | `"- ".repeat(n)+"x"+"\n".repeat(n)` | O(n·d) |
| B5 | `list_item.rs:109-112` thematic-break check per marker | `"- ".repeat(d)+"a"` | O(d²) per line |
| B6 | `list_item.rs:386-418` item ends and the `lists_wip` walk | `"- ".repeat(d)+"a"` | O(d·E + L·d) |
| A6 | `raw_text.rs:194-271` unclosed code span rescans | Rising backtick lengths; `` "\\`` ".repeat(n) `` | O(N^1.5); O(n²) |
| B11 | `html_text.rs` comment, PI, CDATA, declaration | `"<!--".repeat(n)` | O(n·N) |
| B7 | `label_end.rs:354-363` marks all starts inactive | `"![[]()".repeat(n)` | O(n²) |
| B8 | `label_end.rs:262-264` normalize at every `]` | `"[".repeat(n)+"a"+"]".repeat(n)` | O(n²) bytes, 3 allocations per `]` |
| B9 | `gfm_autolink_literal.rs:266-428` www retries | `"_www.".repeat(k)` | O(k²) |
| B10 | `gfm_autolink_literal.rs:520-613` trail rescans | `"http://a.b/"+"!".repeat(k)+"x"` | O(k²) per literal |
| A4 | `label_end.rs:268,280,593-610`; `to_html.rs:955,1428-1444`; footnotes | D definitions, R references | O(R·D) |
| A3 | `to_html.rs:877-889`, `to_mdast.rs:1065-1084` | Table code span full of `\|` | O(k·len) |
| A5 | `to_mdast.rs:1718-1727` `delve_mut` | Deep quotes, lists, emphasis | O(E·d) |
| B13-B17 | `to_html.rs` nested label buffers, normalization, media stack; `util/slice.rs:27-40`; `util/infer.rs` | `"![x".repeat(n)+"](b)".repeat(n)`, `"- ".repeat(n)` | O(n²) |
| B18 | `mdast.rs:306-360` recursive `to_string`; derived Drop, Clone, Debug, serde | `"> ".repeat(n)` | O(depth·len); stack overflow |
| A7 | `util/mdx_collect.rs`, `partial_mdx_expression.rs`, `mdx_esm.rs` | Unclosed ESM, many candidate `}` | O(k·P) here plus the host |
| A8 | `parser.rs:42-46`, `util/location.rs` | Any MDX with callbacks | O(N) eager, off the success path |
| B12 | `mdx_expression_flow.rs`, `mdx_jsx_flow.rs`, retried every line | `"{\n".repeat(n)+"}".repeat(n)+"x"` | O(n²); O(n³) with a host parser |

Other findings:
- No exponential path. The worst cases are cubic: A1c, and B12 with a host parser.
- Only the host re-parse in MDX has no exact algorithmic fix, because the host parser decides where an expression ends.
- micromark shares almost every mechanism. cmark and pulldown-cmark supply reference algorithms. Their syntax caps are not copied.
- Semantic trap: markdown-rs applies the rule of 3 and the tilde size rule to remaining delimiter sizes, while cmark uses the original run lengths. A straight port of cmark's `openers_bottom` therefore changes output: `a****b c** d*  e**` would differ (worked by hand, to confirm by running).

### SIMD and scanning fit

Stop sets, with `\n \r \t` included: text 19 bytes (12 in CommonMark mode), string 5, line scanners 3, raw text 4, HTML 3 to 4, table cell 6, MDX expression 4 to 5, HTML encode 5. Every set spans at most 8 high nibbles, so an exact two-table nibble classifier works.

| Option | Version and MSRV | no_std | Targets | Fit |
|---|---|---|---|---|
| Scalar byte-table batching | none; MSRV unchanged | yes | all | Baseline that every SIMD variant must beat |
| memchr | 2.8.3 (2026-07-08), MSRV 1.61, zero required deps | `default-features = false` | SSE2 without std; AVX2 by compile flag only; NEON; wasm only with `+simd128` | Exact fit for sets of 1 to 3 bytes |
| wide | 1.7.1 (2026-09-14), MSRV 1.89 | default off | Compile time only; `pshufb` needs SSSE3, not in the x86-64 baseline | Nibble classifier for the 19-byte set |
| fearless_simd | 1.0.0 (2026-09-21), MSRV 1.89 | needs `libm` | Runtime detection needs std | Same classifier; multiversioning adds code size |
| core::simd | nightly only | – | – | Not usable |
| core::arch | Rust 1.87+ | yes | all | Needs `unsafe` at call sites and loads; not used |

### Measurement assets

- `benches/bench.rs` measures only `readme.md` through `to_html`.
- Corpora: the cmark pathological families, micromark `test/perf.js`, `tests/commonmark.rs`, the GFM and MDX tests, `commonmark-data.txt`, about 46k fuzz corpus files under `fuzz/corpus/`, and the input families in this plan.
- Tools: criterion 0.5 is already a dev-dependency. Callgrind gives hotspots and deterministic instruction counts. A counting global allocator gives peak heap.

## Small-document rule

Design rules for every fix:
- Allocate nothing until the pathological condition appears. Memos start empty and fill only after a failed scan.
- Where an index is needed, keep the current fast path below a measured threshold.
- Prefer fixes that are cheaper at every size where one exists.
- Build an always-indexed variant next to each hybrid, so the crossover is measured, not guessed.

Measurement rules:
- Size bins: under 256 B (CommonMark examples, median 18 B), 256 B to 4 KB, 4 to 64 KB, above 64 KB. Report latency per document, not only throughput.
- Gate: callgrind instruction counts on the two small bins may rise by no more than about 1%. Wall-clock noise at microsecond scale hides smaller changes.

## Fix alternatives

How to read these tables:
- Compute gives the worst case after the fix, then the effect on small documents: faster, same, or slower.
- Memory is none, lazy (allocated only once the pathological case appears), or eager.
- Code is S (under 50 lines, 1 file), M (50 to 200 lines or 2 to 3 files), or L (over 200 lines or 4 or more files). A new `TokenizeState` field makes a change touch 2 files.
- Cognitive complexity:
  - 1: the pattern already exists in this codebase.
  - 2: standard pattern, such as a sort, a stack, or a BTreeMap.
  - 3: new to this codebase but common elsewhere, such as a memo, a watermark, or a delimiter stack.
  - 4: correctness rests on a subtle invariant.
  - 5: novel; needs a proof.
- Exactness is exact, needs a differential proof, or changes output.

"Pick" marks the recommendation. The pattern behind each pick is the simplest option that removes the superlinear cost without penalizing small documents.

### Edit map (A2a, A2b)

For one index, any interleaving of `add` and `add_before` yields the same buffer: befores in reverse call order, then afters in call order. That fact makes every option below exact.

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| BTreeMap (PR #197) | O(M log M); small slower: allocates a new leaf in each resolver pass and loses the Vec's kept capacity | eager nodes | S | 2 | exact; A2b remains |
| Vec plus index above 32 entries (PR #217) | O(M log M); small same | lazy above 32 | S | 2 | exact; A2b remains |
| Pick: append-only log, grouped at `consume` | O(1) per add, O(M log M) at consume; small faster, since no scan per add and the sort already exists | about 8 B more per entry | M, about 80 lines | 3 | exact; also fixes A2b |
| Fast path for increasing indices | still O(M²): the main callers add in descending order | none | S | 2 | exact, but partial |
| Sorted Vec with `partition_point` | O(M²) memmove: `document.rs:593` inserts at the front | none | S to M | 2 | exact |

If #197 or #217 wins the benchmark, pair it with a two-sided entry for A2b. That keeps a reversed `before` Vec per entry: S, cognitive 3, exact.

### Attention (A1c, A1b, A1a), landing as one change to `attention.rs`

A1c, scope per sequence:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Innermost open Enter index | O(1) compare; small faster | none | S | 2 | exact: an index is pushed once, so equal tops imply equal stacks |
| Pick: (scope, depth), which feeds the A1a frames | O(1); small faster | 8 B per sequence | S | 2 | exact |
| Dense scope counter | O(1) | none | S | 2 | exact |
| Interned stacks | O(1) | eager per scope | M | 3 | exact |
| Depth only | – | – | S | 2 | changes output |

A1b, removing matched sequences:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Tombstones | still O(S²) walking over dead entries; small faster | none | S | 2 | exact, but partial |
| Index-linked list | O(1) removal | 8 B per sequence | M | 3 | differential proof |
| Pick: delimiter stack shared with A1a; a match truncates to the opener and pops the entries between | O(1) amortized; also removes the between loop; small faster | none | M, about 90 lines | 3 | differential proof |
| Lazy compaction | amortized O(S) | none | M | 3 | exact |

A1a, finding an opener:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Delimiter stack only | O(S·B), still quadratic for mixed markers; small faster | none | M | 3 | differential proof |
| Plus a stack per marker | removes other-marker walks | none | M | 3 | differential proof |
| Plus scope frames | removes other-scope walks | lazy per nesting | M | 3 to 4 | differential proof |
| Pick: stack, frames, and per-category bounds clamped to the opener's position after each match | O(S + E) amortized; small same | lazy per nesting | M to L, about 200 lines | 4 | differential proof |
| Straight port of cmark `openers_bottom` | O(S) | none | M | 3 | changes output |

Land it in two steps. First the stack, frames, A1b, and A1c, where each part has a short exactness argument. Then the clamped bounds, gated on a differential fuzz run over `* _ ~ a ␠ . [ ] ( ) ! \n`.

### Containers (B3, B4, B5, B6)

B3, blank-line check:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Byte scan with no attempt | O(d·W) per line, small constant; small faster | none | M, about 20 lines | 1 | exact |
| Pick: first-non-space watermark (cmark `S_find_first_nonspace`) | O(W + d) per line; small faster | 2 usize | M, about 35 lines | 3 | exact |
| Per-line flag reset after each `>` | O(W + d) | none | M | 3 | exact |
| Eager next-non-space array | O(N) | eager O(N) | M | 2 | exact; breaks the small-document rule |

B4, blank lines under deep containers:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Tight loop over the container stack | O(d) per line, still quadratic | none | S | 2 | exact |
| cmark-style flag | O(1) for empty lines; whitespace-only lines stay O(d) | a bool | M | 3 | exact |
| Pick: suffix watermark valid for one line | O(1) per blank line; small same | 2 usize | M, about 30 lines | 4 | differential proof |
| Sorted blocking-container indices | O(log d) per line | lazy; allocates for any document with a block quote | M to L | 3 | exact |

B5, thematic-break check at each list marker:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Kill-position memo (cmark `thematic_break_kill_pos`) | O(line) per line; small same | 2 usize | M, about 15 lines | 3 | exact |
| Pick: byte prefilter plus kill memo | O(line); small faster, since plain `- text` skips the check | none | M, about 30 lines | 3 | exact |
| Backward suffix scan | O(line) | none | M | 4 | exact |
| Full byte replacement | faster | none | M | 3 | differential proof |

B6, list merge resolver:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Lazy `before` plus early stop | wip walk O(1) amortized; item ends still O(d·E) | none | S | 4 | exact, but partial |
| Early stop plus stack pre-pass for ends | O(E + L); small same | lazy, once per document with lists | S to M | 4 for the early stop | exact |
| Pick: early stop plus ends filled at each item exit | O(E + L); small faster | none | S to M, about 40 lines | 4 | differential proof |
| Separate open-list stack | O(E + L) | lazy O(d) | M | 3 | differential proof |

### Inline scan memos (A6, B11, B7, B8, B9, B10)

These memos share one design. Each is a fact about the bytes of one tokenizer, stored in `TokenizeState` by absolute byte index. Attempts do not roll them back, and they never need invalidating. They use shared naming and one doc comment, not a generic abstraction.

A6, unclosed code spans and inline math:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| "No closer of length L after p" set | fixes the escape family only | lazy | M | 3 | exact, but partial |
| Pick: per-length run table, recorded only after the first failure (cmark `backticks[]` without its length cap) | O(N log k), k ≤ √(2N); small same | lazy O(k) | M, about 80 lines | 3 | exact |
| Index built from the failing scan's events | O(N) | allocates on the first failure | M | 4 | differential proof |
| Eager run index (pulldown `CodeDelims`) | O(N) | eager for every paragraph with code | M | 3 | breaks the small-document rule |

B11, unclosed inline HTML:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| cmark skip flags | O(N); small same | none | M, about 25 lines | 3 | exact while attempts only move forward |
| Pick: flags with start positions | O(N); small same | `[usize; 4]` | M, about 25 lines | 3 | exact |
| pulldown `HtmlScanGuard` | O(N) | none | M | 3 | exact |
| Byte-level closer prefilter | O(N) | none | M to L | 4 | differential proof |

B7, marking earlier link starts inactive:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Backward loop with an early stop | still O(n²) for image starts | none | S | 2 | exact, but partial |
| Pick: stack watermark, clamped on each pop | O(1); small slightly faster | 1 usize | M, about 15 lines | 3 | exact |
| Epoch counter per start | O(1) | replaces a bool | M, 5 files | 3 | exact |
| Separate image and link stacks | O(1) | none | L | 3 | differential proof |

B8, normalizing labels:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Skip when no definitions exist | unchanged with any definition; small faster, saving 3 allocations per `]` | none | S | 1 | exact |
| Skip when a label start sits inside the label (no definition can contain an unescaped bracket) | O(1) per `]` | none | S | 4 | exact by argument |
| Pick: both | O(n); small faster | none | S | 4 | exact; plus a unit test that case mapping never produces `\ [ ]` |
| Spec's 999-byte cap on every label | bounded | none | S | 1 | changes output and diverges from micromark: rejected |
| Stop normalizing past the longest definition id | bounded | none | M | 3 | exact |

B9, `www.` retries:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: failure watermark built by a backward scan over the last two segments | O(N); small same | 2 to 3 usize | M, about 30 lines | 4 | exact; differential recommended |
| Same watermark tracked forward | O(N); small same | none | M | 4 | exact |
| Skip only up to the last dot | partial | none | M | 3 | exact |
| cmark-gfm rule for more than 10 segments | O(N) | none | S | 1 | changes output |

B10, trailing punctuation:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: trail-failure watermark | O(k) per literal; small same | 3 usize | M, about 25 lines | 3 to 4 | exact |
| Bulk consume up to the known failure | O(k) | none | M, about 50 lines | 4 | exact; duplicates logic |
| cmark-gfm scan then trim backward | O(k) | none | L | 4 | differential proof |
| Eager next-non-trail array | O(N) | eager O(N) | M | 2 | breaks the small-document rule |

### Definitions and compile phase (A4, A3, A5, B13 to B17, B18)

A4, definition lookups. Parse side:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: sort and dedup once per pass in `parser.rs`, then `binary_search` | O((D+R) log D); small same | none, in place | M, about 15 lines | 2 | exact |
| Pick: skip normalizing when there are no definitions (overlaps B8) | small faster | none | S | 1 | exact |
| Defer `defined` to the failure paths | linear for nested resources | none | M | 3 | exact after review of the revert |
| `BTreeSet` | O((D+R) log D) | eager, about 300 B even for one definition | M | 1 | exact |
| Hand-written hash set | expected O(D+R); collisions O(R·D), since no_std has no random seed | eager | L | 3 | exact |

A4, `to_html` side:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: stable sort, dedup keeping the first, then binary search | O((D+R) log D); small same | in place | S | 2 | exact |
| `BTreeMap` with `entry().or_insert` | same | eager nodes | S | 1 | exact |
| Pick for footnotes: lazy index above 16 entries, plus a stable-sort dedup | O((C+U+F) log) | lazy | M, about 50 lines | 2 | exact |

A3, table pipe unescaping:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| In-place compaction with read and write cursors | O(len); small same | none | S | 2 | exact |
| `replace("\\|", "|")` behind a `contains` guard | O(len); small same | lazy, only when an escape exists | S | 1 | exact |
| Pick: shared helper for both compilers using compaction, and drop the unconditional `to_vec` in `to_html.rs:873` | O(len); small faster, one less copy per code span | less than today | M, 3 files | 1 | exact |

A5, tree building:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: stack of owned open nodes, attached to the parent on exit | O(E); small same, to confirm by measuring | none extra | M, about 100 lines | 2 | exact |
| Cache depth or tail | no gain; caching the tail needs `unsafe` | – | S | – | – |
| Arena, then convert | O(E + N); small slower | eager, about 2x peak | L | 3 | differential proof |
| Recursive-descent compile | O(E), but the stack can overflow | O(d) stack | L | 2 | rejected |

B13 to B17, nested media and lists in the compilers:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick for B13: stream the labels of media nested inside an image into the outer alt buffer | O(n); small same | less | S | 3 | exact |
| B13: one String plus an offset stack | still O(n²) on its own | none | L | 3 | exact |
| Pick for B14: normalize ids only for references, in both compilers | small faster | none | S | 1 to 2 | exact |
| Pick for B15: record the `LabelText` start on Enter | O(1) | none | S | 3 | exact output |
| B15: Enter-index stack for every `from_exit_event` | O(E) | lazy O(depth) | M | 2 | differential proof |
| Pick for B16: compute `is_in_link` only when needed | O(1) | none | S | 2 | exact |
| B16: link-depth counter | O(1) | none | S | 1 | exact |
| Pick for B17 in `to_mdast`: set `spread` at exit through the A5 stack | O(E); small faster | none with A5 | M | 4 | differential proof |
| Pick for B17 in `to_html`: one memo per top-level list | O(E); small same | lazy | M | 4 | differential proof |

B18, `Node::to_string`:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Recursive `write_to(&mut String)` | O(N); depth risk remains | none | S | 1 | exact |
| Pick: iterative walk with an explicit stack | O(N); small same or faster | lazy O(depth) | S, about 50 lines | 2 | exact, including `""` for images |
| `impl Display` in place of `ToString` | slightly slower | none | S | 2 | exact output; the author calls it breaking |
| Recurse to a threshold, then use a stack | O(N) | lazy | M | 3 | exact |

B18 write-up only, for Drop, Clone, Debug, and serde:
- `impl Drop for Node`: breaking (E0509), and slower for small documents.
- Drop on `Root` only: a narrower break.
- Public `drop_iteratively` helper: non-breaking, S, cognitive 2.
- Opt-in `max_nesting`: the only option that also bounds Clone, Debug, PartialEq, and serde.
- Document the stack size callers need.
- Hand-written iterative `PartialEq` and `Clone`: L.

### MDX (A7, A8, B12, reading budget)

A7, re-collecting at each candidate end:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: incremental collect in `TokenizeState`, with a debug assertion against a full collect | O(bytes) per construct; small same | none | M, 3 files | 3 | exact |
| Follow-up: borrowed slice when chunks are contiguous | O(1) per call; small faster | none | M | 2 | exact |
| Build `stops` only on the error path | small faster | none | S | 1 | exact |
| Pick: remove the per-chunk `format!` in `Slice::serialize` | small faster | none | S | 1 | exact |

A8, line index:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: build `Location` only on the error path | small faster: no extra pass or allocation for MDX documents | brief, on error only | M, about 10 lines in 3 files | 1 | exact |
| Scan straight to a `Point` | O(N) on error | none | S | 2 | exact |
| Pick as an add-on: binary search in `to_point` | O(log L) | none | S | 1 | exact |
| Derive the point from event points | O(1) | none | S | 2 | changes output: tabs and `\r` |

B12, flow expression and JSX retries in agnostic mode (no host parser):

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Brace-match memo recorded during failed attempts | about O(n log n); small same | lazy O(braces) | M | 5 | differential proof |
| Raw-byte brace table, used only when no containers are open | O(n) | lazy | M | 4 | differential proof |
| Count agnostic attempt bytes in the opt-in budget | O(k·n) when enabled; default stays O(n²) | none | S to M | 2 | exact when off |
| Skip lines inside a failed attempt | O(n) | none | S | 1 | changes output |

Reading budget implementation:

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: `Cell<usize>` counter in `ParseState`, checked before each host call | one branch per host call when off | 16 B | about 80 lines, 5 files | 2 | exact when `None` |
| Counters per tokenizer, summed through `Subresult` | same | none | L | 3 | error-prone |
| `&Cell` field on `Tokenizer` | same | none | M | 2 | no benefit over the pick |
| Wrap the host closures | same | a Box per parse when enabled | M | 2 | error place differs |
| Check after each host call | overshoots by one call | none | M | 2 | breaks the contract: rejected |

### Scanning (S1)

| Option | Compute | Memory | Code | Cog | Exact |
|---|---|---|---|---|---|
| Pick: `u128` or `[bool; 256]` in place of `contains` | small faster | none | about 25 lines, 4 files | 1 to 2 | exact |
| Pick: stop mask from enabled constructs, plus the rule for `h/w` after a letter | small faster: fewer Data splits and merge edits | 16 B | same files | 2 | exact given the Data merge; differential proof |
| Pick: `consume_run(stop)` from state functions, bounded by `push_end` | small faster: one branch per run | 1 usize | M to L | 3 | differential proof for positions |
| Central hook in `push_impl`, keyed by state | faster, plus one match per `Next` | none | M | 3 | differential proof |
| Cache `ByteAction` between `push_impl` and `move_one` | faster | none | S | 2 | exact |

Other constant-cost wins, all exact and cheaper at every size:
- `binary_search` in place of the 9,369-entry `PUNCTUATION.contains`, once the table is confirmed sorted.
- No allocation in `char.rs` window decoding.

## Compatibility with the plugin prototypes (#32)

The session `markdown-plugin-prototypes` works on `feat/plugin-prototypes` in the shared checkout. Its layer 3, text syntax extensions, is committed at `49d1b7b`; block and nested-content plugins are next. Its crate-internal changes overlap this plan in `parser.rs`, `construct/text.rs`, `state.rs`, `to_mdast.rs`, `to_html.rs`, and `mdast.rs`. Both sessions agreed to these rules:

1. The text data stop set is one byte set in `ParseState`, built once per parse. It holds the built-in markers for enabled constructs plus every `TextConstruct::markers()` byte, so `text::before` always gets control at a plugin marker. Only markers of disabled built-ins are dropped. The byte set replaces the plugin branch's `ParseState.text_markers: Vec<u8>`.
2. `to_mdast` keeps `tail_push`, `tail_pop`, `tail_mut`, and `tail_penultimate_mut` with the same meaning after `delve_mut` is replaced, because `on_enter_extension` builds `Custom` nodes through them.
3. The iterative `Node::to_string` handles `Custom` as the plugin branch defines it: the value if present, otherwise the children's text.
4. Plugin constructs cannot register resolvers, touch the `EditMap`, run attempts, or read `tokenize_state`. They run before built-ins at their marker byte and never push onto `label_starts`. So the attention, edit-map, and label changes here do not interact with them. The core facade does run one attempt per matching construct in `text::before_construct` and keeps its own `extension_*` fields in `tokenize_state`.
5. Both sessions keep MSRV 1.56: no `let-else`, `OnceCell`, `is_some_and`, or inline format args.
6. After the plugin branch commits layer 3, this harness measures that branch with no plugins registered, using the small-bin instruction counts, and reports the result to that session.
7. `Tokenizer` stays at or under 1000 bytes; `test_tokenizer_size` asserts it. This branch uses 960 bytes; the plugin branch moves its extension fields into one lazily boxed field, 8 bytes.
8. Nested content from plugins gets a fresh tokenizer through `subtokenize`, never a recursive parse inside an attempt: attempts do not roll back `label_starts`, the label watermark, attention sequences, or queued edits. A flow plugin fails only on its opening line, as fenced code does, which bounds its reverts to one line and keeps the B12 memo's lazy-line argument intact. This rule is tentative until the developer decides the plugin design.
9. `Node::Custom` has `value: Option<String>` and children. The iterative `to_string` needs a `Node::Custom(x) if x.value.is_some()` arm before its `children()` check when the branches meet.

Merging the two branches is the developer's call.

## Environment

A fresh `Cargo.lock` fails to build the dev-dependencies: `swc_common` 8.1.1 imports `serde::__private`, which `serde` 1.0.220 and later no longer export. These pins in the git-ignored lockfile restore a passing build, applied in this order:
- `vergen` 9.0.6, which brings in `cargo_metadata` 0.19.2
- `serde_json` 1.0.140
- `camino` 1.1.9
- `semver` 1.0.26
- `env_logger` 0.11.6, which drops `jiff`
- `serde` 1.0.218

With these pins, the baseline `cargo test --all-features --workspace` passes: 198 passed, 0 failed, 11 ignored. There are no known failing tests.

## Design decisions and alternatives

Decisions made during the work, beyond the per-fix tables in "Fix alternatives". Each lists the alternatives with their consequences, the evidence, the choice, and when to revisit it.

### What the MDX reading budget counts

Question: with a JavaScript parser, MDX re-sends an expression's text at each `}` until the parser accepts it, so the reads grow with the square of the closing braces in one expression. How should the opt-in budget treat that?

| Alternative | Consequence |
|---|---|
| Chosen: factor × document size over all reads, parser and brace counting | Bounds crafted input at the chosen factor. Ordinary data expressions need large factors: an array of 10 small objects needs 6, 100 need 51, 1000 need 501 |
| Brace counting only | Ordinary documents need a factor of 0 or 1; parser reads are unbounded again, as in 1.0.0 |
| A new host callback that reports where an expression ends, like acorn's `parseExpressionAt` | One call per expression, so linear; exact for valid code, error messages may differ; needs new API and host support; flow expressions and ESM would call once their block ends |
| A JavaScript-aware scanner that skips strings, template literals, comments, and regex literals, and calls the parser at the balanced `}` | Usually one call per expression and no API change; JavaScript-specific, and where it misreads code (regex against division, a host for another language) expressions end later than in 1.0.0 |
| Binary search over the `}` positions, assuming the parser answers "need more" until it accepts | A logarithmic number of calls; needs that contract from hosts and every `}` in view, which flow expressions arriving by line do not give |

Evidence: smallest factors measured with the harness's stand-in parser on generated documents and the corpus (78 of 98 corpus documents with braces need 0 without a parser, 20 need 1).
Why: it still bounds crafted input, needs no new API, and the exact fixes for parser reads are separate API decisions.
Revisit if: data-heavy MDX users need a cap, or a host API that reports where an expression ends becomes acceptable.

### Shared mechanisms against per-construct memos

Question (developer, 2026-09-27): weigh internal complexity, and when approaches perform about the same, prefer one that works in more than one place. Which memos can share a mechanism?

| Alternative | Consequence |
|---|---|
| Attempt-result cache keyed by state and position | Rejected: the costly inputs are different starts rescanning overlapping text, so keys never repeat; results also depend on `tokenize_state`, lazy lines, and containers, so the key is not exact; a lookup per attempt slows small documents |
| One "no closer after here" memo for inline HTML and code spans | Rejected: code spans need every run size counted in one pass, inline HTML a watermark per closer, so one type needs a mode flag; only two users |
| One per-line facts cache for container and flow blank checks | Rejected: only the document tokenizer asks repeatedly within a line; the others run once per line in another tokenizer |
| Chosen: reuse `blank_line::rest_is_blank` in `list_item::marker_after` | Its third caller; removes one attempt per list item |
| Chosen: `core::ops::Range<usize>` for the thematic-break, trailing-punctuation, and `www.` memos | One convention with exclusive ends instead of three hand-made tuples and sentinels; no size or speed change |
| Chosen: box the code-span run table until an opener fails, as the MDX brace memo already is | One lazy-allocation pattern; `Tokenizer` drops from 960 bytes |
| Chosen: allocate the MDX brace memo only without a JavaScript parser | No allocation where nothing records |
| Kept bespoke | The container blank skip and the label watermark hold facts about stacks, not bytes; inline HTML's four fixed closers; the code-span run table; the MDX brace structure; MDX collection, which accumulates a body |

Evidence: a read-only audit of every memo, with its state, invariant, and operations; each consolidation then passed the tests, mutation checks, the differential, and small-bin instruction counts.
Revisit if: a third construct needs a "no closer after here" search, or another memo repeats the "starts in this range fail" shape.

### Stop tables for text data

| Alternative | Consequence |
|---|---|
| Tables built per parse from the enabled constructs, stored in `ParseState` | Fewest data splits; moving the larger `ParseState` cost tiny documents up to +0.73% in instructions |
| Chosen: two static tables, with and without `H W h w`, plus an exact `text::may_start` check at each stop | No setup per parse; a stop at a disabled marker costs one extra state call |
| One static table with all markers | Data splits at markers of disabled constructs and at `h`/`w` in words |

Why: the chosen form keeps nearly all of the small-document gain without a tiny-document cost.
Revisit if: plugins add marker bytes, which need a table per parse again (compatibility rule 1).

### Keeping `Tokenizer` small

| Alternative | Consequence |
|---|---|
| Chosen: box state used only by MDX, created on first use | `Tokenizer` 960 bytes; an allocation on the first MDX expression with a parser or the first failed flow expression |
| Shrink other fields, such as `Option<usize>` sentinels | Harder-to-read code for 16 to 32 bytes |
| Accept the size | Tiny documents up to 0.75% slower in instructions on glibc |

Evidence: glibc serves requests over 1000 bytes on a slower path, and the flow tokenizer is boxed once per document.
Revisit if: another allocator is the target, or a test on `size_of::<Tokenizer>()` fails after a merge.

### Timing tests for superlinear paths

| Alternative | Consequence |
|---|---|
| Fixed sizes, fastest of 3, parallel tests | Failed under load once parsing got faster: small cases took 0.5 to 2 ms |
| Double `n` until the small parse takes a few milliseconds | Inputs grow to where cache effects make linear work look superlinear; failed in release |
| Chosen: fixed sizes, fastest of 5, one retry, and timed parses serialized across tests | Passes 5 of 5 runs in debug and release; all six test functions still fail on 1.0.0, at 21.8x to 63.2x |

Revisit if: the suite gets slow (debug about 5.5 s now) or a case sits near its threshold.

### SIMD for scanning

| Alternative | Consequence |
|---|---|
| Chosen: scalar run batching | Most of the gain, no dependency, builds on every target |
| Optional `memchr` feature for line scanners | Small −1.9% to −3.1% more; needs Rust 1.61 with the feature on |
| `wide` or `fearless_simd` nibble classifier | Slower than scalar on default x86-64, where `pshufb` needs SSSE3; faster with `+ssse3` or `+sse4.2`; vectorized by default on aarch64 (NEON) |

Why: default x86-64 builds are the common case, and neither SIMD library speeds them up.
Revisit if: the crate gains a `std` feature (runtime detection in `fearless_simd`), or users target aarch64 or wasm with `simd128`.

## Results

All numbers come from the harness in `performance/`. The instruction columns give the callgrind change against 1.0.0, run inside the same binary; the two implementations match within 0.0003% on an unchanged tree. Scaling exponents are the local slope of log time over log bytes.

### Baseline (1.0.0)

- Tests: 198 passed, 0 failed, 11 ignored, with the lockfile pins in "Environment".
- The differential runner reports 0 differences on 1,418 inputs times 5 configurations. It reported 323 differences after a deliberate one-line change to heading depth, so it detects real changes.
- Small documents are expensive. The tiny bin (671 documents, 16,959 bytes) costs about 80,000 instructions per document, or 3,150 per byte. The small bin (192 documents, 313,278 bytes) costs about 1,110 per byte.
- Tiny-document profile, CommonMark:
  - malloc and free take about 17% of instructions and `memcpy` 9.5%.
  - `EditMap` takes 15% inclusive, mostly `consume` rebuilding events through `split_off`.
  - `Vec` growth takes 11%, `classify_opt` 5.5% (the linear punctuation scan), and `drop_in_place<State>` 3.8%.
- Scaling: 38 of 45 families grow superlinearly. Line count drives most of it: 128 KB of blank lines takes 4.6 s, and a single 14 MB paragraph parses at 62 ns/byte. Families that were meant as controls also turned out quadratic: `"[a".repeat(n)` has an exponent of 2.6 and `"[a](b".repeat(n)` 2.3, both through the edit map.
- `openers-only`, `unclosed-cdata`, `unclosed-instructions`, and `unclosed-declarations` are already linear. `rising-backticks` measures about 1.43, close to N^1.5.

### Edit map (A2a, A2b)

| Variant | Tiny instructions | Small instructions | ns/byte near 128 KB | Scaling | Output |
|---|---|---|---|---|---|
| PR #197, `BTreeMap` | +1.6% to +1.9% | +0.9% to +1.5% | 336 to 1,453 | linear | identical |
| PR #217, index above 32 entries | +0.2% to +0.4% | −0.2% to −0.4% | 313 to 1,305 | linear | identical |
| Append log, one-pass `consume` | −5.6% to −6.7% | −10.4% to −11.2% | 228 to 984 | linear | identical |

- PR #197 fails the small-document gate.
- The append log is fastest at every size. It fixes the per-add scan, and `consume` rebuilds the event list with one allocation, in place of one `split_off` per edit.
- Some exponents jump above 1 only past 1 MB, where event vectors exceed about 100 MB. `headings` is linear to 1 MB and then measures 2.06 at 2 MB. These are memory effects, not algorithmic.
- `image-link-pattern` stays near 1.9; its cost is B7.
- The `"*"*n + "a" + "*"*n` family stays quadratic because its nested `strong` nodes hit A5, `delve_mut`. Its regression test moves to the compile-phase prototype.
- Tests: unit tests for the ordering rule, links, and a seeded randomized comparison against the previous implementation, which fails when the rule is broken on purpose. `tests/pathological.rs` checks 4x-input time ratios: the old code fails at 14.7x, the fix passes.
- Check loop: `cargo fmt --check` clean. `cargo clippy` clean except one `implicit_saturating_sub` warning in `src/util/char.rs:44`, untouched code, from the newer clippy. `cargo test --all-features --workspace`: 202 passed, 0 failed, 11 ignored.

### Attention (A1a, A1b, A1c)

The attention branch is stacked on the edit-map branch.

- Step 1, the delimiter stack plus the innermost-scope index:
  - 0 differences.
  - Linear for `closers-only`, `emphasis-pairs`, `strong-pairs`, and `ambiguous-attention`. `closers-only` reaches 6 MB, up from 196 KB.
- Step 2, scope frames plus per-category bounds clamped after each match:
  - Every attention family is linear. `mixed-markers` reaches 12 MB; before, 196 KB took 3.3 s. `mismatched-markers` reaches 8 MB.
  - 0 differences on the corpus plus 1,000,000 random inputs, which is 5,007,090 comparisons.
- The random generator mixes single bytes with whole tokens such as marker runs. With single bytes only, 200,000 inputs did not catch a removed clamp. The token mix catches it with 244 differences.
- The clamp is required. 1.0.0 renders `a****b c** d*  e**` as `a*<strong><em>b c** d</em>  e</strong>`, and without the clamp the output changes. A regression test pins this. It has not been cross-checked with cmark, which is not installed, or with local micromark, which is not built.
- Step 2's own cost is about 0.12% of tiny-document instructions. Other small shifts come from inlining and malloc state differences between builds.
- Families that build deep trees stay quadratic until A5: `nested-strong-emphasis`, `attention-in-nested-images`, and `strong-run`.

### Punctuation lookup (constant factor)

- `classify` scanned all 9,369 entries of `PUNCTUATION` for every letter next to an attention sequence. It now uses `binary_search`, and only for non-ASCII characters. Tests prove the table is strictly sorted and that its ASCII part equals `is_ascii_punctuation`.
- Stacked result, edit map plus attention plus punctuation, against 1.0.0: tiny −11.0% to −12.1% instructions, small −11.9% to −12.9%.
- In debug builds attention got about 50x faster; 12 KB of `closers-only` takes 4 ms, down from 217 ms. Before this fix, the table scan hid the attention quadratic in debug, so the pathological tests could not fail. Now the old attention code fails them at 15.7x, and the new code passes, with the suite taking 0.96 s.
- For upstream, land this fix before attention, so the attention pathological tests discriminate in debug builds.
- Check loop on the stacked branch: rustfmt clean, clippy clean apart from the existing `char.rs:44` warning, 204 tests passed.

### Containers (B3, B4, B5, B6)

The containers branch is stacked on punctuation.

- B3: a byte scan with a first-non-space memo replaces the per-container `check(BlankLineStart)`. After a check the tokenizer reverts and feeds the same byte to the continuation state, so calling it directly with `Retry` is equivalent. The random differential catches a mutant that treats only `\n` as a line end, but only since `\r` was added to the random alphabet.
- B5: a byte prefilter with a memo skips the thematic-break check at list markers whenever the line cannot be a thematic break.
- B6: item ends come from a one-pass pre-pass: a stack pairs each ListItem Enter with its Exit, and a reverse pass extends each end over directly adjacent sibling items. This matches `skip::opt`, which skips runs of adjacent items. The match loop stops early, since list balances never decrease up the stack. Found along the way:
  - My first B6 filled item ends at each Exit, assuming one list per depth. The first differential runs never tested it: the edit was chained after `git commit` with `&&` and did not run when the commit found nothing to commit. Once applied, it gave 46 differences in 1,000,000 inputs.
  - Cause: when an unclosed fence absorbs the blank line, two list items become adjacent. 1.0.0 then treats the first item's end as the second's and renders a second `<ul>` directly inside the first. That output is probably a 1.0.0 bug; it is a draft issue, and this work keeps it.
- B4: on an empty line, jump over the suffix of containers that continued on the previous empty line. Those are list items without `blank_initial` and footnote definitions, which consume nothing and emit nothing on an empty line. The differential catches a mutant that always skips. The guard that the whole stack continued is redundant in practice, because `flow_end` exits truncate the stack; a mutant without it survives. It stays as a safety net.
- Differential: the corpus plus 1,000,000 random inputs, in release and with debug assertions: 0 differences.
- Instructions against 1.0.0, for the whole stack: tiny −11.9% to −13.2%, small −12.3% to −13.5%. B4 and B6 cost about 0.1 to 0.2 points, for the pre-pass allocation.
- Scaling: B3 and B5 halve the times for `deep-list` and `deep-list-indented-line`, but those families stay quadratic. A profile of `deep-list` shows tree building (A5, about 32%) and `infer::list_loose` (B17, 22.6%) as the remaining costs. Container pathological tests wait for the compile-phase prototype, because every deep-container input also builds a deep tree.

### Compile phase (A3, A4, A5, B8, B13 to B17)

The compile branch is stacked on containers.

- A5: an owned open-node stack replaces `delve_mut`. `tail_push`, `tail_pop`, `tail_mut`, and `tail_penultimate_mut` keep their meaning, as agreed with the plugin session. `resume` still attaches nodes that are left open, as 1.0.0 does in release builds; in debug builds, 1.0.0 fails an assertion there.
  - Found along the way: in MDX, `[<>a](c)` leaves a JSX fragment open inside a link label. 1.0.0 panics on that `debug_assert` in debug builds. Draft issue.
- B17: one pass computes every list's and item's spread and looseness, lazily, when the first list starts, and both compilers use it.
- A4: parse-side definitions are sorted and deduplicated at each pass boundary and found with binary search. In `to_html`, definitions are stable-sorted with the first definition kept. Footnote definitions get the same treatment, and footnote calls get an index once there are more than 16.
- B8: a label containing a later label start, or any label when there are no definitions, skips `normalize_identifier`. An exhaustive test over all characters confirms that case folding never produces `[`, `]`, or `\`.
- A3: a guarded `replace` for table pipe escapes, and `to_html` no longer copies every code span.
- B13 to B16 in `to_html`: the labels of images and links inside an image stream straight into the alt; footnote calls and definitions keep their buffers, because their labels are never written. The label position and identifier are computed only for references, and the check for being in a link runs only when it can matter.
- B14 and B15 in `to_mdast`: the identifier from the label text is computed only for shortcut, collapsed, and footnote references.
- Verified each step with the corpus plus 1,000,000 random inputs: 0 differences. Mutants caught: an ignored item-prefix exemption (490 differences), a reversed label fast path (3,592), and dropping the first-wins deduplication (52). The random generator gained `:`, digits, `+`, `)`, and tokens for definitions, footnotes, and ordered lists.
- Scaling, where 1.0.0 was quadratic, from the largest input 1.0.0 handled within the time budget to the largest the stack handles now:

| Family | 1.0.0 | Stack |
|---|---|---|
| deep block quote | 65 KB in 5.6 s | 4 MB in 3.5 s |
| deep list | 16 KB in 5.6 s | 2 MB in 3.4 s |
| staircase list | 1 MB in 14.1 s | 16 MB in 4.1 s |
| nested strong emphasis | 114 KB in 6.4 s | 7 MB in 4.5 s |
| definitions then misses | 754 KB in 3.7 s | 6 MB in 2.4 s |
| table escaped pipes | 1 MB in 3.5 s | 8 MB in 0.29 s |
| nested image labels | 114 KB in 6.3 s | 1.8 MB in 2.2 s |

- Still superlinear: `image-link-pattern` and `link-openers-then-links`, until B7 in the inline memos branch.
- Memory finding: peak heap is about 600 to 850 bytes per input byte on these families; 8 MB of nested brackets peaks at 5 GB. That is not a time problem, but it matters for the memory goal. It is recorded for later.
- Tests:
  - `tests/pathological.rs` checks the growth ratio against byte growth, running parses on a large-stack thread. Its three tests (edit map, attention, containers and trees) all fail on 1.0.0, at 54.8x, 24.4x, and 67.6x, and pass on the stack in 1.3 s.
  - Container and list regression cases come from the container review.
- Instructions against 1.0.0, for the whole stack: tiny −12.6% to −13.8%, small −13.3% to −14.5%.

### Inline scan memos (B7, B9, B10, B11, A6)

The inline branch is stacked on compile.

- B7: a watermark on the label-start stack replaces marking each earlier link start inactive. A non-image link sets it to the stack length, and every pop clamps it. The `inactive` flag is gone.
- B11: per kind of inline HTML (comment, CDATA, declaration, instruction), the earliest opener that reached the end without a closer. Later openers of that kind fail at once, because they scan a suffix of the same bytes for a fixed closer.
  - Found along the way: the baseline families for instructions, CDATA, and declarations started at a line start, where they open an HTML flow block, which is linear. With an `a ` prefix they reach inline HTML, and there 1.0.0 is quadratic too, with an exponent of about 2.
- A6: a per-length table of marker runs, recorded after the first opener without a closer. Once a recording scan reaches the end, an opener with no later run of its length fails at once. `` ` `` and `$` have separate tables, and there is no length cap.
- B10: the last failed trailing-punctuation check is kept as a (start, failure) pair. A check that starts inside it fails at once. The check is deterministic and is in its initial state at every place a check can start.
- B9: the last invalid `www.` domain is kept as a range, up to the last underscore in its last two segments, or up to its end when no letter or digit was seen. A `www.` inside that range fails at once. Domains end at the same place wherever they start before it.
- Mutants caught, by the differential or new tests:
  - no clamp on `nok`, 550 differences;
  - one memo for all HTML kinds, caught by the new cross-kind tests;
  - never recording runs, 30 differences, plus a test;
  - an inclusive trail range, caught by a test;
  - a `www` range one past the underscore, caught by a test.
- Two defensive guards cannot be pinned by tests: the run table's region check and the B4 guard. The code never reaches the states they protect against.
- Scaling: every inline family is linear. For example, `unclosed-comments` goes from 65 KB in 6.0 s to 8 MB in 1.3 s; `escaped-backticks` from 32 KB in 2.8 s to 4 MB in 1.9 s; `www-underscores` from 40 KB in 4.2 s to 10 MB in 1.9 s; and `autolink-trail-punctuation` from 32 KB in 5.5 s to 8 MB in 0.56 s.
- The pathological tests cover every inline family. Each case fails on 1.0.0, at 55x to 320x against thresholds of 20 to 153, and all pass on the stack in about 2 s.

### Compile review follow-up

- Major, fixed: B8's fast path was wrong with `character_escape` off. Text then starts a label at `\[`, while definition labels still read `\[` as escaped, so `[\[\]]` with a matching definition stopped linking. The fast path now needs character escapes on. A regression test fails without the fix. The harness gained a sixth configuration, `reduced`, with several CommonMark constructs off.
- Minor, fixed:
  - `thread::scope` in the tests needed Rust 1.63; the tests now use `thread::Builder::spawn`, so clippy shows only the existing `char.rs:44` warning again.
  - A helper had been inserted inside a doc comment.
  - `to_html` now panics on a missing definition only outside image alts, as 1.0.0 does.
  - Stale comments and test docs were updated.
- The review's own differential used 6 option sets and 200,000 random inputs each, with 0 differences. It confirmed that nothing writes between a label's end and its media's end.

### Inline review follow-up

- Critical, fixed: the A6 run table gave wrong output. A scan that found its closing sequence after a complete scan replaced `last_start` entries with earlier runs, so a later opener failed. With default options, `` ``` ```` ``a`b`` `c` `` lost its last code span. Recording now stops once a scan reaches the end. A regression test fails without the fix.
- The differential had missed it: its random alphabet lacked `$`, `<!--`, `<?`, and CDATA tokens, and single inputs rarely repeat one construct. The generator now draws a third of its inputs from 2 to 5 tokens only, and has those tokens. It catches the A6 mutant with 4 differences in 200,000 inputs; before, it found none. The `reduced` configuration now also turns on single-dollar math.
- Nits, fixed: `www_nok` lost a branch that cannot run, since a `www.` domain always has letters; the trail memo doc now says the check returns to its initial state after one byte at `&name;`.
- Differential after the fix: 0 differences over 7,010,122 comparisons.

### MDX (A7, A8, B12, reading budget)

The MDX branch is stacked on inline.

- A8: the line index is built only on the error path, and `to_point` uses a binary search.
- A7: collection of expression and ESM bodies is incremental. For the same time, markdown-rs handles 262 KB of `mdx-esm-blank-lines`, up from 33 KB, and 131 KB of `mdx-expression-string-braces`, up from 16 KB. The host parser's share stays quadratic: it reads each prefix again.
  - A debug assertion compared every incremental collect with a full one. It made debug builds quadratic, so it is gone; the `mdx-aware` differential covers the equivalence through every host decision and error position.
- Reading budget, `mdx_parse_budget_factor`: off by default. With `Some(k)`, parsing stops with a `mdx-parse-budget` error once k times the document size has been read. It counts bytes passed to the host parser and, without one, bytes of each brace-counted expression at its close.
  - With the stand-in parser, 21 corpus documents read a median of 0.124 and at most 0.743 times their size. The docs suggest a small factor such as 4.
- B12, exact memo: after a flow expression fails at its end, later scans record each inner `{`, its matching `}`, and whether the rest of that line makes a flow expression fail. A later attempt at a recorded brace fails without scanning, unless the current line is lazy.
  - `mdx-flow-expression-retries`: 1.0.0 took 9.9 s for 48 KB; now 6 MB takes 5.4 s. The block-quote variant is linear too.
  - Not covered, so bounded only by the budget: a tag after the closing brace (`}<a/>x`), and JSX flow tags whose attribute expressions span the lines of later tags. Both stay quadratic without a budget, as in 1.0.0; with a factor of 4 both are linear.
  - The lazy-line guard is defensive: the flow tokenizer revisits a brace only after reverts that error on lazy lines. Like the B4 guard, no test can reach it.
  - Tests: memo cases in `tests/mdx_expression_flow.rs` catch a memo that ignores the rest of the line and one that treats `<` as a failure. The random differential does not reach the memo's success path: that needs a failed line, a code span, and balanced braces together.
- `Tokenizer` size: the MDX fields took `Tokenizer` from 944 to 1064 bytes. glibc treats a request over 1000 bytes as large and first merges freed small chunks, and the flow tokenizer is boxed once per document. That cost tiny documents up to 0.75% of instructions. Both MDX fields are now boxed on first use; `Tokenizer` is 960 bytes, and a unit test holds it at or under 1000.
- `ParseOptions` serializes the new field as `"mdxParseBudgetFactor":null`; `tests/serde.rs` expects it. The earlier budget commit's check loop had missed that test.
- Pathological tests for MDX pass in debug and release. Each unbudgeted case fails on 1.0.0 at 66x to 73x against a threshold near 20. The budgeted case fails at 60.8x with the budget removed.
- Differential: 0 differences over 7,010,269 comparisons.
- Instructions against 1.0.0 for the whole stack: tiny −12.13% to −13.63%, small −13.29% to −15.14%, across all 7 configurations. Against the inline stage, the MDX stage moves tiny by −0.24 to +0.41 points and small by −0.51 to +0.01 points, within the noise between builds.

### Deep trees (B18)

- `Node::to_string` walks a stack of sibling iterators instead of recursing. A parent whose children are all literals allocates only the result string, as before. A test calls it on 20,000 nested block quotes on a 64 KB stack; the recursive version overflows there.
- Stack use of the derived operations, on a 2 MB thread in release: `Drop` survives 32,672 nested block quotes, `Clone` 3,036, `Debug` 4,080, and `PartialEq` 43,565. At 2 bytes per level, a 6 KB document overflows `Clone`. The options go to the draft `deep-tree-drop.md`.

### MDX review follow-up

- Correct under review: 0 differences over 9.02 million inputs × 8 option sets, with `None`.
- Wrong guidance, fixed in the docs: with a JavaScript parser, each `}` in an expression passes the whole expression so far again, so ordinary documents need large factors. Measured: an array of 10 small objects needs 6, 100 objects 51, and 1000 objects 501. The docs now say so and give these numbers instead of suggesting 4.
  - This is a limit of the "K times document size" unit, not of the counting: the host's reads grow with the square of the closing braces in one expression, so no fixed factor both stops crafted input and accepts large data expressions. The developer should decide whether that unit stays for parsers.
- `None` still charged a counter that started at `usize::MAX`; on 32-bit targets, crafted input could exhaust it where 1.0.0 is only slow. The budget is now `Option<Cell<usize>>`, and `None` charges nothing.
- `skip_serializing_if` keeps the serialized options identical to 1.0.0's.
- New tests catch mutants that the suite missed: whitespace and `\r` after a closing brace in `fails_after`, a collect cursor that is not reset, and a budget that is not shared.
- Open for upstream: `ParseOptions` is not `#[non_exhaustive]`, so a new public field breaks struct literals that list every field. That is a semver-major change by cargo's rules.
- Pre-existing, for a draft issue: `extern crate std` in `src/util/gfm_tagfilter.rs` breaks `no_std` builds, also in 1.0.0.

### Scanning (Stage 2)

Branches: `perf/scalar-run-batching` on the MDX branch; `perf/simd-memchr` and `perf/simd-classifier` on it.

- Scalar batching: `Tokenizer::consume_run(stop)` consumes the current byte and then every byte up to the next one in a 256-entry stop table, bounded by the end of the current push. Every table includes `\n`, `\r`, and `\t`, which move the point differently.
  - Pass 1, line scanners (`document::flow_inside`, `content::chunk_inside`, `paragraph::inside`) and text data: small −30% to −38% and tiny −8.5% to −9.9% against the MDX stage.
  - Pass 2, data stops only where an enabled construct can start: `text::may_start` mirrors the dispatch in `text::before`, and `h`/`w` inside words no longer split data. Two static tables, with and without `H W h w`, avoid building tables per parse; a first version built them per parse into `ParseState` and cost tiny documents up to +0.73%, from moving the larger `ParseState`. Small −2.3% to −5.6% against pass 1.
  - Pass 3, table header cells, fenced code lines, ATX heading text, and code span content: small GFM −9.0% to −9.4%, small CommonMark −3.4%.
  - Whole stack against 1.0.0: tiny −20.9% to −22.3%, small −46.0% to −51.5% in instructions. Differential: 0 differences at each pass.
  - Mutants caught: runs that ignore the push end (43 failing tests) and runs that do not update `previous` (1).
- The timing tests became fragile: small cases took 0.5 to 2 ms and failed under load. `assert_near_linear` now doubles `n` until the small case takes 5 ms. All six test functions still fail on 1.0.0, at 25x to 83x.
- Remaining per-byte work is spread thin, about 696,000 state calls for the 313 KB small bin, mostly construct attempts at line starts. Moving and dropping the large `State` enum, whose `Error` variant holds a `Message`, costs about 8% of small GFM instructions; boxing it is a follow-up outside this plan, and it touches `state.rs`, which the plugin branch also changes.

SIMD variants, instruction change against scalar batching (small bin, then tiny):

| Variant | Build | Small | Tiny | Clean build | `.text` | New dependencies |
|---|---|---|---|---|---|---|
| Scalar | default | – | – | 1.3 s | 507,561 B | none |
| `memchr3` in line scanners | default (SSE2) | −1.9% to −3.1% | −0.7% to +0.1% | 1.6 s | 506,751 B | `memchr` |
| `wide` nibble classifier | default (SSE2) | +0.9% to +2.4% | −0.4% to +0.1% | 4.3 s | 513,346 B | `wide`, `safe_arch`, `bytemuck` |
| `wide` nibble classifier | `+ssse3` | −3.2% to −4.2% | −0.5% to −1.1% | | | |
| `fearless_simd` nibble classifier | default (SSE2) | +0.4% to +1.9% | −0.5% to −1.1% | 4.5 s | 510,283 B | `fearless_simd`, `libm` |
| `fearless_simd` nibble classifier | `+sse4.2` | −0.8% to −2.1% | −0.8% to −1.3% | | | |

- Both SIMD libraries shuffle bytes with `pshufb`, which x86-64 has only from SSSE3. On a default build both fall back to a scalar loop over 16 lanes and are slower than the table. `wide` chooses at compile time; `fearless_simd` detects at run time only with `std`, which this crate does not use, and without `std` it also requires `libm`.
- `memchr` works on a default build and fits only the three-byte line stop set.
- Text data runs in the corpus have a median of 10 to 12 bytes and a mean of 18 to 23; 81% to 85% of their bytes are in runs of 16 bytes or more.
- A unit test compares the classifier with the table on 10,000 random inputs, including non-ASCII bytes, in each build.
- All variants build for `aarch64-unknown-linux-gnu` and `wasm32-unknown-unknown`, with and without `+simd128`. On aarch64 the shuffle is part of the baseline (NEON), so the classifiers vectorize there by default; that speed is not measured.
- Wall clock, criterion with GitHub Flavored Markdown options: every variant runs 1.2 to 1.4 times as fast as 1.0.0 on tiny documents and 1.7 to 2.0 times on larger ones, about 11 to 16 MiB/s against 6 to 9. The variants cannot be ranked by wall clock on this machine: 1.0.0's own throughput varied from 7.5 to 9.1 MiB/s on small documents between runs. Instruction counts are the basis for the comparison above.

### Plugin branch cost (feat/plugin-prototypes), measured at the developer's request

All against 1.0.0, with no plugins registered.

| Commit | Output | Tiny mdast | Small mdast | Tiny HTML | Small HTML |
|---|---|---|---|---|---|
| `f2f43d8` | 0 differences, 507,090 comparisons | +9.9% to +10.8% | +12.4% to +13.5% | – | – |
| `cae4b6a` | – | +0.84% to +1.28% | −0.11% to +0.34% | +1.48% to +1.79% | +0.63% to +0.81% |
| `49d1b7b` | 0 differences, 2,507,160 comparisons | +0.27% to +0.80% | −0.29% to +0.55% | +0.90% to +1.60% | +0.70% to +1.00% |

- `f2f43d8`: a larger `event::Name` holding an `&'static str`, derived `State` equality over data-carrying names, and the early match in `state::call`.
- `49d1b7b`, tiny CommonMark HTML, +842,612 instructions: about 480,000 come from comparing the 4-byte `event::Name`, whose `Extension(u8, u16)` variant makes derived equality compare fields; `State::to_result` adds 87,723; the per-event extension check in `to_html::handle` adds 59,514.
- The plugin session plans a fieldless `Name::Extension` and a gated check, then a re-measure.

## Assumptions

Override any of these at the checkpoint.

1. Work happens in a git worktree at `markdown-rs-worktrees/parse-performance`, next to the main checkout, cut from `upstream/main` after a fetch. That keeps it away from the other session's checkout. Prototypes that run in parallel get their own worktrees in the same directory.
2. Branches use the `perf/` prefix: a harness branch, then one branch per prototype on top of it. SIMD branches build on the scalar-batching branch.
3. Commits:
   - Scaling commits carry `Refs: wooorm/markdown-rs#113`.
   - Batching and SIMD commits note that a local draft issue exists.
   - The plan lives at `plans/113-parse-performance.md` on the harness branch, following the repo convention, and stays out of any upstream PR.
   - Nothing is pushed or posted without asking.
4. MSRV stays 1.56 for the algorithmic fixes: no `let-else`, `OnceCell`, `is_some_and`, or `retain_mut`. Only the SIMD feature branches raise it: 1.61 for memchr, 1.89 for wide and fearless_simd.
5. The differential baseline is `markdown = "=1.0.0"` from crates.io under a renamed dependency in the harness crate. It compares `to_mdast` with positions, HTML, and error text. Fallback: dump baseline outputs from a `1506572` worktree.
6. EditMap: benchmark the append log against PR #197 and PR #217. If the upstream PRs fall within noise on the small bins, prefer #217 plus the two-sided entry, and credit its author.
7. No `unsafe` in markdown-rs. New dependencies are optional, with `default-features = false`. `#![no_std]` stays.
8. Rejected outright: syntax caps (a 999-byte cap on every label, cmark's `MAXBACKTICKS`, the cmark-gfm segment rule), a straight `openers_bottom` port, and depth-only attention scope.
9. Provisional gates, revised after the baseline run:
   - Identical output on the full corpus.
   - Targeted families go from about 4x time per input doubling to about 2x.
   - Small bins rise by no more than about 1% in instructions.
   - Larger natural documents slow down by no more than about 3%.
   - SIMD must beat scalar batching by about 10% end to end, and engages only above a measured run length.
10. Spikes are capped at 50 lines per pass. Larger prototypes split into passes.

## Scope and per-directory impact

- `performance/` (new, unpublished, its own workspace): the harness crate.
- `fuzz/`: one differential target.
- `src/`: the fixes above, and the reading budget as one public field in `configuration.rs`.
- `tests/`: pathological regression tests in the cmark style, one per fixed family.
- `Cargo.toml`: optional SIMD features on the SIMD branches only.

## Skills

- spike-and-stabilize-generic: the process for each prototype.
- deep-research: the research phase.
- humanize: commit messages and any issue or PR text, linted with Vale.
- The built-in `/code-review` skill handles the stabilize review, because code-review-generic does not cover Rust.

## Plan

### Stage 0: measurement harness (`perf/measurement-harness`)

1. Create the worktree and the `performance/` crate. The corpus loader covers every size bin, generated dense GFM tables, and GFM plus MDX documents.
2. Add scaling generators for every input family in the tables above, at six or more geometric sizes, recording bytes, events, nodes, and depth.
3. Add a criterion bench per bin and option set, a scaling runner that reports the time ratio per doubling, a counting allocator, and a small-document runner for callgrind.
4. Add the differential runner and the `fuzz/` differential target. A panic in both builds counts as an equal outcome.
5. Run the baseline into `.agent-tmp/perf-simd/baseline/`, and profile the small and large bins separately with callgrind.
6. Confirm the `a****b c** d*  e**` output and whether it differs from cmark. Record the result either way.

### Stage 1: algorithm prototypes, one branch each, in order of impact

1. `perf/edit-map-log`: A2a and A2b, benchmarked against PR #197 and PR #217.
2. `perf/attention-delimiter-stack`: A1c, A1b, and the stack with frames first; the clamped bounds second.
3. `perf/container-continuation`: B3, B5, and B6 first; B4 second.
4. `perf/inline-scan-memos`: B11, A6, B7, and B8 first; B9 and B10 second.
5. `perf/compile-nesting`: A3, A4, A5, B13 to B17, and the iterative `to_string` (B18). The Drop write-up goes to `.agent-tmp/perf-simd/drafts/`.
6. `perf/mdx-collect`, in three passes:
   - A7 and A8.
   - The reading budget: a `Cell<usize>` counter in `ParseState`, checked before each host call.
   - The B12 brace-match memo, behind differential fuzzing. If it can't be shown exact, drop it and count agnostic retries in the budget instead.

Each cognitive-4 pick (A1a bounds, B4, B6, B8, B9, B17) lands with:
- a doc comment naming its invariant
- `debug_assert!`s that check the invariant
- targeted tests
- a differential fuzz run

After each prototype, run the differential runner, the small-bin instruction counts, the scaling slope for its families, the corpus bench, and the full check loop. A prototype that fails the small-bin gate is redesigned; it does not land as a tradeoff. Then re-profile.

### Stage 2: scanning prototypes, on top of the Stage 1 winners

1. `perf/scalar-run-batching`: the stop masks, the enabled-construct mask, the `h/w` rule, and `consume_run`. First in `partial_data::inside`, then in the document and content line scanners. Include a debug-build cross-check that replays byte by byte and compares `Point`. Add the punctuation `binary_search` and the `char.rs` allocation fix.
2. `perf/simd-memchr`: `memchr2` and `memchr3` for the line scanners, raw text, HTML bodies, and the line index.
3. `perf/simd-classifier`: a nibble classifier for the text stop set, written once for wide and once for fearless_simd with `libm`. Both share one scalar-contract test suite.
4. Measure throughput, code size, and compile time. Build-check wasm32 with and without `+simd128`, and aarch64. Installing those targets needs approval.

### Stage 3: decision report

- A results table per prototype: exactness, slope before and after, small-bin instruction counts, ns/byte per bin, peak heap, code size, and dependencies.
- Upstreamable patches ranked, smallest first.
- Draft issues in `.agent-tmp/perf-simd/drafts/`, never posted:
  - batching and SIMD
  - the GFM `to_html` panic at `src/to_html.rs:197`
  - `Node::to_string` returning `""` for images
  - multi-line label ids that may include block-quote prefixes
  - the B18 Drop options

Written so far, each checked against 1.0.0: `deep-tree-drop.md`, `to-html-buffer-panic.md` (both compilers panic on `(\n-\n=\n_\n-`), `subtokenize-link-title-panic.md` (`[](/ "b \n")` panics while parsing), `to-string-image-alt.md`, `label-block-quote-prefix.md`, `list-after-unclosed-fence.md`, `mdx-jsx-fragment-in-link-label.md`, `gfm-tagfilter-extern-std.md`, and `memory-per-byte.md`. The batching and SIMD draft waits for wall-clock numbers.

Proposed upstream split, in landing order. Each patch carries its own tests and pathological cases, and none needs the harness:

1. Punctuation lookup: `binary_search` in `util/char.rs`, and a test that the table is sorted. It lands first so the attention tests can fail on the old code in debug builds.
2. Edit map append log (A2a, A2b), with its unit tests.
3. Attention delimiter stack with scope frames and clamped bounds (A1a to A1c).
4. Container continuation (B3 to B6).
5. Compile phase, in four parts: definition lookups (A4); the open-node stack and list spread (A5, B17); media labels in `to_html` (B13 to B16) with the label fast path (B8); table pipe escapes (A3).
6. Inline scan memos (B7, B9, B10, B11, A6).
7. MDX: the lazy line index (A8), incremental collection (A7), and the brace memo (B12). The reading budget goes separately, because it adds public API.
8. Iterative `Node::to_string` (B18).
9. Run batching (Stage 2 scalar), with the `Tokenizer` size test.
10. Optional: the `memchr` feature, if upstream wants a dependency for 2% to 3% on small documents.

Upstream PRs #197 and #217 target the edit map; patch 2 supersedes both, so it should credit them and say why the append log is faster at every size.

## Verification

- Each branch runs `cargo fmt --check`, `cargo clippy --all-features --all-targets --workspace`, and `cargo test --all-features --workspace`, matching CI. Output goes to `.agent-tmp/perf-simd/<branch>/`.
- The differential runner reports zero differences in all option sets. The differential fuzz target runs for a fixed time budget on each branch.
- The scaling ratio per doubling is recorded for each targeted family, before and after. The pathological tests pass within their time budget.
- The small-bin callgrind instruction counts stay within the gate.
- SIMD branches build for wasm32 and aarch64, and pass tests on x86-64 with the feature on and off.

## Clarify transcript

Mid-turn note from the developer: "with superlinear paths be very thoughtful in the fix, small documents are more common, the fix should not penalize small documents for rare large ones". This became the small-document rule.

Round 1:
- Issue reference: "#113 plus local drafts".
- SIMD variants: "memchr, small sets" and "wide vs fearless_simd". No `unsafe` ceiling.
- MDX host re-parse: "Add opt-in cap, off by default".
- Deep trees: "Fix to_string, write up Drop".

Round 2:
- Cap error: "Message with its own rule_id".
- Side bugs: "Draft issues, fix none here".
- Cap unit: the developer asked for a plainer explanation with trade-offs. The preference was a cap that is simple and truly bounds the work. The answer after the breakdown: "K times document size".

Plan review: the developer asked for several alternative fixes per item, each weighed on compute, memory, code complexity, and cognitive complexity. That became "Fix alternatives".

Round 3:
- Cognitive-4 picks: "Accept with invariant guards". Each lands with its invariant documented, `debug_assert!`s, tests, and a differential fuzz run.
- Agnostic MDX retries (B12): "Exact memo, budget fallback".

Round 4, after the MDX and scanning reviews:
- SIMD build checks: "Install both targets". Installed `wasm32-unknown-unknown` and `aarch64-unknown-linux-gnu`.
- MDX budget coverage: first "Break this down first", then "Keep it as is, docs only". The developer also asked which alternatives to brace counting could fit; the answer is recorded under "Design decisions and alternatives".

## Progress log

- 2026-09-26: research complete; plan drafted.
- 2026-09-26: added the small-document rule at the developer's request.
- 2026-09-26: clarify rounds 1 and 2 answered.
- 2026-09-26: added alternative fixes with weights; found the shared checkout in use by another session, so the work moves to a worktree.
- 2026-09-26: plan approved. Created the worktree on `perf/measurement-harness` from `upstream/main` (`1506572`). Pinned the lockfile around the `swc_common` and `serde` break; baseline tests pass. Agreed the compatibility rules with the plugin session.
- 2026-09-26: harness built and validated; baseline measured. Edit-map variants compared: the append log wins. Attention steps 1 and 2 and the punctuation lookup added, stacked on it. Container fixes B3 to B6 added; plugin branch measured. Compile-phase fixes added; pathological tests now cover deep trees. Inline memos added; compile review fixed a fast-path bug with escapes off.
- 2026-09-26: inline review found an A6 bug, fixed; the differential gained a focused mode. MDX stage done: A7, A8, the reading budget, and the B12 memo, with JSX-bound retries left to the budget. Found and fixed a `Tokenizer` size cliff. B18 `to_string` is iterative. Plugin commit `49d1b7b` measured; compatibility rules 7 to 9 agreed.
- 2026-09-26: MDX review fixed: budget guidance, `None` charges nothing, serde output, new tests. Stage 2: scalar batching in three passes, then memchr, `wide`, and `fearless_simd` variants measured in instructions, build time, and code size.
- 2026-09-26: scanning review fixed: timing tests serialized with fixed sizes and a retry, seven marker tests, the memchr slice guard. Budget decision recorded ("Keep it as is, docs only"). Cross-target builds and wall-clock benchmarks done. Draft issues written.
