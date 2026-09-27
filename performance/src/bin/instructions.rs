//! Parses every document in one size bin, for callgrind instruction counts.
//!
//! Usage: `instructions <bin> <configuration> <implementation> [--html]`.
//! Run under callgrind with `--toggle-collect='*parse_all*'` so corpus
//! loading is not counted.

use markdown_performance::{
    baseline,
    corpus::{self, SizeBin},
    current, Configuration, Implementation,
};
use std::{env, hint::black_box};

#[inline(never)]
fn parse_all(
    values: &[&str],
    configuration: Configuration,
    implementation: Implementation,
    is_html: bool,
) {
    match (implementation, is_html) {
        (Implementation::Baseline, false) => {
            let options = baseline::parse_options(configuration);
            for value in values {
                black_box(baseline::to_mdast(value, &options).ok());
            }
        }
        (Implementation::Current, false) => {
            let options = current::parse_options(configuration);
            for value in values {
                black_box(current::to_mdast(value, &options).ok());
            }
        }
        (Implementation::Baseline, true) => {
            let options = baseline::options(configuration);
            for value in values {
                black_box(baseline::to_html_with_options(value, &options).ok());
            }
        }
        (Implementation::Current, true) => {
            let options = current::options(configuration);
            for value in values {
                black_box(current::to_html_with_options(value, &options).ok());
            }
        }
    }
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let bin = SizeBin::from_name(&arguments[0]).expect("bin: tiny, small, medium, or large");
    let configuration = Configuration::from_name(&arguments[1]).expect("unknown configuration");
    let implementation = Implementation::from_name(&arguments[2]).expect("baseline or current");
    let is_html = arguments.iter().any(|argument| argument == "--html");

    let documents = corpus::load();
    let values: Vec<&str> = corpus::in_bin(&documents, bin)
        .into_iter()
        .map(|document| document.value.as_str())
        .collect();
    let bytes: usize = values.iter().map(|value| value.len()).sum();

    parse_all(&values, configuration, implementation, is_html);
    eprintln!("documents: {}, bytes: {}", values.len(), bytes);
}
