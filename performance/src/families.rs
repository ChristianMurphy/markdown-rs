//! Generated inputs that grow with a repetition count `n`.
//!
//! Each family targets one mechanism from the plan's inventory; `ids` names
//! it. Sources: cmark `test/pathological_tests.py`, micromark `test/perf.js`,
//! and the audit in `plans/113-parse-performance.md`.

use crate::Configuration;

pub struct Family {
    pub name: &'static str,
    pub ids: &'static str,
    pub configuration: Configuration,
    /// Smallest `n`; the scaling runner doubles it.
    pub start: usize,
    /// Largest `n`, for families whose memory grows faster than their input.
    pub limit: usize,
    pub generate: fn(usize) -> String,
}

const UNLIMITED: usize = usize::MAX;

fn repeat(text: &str, n: usize) -> String {
    text.repeat(n)
}

macro_rules! family {
    ($name:expr, $ids:expr, $configuration:ident, $start:expr, $limit:expr, $generate:expr) => {
        Family {
            name: $name,
            ids: $ids,
            configuration: Configuration::$configuration,
            start: $start,
            limit: $limit,
            generate: $generate,
        }
    };
}

pub const FAMILIES: &[Family] = &[
    family!(
        "control-prose",
        "control",
        CommonMark,
        1024,
        UNLIMITED,
        |n| repeat("lorem ipsum dolor sit amet ", n)
    ),
    family!(
        "control-plain-run",
        "control",
        CommonMark,
        1024,
        UNLIMITED,
        |n| repeat("xxxx", n)
    ),
    family!("lines", "A2a", CommonMark, 1024, UNLIMITED, |n| repeat(
        "a\n", n
    )),
    family!("blank-lines", "A2a", CommonMark, 1024, UNLIMITED, |n| {
        repeat("\n", n)
    }),
    family!("paragraphs", "A2a", CommonMark, 512, UNLIMITED, |n| repeat(
        "a\nb\n\n", n
    )),
    family!("headings", "A2a", CommonMark, 512, UNLIMITED, |n| repeat(
        "# a\n", n
    )),
    family!(
        "hard-breaks",
        "A2a",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("a  \nb\n", n)
    ),
    family!("table-rows", "A2a", Gfm, 512, UNLIMITED, |n| format!(
        "| a |\n| - |\n{}",
        repeat("| b |\n", n)
    )),
    family!(
        "strong-run",
        "A2b",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}a{}", repeat("*", n), repeat("*", n))
    ),
    family!("emphasis-pairs", "A1b", CommonMark, 512, UNLIMITED, |n| {
        repeat("a*", n)
    }),
    family!("strong-pairs", "A1b", CommonMark, 512, UNLIMITED, |n| {
        repeat("a**b", n)
    }),
    family!(
        "ambiguous-attention",
        "A1a",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("a**b{}", repeat("c*", n))
    ),
    family!("closers-only", "A1a", CommonMark, 512, UNLIMITED, |n| {
        repeat("a_ ", n)
    }),
    family!("openers-only", "A1a", CommonMark, 512, UNLIMITED, |n| {
        repeat("_a ", n)
    }),
    family!(
        "mixed-markers",
        "A1a",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}{}", repeat("*a ", n), repeat("a_ ", n))
    ),
    family!(
        "mismatched-markers",
        "A1a",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("*a_ ", n)
    ),
    family!(
        "nested-strong-emphasis",
        "A1a A2b",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}b{}", repeat("*a **a ", n), repeat(" a** a*", n))
    ),
    family!(
        "attention-in-nested-images",
        "A1c",
        CommonMark,
        64,
        4096,
        |n| format!(
            "{}{}{}",
            repeat("![", n),
            repeat("*a ", n),
            repeat("](b)", n)
        )
    ),
    family!(
        "deep-list",
        "B5 B6",
        CommonMark,
        256,
        UNLIMITED,
        |n| format!("{}a", repeat("- ", n))
    ),
    family!(
        "deep-list-indented-line",
        "B3",
        CommonMark,
        256,
        UNLIMITED,
        |n| format!("{}a\n{}b", repeat("- ", n), repeat("  ", n))
    ),
    family!(
        "deep-list-blank-lines",
        "B4",
        CommonMark,
        256,
        UNLIMITED,
        |n| format!("{}x{}", repeat("- ", n), repeat("\n", n))
    ),
    family!("staircase-list", "B6", CommonMark, 64, UNLIMITED, |n| (0
        ..n)
        .map(|depth| format!("{}* a\n", repeat("  ", depth)))
        .collect()),
    family!(
        "deep-block-quote",
        "A5",
        CommonMark,
        256,
        UNLIMITED,
        |n| format!("{}a", repeat("> ", n))
    ),
    family!("rising-backticks", "A6", CommonMark, 64, UNLIMITED, |n| (1
        ..=n)
        .map(|length| format!("{} ", repeat("`", length)))
        .collect()),
    family!("escaped-backticks", "A6", CommonMark, 512, UNLIMITED, |n| {
        repeat("\\`` ", n)
    }),
    family!(
        "unclosed-comments",
        "B11",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("</{}", repeat("<!--", n))
    ),
    family!(
        "unclosed-instructions",
        "B11",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("a {}", repeat("<?", n))
    ),
    family!("unclosed-cdata", "B11", CommonMark, 512, UNLIMITED, |n| {
        format!("a {}", repeat("<![CDATA[", n))
    }),
    family!(
        "unclosed-declarations",
        "B11",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("a {}", repeat("<!A", n))
    ),
    family!(
        "image-link-pattern",
        "B7",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("![[]()", n)
    ),
    family!(
        "link-openers-then-links",
        "B7",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}{}", repeat("[", n), repeat("[a](b)", n))
    ),
    family!(
        "nested-brackets",
        "B8",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}a{}", repeat("[", n), repeat("]", n))
    ),
    family!(
        "nested-brackets-defined",
        "B8",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}a{}\n\n[a]: b", repeat("[", n), repeat("]", n))
    ),
    family!(
        "nested-image-labels",
        "B8 B13 B14 B15",
        CommonMark,
        256,
        UNLIMITED,
        |n| format!("{}{}", repeat("![x", n), repeat("](b)", n))
    ),
    family!(
        "link-closers-only",
        "control",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("a]", n)
    ),
    family!(
        "link-openers-only",
        "control",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("[a", n)
    ),
    family!(
        "unclosed-links",
        "control",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("[a](b", n)
    ),
    family!(
        "unclosed-links-angle",
        "control",
        CommonMark,
        512,
        UNLIMITED,
        |n| repeat("[a](<b", n)
    ),
    family!("www-underscores", "B9", Gfm, 512, UNLIMITED, |n| repeat(
        "_www.", n
    )),
    family!(
        "autolink-trail-punctuation",
        "B10",
        Gfm,
        512,
        UNLIMITED,
        |n| format!("http://a.b/{}x", repeat("!", n))
    ),
    family!(
        "autolink-trail-underscores",
        "B10",
        Gfm,
        512,
        UNLIMITED,
        |n| format!("www.a{}b", repeat("._", n))
    ),
    family!(
        "definitions-then-misses",
        "A4",
        CommonMark,
        256,
        UNLIMITED,
        |n| {
            let definitions: String = (0..n)
                .map(|index| format!("[d{:06}]: u\n", index))
                .collect();
            format!("{}\n{}", definitions, repeat("[q000000] ", n))
        }
    ),
    family!(
        "definitions-then-hits",
        "A4",
        CommonMark,
        256,
        UNLIMITED,
        |n| {
            let definitions: String = (0..n)
                .map(|index| format!("[d{}]: u{}\n[D{}]: v{}\n", index, index, index, index))
                .collect();
            let hits: String = (0..n).rev().map(|index| format!("[d{}] ", index)).collect();
            format!("{}\n{}", definitions, hits)
        }
    ),
    family!("footnote-calls", "A4", Gfm, 256, UNLIMITED, |n| {
        let calls: String = (0..n)
            .rev()
            .map(|index| format!("[^f{}] ", index))
            .collect();
        let definitions: String = (0..n)
            .map(|index| format!("[^f{}]: x{}\n[^F{}]: y{}\n", index, index, index, index))
            .collect();
        format!("{}[^f0]\n\n{}", calls, definitions)
    }),
    family!(
        "repeated-definitions",
        "A4",
        CommonMark,
        512,
        UNLIMITED,
        |n| format!("{}\n{}", repeat("[a]: u\n", n), repeat("[a] ", n))
    ),
    family!(
        "table-escaped-pipes",
        "A3",
        Gfm,
        512,
        UNLIMITED,
        |n| format!("| a |\n| - |\n| `{}` |\n", repeat("\\|", n))
    ),
    family!(
        "mdx-esm-blank-lines",
        "A7",
        MdxAware,
        256,
        UNLIMITED,
        |n| format!("export const a = [\n{}]", repeat("b,\n\n", n))
    ),
    family!(
        "mdx-expression-string-braces",
        "A7",
        MdxAware,
        256,
        UNLIMITED,
        |n| format!("{{\"{}\"}}", repeat("}", n))
    ),
    family!(
        "mdx-flow-expression-retries",
        "B12",
        Mdx,
        256,
        UNLIMITED,
        |n| format!("{}{}x", repeat("{\n", n), repeat("}", n))
    ),
    family!(
        "mdx-flow-expression-quote-retries",
        "B12",
        Mdx,
        256,
        UNLIMITED,
        |n| format!("{}> {}x", repeat("> {\n", n), repeat("}", n))
    ),
    family!(
        "mdx-flow-expression-tag-tails",
        "B12",
        Mdx,
        256,
        UNLIMITED,
        |n| format!("{}{}", repeat("{\n", n), repeat("}<a/>x", n))
    ),
    family!(
        "mdx-jsx-flow-retries",
        "B12",
        Mdx,
        256,
        UNLIMITED,
        |n| format!("{}{}x</a>", repeat("<a b={\n", n), repeat("}>", n))
    ),
];
