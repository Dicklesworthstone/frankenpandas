//! Isolates the single-key inner-join build+probe on UNSORTED, near-unique,
//! bounded-range Int64 keys — the shape where `dense_int64_inner_positions`
//! (counting-sort/CSR direct-address) replaces the FxHashMap build+probe.
//!
//! The pre-existing `perf_profile inner_join` scenario uses cardinality-512
//! keys, so its 100k×100k → ~19.5M-row cross product is output-materialization
//! bound and hides the build/probe cost. Here the keys are a scrambled, mostly
//! 1:1 mapping over `[0, ~1.6n)`, so the output is ≈ n rows and the build+probe
//! dominates the timed work.
//!
//! Modes:
//!   dense_join_bench golden <n>            -> single-key output digest (sha proof)
//!   dense_join_bench <n> <iters>           -> single-key timed loop (hyperfine)
//!   dense_join_bench golden_mk <n>         -> two-key fanout output digest
//!   dense_join_bench mk <n> <iters>        -> two-key fanout timed loop: an
//!     output-bound multi-key inner merge through the general builder, where
//!     take_positions_typed keeps the per-column Int64 gather typed.

use std::time::Instant;

use fp_frame::DataFrame;
use fp_join::{JoinType, merge_dataframes, merge_dataframes_on};
use fp_types::Scalar;

/// Two-key, moderate-fanout frame: `id1`,`id2` each at cardinality `card` plus
/// `n_val` all-valid Int64 payload columns. With card^2 < n every (id1,id2)
/// combo repeats, so an inner self-join fans out to ~n^2/card^2 rows — an
/// output-bound multi-key merge that flows through the general builder and its
/// per-column positional gather (the part `take_positions_typed` keeps typed).
fn build_mk_frame(tag: &str, n: usize, card: i64, salt: i64, n_val: usize) -> DataFrame {
    // id1 cycles fast, id2 cycles slow => the two keys are independent and the
    // distinct (id1,id2) combos number ~card^2 (not card), giving a controlled
    // ~n/card^2 fanout per combo rather than a degenerate cross product.
    let id1: Vec<Scalar> = (0..n).map(|i| Scalar::Int64((i as i64) % card)).collect();
    let id2: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i as i64 / card) % card))
        .collect();
    let mut names: Vec<String> = vec!["id1".into(), "id2".into()];
    let mut cols: Vec<(String, Vec<Scalar>)> = vec![("id1".into(), id1), ("id2".into(), id2)];
    for c in 0..n_val {
        let name = format!("{tag}{c}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64((i as i64).wrapping_mul(salt + c as i64).wrapping_add(1)))
            .collect();
        names.push(name.clone());
        cols.push((name, v));
    }
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let col_refs: Vec<(&str, Vec<Scalar>)> =
        cols.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
    DataFrame::from_dict(&name_refs, col_refs).expect("mk frame")
}

/// Unsorted, near-unique key in `[0, span)` via a Fibonacci-hash scramble of the
/// row index. span = next-power-of-two ≥ ~1.6n keeps the span bounded (the dense
/// gate needs span ≤ 16·rows) while leaving a low collision rate (mostly 1:1).
fn scramble(i: usize, span: u64) -> i64 {
    let h = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    ((h >> 17) % span) as i64
}

fn build_frame(value_name: &str, n: usize, span: u64, salt: i64) -> DataFrame {
    let keys: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(scramble(i, span))).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i as i64).wrapping_mul(salt).wrapping_add(1)))
        .collect();
    DataFrame::from_dict(
        &["id", value_name],
        vec![("id", keys), (value_name, values)],
    )
    .expect("frame")
}

fn span_for(n: usize) -> u64 {
    // ~1.6n rounded up to a power of two (≥ 8): both sides scatter over the same
    // key space and share the same scramble, so the inner join is ~1:1 (output ≈
    // n rows) rather than a cross-product explosion. This isolates the single-key
    // build+probe (the part `dense_int64_inner_positions` replaces) instead of
    // the output-materialization-bound low-cardinality shape in `perf_profile
    // inner_join`.
    ((n as u64) * 8 / 5).max(8).next_power_of_two()
}

fn golden(n: usize) {
    let span = span_for(n);
    let left = build_frame("lv", n, span, 7);
    let right = build_frame("rv", n, span, 13);
    let merged = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
    let out = DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
        .expect("rebuild merged frame");

    // Order-sensitive digest of the merged output: fold every cell of every
    // column (in column-order, row-order) into a rolling FNV-1a hash so any
    // change in row order, row count, or values flips the digest.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(out.len() as u64);
    for name in out.column_names() {
        for s in name.bytes() {
            mix(u64::from(s));
        }
        let col = out.column(name).expect("col");
        for v in col.values().iter() {
            match v {
                Scalar::Int64(i) => mix(*i as u64),
                Scalar::Null(_) => mix(0xDEAD_BEEF),
                other => mix(format!("{other:?}")
                    .bytes()
                    .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b)))),
            }
        }
    }
    println!("rows={} digest={:016x}", out.len(), h);
}

fn mk_card(n: usize) -> i64 {
    // card^2 ~= n/4 so the two-key inner self-join fans out to ~4n rows.
    ((n as f64 / 4.0).sqrt() as i64).max(2)
}

fn mku_card(n: usize) -> i64 {
    // card^2 ~= n so the two-key combos are ~all distinct (near-1:1 inner join,
    // output ~= n). With a single payload column the build (composite key hash
    // vs packed dense) dominates — isolating dense_packed_int64_inner_positions.
    ((n as f64).sqrt() as i64 + 1).max(2)
}

fn run_mk(n: usize, card: i64, n_val: usize, golden_mode: bool, iters: usize) {
    let left = build_mk_frame("lv", n, card, 7, n_val);
    let right = build_mk_frame("rv", n, card, 13, n_val);
    if golden_mode {
        let merged =
            merge_dataframes_on(&left, &right, &["id1", "id2"], JoinType::Inner).expect("mk join");
        let out =
            DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
                .expect("rebuild mk frame");
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |x: u64| {
            h ^= x;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        mix(out.len() as u64);
        for name in out.column_names() {
            for s in name.bytes() {
                mix(u64::from(s));
            }
            let col = out.column(name).expect("col");
            for v in col.values().iter() {
                match v {
                    Scalar::Int64(i) => mix(*i as u64),
                    Scalar::Null(_) => mix(0xDEAD_BEEF),
                    other => mix(format!("{other:?}")
                        .bytes()
                        .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b)))),
                }
            }
        }
        println!("rows={} digest={:016x}", out.len(), h);
        return;
    }
    let start = Instant::now();
    let mut sink: usize = 0;
    for _ in 0..iters {
        let out =
            merge_dataframes_on(&left, &right, &["id1", "id2"], JoinType::Inner).expect("mk join");
        sink = sink.wrapping_add(out.index.len());
    }
    let elapsed = start.elapsed();
    eprintln!(
        "dense_join_bench: n={n} card={card} n_val={n_val} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / iters as f64,
    );
}

fn golden_mk(n: usize) {
    let card = mk_card(n);
    let left = build_mk_frame("lv", n, card, 7, 4);
    let right = build_mk_frame("rv", n, card, 13, 4);
    let merged =
        merge_dataframes_on(&left, &right, &["id1", "id2"], JoinType::Inner).expect("mk join");
    let out = DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
        .expect("rebuild mk frame");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(out.len() as u64);
    for name in out.column_names() {
        for s in name.bytes() {
            mix(u64::from(s));
        }
        let col = out.column(name).expect("col");
        for v in col.values().iter() {
            match v {
                Scalar::Int64(i) => mix(*i as u64),
                Scalar::Null(_) => mix(0xDEAD_BEEF),
                other => mix(format!("{other:?}")
                    .bytes()
                    .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b)))),
            }
        }
    }
    println!("rows={} digest={:016x}", out.len(), h);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("golden") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        golden(n);
        return;
    }
    if args.get(1).map(String::as_str) == Some("golden_mk") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        golden_mk(n);
        return;
    }
    if args.get(1).map(String::as_str) == Some("golden_mku") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        run_mk(n, mku_card(n), 1, true, 0);
        return;
    }
    if args.get(1).map(String::as_str) == Some("mku") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200_000);
        let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
        run_mk(n, mku_card(n), 1, false, iters);
        return;
    }
    if args.get(1).map(String::as_str) == Some("mk") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50_000);
        let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
        let card = mk_card(n);
        let left = build_mk_frame("lv", n, card, 7, 4);
        let right = build_mk_frame("rv", n, card, 13, 4);
        let start = Instant::now();
        let mut sink: usize = 0;
        for _ in 0..iters {
            let out = merge_dataframes_on(&left, &right, &["id1", "id2"], JoinType::Inner)
                .expect("mk join");
            sink = sink.wrapping_add(out.index.len());
        }
        let elapsed = start.elapsed();
        eprintln!(
            "dense_join_bench mk: n={n} card={card} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / iters as f64,
        );
        return;
    }

    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let span = span_for(n);
    let left = build_frame("lv", n, span, 7);
    let right = build_frame("rv", n, span, 13);

    let start = Instant::now();
    let mut sink: usize = 0;
    for _ in 0..iters {
        let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
        sink = sink.wrapping_add(out.index.len());
    }
    let elapsed = start.elapsed();
    eprintln!(
        "dense_join_bench: n={n} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / iters as f64,
    );
}
