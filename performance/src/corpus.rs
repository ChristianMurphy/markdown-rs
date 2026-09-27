//! Natural documents, split into size bins.
//!
//! Sources:
//! - the 652 examples in the CommonMark 0.31.2 spec, `commonmark-data.txt`;
//! - the spec itself and `readme.md`;
//! - a snapshot of local Markdown and MDX files in `MARKDOWN_PERFORMANCE_CORPUS`,
//!   by default `.agent-tmp/perf-simd/corpus` at the repository root.

use std::{env, fs, path::PathBuf};

pub struct Document {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeBin {
    Tiny,
    Small,
    Medium,
    Large,
}

impl SizeBin {
    pub const ALL: [SizeBin; 4] = [
        SizeBin::Tiny,
        SizeBin::Small,
        SizeBin::Medium,
        SizeBin::Large,
    ];

    pub fn of(length: usize) -> SizeBin {
        match length {
            0..=255 => SizeBin::Tiny,
            256..=4095 => SizeBin::Small,
            4096..=65535 => SizeBin::Medium,
            _ => SizeBin::Large,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SizeBin::Tiny => "tiny",
            SizeBin::Small => "small",
            SizeBin::Medium => "medium",
            SizeBin::Large => "large",
        }
    }

    pub fn from_name(name: &str) -> Option<SizeBin> {
        SizeBin::ALL.into_iter().find(|bin| bin.name() == name)
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn snapshot_directory() -> PathBuf {
    env::var_os("MARKDOWN_PERFORMANCE_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root().join(".agent-tmp/perf-simd/corpus"))
}

/// Every document, sorted by name so runs are reproducible.
pub fn load() -> Vec<Document> {
    let root = repository_root();
    let spec = fs::read_to_string(root.join("commonmark-data.txt"))
        .expect("reading commonmark-data.txt at the repository root");
    let mut documents: Vec<Document> = spec_examples(&spec)
        .into_iter()
        .enumerate()
        .map(|(index, value)| Document {
            name: format!("spec-example-{:03}", index + 1),
            value,
        })
        .collect();

    documents.push(Document {
        name: "readme.md".into(),
        value: fs::read_to_string(root.join("readme.md")).expect("reading readme.md"),
    });
    documents.push(Document {
        name: "commonmark-spec.md".into(),
        value: spec,
    });

    let directory = snapshot_directory();
    let entries = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("reading corpus {}: {}", directory.display(), error));
    for entry in entries {
        let path = entry.expect("reading a corpus entry").path();
        if let Ok(value) = fs::read_to_string(&path) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            documents.push(Document { name, value });
        }
    }

    documents.sort_by(|left, right| left.name.cmp(&right.name));
    documents
}

pub fn in_bin(documents: &[Document], bin: SizeBin) -> Vec<&Document> {
    documents
        .iter()
        .filter(|document| SizeBin::of(document.value.len()) == bin)
        .collect()
}

/// The Markdown input of each example in the CommonMark spec text.
pub fn spec_examples(spec: &str) -> Vec<String> {
    let fence = "`".repeat(32);
    let opening = format!("{} example", fence);
    let mut examples = vec![];
    let mut lines = spec.lines();

    while let Some(line) = lines.next() {
        if line != opening {
            continue;
        }
        let mut markdown = String::new();
        for line in lines.by_ref() {
            if line == "." {
                break;
            }
            markdown.push_str(line);
            markdown.push('\n');
        }
        for line in lines.by_ref() {
            if line == fence {
                break;
            }
        }
        examples.push(markdown.replace('→', "\t"));
    }

    examples
}
