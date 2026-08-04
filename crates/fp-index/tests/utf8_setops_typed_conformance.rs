//! No-mock conformance guard for the typed all-Utf8 fast paths of
//! `Index::union_with` / `difference` / `symmetric_difference` (hash &str +
//! one-set / membership-set-doubles-as-dedup tricks). Each is checked against an
//! independent oracle mirroring the generic `FxHashMap<&IndexLabel>` semantics.

use std::collections::HashSet;

use fp_index::{Index, IndexLabel};

fn idx(items: &[&str]) -> Index {
    Index::new(
        items
            .iter()
            .map(|s| IndexLabel::Utf8((*s).to_string()))
            .collect(),
    )
}
fn labels(i: &Index) -> Vec<String> {
    i.labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Utf8(s) => s.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn oracle_union(a: &[&str], b: &[&str]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &s in a.iter().chain(b.iter()) {
        if seen.insert(s) {
            out.push(s.to_string());
        }
    }
    out
}
fn oracle_difference(a: &[&str], b: &[&str]) -> Vec<String> {
    let bset: HashSet<&str> = b.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &s in a {
        if !bset.contains(s) && seen.insert(s) {
            out.push(s.to_string());
        }
    }
    out
}
fn oracle_symdiff(a: &[&str], b: &[&str]) -> Vec<String> {
    let aset: HashSet<&str> = a.iter().copied().collect();
    let bset: HashSet<&str> = b.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &s in a {
        if !bset.contains(s) && seen.insert(s) {
            out.push(s.to_string());
        }
    }
    for &s in b {
        if !aset.contains(s) && seen.insert(s) {
            out.push(s.to_string());
        }
    }
    out
}

fn cases() -> Vec<(Vec<&'static str>, Vec<&'static str>)> {
    vec![
        (vec!["b", "a", "c", "a", "d"], vec!["a", "c", "x", "c"]),
        (vec!["a", "b", "c"], vec!["x", "y", "z"]),
        (vec!["d", "c", "b", "a"], vec!["a", "b", "c", "d"]),
        (vec!["a", "a", "a"], vec!["a", "b"]),
        (vec![], vec!["a", "b"]),
        (vec!["a", "b"], vec![]),
    ]
}

#[test]
fn union_typed_matches_oracle() {
    for (a, b) in cases() {
        assert_eq!(
            labels(&idx(&a).union(&idx(&b))),
            oracle_union(&a, &b),
            "union {a:?} {b:?}"
        );
    }
}

#[test]
fn difference_typed_matches_oracle() {
    for (a, b) in cases() {
        assert_eq!(
            labels(&idx(&a).difference(&idx(&b))),
            oracle_difference(&a, &b),
            "difference {a:?} {b:?}"
        );
    }
}

#[test]
fn symmetric_difference_typed_matches_oracle() {
    for (a, b) in cases() {
        assert_eq!(
            labels(&idx(&a).symmetric_difference(&idx(&b))),
            oracle_symdiff(&a, &b),
            "symdiff {a:?} {b:?}"
        );
    }
}

#[test]
fn larger_shuffled_all_three() {
    let a: Vec<String> = (0..600).map(|i| format!("k{}", (i * 7) % 350)).collect();
    let b: Vec<String> = (0..500).map(|i| format!("k{}", i * 2)).collect();
    let ar: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
    let br: Vec<&str> = b.iter().map(|s| s.as_str()).collect();
    assert_eq!(labels(&idx(&ar).union(&idx(&br))), oracle_union(&ar, &br));
    assert_eq!(
        labels(&idx(&ar).difference(&idx(&br))),
        oracle_difference(&ar, &br)
    );
    assert_eq!(
        labels(&idx(&ar).symmetric_difference(&idx(&br))),
        oracle_symdiff(&ar, &br)
    );
}
