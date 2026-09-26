//! Sketch for comparison, not the chosen design (see D5 in
//! `plans/32-plugin-prototypes.md`): the alerts plugin with one trait per
//! stage instead of one `Plugin::attach`.
//!
//! The package exports two pieces, and each user wires up both.

use markdown::{mdast, message::Message, to_mdast, ParseOptions};
use markdown_processor::{
    hast,
    hast_util_to_html::{hast_util_to_html, Options},
    mdast_util_to_hast::{mdast_util_to_hast_with_handlers, Handlers},
};

trait MdastTransform {
    fn transform(&self, tree: &mut mdast::Node) -> Result<(), Message>;
}

#[derive(Default)]
struct PerStageProcessor {
    parse: ParseOptions,
    transforms: Vec<Box<dyn MdastTransform>>,
    handlers: Handlers,
}

impl PerStageProcessor {
    fn mdast(mut self, transform: impl MdastTransform + 'static) -> Self {
        self.transforms.push(Box::new(transform));
        self
    }

    fn hast_handler(
        mut self,
        name: &str,
        handler: impl Fn(&mdast::Custom, Vec<hast::Node>) -> Vec<hast::Node> + 'static,
    ) -> Self {
        self.handlers.insert(name.into(), Box::new(handler));
        self
    }

    fn process(&self, value: &str) -> Result<String, Message> {
        let mut tree = to_mdast(value, &self.parse)?;
        for transform in &self.transforms {
            transform.transform(&mut tree)?;
        }
        let hast = mdast_util_to_hast_with_handlers(&tree, &self.handlers);
        Ok(hast_util_to_html(&hast, &Options::default()))
    }
}

// What an alerts package would export in this design.

/// Piece 1: turn `> [!NOTE]` block quotes into `alert` nodes (NOTE only here).
struct AlertTransform;

impl MdastTransform for AlertTransform {
    fn transform(&self, tree: &mut mdast::Node) -> Result<(), Message> {
        for child in tree.children_mut().into_iter().flatten() {
            if let mdast::Node::Blockquote(quote) = child {
                if let Some(mdast::Node::Paragraph(paragraph)) = quote.children.first_mut() {
                    if let Some(mdast::Node::Text(text)) = paragraph.children.first_mut() {
                        if let Some(rest) = text.value.strip_prefix("[!NOTE]\n") {
                            text.value = rest.into();
                            *child = mdast::Node::Custom(mdast::Custom {
                                name: "alert".into(),
                                children: std::mem::take(&mut quote.children),
                                ..mdast::Custom::default()
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Piece 2: render `alert` nodes.
fn alert_handler(_node: &mdast::Custom, children: Vec<hast::Node>) -> Vec<hast::Node> {
    vec![hast::Node::Element(hast::Element {
        tag_name: "div".into(),
        properties: vec![(
            "className".into(),
            hast::PropertyValue::SpaceSeparated(vec!["markdown-alert".into()]),
        )],
        children,
        position: None,
    })]
}

fn main() {
    // Users must add both pieces; forgetting the handler renders a bare `div`.
    let processor = PerStageProcessor::default()
        .mdast(AlertTransform)
        .hast_handler("alert", alert_handler);

    println!("{}", processor.process("> [!NOTE]\n> a").unwrap());
}
