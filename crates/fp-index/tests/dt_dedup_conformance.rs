//! No-mock conformance guard for the Datetime64/Timedelta64 dedup family
//! (nunique / unique / duplicated / drop_duplicates / value_counts) routed
//! through the i64 kernels. Differential against the ALREADY-TRUSTED Int64 path:
//! a temporal index and an Int64 index built from the SAME ns must agree on
//! every answer (modulo the label dtype). A NaT-bearing index must bail to the
//! generic fallback (and still match the pointer-key result).

use fp_index::{DuplicateKeep, Index, IndexLabel};

fn lab_ns(labels: &[IndexLabel]) -> Vec<i64> {
    labels
        .iter()
        .map(|l| match l {
            IndexLabel::Datetime64(v) | IndexLabel::Timedelta64(v) | IndexLabel::Int64(v) => *v,
            other => panic!("unexpected label {other:?}"),
        })
        .collect()
}

fn cases() -> Vec<Vec<i64>> {
    vec![
        vec![30, 10, 20, 10, 40, 20, 30],
        vec![5, 5, 5, 5],
        vec![1, 2, 3, 4, 5], // all distinct, unsorted-ish
        vec![9, 1, 9, 1, 9, 2],
        (0..400).map(|i| (i * 2654435761_i64) % 137).collect(),
    ]
}

#[test]
fn datetime64_dedup_family_matches_int64() {
    for ns in cases() {
        let dt = Index::from_datetime64(ns.clone());
        let i64i = Index::new(ns.iter().map(|&v| IndexLabel::Int64(v)).collect());

        assert_eq!(dt.nunique(), i64i.nunique(), "nunique {ns:?}");
        assert_eq!(
            lab_ns(dt.unique().labels()),
            lab_ns(i64i.unique().labels()),
            "unique {ns:?}"
        );
        for keep in [DuplicateKeep::First, DuplicateKeep::Last] {
            assert_eq!(
                dt.duplicated(keep),
                i64i.duplicated(keep),
                "duplicated {ns:?}"
            );
            assert_eq!(
                lab_ns(dt.drop_duplicates_keep(keep).labels()),
                lab_ns(i64i.drop_duplicates_keep(keep).labels()),
                "drop_dup {ns:?}"
            );
        }
        // value_counts: same (count) sequence; labels differ only in dtype.
        let vc_dt = dt.value_counts();
        let vc_i64 = i64i.value_counts();
        assert_eq!(vc_dt.len(), vc_i64.len(), "vc len {ns:?}");
        for ((ldt, cdt), (li64, ci64)) in vc_dt.iter().zip(vc_i64.iter()) {
            assert_eq!(cdt, ci64, "vc count {ns:?}");
            assert!(matches!(ldt, IndexLabel::Datetime64(_)), "vc dtype {ns:?}");
            let (vdt, vi) = (
                lab_ns(std::slice::from_ref(ldt))[0],
                lab_ns(std::slice::from_ref(li64))[0],
            );
            assert_eq!(vdt, vi, "vc value {ns:?}");
        }
        // output dtype carried
        assert!(
            dt.unique()
                .labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Datetime64(_)))
        );
    }
}

#[test]
fn timedelta64_dedup_family_matches_int64() {
    for ns in cases() {
        let td = Index::from_timedelta64(ns.clone());
        let i64i = Index::new(ns.iter().map(|&v| IndexLabel::Int64(v)).collect());
        assert_eq!(td.nunique(), i64i.nunique(), "td nunique {ns:?}");
        assert_eq!(
            lab_ns(td.unique().labels()),
            lab_ns(i64i.unique().labels()),
            "td unique {ns:?}"
        );
        assert_eq!(
            td.duplicated(DuplicateKeep::First),
            i64i.duplicated(DuplicateKeep::First)
        );
        assert!(
            td.unique()
                .labels()
                .iter()
                .all(|l| matches!(l, IndexLabel::Timedelta64(_)))
        );
    }
}

#[test]
fn datetime64_with_nat_bails_to_fallback() {
    // NaT (i64::MIN) present -> temporal fast path bails; the result must still be
    // correct (one NaT kept by unique, counted distinctly, etc.).
    let ns = vec![10_i64, i64::MIN, 20, 10, i64::MIN];
    let dt = Index::from_datetime64(ns);
    // unique keeps first-occurrence incl. the single NaT.
    let uniq = dt.unique();
    assert_eq!(uniq.labels().len(), 3, "10, NaT, 20");
    assert!(matches!(uniq.labels()[1], IndexLabel::Datetime64(i64::MIN)));
    // nunique(dropna=true default) excludes NaT -> 2 present distinct.
    assert_eq!(dt.nunique(), 2);
}
