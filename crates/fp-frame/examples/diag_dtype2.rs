use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn main() {
    // group 2 is FULLY missing in value "a"
    let keys: Vec<Scalar> = (0..12).map(|i| Scalar::Int64((i % 3) as i64)).collect();
    let a: Vec<Scalar> = (0..12)
        .map(|i| {
            let g = i % 3;
            if g == 2 || i % 4 == 0 {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((i * 10) as i64)
            }
        })
        .collect();
    let idx = Index::from_range(0, 12, 1);
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys).unwrap());
    map.insert("a".into(), Column::from_values(a).unwrap());
    let df = DataFrame::new_with_column_order(idx, map, vec!["k".into(), "a".into()]).unwrap();
    for op in [
        "sum", "mean", "max", "min", "first", "last", "prod", "var", "std", "median",
    ] {
        let g = df.groupby(&["k"]).unwrap();
        let r = match op {
            "sum" => g.sum(),
            "mean" => g.mean(),
            "max" => g.max(),
            "min" => g.min(),
            "first" => g.first(),
            "last" => g.last(),
            "prod" => g.prod(),
            "var" => g.var(),
            "std" => g.std(),
            _ => g.median(),
        };
        match r {
            Ok(r) => {
                let col = r.column("a").unwrap();
                println!("{op}: dtype={:?} vals={:?}", col.dtype(), col.values());
            }
            Err(e) => println!("{op}: ERR {e:?}"),
        }
    }
}
