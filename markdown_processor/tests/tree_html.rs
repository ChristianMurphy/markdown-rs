use markdown::{to_html, to_html_with_options, to_mdast, CompileOptions, ParseOptions};
use markdown_processor::{
    hast,
    hast_util_to_html::{hast_util_to_html, Options},
    mdast_util_to_hast::mdast_util_to_hast,
};
use pretty_assertions::assert_eq;
use std::borrow::Cow;

#[test]
fn matches_to_html_for_commonmark_basics() {
    for input in [
        "# a *b*",
        "a\nb",
        "* a\n* b",
        "1. a\n\n2. b",
        "> a",
        "![a](b \"c\")",
        "[a](<b c> 'd')",
        "a  \nb",
        "`a`",
        "```js\na\n```",
        "```\n```",
        "***",
        "<div>",
        "[a](b?c=1&d=2)",
        "![a](b&c)",
        "[a](javascript:alert(1))",
        "![a](data:x)",
        "a\u{0}b",
    ] {
        let mdast = to_mdast(input, &ParseOptions::default()).unwrap();
        assert_eq!(
            hast_util_to_html(&mdast_util_to_hast(&mdast), &Options::default()),
            to_html(input),
            "should match `to_html` for {:?}",
            input
        );
    }
}

#[test]
fn matches_to_html_when_dangerous_output_is_allowed() {
    let options = markdown::Options {
        compile: CompileOptions {
            allow_dangerous_html: true,
            allow_dangerous_protocol: true,
            ..CompileOptions::default()
        },
        ..markdown::Options::default()
    };

    for input in [
        "<div>\u{0}",
        "[a](javascript:alert(1))",
        "![a](data:x)",
        "a <b>c</b>",
    ] {
        let mdast = to_mdast(input, &ParseOptions::default()).unwrap();
        assert_eq!(
            hast_util_to_html(
                &mdast_util_to_hast(&mdast),
                &Options {
                    allow_dangerous_html: true,
                    allow_dangerous_protocol: true,
                }
            ),
            to_html_with_options(input, &options).unwrap(),
            "should match `to_html_with_options` for {:?}",
            input
        );
    }
}

#[test]
fn borrows_constant_strings() {
    let mdast = to_mdast("a\n\n[b](c)", &ParseOptions::default()).unwrap();
    let hast = mdast_util_to_hast(&mdast);
    let children = match &hast {
        hast::Node::Root(root) => &root.children,
        _ => panic!("expected root"),
    };
    let (first, between, second) = match &children[..] {
        [hast::Node::Element(first), hast::Node::Text(between), hast::Node::Element(second)] => {
            (first, between, second)
        }
        _ => panic!("expected two paragraphs with a line ending between them"),
    };
    let link = match &second.children[..] {
        [hast::Node::Element(link)] => link,
        _ => panic!("expected a link"),
    };

    assert!(
        matches!(first.tag_name, Cow::Borrowed("p")),
        "should borrow tag names"
    );
    assert!(
        matches!(link.properties[0].0, Cow::Borrowed("href")),
        "should borrow property names"
    );
    assert!(
        matches!(between.value, Cow::Borrowed("\n")),
        "should borrow line endings between blocks"
    );
}
