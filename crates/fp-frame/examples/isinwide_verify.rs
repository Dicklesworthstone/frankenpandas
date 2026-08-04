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
    let n = 50000usize;
    let nn = 5000usize;
    let hv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % (n as u64)) as i64) * 2654435761))
        .collect();
    let needles: Vec<Scalar> = (0..nn)
        .map(|i| Scalar::Int64(((sm(i, 3) % (n as u64)) as i64) * 2654435761))
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let s = Series::new("s", idx, Column::from_values(hv.clone()).unwrap()).unwrap();
    let r = s.isin(&needles).unwrap();
    let col = r.column().values();
    let mut f = std::fs::File::create("/tmp/fp_isinwide.txt").unwrap();
    writeln!(
        f,
        "H\t{}",
        hv.iter()
            .map(|x| if let Scalar::Int64(v) = x {
                v.to_string()
            } else {
                "NA".into()
            })
            .collect::<Vec<_>>()
            .join(",")
    )
    .unwrap();
    writeln!(
        f,
        "N\t{}",
        needles
            .iter()
            .map(|x| if let Scalar::Int64(v) = x {
                v.to_string()
            } else {
                "NA".into()
            })
            .collect::<Vec<_>>()
            .join(",")
    )
    .unwrap();
    writeln!(
        f,
        "R\t{}",
        col.iter()
            .map(|x| match x {
                Scalar::Bool(b) =>
                    if *b {
                        "1"
                    } else {
                        "0"
                    },
                _ => "?",
            }
            .to_string())
            .collect::<Vec<_>>()
            .join("")
    )
    .unwrap();
    println!("wrote");
}
