//! No-mock conformance guard for the multi-Utf8-key dense groupby bypass
//! (multi_utf8_dense_grouping in the moments engine). The dense path fires only
//! for CONTIGUOUS Utf8 key columns; a Scalar-backed column (from_values) bails
//! as_utf8_contiguous and flows through the generic build_groups path. So the
//! same data built both ways must yield byte-identical results — a direct
//! dense-vs-generic bit-identity check — and both must match pandas 2.2.3.

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

// pandas groupby(['k1','k2']) on k1=[b,a,b,a,c] k2=[y,x,x,x,x] v=[1,2,3,4,5]:
//   order (a,x),(b,x),(b,y),(c,x); sum [6,3,1,5]; mean [3,3,1,5]; count [2,1,1,1]
const K1: [&str; 5] = ["b", "a", "b", "a", "c"];
const K2: [&str; 5] = ["y", "x", "x", "x", "x"];
const V: [f64; 5] = [1.0, 2.0, 3.0, 4.0, 5.0];

fn contig(keys: &[&str]) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    for s in keys {
        bytes.extend_from_slice(s.as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn scalar(keys: &[&str]) -> Column {
    Column::from_values(keys.iter().map(|s| Scalar::Utf8((*s).to_string())).collect()).unwrap()
}

fn frame(k1: Column, k2: Column) -> DataFrame {
    let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
    let mut cols = std::collections::BTreeMap::new();
    cols.insert("k1".to_string(), k1);
    cols.insert("k2".to_string(), k2);
    cols.insert("v".to_string(), Column::from_values(V.iter().map(|x| Scalar::Float64(*x)).collect()).unwrap());
    DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v".into()]).unwrap()
}

fn vals(df: &DataFrame) -> Vec<f64> {
    df.column("v")
        .unwrap()
        .values()
        .iter()
        .map(|s| match s {
            Scalar::Float64(x) => *x,
            Scalar::Int64(x) => *x as f64,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}
fn idx_labels(df: &DataFrame) -> Vec<String> {
    df.index().labels().iter().map(|l| l.to_string()).collect()
}

fn run(op: &str, df: &DataFrame) -> DataFrame {
    let gb = df.groupby(&["k1", "k2"]).unwrap();
    match op {
        "sum" => gb.sum().unwrap(),
        "mean" => gb.mean().unwrap(),
        "count" => gb.count().unwrap(),
        "min" => gb.min().unwrap(),
        "max" => gb.max().unwrap(),
        "median" => gb.median().unwrap(),
        "idxmax" => gb.idxmax().unwrap(),
        "nunique" => gb.nunique().unwrap(),
        _ => unreachable!(),
    }
}

// Reads "v" as display strings — works for Float64 / Int64 / Utf8 (idxmax) results.
fn cells(df: &DataFrame) -> Vec<String> {
    df.column("v")
        .unwrap()
        .values()
        .iter()
        .map(|s| match s {
            Scalar::Float64(x) => format!("{x}"),
            Scalar::Int64(x) => format!("{x}"),
            Scalar::Utf8(x) => x.clone(),
            Scalar::Null(_) => "NA".to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn frame_i64(k1: Column, k2: Column, v: &[i64]) -> DataFrame {
    let index = Index::new((0..v.len() as i64).map(IndexLabel::Int64).collect());
    let mut cols = std::collections::BTreeMap::new();
    cols.insert("k1".to_string(), k1);
    cols.insert("k2".to_string(), k2);
    cols.insert("v".to_string(), Column::from_i64_values(v.to_vec()));
    DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v".into()]).unwrap()
}

// idxmax (f64 values): result is the index label of each group's max row.
// pandas -> [3,2,0,4]; dense (contiguous keys) must equal generic (scalar keys).
#[test]
fn idxmax_dense_matches_generic_and_pandas() {
    let dense = run("idxmax", &frame(contig(&K1), contig(&K2)));
    let generic = run("idxmax", &frame(scalar(&K1), scalar(&K2)));
    assert_eq!(cells(&dense), cells(&generic), "idxmax dense vs generic");
    assert_eq!(idx_labels(&dense), idx_labels(&generic));
    assert_eq!(cells(&dense), vec!["3", "2", "0", "4"], "idxmax vs pandas");
}

// Mixed Int64 + Utf8 keys: k1=[2,1,2,1,3] (i64), k2=K2 (utf8), v=V (f64).
// pandas sorts by (k1 numeric, k2 str): (1,x),(2,x),(2,y),(3,x); sum [6,3,1,5],
// max [4,3,1,5]. dense path needs BOTH keys typed-contiguous; a scalar-backed k2
// forces the generic build_groups path for the comparison.
#[test]
fn mixed_int_utf8_dense_matches_generic_and_pandas() {
    let k1 = [2i64, 1, 2, 1, 3];
    let mk = |k2: Column| -> DataFrame {
        let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
        let mut cols = std::collections::BTreeMap::new();
        cols.insert("k1".to_string(), Column::from_i64_values(k1.to_vec()));
        cols.insert("k2".to_string(), k2);
        cols.insert("v".to_string(), Column::from_values(V.iter().map(|x| Scalar::Float64(*x)).collect()).unwrap());
        DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v".into()]).unwrap()
    };
    for (op, want) in [("sum", vec!["6", "3", "1", "5"]), ("max", vec!["4", "3", "1", "5"])] {
        let dense = run(op, &mk(contig(&K2))); // contiguous k2 -> mixed dense path
        let generic = run(op, &mk(scalar(&K2))); // scalar-backed k2 -> build_groups
        assert_eq!(cells(&dense), cells(&generic), "{op}: mixed dense vs generic");
        assert_eq!(idx_labels(&dense), idx_labels(&generic), "{op}: mixed index");
        assert_eq!(cells(&dense), want, "{op}: mixed vs pandas");
        assert_eq!(idx_labels(&dense), vec!["1, x", "2, x", "2, y", "3, x"], "{op}: numeric k1 sort");
    }
}

// nunique needs i64 values (its dense bitset); pandas -> [2,1].
#[test]
fn nunique_i64_dense_matches_generic_and_pandas() {
    let k1 = ["a", "a", "a", "b"];
    let k2 = ["x", "x", "x", "x"];
    let v = [10i64, 20, 10, 30];
    let dense = run("nunique", &frame_i64(contig(&k1), contig(&k2), &v));
    let generic = run("nunique", &frame_i64(scalar(&k1), scalar(&k2), &v));
    assert_eq!(cells(&dense), cells(&generic), "nunique dense vs generic");
    assert_eq!(idx_labels(&dense), idx_labels(&generic));
    assert_eq!(cells(&dense), vec!["2", "1"], "nunique vs pandas");
    assert_eq!(idx_labels(&dense), vec!["a, x", "b, x"]);
}

#[test]
fn dense_matches_generic_and_pandas() {
    let expected: &[(&str, Vec<f64>)] = &[
        ("sum", vec![6.0, 3.0, 1.0, 5.0]),
        ("mean", vec![3.0, 3.0, 1.0, 5.0]),
        ("count", vec![2.0, 1.0, 1.0, 1.0]),
        ("min", vec![2.0, 3.0, 1.0, 5.0]),
        ("max", vec![4.0, 3.0, 1.0, 5.0]),
        ("median", vec![3.0, 3.0, 1.0, 5.0]),
    ];
    for (op, want) in expected {
        let dense = run(op, &frame(contig(&K1), contig(&K2))); // contiguous -> my dense path
        let generic = run(op, &frame(scalar(&K1), scalar(&K2))); // scalar-backed -> build_groups
        // dense == generic (bit-identity of the bypass)
        assert_eq!(vals(&dense), vals(&generic), "{op}: dense vs generic values");
        assert_eq!(idx_labels(&dense), idx_labels(&generic), "{op}: dense vs generic index");
        // both == pandas
        assert_eq!(&vals(&dense), want, "{op}: vs pandas values");
        assert_eq!(idx_labels(&dense), vec!["a, x", "b, x", "b, y", "c, x"], "{op}: sorted group order");
    }
}
