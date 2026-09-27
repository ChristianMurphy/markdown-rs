//! HTML syntax tree: [hast][].
//!
//! Port of `hast.rs` from `mdxjs-rs`, without the MDX nodes and with `Raw`.
//!
//! [hast]: https://github.com/syntax-tree/hast

use alloc::{borrow::Cow, string::String, vec::Vec};
use markdown::unist::Position;

/// Nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Root.
    Root(Root),
    /// Element.
    Element(Element),
    /// Document type.
    Doctype(Doctype),
    /// Comment.
    Comment(Comment),
    /// Text.
    Text(Text),
    /// Raw HTML, passed through by the serializer when dangerous HTML is
    /// allowed, and encoded as text otherwise.
    Raw(Raw),
}

impl Node {
    /// Get children of a hast node.
    #[must_use]
    pub fn children(&self) -> Option<&Vec<Node>> {
        match self {
            Node::Root(x) => Some(&x.children),
            Node::Element(x) => Some(&x.children),
            _ => None,
        }
    }

    /// Get children of a hast node, mutably.
    pub fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        match self {
            Node::Root(x) => Some(&mut x.children),
            Node::Element(x) => Some(&mut x.children),
            _ => None,
        }
    }

    /// Get the position of a hast node.
    pub fn position(&self) -> Option<&Position> {
        match self {
            Node::Root(x) => x.position.as_ref(),
            Node::Element(x) => x.position.as_ref(),
            Node::Doctype(x) => x.position.as_ref(),
            Node::Comment(x) => x.position.as_ref(),
            Node::Text(x) => x.position.as_ref(),
            Node::Raw(x) => x.position.as_ref(),
        }
    }
}

/// Document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    /// Content model.
    pub children: Vec<Node>,
    /// Positional info.
    pub position: Option<Position>,
}

/// Element.
///
/// ```html
/// > | <a href="b">c</a>
///     ^^^^^^^^^^^^^^^^^
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    /// Tag name.
    pub tag_name: Cow<'static, str>,
    /// Properties, keyed by hast property name (such as `className`).
    pub properties: Vec<(Cow<'static, str>, PropertyValue)>,
    /// Children.
    pub children: Vec<Node>,
    /// Positional info.
    pub position: Option<Position>,
}

/// Property value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyValue {
    /// A boolean.
    Boolean(bool),
    /// A string.
    String(String),
    /// A comma-separated list of strings.
    CommaSeparated(Vec<String>),
    /// A space-separated list of strings.
    SpaceSeparated(Vec<String>),
}

/// Document type.
///
/// ```html
/// > | <!doctype html>
///     ^^^^^^^^^^^^^^^
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doctype {
    /// Positional info.
    pub position: Option<Position>,
}

/// Comment.
///
/// ```html
/// > | <!-- a -->
///     ^^^^^^^^^^
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// Content model.
    pub value: String,
    /// Positional info.
    pub position: Option<Position>,
}

/// Text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    /// Content model.
    pub value: Cow<'static, str>,
    /// Positional info.
    pub position: Option<Position>,
}

/// Raw HTML.
///
/// ```html
/// > | <div>
///     ^^^^^
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raw {
    /// Content model.
    pub value: String,
    /// Positional info.
    pub position: Option<Position>,
}
