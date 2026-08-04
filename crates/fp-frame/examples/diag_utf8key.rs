use fp_columnar::Column;
use fp_types::Scalar;
fn main() {
    let v: Vec<Scalar> = (0..20)
        .map(|i| Scalar::Utf8(format!("cat{}", i % 5)))
        .collect();
    let c = Column::from_values(v).unwrap();
    println!("dtype={:?}", c.dtype());
    println!("as_utf8_contiguous = {}", c.as_utf8_contiguous().is_some());
}
