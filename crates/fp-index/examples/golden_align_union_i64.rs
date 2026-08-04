//! Differential golden for the typed-i64 `align_union` fast path vs an
//! independent reference (union = left ++ right-not-in-left; per-side
//! first-occurrence position vectors). Both inputs are unique (the precondition
//! align_union enforces). Run:
//!   cargo run -p fp-index --example golden_align_union_i64 --release

use std::collections::HashMap;

use fp_index::{Index, IndexLabel, align_union};

fn reference(l: &[i64], r: &[i64]) -> (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut lp: HashMap<i64, usize> = HashMap::new();
    for (i, &v) in l.iter().enumerate() {
        lp.entry(v).or_insert(i);
    }
    let mut rp: HashMap<i64, usize> = HashMap::new();
    for (i, &v) in r.iter().enumerate() {
        rp.entry(v).or_insert(i);
    }
    let mut u = l.to_vec();
    for &v in r {
        if !lp.contains_key(&v) {
            u.push(v);
        }
    }
    let lpos = u.iter().map(|v| lp.get(v).copied()).collect();
    let rpos = u.iter().map(|v| rp.get(v).copied()).collect();
    (u, lpos, rpos)
}

fn actual(l: &[i64], r: &[i64]) -> (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>) {
    let li = Index::new(l.iter().map(|&x| IndexLabel::Int64(x)).collect());
    let ri = Index::new(r.iter().map(|&x| IndexLabel::Int64(x)).collect());
    let plan = align_union(&li, &ri);
    let u: Vec<i64> = plan
        .union_index
        .labels()
        .iter()
        .map(|x| match x {
            IndexLabel::Int64(v) => *v,
            o => panic!("{o:?}"),
        })
        .collect();
    (u, plan.left_positions, plan.right_positions)
}

fn check(name: &str, l: &[i64], r: &[i64]) {
    let rf = reference(l, r);
    let ac = actual(l, r);
    if rf != ac {
        println!("FAIL {name}");
        if rf.0 != ac.0 {
            println!(
                "  union differs: ref.len={} act.len={}",
                rf.0.len(),
                ac.0.len()
            );
        }
        if rf.1 != ac.1 {
            println!("  left_positions differ");
        }
        if rf.2 != ac.2 {
            println!("  right_positions differ");
        }
        std::process::exit(1);
    }
    println!("OK   {name} (|union|={})", rf.0.len());
}

fn main() {
    let mut z = 0xbeef_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 30_000usize;

    let shuf = |base: Vec<i64>, rnd: &mut dyn FnMut(i64) -> i64| -> Vec<i64> {
        let mut v = base;
        for i in (1..v.len()).rev() {
            let j = rnd((i + 1) as i64) as usize;
            v.swap(i, j);
        }
        v
    };

    // Sorted, ~50% overlap.
    let ls: Vec<i64> = (0..n as i64).collect();
    let rs: Vec<i64> = (0..n as i64).map(|i| i + n as i64 / 2).collect();
    check("sorted_overlap", &ls, &rs);

    // Unsorted, ~50% overlap.
    let lu = shuf((0..n as i64).collect(), &mut rnd);
    let ru = shuf((0..n as i64).map(|i| i + n as i64 / 2).collect(), &mut rnd);
    check("unsorted_overlap", &lu, &ru);

    check("disjoint", &[1, 2, 3], &[4, 5, 6]);
    check("identical", &ls, &ls);
    check(
        "subset",
        &(0..100i64).collect::<Vec<_>>(),
        &(0..50i64).collect::<Vec<_>>(),
    );
    check(
        "negative",
        &shuf((-100..100i64).collect(), &mut rnd),
        &shuf((-50..150i64).collect(), &mut rnd),
    );
    check("empty_left", &[], &[1, 2, 3]);
    check("empty_right", &[1, 2, 3], &[]);
    check("both_empty", &[], &[]);
    check(
        "i64_extremes",
        &[i64::MIN, 0, i64::MAX],
        &[i64::MAX, 5, i64::MIN, 7],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
