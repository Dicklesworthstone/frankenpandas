//! Bench + golden digest for CategoricalIndex label->category-position ops.
//!
//! Run: cargo run -p fp-index --example bench_categorical_codes --release
//!
//! codes()/argmax()/argmin()/min()/max() (ordered) each map every label to its
//! category index via a linear `categories.position()` scan — O(n·k). One
//! precomputed category->index map makes them O(n+k).

use fp_index::CategoricalIndex;
use std::time::Instant;

fn golden() -> String {
    let mut out = String::new();
    // ordered categorical with an explicit (non-lexicographic) category order
    let cats = vec!["m".to_string(), "z".to_string(), "a".to_string(), "q".to_string()];
    let labels = vec![
        "z".to_string(), "a".to_string(), "m".to_string(), "q".to_string(),
        "a".to_string(), "z".to_string(), "m".to_string(),
    ];
    let ci = CategoricalIndex::with_categories(labels.clone(), cats.clone(), true).unwrap();
    out.push_str(&format!("codes={:?}\n", ci.codes()));
    out.push_str(&format!("argmax={:?} argmin={:?}\n", ci.argmax(), ci.argmin()));
    out.push_str(&format!("min={:?} max={:?}\n", ci.min(), ci.max()));

    // unordered (lexicographic) path unchanged
    let cu = CategoricalIndex::from_values(labels.clone(), false);
    out.push_str(&format!("u_codes={:?}\n", cu.codes()));
    out.push_str(&format!("u_argmax={:?} u_argmin={:?}\n", cu.argmax(), cu.argmin()));
    out.push_str(&format!("u_min={:?} u_max={:?}\n", cu.min(), cu.max()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let k: usize = 4000;
    let n: usize = 400_000;
    let cats: Vec<String> = (0..k).map(|i| format!("cat{i}")).collect();
    let labels: Vec<String> = (0..n).map(|i| format!("cat{}", (i * 7) % k)).collect();
    let ci = CategoricalIndex::with_categories(labels.clone(), cats.clone(), true).unwrap();

    let t = Instant::now();
    let codes = ci.codes();
    let d_codes = t.elapsed();
    assert_eq!(codes.len(), n);

    let t = Instant::now();
    let _ = ci.argmax().unwrap();
    let _ = ci.argmin().unwrap();
    let d_arg = t.elapsed();

    let t = Instant::now();
    let _ = ci.min();
    let _ = ci.max();
    let d_minmax = t.elapsed();

    println!(
        "TIMING n={n} k={k} codes={:.3}ms argmax+argmin={:.3}ms min+max={:.3}ms",
        d_codes.as_secs_f64() * 1e3,
        d_arg.as_secs_f64() * 1e3,
        d_minmax.as_secs_f64() * 1e3,
    );
}
