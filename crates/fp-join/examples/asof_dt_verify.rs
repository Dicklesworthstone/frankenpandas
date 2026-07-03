use fp_frame::DataFrame;
use fp_index::Index;
use fp_columnar::Column;
use fp_types::Scalar;
use fp_join::{merge_asof, AsofDirection};
use std::collections::BTreeMap;
fn mk(keys:&[i64], vals:&[i64], vn:&str)->DataFrame{
    let idx=Index::from_range(0,keys.len() as i64,1);
    let mut m=BTreeMap::new();
    let base=1_600_000_000_000_000_000i64;
    m.insert("t".to_string(),Column::from_values(keys.iter().map(|&k|Scalar::Datetime64(base+k*1_000_000_000)).collect()).unwrap());
    m.insert(vn.to_string(),Column::from_values(vals.iter().map(|&v|Scalar::Int64(v)).collect()).unwrap());
    DataFrame::new_with_column_order(idx,m,vec!["t".to_string(),vn.to_string()]).unwrap()
}
fn main(){
    let left=mk(&[1,5,10,15,20],&[100,101,102,103,104],"lv");
    let right=mk(&[2,3,7,16,25],&[200,201,202,203,204],"rv");
    for (tag,d) in [("bwd",AsofDirection::Backward),("fwd",AsofDirection::Forward),("near",AsofDirection::Nearest)] {
        let df=merge_asof(&left,&right,"t",d).unwrap();
        let n=df.index.labels().len();
        let g=|c:&str,i:usize|->String{ df.columns.get(c).map(|col| match &col.values()[i]{Scalar::Int64(x)=>x.to_string(),Scalar::Float64(f)=>if f.is_nan(){"NA".into()}else{format!("{f:.0}")},Scalar::Null(_)=>"NA".into(),Scalar::Datetime64(ns)=>((ns-1_600_000_000_000_000_000)/1_000_000_000).to_string(),o=>format!("{o:?}")}).unwrap_or("_".into())};
        print!("{tag} {n} ");
        for i in 0..n { print!("[{},{},{}] ", g("t",i),g("lv",i),g("rv",i)); }
        println!();
    }
}
