use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let op = a.get(1).map(String::as_str).unwrap_or("quantile");
    let n: usize = 1_000_000;
    let idx = Index::new_known_unique_int64_unit_range(0, n);
    let data: Vec<f64> = (0..n).map(|i| (sm(i, 1) % 1_000_000) as f64).collect();
    let s = Series::new("v", idx.clone(), Column::from_f64_values(data.clone())).unwrap();
    let data2: Vec<f64> = (0..n).map(|i| (sm(i, 2) % 1_000_000) as f64).collect();
    let s2 = Series::new("w", idx.clone(), Column::from_f64_values(data2)).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "quantile" => {
                std::hint::black_box(s.quantile(0.5).unwrap());
            }
            "nunique" => {
                std::hint::black_box(s.nunique());
            }
            "duplicated" => {
                std::hint::black_box(s.duplicated().unwrap());
            }
            "corr" => {
                std::hint::black_box(s.corr(&s2).unwrap());
            }
            "cov" => {
                std::hint::black_box(s.cov(&s2).unwrap());
            }
            "autocorr" => {
                std::hint::black_box(s.autocorr(1).unwrap());
            }
            "is_monotonic" => {
                std::hint::black_box(s.is_monotonic_increasing());
            }
            _ => panic!(),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("probe2_{op} n={n}: best={best}ns");
}
