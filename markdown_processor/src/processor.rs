//! The processor and the plugin trait.

use crate::{
    hast,
    hast_util_to_html::{self, hast_util_to_html},
    mdast_util_to_hast::{mdast_util_to_hast_with_handlers, Handlers},
};
use alloc::{boxed::Box, string::String, vec::Vec};
use markdown::{extension::TextConstruct, mdast, message::Message, to_mdast, ParseOptions};

/// A plugin: attaches parser config, transforms, and handlers to a processor.
pub trait Plugin {
    /// Configure `processor`.
    fn attach(self, processor: &mut Processor);
}

/// Any `FnOnce(&mut Processor)` is a plugin, like an inline unified attacher.
impl<F: FnOnce(&mut Processor)> Plugin for F {
    fn attach(self, processor: &mut Processor) {
        self(processor);
    }
}

type MdastTransform = Box<dyn Fn(&mut mdast::Node) -> Result<(), Message>>;
type HastTransform = Box<dyn Fn(&mut hast::Node) -> Result<(), Message>>;

/// Markdown → mdast → hast → HTML, with plugins.
pub struct Processor {
    /// Parse options; plugins such as [`Gfm`][crate::Gfm] change them.
    pub parse: ParseOptions,
    /// Serialize options: whether raw HTML and any URL protocol are allowed.
    pub compile: hast_util_to_html::Options,
    mdast_transforms: Vec<MdastTransform>,
    hast_transforms: Vec<HastTransform>,
    handlers: Handlers,
}

impl Processor {
    /// Create a `CommonMark` processor without plugins.
    pub fn new() -> Self {
        Processor {
            parse: ParseOptions::default(),
            compile: hast_util_to_html::Options::default(),
            mdast_transforms: Vec::new(),
            hast_transforms: Vec::new(),
            handlers: Handlers::new(),
        }
    }

    /// Attach a plugin.
    #[must_use]
    pub fn plugin(mut self, plugin: impl Plugin) -> Self {
        plugin.attach(&mut self);
        self
    }

    /// Add a construct to text, tried before the built-in constructs at its
    /// markers, in the order added.
    pub fn add_syntax(&mut self, construct: impl TextConstruct + 'static) {
        self.parse.text_constructs.push(Box::new(construct));
    }

    /// Add a transform that runs on mdast, in the order added.
    pub fn add_mdast_transform(
        &mut self,
        transform: impl Fn(&mut mdast::Node) -> Result<(), Message> + 'static,
    ) {
        self.mdast_transforms.push(Box::new(transform));
    }

    /// Add a transform that runs on hast, in the order added.
    pub fn add_hast_transform(
        &mut self,
        transform: impl Fn(&mut hast::Node) -> Result<(), Message> + 'static,
    ) {
        self.hast_transforms.push(Box::new(transform));
    }

    /// Set how custom nodes named `name` turn into hast, from their converted
    /// children; a later handler for a name replaces an earlier one.
    pub fn add_hast_handler(
        &mut self,
        name: &str,
        handler: impl Fn(&mdast::Custom, Vec<hast::Node>) -> Vec<hast::Node> + 'static,
    ) {
        self.handlers.insert(name.into(), Box::new(handler));
    }

    /// Turn markdown into HTML.
    ///
    /// MDX nodes are dropped: MDX compiles to JavaScript, not HTML.
    ///
    /// ## Errors
    ///
    /// Errors when parsing fails (only with MDX) or when a transform fails.
    pub fn process(&self, value: &str) -> Result<String, Message> {
        let mut mdast = to_mdast(value, &self.parse)?;
        for transform in &self.mdast_transforms {
            transform(&mut mdast)?;
        }
        let mut hast = mdast_util_to_hast_with_handlers(&mdast, &self.handlers);
        for transform in &self.hast_transforms {
            transform(&mut hast)?;
        }
        Ok(hast_util_to_html(&hast, &self.compile))
    }
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}
