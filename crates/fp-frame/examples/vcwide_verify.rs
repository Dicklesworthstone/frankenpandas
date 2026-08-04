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
    // wide range but with real duplicates so counts vary (and ties exist)
    let n = 40000usize;
    let wv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % (n as u64 / 8)) as i64) * 104729))
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let s = Series::new("s", idx, Column::from_values(wv.clone()).unwrap()).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_vcwide.txt").unwrap();
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
    let r = s.value_counts().unwrap();
    let lbls = r.index().labels();
    let col = r.column().values();
    let mut lines = vec![];
    for i in 0..r.len() {
        let l = match &lbls[i] {
            fp_index::IndexLabel::Int64(v) => v.to_string(),
            o => format!("{o:?}"),
        };
        let c = match &col[i] {
            Scalar::Int64(v) => v.to_string(),
            o => format!("{o:?}"),
        };
        lines.push(format!("{l}:{c}"));
    }
    writeln!(f, "VC\t{}", lines.join(",")).unwrap();
    println!("wrote /tmp/fp_vcwide.txt");
}
