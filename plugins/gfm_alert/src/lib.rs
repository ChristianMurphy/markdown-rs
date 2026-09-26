//! GitHub alerts, as a plugin: top-level block quotes that start with a
//! marker such as `[!NOTE]` become alert boxes.
//!
//! ```markdown
//! > [!NOTE]
//! > Useful information.
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use markdown::{mdast, unist::Position};
use markdown_processor::{hast, mdast_util_to_hast::wrap, Plugin, Processor};

/// Alert kinds GitHub supports, with their titles.
const KINDS: [(&str, &str); 5] = [
    ("note", "Note"),
    ("tip", "Tip"),
    ("important", "Important"),
    ("warning", "Warning"),
    ("caution", "Caution"),
];

/// Name of the custom node this plugin makes.
const NAME: &str = "gfmAlert";

/// The plugin.
pub struct GfmAlert;

impl Plugin for GfmAlert {
    fn attach(self, processor: &mut Processor) {
        processor.add_mdast_transform(|tree| {
            // GitHub: alerts cannot be nested within other elements.
            if let mdast::Node::Root(root) = tree {
                for child in &mut root.children {
                    if let Some(alert) = to_alert(child) {
                        *child = alert;
                    }
                }
            }
            Ok(())
        });

        processor.add_hast_handler(NAME, |node, children| {
            // Other plugins can make these nodes too.
            let kind = node.attributes.get("kind").map_or("", String::as_str);
            let (kind, title) = KINDS
                .iter()
                .copied()
                .find(|(name, _)| *name == kind)
                .unwrap_or(KINDS[0]);
            let mut content = vec![element(
                "p",
                vec!["markdown-alert-title".into()],
                vec![hast::Node::Text(hast::Text {
                    value: title.into(),
                    position: None,
                })],
                None,
            )];
            content.extend(children);
            vec![element(
                "div",
                vec!["markdown-alert".into(), format!("markdown-alert-{}", kind)],
                wrap(content, true),
                node.position.clone(),
            )]
        });
    }
}

/// Turn a block quote that starts with an alert marker into an alert.
fn to_alert(node: &mut mdast::Node) -> Option<mdast::Node> {
    let quote = match node {
        mdast::Node::Blockquote(quote) => quote,
        _ => return None,
    };
    let paragraph = match quote.children.first_mut() {
        Some(mdast::Node::Paragraph(paragraph)) => paragraph,
        _ => return None,
    };
    let text = match paragraph.children.first_mut() {
        Some(mdast::Node::Text(text)) => text,
        _ => return None,
    };
    let rest = text.value.strip_prefix("[!")?;
    let end = rest.find(']')?;
    let kind = rest[..end].to_ascii_lowercase();
    let after = &rest[end + 1..];

    if !KINDS.iter().any(|(name, _)| *name == kind) {
        return None;
    }

    if let Some(content) = ["\r\n", "\n", "\r"]
        .iter()
        .find_map(|eol| after.strip_prefix(eol))
    {
        if content.is_empty() {
            paragraph.children.remove(0);
        } else {
            text.value = content.into();
            // Its start moved past the marker.
            text.position = None;
        }
    } else if after.is_empty() {
        match paragraph.children.get(1) {
            None => {}
            Some(mdast::Node::Break(_)) => {
                paragraph.children.remove(1);
            }
            Some(_) => return None,
        }
        paragraph.children.remove(0);
    } else {
        return None;
    }

    if paragraph.children.is_empty() {
        quote.children.remove(0);
    } else {
        paragraph.position = None;
    }

    Some(mdast::Node::Custom(mdast::Custom {
        name: NAME.into(),
        attributes: vec![("kind".into(), kind)].into_iter().collect(),
        children: core::mem::take(&mut quote.children),
        position: quote.position.clone(),
        ..mdast::Custom::default()
    }))
}

/// Create an element with classes.
fn element(
    tag_name: &str,
    class_names: Vec<String>,
    children: Vec<hast::Node>,
    position: Option<Position>,
) -> hast::Node {
    hast::Node::Element(hast::Element {
        tag_name: tag_name.into(),
        properties: vec![(
            "className".into(),
            hast::PropertyValue::SpaceSeparated(class_names),
        )],
        children,
        position,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use markdown::{to_mdast, ParseOptions};

    /// First child of `node`.
    fn head(node: &mdast::Node) -> &mdast::Node {
        &node.children().unwrap()[0]
    }

    #[test]
    fn removes_marker_text_before_inline_content() {
        let mut tree = to_mdast("> [!NOTE]\n> *a*", &ParseOptions::default()).unwrap();
        let alert = to_alert(&mut tree.children_mut().unwrap()[0]).unwrap();
        let paragraph = head(&alert);

        assert!(
            matches!(head(paragraph), mdast::Node::Emphasis(_)),
            "should drop the emptied text node"
        );
        assert_eq!(
            paragraph.position(),
            None,
            "should clear the paragraph position, whose start moved"
        );
        assert!(alert.position().is_some(), "should keep the alert position");
    }

    #[test]
    fn clears_position_of_trimmed_text() {
        let mut tree = to_mdast("> [!NOTE]\n> a", &ParseOptions::default()).unwrap();
        let alert = to_alert(&mut tree.children_mut().unwrap()[0]).unwrap();

        assert_eq!(head(head(&alert)).to_string(), "a");
        assert_eq!(
            head(head(&alert)).position(),
            None,
            "should clear the text position, whose start moved"
        );
    }
}
