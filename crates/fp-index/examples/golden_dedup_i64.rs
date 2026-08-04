//! Differential golden for typed-i64 `Index::unique` / `duplicated` (First /
//! Last / None) / `drop_duplicates` vs independent references.
//! Run: cargo run -p fp-index --example golden_dedup_i64 --release

use std::collections::HashMap;

use fp_index::{DuplicateKeep, Index, IndexLabel};

fn mk(v: &[i64]) -> Index {
    Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect())
}
fn to_i64(idx: Index) -> Vec<i64> {
    idx.labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(v) => *v,
            o => panic!("{o:?}"),
        })
        .collect()
}

fn ref_unique(v: &[i64]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    v.iter().copied().filter(|x| seen.insert(*x)).collect()
}
fn ref_dup(v: &[i64], keep: &str) -> Vec<bool> {
    let n = v.len();
    let mut r = vec![false; n];
    match keep {
        "first" => {
            let mut seen = std::collections::HashSet::new();
            for i in 0..n {
                if !seen.insert(v[i]) {
                    r[i] = true;
                }
            }
        }
        "last" => {
            let mut seen = std::collections::HashSet::new();
            for i in (0..n).rev() {
                if !seen.insert(v[i]) {
                    r[i] = true;
                }
            }
        }
        _ => {
            let mut c: HashMap<i64, usize> = HashMap::new();
            for &x in v {
                *c.entry(x).or_insert(0) += 1;
            }
            for i in 0..n {
                r[i] = c[&v[i]] > 1;
            }
        }
    }
    r
}

fn check(name: &str, v: &[i64]) {
    let idx = mk(v);
    if ref_unique(v) != to_i64(idx.unique()) {
        println!("FAIL unique {name}");
        std::process::exit(1);
    }
    for keep in ["first", "last", "none"] {
        let k = match keep {
            "first" => DuplicateKeep::First,
            "last" => DuplicateKeep::Last,
            _ => DuplicateKeep::None,
        };
        if ref_dup(v, keep) != idx.duplicated(k) {
            println!("FAIL duplicated({keep}) {name}");
            std::process::exit(1);
        }
    }
    // drop_duplicates (keep=first)
    let dd: Vec<i64> = to_i64(idx.drop_duplicates());
    if dd != ref_unique(v) {
        println!("FAIL drop_duplicates {name}");
        std::process::exit(1);
    }
    println!("OK   {name} (|uniq|={})", ref_unique(v).len());
}

fn main() {
    let mut z = 0x55aa_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 30_000usize;

    // ~10% dups, shuffled (dense bounded).
    let mut a: Vec<i64> = (0..n).map(|i| (i as i64) * 9 / 10).collect();
    for i in (1..n).rev() {
        let j = rnd((i + 1) as i64) as usize;
        a.swap(i, j);
    }
    check("dense_10pct_dups", &a);
    check("all_unique", &(0..n as i64).rev().collect::<Vec<_>>());
    check("all_same", &vec![7i64; n]);
    check(
        "heavy_dups",
        &(0..n).map(|i| (i % 50) as i64).collect::<Vec<_>>(),
    );
    check(
        "negative",
        &(0..n).map(|_| rnd(400) - 200).collect::<Vec<_>>(),
    );
    check("sparse", &{
        let mut v: Vec<i64> = (0..n).map(|i| i as i64 * 1_000_003).collect();
        for i in 0..n / 10 {
            v[i] = v[0];
        }
        for i in (1..n).rev() {
            let j = rnd((i + 1) as i64) as usize;
            v.swap(i, j);
        }
        v
    });
    check("empty", &[]);
    check("single", &[42]);
    check("i64_extremes", &[i64::MIN, i64::MAX, 0, i64::MIN, i64::MAX]);

    println!("ALL GOLDEN CHECKS PASSED");
}
