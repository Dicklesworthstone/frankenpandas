//! Differential golden for the typed-i64 `Index::value_counts(_with_options)`
//! path vs an independent first-seen + stable-count-sort reference.
//! Run: cargo run -p fp-index --example golden_value_counts_idx_i64 --release

use std::collections::HashMap;

use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn reference(v: &[i64], ascending: bool) -> Vec<(i64, i64)> {
    let mut seen = Vec::new();
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for &x in v {
        let c = counts.entry(x).or_insert(0);
        if *c == 0 {
            seen.push(x);
        }
        *c += 1;
    }
    let mut pairs: Vec<(i64, i64)> = seen.iter().map(|&x| (x, counts[&x] as i64)).collect();
    // Stable sort by count (asc or desc) — ties keep first-seen order.
    if ascending {
        pairs.sort_by_key(|p| p.1);
    } else {
        pairs.sort_by_key(|p| std::cmp::Reverse(p.1));
    }
    pairs
}

fn actual(v: &[i64], ascending: bool) -> Vec<(i64, i64)> {
    let idx = Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect());
    // value_counts_with_options(normalize=false, sort=true, ascending, dropna=true)
    idx.value_counts_with_options(false, true, ascending, true)
        .into_iter()
        .map(|(l, c)| {
            let lv = match l {
                IndexLabel::Int64(x) => x,
                o => panic!("{o:?}"),
            };
            let cv = match c {
                Scalar::Int64(x) => x,
                o => panic!("{o:?}"),
            };
            (lv, cv)
        })
        .collect()
}

fn check(name: &str, v: &[i64]) {
    for asc in [false, true] {
        let r = reference(v, asc);
        let a = actual(v, asc);
        if r != a {
            println!(
                "FAIL {name} (ascending={asc}): ref.len={} act.len={}",
                r.len(),
                a.len()
            );
            for i in 0..r.len().max(a.len()) {
                if r.get(i) != a.get(i) {
                    println!("  [{i}] ref={:?} act={:?}", r.get(i), a.get(i));
                    break;
                }
            }
            std::process::exit(1);
        }
    }
    println!("OK   {name}");
}

fn main() {
    let mut z = 0x99_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 30_000usize;

    // Skewed counts (ties + varied) so the stable sort order matters.
    let mut a: Vec<i64> = (0..n).map(|_| rnd(500)).collect();
    let _ = &mut a;
    check("skewed_dense", &a);
    check(
        "heavy_dups",
        &(0..n).map(|i| (i % 30) as i64).collect::<Vec<_>>(),
    );
    check("all_unique", &{
        let mut v: Vec<i64> = (0..n as i64).collect();
        for i in (1..n).rev() {
            let j = rnd((i + 1) as i64) as usize;
            v.swap(i, j);
        }
        v
    });
    check("all_same", &vec![5i64; n]);
    check(
        "negative",
        &(0..n).map(|_| rnd(200) - 100).collect::<Vec<_>>(),
    );
    check(
        "sparse",
        &(0..n)
            .map(|i| ((i as i64) % 1000) * 1_000_003)
            .collect::<Vec<_>>(),
    );
    check("empty", &[]);
    check("single", &[7]);
    check(
        "i64_extremes",
        &[i64::MIN, i64::MAX, 0, i64::MAX, i64::MIN, i64::MAX],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
