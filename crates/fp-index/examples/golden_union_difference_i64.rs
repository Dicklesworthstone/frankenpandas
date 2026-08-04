//! Differential golden for the typed-i64 `Index::union_with` and
//! `Index::difference` fast paths vs independent references (self-then-other
//! first-occurrence union; self-order not-in-other difference) across
//! dense/hashset and edge cases.
//! Run: cargo run -p fp-index --example golden_union_difference_i64 --release

use std::collections::HashSet;

use fp_index::{Index, IndexLabel};

fn ref_union(a: &[i64], b: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &v in a.iter().chain(b.iter()) {
        if seen.insert(v) {
            out.push(v);
        }
    }
    out
}

fn ref_difference(a: &[i64], b: &[i64]) -> Vec<i64> {
    let bset: HashSet<i64> = b.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &v in a {
        if !bset.contains(&v) && seen.insert(v) {
            out.push(v);
        }
    }
    out
}

fn to_i64(idx: Index) -> Vec<i64> {
    idx.labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(v) => *v,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn mk(v: &[i64]) -> Index {
    Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect())
}

fn check(name: &str, a: &[i64], b: &[i64]) {
    let u_ref = ref_union(a, b);
    let u_act = to_i64(mk(a).union_with(&mk(b)));
    if u_ref != u_act {
        println!(
            "FAIL union {name}: ref.len={} act.len={}",
            u_ref.len(),
            u_act.len()
        );
        std::process::exit(1);
    }
    let d_ref = ref_difference(a, b);
    let d_act = to_i64(mk(a).difference(&mk(b)));
    if d_ref != d_act {
        println!(
            "FAIL difference {name}: ref.len={} act.len={}",
            d_ref.len(),
            d_act.len()
        );
        std::process::exit(1);
    }
    println!("OK   {name} (|u|={} |d|={})", u_ref.len(), d_ref.len());
}

fn main() {
    let mut z = 0xc0ffee_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 20_000usize;

    let mut a: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        let j = rnd((i + 1) as i64) as usize;
        a.swap(i, j);
    }
    let b: Vec<i64> = (0..n as i64).map(|i| i * 2).collect();
    check("dense_shuffled", &a, &b);

    let adup: Vec<i64> = (0..n).map(|i| (i % 300) as i64).collect();
    let bdup: Vec<i64> = (0..n).map(|i| (i % 250) as i64).collect();
    check("with_duplicates", &adup, &bdup);

    let aneg: Vec<i64> = (0..n).map(|_| rnd(2000) - 1000).collect();
    let bneg: Vec<i64> = (0..n).map(|_| rnd(2000) - 1000).collect();
    check("negative_range", &aneg, &bneg);

    let asp: Vec<i64> = (0..n).map(|i| i as i64 * 1_000_003).collect();
    let bsp: Vec<i64> = (0..n).map(|i| i as i64 * 2_000_006).collect();
    check("sparse_hashset", &asp, &bsp);

    check("a_dense_b_sparse", &a, &bsp);
    check("disjoint", &[1, 2, 3], &[4, 5, 6]);
    check("identical", &a, &a);
    check("empty_a", &[], &[1, 2, 3]);
    check("empty_b", &[1, 2, 3], &[]);
    check("both_empty", &[], &[]);
    check(
        "i64_extremes",
        &[i64::MIN, 0, i64::MAX, 0],
        &[0, i64::MAX, 7],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
