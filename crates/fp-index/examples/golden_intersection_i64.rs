//! Differential golden for the typed-i64 `Index::intersection` fast path:
//! proves it is bit-identical to an independent self-ordered first-occurrence
//! reference across dense/hashset membership and dedup sub-paths plus edge
//! cases. Run: cargo run -p fp-index --example golden_intersection_i64 --release

use std::collections::HashSet;

use fp_index::{Index, IndexLabel};

fn reference(a: &[i64], b: &[i64]) -> Vec<i64> {
    let bset: HashSet<i64> = b.iter().copied().collect();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut out = Vec::new();
    for &v in a {
        if bset.contains(&v) && seen.insert(v) {
            out.push(v);
        }
    }
    out
}

fn actual(a: &[i64], b: &[i64]) -> Vec<i64> {
    let ia = Index::new(a.iter().map(|&v| IndexLabel::Int64(v)).collect());
    let ib = Index::new(b.iter().map(|&v| IndexLabel::Int64(v)).collect());
    ia.intersection(&ib)
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(v) => *v,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn check(name: &str, a: &[i64], b: &[i64]) {
    let r = reference(a, b);
    let x = actual(a, b);
    if r == x {
        println!("OK   {name} (|out|={})", r.len());
    } else {
        println!("FAIL {name}: ref.len={} act.len={}", r.len(), x.len());
        for i in 0..r.len().max(x.len()) {
            if r.get(i) != x.get(i) {
                println!("  [{i}] ref={:?} act={:?}", r.get(i), x.get(i));
                break;
            }
        }
        std::process::exit(1);
    }
}

fn main() {
    let mut z = 0xfeed_face_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 30_000usize;

    let mut a: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        let j = rnd((i + 1) as i64) as usize;
        a.swap(i, j);
    }
    let b: Vec<i64> = (0..n as i64).map(|i| i * 2).collect();
    check("dense_shuffled", &a, &b);

    // Duplicates in self (dedup must keep first occurrence in self order).
    let adup: Vec<i64> = (0..n).map(|i| (i % 300) as i64).collect();
    check("self_duplicates", &adup, &(0i64..400).collect::<Vec<_>>());

    // Negative range.
    let aneg: Vec<i64> = (0..n).map(|_| rnd(2000) - 1000).collect();
    let bneg: Vec<i64> = (0..n).map(|_| rnd(2000) - 1000).collect();
    check("negative_range", &aneg, &bneg);

    // Sparse / unbounded -> hashset path for both.
    let asp: Vec<i64> = (0..n).map(|i| i as i64 * 1_000_003).collect();
    let bsp: Vec<i64> = (0..n).map(|i| i as i64 * 2_000_006).collect();
    check("sparse_hashset", &asp, &bsp);

    // Mixed: self dense bounded, other sparse unbounded.
    check("a_dense_b_sparse", &a, &bsp);
    // self sparse, other dense.
    check("a_sparse_b_dense", &asp, &b);

    check("disjoint", &[1, 2, 3], &[4, 5, 6]);
    check("empty_a", &[], &[1, 2, 3]);
    check("empty_b", &[1, 2, 3], &[]);
    check("i64_extremes", &[i64::MIN, 0, i64::MAX, 0], &[0, i64::MAX]);

    println!("ALL GOLDEN CHECKS PASSED");
}
