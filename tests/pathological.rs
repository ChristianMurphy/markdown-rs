use markdown::{to_mdast, ParseOptions};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

/// Whether a test is timing parses, as parallel large parses slow each other.
static TIMING: AtomicBool = AtomicBool::new(false);

/// Holds `TIMING` until dropped, also when a parse panics.
struct Timing;

impl Timing {
    fn start() -> Timing {
        while TIMING
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            thread::sleep(Duration::from_millis(1));
        }
        Timing
    }
}

impl Drop for Timing {
    fn drop(&mut self) {
        TIMING.store(false, Ordering::Release);
    }
}

/// Fastest of a few parses, on a large stack for deep trees.
fn fastest_parse(value: String, options: fn() -> ParseOptions) -> Duration {
    let _timing = Timing::start();
    thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || {
            let options = options();
            (0..5)
                .map(|_| {
                    let start = Instant::now();
                    to_mdast(&value, &options).unwrap();
                    start.elapsed()
                })
                .min()
                .unwrap()
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Asserts that 8 times the repetitions parse within 2.5 times the byte growth.
fn assert_near_linear(
    name: &str,
    generate: fn(usize) -> String,
    n: usize,
    options: fn() -> ParseOptions,
) {
    let small_value = generate(n);
    let large_value = generate(n * 8);
    let growth = large_value.len() as f64 / small_value.len() as f64;
    let measure = || {
        let small = fastest_parse(small_value.clone(), options);
        let large = fastest_parse(large_value.clone(), options);
        (large.as_secs_f64() / small.as_secs_f64(), small, large)
    };
    let mut result = measure();
    if result.0 >= growth * 2.5 {
        result = measure();
    }
    let (ratio, small, large) = result;
    assert!(
        ratio < growth * 2.5,
        "{}: {:.1}x the input took {:.1}x as long ({:?} vs {:?})",
        name,
        growth,
        ratio,
        large,
        small
    );
}

#[test]
fn pathological_edit_map() {
    let commonmark = ParseOptions::default;
    let gfm = ParseOptions::gfm;

    assert_near_linear("lines", |n| "a\n".repeat(n), 2_500, commonmark);
    assert_near_linear("blank lines", |n| "\n".repeat(n), 5_000, commonmark);
    assert_near_linear(
        "table rows",
        |n| format!("| a |\n| - |\n{}", "| b |\n".repeat(n)),
        1_000,
        gfm,
    );
}
