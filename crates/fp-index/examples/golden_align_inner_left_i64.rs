//! Differential golden for the typed-i64 `align_inner` / `align_left` fast paths
//! vs independent references. Run:
//!   cargo run -p fp-index --example golden_align_inner_left_i64 --release

use std::collections::HashMap;

use fp_index::{AlignMode, Index, IndexLabel, align};

fn mk(v: &[i64]) -> Index {
    Index::new(v.iter().map(|&x| IndexLabel::Int64(x)).collect())
}
fn plan_to_tuple(p: fp_index::AlignmentPlan) -> (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>) {
    let u: Vec<i64> = p
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

fn ref_inner(l: &[i64], r: &[i64]) -> (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut rp: HashMap<i64, usize> = HashMap::new();
    for (i, &v) in r.iter().enumerate() {
        rp.entry(v).or_insert(i);
    }
    let (mut u, mut lp, mut rpos) = (Vec::new(), Vec::new(), Vec::new());
    for (i, &v) in l.iter().enumerate() {
        if let Some(&j) = rp.get(&v) {
            u.push(v);
            lp.push(Some(i));
            rpos.push(Some(j));
        }
    }
    (u, lp, rpos)
}

fn ref_left(l: &[i64], r: &[i64]) -> (Vec<i64>, Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut rp: HashMap<i64, usize> = HashMap::new();
    for (i, &v) in r.iter().enumerate() {
        rp.entry(v).or_insert(i);
    }
    let u = l.to_vec();
    let lp = (0..l.len()).map(Some).collect();
    let rpos = l.iter().map(|v| rp.get(v).copied()).collect();
    (u, lp, rpos)
}

fn check(name: &str, l: &[i64], r: &[i64]) {
    let ai = plan_to_tuple(align(&mk(l), &mk(r), AlignMode::Inner));
    if ai != ref_inner(l, r) {
        println!("FAIL inner {name}");
        std::process::exit(1);
    }
    let al = plan_to_tuple(align(&mk(l), &mk(r), AlignMode::Left));
    if al != ref_left(l, r) {
        println!("FAIL left {name}");
        std::process::exit(1);
    }
    println!("OK   {name} (|inner|={})", ref_inner(l, r).0.len());
}

fn main() {
    let mut z = 0xfade_u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let shuf = |base: Vec<i64>, rnd: &mut dyn FnMut(i64) -> i64| -> Vec<i64> {
        let mut v = base;
        for i in (1..v.len()).rev() {
            let j = rnd((i + 1) as i64) as usize;
            v.swap(i, j);
        }
        v
    };
    let n = 30_000usize;

    let ls: Vec<i64> = (0..n as i64).collect();
    let rs: Vec<i64> = (0..n as i64).map(|i| i + n as i64 / 2).collect();
    check("sorted_overlap", &ls, &rs);
    check(
        "unsorted_overlap",
        &shuf((0..n as i64).collect(), &mut rnd),
        &shuf((0..n as i64).map(|i| i + n as i64 / 2).collect(), &mut rnd),
    );
    check("disjoint", &[1, 2, 3], &[4, 5, 6]);
    check("identical", &ls, &ls);
    check(
        "left_subset_right",
        &(0..50i64).collect::<Vec<_>>(),
        &(0..100i64).collect::<Vec<_>>(),
    );
    check(
        "right_subset_left",
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
    check(
        "i64_extremes",
        &[i64::MIN, 0, i64::MAX],
        &[i64::MAX, 5, i64::MIN],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
