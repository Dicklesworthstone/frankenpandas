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
fn main() {
    let n = 2_000_000usize;
    let mut hv: Vec<i64> = (0..n).map(|i| (sm(i, 7) % 1000000) as i64).collect();
    hv.sort_unstable();
    let hay = Series::new(
        "h",
        Index::from_range(0, n as i64, 1),
        Column::from_values(hv.iter().cloned().map(Scalar::Int64).collect()).unwrap(),
    )
    .unwrap();
    let big: Vec<Scalar> = (0..2_000_000usize)
        .map(|i| Scalar::Int64((sm(i, 3) % 1000000) as i64))
        .collect();
    t("ss_i64_bigm_left", || {
        std::hint::black_box(hay.searchsorted_values(&big, "left").unwrap());
    });
    t("ss_i64_bigm_right", || {
        std::hint::black_box(hay.searchsorted_values(&big, "right").unwrap());
    });
    // differential vs oracle
    let mut fails = 0usize;
    for seed in 0..40u64 {
        let nn = (sm(seed as usize, 5) % 20000 + 4096) as usize;
        let hn = (sm(seed as usize, 6) % 50000 + 1000) as usize;
        let mut hv: Vec<i64> = (0..hn)
            .map(|i| (sm(i, seed * 3 + 1) % 1000) as i64)
            .collect();
        hv.sort_unstable();
        let h = Series::new(
            "h",
            Index::from_range(0, hn as i64, 1),
            Column::from_values(hv.iter().cloned().map(Scalar::Int64).collect()).unwrap(),
        )
        .unwrap();
        let nd: Vec<i64> = (0..nn)
            .map(|i| (sm(i, seed * 7 + 2) % 1000) as i64)
            .collect();
        let needles: Vec<Scalar> = nd.iter().cloned().map(Scalar::Int64).collect();
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
            }
        }
    }
    println!("differential: {fails} fails / 80");
    assert_eq!(fails, 0);
}
