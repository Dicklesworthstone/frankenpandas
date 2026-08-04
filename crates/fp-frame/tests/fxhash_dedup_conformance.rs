//! No-mock conformance guard for the FxHash-over-SipHash vein (br-frankenpandas-g1de8/
//! 6vep3/0jdij): the hasher swap must be BIT-TRANSPARENT — same counts and same first-seen
//! ordering as the SipHash path. Asserts exact value_counts / unique / drop_duplicates /
//! factorize output on a Utf8 column (the general FxHashMap/FxHashSet path). Compiled via
//! `cargo check --tests`; full run batch-pending.

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn utf8_series(vals: &[&str]) -> Series {
    Series::from_values(
        "s",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter()
            .map(|s| Scalar::Utf8((*s).to_string()))
            .collect(),
    )
    .unwrap()
}

fn utf8_of(v: &Scalar) -> String {
    match v {
        Scalar::Utf8(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

fn utf8_label(l: &IndexLabel) -> String {
    match l {
        IndexLabel::Utf8(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

// ["b","a","b","c","a","b"] => b:3, a:2, c:1 (count-desc, first-seen tiebreak)
fn fixture() -> Series {
    utf8_series(&["b", "a", "b", "c", "a", "b"])
}

#[test]
fn value_counts_fxhash_exact_counts_and_order() {
    let vc = fixture().value_counts().unwrap();
    let labels: Vec<String> = vc.index().labels().iter().map(utf8_label).collect();
    let counts: Vec<i64> = vc
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect();
    assert_eq!(labels, vec!["b", "a", "c"]);
    assert_eq!(counts, vec![3, 2, 1]);
}

#[test]
fn unique_fxhash_first_seen_order() {
    let u: Vec<String> = fixture().unique().iter().map(utf8_of).collect();
    assert_eq!(u, vec!["b", "a", "c"]);
}

#[test]
fn drop_duplicates_fxhash_keeps_first_seen() {
    let dd = fixture().drop_duplicates().unwrap();
    let got: Vec<String> = dd.values().iter().map(utf8_of).collect();
    assert_eq!(got, vec!["b", "a", "c"]);
}

#[test]
fn factorize_fxhash_codes_and_uniques() {
    let (codes, uniques) = fixture().factorize().unwrap();
    let code_vals: Vec<i64> = codes
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect();
    let uniq_vals: Vec<String> = uniques.values().iter().map(utf8_of).collect();
    // first-seen codes: b=0, a=1, c=2
    assert_eq!(code_vals, vec![0, 1, 0, 2, 1, 0]);
    assert_eq!(uniq_vals, vec!["b", "a", "c"]);
}
