use markdown::{to_html, to_html_with_options, to_mdast, CompileOptions, ParseOptions};
use markdown_processor::{
    hast_util_to_html::{hast_util_to_html, Options},
    mdast_util_to_hast::mdast_util_to_hast,
};
use pretty_assertions::assert_eq;

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
