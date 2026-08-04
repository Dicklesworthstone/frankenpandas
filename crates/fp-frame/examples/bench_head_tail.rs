//! Bench + golden for `Series.head(k)` / `Series.tail(k)` and
//! `DataFrame.head(k)` / `DataFrame.tail(k)` on large Int64 data.
//!
//! This exercises the zero-copy `Column::slice` + `Index::slice` levers
//! (br-frankenpandas-6wx84 and br-frankenpandas-4dhu8). The win is largest for a
//! small `k` on big data: the pre-patch path materialized all `n` values to
//! `Vec<Scalar>` before taking the prefix/suffix, whereas the slice path touches
//! only `k` typed elements per column.
//! Golden = FNV-1a64 over the head and tail (index-label, value) pairs.
//!
//! Run: cargo run -p fp-frame --example bench_head_tail --release -- 2000000 5 100 10

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn build(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    Series::from_values("v", labels, values).expect("build series")
}

fn build_frame(n: usize, cols: usize) -> DataFrame {
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let name = format!("c{c}");
        let values = (0..n as i64).map(|i| i + c as i64).collect();
        columns.insert(name.clone(), Column::from_i64_values(values));
        column_order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("build frame")
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn digest(s: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (lbl, val) in s.index().labels().iter().zip(s.values().iter()) {
        let l = match lbl {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        };
        let v = match val {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        };
        fnv1a64_update(&mut h, &l.to_le_bytes());
        fnv1a64_update(&mut h, &v.to_le_bytes());
    }
    h
}

fn digest_frame(df: &DataFrame) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let names = df.column_names();
    for (row, lbl) in df.index().labels().iter().enumerate() {
        let l = match lbl {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        };
        fnv1a64_update(&mut h, &l.to_le_bytes());
        for name in &names {
            fnv1a64_update(&mut h, name.as_bytes());
            let value = df
                .column(name)
                .expect("column listed in names")
                .values()
                .get(row)
                .and_then(|value| match value {
                    Scalar::Int64(x) => Some(*x),
                    _ => None,
                })
                .unwrap_or(i64::MIN);
            fnv1a64_update(&mut h, &value.to_le_bytes());
        }
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let k: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
    let cols: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    let series = build(n);
    let golden_head = digest(&series.head(k).expect("head"));
    let golden_tail = digest(&series.tail(k).expect("tail"));

    let mut best_head = u128::MAX;
    let mut best_tail = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let h = series.head(k).expect("head");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&h);
        if e < best_head {
            best_head = e;
        }

        let t = Instant::now();
        let tl = series.tail(k).expect("tail");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&tl);
        if e < best_tail {
            best_tail = e;
        }
    }

    println!(
        "head_tail n={n} k={k} iters={iters}: head_best={best_head}ns tail_best={best_tail}ns \
         golden_head={golden_head:016x} golden_tail={golden_tail:016x}"
    );

    let frame = build_frame(n, cols);
    let frame_golden_head = digest_frame(&frame.head(k).expect("df head"));
    let frame_golden_tail = digest_frame(&frame.tail(k).expect("df tail"));

    let mut frame_best_head = u128::MAX;
    let mut frame_best_tail = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let h = frame.head(k).expect("df head");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&h);
        if e < frame_best_head {
            frame_best_head = e;
        }

        let t = Instant::now();
        let tl = frame.tail(k).expect("df tail");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&tl);
        if e < frame_best_tail {
            frame_best_tail = e;
        }
    }

    println!(
        "df_head_tail n={n} rows={k} cols={cols} iters={iters}: \
         head_best={frame_best_head}ns tail_best={frame_best_tail}ns \
         golden_head={frame_golden_head:016x} golden_tail={frame_golden_tail:016x}"
    );
}
