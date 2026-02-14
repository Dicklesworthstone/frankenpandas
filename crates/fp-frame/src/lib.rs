#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fp_columnar::{ArithmeticOp, Column, ColumnError};
use fp_index::{Index, IndexError, IndexLabel, align_union, validate_alignment_plan};
use fp_runtime::{DecisionAction, EvidenceLedger, RuntimeMode, RuntimePolicy};
use fp_types::Scalar;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("index length ({index_len}) does not match column length ({column_len})")]
    LengthMismatch { index_len: usize, column_len: usize },
    #[error("duplicate index labels are unsupported in strict mode for MVP slice")]
    DuplicateIndexUnsupported,
    #[error("compatibility gate rejected operation: {0}")]
    CompatibilityRejected(String),
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Index(#[from] IndexError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    name: String,
    index: Index,
    column: Column,
}

impl Series {
    pub fn new(name: impl Into<String>, index: Index, column: Column) -> Result<Self, FrameError> {
        if index.len() != column.len() {
            return Err(FrameError::LengthMismatch {
                index_len: index.len(),
                column_len: column.len(),
            });
        }

        Ok(Self {
            name: name.into(),
            index,
            column,
        })
    }

    pub fn from_values(
        name: impl Into<String>,
        index_labels: Vec<IndexLabel>,
        values: Vec<Scalar>,
    ) -> Result<Self, FrameError> {
        let index = Index::new(index_labels);
        let column = Column::from_values(values)?;
        Self::new(name, index, column)
    }

    /// Construct a Series from key-value pairs (dict-style).
    ///
    /// Keys become the index labels, values become the column.
    /// Matches `pd.Series({"a": 1, "b": 2})`.
    pub fn from_pairs(
        name: impl Into<String>,
        pairs: Vec<(IndexLabel, Scalar)>,
    ) -> Result<Self, FrameError> {
        let (labels, values): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        Self::from_values(name, labels, values)
    }

    /// Broadcast a single scalar value across the given index.
    ///
    /// Matches `pd.Series(5, index=[0, 1, 2])`.
    pub fn broadcast(
        name: impl Into<String>,
        value: Scalar,
        index_labels: Vec<IndexLabel>,
    ) -> Result<Self, FrameError> {
        let n = index_labels.len();
        let values = vec![value; n];
        Self::from_values(name, index_labels, values)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn index(&self) -> &Index {
        &self.index
    }

    #[must_use]
    pub fn column(&self) -> &Column {
        &self.column
    }

    #[must_use]
    pub fn values(&self) -> &[Scalar] {
        self.column.values()
    }

    /// Core binary arithmetic: align indexes, reindex columns, apply op.
    fn binary_op_with_policy(
        &self,
        other: &Self,
        op: ArithmeticOp,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<Self, FrameError> {
        if self.index.has_duplicates() || other.index.has_duplicates() {
            policy.decide_unknown_feature(
                "index_alignment",
                "duplicate labels are not yet fully modeled",
                ledger,
            );
            if matches!(policy.mode, RuntimeMode::Strict) {
                return Err(FrameError::DuplicateIndexUnsupported);
            }
        }

        let plan = align_union(&self.index, &other.index);
        validate_alignment_plan(&plan)?;

        let left = self.column.reindex_by_positions(&plan.left_positions)?;
        let right = other.column.reindex_by_positions(&plan.right_positions)?;

        let action = policy.decide_join_admission(plan.union_index.len(), ledger);
        if matches!(action, DecisionAction::Reject) {
            return Err(FrameError::CompatibilityRejected(
                "runtime policy rejected alignment admission".to_owned(),
            ));
        }

        let column = left.binary_numeric(&right, op)?;

        let op_symbol = match op {
            ArithmeticOp::Add => "+",
            ArithmeticOp::Sub => "-",
            ArithmeticOp::Mul => "*",
            ArithmeticOp::Div => "/",
        };

        let out_name = if self.name == other.name {
            self.name.clone()
        } else {
            format!("{}{op_symbol}{}", self.name, other.name)
        };

        Self::new(out_name, plan.union_index, column)
    }

    pub fn add_with_policy(
        &self,
        other: &Self,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<Self, FrameError> {
        self.binary_op_with_policy(other, ArithmeticOp::Add, policy, ledger)
    }

    pub fn add(&self, other: &Self) -> Result<Self, FrameError> {
        let mut ledger = EvidenceLedger::new();
        self.add_with_policy(other, &RuntimePolicy::strict(), &mut ledger)
    }

    pub fn sub_with_policy(
        &self,
        other: &Self,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<Self, FrameError> {
        self.binary_op_with_policy(other, ArithmeticOp::Sub, policy, ledger)
    }

    pub fn sub(&self, other: &Self) -> Result<Self, FrameError> {
        let mut ledger = EvidenceLedger::new();
        self.sub_with_policy(other, &RuntimePolicy::strict(), &mut ledger)
    }

    pub fn mul_with_policy(
        &self,
        other: &Self,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<Self, FrameError> {
        self.binary_op_with_policy(other, ArithmeticOp::Mul, policy, ledger)
    }

    pub fn mul(&self, other: &Self) -> Result<Self, FrameError> {
        let mut ledger = EvidenceLedger::new();
        self.mul_with_policy(other, &RuntimePolicy::strict(), &mut ledger)
    }

    pub fn div_with_policy(
        &self,
        other: &Self,
        policy: &RuntimePolicy,
        ledger: &mut EvidenceLedger,
    ) -> Result<Self, FrameError> {
        self.binary_op_with_policy(other, ArithmeticOp::Div, policy, ledger)
    }

    pub fn div(&self, other: &Self) -> Result<Self, FrameError> {
        let mut ledger = EvidenceLedger::new();
        self.div_with_policy(other, &RuntimePolicy::strict(), &mut ledger)
    }

    /// Return the number of elements in this Series.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Return true if this Series has zero elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

/// Concatenate multiple Series along axis 0 (row-wise).
///
/// Matches `pd.concat([s1, s2, ...])` semantics:
/// - Index labels are concatenated in order (duplicates preserved).
/// - Values are type-coerced to a common dtype.
/// - Empty input returns an empty Series named "concat".
pub fn concat_series(series_list: &[&Series]) -> Result<Series, FrameError> {
    if series_list.is_empty() {
        return Series::from_values("concat", Vec::new(), Vec::new());
    }

    let total_len: usize = series_list.iter().map(|s| s.len()).sum();
    let mut labels = Vec::with_capacity(total_len);
    let mut values = Vec::with_capacity(total_len);

    for s in series_list {
        labels.extend_from_slice(s.index().labels());
        values.extend_from_slice(s.values());
    }

    let name = if series_list.len() == 1 {
        series_list[0].name().to_owned()
    } else {
        "concat".to_owned()
    };

    Series::from_values(name, labels, values)
}

/// Concatenate DataFrames along axis 0 (row-wise).
///
/// Matches `pd.concat([df1, df2, ...], axis=0)` semantics:
/// - All DataFrames must share the same column names.
/// - Index labels are concatenated in order (duplicates preserved).
/// - Empty input returns an empty DataFrame.
pub fn concat_dataframes(frames: &[&DataFrame]) -> Result<DataFrame, FrameError> {
    if frames.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new());
    }

    let col_names: Vec<&String> = frames[0].columns().keys().collect();

    // Verify all frames have the same columns.
    for (i, frame) in frames.iter().enumerate().skip(1) {
        let frame_cols: Vec<&String> = frame.columns().keys().collect();
        if frame_cols != col_names {
            return Err(FrameError::CompatibilityRejected(format!(
                "frame[{i}] columns do not match frame[0]"
            )));
        }
    }

    // Concatenate index labels.
    let total_len: usize = frames.iter().map(|f| f.len()).sum();
    let mut labels = Vec::with_capacity(total_len);
    for frame in frames {
        labels.extend_from_slice(frame.index().labels());
    }
    let index = Index::new(labels);

    // Concatenate each column.
    let mut columns = BTreeMap::new();
    for col_name in &col_names {
        let mut values = Vec::with_capacity(total_len);
        for frame in frames {
            values.extend_from_slice(frame.column(col_name).unwrap().values());
        }
        columns.insert((*col_name).clone(), Column::from_values(values)?);
    }

    DataFrame::new(index, columns)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataFrame {
    index: Index,
    columns: BTreeMap<String, Column>,
}

impl DataFrame {
    pub fn new(index: Index, columns: BTreeMap<String, Column>) -> Result<Self, FrameError> {
        for column in columns.values() {
            if column.len() != index.len() {
                return Err(FrameError::LengthMismatch {
                    index_len: index.len(),
                    column_len: column.len(),
                });
            }
        }

        Ok(Self { index, columns })
    }

    /// AG-05: Pre-compute N-way union index across all series first, then
    /// reindex each column exactly once. Eliminates O(N²) iterative
    /// realignment where N = number of series.
    pub fn from_series(series_list: Vec<Series>) -> Result<Self, FrameError> {
        if series_list.is_empty() {
            return Self::new(Index::new(Vec::new()), BTreeMap::new());
        }

        // Phase 1: Compute global union index across all series.
        let mut union_index = series_list[0].index.clone();
        for series in &series_list[1..] {
            let plan = align_union(&union_index, &series.index);
            validate_alignment_plan(&plan)?;
            union_index = plan.union_index;
        }

        // Phase 2: Reindex each series column exactly once against the final union index.
        let mut columns = BTreeMap::new();
        for series in series_list {
            let plan = align_union(&union_index, &series.index);
            // The right_positions map from the union to this series's positions.
            // Since union_index already contains all labels, the union_index in
            // this plan equals our pre-computed union. We use right_positions to
            // locate each series's values within the union.
            let aligned_column = series.column.reindex_by_positions(&plan.right_positions)?;
            columns.insert(series.name, aligned_column);
        }

        Self::new(union_index, columns)
    }

    /// Construct a DataFrame from a dict of column vectors.
    ///
    /// Matches `pd.DataFrame({"a": [1, 2], "b": [3, 4]})`.
    /// All vectors must have the same length. Index is auto-generated
    /// as 0..n (RangeIndex-style).
    ///
    /// `column_order` controls insertion order since BTreeMap sorts by key.
    pub fn from_dict(
        column_order: &[&str],
        data: Vec<(&str, Vec<Scalar>)>,
    ) -> Result<Self, FrameError> {
        if data.is_empty() {
            return Self::new(Index::new(Vec::new()), BTreeMap::new());
        }

        let n = data[0].1.len();
        let index = Index::new((0..n as i64).map(IndexLabel::from).collect());

        let mut columns = BTreeMap::new();
        for (name, values) in data {
            if values.len() != n {
                return Err(FrameError::LengthMismatch {
                    index_len: n,
                    column_len: values.len(),
                });
            }
            columns.insert(name.to_owned(), Column::from_values(values)?);
        }

        // If column_order is provided, verify all requested columns exist.
        // BTreeMap handles ordering by key, but column_order documents intent.
        if !column_order.is_empty() {
            for &col in column_order {
                if !columns.contains_key(col) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "column '{col}' not found in data"
                    )));
                }
            }
        }

        Self::new(index, columns)
    }

    /// Construct a DataFrame from a dict of column vectors with an explicit index.
    ///
    /// Matches `pd.DataFrame({"a": [1, 2]}, index=["x", "y"])`.
    pub fn from_dict_with_index(
        data: Vec<(&str, Vec<Scalar>)>,
        index_labels: Vec<IndexLabel>,
    ) -> Result<Self, FrameError> {
        let n = index_labels.len();
        let index = Index::new(index_labels);

        let mut columns = BTreeMap::new();
        for (name, values) in data {
            if values.len() != n {
                return Err(FrameError::LengthMismatch {
                    index_len: n,
                    column_len: values.len(),
                });
            }
            columns.insert(name.to_owned(), Column::from_values(values)?);
        }

        Self::new(index, columns)
    }

    /// Return a new DataFrame with only the specified columns, in order.
    ///
    /// Matches `df[["a", "c"]]` column selection in pandas.
    pub fn select_columns(&self, names: &[&str]) -> Result<Self, FrameError> {
        let mut columns = BTreeMap::new();
        for &name in names {
            match self.columns.get(name) {
                Some(col) => {
                    columns.insert(name.to_owned(), col.clone());
                }
                None => {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "column '{name}' not found"
                    )));
                }
            }
        }
        Self::new(self.index.clone(), columns)
    }

    /// Return the number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Return true if the DataFrame has zero rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Return the number of columns.
    #[must_use]
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    #[must_use]
    pub fn index(&self) -> &Index {
        &self.index
    }

    #[must_use]
    pub fn columns(&self) -> &BTreeMap<String, Column> {
        &self.columns
    }

    #[must_use]
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.get(name)
    }
}

#[cfg(test)]
mod tests {
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::{NullKind, Scalar};

    use super::{DataFrame, FrameError, IndexLabel, Series};

    #[test]
    fn series_add_aligns_on_union_index() {
        let left = Series::from_values(
            "left",
            vec![1_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(30)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec![2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(2), Scalar::Int64(4)],
        )
        .expect("right");

        let out = left
            .add_with_policy(
                &right,
                &RuntimePolicy::hardened(Some(100)),
                &mut EvidenceLedger::new(),
            )
            .expect("add should pass");

        assert_eq!(
            out.index().labels(),
            &[1_i64.into(), 3_i64.into(), 2_i64.into()]
        );
        assert_eq!(
            out.values(),
            &[
                Scalar::Null(NullKind::Null),
                Scalar::Int64(34),
                Scalar::Null(NullKind::Null)
            ]
        );
    }

    #[test]
    fn strict_mode_rejects_duplicate_indices() {
        let left = Series::from_values(
            "left",
            vec![IndexLabel::from("a"), IndexLabel::from("a")],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right =
            Series::from_values("right", vec![IndexLabel::from("a")], vec![Scalar::Int64(3)])
                .expect("right");

        let err = left.add(&right).expect_err("strict mode should reject");
        assert!(matches!(err, FrameError::DuplicateIndexUnsupported));
    }

    #[test]
    fn concat_series_basic() {
        use super::concat_series;

        let s1 = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("s1");

        let s2 = Series::from_values(
            "b",
            vec![3_i64.into(), 4_i64.into()],
            vec![Scalar::Int64(30), Scalar::Int64(40)],
        )
        .expect("s2");

        let result = concat_series(&[&s1, &s2]).expect("concat");
        assert_eq!(result.len(), 4);
        assert_eq!(
            result.index().labels(),
            &[1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()]
        );
        assert_eq!(
            result.values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40)
            ]
        );
    }

    #[test]
    fn concat_series_preserves_duplicates() {
        use super::concat_series;

        let s1 = Series::from_values(
            "a",
            vec!["x".into(), "y".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("s1");

        let s2 = Series::from_values(
            "b",
            vec!["x".into(), "z".into()],
            vec![Scalar::Int64(3), Scalar::Int64(4)],
        )
        .expect("s2");

        let result = concat_series(&[&s1, &s2]).expect("concat");
        assert_eq!(result.len(), 4);
        // "x" appears twice (from both series)
        assert_eq!(
            result.index().labels(),
            &["x".into(), "y".into(), "x".into(), "z".into()]
        );
    }

    #[test]
    fn concat_series_empty_input() {
        use super::concat_series;
        let result = concat_series(&[]).expect("empty concat");
        assert_eq!(result.len(), 0);
        assert!(result.is_empty());
    }

    #[test]
    fn concat_series_single_input() {
        use super::concat_series;

        let s =
            Series::from_values("only", vec![1_i64.into()], vec![Scalar::Int64(42)]).expect("s");

        let result = concat_series(&[&s]).expect("single concat");
        assert_eq!(result.len(), 1);
        assert_eq!(result.name(), "only"); // preserves name for single input
        assert_eq!(result.values(), &[Scalar::Int64(42)]);
    }

    #[test]
    fn concat_series_with_nulls() {
        use super::concat_series;

        let s1 = Series::from_values("a", vec![1_i64.into()], vec![Scalar::Null(NullKind::Null)])
            .expect("s1");

        let s2 = Series::from_values("b", vec![2_i64.into()], vec![Scalar::Int64(10)]).expect("s2");

        let result = concat_series(&[&s1, &s2]).expect("concat with nulls");
        assert_eq!(result.len(), 2);
        assert!(result.values()[0].is_missing());
        assert_eq!(result.values()[1], Scalar::Int64(10));
    }

    #[test]
    fn concat_series_three_series() {
        use super::concat_series;

        let s1 = Series::from_values("a", vec![1_i64.into()], vec![Scalar::Int64(1)]).expect("s1");
        let s2 = Series::from_values("b", vec![2_i64.into()], vec![Scalar::Int64(2)]).expect("s2");
        let s3 = Series::from_values("c", vec![3_i64.into()], vec![Scalar::Int64(3)]).expect("s3");

        let result = concat_series(&[&s1, &s2, &s3]).expect("triple concat");
        assert_eq!(result.len(), 3);
        assert_eq!(
            result.values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
    }

    #[test]
    fn dataframe_from_series_reindexes_existing_columns() {
        let s1 = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("s1");
        let s2 = Series::from_values(
            "b",
            vec![2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("s2");

        let df = DataFrame::from_series(vec![s1, s2]).expect("frame");
        assert_eq!(
            df.index().labels(),
            &[1_i64.into(), 2_i64.into(), 3_i64.into()]
        );
        assert_eq!(
            df.column("a").expect("a").values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null)
            ]
        );
    }

    // ---- Series sub/mul/div tests ----

    fn make_pair() -> (Series, Series) {
        let left = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec![2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(7), Scalar::Int64(9)],
        )
        .unwrap();
        (left, right)
    }

    fn hardened_add(left: &Series, right: &Series) -> Series {
        left.add_with_policy(
            right,
            &RuntimePolicy::hardened(Some(100)),
            &mut EvidenceLedger::new(),
        )
        .unwrap()
    }

    fn hardened_sub(left: &Series, right: &Series) -> Series {
        left.sub_with_policy(
            right,
            &RuntimePolicy::hardened(Some(100)),
            &mut EvidenceLedger::new(),
        )
        .unwrap()
    }

    fn hardened_mul(left: &Series, right: &Series) -> Series {
        left.mul_with_policy(
            right,
            &RuntimePolicy::hardened(Some(100)),
            &mut EvidenceLedger::new(),
        )
        .unwrap()
    }

    fn hardened_div(left: &Series, right: &Series) -> Series {
        left.div_with_policy(
            right,
            &RuntimePolicy::hardened(Some(100)),
            &mut EvidenceLedger::new(),
        )
        .unwrap()
    }

    #[test]
    fn series_sub_aligns_on_union_index() {
        let (left, right) = make_pair();
        let out = hardened_sub(&left, &right);

        // Union: [1, 2, 3, 4]. Overlap at 2 (20-5=15) and 3 (30-7=23).
        assert_eq!(
            out.index().labels(),
            &[1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()]
        );
        assert_eq!(
            out.values(),
            &[
                Scalar::Null(NullKind::Null),
                Scalar::Int64(15),
                Scalar::Int64(23),
                Scalar::Null(NullKind::Null)
            ]
        );
        assert_eq!(out.name(), "x-y");
    }

    #[test]
    fn series_mul_aligns_on_union_index() {
        let (left, right) = make_pair();
        let out = hardened_mul(&left, &right);

        // Overlap at 2 (20*5=100) and 3 (30*7=210).
        assert_eq!(
            out.values(),
            &[
                Scalar::Null(NullKind::Null),
                Scalar::Int64(100),
                Scalar::Int64(210),
                Scalar::Null(NullKind::Null)
            ]
        );
        assert_eq!(out.name(), "x*y");
    }

    #[test]
    fn series_div_aligns_on_union_index() {
        let (left, right) = make_pair();
        let out = hardened_div(&left, &right);

        // Division promotes to Float64. Overlap at 2 (20/5=4.0) and 3 (30/7≈4.2857).
        assert_eq!(
            out.index().labels(),
            &[1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()]
        );
        assert!(out.values()[0].is_missing());
        assert_eq!(out.values()[1], Scalar::Float64(4.0));
        let v = match out.values()[2] {
            Scalar::Float64(f) => f,
            _ => panic!("expected Float64"),
        };
        assert!((v - 30.0 / 7.0).abs() < 1e-10);
        assert!(out.values()[3].is_missing());
        assert_eq!(out.name(), "x/y");
    }

    #[test]
    fn series_add_same_name_preserves_name() {
        let left = Series::from_values("val", vec![1_i64.into()], vec![Scalar::Int64(10)]).unwrap();
        let right = Series::from_values("val", vec![1_i64.into()], vec![Scalar::Int64(5)]).unwrap();
        let out = hardened_add(&left, &right);
        assert_eq!(out.name(), "val");
        assert_eq!(out.values(), &[Scalar::Int64(15)]);
    }

    #[test]
    fn series_div_by_zero_produces_inf() {
        let left =
            Series::from_values("a", vec![1_i64.into()], vec![Scalar::Float64(10.0)]).unwrap();
        let right =
            Series::from_values("b", vec![1_i64.into()], vec![Scalar::Float64(0.0)]).unwrap();
        let out = hardened_div(&left, &right);
        let v = match out.values()[0] {
            Scalar::Float64(f) => f,
            _ => panic!("expected Float64"),
        };
        assert!(v.is_infinite() && v.is_sign_positive());
    }

    #[test]
    fn series_arithmetic_with_nulls_propagates() {
        let left = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Null(NullKind::Null), Scalar::Int64(10)],
        )
        .unwrap();
        let right = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(3)],
        )
        .unwrap();

        let add = hardened_add(&left, &right);
        assert!(add.values()[0].is_missing());
        assert_eq!(add.values()[1], Scalar::Int64(13));

        let sub = hardened_sub(&left, &right);
        assert!(sub.values()[0].is_missing());
        assert_eq!(sub.values()[1], Scalar::Int64(7));

        let mul = hardened_mul(&left, &right);
        assert!(mul.values()[0].is_missing());
        assert_eq!(mul.values()[1], Scalar::Int64(30));

        let div = hardened_div(&left, &right);
        assert!(div.values()[0].is_missing());
    }

    #[test]
    fn series_arithmetic_float64_precision() {
        let left =
            Series::from_values("a", vec![1_i64.into()], vec![Scalar::Float64(0.1)]).unwrap();
        let right =
            Series::from_values("b", vec![1_i64.into()], vec![Scalar::Float64(0.2)]).unwrap();

        let add = hardened_add(&left, &right);
        let v = match add.values()[0] {
            Scalar::Float64(f) => f,
            _ => panic!("expected Float64"),
        };
        // IEEE 754: 0.1 + 0.2 != 0.3 exactly
        assert!((v - 0.3).abs() < 1e-15);
    }

    #[test]
    fn series_sub_strict_rejects_duplicates() {
        let left = Series::from_values(
            "a",
            vec!["x".into(), "x".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let right = Series::from_values("b", vec!["x".into()], vec![Scalar::Int64(3)]).unwrap();

        let err = left.sub(&right).expect_err("strict should reject");
        assert!(matches!(err, FrameError::DuplicateIndexUnsupported));
    }

    #[test]
    fn series_all_four_ops_identical_indexes() {
        let left = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let right = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(3), Scalar::Int64(4)],
        )
        .unwrap();

        let add = hardened_add(&left, &right);
        assert_eq!(add.values(), &[Scalar::Int64(13), Scalar::Int64(24)]);

        let sub = hardened_sub(&left, &right);
        assert_eq!(sub.values(), &[Scalar::Int64(7), Scalar::Int64(16)]);

        let mul = hardened_mul(&left, &right);
        assert_eq!(mul.values(), &[Scalar::Int64(30), Scalar::Int64(80)]);

        let div = hardened_div(&left, &right);
        let v0 = match div.values()[0] {
            Scalar::Float64(f) => f,
            _ => panic!("expected Float64"),
        };
        let v1 = match div.values()[1] {
            Scalar::Float64(f) => f,
            _ => panic!("expected Float64"),
        };
        assert!((v0 - 10.0 / 3.0).abs() < 1e-10);
        assert!((v1 - 5.0).abs() < 1e-10);
    }

    #[test]
    fn series_arithmetic_empty_series() {
        let left =
            Series::from_values("a", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();
        let right =
            Series::from_values("b", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();

        let add = hardened_add(&left, &right);
        assert!(add.is_empty());

        let sub = hardened_sub(&left, &right);
        assert!(sub.is_empty());
    }

    #[test]
    fn series_arithmetic_disjoint_indexes_all_null() {
        let left = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let right = Series::from_values(
            "b",
            vec![3_i64.into(), 4_i64.into()],
            vec![Scalar::Int64(30), Scalar::Int64(40)],
        )
        .unwrap();

        let add = hardened_add(&left, &right);
        assert_eq!(add.len(), 4);
        for v in add.values() {
            assert!(v.is_missing(), "disjoint indexes should produce all nulls");
        }
    }

    // ---- Construction parity tests (bd-2gi.12) ----

    #[test]
    fn series_from_pairs_basic() {
        let s = Series::from_pairs(
            "scores",
            vec![
                ("alice".into(), Scalar::Int64(95)),
                ("bob".into(), Scalar::Int64(87)),
            ],
        )
        .unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.index().labels(), &["alice".into(), "bob".into()]);
        assert_eq!(s.values(), &[Scalar::Int64(95), Scalar::Int64(87)]);
    }

    #[test]
    fn series_from_pairs_empty() {
        let s = Series::from_pairs("empty", Vec::new()).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn series_broadcast_basic() {
        let s = Series::broadcast(
            "fill",
            Scalar::Float64(7.5),
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        )
        .unwrap();
        assert_eq!(s.len(), 3);
        for v in s.values() {
            assert_eq!(*v, Scalar::Float64(7.5));
        }
    }

    #[test]
    fn series_broadcast_empty_index() {
        let s = Series::broadcast("x", Scalar::Int64(0), Vec::new()).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn series_broadcast_null_fills_with_null() {
        let s = Series::broadcast(
            "n",
            Scalar::Null(NullKind::Null),
            vec![1_i64.into(), 2_i64.into()],
        )
        .unwrap();
        for v in s.values() {
            assert!(v.is_missing());
        }
    }

    #[test]
    fn dataframe_from_dict_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        assert_eq!(df.len(), 2);
        assert_eq!(df.num_columns(), 2);
        assert_eq!(df.index().labels(), &[0_i64.into(), 1_i64.into()]);
        assert_eq!(
            df.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            df.column("b").unwrap().values(),
            &[Scalar::Int64(3), Scalar::Int64(4)]
        );
    }

    #[test]
    fn dataframe_from_dict_empty() {
        let df = DataFrame::from_dict(&[], Vec::new()).unwrap();
        assert!(df.is_empty());
        assert_eq!(df.num_columns(), 0);
    }

    #[test]
    fn dataframe_from_dict_mismatched_lengths() {
        let err = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3)]),
            ],
        )
        .expect_err("should reject mismatched lengths");
        assert!(matches!(err, FrameError::LengthMismatch { .. }));
    }

    #[test]
    fn dataframe_from_dict_with_index_custom_labels() {
        let df = DataFrame::from_dict_with_index(
            vec![("val", vec![Scalar::Int64(10), Scalar::Int64(20)])],
            vec!["x".into(), "y".into()],
        )
        .unwrap();

        assert_eq!(df.len(), 2);
        assert_eq!(df.index().labels(), &["x".into(), "y".into()]);
        assert_eq!(
            df.column("val").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
    }

    #[test]
    fn dataframe_from_dict_with_index_mismatch() {
        let err = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1)])],
            vec!["x".into(), "y".into()],
        )
        .expect_err("should reject length mismatch");
        assert!(matches!(err, FrameError::LengthMismatch { .. }));
    }

    #[test]
    fn dataframe_select_columns_basic() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Int64(2)]),
                ("c", vec![Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let subset = df.select_columns(&["c", "a"]).unwrap();
        assert_eq!(subset.num_columns(), 2);
        assert!(subset.column("a").is_some());
        assert!(subset.column("c").is_some());
        assert!(subset.column("b").is_none());
    }

    #[test]
    fn dataframe_select_columns_missing_rejects() {
        let df = DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(1)])]).unwrap();

        let err = df.select_columns(&["a", "z"]).expect_err("missing column");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
    }

    #[test]
    fn dataframe_from_dict_with_nulls() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![(
                "a",
                vec![
                    Scalar::Int64(1),
                    Scalar::Null(NullKind::Null),
                    Scalar::Int64(3),
                ],
            )],
        )
        .unwrap();

        assert_eq!(df.len(), 3);
        assert!(df.column("a").unwrap().values()[1].is_missing());
    }

    #[test]
    fn dataframe_len_and_num_columns() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                (
                    "x",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "y",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
            ],
        )
        .unwrap();

        assert_eq!(df.len(), 3);
        assert_eq!(df.num_columns(), 2);
        assert!(!df.is_empty());
    }

    // ---- DataFrame concat tests (bd-2gi.17) ----

    use super::concat_dataframes;

    #[test]
    fn concat_dataframes_basic() {
        let df1 = DataFrame::from_dict(
            &["a", "b"],
            vec![("a", vec![Scalar::Int64(1)]), ("b", vec![Scalar::Int64(2)])],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["a", "b"],
            vec![("a", vec![Scalar::Int64(3)]), ("b", vec![Scalar::Int64(4)])],
        )
        .unwrap();

        let result = concat_dataframes(&[&df1, &df2]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.num_columns(), 2);
        assert_eq!(
            result.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(3)]
        );
        assert_eq!(
            result.column("b").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(4)]
        );
    }

    #[test]
    fn concat_dataframes_empty_list() {
        let result = concat_dataframes(&[]).unwrap();
        assert!(result.is_empty());
        assert_eq!(result.num_columns(), 0);
    }

    #[test]
    fn concat_dataframes_mismatched_columns_rejects() {
        let df1 = DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(1)])]).unwrap();
        let df2 = DataFrame::from_dict(&["b"], vec![("b", vec![Scalar::Int64(2)])]).unwrap();

        let err = concat_dataframes(&[&df1, &df2]).expect_err("should reject");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
    }

    #[test]
    fn concat_dataframes_preserves_index_labels() {
        let df1 =
            DataFrame::from_dict_with_index(vec![("v", vec![Scalar::Int64(10)])], vec!["x".into()])
                .unwrap();
        let df2 =
            DataFrame::from_dict_with_index(vec![("v", vec![Scalar::Int64(20)])], vec!["y".into()])
                .unwrap();

        let result = concat_dataframes(&[&df1, &df2]).unwrap();
        assert_eq!(result.index().labels(), &["x".into(), "y".into()]);
    }

    #[test]
    fn concat_dataframes_three_frames() {
        let mk = |val: i64| {
            DataFrame::from_dict(&["col"], vec![("col", vec![Scalar::Int64(val)])]).unwrap()
        };
        let df1 = mk(1);
        let df2 = mk(2);
        let df3 = mk(3);

        let result = concat_dataframes(&[&df1, &df2, &df3]).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(
            result.column("col").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
    }
}
