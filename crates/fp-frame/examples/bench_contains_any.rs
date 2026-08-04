//! Bench + golden digest for Series.str.contains_any(&[&str]).
//!
//! Run: cargo run -p fp-frame --example bench_contains_any --release
//!
//! contains_any ran an independent `s.contains(p)` for EVERY pattern on EVERY
//! string — O(|pats|·L) per element. Compiling the literal set into one regex
//! alternation (Aho-Corasick under the hood) scans each string once: O(L).
//! Boolean output is identical (contains any pattern as a substring).

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn s_from(strings: Vec<&str>) -> Series {
    let idx: Vec<IndexLabel> = (0..strings.len() as i64).map(IndexLabel::Int64).collect();
    let vals: Vec<Scalar> = strings
        .into_iter()
        .map(|x| Scalar::Utf8(x.to_string()))
        .collect();
    Series::from_values("s", idx, vals).unwrap()
}

fn golden() -> String {
    let mut out = String::new();
    let s = s_from(vec!["hello world", "foobar", "BAZ qux", "", "a.b+c"]);

    let r = s.str().contains_any(&["world", "qux"]).unwrap();
    out.push_str(&format!("hit={:?}\n", r.values()));

    // Regex metacharacters must be treated as LITERALS.
    let r2 = s.str().contains_any(&["a.b+c", "zz"]).unwrap();
    out.push_str(&format!("literal_meta={:?}\n", r2.values()));

    // Case sensitive (no match for lowercase 'baz').
    let r3 = s.str().contains_any(&["baz"]).unwrap();
    out.push_str(&format!("case={:?}\n", r3.values()));

    // Empty pattern set => all false.
    let r4 = s.str().contains_any(&[]).unwrap();
    out.push_str(&format!("empty={:?}\n", r4.values()));

    // Empty-string pattern matches every (non-null) string.
    let r5 = s.str().contains_any(&["", "zz"]).unwrap();
    out.push_str(&format!("empty_pat={:?}\n", r5.values()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Many patterns, mostly-missing, over many medium strings (worst case:
    // every pattern scanned per string in the naive path).
    let pats_owned: Vec<String> = (0..200).map(|i| format!("needle{i:04}xyz")).collect();
    let pats: Vec<&str> = pats_owned.iter().map(String::as_str).collect();
    let base = "the quick brown fox jumps over the lazy dog ".repeat(4);
    let n = 20_000;
    let s = s_from(vec![base.as_str(); n]);

    // warmup
    let _ = s.str().contains_any(&pats).unwrap();

    let t = Instant::now();
    let r = s.str().contains_any(&pats).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!(
        "TIMING n={n} npats={} contains_any={:.3}ms",
        pats.len(),
        d.as_secs_f64() * 1e3
    );
}
