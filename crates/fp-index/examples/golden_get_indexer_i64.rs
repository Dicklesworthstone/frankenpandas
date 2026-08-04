//! Differential golden for the typed-i64 unsorted `Index::get_indexer` fast
//! path: proves it is bit-identical to an independent first-occurrence
//! reference across the dense-table and hashmap sub-paths plus edge cases
//! (negatives, duplicates, absent targets, span just over the dense cap, empty).
//!
//! Run: cargo run -p fp-index --example golden_get_indexer_i64 --release

use std::collections::HashMap;

use fp_index::{Index, IndexLabel};

fn reference(haystack: &[i64], target: &[i64]) -> Vec<Option<usize>> {
    // First-occurrence position of each target value within haystack.
    let mut map: HashMap<i64, usize> = HashMap::new();
    for (i, &v) in haystack.iter().enumerate() {
        map.entry(v).or_insert(i);
    }
    target.iter().map(|&v| map.get(&v).copied()).collect()
}

fn actual(haystack: &[i64], target: &[i64]) -> Vec<Option<usize>> {
    let h: Vec<IndexLabel> = haystack.iter().map(|&v| IndexLabel::Int64(v)).collect();
    let t: Vec<IndexLabel> = target.iter().map(|&v| IndexLabel::Int64(v)).collect();
    Index::new(h).get_indexer(&Index::new(t))
}

fn check(name: &str, haystack: &[i64], target: &[i64]) {
    let r = reference(haystack, target);
    let a = actual(haystack, target);
    if r == a {
        println!("OK   {name}");
    } else {
        println!("FAIL {name}");
        for i in 0..r.len().max(a.len()) {
            if r.get(i) != a.get(i) {
                println!("  [{i}] ref={:?} act={:?}", r.get(i), a.get(i));
            }
        }
        std::process::exit(1);
    }
}

fn main() {
    let mut z = 0xdead_beef_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };

    // Dense bounded path: shuffled permutation, half-matching target.
    let n = 30_000usize;
    let mut perm: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        let j = rnd((i + 1) as i64) as usize;
        perm.swap(i, j);
    }
    let target: Vec<i64> = (0..n as i64).map(|i| i * 2).collect();
    check("dense_shuffled_perm", &perm, &target);

    // Dense with duplicates (must keep first occurrence).
    let dup: Vec<i64> = (0..n).map(|i| (i % 500) as i64).collect();
    check(
        "dense_with_duplicates",
        &dup,
        &(0i64..600).collect::<Vec<_>>(),
    );

    // Negative range (dense, min < 0).
    let neg: Vec<i64> = (0..n).map(|_| rnd(2000) - 1000).collect();
    check(
        "dense_negative_range",
        &neg,
        &(-1200i64..1200).collect::<Vec<_>>(),
    );

    // Sparse / unbounded span -> hashmap path (values spread across huge range).
    let sparse: Vec<i64> = (0..n).map(|i| (i as i64) * 1_000_003).collect();
    let mut sparse_shuf = sparse.clone();
    for i in (1..sparse_shuf.len()).rev() {
        let j = rnd((i + 1) as i64) as usize;
        sparse_shuf.swap(i, j);
    }
    let sparse_target: Vec<i64> = (0..n as i64).map(|i| i * 2_000_006).collect();
    check("sparse_hashmap_path", &sparse_shuf, &sparse_target);

    // Absent targets only.
    check("all_absent", &[5, 1, 9, 3], &[100, 200, 300]);

    // Empty haystack / empty target.
    check("empty_haystack", &[], &[1, 2, 3]);
    check("empty_target", &[5, 1, 9], &[]);

    // i64 extremes (span overflow guard -> hashmap path).
    check(
        "i64_extremes",
        &[i64::MIN, 0, i64::MAX],
        &[i64::MAX, 7, i64::MIN],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
