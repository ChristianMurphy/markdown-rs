use admonition::Admonitions;
use markdown_processor::Processor;
use pretty_assertions::assert_eq;

fn html(input: &str) -> String {
    Processor::new().plugin(Admonitions).process(input).unwrap()
}

#[test]
fn makes_admonitions() {
    assert_eq!(
        html("!!! note\n    Body *text*.\n\n    More.\n\nOutside"),
        "<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<p>Body <em>text</em>.</p>\n<p>More.</p>\n</div>\n<p>Outside</p>",
        "should use the kind as the title, and end at a line without indent"
    );
    assert_eq!(
        html("!!! warning \"Custom *title*\"\n    - a\n    - b"),
        "<div class=\"admonition warning\">\n<p class=\"admonition-title\">Custom <em>title</em></p>\n<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n</div>",
        "should parse a title as markdown, and keep one list across prefixes"
    );
    assert_eq!(
        html("!!! danger highlight \"\"\n    x"),
        "<div class=\"admonition danger highlight\">\n<p class=\"admonition-title\">Danger</p>\n<p>x</p>\n</div>",
        "should support several kinds, and an empty title"
    );
}

#[test]
fn makes_collapsible_admonitions() {
    assert_eq!(
        html("??? tip\n    hidden"),
        "<details class=\"tip\">\n<summary>Tip</summary>\n<p>hidden</p>\n</details>"
    );
    assert_eq!(
        html("???+ tip \"Open\"\n    shown"),
        "<details class=\"tip\" open=\"\">\n<summary>Open</summary>\n<p>shown</p>\n</details>",
        "should be open with `+`"
    );
}

#[test]
fn works_with_other_containers_and_flow() {
    assert_eq!(
        html("> !!! note\n>     in quote\n> after"),
        "<blockquote>\n<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<p>in quote\nafter</p>\n</div>\n</blockquote>",
        "should work in a block quote, with a lazy line"
    );
    assert_eq!(
        html("- !!! note\n      in list"),
        "<ul>\n<li>\n<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<p>in list</p>\n</div>\n</li>\n</ul>",
        "should work in a list item"
    );
    assert_eq!(
        html("!!! note\n    ```\n    code\n    ```\nout"),
        "<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<pre><code>code\n</code></pre>\n</div>\n<p>out</p>",
        "should hold fenced code"
    );
    assert_eq!(
        html("!!! note\n    Title\n    ==="),
        "<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<h1>Title</h1>\n</div>",
        "should find a setext heading across prefixes"
    );
    assert_eq!(
        html("!!! note\n    !!! tip\n        nested"),
        "<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<div class=\"admonition tip\">\n<p class=\"admonition-title\">Tip</p>\n<p>nested</p>\n</div>\n</div>",
        "should nest"
    );
}

#[test]
fn needs_a_kind_and_a_closed_title() {
    for input in ["!!!note", "!!! note \"unclosed", "!! note"] {
        assert!(
            html(input).starts_with("<p>"),
            "should not make an admonition of {:?}",
            input
        );
    }
}

#[test]
fn keeps_lists_apart_around_admonitions() {
    assert_eq!(
        html("- a\n!!! note\n    b\n- c"),
        "<ul>\n<li>a</li>\n</ul>\n<div class=\"admonition note\">\n<p class=\"admonition-title\">Note</p>\n<p>b</p>\n</div>\n<ul>\n<li>c</li>\n</ul>",
        "should end a list at an admonition"
    );
}
