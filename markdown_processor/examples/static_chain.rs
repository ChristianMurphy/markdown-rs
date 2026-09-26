//! Sketch for comparison, not the chosen design (see D5 in
//! `plans/32-plugin-prototypes.md`): plugins composed statically, like
//! tower's `ServiceBuilder`, so transforms need no `Box<dyn>`.
//!
//! Handlers and syntax constructs still end up boxed: the converter takes
//! `Handlers`, and `ParseOptions.text_constructs` holds `Box<dyn>`.

use markdown::{mdast, message::Message, to_mdast, ParseOptions};
use markdown_processor::{
    hast,
    hast_util_to_html::{hast_util_to_html, Options},
    mdast_util_to_hast::{mdast_util_to_hast_with_handlers, Handlers},
};

trait Plugins {
    fn parse(&self, _options: &mut ParseOptions) {}

    fn mdast(&self, _tree: &mut mdast::Node) -> Result<(), Message> {
        Ok(())
    }

    fn handlers(&self, _handlers: &mut Handlers) {}

    fn hast(&self, _tree: &mut hast::Node) -> Result<(), Message> {
        Ok(())
    }
}

struct Base;

impl Plugins for Base {}

struct Chain<A, B>(A, B);

impl<A: Plugins, B: Plugins> Plugins for Chain<A, B> {
    fn parse(&self, options: &mut ParseOptions) {
        self.0.parse(options);
        self.1.parse(options);
    }

    fn mdast(&self, tree: &mut mdast::Node) -> Result<(), Message> {
        self.0.mdast(tree)?;
        self.1.mdast(tree)
    }

    fn handlers(&self, handlers: &mut Handlers) {
        self.0.handlers(handlers);
        self.1.handlers(handlers);
    }

    fn hast(&self, tree: &mut hast::Node) -> Result<(), Message> {
        self.0.hast(tree)?;
        self.1.hast(tree)
    }
}

struct Processor<P>(P);

impl<P: Plugins> Processor<P> {
    fn with<Q: Plugins>(self, plugin: Q) -> Processor<Chain<P, Q>> {
        Processor(Chain(self.0, plugin))
    }

    fn process(&self, value: &str) -> Result<String, Message> {
        let mut options = ParseOptions::default();
        self.0.parse(&mut options);
        let mut tree = to_mdast(value, &options)?;
        self.0.mdast(&mut tree)?;
        let mut handlers = Handlers::new();
        self.0.handlers(&mut handlers);
        let mut hast = mdast_util_to_hast_with_handlers(&tree, &handlers);
        self.0.hast(&mut hast)?;
        Ok(hast_util_to_html(&hast, &Options::default()))
    }
}

/// The alerts plugin: one type, like the chosen design.
struct Alerts;

impl Plugins for Alerts {
    fn mdast(&self, tree: &mut mdast::Node) -> Result<(), Message> {
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

    fn handlers(&self, handlers: &mut Handlers) {
        handlers.insert(
            "alert".into(),
            Box::new(|_: &mdast::Custom, children: Vec<hast::Node>| {
                vec![hast::Node::Element(hast::Element {
                    tag_name: "div".into(),
                    properties: vec![],
                    children,
                    position: None,
                })]
            }),
        );
    }
}

/// GFM, through the parse hook.
struct Gfm;

impl Plugins for Gfm {
    fn parse(&self, options: &mut ParseOptions) {
        options.constructs = markdown::Constructs::gfm();
    }
}

fn type_name<T>(_: &T) -> &'static str {
    std::any::type_name::<T>()
}

fn main() {
    let processor = Processor(Base).with(Gfm).with(Alerts);

    // The whole chain is one type, which shows up in compiler errors.
    println!("{}", type_name(&processor));
    println!("{}", processor.process("> [!NOTE]\n> ~~a~~").unwrap());
}
