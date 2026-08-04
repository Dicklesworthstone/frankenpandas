use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let mut d = 0u64;
    for seed in 0..40u64 {
        let n = (sm(seed as usize, 3) % 2000 + 1) as usize;
        let card = sm(seed as usize, 4) % 20 + 1;
        let mut m = BTreeMap::new();
        m.insert(
            "c".to_string(),
            Column::from_values(
                (0..n)
                    .map(|i| Scalar::Utf8(format!("v{}", sm(i, seed * 7 + 1) % card)))
                    .collect(),
            )
            .unwrap(),
        );
        let df = DataFrame::new_with_column_order(
            Index::from_range(0, n as i64, 1),
            m,
            vec!["c".into()],
        )
        .unwrap();
        let r = df.get_dummies(&["c"]).unwrap();
        for name in r.column_names().iter() {
            d = d.wrapping_mul(1099511628211);
            for b in name.bytes() {
                d = d.wrapping_add(b as u64);
            }
            if let Some(col) = r.column(name) {
                for v in col.values().iter() {
                    let t = match v {
                        Scalar::Bool(true) => 1u64,
                        Scalar::Bool(false) => 2,
                        _ => 9,
                    };
                    d = d.wrapping_mul(1099511628211).wrapping_add(t);
                }
            }
        }
    }
    println!("dum_digest={d}");
}
