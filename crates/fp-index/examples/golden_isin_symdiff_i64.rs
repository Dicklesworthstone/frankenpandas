//! Differential golden for the typed-i64 `Index::isin` and
//! `Index::symmetric_difference` fast paths vs independent references.
//! Run: cargo run -p fp-index --example golden_isin_symdiff_i64 --release

use std::collections::HashSet;

use fp_index::{Index, IndexLabel};

fn mk(v: &[i64]) -> Index {
    Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect())
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

fn ref_symdiff(a: &[i64], b: &[i64]) -> Vec<i64> {
    let aset: HashSet<i64> = a.iter().copied().collect();
    let bset: HashSet<i64> = b.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &v in a {
        if !bset.contains(&v) && seen.insert(v) {
            out.push(v);
        }
    }
    for &v in b {
        if !aset.contains(&v) && seen.insert(v) {
            out.push(v);
        }
    }
    out
}

fn check_symdiff(name: &str, a: &[i64], b: &[i64]) {
    let r = ref_symdiff(a, b);
    let x = to_i64(mk(a).symmetric_difference(&mk(b)));
    if r != x {
        println!(
            "FAIL symdiff {name}: ref.len={} act.len={}",
            r.len(),
            x.len()
        );
        std::process::exit(1);
    }
    println!("OK   symdiff {name} (|out|={})", r.len());
}

fn check_isin(name: &str, hay: &[i64], needles: &[IndexLabel]) {
    // Reference: original semantics — a self label is "in" the value set.
    let set: HashSet<&IndexLabel> = needles.iter().collect();
    let r: Vec<bool> = hay
        .iter()
        .map(|&v| set.contains(&IndexLabel::Int64(v)))
        .collect();
    let x = mk(hay).isin(needles);
    if r != x {
        let diff = r.iter().zip(x.iter()).filter(|(a, b)| a != b).count();
        println!("FAIL isin {name}: {diff} differing of {}", r.len());
        std::process::exit(1);
    }
    println!(
        "OK   isin {name} (matched={})",
        r.iter().filter(|b| **b).count()
    );
}

fn main() {
    let mut z = 0xabcdef_u64;
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

    check_symdiff("dense_shuffled", &a, &b);
    check_symdiff(
        "with_duplicates",
        &(0..n).map(|i| (i % 300) as i64).collect::<Vec<_>>(),
        &(0..n).map(|i| (i % 250) as i64).collect::<Vec<_>>(),
    );
    check_symdiff(
        "negative",
        &(0..n).map(|_| rnd(2000) - 1000).collect::<Vec<_>>(),
        &(0..n).map(|_| rnd(2000) - 1000).collect::<Vec<_>>(),
    );
    check_symdiff(
        "sparse",
        &(0..n).map(|i| i as i64 * 1_000_003).collect::<Vec<_>>(),
        &(0..n).map(|i| i as i64 * 2_000_006).collect::<Vec<_>>(),
    );
    check_symdiff("disjoint", &[1, 2, 3], &[4, 5, 6]);
    check_symdiff("identical", &a, &a);
    check_symdiff("empty_a", &[], &[1, 2, 3]);
    check_symdiff("empty_b", &[1, 2, 3], &[]);
    check_symdiff("i64_extremes", &[i64::MIN, 0, i64::MAX], &[0, i64::MAX, 7]);

    // isin: dense needles.
    let needles_dense: Vec<IndexLabel> = b.iter().map(|&v| IndexLabel::Int64(v)).collect();
    check_isin("dense_needles", &a, &needles_dense);
    // isin: sparse needles -> hashset path.
    let needles_sparse: Vec<IndexLabel> = (0..n as i64)
        .map(|i| IndexLabel::Int64(i * 2_000_006))
        .collect();
    check_isin("sparse_needles", &a, &needles_sparse);
    // isin: MIXED needles (some non-Int64 must be ignored, never match).
    let mixed: Vec<IndexLabel> = vec![
        IndexLabel::Int64(5),
        IndexLabel::Utf8("nope".into()),
        IndexLabel::Int64(17),
        IndexLabel::Bool(true),
        IndexLabel::Int64(99999),
    ];
    check_isin("mixed_needles", &a, &mixed);
    check_isin("empty_needles", &a, &[]);
    check_isin(
        "negatives",
        &[-5, 0, 5, -1000, 1000],
        &[IndexLabel::Int64(-5), IndexLabel::Int64(1000)],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
