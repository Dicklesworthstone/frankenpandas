use std::time::Instant;

use fp_columnar::{Column, ValidityMask};
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
fn sm(i: usize, s: u64) -> f64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 11) as f64
}

fn percentile_ns(samples: &mut [u128], percentile: usize) -> u128 {
    samples.sort_unstable();
    let index = (samples.len() - 1) * percentile / 100;
    match samples.get(index) {
        Some(sample) => *sample,
        None => 0,
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let g: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let op = a.get(3).map(String::as_str).unwrap_or("idxmax");
    let it: usize = a
        .get(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6)
        .max(1);
    let dtype = a.get(5).map(String::as_str).unwrap_or("f64");
    let n = n.max(2);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut key_data: Vec<i64> = (0..n)
        .map(|i| (sm(i, 0) as u64 as i64) % g)
        .collect();
    let mut value_data: Vec<f64> = (0..n).map(|i| sm(i, 1)).collect();
    // The control key drops only the final row. Make that row an exact duplicate
    // of its predecessor so candidate/control OHLC outputs remain identical while
    // the nullable control stays on the historical group-Vec implementation.
    if let [.., previous, last] = key_data.as_mut_slice() {
        *last = *previous;
    }
    if let [.., previous, last] = value_data.as_mut_slice() {
        *last = *previous;
    }
    let keys = Series::new(
        "k".to_string(),
        Index::new(labels.clone()),
        Column::from_i64_values(key_data.clone()),
    )
    .unwrap();
    let reference_keys = if op == "ohlc-ab" {
        let mut validity = ValidityMask::all_valid(n);
        validity.set(n - 1, false);
        Some(
            Series::new(
                "k".to_string(),
                Index::new(labels.clone()),
                Column::from_i64_values_with_validity(key_data, validity),
            )
            .unwrap(),
        )
    } else {
        None
    };
    let values = if dtype == "i64" {
        Column::from_i64_values(value_data.iter().map(|&value| value as i64).collect())
    } else {
        Column::from_f64_values(value_data)
    };
    let vs = Series::new("a".to_string(), Index::new(labels), values).unwrap();
    if let Some(reference_keys) = reference_keys {
        let candidate = vs.groupby(&keys).unwrap().ohlc().unwrap();
        let control = vs.groupby(&reference_keys).unwrap().ohlc().unwrap();
        assert_eq!(candidate, control, "typed and historical OHLC paths differ");

        for _ in 0..2 {
            std::hint::black_box(vs.groupby(&keys).unwrap().ohlc().unwrap());
            std::hint::black_box(vs.groupby(&reference_keys).unwrap().ohlc().unwrap());
        }
        let mut candidate_ns = Vec::with_capacity(it);
        let mut control_ns = Vec::with_capacity(it);
        for iteration in 0..it {
            let mut time_candidate = || {
                let start = Instant::now();
                std::hint::black_box(vs.groupby(&keys).unwrap().ohlc().unwrap());
                candidate_ns.push(start.elapsed().as_nanos());
            };
            let mut time_control = || {
                let start = Instant::now();
                std::hint::black_box(vs.groupby(&reference_keys).unwrap().ohlc().unwrap());
                control_ns.push(start.elapsed().as_nanos());
            };
            if iteration % 2 == 0 {
                time_candidate();
                time_control();
            } else {
                time_control();
                time_candidate();
            }
        }
        let candidate_p50 = percentile_ns(&mut candidate_ns, 50);
        let candidate_p95 = percentile_ns(&mut candidate_ns, 95);
        let control_p50 = percentile_ns(&mut control_ns, 50);
        let control_p95 = percentile_ns(&mut control_ns, 95);
        println!(
            "ohlc-ab dtype={dtype} n={n} g={g}: CANDIDATE(p50/p95)={:.3}/{:.3}ms \
             CONTROL(p50/p95)={:.3}/{:.3}ms speedup={:.3}x",
            candidate_p50 as f64 / 1e6,
            candidate_p95 as f64 / 1e6,
            control_p50 as f64 / 1e6,
            control_p95 as f64 / 1e6,
            control_p50 as f64 / candidate_p50 as f64,
        );
        return;
    }
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "idxmax" => {
                std::hint::black_box(vs.groupby(&keys).unwrap().idxmax().unwrap());
            }
            "idxmin" => {
                std::hint::black_box(vs.groupby(&keys).unwrap().idxmin().unwrap());
            }
            "value_counts" => {
                std::hint::black_box(keys.groupby(&keys).unwrap().value_counts().unwrap());
            }
            "first" => {
                std::hint::black_box(vs.groupby(&keys).unwrap().first().unwrap());
            }
            _ => panic!(),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} dtype={dtype} n={n} g={g}: best={best}ns");
}
