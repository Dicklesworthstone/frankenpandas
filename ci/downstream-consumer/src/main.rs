//! The README Quick Example, compiled as a downstream crate in release and
//! checked for aarch64 (br-frankenpandas-rc0923-epic-buildable-everywhere-0zz8y.2).
//! Expected values are pandas 2.2.3's for the same pipeline:
//!   df[df.age > 28].groupby('city').agg(avg_age=('age', 'mean'), count=('age', 'count'))
//!   -> index ['NYC'], avg_age [32.5], count [2]
use frankenpandas::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("name,age,city\nAlice,30,NYC\nBob,25,LA\nCarol,35,NYC")?;
    let adults = df.query("age > 28")?;
    let summary = adults
        .groupby(&["city"])?
        .agg_named(&[("avg_age", "age", "mean"), ("count", "age", "count")])?;

    assert_eq!(summary.index().labels(), &[IndexLabel::Utf8("NYC".to_owned())]);
    let avg_age = summary.column("avg_age").ok_or("avg_age column")?;
    assert_eq!(avg_age.values(), &[Scalar::Float64(32.5)]);
    let count = summary.column("count").ok_or("count column")?;
    assert_eq!(count.values(), &[Scalar::Int64(2)]);

    let json = write_json_string(&summary, JsonOrient::Records)?;
    assert!(json.contains("32.5"), "{json}");
    assert!(!write_feather_bytes(&summary)?.is_empty());
    assert!(write_html_string(&summary)?.contains("NYC"));
    assert!(summary.to_markdown(true, None)?.contains("NYC"));

    println!("downstream consumer OK: {json}");
    Ok(())
}
