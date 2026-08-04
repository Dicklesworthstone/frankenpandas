use fp_columnar::Column;
use fp_types::{NullKind, Scalar};
fn main() {
    let av: Vec<Scalar> = (0..10).map(Scalar::Int64).collect();
    let nv: Vec<Scalar> = (0..10)
        .map(|i| {
            if i % 5 == 0 {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64(i)
            }
        })
        .collect();
    let ca = Column::from_values(av).unwrap();
    let cn = Column::from_values(nv).unwrap();
    println!(
        "av dtype={:?} as_i64_wv.is_some={}",
        ca.dtype(),
        ca.as_i64_slice_with_validity().is_some()
    );
    println!(
        "nv dtype={:?} as_i64_wv.is_some={}",
        cn.dtype(),
        cn.as_i64_slice_with_validity().is_some()
    );
    println!("nv as_i64_slice.is_some={}", cn.as_i64_slice().is_some());
}
