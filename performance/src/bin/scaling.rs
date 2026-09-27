//! Times each family at doubling sizes and reports the local growth exponent:
//! about 1 for linear work, 2 for quadratic.
//!
//! Usage: `scaling [family-substrings,comma-separated] [--html] [--only baseline|current]
//! [--budget-ms N] [--max-bytes N]`. Output is tab-separated.

use markdown_performance::{
    allocator::{self, PeakAllocator},
    baseline, current,
    families::{Family, FAMILIES},
    with_large_stack, Implementation,
};
use std::{env, time::Duration, time::Instant};

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator;

struct Settings {
    filter: Option<String>,
    is_html: bool,
    implementations: Vec<Implementation>,
    budget: Duration,
    max_bytes: usize,
}

fn settings() -> Settings {
    let mut settings = Settings {
        filter: None,
        is_html: false,
        implementations: Implementation::ALL.to_vec(),
        budget: Duration::from_millis(2000),
        max_bytes: 8 << 20,
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--html" => settings.is_html = true,
            "--only" => {
                let name = arguments.next().expect("--only needs a value");
                settings.implementations =
                    vec![Implementation::from_name(&name).expect("unknown implementation")];
            }
            "--budget-ms" => {
                settings.budget = Duration::from_millis(arguments.next().unwrap().parse().unwrap());
            }
            "--max-bytes" => settings.max_bytes = arguments.next().unwrap().parse().unwrap(),
            _ => settings.filter = Some(argument),
        }
    }
    settings
}

struct Sample {
    parse: Duration,
    drop: Duration,
    peak: usize,
}

/// Parses once and drops the result, timing each part separately.
fn run_once(family: &Family, implementation: Implementation, is_html: bool, value: &str) -> Sample {
    let live = allocator::live();
    allocator::reset_peak();
    let configuration = family.configuration;
    let start = Instant::now();
    let (parse, drop) = match (implementation, is_html) {
        (Implementation::Baseline, false) => {
            let options = baseline::parse_options(configuration);
            let tree = baseline::to_mdast(value, &options);
            let parse = start.elapsed();
            let drop_start = Instant::now();
            drop(tree);
            (parse, drop_start.elapsed())
        }
        (Implementation::Current, false) => {
            let options = current::parse_options(configuration);
            let tree = current::to_mdast(value, &options);
            let parse = start.elapsed();
            let drop_start = Instant::now();
            drop(tree);
            (parse, drop_start.elapsed())
        }
        (Implementation::Baseline, true) => {
            let html = baseline::to_html_with_options(value, &baseline::options(configuration));
            (start.elapsed(), drop_duration(html))
        }
        (Implementation::Current, true) => {
            let html = current::to_html_with_options(value, &current::options(configuration));
            (start.elapsed(), drop_duration(html))
        }
    };
    Sample {
        parse,
        drop,
        peak: allocator::peak_above(live),
    }
}

fn drop_duration<T>(value: T) -> Duration {
    let start = Instant::now();
    drop(value);
    start.elapsed()
}

/// Repeats short runs and keeps the fastest, to damp noise at small sizes.
fn measure(family: &Family, implementation: Implementation, is_html: bool, value: &str) -> Sample {
    let mut best = run_once(family, implementation, is_html, value);
    let mut total = best.parse;
    let mut runs = 1;
    while runs < 5 && total < Duration::from_millis(200) {
        let sample = run_once(family, implementation, is_html, value);
        total += sample.parse;
        runs += 1;
        if sample.parse < best.parse {
            best = sample;
        }
    }
    best
}

fn main() {
    let settings = settings();
    println!("family\tids\tconfiguration\timplementation\tn\tbytes\tparse_ms\tdrop_ms\tns_per_byte\texponent\tpeak_mib");

    for family in FAMILIES {
        if let Some(filter) = &settings.filter {
            if !filter.split(',').any(|part| family.name.contains(part)) {
                continue;
            }
        }
        for &implementation in &settings.implementations {
            let mut previous: Option<(usize, Duration)> = None;
            let mut n = family.start;
            while n <= family.limit {
                let value = (family.generate)(n);
                let bytes = value.len();
                let sample =
                    with_large_stack(|| measure(family, implementation, settings.is_html, &value));
                let exponent = previous.map_or(String::new(), |(previous_bytes, previous_time)| {
                    let time_ratio = sample.parse.as_secs_f64() / previous_time.as_secs_f64();
                    let size_ratio = bytes as f64 / previous_bytes as f64;
                    format!("{:.2}", time_ratio.ln() / size_ratio.ln())
                });
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.1}\t{}\t{:.1}",
                    family.name,
                    family.ids,
                    family.configuration.name(),
                    implementation.name(),
                    n,
                    bytes,
                    sample.parse.as_secs_f64() * 1e3,
                    sample.drop.as_secs_f64() * 1e3,
                    sample.parse.as_secs_f64() * 1e9 / bytes.max(1) as f64,
                    exponent,
                    sample.peak as f64 / (1 << 20) as f64,
                );
                if sample.parse > settings.budget || bytes > settings.max_bytes {
                    break;
                }
                previous = Some((bytes, sample.parse));
                n *= 2;
            }
        }
    }
}
