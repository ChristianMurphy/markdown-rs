---
status: Active
---

# Make `State` smaller (#229)

## Context

Every state function returns `State` by value, and the tokenizer moves and drops it on each transition. `State::Error` holds a `Message` of about 48 bytes, so every `State` is that large. Profiling on `perf/parse-performance` put moving and dropping `State` at about 8% of GFM instructions on medium documents, and `drop_in_place<State>` at 3.8% on short CommonMark documents (`plans/113-parse-performance.md`, lines 498 and 671). wooorm/markdown-rs#228 lists this under "What's left".

`State` is crate-private: `mod state` and `mod tokenizer` are private in `src/lib.rs`. Public functions return `Result<_, message::Message>`, which this work doesn't change.

## Decisions

Clarify round, 2026-09-27:

- Design: box first, unit variant if needed. Prototype `Error(Box<Message>)` first. Try a unit `Error` variant, with the message on the tokenizer, only if boxing recovers little of the 8%, and ask before starting it.
- Base: `perf/parse-performance` at `ed41bb7`, where the cost was measured. Branch `perf/state-size`.
- Tidy: include the `matches!` comparisons in `tokenizer.rs`, measured as a separate step.
- Tracking: a new issue, drafted but not posted until the developer says so. Filed as wooorm/markdown-rs#229.

The developer approved the plan on 2026-09-27.

## Steps

1. Baseline at `ed41bb7`: `size_of::<State>()`, and callgrind instructions on short and medium documents in all 7 option sets.
2. Box the payload: `Error(Box<Message>)`, with one `#[cold]` constructor for the 10 construction sites, and `to_result` adjusted.
3. Replace `state == State::Nok` and `state == State::Ok` in `tokenizer.rs` with `matches!`.
4. Only if steps 2 and 3 recover little: a unit `Error` variant, after asking.
5. Stabilize: a test that pins `size_of::<State>()`, fmt, clippy with all features and targets, the full tests, the 1,000,000-input differential against 1.0.0, the `Tokenizer` size test, and a review.
6. Draft the new issue with the measurements.

## Alternatives

- `Error(Box<Message>)`: `State` goes from about 48 to about 16 bytes. Drop glue remains, as a tag check. It touches `state.rs` and 5 files with construction sites.
- A unit `Error` variant, with the message kept in `TokenizeState`: `State` goes to about 4 bytes and becomes `Copy`, with no drop glue. It changes how errors come back from the document's child tokenizer (`document.rs`, `child.flush`).
- Leave `State` alone: no change, and the 8% stays.

The plugin branch (`feat/plugin-prototypes`) adds `Name` variants but leaves the `State` enum alone. After a merge, it has one more `State::Error` site, in `extension.rs`.

## Verification

- `size_of::<State>()` before and after, measured.
- Callgrind instructions against `ed41bb7`, on short and medium documents, in all 7 option sets. No option set may get slower beyond noise.
- The differential: 0 differences over 7,010,269 comparisons.
- `cargo fmt --check`, `cargo clippy --all-features --all-targets --workspace`, and `cargo test --all-features --workspace`.

## Progress log

- 2026-09-27: worktree `markdown-rs-worktrees/state-size` on `perf/state-size` from `ed41bb7`.
- 2026-09-27: spike. `size_of::<State>()` went from 48 to 16 bytes with `Error(Box<Message>)` and a `#[cold]` constructor, `State::error`, used at all 10 construction sites. Callgrind against `ed41bb7`: short documents −8.55% to −9.27%, medium −8.54% to −9.85%. Then `matches!` for the three `State` comparisons in `tokenizer.rs`, against the boxed step: short −4.23% to −4.47%, medium −3.84% to −4.75%. Together against `ed41bb7`: short −12.49% to −13.17%, medium −12.05% to −14.14%. The diff is 7 files, +33 −15. 216 tests pass. The unit `Error` variant isn't needed.
- 2026-09-27: stabilized. A new test, `test_state_size`, pins `State` at 16 bytes or less, and it failed at 48. `partial_mdx_expression.rs:125` uses `matches!` too. Review, by sub-agent with no critical findings, led to these changes: `PartialEq` dropped from `State`, since nothing uses it and a future `==` now fails to compile; a verb-first doc comment on `State::error`; and callgrind re-run on the final tree. Checks: fmt clean, clippy with only the existing `char.rs:44` warning, all 216 tests passing, and 0 differences over 7,010,269 comparisons. WIP commit `2687270`.
- 2026-09-27: codegen-unit finding. With the default release profile, the final tree measures against `ed41bb7` at −11.10% to −11.75% on short documents and −10.81% to −12.72% on medium ones. That's about 1.5% less than the spike step, and the whole gap comes from converting the MDX-only `==`: with it reverted, short CommonMark measures 36,236,142 against 36,806,689. With `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`, both variants measure identically (short CommonMark 36,019,365, medium GFM 185,690,908), so the gap is codegen-unit partitioning, not code. The base also measures lower with one codegen unit (38,689,812 and 199,922,294), so the robust gain is about 7% (−6.90% and −7.12% on those two). A full 7-option-set run with one codegen unit is in progress.
- 2026-09-27: robust numbers. With one codegen unit, all 7 option sets against `ed41bb7`: short documents −6.34% to −7.10%, medium −6.04% to −7.12%. Wall clock with the default release profile, fastest of 40 passes, relative to 1.0.0 in the same process: short −5.3% to −7.1%, medium −6.9% to −7.3%, long (4 to 64 KB) −5.7% to −8.3%. Raw times moved −1.8% to −9.7%, while 1.0.0 itself drifted up to 5.4% between runs.
- 2026-09-27: measured on 1.0.0 as well, in a scratch worktree with the same change (9 error sites there). Instructions against 1.0.0 with one codegen unit: short documents −6.40% to −6.86%, medium −7.23% to −8.11%. With the default release profile: short −10.42% to −10.97%, medium −11.62% to −12.91%. Wall clock, 1.0.0 in the same binary: 6.4% to 8.8% faster on short, medium, and long documents. 199 tests pass, and the differential shows 0 differences over 7,010,269 comparisons.
- 2026-09-27: pushed `perf/state-size-1.0.0` (`3b71360`, the change on 1.0.0) and this branch. Filed as #229.
