//! Prototype variants for unordered unique-int64 inner-join position computation.
use std::{hint::black_box, time::Instant};
fn mkdata(n: usize, scale: i64) -> (Vec<i64>, Vec<i64>) {
    let lk: Vec<i64> = (0..n as i64)
        .map(|i| ((i * 2654435761i64).rem_euclid(n as i64)) * scale)
        .collect();
    let rk: Vec<i64> = (0..n as i64)
        .map(|i| ((i * 7) % (n as i64)) * scale)
        .collect();
    (lk, rk)
}
// Packed single-array open addressing; sentinel empty key = i64::MIN; entry=(key,pos).
fn hashjoin_packed(lk: &[i64], rk: &[i64]) -> (Vec<u32>, Vec<u32>) {
    const EMPTY: i64 = i64::MIN;
    let cap = (rk.len().saturating_mul(2).max(16)).next_power_of_two();
    let mask = cap - 1;
    let mut keys = vec![EMPTY; cap];
    let mut poss = vec![0u32; cap];
    #[inline]
    fn mix(k: i64) -> usize {
        let mut x = k as u64;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
        x ^= x >> 33;
        x as usize
    }
    for (i, &k) in rk.iter().enumerate() {
        let mut s = mix(k) & mask;
        loop {
            if keys[s] == EMPTY {
                keys[s] = k;
                poss[s] = i as u32;
                break;
            }
            if keys[s] == k {
                poss[s] = i as u32;
                break;
            }
            s = (s + 1) & mask;
        }
    }
    let mut lp = Vec::with_capacity(lk.len());
    let mut rp = Vec::with_capacity(lk.len());
    for (i, &k) in lk.iter().enumerate() {
        let mut s = mix(k) & mask;
        loop {
            let kk = keys[s];
            if kk == EMPTY {
                break;
            }
            if kk == k {
                lp.push(i as u32);
                rp.push(poss[s]);
                break;
            }
            s = (s + 1) & mask;
        }
    }
    (lp, rp)
}
// Direct-address for unique keys: right_pos[key-min]=pos+1 (0=empty). span-sized.
fn directaddr(lk: &[i64], rk: &[i64]) -> Option<(Vec<u32>, Vec<u32>)> {
    let mn = *rk.iter().chain(lk.iter()).min()?;
    let mx = *rk.iter().chain(lk.iter()).max()?;
    let span = (mx as i128 - mn as i128 + 1) as usize;
    if span
        > lk.len()
            .saturating_add(rk.len())
            .saturating_mul(4)
            .max(1024)
    {
        return None;
    }
    let mut tbl = vec![0u32; span]; // pos+1, 0=empty
    for (i, &k) in rk.iter().enumerate() {
        tbl[(k - mn) as usize] = i as u32 + 1;
    }
    let mut lp = Vec::with_capacity(lk.len());
    let mut rp = Vec::with_capacity(lk.len());
    for (i, &k) in lk.iter().enumerate() {
        let v = tbl[(k - mn) as usize];
        if v != 0 {
            lp.push(i as u32);
            rp.push(v - 1);
        }
    }
    Some((lp, rp))
}
fn bench(name: &str, mut f: impl FnMut() -> usize, it: usize) {
    for _ in 0..3 {
        black_box(f());
    }
    let st = Instant::now();
    let mut s = 0usize;
    for _ in 0..it {
        s ^= black_box(f());
    }
    println!(
        "{name:30}: {:.3} ms/call (s={s})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
fn main() {
    let n = 1_000_000usize;
    let (lk, rk) = mkdata(n, 1);
    bench(
        "packed_span_n",
        || {
            let (a, b) = hashjoin_packed(&lk, &rk);
            a.len() ^ b.len()
        },
        30,
    );
    bench(
        "directaddr_span_n",
        || {
            let (a, b) = directaddr(&lk, &rk).unwrap();
            a.len() ^ b.len()
        },
        30,
    );
    let (lk5, rk5) = mkdata(n, 5);
    bench(
        "packed_span_5n",
        || {
            let (a, b) = hashjoin_packed(&lk5, &rk5);
            a.len() ^ b.len()
        },
        30,
    );
    bench(
        "directaddr_span_5n(bail?)",
        || match directaddr(&lk5, &rk5) {
            Some((a, b)) => a.len() ^ b.len(),
            None => 0,
        },
        30,
    );
}
