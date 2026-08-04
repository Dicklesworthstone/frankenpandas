fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
#[inline(never)]
fn norm_i128(indices: &[i64], len: usize) -> Vec<usize> {
    indices
        .iter()
        .map(|&position| {
            let len_i128 = len as i128;
            let position_i128 = position as i128;
            let normalized = if position_i128 < 0 {
                len_i128 + position_i128
            } else {
                position_i128
            };
            assert!(!(normalized < 0 || normalized >= len_i128));
            normalized as usize
        })
        .collect()
}
#[inline(never)]
fn norm_i64(indices: &[i64], len: usize) -> Vec<usize> {
    let len_i64 = len as i64;
    indices
        .iter()
        .map(|&position| {
            let normalized = if position < 0 {
                len_i64 + position
            } else {
                position
            };
            assert!(!(normalized < 0 || normalized >= len_i64));
            normalized as usize
        })
        .collect()
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..30 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.3}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let pos: Vec<i64> = (0..n)
        .map(|i| {
            let p = (sm(i, 3) % n as u64) as i64;
            if i % 5 == 0 { p - (n as i64) } else { p }
        })
        .collect();
    // interleave to defeat cross-run drift
    for _ in 0..3 {
        t("i128", || {
            std::hint::black_box(norm_i128(&pos, n));
        });
        t("i64 ", || {
            std::hint::black_box(norm_i64(&pos, n));
        });
    }
    // correctness: identical outputs
    println!("equal={}", norm_i128(&pos, n) == norm_i64(&pos, n));
}
