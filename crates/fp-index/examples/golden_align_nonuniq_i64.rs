//! Differential golden for the typed-i64 `align_non_unique` fast path (all 4
//! modes) vs an independent reference, over DUPLICATE-containing Int64 indexes
//! (so the non-unique path is taken). Run:
//!   cargo run -p fp-index --example golden_align_nonuniq_i64 --release

use std::collections::HashMap;

use fp_index::{align, AlignMode, Index, IndexLabel};

fn mk(v: &[i64]) -> Index {
    Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect())
}
fn plan_tuple(p: fp_index::AlignmentPlan) -> (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>) {
    let u = p
        .union_index
        .labels()
        .iter()
        .map(|x| match x {
            IndexLabel::Int64(v) => *v,
            o => panic!("{o:?}"),
        })
        .collect();
    (u, p.left_positions, p.right_positions)
}

fn groups(v: &[i64]) -> HashMap<i64, Vec<usize>> {
    let mut g: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, &x) in v.iter().enumerate() {
        g.entry(x).or_default().push(i);
    }
    g
}

type Tup = (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>);

fn reference(l: &[i64], r: &[i64], mode: AlignMode) -> Tup {
    let lg = groups(l);
    let rg = groups(r);
    let (mut u, mut lp, mut rp) = (Vec::new(), Vec::new(), Vec::new());
    let mut push = |u: &mut Vec<i64>, lp: &mut Vec<Option<usize>>, rp: &mut Vec<Option<usize>>, v, a, b| {
        u.push(v);
        lp.push(a);
        rp.push(b);
    };
    match mode {
        AlignMode::Inner => {
            for (i, &v) in l.iter().enumerate() {
                if let Some(h) = rg.get(&v) {
                    for &j in h {
                        push(&mut u, &mut lp, &mut rp, v, Some(i), Some(j));
                    }
                }
            }
        }
        AlignMode::Left => {
            for (i, &v) in l.iter().enumerate() {
                match rg.get(&v) {
                    Some(h) if !h.is_empty() => {
                        for &j in h {
                            push(&mut u, &mut lp, &mut rp, v, Some(i), Some(j));
                        }
                    }
                    _ => push(&mut u, &mut lp, &mut rp, v, Some(i), None),
                }
            }
        }
        AlignMode::Right => {
            for (j, &v) in r.iter().enumerate() {
                match lg.get(&v) {
                    Some(h) if !h.is_empty() => {
                        for &i in h {
                            push(&mut u, &mut lp, &mut rp, v, Some(i), Some(j));
                        }
                    }
                    _ => push(&mut u, &mut lp, &mut rp, v, None, Some(j)),
                }
            }
        }
        AlignMode::Outer => {
            for (i, &v) in l.iter().enumerate() {
                match rg.get(&v) {
                    Some(h) if !h.is_empty() => {
                        for &j in h {
                            push(&mut u, &mut lp, &mut rp, v, Some(i), Some(j));
                        }
                    }
                    _ => push(&mut u, &mut lp, &mut rp, v, Some(i), None),
                }
            }
            for (j, &v) in r.iter().enumerate() {
                if !lg.contains_key(&v) {
                    push(&mut u, &mut lp, &mut rp, v, None, Some(j));
                }
            }
        }
    }
    (u, lp, rp)
}

fn check(name: &str, l: &[i64], r: &[i64]) {
    for mode in [AlignMode::Inner, AlignMode::Left, AlignMode::Right, AlignMode::Outer] {
        let rf = reference(l, r, mode);
        let ac = plan_tuple(align(&mk(l), &mk(r), mode));
        if rf != ac {
            println!("FAIL {name} mode={mode:?}: ref.len={} act.len={}", rf.0.len(), ac.0.len());
            std::process::exit(1);
        }
    }
    println!("OK   {name}");
}

fn main() {
    let mut z = 0xd00d_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 5_000usize;

    // Heavy duplicates both sides (cartesian explosion is real).
    let a: Vec<i64> = (0..n).map(|_| rnd(20)).collect();
    let b: Vec<i64> = (0..n).map(|_| rnd(20)).collect();
    check("heavy_dups_both", &a, &b);

    // Left dup, right unique-ish.
    let c: Vec<i64> = (0..n).map(|i| (i % 50) as i64).collect();
    let d: Vec<i64> = (0..n).map(|_| rnd(80)).collect();
    check("left_dups", &c, &d);

    // Negatives + dups.
    check("negatives", &(0..n).map(|_| rnd(30) - 15).collect::<Vec<_>>(), &(0..n).map(|_| rnd(30) - 15).collect::<Vec<_>>());

    // Sparse (unbounded span) with dups.
    check(
        "sparse_dups",
        &(0..n).map(|i| ((i % 10) as i64) * 1_000_003).collect::<Vec<_>>(),
        &(0..n).map(|i| ((i % 13) as i64) * 1_000_003).collect::<Vec<_>>(),
    );

    // Disjoint with dups.
    check("disjoint_dups", &[1, 1, 2, 2, 3], &[4, 4, 5, 5]);
    // One empty.
    check("empty_left", &[], &[1, 1, 2]);
    check("empty_right", &[1, 1, 2], &[]);
    check("i64_extremes", &[i64::MIN, i64::MIN, i64::MAX, 0], &[i64::MAX, i64::MAX, i64::MIN, 7]);

    println!("ALL GOLDEN CHECKS PASSED");
}
