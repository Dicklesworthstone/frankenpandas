//! No-mock conformance guard for DataFrame.loc[[Timedelta64 labels]] routed
//! through the identity-cached `unique_timedelta64_positions` batch resolver
//! (the deferred mirror of the Datetime64 path). A UNIQUE Timedelta64 index
//! hits the cached ns->position fast path; a DUPLICATE Timedelta64 index must
//! fall through to the duplicate-aware pointer-key map (returning every match
//! in ascending position). A missing label fails closed. Cross-checked against
//! the same logical selection over an Int64 index (already-trusted path).

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn df_with_index(labels: Vec<IndexLabel>, a: Vec<i64>) -> DataFrame {
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(a.into_iter().map(Scalar::Int64).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(Index::new(labels), cols, vec!["a".into()]).unwrap()
}

#[test]
fn td_loc_unique_fast_path_matches_int64() {
    let ns = [10_i64, 20, 30, 40, 50];
    let a = vec![100_i64, 200, 300, 400, 500];
    let td = df_with_index(
        ns.iter().map(|&v| IndexLabel::Timedelta64(v)).collect(),
        a.clone(),
    );
    let i64df = df_with_index(ns.iter().map(|&v| IndexLabel::Int64(v)).collect(), a);

    let sel_ns = [40_i64, 10, 50, 20];
    let td_sel: Vec<IndexLabel> = sel_ns.iter().map(|&v| IndexLabel::Timedelta64(v)).collect();
    let i64_sel: Vec<IndexLabel> = sel_ns.iter().map(|&v| IndexLabel::Int64(v)).collect();

    let r_td = td.loc(&td_sel).unwrap();
    let r_i64 = i64df.loc(&i64_sel).unwrap();

    // Same selection order, same payload values; labels carry the matching dtype.
    assert_eq!(
        r_td.columns().get("a").unwrap().values(),
        r_i64.columns().get("a").unwrap().values(),
        "td-index loc payload must equal int64-index loc payload"
    );
    assert_eq!(
        r_td.index().labels(),
        &sel_ns
            .iter()
            .map(|&v| IndexLabel::Timedelta64(v))
            .collect::<Vec<_>>()
    );
}

#[test]
fn td_loc_duplicate_index_returns_all_matches() {
    // Duplicate label 10 at positions 0 and 2 -> has_duplicates() true -> the
    // fast path bails and the pointer-key map returns both, ascending position.
    let td = df_with_index(
        vec![
            IndexLabel::Timedelta64(10),
            IndexLabel::Timedelta64(20),
            IndexLabel::Timedelta64(10),
            IndexLabel::Timedelta64(30),
        ],
        vec![1, 2, 3, 4],
    );
    let r = td.loc(&[IndexLabel::Timedelta64(10)]).unwrap();
    assert_eq!(
        r.columns().get("a").unwrap().values(),
        vec![Scalar::Int64(1), Scalar::Int64(3)],
        "duplicate td label must return all matches in ascending position"
    );
}

#[test]
fn td_loc_missing_fails_closed() {
    let td = df_with_index(
        vec![IndexLabel::Timedelta64(10), IndexLabel::Timedelta64(20)],
        vec![1, 2],
    );
    assert!(td.loc(&[IndexLabel::Timedelta64(99)]).is_err());
}

#[test]
fn td_loc_warm_cache_repeated_calls_stable() {
    // Repeated loc against the same index lineage exercises the warm cache;
    // every call must return the identical result (cache returns the same Arc).
    let ns: Vec<i64> = (0..200).map(|i| i * 7 + 3).collect();
    let td = df_with_index(
        ns.iter().map(|&v| IndexLabel::Timedelta64(v)).collect(),
        ns.clone(),
    );
    // All present: ns[i] = i*7+3 -> 3 (i=0), 703 (i=100), 353 (i=50), 10 (i=1).
    let sel: Vec<IndexLabel> = [3_i64, 703, 353, 3, 10]
        .iter()
        .map(|&v| IndexLabel::Timedelta64(v))
        .collect();
    let first = td.loc(&sel).unwrap();
    for _ in 0..3 {
        let again = td.loc(&sel).unwrap();
        assert_eq!(
            again.columns().get("a").unwrap().values(),
            first.columns().get("a").unwrap().values()
        );
        assert_eq!(again.index().labels(), first.index().labels());
    }
}
