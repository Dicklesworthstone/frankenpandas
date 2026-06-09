//! Generative perf gap-hunt probe (measurement only): times un-benchmarked
//! DataFrame ops at scale to surface a fresh vs-pandas algorithmic gap.
//! Run: cargo run -p fp-conformance --profile release-perf --example gap_hunt -- 200000
use std::time::Instant;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_types::Scalar;

fn numeric_frame(n: usize, cols: usize, with_nulls: bool) -> DataFrame {
    // TYPED columns (from_f64_values / _with_validity) — the realistic backing
    // a CSV read or numeric computation produces. This isolates per-op typed
    // fast-path gaps from the from_dict-Scalar-backing question.
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = std::collections::BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..cols {
        let name = format!("c{c}");
        let v: Vec<f64> = (0..n)
            .map(|i| ((i * (c + 1)) % 9973) as f64 * 0.25)
            .collect();
        let col = if with_nulls {
            let mut validity = fp_columnar::ValidityMask::all_valid(n);
            for i in 0..n {
                if (i + c) % 7 == 0 {
                    validity.set(i, false);
                }
            }
            Column::from_f64_values_with_validity(v, validity)
        } else {
            Column::from_f64_values(v)
        };
        columns.insert(name.clone(), col);
        order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, order).expect("frame")
}

fn shuffled_index_frame(n: usize, cols: usize) -> DataFrame {
    // Deterministic shuffle of the Int64 index so sort_index does real work.
    let mut labels: Vec<IndexLabel> = (0..n)
        .map(|i| {
            let mixed = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17)
                ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            IndexLabel::Int64((mixed % (n as u64 * 4)) as i64)
        })
        .collect();
    // ensure uniqueness not required for sort_index timing
    labels.truncate(n);
    let index = Index::new(labels);
    let mut columns = std::collections::BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..cols {
        let name = format!("c{c}");
        let v: Vec<f64> = (0..n).map(|i| ((i * (c + 1)) % 9973) as f64 * 0.25).collect();
        columns.insert(name.clone(), Column::from_f64_values(v));
        order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, order).expect("shuffled frame")
}

fn int_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let mut columns = std::collections::BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..cols {
        let name = format!("c{c}");
        let v: Vec<i64> = (0..n).map(|i| ((i * (c + 1)) % 9973) as i64 - 4000).collect();
        columns.insert(name.clone(), Column::from_i64_values(v));
        order.push(name);
    }
    DataFrame::new_with_column_order(Index::new(labels), columns, order).expect("int frame")
}

fn time_it<F: FnMut()>(label: &str, warmup: usize, iters: usize, mut f: F) {
    for _ in 0..warmup {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let ms = start.elapsed().as_secs_f64() * 1e3 / iters as f64;
    println!("{ms:>10.3} ms/iter  {label}");
}

fn golden_dump(df: &DataFrame) -> String {
    let mut out = String::new();
    for name in df.column_names() {
        out.push_str(name);
        out.push(':');
        for v in df.columns()[name].values().iter() {
            out.push_str(&format!("{v:?};"));
        }
        out.push('\n');
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("golden") {
        // Deterministic small frames (typed + nullable) for sha256 isomorphism
        // proofs across the diff/pct_change typed fast paths.
        let f = numeric_frame(5000, 4, false);
        let fnull = numeric_frame(5000, 4, true);
        print!("{}", golden_dump(&f.diff(1).unwrap()));
        print!("{}", golden_dump(&f.diff(3).unwrap()));
        print!("{}", golden_dump(&f.diff(-2).unwrap()));
        print!("{}", golden_dump(&fnull.diff(1).unwrap()));
        print!("{}", golden_dump(&f.pct_change(1).unwrap()));
        print!("{}", golden_dump(&fnull.pct_change(1).unwrap()));
        // combine_first: nullable self over nullable other (overlapping nulls),
        // and nullable self over all-valid other.
        let other_null = numeric_frame(5000, 4, true);
        print!("{}", golden_dump(&fnull.combine_first(&other_null).unwrap()));
        print!("{}", golden_dump(&fnull.combine_first(&f).unwrap()));
        // ffill / bfill (carry fills) over a nullable frame, with and without limit.
        print!("{}", golden_dump(&fnull.ffill(None).unwrap()));
        print!("{}", golden_dump(&fnull.ffill(Some(2)).unwrap()));
        print!("{}", golden_dump(&fnull.bfill(None).unwrap()));
        print!("{}", golden_dump(&fnull.bfill(Some(2)).unwrap()));
        // corrwith: all-valid self over nullable other (pairs dropped), and
        // nullable self over nullable other.
        print!("{}", golden_dump(&f.corrwith(&other_null).unwrap().to_frame(Some("c")).unwrap()));
        print!("{}", golden_dump(&fnull.corrwith(&other_null).unwrap().to_frame(Some("c")).unwrap()));
        // interpolate: interior gaps linear-filled, trailing carried, leading NaN.
        print!("{}", golden_dump(&fnull.interpolate().unwrap()));
        // duplicated / drop_duplicates: n>9973 gives real period-9973 row dups;
        // nullable variant exercises missing-equality. All keep modes.
        let dup = numeric_frame(20000, 3, false);
        let dupn = numeric_frame(20000, 3, true);
        for keep in [DuplicateKeep::First, DuplicateKeep::Last, DuplicateKeep::None] {
            print!(
                "{}",
                golden_dump(&dup.duplicated(None, keep).unwrap().to_frame(Some("d")).unwrap())
            );
            print!(
                "{}",
                golden_dump(&dupn.duplicated(None, keep).unwrap().to_frame(Some("d")).unwrap())
            );
            print!("{}", golden_dump(&dup.drop_duplicates(None, keep, false).unwrap()));
            print!("{}", golden_dump(&dupn.drop_duplicates(None, keep, false).unwrap()));
        }
        // clip: two-sided, nullable, reversed-bound swap (GH2747), one-sided.
        print!("{}", golden_dump(&f.clip(Some(0.0), Some(1000.0)).unwrap()));
        print!("{}", golden_dump(&fnull.clip(Some(100.0), Some(500.0)).unwrap()));
        print!("{}", golden_dump(&f.clip(Some(7.0), Some(3.0)).unwrap()));
        print!("{}", golden_dump(&fnull.clip(None, Some(400.0)).unwrap()));
        // fillna: nullable filled with a finite constant; all-valid is a no-op.
        print!("{}", golden_dump(&fnull.fillna(&Scalar::Float64(0.0)).unwrap()));
        print!("{}", golden_dump(&fnull.fillna(&Scalar::Float64(-1.5)).unwrap()));
        print!("{}", golden_dump(&f.fillna(&Scalar::Float64(9.0)).unwrap()));
        // frame-vs-frame comparison: all-valid vs nullable (result has nulls),
        // and all-valid vs all-valid (all-valid Bool result).
        print!("{}", golden_dump(&f.gt(&other_null).unwrap()));
        print!("{}", golden_dump(&f.lt(&other_null).unwrap()));
        print!("{}", golden_dump(&f.eq(&other_null).unwrap()));
        print!("{}", golden_dump(&f.ge(&f).unwrap()));
        print!("{}", golden_dump(&fnull.ne(&other_null).unwrap()));
        // where_cond / mask: cond has nulls (from gt vs nullable); self all-valid
        // and nullable; finite fill and the default (None) fill.
        let cond = f.gt(&other_null).unwrap();
        print!("{}", golden_dump(&f.where_cond(&cond, Some(&Scalar::Float64(0.0))).unwrap()));
        print!("{}", golden_dump(&f.mask(&cond, Some(&Scalar::Float64(-1.0))).unwrap()));
        print!("{}", golden_dump(&fnull.where_cond(&cond, Some(&Scalar::Float64(7.0))).unwrap()));
        print!("{}", golden_dump(&f.where_cond(&cond, None).unwrap()));
        // frame-vs-frame arithmetic: missing propagation (nullable other) and a
        // NaN op result (div by self: 0.0/0.0 at row 0 -> Float64(NaN)).
        print!("{}", golden_dump(&f.add_df(&other_null).unwrap()));
        print!("{}", golden_dump(&f.sub_df(&other_null).unwrap()));
        print!("{}", golden_dump(&f.mul_df(&other_null).unwrap()));
        print!("{}", golden_dump(&f.div_df(&f).unwrap()));
        print!("{}", golden_dump(&fnull.add_df(&other_null).unwrap()));
        // isin: exact float needles, cross-type Int64 needles (0.0 matches
        // Int64(0)), and a NaN needle that matches missing slots of fnull.
        print!(
            "{}",
            golden_dump(
                &f.isin(&[Scalar::Float64(1.0), Scalar::Float64(2.5), Scalar::Float64(100.0)])
                    .unwrap()
            )
        );
        print!("{}", golden_dump(&f.isin(&[Scalar::Int64(0), Scalar::Int64(250)]).unwrap()));
        print!("{}", golden_dump(&fnull.isin(&[Scalar::Float64(f64::NAN)]).unwrap()));
        // quantile: several q over all-valid and nullable (nulls filtered) frames.
        for q in [0.0, 0.25, 0.5, 0.9, 1.0] {
            print!("{}", golden_dump(&f.quantile(q).unwrap().to_frame(Some("q")).unwrap()));
            print!("{}", golden_dump(&fnull.quantile(q).unwrap().to_frame(Some("q")).unwrap()));
        }
        // nunique: distinct counts (all-valid + nullable), and dropna=false which
        // counts the missing bucket as one extra distinct value.
        print!("{}", golden_dump(&f.nunique().unwrap().to_frame(Some("nu")).unwrap()));
        print!("{}", golden_dump(&fnull.nunique().unwrap().to_frame(Some("nu")).unwrap()));
        print!(
            "{}",
            golden_dump(&fnull.nunique_with_dropna(false).unwrap().to_frame(Some("nu")).unwrap())
        );
        // describe: count/mean/std/min/25%/50%/75%/max over all-valid + nullable.
        print!("{}", golden_dump(&f.describe().unwrap()));
        print!("{}", golden_dump(&fnull.describe().unwrap()));
        // skew / kurtosis: two/four-moment reductions via numeric_values.
        print!("{}", golden_dump(&f.skew().unwrap().to_frame(Some("sk")).unwrap()));
        print!("{}", golden_dump(&fnull.skew().unwrap().to_frame(Some("sk")).unwrap()));
        print!("{}", golden_dump(&f.kurtosis_agg().unwrap().to_frame(Some("ku")).unwrap()));
        print!("{}", golden_dump(&fnull.kurtosis_agg().unwrap().to_frame(Some("ku")).unwrap()));
        // mode: repeated-value frame (period-9973 dups at n=20000 -> real modes)
        // all-valid and nullable; plus an all-distinct frame (every value a mode).
        print!("{}", golden_dump(&dup.mode().unwrap()));
        print!("{}", golden_dump(&dupn.mode().unwrap()));
        print!("{}", golden_dump(&f.mode().unwrap()));
        // Int64 frame comparison (compared as f64, matching the Scalar path).
        let fi = int_frame(5000, 4);
        let fi2 = int_frame(5000, 4);
        print!("{}", golden_dump(&fi.gt(&fi2).unwrap()));
        print!("{}", golden_dump(&fi.lt(&fi2).unwrap()));
        print!("{}", golden_dump(&fi.eq(&fi2).unwrap()));
        print!("{}", golden_dump(&fi.ge(&fi).unwrap()));
        // Int64 diff -> Float64 output (a as f64 - b as f64), in/out-of-range.
        print!("{}", golden_dump(&fi.diff(1).unwrap()));
        print!("{}", golden_dump(&fi.diff(3).unwrap()));
        print!("{}", golden_dump(&fi.diff(-2).unwrap()));
        return;
    }
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200_000);
    let cols = 8;
    println!("gap_hunt n={n} cols={cols}");

    let f = numeric_frame(n, cols, false);
    let fnull = numeric_frame(n, cols, true);
    let other = numeric_frame(n, cols, true);
    let shuf = shuffled_index_frame(n, cols);

    time_it("drop_duplicates(all cols)", 1, 10, || {
        let _ = f.drop_duplicates(None, DuplicateKeep::First, false).unwrap();
    });
    time_it("duplicated(all cols)", 1, 10, || {
        let _ = f.duplicated(None, DuplicateKeep::First).unwrap();
    });
    time_it("sort_index(shuffled)", 1, 10, || {
        let _ = shuf.sort_index(true).unwrap();
    });
    time_it("combine_first", 1, 10, || {
        let _ = fnull.combine_first(&other).unwrap();
    });
    time_it("cumsum", 1, 20, || {
        let _ = f.cumsum().unwrap();
    });
    time_it("diff(1)", 1, 20, || {
        let _ = f.diff(1).unwrap();
    });
    time_it("pct_change(1)", 1, 20, || {
        let _ = f.pct_change(1).unwrap();
    });
    time_it("corrwith", 1, 20, || {
        let _ = f.corrwith(&other).unwrap();
    });
    time_it("ffill(nulls)", 1, 20, || {
        let _ = fnull.ffill(None).unwrap();
    });
    time_it("bfill(nulls)", 1, 20, || {
        let _ = fnull.bfill(None).unwrap();
    });
    time_it("fillna(nulls)", 1, 20, || {
        let _ = fnull.fillna(&Scalar::Float64(0.0)).unwrap();
    });
    time_it("interpolate(nulls)", 1, 20, || {
        let _ = fnull.interpolate().unwrap();
    });
    // Second-wave probes: more un-benched elementwise / scan ops.
    time_it("clip(0,1000)", 1, 20, || {
        let _ = f.clip(Some(0.0), Some(1000.0)).unwrap();
    });
    time_it("round(2)", 1, 20, || {
        let _ = f.round(2).unwrap();
    });
    time_it("abs", 1, 20, || {
        let _ = f.abs().unwrap();
    });
    time_it("cummax", 1, 20, || {
        let _ = f.cummax().unwrap();
    });
    time_it("cummin", 1, 20, || {
        let _ = f.cummin().unwrap();
    });
    time_it("cumprod", 1, 20, || {
        let _ = f.cumprod().unwrap();
    });
    time_it("rank(average)", 1, 10, || {
        let _ = f.rank("average", true, "keep").unwrap();
    });
    // Third-wave probes: comparison / where / mask / isin / scalar arithmetic.
    let cond = f.gt(&other).unwrap();
    time_it("gt(frame)", 1, 20, || {
        let _ = f.gt(&other).unwrap();
    });
    time_it("where_cond", 1, 20, || {
        let _ = f.where_cond(&cond, Some(&Scalar::Float64(0.0))).unwrap();
    });
    time_it("mask", 1, 20, || {
        let _ = f.mask(&cond, Some(&Scalar::Float64(0.0))).unwrap();
    });
    time_it("isin", 1, 20, || {
        let _ = f
            .isin(&[Scalar::Float64(1.0), Scalar::Float64(2.5), Scalar::Float64(100.0)])
            .unwrap();
    });
    time_it("add_scalar(1)", 1, 20, || {
        let _ = f.add_scalar(1.0).unwrap();
    });
    time_it("mul_scalar(2)", 1, 20, || {
        let _ = f.mul_scalar(2.0).unwrap();
    });
    // Fourth-wave probes: frame-vs-frame arithmetic (same shape, align skipped
    // already, but still a per-element Scalar to_f64 loop).
    time_it("add_df(frame)", 1, 20, || {
        let _ = f.add_df(&other).unwrap();
    });
    time_it("sub_df(frame)", 1, 20, || {
        let _ = f.sub_df(&other).unwrap();
    });
    time_it("mul_df(frame)", 1, 20, || {
        let _ = f.mul_df(&other).unwrap();
    });
    time_it("div_df(frame)", 1, 20, || {
        let _ = f.div_df(&other).unwrap();
    });
    // Fifth-wave probes: DataFrame column reductions (each -> a per-column Series).
    time_it("var", 1, 20, || {
        let _ = f.var().unwrap();
    });
    time_it("mode", 1, 10, || {
        let _ = f.mode().unwrap();
    });
    let fi = int_frame(n, 8);
    time_it("i64.clip", 1, 20, || {
        let _ = fi.clip(Some(-1000.0), Some(1000.0)).unwrap();
    });
    time_it("i64.abs", 1, 20, || {
        let _ = fi.abs().unwrap();
    });
    time_it("i64.diff", 1, 20, || {
        let _ = fi.diff(1).unwrap();
    });
    time_it("i64.cumsum", 1, 20, || {
        let _ = fi.cumsum().unwrap();
    });
    time_it("i64.nunique", 1, 10, || {
        let _ = fi.nunique().unwrap();
    });
    time_it("i64.gt", 1, 20, || {
        let _ = fi.gt(&fi).unwrap();
    });
    time_it("sem", 1, 20, || {
        let _ = f.sem_agg().unwrap();
    });
    time_it("prod", 1, 20, || {
        let _ = f.prod().unwrap();
    });
    time_it("std", 1, 20, || {
        let _ = f.std().unwrap();
    });
    time_it("median", 1, 20, || {
        let _ = f.median().unwrap();
    });
    time_it("skew", 1, 20, || {
        let _ = f.skew().unwrap();
    });
    time_it("quantile(0.5)", 1, 20, || {
        let _ = f.quantile(0.5).unwrap();
    });
    time_it("nunique", 1, 10, || {
        let _ = f.nunique().unwrap();
    });
    time_it("describe", 1, 10, || {
        let _ = f.describe().unwrap();
    });
}
