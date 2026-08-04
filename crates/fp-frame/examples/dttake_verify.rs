use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn main() {
    let n = 50i64;
    let base = 1_600_000_000_000_000_000i64;
    let s = Series::new(
        "dt",
        Index::from_range(0, n, 1),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Datetime64(base + i * 1_000_000_000))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let pos: Vec<i64> = vec![3, 0, 49, 7, 7, 25, 49, 1];
    let t = s.take(&pos).unwrap();
    let ok = t
        .column()
        .values()
        .iter()
        .zip(&pos)
        .all(|(v, &p)| matches!(v, Scalar::Datetime64(ns) if *ns==base+p*1_000_000_000));
    println!(
        "datetime take correct: {ok}  first={:?}",
        t.column().values()[0]
    );
}
