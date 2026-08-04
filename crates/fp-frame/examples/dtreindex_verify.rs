use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn main() {
    let base = 1_600_000_000_000_000_000i64;
    let s = Series::new(
        "dt",
        Index::from_range(0, 5, 1),
        Column::from_datetime64_values((0..5).map(|i| base + i * 1_000_000_000).collect()),
    )
    .unwrap();
    // reindex to [2,3,4,5,6,-1] -> 2,3,4 present; 5,6,-1 missing
    let nl: Vec<IndexLabel> = [2i64, 3, 4, 5, 6, -1]
        .iter()
        .map(|&x| IndexLabel::Int64(x))
        .collect();
    let r = s.reindex(nl).unwrap();
    let got: Vec<String> = r
        .column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Datetime64(ns) => ((ns - base) / 1_000_000_000).to_string(),
            Scalar::Null(_) => "NaT".into(),
            o => format!("{o:?}"),
        })
        .collect();
    println!("datetime reindex: {:?}", got);
    println!("expected: [\"2\",\"3\",\"4\",\"NaT\",\"NaT\",\"NaT\"]");
    // timedelta
    let td = Series::new(
        "td",
        Index::from_range(0, 5, 1),
        Column::from_values(
            (0..5)
                .map(|i| Scalar::Timedelta64(i as i64 * 1_000_000_000))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let nl2: Vec<IndexLabel> = [2i64, 3, 9].iter().map(|&x| IndexLabel::Int64(x)).collect();
    let r2 = td.reindex(nl2).unwrap();
    let got2: Vec<String> = r2
        .column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Timedelta64(ns) => (ns / 1_000_000_000).to_string(),
            Scalar::Null(_) => "NaT".into(),
            o => format!("{o:?}"),
        })
        .collect();
    println!("timedelta reindex: {:?} (expect [2,3,NaT])", got2);
}
