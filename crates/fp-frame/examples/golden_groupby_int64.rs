//! Golden bit-identity harness for int64-keyed DataFrameGroupBy aggregations.
//!
//! Exercises every dense-supported reduction (sum/mean/count/min/max/var/std/
//! first/last/prod/median) over single- and multi-int64-key groupings, with
//! both `sort=True`/`sort=False` and `as_index=True`/`as_index=False`, over a
//! mix of Float64 and Int64 value columns plus a non-dense (Utf8) value column
//! to force the build_groups fallback. Prints a stable string of every output
//! cell so callers can sha256 it before/after a refactor and prove the result
//! is byte-identical.
//!
//! Run: cargo run -p fp-frame --example golden_groupby_int64 --release

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};

fn splitmix_u64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn build_frame(n: usize, n_groups: i64, two_keys: bool, with_str: bool) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut order = Vec::new();

    let key0: Vec<i64> = (0..n)
        .map(|i| (splitmix_u64(i as u64) % n_groups as u64) as i64)
        .collect();
    columns.insert("key".to_string(), Column::from_i64_values(key0));
    order.push("key".to_string());

    if two_keys {
        let key1: Vec<i64> = (0..n)
            .map(|i| (splitmix_u64(i as u64 ^ 0xabcd) % 3) as i64)
            .collect();
        columns.insert("key2".to_string(), Column::from_i64_values(key1));
        order.push("key2".to_string());
    }

    let fcol: Vec<f64> = (0..n)
        .map(|i| {
            let u = (splitmix_u64(i as u64 ^ 0x1111) >> 11) as f64 / (1u64 << 53) as f64;
            u.mul_add(20.0, -10.0)
        })
        .collect();
    columns.insert("fval".to_string(), Column::from_f64_values(fcol));
    order.push("fval".to_string());

    let icol: Vec<i64> = (0..n)
        .map(|i| (splitmix_u64(i as u64 ^ 0x2222) % 1000) as i64 - 500)
        .collect();
    columns.insert("ival".to_string(), Column::from_i64_values(icol));
    order.push("ival".to_string());

    if with_str {
        let scol: Vec<fp_types::Scalar> = (0..n)
            .map(|i| fp_types::Scalar::Utf8(format!("s{}", splitmix_u64(i as u64 ^ 0x3333) % 7)))
            .collect();
        columns.insert("sval".to_string(), Column::from_values(scol).unwrap());
        order.push("sval".to_string());
    }

    DataFrame::new_with_column_order(index, columns, order).expect("frame")
}

fn emit(out: &mut String, tag: &str, df: &Result<DataFrame, fp_frame::FrameError>) {
    match df {
        Ok(df) => {
            out.push_str(tag);
            out.push('\n');
            // Index name + labels
            out.push_str("idx:");
            for lbl in df.index().labels() {
                out.push_str(&format!("{lbl:?}|"));
            }
            out.push('\n');
            for name in df.column_names() {
                out.push_str(name);
                out.push(':');
                let col = df.column(name).expect("col");
                for v in col.values() {
                    out.push_str(&format!("{v:?}|"));
                }
                out.push('\n');
            }
        }
        Err(e) => {
            out.push_str(&format!("{tag} ERR {e:?}\n"));
        }
    }
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000);

    let funcs = [
        "sum", "mean", "count", "min", "max", "var", "std", "first", "last", "prod", "median",
    ];
    let mut out = String::new();

    for &two_keys in &[false, true] {
        for &with_str in &[false, true] {
            for &sort in &[true, false] {
                for &as_index in &[true, false] {
                    let df = build_frame(n, 100, two_keys, with_str);
                    let by: Vec<&str> = if two_keys {
                        vec!["key", "key2"]
                    } else {
                        vec!["key"]
                    };
                    let gb = df.groupby_full(&by, as_index, sort).expect("groupby_full");
                    for &f in &funcs {
                        let res = match f {
                            "sum" => gb.sum(),
                            "mean" => gb.mean(),
                            "count" => gb.count(),
                            "min" => gb.min(),
                            "max" => gb.max(),
                            "var" => gb.var(),
                            "std" => gb.std(),
                            "first" => gb.first(),
                            "last" => gb.last(),
                            "prod" => gb.prod(),
                            "median" => gb.median(),
                            _ => unreachable!(),
                        };
                        let tag = format!(
                            "two_keys={two_keys} with_str={with_str} sort={sort} as_index={as_index} f={f}"
                        );
                        emit(&mut out, &tag, &res);
                    }
                }
            }
        }
    }

    // Tiny FNV-1a hash so we don't pull in a sha crate; stable + sufficient to
    // catch any byte drift between before/after builds.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in out.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    println!("GOLDEN_FNV1A {h:016x}  bytes={}", out.len());
}
