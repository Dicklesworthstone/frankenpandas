//! No-mock guard for the int64 key-offset SeriesGroupBy shift/diff fast path
//! (dense_groupby_{shift,diff}_f64_by_key). It must be bit-identical to the
//! gid-based dense path. Differential: group the SAME values by an i64 key
//! (by-key path) vs by the equivalent Utf8 key (the gid path, via
//! dense_group_ids' as_utf8_contiguous branch). Group numbering is irrelevant to
//! shift/diff (per-group, row order), so the outputs must match bit-for-bit.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn sm(i: usize) -> u64 {
    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn contig_utf8(keys: &[i64]) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    for &k in keys {
        bytes.extend_from_slice(format!("{k}").as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn bits(s: &Scalar) -> u64 {
    if s.is_missing() {
        return u64::MAX;
    }
    match s {
        Scalar::Float64(f) => f.to_bits(),
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn gb_shift_diff_int64_bykey_matches_utf8_gid() {
    let n = 50_000usize;
    for &g in &[1usize, 7, 1000] {
        let keys: Vec<i64> = (0..n).map(|i| (sm(i) % g as u64) as i64).collect();
        let vals: Vec<f64> = (0..n).map(|i| (sm(i + 1) % 100_000) as f64).collect();
        let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
        let value = Series::new(
            "v",
            Index::new(labels.clone()),
            Column::from_f64_values(vals),
        )
        .unwrap();
        let key_i64 = Series::new(
            "k",
            Index::new(labels.clone()),
            Column::from_i64_values(keys.clone()),
        )
        .unwrap();
        let key_str = Series::new("k", Index::new(labels), contig_utf8(&keys)).unwrap();

        for periods in [1usize, 2, 3] {
            let s_i = value
                .groupby(&key_i64)
                .unwrap()
                .shift(periods as i64)
                .unwrap();
            let s_u = value
                .groupby(&key_str)
                .unwrap()
                .shift(periods as i64)
                .unwrap();
            let d_i = value.groupby(&key_i64).unwrap().diff(periods).unwrap();
            let d_u = value.groupby(&key_str).unwrap().diff(periods).unwrap();
            for r in 0..n {
                assert_eq!(
                    bits(&s_i.values()[r]),
                    bits(&s_u.values()[r]),
                    "shift g={g} p={periods} r={r}"
                );
                assert_eq!(
                    bits(&d_i.values()[r]),
                    bits(&d_u.values()[r]),
                    "diff g={g} p={periods} r={r}"
                );
            }
        }
    }
}
