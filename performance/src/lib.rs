//! Measurement harness for markdown-rs parse speed.
//!
//! Every measurement runs two implementations side by side: `current`, the
//! crate in this repository, and `baseline`, the published `markdown` 1.0.0.

pub mod allocator;
pub mod corpus;
pub mod families;

use std::thread;

/// Option sets that every measurement covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Configuration {
    CommonMark,
    Gfm,
    Mdx,
    GfmMdx,
    Everything,
    Reduced,
    MdxAware,
}

impl Configuration {
    pub const ALL: [Configuration; 7] = [
        Configuration::CommonMark,
        Configuration::Gfm,
        Configuration::Mdx,
        Configuration::GfmMdx,
        Configuration::Everything,
        Configuration::Reduced,
        Configuration::MdxAware,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Configuration::CommonMark => "commonmark",
            Configuration::Gfm => "gfm",
            Configuration::Mdx => "mdx",
            Configuration::GfmMdx => "gfm-mdx",
            Configuration::Everything => "everything",
            Configuration::Reduced => "reduced",
            Configuration::MdxAware => "mdx-aware",
        }
    }

    pub fn from_name(name: &str) -> Option<Configuration> {
        Configuration::ALL
            .into_iter()
            .find(|configuration| configuration.name() == name)
    }
}

/// Which parser a measurement runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Implementation {
    Baseline,
    Current,
}

impl Implementation {
    pub const ALL: [Implementation; 2] = [Implementation::Baseline, Implementation::Current];

    pub fn name(self) -> &'static str {
        match self {
            Implementation::Baseline => "baseline",
            Implementation::Current => "current",
        }
    }

    pub fn from_name(name: &str) -> Option<Implementation> {
        Implementation::ALL
            .into_iter()
            .find(|implementation| implementation.name() == name)
    }
}

macro_rules! implementation {
    ($module:ident, $crate_name:ident) => {
        pub mod $module {
            use super::Configuration;
            pub use $crate_name::{
                mdast::Node, to_html_with_options, to_mdast, CompileOptions, Constructs, MdxSignal,
                Options, ParseOptions,
            };

            /// A stand-in for a JavaScript parser: brackets must match, and
            /// open brackets or `"` strings at the end ask for more.
            pub fn fake_javascript(value: &str) -> MdxSignal {
                let mut open = vec![];
                let mut in_string = false;
                for (index, byte) in value.bytes().enumerate() {
                    if in_string {
                        in_string = byte != b'"';
                        continue;
                    }
                    match byte {
                        b'"' => in_string = true,
                        b'(' => open.push(b')'),
                        b'[' => open.push(b']'),
                        b'{' => open.push(b'}'),
                        b')' | b']' | b'}' if open.pop() != Some(byte) => {
                            return MdxSignal::Error(
                                "Unexpected closing bracket".into(),
                                index,
                                Box::new("fake".into()),
                                Box::new("unexpected-close".into()),
                            );
                        }
                        _ => {}
                    }
                }
                if in_string || !open.is_empty() {
                    MdxSignal::Eof(
                        "Unexpected end of file".into(),
                        Box::new("fake".into()),
                        Box::new("unexpected-eof".into()),
                    )
                } else {
                    MdxSignal::Ok
                }
            }

            pub fn parse_options(configuration: Configuration) -> ParseOptions {
                match configuration {
                    Configuration::CommonMark => ParseOptions::default(),
                    Configuration::Gfm => ParseOptions::gfm(),
                    Configuration::Mdx => ParseOptions::mdx(),
                    Configuration::GfmMdx => ParseOptions {
                        constructs: Constructs {
                            gfm_autolink_literal: true,
                            gfm_footnote_definition: true,
                            gfm_label_start_footnote: true,
                            gfm_strikethrough: true,
                            gfm_table: true,
                            gfm_task_list_item: true,
                            ..Constructs::mdx()
                        },
                        ..ParseOptions::mdx()
                    },
                    Configuration::Everything => ParseOptions {
                        constructs: Constructs {
                            frontmatter: true,
                            math_flow: true,
                            math_text: true,
                            ..Constructs::gfm()
                        },
                        gfm_strikethrough_single_tilde: false,
                        math_text_single_dollar: false,
                        ..ParseOptions::gfm()
                    },
                    Configuration::MdxAware => ParseOptions {
                        mdx_expression_parse: Some(Box::new(|value, _kind| fake_javascript(value))),
                        mdx_esm_parse: Some(Box::new(fake_javascript)),
                        ..ParseOptions::mdx()
                    },
                    Configuration::Reduced => ParseOptions {
                        constructs: Constructs {
                            character_escape: false,
                            character_reference: false,
                            code_indented: false,
                            html_text: false,
                            thematic_break: false,
                            math_text: true,
                            ..Constructs::default()
                        },
                        ..ParseOptions::default()
                    },
                }
            }

            pub fn options(configuration: Configuration) -> Options {
                Options {
                    parse: parse_options(configuration),
                    compile: CompileOptions {
                        allow_dangerous_html: true,
                        allow_dangerous_protocol: true,
                        ..CompileOptions::gfm()
                    },
                }
            }

            /// The tree printed with `Debug`, which includes every position.
            pub fn mdast_text(value: &str, options: &ParseOptions) -> Result<String, String> {
                to_mdast(value, options)
                    .map(|tree| format!("{:?}", tree))
                    .map_err(|message| message.to_string())
            }

            pub fn html_text(value: &str, options: &Options) -> Result<String, String> {
                to_html_with_options(value, options).map_err(|message| message.to_string())
            }
        }
    };
}

implementation!(current, markdown);
implementation!(baseline, markdown_baseline);

/// Runs `work` on a thread with a stack large enough for trees as deep as
/// the pathological inputs, whose derived `Drop` recurses once per level.
pub fn with_large_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    thread::scope(|scope| {
        thread::Builder::new()
            .stack_size(1 << 30)
            .spawn_scoped(scope, work)
            .expect("spawning a measurement thread")
            .join()
            .expect("measurement thread panicked")
    })
}
