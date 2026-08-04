//! No-mock conformance guard for Index intersection/difference/symmetric_difference
//! over UNSORTED Datetime64 / Timedelta64 indexes routed through the i64-keyed
//! membership_filter_i64 fast path (instead of the pointer-key FxHashMap). Cases
//! are deliberately UNSORTED so the sorted-merge two-pointer path bails and the
//! new temporal i64 path is exercised. Oracles mirror the first-occurrence
//! semantics of utf8_setops_typed_conformance.

use fp_index::{Index, IndexLabel};

fn oracle_intersection(a: &[i64], b: &[i64]) -> Vec<i64> {
    let bset: std::collections::HashSet<i64> = b.iter().copied().collect();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &v in a {
        if bset.contains(&v) && seen.insert(v) {
            out.push(v);
        }
    }
    out
}
fn oracle_difference(a: &[i64], b: &[i64]) -> Vec<i64> {
    let bset: std::collections::HashSet<i64> = b.iter().copied().collect();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &v in a {
        if !bset.contains(&v) && seen.insert(v) {
            out.push(v);
        }
    }
    out
}
fn oracle_symdiff(a: &[i64], b: &[i64]) -> Vec<i64> {
    let mut out = oracle_difference(a, b);
    out.extend(oracle_difference(b, a));
    out
}

// All UNSORTED so the sorted-merge path bails and the temporal i64 path fires.
fn cases() -> Vec<(Vec<i64>, Vec<i64>)> {
    vec![
        (vec![30, 10, 20, 10, 40], vec![10, 40, 99, 40]),
        (vec![40, 30, 20, 10], vec![10, 20, 30, 40]), // reverse-sorted self
        (vec![5, 3, 9, 1, 7], vec![2, 8, 4, 6]),      // disjoint, shuffled
        (vec![5, 5, 5, 2], vec![5, 2, 5]),            // dups both sides
        (
            (0..400).map(|i| (i * 2654435761_i64) % 777).collect(),
            (0..400).map(|i| (i * 40503_i64) % 777).collect(),
        ),
    ]
}

fn dt(ns: &[i64]) -> Vec<IndexLabel> {
    ns.iter().map(|&v| IndexLabel::Datetime64(v)).collect()
}
fn td(ns: &[i64]) -> Vec<IndexLabel> {
    ns.iter().map(|&v| IndexLabel::Timedelta64(v)).collect()
}

#[test]
fn datetime64_setops_match_oracle() {
    for (a, b) in cases() {
        let ia = Index::from_datetime64(a.clone());
        let ib = Index::from_datetime64(b.clone());
        assert_eq!(
            ia.intersection(&ib).labels(),
            dt(&oracle_intersection(&a, &b)).as_slice(),
            "dt inter {a:?} {b:?}"
        );
        assert_eq!(
            ia.difference(&ib).labels(),
            dt(&oracle_difference(&a, &b)).as_slice(),
            "dt diff {a:?} {b:?}"
        );
        assert_eq!(
            ia.symmetric_difference(&ib).labels(),
            dt(&oracle_symdiff(&a, &b)).as_slice(),
            "dt symdiff {a:?} {b:?}"
        );
        // dtype preserved
        assert!(
            ia.intersection(&ib)
                .labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Datetime64(_)))
        );
    }
}

#[test]
fn timedelta64_setops_match_oracle() {
    for (a, b) in cases() {
        let ia = Index::from_timedelta64(a.clone());
        let ib = Index::from_timedelta64(b.clone());
        assert_eq!(
            ia.intersection(&ib).labels(),
            td(&oracle_intersection(&a, &b)).as_slice(),
            "td inter {a:?} {b:?}"
        );
        assert_eq!(
            ia.difference(&ib).labels(),
            td(&oracle_difference(&a, &b)).as_slice(),
            "td diff {a:?} {b:?}"
        );
        assert_eq!(
            ia.symmetric_difference(&ib).labels(),
            td(&oracle_symdiff(&a, &b)).as_slice(),
            "td symdiff {a:?} {b:?}"
        );
        assert!(
            ia.difference(&ib)
                .labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Timedelta64(_)))
        );
    }
}
