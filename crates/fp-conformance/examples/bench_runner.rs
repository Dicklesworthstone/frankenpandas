//! Quick benchmark runner that outputs JSON for comparison with pandas.
//!
//! Run: cargo run --release -p fp-conformance --example bench_runner

use std::collections::BTreeMap;
use std::time::Instant;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_io::read_csv_str;
use fp_types::Scalar;

const SIZES: &[usize] = &[10_000, 100_000];
const RUNS: usize = 20;
const WARMUP: usize = 3;

fn build_numeric_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let col_name = format!("c{}", c);
        let values: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Float64((i * (c + 1)) as f64 * 0.1))
            .collect();
        let column = Column::from_values(values).expect("column");
        columns.insert(col_name.clone(), column);
        column_order.push(col_name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_groupby_frame(n: usize, num_groups: usize) -> DataFrame {
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i % num_groups) as i64))
        .collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let key_column = Column::from_values(keys).expect("key column");
    let value_column = Column::from_values(values).expect("value column");
    let mut columns = BTreeMap::new();
    columns.insert("k".to_string(), key_column);
    columns.insert("v".to_string(), value_column);
    let column_order = vec!["k".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_series(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    Series::from_values("s", labels, values).expect("series")
}

fn build_csv_string(n: usize, cols: usize) -> String {
    let mut csv = String::with_capacity(n * cols * 15);
    let header: Vec<String> = (0..cols).map(|c| format!("c{}", c)).collect();
    csv.push_str(&header.join(","));
    csv.push('\n');
    for i in 0..n {
        let row: Vec<String> = (0..cols).map(|c| format!("{}", (i * (c + 1)) as f64 * 0.1)).collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    csv
}

struct BenchResult {
    name: String,
    size: usize,
    times_ns: Vec<u128>,
}

impl BenchResult {
    fn p50_ns(&self) -> u128 {
        let mut sorted: Vec<_> = self.times_ns.clone();
        sorted.sort();
        sorted[sorted.len() / 2]
    }

    fn p95_ns(&self) -> u128 {
        let mut sorted: Vec<_> = self.times_ns.clone();
        sorted.sort();
        sorted[(sorted.len() as f64 * 0.95) as usize]
    }

    fn p99_ns(&self) -> u128 {
        let mut sorted: Vec<_> = self.times_ns.clone();
        sorted.sort();
        sorted.get((sorted.len() as f64 * 0.99) as usize).copied().unwrap_or(sorted[sorted.len() - 1])
    }

    fn to_json(&self) -> String {
        format!(
            r#"{{"name":"{}","size":{},"runs":{},"p50_ns":{},"p95_ns":{},"p99_ns":{},"p50_ms":{:.3},"p95_ms":{:.3},"p99_ms":{:.3}}}"#,
            self.name,
            self.size,
            self.times_ns.len(),
            self.p50_ns(),
            self.p95_ns(),
            self.p99_ns(),
            self.p50_ns() as f64 / 1_000_000.0,
            self.p95_ns() as f64 / 1_000_000.0,
            self.p99_ns() as f64 / 1_000_000.0,
        )
    }
}

fn bench<F, T>(name: &str, size: usize, mut op: F) -> BenchResult
where
    F: FnMut() -> T,
{
    // Warmup
    for _ in 0..WARMUP {
        let _ = op();
    }

    // Timed runs
    let mut times = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let start = Instant::now();
        let _ = op();
        times.push(start.elapsed().as_nanos());
    }

    BenchResult {
        name: name.to_string(),
        size,
        times_ns: times,
    }
}

fn main() {
    eprintln!("Running FrankenPandas benchmarks...");
    eprintln!("Sizes: {:?}, Runs: {}", SIZES, RUNS);

    let mut results = Vec::new();

    for &n in SIZES {
        eprintln!("  Size: {}...", n);

        // IO benchmarks
        let csv_str = build_csv_string(n, 10);
        let csv_str_clone = csv_str.clone();
        results.push(bench("io/csv_read", n, || read_csv_str(&csv_str_clone).unwrap()));

        let frame = build_numeric_frame(n, 10);
        let frame_clone = frame.clone();
        results.push(bench("io/csv_write", n, || frame_clone.to_csv(',', false)));

        // DataFrame ops
        let frame = build_numeric_frame(n, 10);
        let frame_clone = frame.clone();
        results.push(bench("dataframe_ops/sort_single", n, || frame_clone.sort_values("c0", true).unwrap()));

        let frame_clone = frame.clone();
        results.push(bench("dataframe_ops/drop_duplicates", n, || {
            frame_clone.drop_duplicates(None, fp_index::DuplicateKeep::First, false).unwrap()
        }));

        let frame_clone = frame.clone();
        results.push(bench("dataframe_ops/cumsum", n, || frame_clone.cumsum().unwrap()));

        // GroupBy
        let frame = build_groupby_frame(n, 100);
        let frame_clone = frame.clone();
        results.push(bench("groupby/sum", n, || {
            frame_clone.groupby(&["k"]).unwrap().sum().unwrap()
        }));

        let frame_clone = frame.clone();
        results.push(bench("groupby/mean", n, || {
            frame_clone.groupby(&["k"]).unwrap().mean().unwrap()
        }));

        // Rolling
        let series = build_series(n);
        let series_clone = series.clone();
        results.push(bench("rolling/mean", n, || {
            series_clone.rolling(100, None).mean().unwrap()
        }));

        let series_clone = series.clone();
        results.push(bench("rolling/std", n, || {
            series_clone.rolling(100, None).std().unwrap()
        }));

        // Indexing
        let frame = build_numeric_frame(n, 10);
        let frame_clone = frame.clone();
        results.push(bench("indexing/iloc_slice", n, || {
            frame_clone.iloc_slice(Some(100), Some((n - 100) as i64)).unwrap()
        }));

        let series = build_series(n);
        let series_clone = series.clone();
        let new_labels: Vec<IndexLabel> = (0..n)
            .map(|i| IndexLabel::Int64(((i * 3) % (n * 2)) as i64))
            .collect();
        results.push(bench("indexing/reindex", n, || {
            series_clone.reindex(new_labels.clone()).unwrap()
        }));
    }

    // Output JSON
    println!("{{");
    println!("  \"environment\": {{");
    println!("    \"library\": \"frankenpandas\",");
    println!("    \"version\": \"0.1.0\",");
    println!("    \"rust_version\": \"nightly\",");
    println!("    \"timestamp\": \"{}\"", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs());
    println!("  }},");
    println!("  \"results\": [");
    for (i, r) in results.iter().enumerate() {
        let comma = if i < results.len() - 1 { "," } else { "" };
        println!("    {}{}", r.to_json(), comma);
    }
    println!("  ]");
    println!("}}");
}
