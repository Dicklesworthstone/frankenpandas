//! No-mock conformance guard for Datetime64/Timedelta64 Index argsort /
//! sort_values routed through argsort_i64. The oracle is a STABLE sort of the
//! positions by ns — which is exactly (a) what the old comparison-sort fallback
//! did (IndexLabel derives Ord, so Datetime64/Timedelta64 compare by inner i64,
//! NaT==i64::MIN sorting first) AND (b) what argsort_i64's stable sort_by_key
//! does. Duplicate-heavy cases verify tie stability; a NaT case verifies NaT
//! sorts first identically.

use fp_index::{Index, IndexLabel};

fn oracle_argsort(ns: &[i64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..ns.len()).collect();
    idx.sort_by_key(|&i| ns[i]); // stable
    idx
}

fn ns_of(labels: &[IndexLabel]) -> Vec<i64> {
    labels
        .iter()
        .map(|l| match l {
            IndexLabel::Datetime64(v) | IndexLabel::Timedelta64(v) => *v,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn cases() -> Vec<Vec<i64>> {
    vec![
        vec![30, 10, 20, 10, 40, 20, 30, 10], // many ties
        vec![5, 4, 3, 2, 1],                  // reverse
        (0..400).map(|i| (i * 2654435761_i64) % 50).collect(), // heavy dup (card 50)
        (0..400).map(|i| i * 7 - 1000).collect(), // distinct, includes negatives
        vec![i64::MIN, 10, 5, i64::MIN, 7],   // NaT (i64::MIN) sorts first, stable
    ]
}

#[test]
fn datetime64_argsort_and_sort_values_match_stable_oracle() {
    for ns in cases() {
        let idx = Index::from_datetime64(ns.clone());
        assert_eq!(idx.argsort(), oracle_argsort(&ns), "dt argsort {ns:?}");
        let sv = idx.sort_values();
        let want: Vec<i64> = oracle_argsort(&ns).iter().map(|&i| ns[i]).collect();
        assert_eq!(ns_of(sv.labels()), want, "dt sort_values {ns:?}");
        assert!(
            sv.labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Datetime64(_)))
        );
    }
}

#[test]
fn timedelta64_argsort_and_sort_values_match_stable_oracle() {
    for ns in cases() {
        let idx = Index::from_timedelta64(ns.clone());
        assert_eq!(idx.argsort(), oracle_argsort(&ns), "td argsort {ns:?}");
        let sv = idx.sort_values();
        let want: Vec<i64> = oracle_argsort(&ns).iter().map(|&i| ns[i]).collect();
        assert_eq!(ns_of(sv.labels()), want, "td sort_values {ns:?}");
        assert!(
            sv.labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Timedelta64(_)))
        );
    }
}

#[test]
fn datetime64_sort_matches_int64_path() {
    // Differential vs the trusted Int64 path over the same ns.
    let ns: Vec<i64> = (0..500).map(|i| (i * 40503_i64) % 60).collect();
    let dt = Index::from_datetime64(ns.clone());
    let i64i = Index::new(ns.iter().map(|&v| IndexLabel::Int64(v)).collect());
    assert_eq!(dt.argsort(), i64i.argsort());
    assert_eq!(ns_of(dt.sort_values().labels()), {
        let s = i64i.sort_values();
        s.labels()
            .iter()
            .map(|l| {
                if let IndexLabel::Int64(v) = l {
                    *v
                } else {
                    unreachable!()
                }
            })
            .collect::<Vec<_>>()
    });
}
