//! to_csv over a DataFrame with all-valid numeric columns + a NULLABLE Utf8 column.
//! Before the FastCol::UN arm, one null-bearing string column forced the WHOLE frame onto
//! the slow general (Scalar + csv-record) writer. NEW builds the nullable Utf8 column as a
//! LazyNullableUtf8 backing (→ fast typed writer); CONTROL builds it eagerly (→ general
//! writer) so the two are byte-identical but exercise different writers.
//!
//! Run: cargo run -p fp-io --release --example bench_to_csv_null_utf8 -- 1000000 6
use std::collections::BTreeMap;

use fp_columnar::{Column, ValidityMask};
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);

    // all-valid numeric columns
    let ids: Vec<i64> = (0..n as i64).collect();
    let vals: Vec<f64> = (0..n).map(|i| (i % 100000) as f64 / 100.0).collect();

    // nullable Utf8 "name": every 5th row missing, else item_{c}.
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut validity = ValidityMask::all_valid(n);
    let mut eager: Vec<Scalar> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 5 == 0 {
            validity.set(i, false);
            eager.push(Scalar::Null(fp_types::NullKind::Null));
        } else {
            let s = format!("item_{}", i % 5000);
            bytes.extend_from_slice(s.as_bytes());
            eager.push(Scalar::Utf8(s));
        }
        offsets.push(bytes.len());
    }

    let mk = |name_col: Column| -> DataFrame {
        let mut cols: BTreeMap<String, Column> = BTreeMap::new();
        cols.insert("id".to_string(), Column::from_i64_values_owned(ids.clone()));
        cols.insert("value".to_string(), Column::from_f64_values(vals.clone()));
        cols.insert("name".to_string(), name_col);
        DataFrame::new(Index::new_known_unique_int64_unit_range(0, n), cols).unwrap()
    };

    let frame_fast = mk(Column::from_utf8_values_with_validity(
        bytes.clone(),
        offsets.clone(),
        validity,
    ));
    let frame_general = mk(Column::from_values(eager).unwrap());

    // Confirm the control's name column is off the fast path (general writer).
    let ctl_name = frame_general.column("name").unwrap();
    assert!(
        ctl_name.as_utf8_contiguous().is_none() && ctl_name.as_nullable_utf8_contiguous().is_none(),
        "control name column must NOT be on the fast Utf8 path"
    );

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let s = fp_io::write_csv_string(&frame_fast).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(s.len());
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let s = fp_io::write_csv_string(&frame_general).unwrap();
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(s.len());
    }
    // Byte-for-byte parity.
    let sf = fp_io::write_csv_string(&frame_fast).unwrap();
    let sg = fp_io::write_csv_string(&frame_general).unwrap();
    assert_eq!(sf, sg, "fast (nullable-Utf8) CSV must byte-match the general writer");
    println!(
        "to_csv nullable-utf8 n={n} NEW(fast)={:>7.2}ms CONTROL(general)={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
