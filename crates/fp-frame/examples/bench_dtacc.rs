//! Series.dt.{strftime,day_name,month_name,isocalendar,normalize} @1M. bench_dtacc <n> <op>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("strftime");
    let base = 1_577_836_800_000_000_000i64; // 2020-01-01
    let hour = 3_600_000_000_000i64;
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "t",
        Index::new(labels),
        Column::from_datetime64_values((0..n).map(|i| base + (i as i64 % 87000) * hour).collect()),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let d = s.dt();
        let r: Result<(), _> = match op {
            "strftime" => d.strftime("%Y-%m-%d").map(|_| ()),
            "day_name" => d.day_name().map(|_| ()),
            "month_name" => d.month_name().map(|_| ()),
            "isocalendar" => d.isocalendar().map(|_| ()),
            "normalize" => d.normalize().map(|_| ()),
            _ => panic!("op"),
        };
        let _: () = r.unwrap();
        std::hint::black_box(());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("dt_{op} n={n}: best={best}ns");
}
