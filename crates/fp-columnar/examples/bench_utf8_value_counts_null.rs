//! Bench for `Column::value_counts` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`)
//! after adding the nullable dropna=true arm — tallies present byte spans directly, instead of
//! the generic loop that materializes `Vec<Scalar::Utf8>` (a heap `String` per row) + set_member_key.
//!
//! NEW = col.value_counts(). CONTROL = a replica of the generic tally (skip-missing +
//! FxHashMap over the cached values()) ⇒ conservative lower bound (control skips the String
//! materialization the cold generic path pays).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_value_counts_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};
use rustc_hash::FxHashMap;

// Faithful COLD generic replica: materialize Vec<Scalar::Utf8> from the backing (the
// per-row String alloc the generic path pays on a cold column) THEN tally. This is what
// `value_counts` cost BEFORE the typed arm; comparing against the pre-warmed Vec would be
// dishonest (it skips the materialization, which is the whole point of the typed bypass).
fn ref_value_counts_cold(bytes: &[u8], offsets: &[usize], present: &[bool]) -> (Column, Column) {
    // (1) materialize present rows into owned Scalar::Utf8 (heap String per row).
    let mut vals: Vec<Scalar> = Vec::with_capacity(offsets.len() - 1);
    for (i, w) in offsets.windows(2).enumerate() {
        if present[i] {
            let s = std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string();
            vals.push(Scalar::Utf8(s));
        }
    }
    // (2) tally over the materialized strings (first-seen order, sort desc by count).
    let mut index: FxHashMap<&str, usize> = FxHashMap::default();
    let mut tally: Vec<(&str, usize)> = Vec::new();
    for v in &vals {
        if let Scalar::Utf8(s) = v {
            if let Some(&j) = index.get(s.as_str()) {
                tally[j].1 += 1;
            } else {
                index.insert(s.as_str(), tally.len());
                tally.push((s.as_str(), 1));
            }
        }
    }
    tally.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let values: Vec<Scalar> = tally.iter().map(|(s, _)| Scalar::Utf8((*s).to_string())).collect();
    let counts: Vec<Scalar> = tally.iter().map(|(_, c)| Scalar::Int64(*c as i64)).collect();
    (
        Column::new(DType::Utf8, values).unwrap(),
        Column::new(DType::Int64, counts).unwrap(),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // ~1000 distinct categories, every 5th row missing.
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut validity = ValidityMask::all_valid(n);
    let mut present: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 5 == 0 {
            validity.set(i, false);
            present.push(false);
        } else {
            bytes.extend_from_slice(format!("cat_{:06}", i % 1000).as_bytes());
            present.push(true);
        }
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_values_with_validity(bytes.clone(), offsets.clone(), validity);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.value_counts().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_value_counts_cold(&bytes, &offsets, &present);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    // Parity: same (value -> count) mapping. Both sorted desc by count; compare as maps.
    let (gv, gc) = col.value_counts().unwrap();
    let (wv, wc) = ref_value_counts_cold(&bytes, &offsets, &present);
    assert_eq!(gv.len(), wv.len(), "distinct count differs");
    let gvv = gv.values();
    let gcv = gc.values();
    let mut got_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for k in 0..gv.len() {
        if let (Scalar::Utf8(s), Scalar::Int64(c)) = (&gvv[k], &gcv[k]) {
            got_map.insert(s.clone(), *c);
        }
    }
    let wvv = wv.values();
    let wcv = wc.values();
    for k in 0..wv.len() {
        if let (Scalar::Utf8(s), Scalar::Int64(c)) = (&wvv[k], &wcv[k]) {
            assert_eq!(got_map.get(s), Some(c), "count mismatch for {s}");
        }
    }
    println!(
        "value_counts utf8_nullable n={n} distinct={} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        gv.len(),
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
