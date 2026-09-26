//! Measure how often the tree path (mdast → hast → HTML) differs from
//! `markdown::to_html` on the CommonMark spec examples.
//!
//! Run `cargo run -p generate` once to download the spec, then
//! `cargo run -p markdown_processor --example parity`.

use markdown::{to_html_with_options, CompileOptions, Options};
use markdown_processor::Processor;
use std::collections::BTreeMap;

const FENCE: &str = "````````````````````````````````";

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../commonmark-data.txt");
    let spec = std::fs::read_to_string(path)
        .expect("expected `commonmark-data.txt`: run `cargo run -p generate` first");
    let options = Options {
        compile: CompileOptions {
            allow_dangerous_html: true,
            allow_dangerous_protocol: true,
            ..CompileOptions::default()
        },
        ..Options::default()
    };
    let mut processor = Processor::new();
    processor.compile.allow_dangerous_html = true;
    processor.compile.allow_dangerous_protocol = true;

    // Section → (examples, event path ≠ spec, tree path ≠ event path,
    // tree path ≠ event path apart from a final line ending).
    let mut sections: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
    let mut samples = vec![];
    let mut section = String::new();
    let mut lines = spec.lines();

    while let Some(line) = lines.next() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading.into();
        }
        if line != format!("{} example", FENCE) {
            continue;
        }

        let mut input = String::new();
        let mut expected = String::new();
        let mut in_output = false;
        for line in lines.by_ref() {
            if line == FENCE {
                break;
            } else if line == "." && !in_output {
                in_output = true;
            } else {
                let target = if in_output { &mut expected } else { &mut input };
                target.push_str(&line.replace('→', "\t"));
                target.push('\n');
            }
        }

        let event = to_html_with_options(&input, &options).unwrap();
        let tree = processor.process(&input).unwrap();
        let entry = sections.entry(section.clone()).or_default();
        entry.0 += 1;
        if event != expected {
            entry.1 += 1;
        }
        if tree != event {
            entry.2 += 1;
        }
        // `to_html` ends with a line ending when the input does; the tree path never does.
        if tree != event && format!("{}\n", tree) != event {
            entry.3 += 1;
            if samples.len() < 60 {
                samples.push((section.clone(), input, event, tree));
            }
        }
    }

    let (mut total, mut event_total, mut tree_total, mut real_total) = (0, 0, 0, 0);
    println!("| Section | Examples | Event path ≠ spec | Tree ≠ event | Tree ≠ event, ignoring final line ending |");
    println!("| --- | --- | --- | --- | --- |");
    for (section, (count, event, tree, real)) in &sections {
        total += count;
        event_total += event;
        tree_total += tree;
        real_total += real;
        if *real > 0 {
            println!(
                "| {} | {} | {} | {} | {} |",
                section, count, event, tree, real
            );
        }
    }
    println!(
        "| Total | {} | {} | {} | {} |",
        total, event_total, tree_total, real_total
    );

    println!("\nSamples:");
    for (section, input, event, tree) in samples {
        println!(
            "\n[{}]\ninput: {:?}\nevent: {:?}\ntree:  {:?}",
            section, input, event, tree
        );
    }
}
