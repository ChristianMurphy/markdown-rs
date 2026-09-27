//! Wall-clock throughput per size bin and configuration, for both
//! implementations. Filter with criterion's name filter, for example
//! `cargo bench -- tiny/gfm`.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use markdown_performance::{
    baseline,
    corpus::{self, SizeBin},
    current, Configuration,
};
use std::time::Duration;

fn corpus_bins(criterion: &mut Criterion) {
    let documents = corpus::load();

    for bin in SizeBin::ALL {
        let values: Vec<&str> = corpus::in_bin(&documents, bin)
            .into_iter()
            .map(|document| document.value.as_str())
            .collect();
        let bytes: usize = values.iter().map(|value| value.len()).sum();

        for configuration in Configuration::ALL {
            let mut group =
                criterion.benchmark_group(format!("{}/{}", bin.name(), configuration.name()));
            group.throughput(Throughput::Bytes(bytes as u64));
            group.measurement_time(Duration::from_secs(4));

            let baseline_options = baseline::parse_options(configuration);
            group.bench_function("baseline", |bencher| {
                bencher.iter(|| {
                    for value in &values {
                        black_box(baseline::to_mdast(value, &baseline_options).ok());
                    }
                })
            });

            let current_options = current::parse_options(configuration);
            group.bench_function("current", |bencher| {
                bencher.iter(|| {
                    for value in &values {
                        black_box(current::to_mdast(value, &current_options).ok());
                    }
                })
            });

            group.finish();
        }
    }
}

criterion_group!(benches, corpus_bins);
criterion_main!(benches);
