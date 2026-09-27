//! Compares `current` against `baseline` on every corpus document, on small
//! instances of every family, and on any directories passed as arguments.
//!
//! `--random N` adds N seeded random strings over a punctuation-heavy
//! alphabet, to exercise delimiter matching; `--seed S` changes the seed.
//!
//! Compares the mdast with positions, the HTML, and error messages, for
//! every configuration. A panic in both implementations counts as equal.
//! Exits non-zero when any output differs.

use markdown_performance::{baseline, corpus, current, families::FAMILIES, Configuration};
use std::{
    env, fs,
    panic::{self, AssertUnwindSafe},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    sync::Mutex,
    thread,
};

type Outcome = Result<Result<String, String>, ()>;

fn outcome(work: impl FnOnce() -> Result<String, String>) -> Outcome {
    panic::catch_unwind(AssertUnwindSafe(work)).map_err(|_| ())
}

fn inputs() -> Vec<(String, String)> {
    let mut inputs: Vec<(String, String)> = corpus::load()
        .into_iter()
        .map(|document| (document.name, document.value))
        .collect();

    for family in FAMILIES {
        for n in [0, 1, 2, 3, 7, 16, 100] {
            inputs.push((format!("{}@{}", family.name, n), (family.generate)(n)));
        }
    }

    let mut arguments = env::args().skip(1);
    let mut random = 0;
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--random" => random = arguments.next().unwrap().parse().unwrap(),
            "--seed" => seed = arguments.next().unwrap().parse().unwrap(),
            _ => collect_directory(Path::new(&argument), &mut inputs),
        }
    }
    for index in 0..random {
        inputs.push((
            format!("random-{}-{}", seed, index),
            random_input(&mut seed),
        ));
    }

    inputs
}

/// A short string of pieces that start or end constructs, seeded for
/// reproducibility: single bytes, whole tokens, or a few tokens repeated.
fn random_input(seed: &mut u64) -> String {
    const BYTES: &[u8] = b"*_~a .[]()!\\`<>#-|{}\n\r\t:1+)$&;?";
    const TOKENS: &[&str] = &[
        "*",
        "**",
        "***",
        "****",
        "_",
        "__",
        "___",
        "~",
        "~~",
        "~~~",
        "a",
        "b",
        " ",
        "  ",
        ".",
        "[",
        "]",
        "](c)",
        "![",
        "[^a]",
        "`",
        "``",
        "\\",
        "<",
        ">",
        "\n",
        "\n\n",
        "- ",
        "> ",
        "# ",
        "|",
        "{",
        "}",
        "http://a.b",
        "www.",
        "&amp;",
        "!",
        "\t",
        "\r",
        "\r\n",
        "[a]: u\n",
        "[^a]: b\n",
        "]: ",
        "[A]",
        "[^A]",
        "[^a]: ",
        "1. ",
        "2) ",
        "+ ",
        "    ",
        "\"",
        "export const x = ",
        "import a from \"b\"\n",
        "<A>",
        "</A>",
        "$",
        "$$",
        "$$$",
        "```",
        "<!--",
        "-->",
        "<?",
        "?>",
        "<![CDATA[",
        "]]>",
        "<!A",
        "_www.",
        "&",
        ";",
        "{\n",
        "}\n",
        "}x",
        "<a/>",
    ];
    let mut next = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    };
    let length = 1 + (next() % 32) as usize;
    match next() % 3 {
        0 => (0..length)
            .map(|_| char::from(BYTES[(next() % BYTES.len() as u64) as usize]))
            .collect(),
        1 => (0..length)
            .map(|_| TOKENS[(next() % TOKENS.len() as u64) as usize])
            .collect(),
        _ => {
            let few: Vec<&str> = (0..2 + next() % 4)
                .map(|_| TOKENS[(next() % TOKENS.len() as u64) as usize])
                .collect();
            (0..length)
                .map(|_| few[(next() % few.len() as u64) as usize])
                .collect()
        }
    }
}

fn collect_directory(directory: &Path, inputs: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(directory).expect("reading an input directory") {
        let path = entry.expect("reading a directory entry").path();
        if path.is_dir() {
            collect_directory(&path, inputs);
        } else if let Ok(value) = fs::read_to_string(&path) {
            inputs.push((path.display().to_string(), value));
        }
    }
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    let inputs = inputs();
    let next = AtomicUsize::new(0);
    let compared = AtomicUsize::new(0);
    let differences = Mutex::new(vec![]);
    let workers = thread::available_parallelism().map_or(4, |count| count.get());

    thread::scope(|scope| {
        for _ in 0..workers {
            thread::Builder::new()
                .stack_size(1 << 28)
                .spawn_scoped(scope, || loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((name, value)) = inputs.get(index) else {
                        break;
                    };
                    for configuration in Configuration::ALL {
                        for difference in compare(value, configuration) {
                            differences.lock().unwrap().push(format!(
                                "{} [{}] input {:.200?}\n  {}",
                                name,
                                configuration.name(),
                                value,
                                difference
                            ));
                        }
                        compared.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .expect("spawning a worker");
        }
    });

    let differences = differences.into_inner().unwrap();
    println!(
        "inputs: {}, comparisons: {}, differences: {}",
        inputs.len(),
        compared.load(Ordering::Relaxed),
        differences.len()
    );
    for difference in differences.iter().take(50) {
        println!("{}", difference);
    }
    if !differences.is_empty() {
        std::process::exit(1);
    }
}

fn compare(value: &str, configuration: Configuration) -> Vec<String> {
    let mut differences = vec![];
    let baseline_mdast =
        outcome(|| baseline::mdast_text(value, &baseline::parse_options(configuration)));
    let current_mdast =
        outcome(|| current::mdast_text(value, &current::parse_options(configuration)));
    if baseline_mdast != current_mdast {
        differences.push(describe("mdast", &baseline_mdast, &current_mdast));
    }
    let baseline_html = outcome(|| baseline::html_text(value, &baseline::options(configuration)));
    let current_html = outcome(|| current::html_text(value, &current::options(configuration)));
    if baseline_html != current_html {
        differences.push(describe("html", &baseline_html, &current_html));
    }
    differences
}

fn describe(output: &str, baseline: &Outcome, current: &Outcome) -> String {
    let show = |outcome: &Outcome| match outcome {
        Ok(Ok(text)) => format!("ok {:.160}", text),
        Ok(Err(message)) => format!("error {}", message),
        Err(()) => "panic".into(),
    };
    format!(
        "{}: baseline {} | current {}",
        output,
        show(baseline),
        show(current)
    )
}
