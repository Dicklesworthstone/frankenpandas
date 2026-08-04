use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn main() {
    let n = 100usize;
    let nv: Vec<Scalar> = (0..n)
        .map(|i| {
            if i % 5 == 0 {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64(i as i64)
            }
        })
        .collect();
    let raw = Column::from_values(nv.clone()).unwrap();
    println!(
        "RAW           dtype={:?} as_i64_wv={}",
        raw.dtype(),
        raw.as_i64_slice_with_validity().is_some()
    );
    let cloned = raw.clone();
    println!(
        "RAW.clone()   dtype={:?} as_i64_wv={}",
        cloned.dtype(),
        cloned.as_i64_slice_with_validity().is_some()
    );
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("nv".to_string(), Column::from_values(nv.clone()).unwrap());
    let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["nv".into()]).unwrap();
    let fromdf = df.column("nv").unwrap();
    println!(
        "DF.column     dtype={:?} as_i64_wv={}",
        fromdf.dtype(),
        fromdf.as_i64_slice_with_validity().is_some()
    );
    let fromdf_cl = df.column("nv").unwrap().clone();
    println!(
        "DF.column.cln dtype={:?} as_i64_wv={}",
        fromdf_cl.dtype(),
        fromdf_cl.as_i64_slice_with_validity().is_some()
    );
    // through Series
    let s = Series::new("nv", idx.clone(), df.column("nv").unwrap().clone()).unwrap();
    println!(
        "Series.column dtype={:?} as_i64_wv={}",
        s.column().dtype(),
        s.column().as_i64_slice_with_validity().is_some()
    );
}
