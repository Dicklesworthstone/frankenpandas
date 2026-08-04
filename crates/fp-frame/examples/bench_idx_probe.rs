use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let vv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 1).is_multiple_of(5) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::Float64((sm(i, 7) % 100000) as f64)
            }
        })
        .collect();
    let s = Series::new(
        "v",
        Index::from_range(0, n as i64, 1),
        Column::from_values(vv).unwrap(),
    )
    .unwrap();
    timeit("idxmax", || {
        std::hint::black_box(s.idxmax().unwrap());
    });
    timeit("idxmin", || {
        std::hint::black_box(s.idxmin().unwrap());
    });
    // correctness: max at a known position
    let sc = Series::new(
        "c",
        Index::new(vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ]),
        Column::from_values(vec![
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(9.0),
            Scalar::Float64(1.0),
            Scalar::Float64(9.0),
        ])
        .unwrap(),
    )
    .unwrap();
    println!(
        "idxmax(ties)={:?} idxmin={:?}",
        sc.idxmax().unwrap(),
        sc.idxmin().unwrap()
    );

    // Differential: fast path (validity-word scan) vs brute-force oracle over the
    // same Scalar values, across many seeds/sizes incl. negatives, all-NaN, ties,
    // word boundaries. Oracle mirrors the generic loop: strict >/<, first-present init.
    let mut fails = 0usize;
    for seed in 0..400u64 {
        let sz = (sm(seed as usize, 3) % 200) as usize; // 0..199 spans <1, =1, >1 words
        let nan_rate = sm(seed as usize, 4) % 4; // 0=none,1=~25%,2=~50%,3=all
        let neg = sm(seed as usize, 5).is_multiple_of(2); // negative-only ranges (tests INIT, not 0.0-fill)
        let vv: Vec<Scalar> = (0..sz)
            .map(|i| {
                let drop = match nan_rate {
                    0 => false,
                    1 => sm(i, seed * 7 + 1).is_multiple_of(4),
                    2 => sm(i, seed * 7 + 1).is_multiple_of(2),
                    _ => true,
                };
                if drop {
                    Scalar::Null(NullKind::NaN)
                } else {
                    let base = (sm(i, seed * 13 + 9) % 1000) as f64;
                    Scalar::Float64(if neg { -base - 1.0 } else { base })
                }
            })
            .collect();
        // brute oracle
        let (mut omax, mut omin): (Option<usize>, Option<usize>) = (None, None);
        let (mut bmax, mut bmin) = (f64::NEG_INFINITY, f64::INFINITY);
        for (i, s) in vv.iter().enumerate() {
            if let Scalar::Float64(v) = s
                && !v.is_nan()
            {
                if omax.is_none() || *v > bmax {
                    bmax = *v;
                    omax = Some(i);
                }
                if omin.is_none() || *v < bmin {
                    bmin = *v;
                    omin = Some(i);
                }
            }
        }
        let s = Series::new(
            "t",
            Index::from_range(0, sz as i64, 1),
            Column::from_values(vv).unwrap(),
        )
        .unwrap();
        let gmax = s.idxmax().ok().map(|l| {
            if let IndexLabel::Int64(x) = l {
                x as usize
            } else {
                usize::MAX
            }
        });
        let gmin = s.idxmin().ok().map(|l| {
            if let IndexLabel::Int64(x) = l {
                x as usize
            } else {
                usize::MAX
            }
        });
        if gmax != omax || gmin != omin {
            fails += 1;
            if fails <= 5 {
                println!(
                    "DIFF seed={seed} sz={sz} nan={nan_rate} neg={neg} max fp={gmax:?} oracle={omax:?} min fp={gmin:?} oracle={omin:?}"
                );
            }
        }
    }
    println!("differential: {} fails / 400", fails);
    assert_eq!(fails, 0, "idxmax/idxmin fast path diverged from oracle");
}
