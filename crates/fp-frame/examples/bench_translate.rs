//! Bench + golden digest for Series.str.translate(from, to).
//!
//! Run: cargo run -p fp-frame --example bench_translate --release
//!
//! translate scanned the `from` table linearly (O(|from|)) for EVERY input
//! char — O(total_chars · |from|). A char->replacement map built once makes it
//! O(total_chars). First occurrence in `from` wins (same as the linear scan);
//! a source char beyond `to`'s length is deleted; unmapped chars pass through.

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
    let s = s_from(vec!["hello world", "abcXYZ", "", "duplicate-d"]);

    // basic 1:1 replacement
    let r = s.str().translate("lo", "LO").unwrap();
    out.push_str(&format!("basic={:?}\n", r.values()));

    // `to` shorter than `from` => extra source chars are DELETED. 'o'->'0',
    // but 'l' (index 1) has no target so every 'l' is removed.
    let r2 = s.str().translate("ol", "0").unwrap();
    out.push_str(&format!("delete={:?}\n", r2.values()));

    // duplicate source char in `from`: first occurrence wins ('d'->'1', not '2')
    let r3 = s.str().translate("dd", "12").unwrap();
    out.push_str(&format!("dupsrc={:?}\n", r3.values()));

    // empty table: identity
    let r4 = s.str().translate("", "").unwrap();
    out.push_str(&format!("empty={:?}\n", r4.values()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Large translation table + many long strings.
    let from: String = (0u32..2000).filter_map(char::from_u32).collect();
    let to: String = (1000u32..3000).filter_map(char::from_u32).collect();
    // Strings made of chars near the END of the `from` table: the linear scan
    // must traverse ~all of `from` per char (the realistic O(|from|) cost).
    let base: String = (1800u32..2000).filter_map(char::from_u32).collect();
    let one = base.repeat(20); // ~4000 chars/string
    let n = 2_000;
    let s = s_from(vec![one.as_str(); n]);

    // warmup
    let _ = s.str().translate(&from, &to).unwrap();

    let t = Instant::now();
    let r = s.str().translate(&from, &to).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!(
        "TIMING n={n} from_len={} translate={:.3}ms",
        from.chars().count(),
        d.as_secs_f64() * 1e3
    );
}
