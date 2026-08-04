use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn timeit<F: FnMut()>(label: &str, mut f: F) {
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{label}: {:.2}ms", best as f64 / 1e6);
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);
    let f = Column::from_f64_values(
        (0..n as u64)
            .map(|i| {
                let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                ((z >> 11) as f64) / 1e9
            })
            .collect(),
    );
    let s = Series::new("s", Index::from_range(0, n as i64, 1), f).unwrap();
    // low-cardinality i64 for value_counts/duplicated/nunique
    let ic = Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect());
    let si = Series::new("si", Index::from_range(0, n as i64, 1), ic).unwrap();
    timeit("sum", || {
        std::hint::black_box(s.sum().unwrap());
    });
    timeit("mean", || {
        std::hint::black_box(s.mean().unwrap());
    });
    timeit("std", || {
        std::hint::black_box(s.std().unwrap());
    });
    timeit("median", || {
        std::hint::black_box(s.median().unwrap());
    });
    timeit("min", || {
        std::hint::black_box(s.min().unwrap());
    });
    timeit("cummax", || {
        std::hint::black_box(s.cummax().unwrap().len());
    });
    timeit("gt_scalar", || {
        std::hint::black_box(s.gt_scalar(&Scalar::Float64(0.5)).unwrap().len());
    });
    let m = s.gt_scalar(&Scalar::Float64(0.5)).unwrap();
    timeit("where(prebuilt)", || {
        std::hint::black_box(s.where_cond(&m, Some(&Scalar::Float64(0.0))).unwrap().len());
    });
    timeit("mask(prebuilt)", || {
        std::hint::black_box(s.mask(&m, Some(&Scalar::Float64(0.0))).unwrap().len());
    });
    let other_f = Series::new(
        "o",
        Index::from_range(0, n as i64, 1),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    )
    .unwrap();
    timeit("where_series_f64", || {
        std::hint::black_box(s.where_cond_series(&m, &other_f).unwrap().len());
    });
    let other_i = Series::new(
        "o",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values((0..n as i64).collect()),
    )
    .unwrap();
    let mi = si.gt_scalar(&Scalar::Int64(500)).unwrap();
    timeit("where_series_i64", || {
        std::hint::black_box(si.where_cond_series(&mi, &other_i).unwrap().len());
    });
    let siq = Series::new(
        "siq",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values(
            (0..n as u64)
                .map(|i| {
                    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                    (z >> 11) as i64
                })
                .collect(),
        ),
    )
    .unwrap();
    timeit("pct_change_i64", || {
        std::hint::black_box(siq.pct_change(1).unwrap().len());
    });
    timeit("nlargest_i64", || {
        std::hint::black_box(siq.nlargest(100).unwrap().len());
    });
    timeit("idxmax_i64", || {
        std::hint::black_box(siq.idxmax().unwrap());
    });
    timeit("ffill_i64_clean", || {
        std::hint::black_box(siq.ffill(None).unwrap().len());
    });
    timeit("rank_i64", || {
        std::hint::black_box(siq.rank("average", true, "keep").unwrap().len());
    });
    timeit("nsmallest_i64", || {
        std::hint::black_box(siq.nsmallest(100).unwrap().len());
    });
    timeit("argsort_i64", || {
        std::hint::black_box(siq.argsort(true).unwrap().len());
    });
    timeit("interpolate_i64_clean", || {
        std::hint::black_box(siq.interpolate().unwrap().len());
    });
    timeit("diff_i64", || {
        std::hint::black_box(siq.diff(1).unwrap().len());
    });
    timeit("cummax_i64", || {
        std::hint::black_box(siq.cummax().unwrap().len());
    });
    timeit("mode_i64", || {
        std::hint::black_box(si.mode().unwrap().len());
    });
    timeit("cut_i64", || {
        std::hint::black_box(fp_frame::cut(&siq, 10).unwrap().len());
    });
    timeit("cut_f64", || {
        std::hint::black_box(fp_frame::cut(&s, 10).unwrap().len());
    });
    let oth_i = Series::new(
        "o",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values((0..n as i64).map(|i| i % 777).collect()),
    )
    .unwrap();
    timeit("update_i64", || {
        std::hint::black_box(si.update(&oth_i).unwrap().len());
    });
    timeit("combine_first_i64", || {
        std::hint::black_box(si.combine_first(&oth_i).unwrap().len());
    });
    let lo_f = Series::new(
        "lo",
        Index::from_range(0, n as i64, 1),
        Column::from_f64_values((0..n).map(|i| (i % 100) as f64 * 0.001).collect()),
    )
    .unwrap();
    let hi_f = Series::new(
        "hi",
        Index::from_range(0, n as i64, 1),
        Column::from_f64_values((0..n).map(|i| 0.5 + (i % 100) as f64 * 0.001).collect()),
    )
    .unwrap();
    timeit("clip_series_f64", || {
        std::hint::black_box(s.clip_with_series(Some(&lo_f), Some(&hi_f)).unwrap().len());
    });
    let lo_i = Series::new(
        "lo",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values((0..n as i64).map(|i| i % 200).collect()),
    )
    .unwrap();
    let hi_i = Series::new(
        "hi",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values((0..n as i64).map(|i| 500 + i % 200).collect()),
    )
    .unwrap();
    timeit("clip_series_i64", || {
        std::hint::black_box(si.clip_with_series(Some(&lo_i), Some(&hi_i)).unwrap().len());
    });
    timeit("mask_series_f64", || {
        std::hint::black_box(s.mask_series(&m, &other_f).unwrap().len());
    });
    timeit("mask_series_i64", || {
        std::hint::black_box(si.mask_series(&mi, &other_i).unwrap().len());
    });
    timeit("where_i64(prebuilt)", || {
        std::hint::black_box(si.where_cond(&mi, Some(&Scalar::Int64(0))).unwrap().len());
    });
    timeit("mask_i64(prebuilt)", || {
        std::hint::black_box(si.mask(&mi, Some(&Scalar::Int64(0))).unwrap().len());
    });
    timeit("value_counts_i64", || {
        std::hint::black_box(si.value_counts().unwrap().len());
    });
    timeit("duplicated_i64", || {
        std::hint::black_box(si.duplicated().unwrap().len());
    });
    timeit("nunique_i64", || {
        std::hint::black_box(si.nunique());
    });
    timeit("unique_i64", || {
        std::hint::black_box(si.unique().len());
    });
}
