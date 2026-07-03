use fp_frame::DataFrame;
use fp_index::Index;
use fp_columnar::Column;
use fp_types::Scalar;
use fp_join::{merge_dataframes_on, JoinType};
use std::collections::BTreeMap;
fn sm(i: usize, s: u64) -> u64 { let mut h=(i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15); h=(h^(h>>30)).wrapping_mul(0xBF58476D1CE4E5B9); h^(h>>31) }
fn timeit(l:&str, mut f: impl FnMut()){ let mut b=u128::MAX; for _ in 0..6 { let t=std::time::Instant::now(); f(); b=b.min(t.elapsed().as_nanos()); } println!("{l}: {:.2}ms", b as f64/1e6); }
// 3 wide-i64 key cols (so packed-dense span overflows -> Scalar composite path) + f64 payload
fn mk(n:usize, seed:u64, payload:&str)->DataFrame{
    let idx=Index::from_range(0,n as i64,1);
    let mut m=BTreeMap::new(); let mut order=vec![];
    let k0:Vec<Scalar>=(0..n).map(|i| Scalar::Int64(((sm(i,seed)%700_000) as i64)*7919)).collect();
    let k1:Vec<Scalar>=(0..n).map(|i| Scalar::Int64(((sm(i,seed+1)%400) as i64)*104729)).collect();
    let k2:Vec<Scalar>=(0..n).map(|i| Scalar::Int64(((sm(i,seed+2)%400) as i64)*1299709)).collect();
    let v:Vec<Scalar>=(0..n).map(|i| Scalar::Float64((sm(i,seed+3)%1000) as f64)).collect();
    for (nm,col) in [("k0",k0),("k1",k1),("k2",k2),(payload,v)] {
        m.insert(nm.to_string(),Column::from_values(col).unwrap()); order.push(nm.to_string());
    }
    DataFrame::new_with_column_order(idx,m,order).unwrap()
}
fn main(){
    let n=1_000_000usize;
    let left=mk(n,1,"lv");
    let right=mk(n,1,"rv"); // same seed -> shared keys, one-to-one-ish matches
    timeit("merge INNER 3 wide-i64 keys (1M)", || {
        std::hint::black_box(merge_dataframes_on(&left,&right,&["k0","k1","k2"],JoinType::Inner).unwrap());
    });
}
