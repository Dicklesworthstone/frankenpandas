//! No-mock conformance guard for the SeriesGroupBy Utf8-key dense moment path
//! (`group_moment_dense` extended from Int64-only to Int64/contiguous-Utf8 keys,
//! sister to `dense_group_var_std`). A CONTIGUOUS Utf8 key routes through the
//! dense `dense_group_ids` path; a SCALAR-BACKED Utf8 key with identical row
//! values bails (`as_utf8_contiguous` is None) to the generic `agg_values_scalar`
//! per-group Vec<Scalar> + nansem/nanskew/nankurt path. Both must produce
//! bit-identical group labels (first-seen order) and bit-identical values.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}

fn contig_utf8(n: usize, f: impl Fn(usize) -> String) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0usize);
    for i in 0..n {
        bytes.extend_from_slice(f(i).as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}

fn build(n: usize, gc: u64) -> (Series, Series, Series) {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let key_str = |i: usize| format!("g{:04}", sm(i, 0) % gc);
    // Dense: contiguous Utf8 key.
    let by_contig = Series::new("k", Index::new(labels.clone()), contig_utf8(n, key_str)).unwrap();
    // Generic: scalar-backed Utf8 key (same row values; bails the dense gate).
    let by_scalar = Series::from_values(
        "k",
        labels.clone(),
        (0..n).map(|i| Scalar::Utf8(key_str(i))).collect(),
    )
    .unwrap();
    let v = Series::new(
        "v",
        Index::new(labels),
        Column::from_f64_values((0..n).map(|i| (sm(i, 1) % 100_000) as f64).collect()),
    )
    .unwrap();
    (by_contig, by_scalar, v)
}

fn assert_series_eq(dense: &Series, generic: &Series) {
    assert_eq!(
        dense.index().labels(),
        generic.index().labels(),
        "group labels differ between dense and generic paths"
    );
    let d = dense.values();
    let g = generic.values();
    assert_eq!(d.len(), g.len(), "group count differs");
    for (i, (a, b)) in d.iter().zip(g.iter()).enumerate() {
        match (a, b) {
            (Scalar::Float64(x), Scalar::Float64(y)) => {
                assert_eq!(x.to_bits(), y.to_bits(), "value {i} differs: {x} vs {y}");
            }
            (Scalar::Null(_), Scalar::Null(_)) => {}
            _ => panic!("value {i} type/variant differs: {a:?} vs {b:?}"),
        }
    }
}

#[test]
fn sgb_utf8_sem_dense_equals_generic() {
    let (c, s, v) = build(50_000, 97);
    let dense = v.groupby(&c).unwrap().sem().unwrap();
    let generic = v.groupby(&s).unwrap().sem().unwrap();
    assert_series_eq(&dense, &generic);
}

#[test]
fn sgb_utf8_skew_dense_equals_generic() {
    let (c, s, v) = build(50_000, 97);
    let dense = v.groupby(&c).unwrap().skew().unwrap();
    let generic = v.groupby(&s).unwrap().skew().unwrap();
    assert_series_eq(&dense, &generic);
}

#[test]
fn sgb_utf8_kurt_dense_equals_generic() {
    let (c, s, v) = build(50_000, 97);
    let dense = v.groupby(&c).unwrap().kurt().unwrap();
    let generic = v.groupby(&s).unwrap().kurt().unwrap();
    assert_series_eq(&dense, &generic);
}

// Tiny groups exercise the n<2 / n<3 / n<4 NaN branches in first-seen order.
#[test]
fn sgb_utf8_small_groups_dense_equals_generic() {
    let (c, s, v) = build(31, 11);
    for op in ["sem", "skew", "kurt"] {
        let dense = match op {
            "sem" => v.groupby(&c).unwrap().sem().unwrap(),
            "skew" => v.groupby(&c).unwrap().skew().unwrap(),
            _ => v.groupby(&c).unwrap().kurt().unwrap(),
        };
        let generic = match op {
            "sem" => v.groupby(&s).unwrap().sem().unwrap(),
            "skew" => v.groupby(&s).unwrap().skew().unwrap(),
            _ => v.groupby(&s).unwrap().kurt().unwrap(),
        };
        assert_series_eq(&dense, &generic);
    }
}
