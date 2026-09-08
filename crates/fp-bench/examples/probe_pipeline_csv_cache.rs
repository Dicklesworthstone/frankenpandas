//! Probe: Measure CSV parse cache hit vs miss for pipeline inputs (br-frankenpandas-qnkah).
//!
//! Evaluates:
//! 1. Warm cache hit: repeated read_csv on sales.csv.
//! 2. Content hit on different path: read_csv on sales_copy.csv (byte-identical).
//! 3. Guaranteed cache miss: round-robin read_csv over 3 distinct 14.6 MB CSVs
//!    (sales.csv, sales_perm1.csv, sales_perm2.csv) defeating the 2-entry cache.
//! 4. Pipeline whole-job under cached load vs uncached load.
//! 5. Verifies output byte-identity across all 3 variants.

use std::{
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

use fp_frame::DataFrame;
use fp_join::{JoinType, merge_dataframes_on_with};
use fp_types::Scalar;

fn run_pipeline_stage(sales_path: &Path, stores_path: &Path, out_path: &Path) -> DataFrame {
    // 1. load
    let (sales, stores) = (
        fp_io::read_csv(sales_path).expect("pipeline: read sales.csv"),
        fp_io::read_csv(stores_path).expect("pipeline: read stores.csv"),
    );

    // 2. filter -- sales[sales["amount"] > 0.0]
    let keep = sales
        .get_column("amount")
        .gt_scalar(&Scalar::Float64(0.0))
        .expect("pipeline: amount > 0");
    let mask = keep
        .column()
        .as_bool_slice()
        .expect("pipeline: filter mask is an all-valid Bool column");
    let kept = sales.loc_bool(mask).expect("pipeline: filter");

    // 3. groupby -- kept.groupby("store_id", as_index=False).sum()
    let agg = kept
        .groupby_with_as_index(&["store_id"], false)
        .expect("pipeline: groupby store_id")
        .sum()
        .expect("pipeline: sum");

    // 4. join -- agg.merge(stores, on="store_id", how="inner")
    let merged =
        merge_dataframes_on_with(&agg, &stores, &["store_id"], &["store_id"], JoinType::Inner)
            .expect("pipeline: merge stores");
    let joined =
        DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
            .expect("pipeline: materialize merge");

    // 5. sort -- descending revenue, store_id tiebreak
    let ranked = joined
        .sort_values_multi(&["amount", "store_id"], &[false, true], "last")
        .expect("pipeline: rank");

    // 6. write
    fp_io::write_csv(&ranked, out_path).expect("pipeline: write output");
    ranked
}

fn stats(times_us: &mut [f64]) -> (f64, f64, f64, f64) {
    if times_us.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    times_us.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = times_us.len();
    let min = times_us.first().copied().unwrap_or(0.0);
    let max = times_us.last().copied().unwrap_or(0.0);
    let med = if n.is_multiple_of(2) {
        let left = times_us.get(n / 2 - 1).copied().unwrap_or(0.0);
        let right = times_us.get(n / 2).copied().unwrap_or(0.0);
        (left + right) / 2.0
    } else {
        times_us.get(n / 2).copied().unwrap_or(0.0)
    };
    let p95_idx = ((n as f64 * 0.95).ceil() as usize).saturating_sub(1);
    let p95 = times_us.get(p95_idx.min(n - 1)).copied().unwrap_or(0.0);
    (med, p95, min, max)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let data_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                "/home/ubuntu/.gemini/antigravity-cli/brain/6e1979ec-96e0-4d12-ac8c-cf4c2e2c9bb8/scratch/pipeline_1m",
            )
        });

    let sales_path = data_dir.join("sales.csv");
    let sales_copy_path = data_dir.join("sales_copy.csv");
    let sales_perm1 = data_dir.join("sales_perm1.csv");
    let sales_perm2 = data_dir.join("sales_perm2.csv");
    let stores_path = data_dir.join("stores.csv");

    assert!(sales_path.is_file(), "Missing sales.csv");
    assert!(sales_copy_path.is_file(), "Missing sales_copy.csv");
    assert!(sales_perm1.is_file(), "Missing sales_perm1.csv");
    assert!(sales_perm2.is_file(), "Missing sales_perm2.csv");
    assert!(stores_path.is_file(), "Missing stores.csv");

    println!("=== PROBE: PIPELINE CSV CACHE CONTAMINATION (br-frankenpandas-qnkah) ===");
    println!("Data directory: {}", data_dir.display());

    // 0. Verify pipeline output byte-identity across permuted variants
    println!("\n--- Step 0: Verify pipeline output identity across permuted variants ---");
    let out_orig = data_dir.join("probe_out_orig.csv");
    let out_perm1 = data_dir.join("probe_out_perm1.csv");
    let out_perm2 = data_dir.join("probe_out_perm2.csv");

    run_pipeline_stage(&sales_path, &stores_path, &out_orig);
    run_pipeline_stage(&sales_perm1, &stores_path, &out_perm1);
    run_pipeline_stage(&sales_perm2, &stores_path, &out_perm2);

    let bytes_orig = std::fs::read(&out_orig).expect("read orig");
    let bytes_perm1 = std::fs::read(&out_perm1).expect("read perm1");
    let bytes_perm2 = std::fs::read(&out_perm2).expect("read perm2");

    assert_eq!(bytes_orig, bytes_perm1, "perm1 output mismatch!");
    assert_eq!(bytes_orig, bytes_perm2, "perm2 output mismatch!");
    println!(
        "PASS: All 3 variants produce 100% byte-identical pipeline output ({} bytes)",
        bytes_orig.len()
    );

    const WARMUP: usize = 3;
    const SAMPLES: usize = 25;

    // 1. Arm 1: Warm cache hit (repeated reads of sales.csv)
    println!("\n--- Step 1: Arm 1 - Warm Cache Hit (repeated read_csv on sales.csv) ---");
    for _ in 0..WARMUP {
        let _ = black_box(fp_io::read_csv(&sales_path).unwrap());
    }
    let mut hit_times: Vec<f64> = (0..SAMPLES)
        .map(|_| {
            let t0 = Instant::now();
            let df = black_box(fp_io::read_csv(&sales_path).unwrap());
            let us = t0.elapsed().as_micros() as f64;
            black_box(df.len());
            us
        })
        .collect();
    let (hit_med, hit_p95, hit_min, hit_max) = stats(&mut hit_times);
    println!(
        "Warm HIT:      median = {:>8.2} µs ({:.3} ms) | p95 = {:>8.2} µs | min = {:>8.2} µs | max = {:>8.2} µs",
        hit_med,
        hit_med / 1000.0,
        hit_p95,
        hit_min,
        hit_max
    );

    // 2. Arm 2: Different path, identical content (sales_copy.csv)
    println!("\n--- Step 2: Arm 2 - Content Match on Different Path (sales_copy.csv) ---");
    // sales.csv is in cache. Now read sales_copy.csv:
    let mut copy_times: Vec<f64> = (0..SAMPLES)
        .map(|_| {
            let t0 = Instant::now();
            let df = black_box(fp_io::read_csv(&sales_copy_path).unwrap());
            let us = t0.elapsed().as_micros() as f64;
            black_box(df.len());
            us
        })
        .collect();
    let (copy_med, copy_p95, copy_min, copy_max) = stats(&mut copy_times);
    println!(
        "Copy Path HIT: median = {:>8.2} µs ({:.3} ms) | p95 = {:>8.2} µs | min = {:>8.2} µs | max = {:>8.2} µs",
        copy_med,
        copy_med / 1000.0,
        copy_p95,
        copy_min,
        copy_max
    );

    // 3. Arm 3: Cache Eviction / Guaranteed Miss (round-robin over 3 variants)
    println!("\n--- Step 3: Arm 3 - Cache Eviction / True Parse (3-way round robin) ---");
    let variants = [&sales_path, &sales_perm1, &sales_perm2];
    // Evict initial cache:
    for variant in &variants {
        let _ = black_box(fp_io::read_csv(variant).unwrap());
    }
    let mut miss_times: Vec<f64> = (0..SAMPLES)
        .map(|i| {
            let path = variants[i % 3];
            let t0 = Instant::now();
            let df = black_box(fp_io::read_csv(path).unwrap());
            let us = t0.elapsed().as_micros() as f64;
            black_box(df.len());
            us
        })
        .collect();
    let (miss_med, miss_p95, miss_min, miss_max) = stats(&mut miss_times);
    println!(
        "True MISS:     median = {:>8.2} µs ({:.3} ms) | p95 = {:>8.2} µs | min = {:>8.2} µs | max = {:>8.2} µs",
        miss_med,
        miss_med / 1000.0,
        miss_p95,
        miss_min,
        miss_max
    );

    let divergence = miss_med / hit_med;
    println!(
        "\n>>> CSV READ DIVERGENCE: True Parse is {:.2}x SLOWER than Cache Hit ({:.3} ms vs {:.3} ms)",
        divergence,
        miss_med / 1000.0,
        hit_med / 1000.0
    );

    // 4. Whole Pipeline: Cached Load vs Uncached Load
    println!("\n--- Step 4: Full 6-Stage Pipeline (Cached Load vs Uncached Load) ---");
    let out_bench = data_dir.join("probe_out_bench.csv");
    // Warmup cached pipeline
    for _ in 0..WARMUP {
        let _ = black_box(run_pipeline_stage(&sales_path, &stores_path, &out_bench));
    }
    let mut pipe_cached_times: Vec<f64> = (0..SAMPLES)
        .map(|_| {
            let t0 = Instant::now();
            let df = black_box(run_pipeline_stage(&sales_path, &stores_path, &out_bench));
            let us = t0.elapsed().as_micros() as f64;
            black_box(df.len());
            us
        })
        .collect();
    let (pipe_c_med, pipe_c_p95, pipe_c_min, pipe_c_max) = stats(&mut pipe_cached_times);
    println!(
        "Pipeline (Cached):   median = {:>8.2} µs ({:.3} ms) | p95 = {:>8.2} µs | min = {:>8.2} µs | max = {:>8.2} µs",
        pipe_c_med,
        pipe_c_med / 1000.0,
        pipe_c_p95,
        pipe_c_min,
        pipe_c_max
    );

    // Uncached pipeline (3-way round robin inputs)
    for variant in &variants {
        let _ = black_box(run_pipeline_stage(variant, &stores_path, &out_bench));
    }
    let mut pipe_uncached_times: Vec<f64> = (0..SAMPLES)
        .map(|i| {
            let t0 = Instant::now();
            let df = black_box(run_pipeline_stage(
                variants[i % 3],
                &stores_path,
                &out_bench,
            ));
            let us = t0.elapsed().as_micros() as f64;
            black_box(df.len());
            us
        })
        .collect();
    let (pipe_u_med, pipe_u_p95, pipe_u_min, pipe_u_max) = stats(&mut pipe_uncached_times);
    println!(
        "Pipeline (Uncached): median = {:>8.2} µs ({:.3} ms) | p95 = {:>8.2} µs | min = {:>8.2} µs | max = {:>8.2} µs",
        pipe_u_med,
        pipe_u_med / 1000.0,
        pipe_u_p95,
        pipe_u_min,
        pipe_u_max
    );

    let pipe_divergence = pipe_u_med / pipe_c_med;
    let load_delta = (miss_med - hit_med) / 1000.0;
    println!(
        "\n>>> PIPELINE WHOLE-JOB DIVERGENCE: Uncached is {:.2}x SLOWER ({:.3} ms vs {:.3} ms, Δ = +{:.3} ms)",
        pipe_divergence,
        pipe_u_med / 1000.0,
        pipe_c_med / 1000.0,
        (pipe_u_med - pipe_c_med) / 1000.0
    );
    println!(
        ">>> Load stage alone accounts for {:.1}% of the whole-job runtime difference",
        (load_delta / ((pipe_u_med - pipe_c_med) / 1000.0)) * 100.0
    );
}
