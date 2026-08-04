use std::io::Write;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n = 30000usize;
    let wv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % (n as u64 / 3)) as i64) * 7919))
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let s = Series::new("s", idx, Column::from_values(wv.clone()).unwrap()).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_uniqwide.txt").unwrap();
    let vs: Vec<String> = wv
        .iter()
        .map(|x| {
            if let Scalar::Int64(v) = x {
                v.to_string()
            } else {
                "NA".into()
            }
        })
        .collect();
    writeln!(f, "V\t{}", vs.join(",")).unwrap();
    let u = s.unique();
    let us: Vec<String> = u
        .iter()
        .map(|x| {
            if let Scalar::Int64(v) = x {
                v.to_string()
            } else {
                "NA".into()
            }
        })
        .collect();
    writeln!(f, "UNIQUE\t{}", us.join(",")).unwrap();
    writeln!(f, "NUNIQUE\t{}", s.nunique()).unwrap();
    println!("wrote /tmp/fp_uniqwide.txt");
}
