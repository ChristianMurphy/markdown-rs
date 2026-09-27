use markdown::{to_mdast, MdxSignal, ParseOptions};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

/// Whether a test is timing parses, as parallel large parses slow each other.
static TIMING: AtomicBool = AtomicBool::new(false);

/// Holds `TIMING` until dropped, also when a parse panics.
struct Timing;

impl Timing {
    fn start() -> Timing {
        while TIMING
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            thread::sleep(Duration::from_millis(1));
        }
        Timing
    }
}

impl Drop for Timing {
    fn drop(&mut self) {
        TIMING.store(false, Ordering::Release);
    }
}

/// Fastest of a few parses, on a large stack for deep trees.
fn fastest_parse(value: String, options: fn() -> ParseOptions) -> Duration {
    let _timing = Timing::start();
    thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || {
            let options = options();
            (0..5)
                .map(|_| {
                    let start = Instant::now();
                    to_mdast(&value, &options).unwrap();
                    start.elapsed()
                })
                .min()
                .unwrap()
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Asserts that 8 times the repetitions parse within 2.5 times the byte growth.
fn assert_near_linear(
    name: &str,
    generate: fn(usize) -> String,
    n: usize,
    options: fn() -> ParseOptions,
) {
    let small_value = generate(n);
    let large_value = generate(n * 8);
    let growth = large_value.len() as f64 / small_value.len() as f64;
    let measure = || {
        let small = fastest_parse(small_value.clone(), options);
        let large = fastest_parse(large_value.clone(), options);
        (large.as_secs_f64() / small.as_secs_f64(), small, large)
    };
    let mut result = measure();
    if result.0 >= growth * 2.5 {
        result = measure();
    }
    let (ratio, small, large) = result;
    assert!(
        ratio < growth * 2.5,
        "{}: {:.1}x the input took {:.1}x as long ({:?} vs {:?})",
        name,
        growth,
        ratio,
        large,
        small
    );
}

#[test]
fn pathological_edit_map() {
    let commonmark = ParseOptions::default;
    let gfm = ParseOptions::gfm;

    assert_near_linear("lines", |n| "a\n".repeat(n), 2_500, commonmark);
    assert_near_linear("blank lines", |n| "\n".repeat(n), 5_000, commonmark);
    assert_near_linear(
        "table rows",
        |n| format!("| a |\n| - |\n{}", "| b |\n".repeat(n)),
        1_000,
        gfm,
    );
}

#[test]
fn pathological_attention() {
    let commonmark = ParseOptions::default;

    assert_near_linear("closers only", |n| "a_ ".repeat(n), 8_000, commonmark);
    assert_near_linear(
        "openers of another marker",
        |n| format!("{}{}", "*a ".repeat(n), "a_ ".repeat(n)),
        4_000,
        commonmark,
    );
    assert_near_linear("emphasis pairs", |n| "a*".repeat(n), 4_000, commonmark);
    assert_near_linear(
        "mismatched markers",
        |n| "*a_ ".repeat(n),
        4_000,
        commonmark,
    );
    assert_near_linear(
        "ambiguous closers",
        |n| format!("a**b{}", "c*".repeat(n)),
        4_000,
        commonmark,
    );
}

#[test]
fn pathological_containers_and_trees() {
    let commonmark = ParseOptions::default;

    assert_near_linear(
        "deep list",
        |n| format!("{}a", "- ".repeat(n)),
        250,
        commonmark,
    );
    assert_near_linear(
        "deep list with an indented line",
        |n| format!("{}a\n{}b", "- ".repeat(n), "  ".repeat(n)),
        250,
        commonmark,
    );
    assert_near_linear(
        "deep list with blank lines",
        |n| format!("{}x{}", "- ".repeat(n), "\n".repeat(n)),
        250,
        commonmark,
    );
    assert_near_linear(
        "staircase list",
        |n| {
            (0..n)
                .map(|depth| format!("{}* a\n", "  ".repeat(depth)))
                .collect()
        },
        60,
        commonmark,
    );
    assert_near_linear(
        "deep block quote",
        |n| format!("{}a", "> ".repeat(n)),
        500,
        commonmark,
    );
    assert_near_linear(
        "one strong run",
        |n| format!("{}a{}", "*".repeat(n), "*".repeat(n)),
        500,
        commonmark,
    );
}

#[test]
fn pathological_inline_scans() {
    let commonmark = ParseOptions::default;

    assert_near_linear(
        "image and link starts",
        |n| "![[]()".repeat(n),
        1_000,
        commonmark,
    );
    assert_near_linear(
        "unclosed comments",
        |n| format!("a {}", "<!--".repeat(n)),
        1_000,
        commonmark,
    );
    assert_near_linear(
        "unclosed instructions",
        |n| format!("a {}", "<?".repeat(n)),
        1_000,
        commonmark,
    );
    assert_near_linear(
        "unclosed CDATA",
        |n| format!("a {}", "<![CDATA[".repeat(n)),
        1_000,
        commonmark,
    );
    assert_near_linear(
        "rising backtick runs",
        |n| {
            (1..=n)
                .map(|size| format!("{} ", "`".repeat(size)))
                .collect()
        },
        60,
        commonmark,
    );
    assert_near_linear(
        "escaped backticks",
        |n| "\\`` ".repeat(n),
        1_000,
        commonmark,
    );
    assert_near_linear(
        "unclosed declarations",
        |n| format!("a {}", "<!A".repeat(n)),
        1_000,
        commonmark,
    );
}

#[test]
fn pathological_autolink_literals() {
    let gfm = ParseOptions::gfm;

    assert_near_linear("www after underscores", |n| "_www.".repeat(n), 1_000, gfm);
    assert_near_linear(
        "trailing punctuation",
        |n| format!("http://a.b/{}x", "!".repeat(n)),
        1_000,
        gfm,
    );
    assert_near_linear(
        "trailing underscores",
        |n| format!("www.a{}b", "._".repeat(n)),
        1_000,
        gfm,
    );
}

/// MDX with a stand-in JavaScript parser that decides in constant time.
fn mdx_constant_parser() -> ParseOptions {
    ParseOptions {
        mdx_expression_parse: Some(Box::new(|value, _kind| {
            if value.ends_with(';') {
                MdxSignal::Ok
            } else {
                MdxSignal::Eof(
                    "more".into(),
                    Box::new("test".into()),
                    Box::new("eof".into()),
                )
            }
        })),
        mdx_esm_parse: Some(Box::new(|value| {
            if value.ends_with(']') {
                MdxSignal::Ok
            } else {
                MdxSignal::Eof(
                    "more".into(),
                    Box::new("test".into()),
                    Box::new("eof".into()),
                )
            }
        })),
        ..ParseOptions::mdx()
    }
}

#[test]
fn pathological_mdx() {
    let mdx = ParseOptions::mdx;

    assert_near_linear(
        "expression asking for more",
        |n| format!("{{{};}}", "}".repeat(n)),
        4_000,
        mdx_constant_parser,
    );
    assert_near_linear(
        "ESM asking for more",
        |n| format!("export const a = [\n{}]", "b,\n\n".repeat(n)),
        500,
        mdx_constant_parser,
    );
    assert_near_linear(
        "flow expressions failing after their brace",
        |n| format!("{}{}x", "{\n".repeat(n), "}".repeat(n)),
        500,
        mdx,
    );
    assert_near_linear(
        "flow expressions failing after their brace in a block quote",
        |n| format!("{}> {}x", "> {\n".repeat(n), "}".repeat(n)),
        500,
        mdx,
    );
}

#[test]
fn pathological_deep_to_string() {
    thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(|| {
            let value = format!("{}a", "> ".repeat(20_000));
            let tree = to_mdast(&value, &ParseOptions::default()).unwrap();
            // Too small for a frame per level.
            let (tree, value) = thread::Builder::new()
                .stack_size(1 << 16)
                .spawn(move || {
                    let value = tree.to_string();
                    (tree, value)
                })
                .unwrap()
                .join()
                .unwrap();
            assert_eq!(value, "a", "should support `ToString` on deep trees");
            drop(tree);
        })
        .unwrap()
        .join()
        .unwrap();
}
