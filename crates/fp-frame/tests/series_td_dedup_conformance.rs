//! No-mock conformance guard for Series nunique/duplicated/unique/drop_duplicates
//! over a Timedelta64 VALUE column routed through the inline-i64 FxHashSet paths
//! (siblings of the Datetime64 paths; Timedelta::NAT == i64::MIN). Oracles are
//! first-occurrence over the raw ns; a NaT case bails to the generic dropna path.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{DType, Scalar};

fn td_series(ns: &[i64]) -> Series {
    let labels: Vec<IndexLabel> = (0..ns.len() as i64).map(IndexLabel::Int64).collect();
    Series::new(
        "s",
        Index::new(labels),
        Column::new(
            DType::Timedelta64,
            ns.iter().map(|&v| Scalar::Timedelta64(v)).collect(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn oracle_unique(ns: &[i64]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &v in ns {
        if seen.insert(v) {
            out.push(v);
        }
    }
    out
}
fn oracle_duplicated(ns: &[i64]) -> Vec<bool> {
    let mut seen = std::collections::HashSet::new();
    ns.iter().map(|&v| !seen.insert(v)).collect()
}

fn cases() -> Vec<Vec<i64>> {
    vec![
        vec![30, 10, 20, 10, 40, 20, 30, 10],
        (0..2000).map(|i| (i * 2654435761_i64) % 137).collect(),
        (0..1000).map(|i| i % 7).collect(),
        (0..500).collect(), // all distinct
    ]
}

#[test]
fn series_timedelta_dedup_family_matches_oracle() {
    for ns in cases() {
        let s = td_series(&ns);
        let distinct: std::collections::HashSet<i64> = ns.iter().copied().collect();
        assert_eq!(s.nunique(), distinct.len(), "nunique {ns:?}");

        let dup: Vec<bool> = s
            .duplicated()
            .unwrap()
            .values()
            .iter()
            .map(|v| matches!(v, Scalar::Bool(true)))
            .collect();
        assert_eq!(dup, oracle_duplicated(&ns), "duplicated");

        let want = oracle_unique(&ns);
        let uniq: Vec<i64> = s
            .unique()
            .iter()
            .map(|v| match v {
                Scalar::Timedelta64(x) => *x,
                other => panic!("unique dtype {other:?}"),
            })
            .collect();
        assert_eq!(uniq, want, "unique");

        let dd: Vec<i64> = s
            .drop_duplicates()
            .unwrap()
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Timedelta64(x) => *x,
                other => panic!("drop_dup dtype {other:?}"),
            })
            .collect();
        assert_eq!(dd, want, "drop_duplicates");
    }
}

#[test]
fn series_timedelta_nat_bails_to_generic() {
    // Timedelta::NAT == i64::MIN; dropna default excludes it from nunique.
    let ns = vec![
        5_000_000_000,
        i64::MIN,
        9_000_000_000,
        5_000_000_000,
        i64::MIN,
    ];
    let s = td_series(&ns);
    assert_eq!(s.nunique(), 2, "two present distinct timedeltas");
}
