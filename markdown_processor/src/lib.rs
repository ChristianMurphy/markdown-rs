//! Prototype of a unified-style processor for markdown-rs.
//!
//! Pipeline: markdown → mdast → hast → HTML.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::pedantic)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]

extern crate alloc;

pub mod hast;
pub mod hast_util_to_html;
pub mod mdast_util_to_hast;
pub mod processor;
pub mod visit;

pub use processor::{Plugin, Processor};

/// GFM, as a plugin: turns on the GFM constructs (without the tag filter).
pub struct Gfm;

impl Plugin for Gfm {
    fn attach(self, processor: &mut Processor) {
        let constructs = &mut processor.parse.constructs;
        constructs.gfm_autolink_literal = true;
        constructs.gfm_footnote_definition = true;
        constructs.gfm_label_start_footnote = true;
        constructs.gfm_strikethrough = true;
        constructs.gfm_table = true;
        constructs.gfm_task_list_item = true;
    }
}
