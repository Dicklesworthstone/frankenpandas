//! No-mock conformance guard for the typed all-Utf8 `Index::value_counts` fast
//! path (tally &str keys, no per-label String clone). Checks the (label, count)
//! pairs AND their order against an independent oracle mirroring the generic
//! FxHashMap<IndexLabel> + stable sort_by_key semantics: counts descending,
//! first-seen order breaks ties.

use fp_index::{Index, IndexLabel};

fn idx(items: &[&str]) -> Index {
    Index::new(
        items
            .iter()
            .map(|s| IndexLabel::Utf8((*s).to_string()))
            .collect(),
    )
}

fn vc(i: &Index) -> Vec<(String, i64)> {
    i.value_counts()
        .into_iter()
        .map(|(l, c)| match l {
            IndexLabel::Utf8(s) => (s, c as i64),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

// Oracle: first-seen tally, then stable sort by count DESCENDING (default
// value_counts: sort=true, ascending=false). Stable keeps first-seen tie order.
fn oracle(items: &[&str]) -> Vec<(String, i64)> {
    let mut order: Vec<&str> = Vec::new();
    let mut counts: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for &s in items {
        let e = counts.entry(s).or_insert(0);
        if *e == 0 {
            order.push(s);
        }
        *e += 1;
    }
    let mut pairs: Vec<(String, i64)> = order.iter().map(|&s| (s.to_string(), counts[s])).collect();
    pairs.sort_by_key(|p| std::cmp::Reverse(p.1));
    pairs
}

fn check(items: &[&str]) {
    assert_eq!(vc(&idx(items)), oracle(items), "value_counts {items:?}");
}

#[test]
fn basic_counts_descending() {
    check(&["a", "b", "a", "c", "a", "b"]);
}

#[test]
fn tie_break_first_seen() {
    // all count 1 -> order must be first-seen: z, y, x
    check(&["z", "y", "x"]);
}

#[test]
fn single_value() {
    check(&["a", "a", "a", "a"]);
}

#[test]
fn empty() {
    check(&[]);
}

#[test]
fn larger_dup_heavy() {
    let v: Vec<String> = (0..2000).map(|i| format!("k{}", (i * 13) % 50)).collect();
    let r: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
    check(&r);
}
