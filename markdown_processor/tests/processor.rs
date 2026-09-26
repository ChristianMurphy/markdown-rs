use markdown::{mdast, message::Message};
use markdown_processor::{hast, visit::visit_mut, Gfm, Plugin, Processor};
use pretty_assertions::assert_eq;
use std::{cell::Cell, rc::Rc};

/// Append `suffix` to every text node.
fn append_to_text(suffix: &'static str) -> impl Fn(&mut Processor) {
    move |processor: &mut Processor| {
        processor.add_mdast_transform(move |tree| {
            visit_mut(tree, &mut |node| {
                if let mdast::Node::Text(text) = node {
                    text.value.push_str(suffix);
                }
            });
            Ok(())
        });
    }
}

#[test]
fn processes_commonmark_without_plugins() {
    assert_eq!(
        Processor::new().process("# a *b* ~~c~~").unwrap(),
        "<h1>a <em>b</em> ~~c~~</h1>",
        "should parse CommonMark only by default"
    );
}

#[test]
fn plugin_configures_parser() {
    assert_eq!(
        Processor::new().plugin(Gfm).process("~~a~~").unwrap(),
        "<p><del>a</del></p>",
        "should let a plugin turn on GFM constructs"
    );
}

#[test]
fn struct_plugin_attaches() {
    struct Shout;

    impl Plugin for Shout {
        fn attach(self, processor: &mut Processor) {
            processor.add_mdast_transform(|tree| {
                visit_mut(tree, &mut |node| {
                    if let mdast::Node::Text(text) = node {
                        text.value = text.value.to_uppercase();
                    }
                });
                Ok(())
            });
        }
    }

    assert_eq!(
        Processor::new().plugin(Shout).process("a *b*").unwrap(),
        "<p>A <em>B</em></p>",
        "should run a transform added by a struct plugin"
    );
}

#[test]
fn closure_plugin_attaches() {
    assert_eq!(
        Processor::new()
            .plugin(append_to_text("!"))
            .process("a")
            .unwrap(),
        "<p>a!</p>",
        "should run a transform added by a closure plugin"
    );
}

#[test]
fn transforms_run_in_order_added() {
    assert_eq!(
        Processor::new()
            .plugin(append_to_text("1"))
            .plugin(append_to_text("2"))
            .process("a")
            .unwrap(),
        "<p>a12</p>",
        "should run the first-added transform first"
    );
}

#[test]
fn hast_transform_runs_before_serializing() {
    let processor = Processor::new().plugin(|processor: &mut Processor| {
        processor.add_hast_transform(|tree| {
            if let Some(children) = tree.children_mut() {
                for child in children {
                    if let hast::Node::Element(element) = child {
                        element.properties.push((
                            "className".into(),
                            hast::PropertyValue::SpaceSeparated(vec!["x".into()]),
                        ));
                    }
                }
            }
            Ok(())
        });
    });

    assert_eq!(
        processor.process("# a").unwrap(),
        "<h1 class=\"x\">a</h1>",
        "should run hast transforms on the converted tree"
    );
}

#[test]
fn transform_error_stops_processing() {
    let ran_after = Rc::new(Cell::new(false));
    let ran_after_in_plugin = Rc::clone(&ran_after);
    let processor = Processor::new().plugin(move |processor: &mut Processor| {
        processor.add_mdast_transform(|_| {
            Err(Message {
                place: None,
                reason: "no".into(),
                rule_id: Box::new("fail".into()),
                source: Box::new("test".into()),
            })
        });
        processor.add_mdast_transform(move |_| {
            ran_after_in_plugin.set(true);
            Ok(())
        });
    });

    assert_eq!(
        processor.process("a").unwrap_err().reason,
        "no",
        "should return the transform's error"
    );
    assert!(!ran_after.get(), "should not run later transforms");
}

#[test]
fn gfm_plugin_keeps_other_constructs() {
    let processor = Processor::new()
        .plugin(|processor: &mut Processor| processor.parse.constructs.frontmatter = true)
        .plugin(Gfm);

    assert_eq!(
        processor.process("---\na: b\n---\n~~c~~").unwrap(),
        "<p><del>c</del></p>",
        "should keep front matter on after turning on GFM"
    );
}

#[test]
fn raw_html_is_encoded_unless_allowed() {
    let mut processor = Processor::new();
    assert_eq!(
        processor.process("<div>").unwrap(),
        "&lt;div&gt;",
        "should encode raw HTML by default"
    );

    processor.compile.allow_dangerous_html = true;
    assert_eq!(
        processor.process("<div>").unwrap(),
        "<div>",
        "should pass raw HTML through when allowed"
    );
}
