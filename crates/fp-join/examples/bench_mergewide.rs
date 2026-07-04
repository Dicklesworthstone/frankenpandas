use std::collections::BTreeMap;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_columnar::Column;
use fp_types::Scalar;
use fp_join::{merge_dataframes, JoinType};
fn sm(i: usize, s: u64) -> u64 { let mut h=(i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15); h=(h^(h>>30)).wrapping_mul(0xBF58476D1CE4E5B9); h^(h>>31) }
fn timeit(l:&str, mut f: impl FnMut()){ let mut b=u128::MAX; for _ in 0..5 { let t=std::time::Instant::now(); f(); b=b.min(t.elapsed().as_nanos()); } println!("{l}: {:.2}ms", b as f64/1e6); }
fn mkdf(n:usize, keyseed:u64, wide:bool)->DataFrame{
    let idx=Index::from_range(0,n as i64,1);
    let keys:Vec<Scalar>=(0..n).map(|i| Scalar::Int64(if wide {((sm(i,keyseed)%(2*n as u64)) as i64)*2654435761} else {(sm(i,keyseed)%(n as u64)) as i64})).collect();
    let val:Vec<Scalar>=(0..n).map(|i| Scalar::Float64((sm(i,7)%1000) as f64)).collect();
    let mut m=BTreeMap::new();
    m.insert("k".into(), Column::from_values(keys).unwrap());
    let vn = if keyseed==1 {"lval"} else {"rval"};
    m.insert(vn.into(), Column::from_values(val).unwrap());
    DataFrame::new_with_column_order(idx, m, vec!["k".into(), vn.into()]).unwrap()
}
fn main(){
    let n=1_000_000usize;
    let l=mkdf(n,1,true); let r=mkdf(n,2,true);
    timeit("merge inner wide-i64", || { std::hint::black_box(merge_dataframes(&l,&r,"k",JoinType::Inner).unwrap()); });
    timeit("merge left wide-i64", || { std::hint::black_box(merge_dataframes(&l,&r,"k",JoinType::Left).unwrap()); });
    timeit("merge right wide-i64", || { std::hint::black_box(merge_dataframes(&l,&r,"k",JoinType::Right).unwrap()); });
    timeit("merge outer wide-i64", || { std::hint::black_box(merge_dataframes(&l,&r,"k",JoinType::Outer).unwrap()); });
}
