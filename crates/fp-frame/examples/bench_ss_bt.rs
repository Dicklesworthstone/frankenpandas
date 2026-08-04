use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn build(n: usize) -> (Series, Vec<f64>) {
    let mut hv: Vec<f64> = (0..n).map(|i| (sm(i, 7) % 1000000) as f64).collect();
    hv.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let hay = Series::new(
        "h",
        Index::from_range(0, n as i64, 1),
        Column::from_values(hv.iter().cloned().map(Scalar::Float64).collect()).unwrap(),
    )
    .unwrap();
    (hay, hv)
}
fn main() {
    let n = 2_000_000usize;
    let (hay, _) = build(n);
    // large-m (finger path)
    let big: Vec<Scalar> = (0..2_000_000usize)
        .map(|i| Scalar::Float64((sm(i, 3) % 1000000) as f64))
        .collect();
    t("ss_f64_bigm_left", || {
        std::hint::black_box(hay.searchsorted_values(&big, "left").unwrap());
    });
    t("ss_f64_bigm_right", || {
        std::hint::black_box(hay.searchsorted_values(&big, "right").unwrap());
    });
    // small-m (per-needle path)
    let small: Vec<Scalar> = (0..1000usize)
        .map(|i| Scalar::Float64((sm(i, 3) % 1000000) as f64))
        .collect();
    t("ss_f64_smallm", || {
        std::hint::black_box(hay.searchsorted_values(&small, "left").unwrap());
    });

    // DIFFERENTIAL: finger (big-m) vs a brute per-needle partition_point oracle, both sides
    let mut fails = 0usize;
    for seed in 0..40u64 {
        let nn = (sm(seed as usize, 5) % 20000 + 4096) as usize; // >=4096 -> finger
        let hn = (sm(seed as usize, 6) % 50000 + 1000) as usize;
        let mut hv: Vec<f64> = (0..hn)
            .map(|i| (sm(i, seed * 3 + 1) % 1000) as f64)
            .collect();
        hv.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let h = Series::new(
            "h",
            Index::from_range(0, hn as i64, 1),
            Column::from_values(hv.iter().cloned().map(Scalar::Float64).collect()).unwrap(),
        )
        .unwrap();
        let nd: Vec<f64> = (0..nn)
            .map(|i| (sm(i, seed * 7 + 2) % 1000) as f64)
            .collect();
        let needles: Vec<Scalar> = nd.iter().cloned().map(Scalar::Float64).collect();
        for right in [false, true] {
            let side = if right { "right" } else { "left" };
            let got = h.searchsorted_values(&needles, side).unwrap();
            let oracle: Vec<usize> = nd
                .iter()
                .map(|&k| {
                    if right {
                        hv.partition_point(|&x| x <= k)
                    } else {
                        hv.partition_point(|&x| x < k)
                    }
                })
                .collect();
            if got != oracle {
                fails += 1;
                if fails <= 3 {
                    println!("DIFF seed={seed} side={side}");
                }
            }
        }
    }
    println!("differential: {fails} fails / 80");
    assert_eq!(fails, 0);
}
