//! Bench + golden digest for Series.str.startswith_any / endswith_any.
//!
//! Run: cargo run -p fp-frame --example bench_startswith_any --release
//!
//! These ran an independent `s.starts_with(p)` / `s.ends_with(p)` for EVERY
//! pattern on EVERY string — O(|pats|·prefix) per element, worst when the
//! patterns share a long common prefix. One anchored literal-alternation regex
//! (`\A(?:…)` / `(?:…)\z`) matches each string once: O(prefix) per element.

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
    let s = s_from(vec!["https://a.com", "ftp://x", "file.txt", "", "a.b+c"]);

    let r = s.str().startswith_any(&["https://", "ftp://"]).unwrap();
    out.push_str(&format!("sw_hit={:?}\n", r.values()));
    // metacharacters stay literal
    let r2 = s.str().startswith_any(&["a.b+c"]).unwrap();
    out.push_str(&format!("sw_meta={:?}\n", r2.values()));
    let r3 = s.str().endswith_any(&[".txt", ".com"]).unwrap();
    out.push_str(&format!("ew_hit={:?}\n", r3.values()));
    // empty pattern set => all false
    let r4 = s.str().startswith_any(&[]).unwrap();
    out.push_str(&format!("empty={:?}\n", r4.values()));
    // empty-string pattern matches every non-null string
    let r5 = s.str().endswith_any(&["", "zz"]).unwrap();
    out.push_str(&format!("empty_pat={:?}\n", r5.values()));

    // with_na variants (null fill).
    let sn = Series::from_values(
        "s",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![
            Scalar::Utf8("https://z".into()),
            Scalar::Null(fp_types::NullKind::NaN),
        ],
    )
    .unwrap();
    let rn = sn
        .str()
        .startswith_any_with_na(&["https://"], Some(true))
        .unwrap();
    out.push_str(&format!("na_fill={:?}\n", rn.values()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Worst case: many prefixes sharing a long common stem.
    let pats_owned: Vec<String> = (0..200)
        .map(|i| format!("https://cdn.example.com/path/{i:04}/"))
        .collect();
    let pats: Vec<&str> = pats_owned.iter().map(String::as_str).collect();
    let one = "https://cdn.example.com/path/9999/asset/file/deep/name.bin";
    let n = 40_000;
    let s = s_from(vec![one; n]);

    let _ = s.str().startswith_any(&pats).unwrap(); // warmup

    let t = Instant::now();
    let r = s.str().startswith_any(&pats).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!(
        "TIMING n={n} npats={} startswith_any={:.3}ms",
        pats.len(),
        d.as_secs_f64() * 1e3
    );
}
