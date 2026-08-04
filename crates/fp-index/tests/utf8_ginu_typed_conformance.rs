//! No-mock conformance guard for the typed all-Utf8 `Index::get_indexer_non_unique`
//! fast path (key the source position map on &str, no per-source-label String
//! clone). Checks (indexer, missing) against an independent oracle mirroring the
//! generic FxHashMap<IndexLabel,Vec<usize>> semantics.

use fp_index::{Index, IndexLabel};

fn idx(items: &[&str]) -> Index {
    Index::new(
        items
            .iter()
            .map(|s| IndexLabel::Utf8((*s).to_string()))
            .collect(),
    )
}

fn oracle(src: &[&str], tgt: &[&str]) -> (Vec<isize>, Vec<usize>) {
    let mut positions: std::collections::HashMap<&str, Vec<usize>> =
        std::collections::HashMap::new();
    for (p, &s) in src.iter().enumerate() {
        positions.entry(s).or_default().push(p);
    }
    let mut indexer = Vec::new();
    let mut missing = Vec::new();
    for (tp, &s) in tgt.iter().enumerate() {
        if let Some(ps) = positions.get(s) {
            indexer.extend(ps.iter().map(|&p| p as isize));
        } else {
            indexer.push(-1);
            missing.push(tp);
        }
    }
    (indexer, missing)
}

fn check(src: &[&str], tgt: &[&str]) {
    let got = idx(src).get_indexer_non_unique(&idx(tgt));
    let want = oracle(src, tgt);
    assert_eq!(got, want, "ginu src={src:?} tgt={tgt:?}");
}

#[test]
fn duplicates_with_missing() {
    check(&["a", "b", "a", "c", "a"], &["a", "c", "x"]);
}

#[test]
fn all_missing() {
    check(&["a", "b"], &["x", "y", "z"]);
}

#[test]
fn empty_target() {
    check(&["a", "b", "a"], &[]);
}

#[test]
fn empty_source() {
    check(&[], &["a", "b"]);
}

#[test]
fn position_order_preserved() {
    // duplicate "k" at 0,2,5 -> indexer must list positions in source order
    check(&["k", "m", "k", "n", "p", "k"], &["k", "n", "k"]);
}

#[test]
fn larger_dup_heavy() {
    let src: Vec<String> = (0..1500).map(|i| format!("s{}", (i * 11) % 200)).collect();
    let tgt: Vec<String> = (0..400).map(|i| format!("s{}", i)).collect();
    let sr: Vec<&str> = src.iter().map(|s| s.as_str()).collect();
    let tr: Vec<&str> = tgt.iter().map(|s| s.as_str()).collect();
    check(&sr, &tr);
}
