//! Prints the baseline HTML for each argument, with `\n` in arguments read as
//! a line ending, and the current HTML when it differs. Use it to take
//! expected values for regression tests and to inspect differences.
//!
//! Usage: `render <configuration> [--mdast] <input>...`. With `--mdast`, prints
//! the tree with positions in place of HTML.

use markdown_performance::{baseline, current, Configuration};
use std::env;

fn main() {
    let mut arguments = env::args().skip(1);
    let configuration = Configuration::from_name(&arguments.next().expect("configuration"))
        .expect("unknown configuration");
    let baseline_options = baseline::options(configuration);
    let current_options = current::options(configuration);
    let mut is_mdast = false;
    for argument in arguments {
        if argument == "--mdast" {
            is_mdast = true;
            continue;
        }
        let input = argument.replace("\\n", "\n");
        let (expected, actual) = if is_mdast {
            (
                baseline::mdast_text(&input, &baseline_options.parse),
                current::mdast_text(&input, &current_options.parse),
            )
        } else {
            (
                baseline::html_text(&input, &baseline_options),
                current::html_text(&input, &current_options),
            )
        };
        println!("{:?} => {:?}", input, expected);
        if actual != expected {
            println!("  current differs: {:?}", actual);
        }
    }
}
