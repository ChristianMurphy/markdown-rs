use directive::{parse_options, Directives};
use markdown::extension::{assert_linear_with, trace, TraceEvent};
use markdown_processor::{assert_same_html, Processor};
use pretty_assertions::assert_eq;

/// HTML through `Processor`, and through `to_html`, which must be the same.
fn html(input: &str) -> String {
    let mut processor = Processor::new().plugin(Directives);
    assert_same_html(&mut processor, &[input]);
    processor.process(input).unwrap()
}

#[test]
fn renders_with_the_default_handlers() {
    assert_eq!(
        html("A :abbr[HTML]{title=\"HyperText Markup Language\"} page."),
        "<p>A <span title=\"HyperText Markup Language\" class=\"abbr\">HTML</span> page.</p>",
        "should render text directives as `span`"
    );
    assert_eq!(
        html("::youtube[Video]{#dQw4w9WgXcQ}"),
        "<div id=\"dQw4w9WgXcQ\" class=\"youtube\">Video</div>",
        "should render leaf directives as `div`"
    );
    assert_eq!(
        html(":::note[Heads *up*]{.warn .big}\nNested *markdown*.\n:::"),
        "<div class=\"note warn big\">\n<p>Heads <em>up</em></p>\n<p>Nested <em>markdown</em>.</p>\n</div>",
        "should render container directives as `div`, with the label first"
    );
}

#[test]
fn drops_unsafe_attributes() {
    assert_eq!(
        html(":a{onclick=x href=y data-b=c}"),
        "<p><span data-b=\"c\" class=\"a\"></span></p>",
        "should keep ids, classes, titles, languages, directions, and data attributes only"
    );
}

#[test]
fn keeps_later_lines_of_indented_containers() {
    assert_eq!(
        html("  :::a\n  b\n  c\n  :::"),
        "<div class=\"a\">\n<p>b\nc</p>\n</div>",
        "should strip the indent of the fence from each line"
    );
}

#[test]
fn keeps_bodies_whose_blocks_start_where_a_line_ends() {
    assert_eq!(
        html("  :::note\n\na\n  b\nc\n  :::"),
        "<div class=\"note\">\n<p>a\nb\nc</p>\n</div>",
        "should keep lines after a blank line in an indented container"
    );
    assert_eq!(
        html("  :::note\n\n  a\nb\n  *c*\n  :::"),
        "<div class=\"note\">\n<p>a\nb\n<em>c</em></p>\n</div>",
        "should keep markdown on later lines"
    );
    assert_eq!(
        html(":::x\na\nb\nc\n:::\n:::y\nd\ne\n:::"),
        "<div class=\"x\">\n<p>a\nb\nc</p>\n</div>\n<div class=\"y\">\n<p>d\ne</p>\n</div>",
        "should parse two containers with multi-line paragraphs"
    );
}

#[test]
fn keeps_lists_apart_around_directives() {
    assert_eq!(
        html("- a\n::x\n- b"),
        "<ul>\n<li>a</li>\n</ul>\n<div class=\"x\"></div>\n<ul>\n<li>b</li>\n</ul>",
        "should end a list at a leaf directive, as at a thematic break"
    );
}

#[test]
fn is_linear() {
    assert_linear_with(
        &parse_options(),
        &[":a[", ":a{", ":a{b=\"", "::a[", ":::a{"],
    );
}

#[test]
fn undoes_a_label_without_its_end() {
    let attempts = trace(":x[ :y[z]", &parse_options())
        .into_iter()
        .filter_map(|event| match event {
            TraceEvent::AttemptEnd { offset, ok } => Some((offset, ok)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attempts,
        vec![(2, true), (2, false), (6, true), (9, true)],
        "should keep the name of `:x`, undo its label, then keep the name and label of `:y`"
    );
}
