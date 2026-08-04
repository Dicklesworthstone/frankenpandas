//! Bench + golden digest for CategoricalIndex set-ops / append category union.
//!
//! Run: cargo run -p fp-index --example bench_cat_setop --release
//!
//! union/intersection/difference/symmetric_difference (via set_op_via_string)
//! and append rebuilt the result category list with an O(k) `Vec::contains`
//! per label — O(n·k) for high-cardinality categoricals. A seen-set makes it
//! O(n+k). First-seen category order and dedup are preserved exactly.

use std::time::Instant;

use fp_index::CategoricalIndex;

fn ci(labels: &[&str], ordered: bool) -> CategoricalIndex {
    CategoricalIndex::from_values(labels.iter().map(|s| s.to_string()).collect(), ordered)
}

fn golden() -> String {
    let mut out = String::new();
    let a = ci(&["b", "a", "b", "c", "a", "d"], false); // cats: b,a,c,d
    let b = ci(&["c", "e", "a", "f", "e"], false); // cats: c,e,a,f

    let u = a.union(&b);
    out.push_str(&format!("union_cats={:?}\n", u.categories()));
    out.push_str(&format!("union_labels={:?}\n", u.labels()));

    let i = a.intersection(&b);
    out.push_str(&format!("inter_cats={:?}\n", i.categories()));
    out.push_str(&format!("inter_labels={:?}\n", i.labels()));

    let d = a.difference(&b);
    out.push_str(&format!("diff_cats={:?}\n", d.categories()));
    out.push_str(&format!("diff_labels={:?}\n", d.labels()));

    let s = a.symmetric_difference(&b);
    out.push_str(&format!("symdiff_cats={:?}\n", s.categories()));
    out.push_str(&format!("symdiff_labels={:?}\n", s.labels()));

    let ap = a.append(&b);
    out.push_str(&format!("append_cats={:?}\n", ap.categories()));
    out.push_str(&format!("append_labels={:?}\n", ap.labels()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // High-cardinality: two disjoint-then-overlapping category universes.
    let n: usize = 8_000;
    let a_labels: Vec<String> = (0..n).map(|i| format!("a{i:07}")).collect();
    let b_labels: Vec<String> = (0..n).map(|i| format!("b{i:07}")).collect();
    let a = CategoricalIndex::from_values(a_labels, false);
    let b = CategoricalIndex::from_values(b_labels, false);

    // warmup
    let _ = a.union(&b);

    let t = Instant::now();
    let u = a.union(&b);
    let d = t.elapsed();
    assert_eq!(u.categories().len(), 2 * n);

    println!("TIMING n={n} cat_union={:.3}ms", d.as_secs_f64() * 1e3);
}
