use gfm_alert::GfmAlert;
use markdown::mdast;
use markdown_processor::{hast, Gfm, Processor};
use pretty_assertions::assert_eq;
use std::{cell::Cell, rc::Rc};

fn process(input: &str) -> String {
    Processor::new()
        .plugin(Gfm)
        .plugin(GfmAlert)
        .process(input)
        .unwrap()
}

#[test]
fn turns_marked_block_quote_into_alert() {
    assert_eq!(
        process("> [!NOTE]\n> Useful information."),
        "<div class=\"markdown-alert markdown-alert-note\">\n<p class=\"markdown-alert-title\">Note</p>\n<p>Useful information.</p>\n</div>",
        "should match GitHub's alert structure"
    );
}

#[test]
fn supports_each_kind() {
    for (marker, class, title) in [
        ("NOTE", "note", "Note"),
        ("TIP", "tip", "Tip"),
        ("IMPORTANT", "important", "Important"),
        ("WARNING", "warning", "Warning"),
        ("CAUTION", "caution", "Caution"),
    ] {
        assert_eq!(
            process(&format!("> [!{}]\n> a", marker)),
            format!(
                "<div class=\"markdown-alert markdown-alert-{}\">\n<p class=\"markdown-alert-title\">{}</p>\n<p>a</p>\n</div>",
                class, title
            ),
            "should support `{}`",
            marker
        );
    }
}

#[test]
fn matches_marker_case_insensitively() {
    assert_eq!(
        process("> [!note]\n> a"),
        process("> [!NOTE]\n> a"),
        "should treat `[!note]` like `[!NOTE]`"
    );
}

#[test]
fn allows_hard_break_after_marker() {
    assert_eq!(
        process("> [!TIP]  \n> a"),
        "<div class=\"markdown-alert markdown-alert-tip\">\n<p class=\"markdown-alert-title\">Tip</p>\n<p>a</p>\n</div>",
        "should drop the marker and its hard break"
    );
}

#[test]
fn requires_marker_alone_on_its_line() {
    assert_eq!(
        process("> [!NOTE] a"),
        "<blockquote>\n<p>[!NOTE] a</p>\n</blockquote>",
        "should keep a block quote when text follows the marker"
    );
}

#[test]
fn renders_empty_alert_with_title() {
    assert_eq!(
        process("> [!WARNING]"),
        "<div class=\"markdown-alert markdown-alert-warning\">\n<p class=\"markdown-alert-title\">Warning</p>\n</div>",
        "should render the title when there is no content"
    );
}

#[test]
fn ignores_nested_block_quotes() {
    assert_eq!(
        process("- > [!NOTE]\n  > a"),
        "<ul>\n<li>\n<blockquote>\n<p>[!NOTE]\na</p>\n</blockquote>\n</li>\n</ul>",
        "should not make alerts inside other elements"
    );
    assert_eq!(
        process("> > [!NOTE]\n> > a"),
        "<blockquote>\n<blockquote>\n<p>[!NOTE]\na</p>\n</blockquote>\n</blockquote>",
        "should not make alerts inside block quotes"
    );
}

#[test]
fn ignores_unknown_kinds() {
    assert_eq!(
        process("> [!FOO]\n> a"),
        "<blockquote>\n<p>[!FOO]\na</p>\n</blockquote>"
    );
}

#[test]
fn keeps_block_content() {
    assert_eq!(
        process("> [!CAUTION]\n> a\n>\n> - b"),
        "<div class=\"markdown-alert markdown-alert-caution\">\n<p class=\"markdown-alert-title\">Caution</p>\n<p>a</p>\n<ul>\n<li>b</li>\n</ul>\n</div>",
        "should keep paragraphs and lists inside the alert"
    );
}

#[test]
fn supports_any_line_ending_after_marker() {
    for input in ["> [!NOTE]\r\n> a", "> [!NOTE]\r> a"] {
        assert_eq!(
            process(input),
            process("> [!NOTE]\n> a"),
            "should support the line ending in {:?}",
            input
        );
    }
}

#[test]
fn drops_marker_line_before_inline_content() {
    assert_eq!(
        process("> [!NOTE]\n> *a*"),
        "<div class=\"markdown-alert markdown-alert-note\">\n<p class=\"markdown-alert-title\">Note</p>\n<p><em>a</em></p>\n</div>",
        "should keep inline content that starts the next line"
    );
}

#[test]
fn keeps_alert_position_in_hast() {
    let seen = Rc::new(Cell::new(false));
    let seen_in_plugin = Rc::clone(&seen);
    let processor = Processor::new()
        .plugin(GfmAlert)
        .plugin(move |processor: &mut Processor| {
            processor.add_hast_transform(move |tree| {
                if let Some(hast::Node::Element(div)) = tree.children().and_then(|x| x.first()) {
                    seen_in_plugin.set(div.position.is_some());
                }
                Ok(())
            });
        });

    processor.process("> [!NOTE]\n> a").unwrap();
    assert!(
        seen.get(),
        "should give the alert `div` the block quote position"
    );
}

#[test]
fn treats_unknown_kind_from_other_plugins_as_note() {
    let processor = Processor::new()
        .plugin(GfmAlert)
        .plugin(|processor: &mut Processor| {
            processor.add_mdast_transform(|tree| {
                if let Some(children) = tree.children_mut() {
                    children.push(mdast::Node::Custom(mdast::Custom {
                        name: "gfmAlert".into(),
                        attributes: vec![("kind".into(), "é".into())].into_iter().collect(),
                        ..mdast::Custom::default()
                    }));
                }
                Ok(())
            });
        });

    assert_eq!(
        processor.process("").unwrap(),
        "<div class=\"markdown-alert markdown-alert-note\">\n<p class=\"markdown-alert-title\">Note</p>\n</div>",
        "should not panic on, and should default, an unknown kind"
    );
}
