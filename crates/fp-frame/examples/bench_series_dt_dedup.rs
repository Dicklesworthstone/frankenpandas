//! Series value_counts/nunique/unique/duplicated/drop_duplicates over a
//! Datetime64 VALUE column @200k. Also covers Period gather probes:
//! `period_take` and `period_reindex`. Run: bench_series_dt_dedup <n> <op>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{Period, PeriodFreq, Scalar};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let op = a.get(2).map(String::as_str).unwrap_or("value_counts");
    if matches!(op, "period_take" | "period_reindex") {
        let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
        let values: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Period(Period::new(i as i64, PeriodFreq::Monthly)))
            .collect();
        let s = Series::new(
            "p",
            Index::new(labels),
            Column::new(fp_types::DType::Period, values)?,
        )?;
        let take_idx: Vec<i64> = (0..n).map(|i| (sm(i, 17) % n as u64) as i64).collect();
        let reindex_labels: Vec<IndexLabel> = (n as i64 / 2..n as i64 + n as i64 / 2)
            .map(IndexLabel::Int64)
            .collect();
        let mut best = u128::MAX;
        for _ in 0..6 {
            let t = std::time::Instant::now();
            if op == "period_take" {
                std::hint::black_box(s.take(&take_idx)?.len());
            } else {
                std::hint::black_box(s.reindex(reindex_labels.clone())?.len());
            }
            let e = t.elapsed().as_nanos();
            if e < best {
                best = e;
            }
        }
        println!("series_{op} n={n}: best={best}ns");
        return Ok(());
    }
    let base = 1_577_836_800_000_000_000i64;
    let step = 60_000_000_000i64;
    let card = (n / 4).max(1) as u64;
    let data: Vec<i64> = (0..n)
        .map(|i| base + (sm(i, 0) % card) as i64 * step)
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "s",
        Index::new(labels),
        Column::from_datetime64_values(data),
    )?;
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "value_counts" => {
                std::hint::black_box(s.value_counts()?);
            }
            "nunique" => {
                std::hint::black_box(s.nunique());
            }
            "unique" => {
                std::hint::black_box(s.unique());
            }
            "duplicated" => {
                std::hint::black_box(s.duplicated()?);
            }
            "drop_duplicates" => {
                std::hint::black_box(s.drop_duplicates()?);
            }
            _ => {
                eprintln!("unknown op: {op}");
                return Ok(());
            }
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("series_dt_{op} n={n}: best={best}ns");
    Ok(())
}
