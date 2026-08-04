//! No-mock conformance guard for Series.nunique over a Datetime64 VALUE column
//! routed through the inline-i64 FxHashSet path. Differential against an Int64
//! column with the SAME ns (distinct count is order-independent), plus a NaT
//! (i64::MIN) case that must bail to the generic dropna path.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn idx(n: usize) -> Index {
    Index::new((0..n as i64).map(IndexLabel::Int64).collect())
}

fn cases() -> Vec<Vec<i64>> {
    let base = 1_577_836_800_000_000_000i64;
    let step = 60_000_000_000i64;
    vec![
        vec![base + 3 * step, base, base + step, base, base + 3 * step],
        (0..2000)
            .map(|i| base + ((i * 2654435761_i64) % 137) * step)
            .collect(),
        (0..1000).map(|i| base + (i % 7) * step).collect(),
        (0..500).map(|i| base + i * step).collect(), // all distinct
    ]
}

#[test]
fn series_datetime_nunique_matches_distinct_count() {
    for ns in cases() {
        let n = ns.len();
        let dt = Series::new("s", idx(n), Column::from_datetime64_values(ns.clone())).unwrap();
        let distinct: std::collections::HashSet<i64> = ns.iter().copied().collect();
        assert_eq!(dt.nunique(), distinct.len(), "nunique {n}");
    }
}

#[test]
fn series_datetime_nunique_nat_bails_to_generic_dropna() {
    let base = 1_577_836_800_000_000_000i64;
    // i64::MIN is the Datetime64 NaT sentinel; dropna=true default excludes it.
    let ns = vec![base, i64::MIN, base + 60_000_000_000, base, i64::MIN];
    let dt = Series::new("s", idx(ns.len()), Column::from_datetime64_values(ns)).unwrap();
    assert_eq!(
        dt.nunique(),
        2,
        "two present distinct timestamps, NaT excluded"
    );
}

// keep=First first-occurrence oracles (dtype-agnostic over the raw ns).
fn oracle_duplicated(ns: &[i64]) -> Vec<bool> {
    let mut seen = std::collections::HashSet::new();
    ns.iter().map(|&v| !seen.insert(v)).collect()
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

#[test]
fn series_datetime_duplicated_matches_oracle() {
    for ns in cases() {
        let n = ns.len();
        let dt = Series::new("s", idx(n), Column::from_datetime64_values(ns.clone())).unwrap();
        let dup = dt.duplicated().unwrap();
        let got: Vec<bool> = dup
            .values()
            .iter()
            .map(|s| matches!(s, fp_types::Scalar::Bool(true)))
            .collect();
        assert_eq!(got, oracle_duplicated(&ns), "duplicated {n}");
    }
}

#[test]
fn series_datetime_unique_and_drop_duplicates_match_oracle() {
    for ns in cases() {
        let n = ns.len();
        let dt = Series::new("s", idx(n), Column::from_datetime64_values(ns.clone())).unwrap();
        let want = oracle_unique(&ns);
        // unique() -> Vec<Scalar::Datetime64> in first-occurrence order.
        let uniq: Vec<i64> = dt
            .unique()
            .iter()
            .map(|s| match s {
                fp_types::Scalar::Datetime64(v) => *v,
                other => panic!("unique dtype {other:?}"),
            })
            .collect();
        assert_eq!(uniq, want, "unique {n}");
        // drop_duplicates() keeps first-occurrence rows; value column is Datetime64.
        let dd = dt.drop_duplicates().unwrap();
        let dd_vals: Vec<i64> = dd
            .values()
            .iter()
            .map(|s| match s {
                fp_types::Scalar::Datetime64(v) => *v,
                other => panic!("drop_dup dtype {other:?}"),
            })
            .collect();
        assert_eq!(dd_vals, want, "drop_duplicates {n}");
    }
}
