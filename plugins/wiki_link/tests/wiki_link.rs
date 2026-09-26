use markdown_processor::{Gfm, Processor};
use pretty_assertions::assert_eq;
use wiki_link::WikiLinks;

fn process(input: &str) -> String {
    Processor::new()
        .plugin(Gfm)
        .plugin(WikiLinks::new("/wiki/"))
        .process(input)
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

    assert_eq!(
        processor.process("[[javascript:alert(1)]]").unwrap(),
        "<p><a href=\"\">javascript:alert(1)</a></p>",
        "should drop dangerous protocols from plugin-made links"
    );
}

#[test]
fn treats_blank_alias_as_no_alias() {
    assert_eq!(process("[[Page|  ]]"), process("[[Page]]"));
}

#[test]
fn turns_wiki_links_inside_links_into_text() {
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
