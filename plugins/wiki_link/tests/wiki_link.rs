use markdown::{
    extension::{assert_linear_with, trace, TraceEvent},
    to_html_with_options, Constructs, Options, ParseOptions,
};
use markdown_processor::{assert_same_html, Gfm, Processor};
use pretty_assertions::assert_eq;
use wiki_link::{parse_options, WikiLinks};

/// HTML through `Processor`, and through `to_html`, which must be the same.
fn process(input: &str) -> String {
    let mut processor = Processor::new()
        .plugin(Gfm)
        .plugin(WikiLinks::new("/wiki/"));
    assert_same_html(&mut processor, &[input]);
    processor.process(input).unwrap()
}

fn direct(input: &str) -> String {
    to_html_with_options(
        input,
        &Options {
            parse: ParseOptions {
                constructs: Constructs::gfm(),
                ..parse_options("/wiki/")
            },
            ..Options::gfm()
        },
    )
    .unwrap()
}

#[test]
fn links_to_page() {
    assert_eq!(
        process("See [[Home]]."),
        "<p>See <a href=\"/wiki/Home\">Home</a>.</p>"
    );
}

#[test]
fn shows_alias() {
    assert_eq!(
        process("[[Home|start here]]"),
        "<p><a href=\"/wiki/Home\">start here</a></p>"
    );
}

#[test]
fn normalizes_target_and_keeps_fragment() {
    assert_eq!(
        process("[[ My Page#Intro ]]"),
        "<p><a href=\"/wiki/My%20Page#Intro\"> My Page#Intro </a></p>",
        "should trim and normalize the target for the href, and show it as written"
    );
}

#[test]
fn treats_empty_alias_as_no_alias() {
    assert_eq!(process("[[Page|]]"), process("[[Page]]"));
}

#[test]
fn keeps_non_links_as_text() {
    for input in ["[[]]", "[[ ]]", "[[a", "[[a]b]]", "[[a|b|c]]", "[[a[b]]"] {
        assert_eq!(
            process(input),
            format!("<p>{}</p>", input),
            "should not link {:?}",
            input
        );
    }
}

#[test]
fn stays_on_one_line() {
    assert_eq!(process("[[a\nb]]"), "<p>[[a\nb]]</p>");
}

#[test]
fn works_inside_containers_and_phrasing() {
    assert_eq!(
        process("> a *[[b]]*\n> c"),
        "<blockquote>\n<p>a <em><a href=\"/wiki/b\">b</a></em>\nc</p>\n</blockquote>"
    );
}

#[test]
fn falls_back_to_commonmark_links() {
    assert_eq!(
        process("[[a](b)]"),
        "<p>[<a href=\"b\">a</a>]</p>",
        "should leave a bracketed link alone"
    );
}

#[test]
fn wins_over_reference_definitions() {
    assert_eq!(
        process("[[Home]]\n\n[Home]: /x"),
        "<p><a href=\"/wiki/Home\">Home</a></p>",
        "should link the wiki page, not the definition"
    );
}

#[test]
fn serializer_checks_plugin_urls() {
    let processor = Processor::new().plugin(WikiLinks::new(""));
    let expected = "<p><a href=\"\">javascript:alert(1)</a></p>";

    assert_eq!(
        processor.process("[[javascript:alert(1)]]").unwrap(),
        expected,
        "should drop dangerous protocols from plugin-made links"
    );
    assert_eq!(
        to_html_with_options(
            "[[javascript:alert(1)]]",
            &Options {
                parse: parse_options(""),
                ..Options::default()
            }
        )
        .unwrap(),
        expected,
        "should drop them in `to_html` too"
    );
}

#[test]
fn treats_blank_alias_as_no_alias() {
    assert_eq!(process("[[Page|  ]]"), process("[[Page]]"));
}

#[test]
fn turns_wiki_links_inside_links_into_text() {
    let process = |input: &str| {
        Processor::new()
            .plugin(Gfm)
            .plugin(WikiLinks::new("/wiki/"))
            .process(input)
            .unwrap()
    };
    assert_eq!(
        direct("[x [[a|b]] y](z)"),
        "<p><a href=\"z\">x <a href=\"/wiki/a\">b</a> y</a></p>",
        "should nest links in `to_html`, which has no tree to check"
    );
    assert_eq!(
        process("[x [[a|b]] y](z)"),
        "<p><a href=\"z\">x b y</a></p>",
        "should not nest links"
    );
    assert_eq!(
        process("[x [[a]] y][z]\n\n[z]: /w"),
        "<p><a href=\"/w\">x a y</a></p>",
        "should not nest links in references"
    );
}

#[test]
fn parses_the_alias_as_markdown() {
    let processor =
        markdown_processor::Processor::new().plugin(wiki_link::WikiLinks::new("/wiki/"));

    assert_eq!(
        processor.process("[[a|*b* c]]").unwrap(),
        "<p><a href=\"/wiki/a\"><em>b</em> c</a></p>",
        "should parse markdown in an alias"
    );
    assert_eq!(
        processor.process("[[a| ]]").unwrap(),
        "<p><a href=\"/wiki/a\">a</a></p>",
        "should use the target for a whitespace-only alias"
    );
}

#[test]
fn keeps_the_whitespace_of_an_alias() {
    assert_eq!(process("[[a| b ]]"), "<p><a href=\"/wiki/a\"> b </a></p>");
}

#[test]
fn keeps_links_that_start_inside_an_opener() {
    assert_eq!(process("[[[a]]"), "<p>[<a href=\"/wiki/a\">a</a></p>");
}

#[test]
fn is_linear() {
    // Unbounded, but a match stops at a `[` or a line ending.
    assert_linear_with(&parse_options("/wiki/"), &["[[a", "[[a|", "[[a|b]"]);
}

#[test]
fn tries_each_bracket_of_an_opener() {
    let starts = trace("[[[a]]", &parse_options("/wiki/"))
        .into_iter()
        .filter_map(|event| match event {
            TraceEvent::Start { offset, .. } => Some(offset),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        starts,
        vec![0, 1],
        "should try the second `[` after the first fails, which `Reach::Pair` would skip"
    );
}
