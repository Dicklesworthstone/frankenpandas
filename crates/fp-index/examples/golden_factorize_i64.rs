//! Differential golden for the typed-i64 `Index::factorize` fast path vs an
//! independent first-occurrence-code reference across dense/hashmap and edge
//! cases. Run: cargo run -p fp-index --example golden_factorize_i64 --release

use std::collections::HashMap;

use fp_index::{Index, IndexLabel};

fn reference(v: &[i64]) -> (Vec<isize>, Vec<i64>) {
    let mut map: HashMap<i64, isize> = HashMap::new();
    let mut uniques = Vec::new();
    let mut codes = Vec::new();
    for &x in v {
        if let Some(&c) = map.get(&x) {
            codes.push(c);
        } else {
            let c = uniques.len() as isize;
            map.insert(x, c);
            uniques.push(x);
            codes.push(c);
        }
    }
    (codes, uniques)
}

fn actual(v: &[i64]) -> (Vec<isize>, Vec<i64>) {
    let idx = Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect());
    let (codes, uniq) = idx.factorize();
    let u: Vec<i64> = uniq
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(x) => *x,
            o => panic!("{o:?}"),
        })
        .collect();
    (codes, u)
}

fn check(name: &str, v: &[i64]) {
    let r = reference(v);
    let a = actual(v);
    if r != a {
        println!("FAIL {name}");
        if r.0 != a.0 {
            println!("  codes differ");
        }
        if r.1 != a.1 {
            println!(
                "  uniques differ: ref.len={} act.len={}",
                r.1.len(),
                a.1.len()
            );
        }
        std::process::exit(1);
    }
    println!("OK   {name} (|uniq|={})", r.1.len());
}

fn main() {
    let mut z = 0x77_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 30_000usize;

    let mut a: Vec<i64> = (0..n).map(|i| (i as i64) * 9 / 10).collect();
    for i in (1..n).rev() {
        let j = rnd((i + 1) as i64) as usize;
        a.swap(i, j);
    }
    check("dense_10pct_dups", &a);
    check("all_unique_sorted", &(0..n as i64).collect::<Vec<_>>());
    check("all_unique_rev", &(0..n as i64).rev().collect::<Vec<_>>());
    check("all_same", &vec![3i64; n]);
    check(
        "heavy_dups",
        &(0..n).map(|i| (i % 40) as i64).collect::<Vec<_>>(),
    );
    check(
        "negative",
        &(0..n).map(|_| rnd(400) - 200).collect::<Vec<_>>(),
    );
    check(
        "sparse",
        &(0..n).map(|i| (i as i64) * 1_000_003).collect::<Vec<_>>(),
    );
    check("empty", &[]);
    check("single", &[42]);
    check("i64_extremes", &[i64::MIN, i64::MAX, 0, i64::MAX, i64::MIN]);

    println!("ALL GOLDEN CHECKS PASSED");
}
