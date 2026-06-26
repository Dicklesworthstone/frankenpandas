//! Index.isin / union over Utf8 indexes @200k. Run: bench_idx_utf8_isin <n> <op>
use std::sync::Arc;

use fp_index::{Index, IndexLabel};

fn contig_index(words: impl Iterator<Item = String>) -> Index {
    let mut bytes: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = vec![0];
    for w in words {
        bytes.extend_from_slice(w.as_bytes());
        offsets.push(bytes.len());
    }
    Index::from_utf8_contiguous(Arc::from(bytes), Arc::from(offsets))
}

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let op = a.get(2).map(String::as_str).unwrap_or("isin");
    // Contiguous-Utf8 backed indexes (as read_csv / string columns produce).
    let idx = contig_index((0..n).map(|i| format!("k{:08}", i)));
    let other = contig_index((0..n).map(|i| format!("k{:08}", i + n / 2)));
    // needles for isin: distinct strings shifted by n/2
    let other_vals: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("k{:08}", i + n / 2)))
        .collect();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "isin" => {
                std::hint::black_box(idx.isin(&other_vals));
            }
            "union" => {
                std::hint::black_box(idx.union(&other));
            }
            "intersection" => {
                std::hint::black_box(idx.intersection(&other));
            }
            _ => panic!("op"),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    // touch sm so it's used
    std::hint::black_box(sm(0, 0));
    println!("idx_utf8_{op} n={n}: best={best}ns");
}
