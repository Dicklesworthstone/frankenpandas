//! No-mock conformance guard for the typed all-Utf8 `Index::intersection` fast
//! path (hash &str + "matched"-flag dedup in the other-map, no separate seen set).
//! Asserts self-order + first-occurrence dedup against an independent oracle that
//! mirrors the generic `FxHashMap<&IndexLabel>` semantics, across overlap,
//! disjoint, full-overlap, and duplicate-bearing cases.

use fp_index::{Index, IndexLabel};

fn utf8_index(items: &[&str]) -> Index {
    Index::new(
        items
            .iter()
            .map(|s| IndexLabel::Utf8((*s).to_string()))
            .collect(),
    )
}

fn labels(idx: &Index) -> Vec<String> {
    idx.labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Utf8(s) => s.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

// Oracle: self labels that appear in other, in self order, first occurrence only.
fn oracle(a: &[&str], b: &[&str]) -> Vec<String> {
    let bset: std::collections::HashSet<&str> = b.iter().copied().collect();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &s in a {
        if bset.contains(s) && seen.insert(s) {
            out.push(s.to_string());
        }
    }
    out
}

fn check(a: &[&str], b: &[&str]) {
    let got = labels(&utf8_index(a).intersection(&utf8_index(b)));
    let want = oracle(a, b);
    assert_eq!(got, want, "intersection({a:?}, {b:?})");
}

#[test]
fn overlap_with_duplicates() {
    // self has duplicate "a"; other has duplicate "c"
    check(&["b", "a", "c", "a", "d"], &["a", "c", "x", "c"]);
}

#[test]
fn disjoint() {
    check(&["a", "b", "c"], &["x", "y", "z"]);
}

#[test]
fn full_overlap_self_order() {
    // result must be in SELF order, not other order
    check(&["d", "c", "b", "a"], &["a", "b", "c", "d"]);
}

#[test]
fn self_all_duplicates() {
    check(&["a", "a", "a"], &["a", "b"]);
}

#[test]
fn empty_self() {
    check(&[], &["a", "b"]);
}

#[test]
fn larger_shuffled() {
    let a: Vec<String> = (0..500).map(|i| format!("k{}", (i * 7) % 300)).collect();
    let b: Vec<String> = (0..400).map(|i| format!("k{}", i * 2)).collect();
    let ar: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
    let br: Vec<&str> = b.iter().map(|s| s.as_str()).collect();
    check(&ar, &br);
}
