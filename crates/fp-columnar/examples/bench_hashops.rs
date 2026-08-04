//! Column::nunique / duplicated over a high-card 5M Int64 column. bench_hashops <n> <op>
use fp_columnar::Column;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("nunique");
    let mode = a.get(3).map(String::as_str).unwrap_or("dense");
    let data: Vec<i64> = if mode == "wide" || mode == "dt" {
        // Sparse full-range i64, ~5M distinct: no dense bitset possible -> hashset.
        (0..n as i64)
            .map(|i| i.wrapping_mul(2_654_435_761))
            .collect()
    } else {
        // ~1M distinct in [0,1M): dense direct-address regime.
        (0..n as i64)
            .map(|i| (i.wrapping_mul(2_654_435_761)).rem_euclid(1_000_000))
            .collect()
    };
    let col = if mode == "dt" {
        Column::from_datetime64_values(data)
    } else if mode == "f64" {
        // Distinct-ish floats from a splitmix-style mix (~5M distinct).
        let f: Vec<f64> = (0..n as u64)
            .map(|i| {
                let mut z = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                (z >> 11) as f64 / (1u64 << 53) as f64
            })
            .collect();
        Column::from_f64_values(f)
    } else {
        Column::from_i64_values(data)
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "mode" => {
                let r = col.mode().unwrap();
                std::hint::black_box(r.len());
            }
            "unique" => {
                let r = col.unique().unwrap();
                std::hint::black_box(r.len());
            }
            "factorize" => {
                let (c, u) = col.factorize().unwrap();
                std::hint::black_box((c.len(), u.len()));
            }
            "duplicated" => {
                let r = col.duplicated().unwrap();
                std::hint::black_box(r.len());
            }
            "value_counts" => {
                let Ok((v, c)) = col.value_counts() else {
                    std::process::abort();
                };
                std::hint::black_box((v.len(), c.len()));
            }
            _ => {
                let r = col.nunique();
                std::hint::black_box(&r);
            }
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} i64 n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
