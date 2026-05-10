#![forbid(unsafe_code)]

//! Performance baseline measurements for join and filter workloads (frankenpandas-n3t).
//!
//! These tests generate representative datasets and measure p50/p95/p99 latencies
//! for key operations. Results are printed to stderr for capture and compared
//! against committed p95 budgets for regression gating.
//!
//! Run with:
//! `cargo test -p fp-conformance --test perf_baselines -- --nocapture --ignored --skip perf_run_all_baselines`
//! (Tests stay `#[ignore]` so CI can opt into them explicitly.)

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};

use fp_frame::{DataFrame, Series};
use fp_index::IndexLabel;
use fp_join::{JoinType, join_series};
use fp_types::Scalar;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
struct PerfBudget {
    p95_secs: f64,
}

#[derive(Debug, Clone, Copy)]
struct PerfSummary {
    mean: Duration,
    p50: Duration,
    p95: Duration,
    p99: Duration,
}

static PERF_BUDGETS: OnceLock<BTreeMap<String, PerfBudget>> = OnceLock::new();

fn perf_budgets_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/perf_budgets.json")
}

fn load_perf_budgets() -> &'static BTreeMap<String, PerfBudget> {
    PERF_BUDGETS.get_or_init(|| {
        serde_json::from_str(
            &std::fs::read_to_string(perf_budgets_path()).expect("read perf_budgets.json"),
        )
        .expect("parse perf_budgets.json")
    })
}

/// Run a closure `iters` times and return sorted durations.
fn bench_iters<F: FnMut()>(mut f: F, iters: usize) -> Vec<Duration> {
    let mut durations = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        f();
        durations.push(start.elapsed());
    }
    durations.sort();
    durations
}

fn summarize(sorted: &[Duration]) -> PerfSummary {
    let n = sorted.len();
    PerfSummary {
        mean: sorted.iter().sum::<Duration>() / sorted.len() as u32,
        p50: sorted[n / 2],
        p95: sorted[(n * 95) / 100],
        p99: sorted[(n * 99) / 100],
    }
}

/// Print baseline results in standard format.
fn report(name: &str, sorted: &[Duration]) -> PerfSummary {
    let summary = summarize(sorted);
    eprintln!("[PERF] {name}");
    eprintln!("  iters: {}", sorted.len());
    eprintln!("  mean:  {:.6} s", summary.mean.as_secs_f64());
    eprintln!("  p50:   {:.6} s", summary.p50.as_secs_f64());
    eprintln!("  p95:   {:.6} s", summary.p95.as_secs_f64());
    eprintln!("  p99:   {:.6} s", summary.p99.as_secs_f64());
    summary
}

fn report_and_assert(name: &str, sorted: &[Duration]) {
    let summary = report(name, sorted);
    let budgets = load_perf_budgets();
    let budget = budgets.get(name).copied().expect("missing perf budget");
    let observed_p95 = summary.p95.as_secs_f64();
    assert!(
        observed_p95 <= budget.p95_secs,
        "{name} p95 regression: observed {observed_p95:.6}s > budget {:.6}s",
        budget.p95_secs
    );
}

/// Generate a numeric Series with `n` rows and index labels from `0..n`.
fn make_numeric_series(name: &str, n: usize, offset: usize) -> Series {
    let labels: Vec<IndexLabel> = (offset..offset + n)
        .map(|i| IndexLabel::Int64(i as i64))
        .collect();
    let values: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((i as f64) * 1.5 + 0.1))
        .collect();
    Series::from_values(name.to_owned(), labels, values).unwrap()
}

/// Generate a DataFrame with `ncols` numeric columns and `nrows` rows.
fn make_numeric_dataframe(nrows: usize, ncols: usize) -> DataFrame {
    let series: Vec<Series> = (0..ncols)
        .map(|c| make_numeric_series(&format!("col_{c}"), nrows, 0))
        .collect();
    DataFrame::from_series(series).unwrap()
}

// ── Join Benchmarks ──────────────────────────────────────────────────

#[test]
#[ignore] // Run explicitly with --ignored
fn perf_join_inner_10k() {
    let n = 10_000;
    let overlap = n / 2; // 50% overlap
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);

    let durations = bench_iters(
        || {
            let _ = join_series(&left, &right, JoinType::Inner).unwrap();
        },
        50,
    );

    report_and_assert("join_inner_10k (50% overlap)", &durations);
}

#[test]
#[ignore]
fn perf_join_left_10k() {
    let n = 10_000;
    let overlap = n / 2;
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);

    let durations = bench_iters(
        || {
            let _ = join_series(&left, &right, JoinType::Left).unwrap();
        },
        50,
    );

    report_and_assert("join_left_10k (50% overlap)", &durations);
}

#[test]
#[ignore]
fn perf_join_right_10k() {
    let n = 10_000;
    let overlap = n / 2;
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);

    let durations = bench_iters(
        || {
            let _ = join_series(&left, &right, JoinType::Right).unwrap();
        },
        50,
    );

    report_and_assert("join_right_10k (50% overlap)", &durations);
}

#[test]
#[ignore]
fn perf_join_outer_10k() {
    let n = 10_000;
    let overlap = n / 2;
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);

    let durations = bench_iters(
        || {
            let _ = join_series(&left, &right, JoinType::Outer).unwrap();
        },
        50,
    );

    report_and_assert("join_outer_10k (50% overlap)", &durations);
}

#[test]
#[ignore]
fn perf_join_inner_100k() {
    let n = 100_000;
    let overlap = n / 2;
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);

    let durations = bench_iters(
        || {
            let _ = join_series(&left, &right, JoinType::Inner).unwrap();
        },
        20,
    );

    report_and_assert("join_inner_100k (50% overlap)", &durations);
}

#[test]
#[ignore]
fn perf_join_outer_100k() {
    let n = 100_000;
    let overlap = n / 2;
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);

    let durations = bench_iters(
        || {
            let _ = join_series(&left, &right, JoinType::Outer).unwrap();
        },
        20,
    );

    report_and_assert("join_outer_100k (50% overlap)", &durations);
}

// ── Filter Benchmarks ──────────────────────────────────────────────

#[test]
#[ignore]
fn perf_filter_boolean_mask_10k() {
    let n = 10_000;
    let df = make_numeric_dataframe(n, 5);
    // Create a boolean mask: keep every other row
    let mask_values: Vec<Scalar> = (0..n).map(|i| Scalar::Bool(i % 2 == 0)).collect();
    let mask_labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let mask = Series::from_values("mask".to_owned(), mask_labels, mask_values).unwrap();

    let durations = bench_iters(
        || {
            let _ = df.filter_rows(&mask).unwrap();
        },
        50,
    );

    report_and_assert(
        "filter_boolean_mask_10k (5 cols, 50% selectivity)",
        &durations,
    );
}

#[test]
#[ignore]
fn perf_filter_boolean_mask_100k() {
    let n = 100_000;
    let df = make_numeric_dataframe(n, 5);
    let mask_values: Vec<Scalar> = (0..n).map(|i| Scalar::Bool(i % 2 == 0)).collect();
    let mask_labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let mask = Series::from_values("mask".to_owned(), mask_labels, mask_values).unwrap();

    let durations = bench_iters(
        || {
            let _ = df.filter_rows(&mask).unwrap();
        },
        20,
    );

    report_and_assert(
        "filter_boolean_mask_100k (5 cols, 50% selectivity)",
        &durations,
    );
}

#[test]
#[ignore]
fn perf_filter_head_tail_100k() {
    let n = 100_000;
    let df = make_numeric_dataframe(n, 5);

    let head_durations = bench_iters(
        || {
            let _ = df.head(1000);
        },
        50,
    );
    report_and_assert("head(1000)_100k (5 cols)", &head_durations);

    let tail_durations = bench_iters(
        || {
            let _ = df.tail(1000);
        },
        50,
    );
    report_and_assert("tail(1000)_100k (5 cols)", &tail_durations);
}

// ── DataFrame Arithmetic Benchmarks ──────────────────────────────────

#[test]
#[ignore]
fn perf_df_add_scalar_100k() {
    let n = 100_000;
    let df = make_numeric_dataframe(n, 5);

    let durations = bench_iters(
        || {
            let _ = df.add_scalar(42.0).unwrap();
        },
        20,
    );

    report_and_assert("df_add_scalar_100k (5 cols)", &durations);
}

#[test]
#[ignore]
fn perf_df_add_df_aligned_100k() {
    let n = 100_000;
    let df1 = make_numeric_dataframe(n, 5);
    let df2 = make_numeric_dataframe(n, 5);

    let durations = bench_iters(
        || {
            let _ = df1.add_df(&df2).unwrap();
        },
        20,
    );

    report_and_assert("df_add_df_aligned_100k (5 cols, same index)", &durations);
}

#[test]
#[ignore]
fn perf_df_eq_scalar_100k() {
    let n = 100_000;
    let df = make_numeric_dataframe(n, 5);
    let scalar = Scalar::Float64(42.0);

    let durations = bench_iters(
        || {
            let _ = df.eq_scalar_df(&scalar).unwrap();
        },
        20,
    );

    report_and_assert("df_eq_scalar_100k (5 cols)", &durations);
}

// ── Series Alignment Benchmarks ──────────────────────────────────────

#[test]
#[ignore]
fn perf_series_add_with_alignment_10k() {
    use fp_runtime::{EvidenceLedger, RuntimePolicy};

    let n = 10_000;
    let overlap = n / 2;
    let left = make_numeric_series("left", n, 0);
    let right = make_numeric_series("right", n, n - overlap);
    let policy = RuntimePolicy::hardened(Some(1_000_000));

    let durations = bench_iters(
        || {
            let mut ledger = EvidenceLedger::new();
            let _ = left.add_with_policy(&right, &policy, &mut ledger).unwrap();
        },
        50,
    );

    report_and_assert("series_add_aligned_10k (50% overlap)", &durations);
}

// ── Summary ──────────────────────────────────────────────────────────

#[test]
#[ignore]
fn perf_run_all_baselines() {
    eprintln!("\n========== FrankenPandas Performance Baselines ==========\n");

    // Join baselines
    {
        let n = 10_000;
        let overlap = n / 2;
        let left = make_numeric_series("left", n, 0);
        let right = make_numeric_series("right", n, n - overlap);

        for (jt, name) in [
            (JoinType::Inner, "inner"),
            (JoinType::Left, "left"),
            (JoinType::Right, "right"),
            (JoinType::Outer, "outer"),
        ] {
            let d = bench_iters(
                || {
                    let _ = join_series(&left, &right, jt);
                },
                30,
            );
            report(&format!("join_{name}_10k"), &d);
        }
    }

    // Filter baselines
    {
        let n = 10_000;
        let df = make_numeric_dataframe(n, 5);
        let mask_values: Vec<Scalar> = (0..n).map(|i| Scalar::Bool(i % 2 == 0)).collect();
        let mask_labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
        let mask = Series::from_values("mask".to_owned(), mask_labels, mask_values).unwrap();

        let d = bench_iters(
            || {
                let _ = df.filter_rows(&mask);
            },
            30,
        );
        report("filter_mask_10k", &d);

        let d = bench_iters(
            || {
                let _ = df.head(100);
            },
            30,
        );
        report("head_100_10k", &d);
    }

    // Arithmetic baselines
    {
        let n = 10_000;
        let df = make_numeric_dataframe(n, 5);

        let d = bench_iters(
            || {
                let _ = df.add_scalar(42.0);
            },
            30,
        );
        report("add_scalar_10k", &d);

        let scalar = Scalar::Float64(42.0);
        let d = bench_iters(
            || {
                let _ = df.eq_scalar_df(&scalar);
            },
            30,
        );
        report("eq_scalar_10k", &d);
    }

    eprintln!("\n========================================================\n");
}
