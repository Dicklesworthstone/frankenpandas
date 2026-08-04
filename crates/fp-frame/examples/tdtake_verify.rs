use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn main() {
    let n = 50i64;
    let s = Series::new(
        "td",
        Index::from_range(0, n, 1),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Timedelta64(i * 1_000_000_000))
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
        .all(|(v, &p)| matches!(v, Scalar::Timedelta64(ns) if *ns==p*1_000_000_000));
    println!("timedelta take correct: {ok}");
}
