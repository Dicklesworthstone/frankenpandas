//! No-mock conformance guard for Index.union over Datetime64 / Timedelta64
//! indexes routed through the i64-keyed union_i64 fast path (instead of the
//! pointer-key FxHashMap<&IndexLabel> fallback). union's contract is
//! self-then-other FIRST-OCCURRENCE order (see utf8_setops_typed_conformance's
//! oracle_union), so the temporal fast path must reproduce exactly that order
//! and ns set, and carry the matching temporal dtype.

use fp_index::{Index, IndexLabel};

fn oracle_union_ns(a: &[i64], b: &[i64]) -> Vec<i64> {
    let mut b_seen = std::collections::HashSet::new();
    let b_is_unique = b.iter().all(|&s| b_seen.insert(s));
    if b_is_unique {
        let a_set: std::collections::HashSet<i64> = a.iter().copied().collect();
        let mut out: Vec<i64> = a.to_vec();
        for &s in b {
            if !a_set.contains(&s) {
                out.push(s);
            }
        }
        out
    } else {
        let mut counts: std::collections::HashMap<i64, (usize, usize)> =
            std::collections::HashMap::new();
        let mut order = Vec::new();
        for &s in a {
            let entry = counts.entry(s).or_insert((0, 0));
            if entry.0 == 0 && entry.1 == 0 {
                order.push(s);
            }
            entry.0 += 1;
        }
        for &s in b {
            let entry = counts.entry(s).or_insert((0, 0));
            if entry.0 == 0 && entry.1 == 0 {
                order.push(s);
            }
            entry.1 += 1;
        }
        let mut out = Vec::new();
        for s in order {
            let (ca, cb) = counts.get(&s).copied().unwrap_or((0, 0));
            for _ in 0..ca.max(cb) {
                out.push(s);
            }
        }
        out
    }
}

fn dt_labels(ns: &[i64]) -> Vec<IndexLabel> {
    ns.iter().map(|&v| IndexLabel::Datetime64(v)).collect()
}
fn td_labels(ns: &[i64]) -> Vec<IndexLabel> {
    ns.iter().map(|&v| IndexLabel::Timedelta64(v)).collect()
}

fn cases() -> Vec<(Vec<i64>, Vec<i64>)> {
    vec![
        // overlap with a dup within self and within other
        (vec![30, 10, 20, 10, 40], vec![10, 40, 99, 40]),
        // disjoint
        (vec![1, 2, 3], vec![7, 8, 9]),
        // reverse-sorted self, sorted other (proves NOT globally sorted output)
        (vec![40, 30, 20, 10], vec![10, 20, 30, 40]),
        // self all-dup
        (vec![5, 5, 5], vec![5, 6]),
        // larger pseudo-random
        (
            (0..500).map(|i| (i * 2654435761_i64) % 1000).collect(),
            (0..500).map(|i| (i * 40503_i64) % 1000).collect(),
        ),
    ]
}

#[test]
fn datetime64_union_matches_first_occurrence_oracle() {
    for (a, b) in cases() {
        let got = Index::from_datetime64(a.clone()).union(&Index::from_datetime64(b.clone()));
        let want = oracle_union_ns(&a, &b);
        assert_eq!(
            got.labels(),
            dt_labels(&want).as_slice(),
            "dt union {a:?} {b:?}"
        );
        // every output label is Datetime64
        assert!(
            got.labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Datetime64(_)))
        );
    }
}

#[test]
fn timedelta64_union_matches_first_occurrence_oracle() {
    for (a, b) in cases() {
        let got = Index::from_timedelta64(a.clone()).union(&Index::from_timedelta64(b.clone()));
        let want = oracle_union_ns(&a, &b);
        assert_eq!(
            got.labels(),
            td_labels(&want).as_slice(),
            "td union {a:?} {b:?}"
        );
        assert!(
            got.labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Timedelta64(_)))
        );
    }
}

#[test]
fn datetime64_union_empty_self_keeps_other() {
    // Empty self bails the temporal fast path (temporal_ns returns None for
    // empty) -> fallback path returns other's datetime labels unchanged.
    let got = Index::from_datetime64(vec![]).union(&Index::from_datetime64(vec![5, 6, 7]));
    assert_eq!(got.labels(), dt_labels(&[5, 6, 7]).as_slice());
}

#[test]
fn datetime64_union_other_empty_keeps_self() {
    let got = Index::from_datetime64(vec![5, 6, 7]).union(&Index::from_datetime64(vec![]));
    assert_eq!(got.labels(), dt_labels(&[5, 6, 7]).as_slice());
}
