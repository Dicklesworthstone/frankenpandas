#![forbid(unsafe_code)]

use fp_frame::DataFrame;
use fp_join::{JoinType, merge_dataframes};
use fp_types::Scalar;

type AnyError = Box<dyn std::error::Error>;

fn smoke_frame(value_name: &str, ids: &[i64], values: &[i64]) -> Result<DataFrame, AnyError> {
    if ids.len() != values.len() {
        return Err(std::io::Error::other(format!(
            "smoke frame {value_name} has mismatched ids={} values={}",
            ids.len(),
            values.len()
        ))
        .into());
    }

    DataFrame::from_dict(
        &["id", value_name],
        vec![
            ("id", ids.iter().copied().map(Scalar::Int64).collect()),
            (
                value_name,
                values.iter().copied().map(Scalar::Int64).collect(),
            ),
        ],
    )
    .map_err(|err| {
        std::io::Error::other(format!("build {value_name} smoke frame failed: {err}")).into()
    })
}

fn expect_column(
    frame: &fp_join::MergedDataFrame,
    name: &str,
    expected: Vec<Scalar>,
) -> Result<(), AnyError> {
    let actual = frame
        .columns
        .get(name)
        .ok_or_else(|| std::io::Error::other(format!("join smoke output missing {name} column")))?
        .values()
        .to_vec();
    if actual == expected {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "join smoke column {name} mismatch: actual={actual:?} expected={expected:?}"
        ))
        .into())
    }
}

fn main() -> Result<(), AnyError> {
    let left = smoke_frame("left_value", &[1, 2, 3], &[10, 20, 30])?;
    let right = smoke_frame("right_value", &[2, 3, 4], &[200, 300, 400])?;
    let joined = merge_dataframes(&left, &right, "id", JoinType::Inner)?;

    expect_column(&joined, "id", vec![Scalar::Int64(2), Scalar::Int64(3)])?;
    expect_column(
        &joined,
        "left_value",
        vec![Scalar::Int64(20), Scalar::Int64(30)],
    )?;
    expect_column(
        &joined,
        "right_value",
        vec![Scalar::Int64(200), Scalar::Int64(300)],
    )?;

    println!("fp-join smoke ok rows=2 join_type=inner key=id");
    Ok(())
}
