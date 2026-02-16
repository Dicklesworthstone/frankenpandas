#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fp_columnar::{ArithmeticOp, Column, ColumnError, ComparisonOp};
use fp_index::{
    AlignMode, Index, IndexError, IndexLabel, align, align_union, validate_alignment_plan,
};
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

    /// Align two Series by their indexes, returning a pair aligned to a common index.
    ///
    /// Matches `pd.Series.align(other, join='inner'|'left'|'right'|'outer')`.
    /// Uses first-match semantics for duplicate labels (same as arithmetic ops).
    pub fn align(&self, other: &Self, mode: AlignMode) -> Result<(Self, Self), FrameError> {
        let plan = align(&self.index, &other.index, mode);
        validate_alignment_plan(&plan)?;

        let left_col = self.column.reindex_by_positions(&plan.left_positions)?;
        let right_col = other.column.reindex_by_positions(&plan.right_positions)?;

        let left_out = Self::new(self.name.clone(), plan.union_index.clone(), left_col)?;
        let right_out = Self::new(other.name.clone(), plan.union_index, right_col)?;

        Ok((left_out, right_out))
    }

    /// Fill missing values from `other`.
    ///
    /// Matches `pd.Series.combine_first(other)`: uses outer alignment,
    /// then for each position takes self's value if non-null, else other's.
    pub fn combine_first(&self, other: &Self) -> Result<Self, FrameError> {
        let plan = align_union(&self.index, &other.index);
        validate_alignment_plan(&plan)?;

        let left_col = self.column.reindex_by_positions(&plan.left_positions)?;
        let right_col = other.column.reindex_by_positions(&plan.right_positions)?;

        let combined_values: Vec<Scalar> = left_col
            .values()
            .iter()
            .zip(right_col.values())
            .map(|(l, r)| if l.is_missing() { r.clone() } else { l.clone() })
            .collect();

        let combined_col = Column::from_values(combined_values)?;
        Self::new(self.name.clone(), plan.union_index, combined_col)
    }

    /// Reindex to a new set of labels, filling missing positions with null.
    ///
    /// Matches `pd.Series.reindex(new_index)`.
    pub fn reindex(&self, new_labels: Vec<IndexLabel>) -> Result<Self, FrameError> {
        let new_index = Index::new(new_labels);
        let current_map = self.index.position_map_first();

        let positions: Vec<Option<usize>> = new_index
            .labels()
            .iter()
            .map(|label| current_map.get(label).copied())
            .collect();

        let col = self.column.reindex_by_positions(&positions)?;
        Self::new(self.name.clone(), new_index, col)
    }

    // --- Comparison Operators ---

    /// Core comparison: align indexes, reindex columns, apply comparison.
    /// Returns a Bool-typed Series.
    fn comparison_op(&self, other: &Self, op: ComparisonOp) -> Result<Self, FrameError> {
        let plan = align_union(&self.index, &other.index);
        validate_alignment_plan(&plan)?;

        let left = self.column.reindex_by_positions(&plan.left_positions)?;
        let right = other.column.reindex_by_positions(&plan.right_positions)?;

        let column = left.binary_comparison(&right, op)?;

        let op_symbol = match op {
            ComparisonOp::Gt => ">",
            ComparisonOp::Lt => "<",
            ComparisonOp::Eq => "==",
            ComparisonOp::Ne => "!=",
            ComparisonOp::Ge => ">=",
            ComparisonOp::Le => "<=",
        };

        let out_name = if self.name == other.name {
            self.name.clone()
        } else {
            format!("{}{op_symbol}{}", self.name, other.name)
        };

        Self::new(out_name, plan.union_index, column)
    }

    /// Element-wise greater-than. Matches `series > other`.
    pub fn gt(&self, other: &Self) -> Result<Self, FrameError> {
        self.comparison_op(other, ComparisonOp::Gt)
    }

    /// Element-wise less-than. Matches `series < other`.
    pub fn lt(&self, other: &Self) -> Result<Self, FrameError> {
        self.comparison_op(other, ComparisonOp::Lt)
    }

    /// Element-wise equality. Matches `series == other`.
    pub fn eq_series(&self, other: &Self) -> Result<Self, FrameError> {
        self.comparison_op(other, ComparisonOp::Eq)
    }

    /// Element-wise not-equal. Matches `series != other`.
    pub fn ne_series(&self, other: &Self) -> Result<Self, FrameError> {
        self.comparison_op(other, ComparisonOp::Ne)
    }

    /// Element-wise greater-or-equal. Matches `series >= other`.
    pub fn ge(&self, other: &Self) -> Result<Self, FrameError> {
        self.comparison_op(other, ComparisonOp::Ge)
    }

    /// Element-wise less-or-equal. Matches `series <= other`.
    pub fn le(&self, other: &Self) -> Result<Self, FrameError> {
        self.comparison_op(other, ComparisonOp::Le)
    }

    /// Compare each element against a scalar value.
    ///
    /// Matches `series > 5` (broadcast scalar comparison).
    pub fn compare_scalar(&self, scalar: &Scalar, op: ComparisonOp) -> Result<Self, FrameError> {
        let column = self.column.compare_scalar(scalar, op)?;
        Self::new(self.name.clone(), self.index.clone(), column)
    }

    // --- Filtering ---

    /// Select elements where `mask` is `True`.
    ///
    /// Matches `series[bool_series]` boolean indexing in pandas.
    /// The mask must be a Bool-typed Series. Indexes are aligned before
    /// applying the mask. Missing values in the mask are treated as `False`.
    pub fn filter(&self, mask: &Self) -> Result<Self, FrameError> {
        let plan = align_union(&self.index, &mask.index);
        validate_alignment_plan(&plan)?;

        let aligned_data = self.column.reindex_by_positions(&plan.left_positions)?;
        let aligned_mask = mask.column.reindex_by_positions(&plan.right_positions)?;

        // Collect indices where mask is True.
        let mut new_labels = Vec::new();
        let mut new_values = Vec::new();

        for (i, mask_val) in aligned_mask.values().iter().enumerate() {
            if matches!(mask_val, Scalar::Bool(true)) {
                new_labels.push(plan.union_index.labels()[i].clone());
                new_values.push(aligned_data.values()[i].clone());
            }
        }

        Self::from_values(self.name.clone(), new_labels, new_values)
    }

    /// Label-based selection for a list of labels.
    ///
    /// Matches `series.loc[[...]]` for list-like selectors. Labels are returned
    /// in selector order and duplicate labels in the selector are preserved.
    /// If a requested label does not exist, this fails closed.
    pub fn loc(&self, labels: &[IndexLabel]) -> Result<Self, FrameError> {
        let mut out_labels = Vec::new();
        let mut out_values = Vec::new();

        for requested in labels {
            let mut found = false;
            for (position, actual) in self.index.labels().iter().enumerate() {
                if actual == requested {
                    out_labels.push(actual.clone());
                    out_values.push(self.column.values()[position].clone());
                    found = true;
                }
            }

            if !found {
                return Err(FrameError::CompatibilityRejected(format!(
                    "loc label not found: {requested:?}"
                )));
            }
        }

        Self::from_values(self.name.clone(), out_labels, out_values)
    }

    /// Position-based selection for a list of integer positions.
    ///
    /// Matches `series.iloc[[...]]` for list-like selectors.
    /// Positions are returned in selector order and duplicates are preserved.
    pub fn iloc(&self, positions: &[usize]) -> Result<Self, FrameError> {
        let mut out_labels = Vec::with_capacity(positions.len());
        let mut out_values = Vec::with_capacity(positions.len());

        for &position in positions {
            if position >= self.len() {
                return Err(FrameError::CompatibilityRejected(format!(
                    "iloc position {position} out of bounds for length {}",
                    self.len()
                )));
            }
            out_labels.push(self.index.labels()[position].clone());
            out_values.push(self.column.values()[position].clone());
        }

        Self::from_values(self.name.clone(), out_labels, out_values)
    }

    // --- Missing Data Operations ---

    /// Fill missing values with a scalar.
    ///
    /// Matches `pd.Series.fillna(value)`.
    pub fn fillna(&self, fill_value: &Scalar) -> Result<Self, FrameError> {
        let column = self.column.fillna(fill_value)?;
        Self::new(self.name.clone(), self.index.clone(), column)
    }

    /// Remove entries with missing values.
    ///
    /// Matches `pd.Series.dropna()`.
    pub fn dropna(&self) -> Result<Self, FrameError> {
        let mut new_labels = Vec::new();
        let mut new_values = Vec::new();

        for (i, val) in self.column.values().iter().enumerate() {
            if !val.is_missing() {
                new_labels.push(self.index.labels()[i].clone());
                new_values.push(val.clone());
            }
        }

        Self::from_values(self.name.clone(), new_labels, new_values)
    }

    /// Return the number of non-null elements.
    ///
    /// Matches `pd.Series.count()`.
    #[must_use]
    pub fn count(&self) -> usize {
        self.column.validity().count_valid()
    }

    // --- Descriptive Statistics ---

    /// Sum of non-null numeric values. Returns `Scalar::Float64(0.0)` for empty.
    ///
    /// Matches `pd.Series.sum()`.
    pub fn sum(&self) -> Result<Scalar, FrameError> {
        let mut total = 0.0_f64;
        for val in self.column.values() {
            if !val.is_missing() {
                total += val.to_f64().map_err(ColumnError::from)?;
            }
        }
        Ok(Scalar::Float64(total))
    }

    /// Mean of non-null numeric values. Returns NaN for empty.
    ///
    /// Matches `pd.Series.mean()`.
    pub fn mean(&self) -> Result<Scalar, FrameError> {
        let count = self.count();
        if count == 0 {
            return Ok(Scalar::Float64(f64::NAN));
        }
        let sum_val = match self.sum()? {
            Scalar::Float64(v) => v,
            _ => return Ok(Scalar::Float64(f64::NAN)),
        };
        Ok(Scalar::Float64(sum_val / count as f64))
    }

    /// Min of non-null numeric values. Returns NaN for empty.
    ///
    /// Matches `pd.Series.min()`.
    pub fn min(&self) -> Result<Scalar, FrameError> {
        let mut result = f64::INFINITY;
        let mut found = false;
        for val in self.column.values() {
            if !val.is_missing() {
                let v = val.to_f64().map_err(ColumnError::from)?;
                if v < result {
                    result = v;
                }
                found = true;
            }
        }
        Ok(if found {
            Scalar::Float64(result)
        } else {
            Scalar::Float64(f64::NAN)
        })
    }

    /// Max of non-null numeric values. Returns NaN for empty.
    ///
    /// Matches `pd.Series.max()`.
    pub fn max(&self) -> Result<Scalar, FrameError> {
        let mut result = f64::NEG_INFINITY;
        let mut found = false;
        for val in self.column.values() {
            if !val.is_missing() {
                let v = val.to_f64().map_err(ColumnError::from)?;
                if v > result {
                    result = v;
                }
                found = true;
            }
        }
        Ok(if found {
            Scalar::Float64(result)
        } else {
            Scalar::Float64(f64::NAN)
        })
    }

    /// Standard deviation of non-null numeric values (ddof=1, sample std).
    ///
    /// Matches `pd.Series.std()`.
    pub fn std(&self) -> Result<Scalar, FrameError> {
        match self.var()? {
            Scalar::Float64(v) => Ok(Scalar::Float64(v.sqrt())),
            other => Ok(other),
        }
    }

    /// Variance of non-null numeric values (ddof=1, sample variance).
    ///
    /// Matches `pd.Series.var()`.
    pub fn var(&self) -> Result<Scalar, FrameError> {
        let count = self.count();
        if count < 2 {
            return Ok(Scalar::Float64(f64::NAN));
        }
        let mean_val = match self.mean()? {
            Scalar::Float64(v) => v,
            _ => return Ok(Scalar::Float64(f64::NAN)),
        };
        let mut sum_sq_diff = 0.0_f64;
        for val in self.column.values() {
            if !val.is_missing() {
                let v = val.to_f64().map_err(ColumnError::from)?;
                let diff = v - mean_val;
                sum_sq_diff += diff * diff;
            }
        }
        Ok(Scalar::Float64(sum_sq_diff / (count as f64 - 1.0)))
    }

    /// Median of non-null numeric values. Returns NaN for empty.
    ///
    /// Matches `pd.Series.median()`.
    pub fn median(&self) -> Result<Scalar, FrameError> {
        let mut vals: Vec<f64> = Vec::new();
        for val in self.column.values() {
            if !val.is_missing() {
                vals.push(val.to_f64().map_err(ColumnError::from)?);
            }
        }
        if vals.is_empty() {
            return Ok(Scalar::Float64(f64::NAN));
        }
        vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = vals.len() / 2;
        let result = if vals.len().is_multiple_of(2) {
            (vals[mid - 1] + vals[mid]) / 2.0
        } else {
            vals[mid]
        };
        Ok(Scalar::Float64(result))
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

    /// Return the column names as a sorted vector.
    ///
    /// Matches `pd.DataFrame.columns` (returns the column labels).
    #[must_use]
    pub fn column_names(&self) -> Vec<&String> {
        self.columns.keys().collect()
    }

    #[must_use]
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.get(name)
    }

    /// Filter rows where `mask` is `True`.
    ///
    /// Matches `df[bool_series]` boolean indexing in pandas.
    /// The mask must be a Bool-typed Series. Indexes are aligned; missing
    /// mask values are treated as `False`.
    pub fn filter_rows(&self, mask: &Series) -> Result<Self, FrameError> {
        let plan = align_union(&self.index, mask.index());
        validate_alignment_plan(&plan)?;

        let aligned_mask = mask.column().reindex_by_positions(&plan.right_positions)?;

        // Determine which rows to keep.
        let keep: Vec<bool> = aligned_mask
            .values()
            .iter()
            .map(|v| matches!(v, Scalar::Bool(true)))
            .collect();

        let mut new_labels = Vec::new();
        for (i, &k) in keep.iter().enumerate() {
            if k {
                new_labels.push(plan.union_index.labels()[i].clone());
            }
        }

        let mut new_columns = BTreeMap::new();
        for (name, col) in &self.columns {
            let aligned = col.reindex_by_positions(&plan.left_positions)?;
            let filtered_values: Vec<Scalar> = aligned
                .values()
                .iter()
                .zip(&keep)
                .filter_map(|(v, &k)| if k { Some(v.clone()) } else { None })
                .collect();
            new_columns.insert(name.clone(), Column::from_values(filtered_values)?);
        }

        Self::new(Index::new(new_labels), new_columns)
    }

    /// Return the first `n` rows.
    ///
    /// Matches `df.head(n)`.
    pub fn head(&self, n: usize) -> Result<Self, FrameError> {
        let take = n.min(self.len());
        let labels = self.index.labels()[..take].to_vec();
        let mut columns = BTreeMap::new();
        for (name, col) in &self.columns {
            let values = col.values()[..take].to_vec();
            columns.insert(name.clone(), Column::from_values(values)?);
        }
        Self::new(Index::new(labels), columns)
    }

    /// Return the last `n` rows.
    ///
    /// Matches `df.tail(n)`.
    pub fn tail(&self, n: usize) -> Result<Self, FrameError> {
        let take = n.min(self.len());
        let start = self.len() - take;
        let labels = self.index.labels()[start..].to_vec();
        let mut columns = BTreeMap::new();
        for (name, col) in &self.columns {
            let values = col.values()[start..].to_vec();
            columns.insert(name.clone(), Column::from_values(values)?);
        }
        Self::new(Index::new(labels), columns)
    }

    /// Label-based row selection for list-like indexers.
    ///
    /// Matches `df.loc[[...]]` for list selectors. Requested labels are
    /// returned in selector order and duplicate labels are preserved.
    /// Missing labels fail closed.
    pub fn loc(&self, labels: &[IndexLabel]) -> Result<Self, FrameError> {
        let mut positions = Vec::new();
        let mut out_labels = Vec::new();

        for requested in labels {
            let mut found = false;
            for (position, actual) in self.index.labels().iter().enumerate() {
                if actual == requested {
                    positions.push(position);
                    out_labels.push(actual.clone());
                    found = true;
                }
            }
            if !found {
                return Err(FrameError::CompatibilityRejected(format!(
                    "loc label not found: {requested:?}"
                )));
            }
        }

        let mut columns = BTreeMap::new();
        for (name, column) in &self.columns {
            let mut values = Vec::with_capacity(positions.len());
            for &position in &positions {
                values.push(column.values()[position].clone());
            }
            columns.insert(name.clone(), Column::from_values(values)?);
        }

        Self::new(Index::new(out_labels), columns)
    }

    /// Position-based row selection for list-like indexers.
    ///
    /// Matches `df.iloc[[...]]` for list selectors. Requested positions are
    /// returned in selector order and duplicates are preserved.
    pub fn iloc(&self, positions: &[usize]) -> Result<Self, FrameError> {
        let mut out_labels = Vec::with_capacity(positions.len());
        for &position in positions {
            if position >= self.len() {
                return Err(FrameError::CompatibilityRejected(format!(
                    "iloc position {position} out of bounds for length {}",
                    self.len()
                )));
            }
            out_labels.push(self.index.labels()[position].clone());
        }

        let mut columns = BTreeMap::new();
        for (name, column) in &self.columns {
            let mut values = Vec::with_capacity(positions.len());
            for &position in positions {
                values.push(column.values()[position].clone());
            }
            columns.insert(name.clone(), Column::from_values(values)?);
        }

        Self::new(Index::new(out_labels), columns)
    }

    /// Add or replace a column.
    ///
    /// Matches `df['new_col'] = values`.
    pub fn with_column(&self, name: impl Into<String>, column: Column) -> Result<Self, FrameError> {
        if column.len() != self.len() {
            return Err(FrameError::LengthMismatch {
                index_len: self.len(),
                column_len: column.len(),
            });
        }
        let mut columns = self.columns.clone();
        columns.insert(name.into(), column);
        Self::new(self.index.clone(), columns)
    }

    /// Remove a column by name, returning the modified DataFrame.
    ///
    /// Matches `df.drop(columns=['col'])`.
    pub fn drop_column(&self, name: &str) -> Result<Self, FrameError> {
        if !self.columns.contains_key(name) {
            return Err(FrameError::CompatibilityRejected(format!(
                "column '{name}' not found"
            )));
        }
        let mut columns = self.columns.clone();
        columns.remove(name);
        Self::new(self.index.clone(), columns)
    }

    /// Rename columns using a mapping.
    ///
    /// Matches `df.rename(columns={'old': 'new'})`.
    pub fn rename_columns(&self, mapping: &[(&str, &str)]) -> Result<Self, FrameError> {
        let rename_map: BTreeMap<&str, &str> = mapping.iter().copied().collect();
        let mut columns = BTreeMap::new();
        for (name, col) in &self.columns {
            let name_str = name.as_str();
            let new_name = rename_map.get(name_str).unwrap_or(&name_str);
            columns.insert((*new_name).to_owned(), col.clone());
        }
        Self::new(self.index.clone(), columns)
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

    // ---- Series.align() tests (bd-2gi.15) ----

    use fp_index::AlignMode;

    #[test]
    fn series_align_inner_keeps_overlapping_labels() {
        let left = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec![2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(400)],
        )
        .unwrap();

        let (la, ra) = left.align(&right, AlignMode::Inner).unwrap();
        assert_eq!(la.len(), 2);
        assert_eq!(la.index().labels(), &[2_i64.into(), 3_i64.into()]);
        assert_eq!(la.values(), &[Scalar::Int64(20), Scalar::Int64(30)]);
        assert_eq!(ra.values(), &[Scalar::Int64(200), Scalar::Int64(300)]);
        assert_eq!(la.name(), "x");
        assert_eq!(ra.name(), "y");
    }

    #[test]
    fn series_align_inner_disjoint_yields_empty() {
        let left = Series::from_values("a", vec![1_i64.into()], vec![Scalar::Int64(1)]).unwrap();
        let right = Series::from_values("b", vec![2_i64.into()], vec![Scalar::Int64(2)]).unwrap();

        let (la, ra) = left.align(&right, AlignMode::Inner).unwrap();
        assert!(la.is_empty());
        assert!(ra.is_empty());
    }

    #[test]
    fn series_align_left_preserves_all_left() {
        let left = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec!["b".into(), "d".into()],
            vec![Scalar::Int64(20), Scalar::Int64(40)],
        )
        .unwrap();

        let (la, ra) = left.align(&right, AlignMode::Left).unwrap();
        assert_eq!(la.len(), 3);
        assert_eq!(la.index().labels(), &["a".into(), "b".into(), "c".into()]);
        assert_eq!(
            la.values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
        // "a" and "c" not in right -> null
        assert!(ra.values()[0].is_missing());
        assert_eq!(ra.values()[1], Scalar::Int64(20));
        assert!(ra.values()[2].is_missing());
    }

    #[test]
    fn series_align_right_preserves_all_right() {
        let left = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec!["b".into(), "c".into(), "d".into()],
            vec![Scalar::Int64(20), Scalar::Int64(30), Scalar::Int64(40)],
        )
        .unwrap();

        let (la, ra) = left.align(&right, AlignMode::Right).unwrap();
        assert_eq!(ra.len(), 3);
        assert_eq!(ra.index().labels(), &["b".into(), "c".into(), "d".into()]);
        assert_eq!(
            ra.values(),
            &[Scalar::Int64(20), Scalar::Int64(30), Scalar::Int64(40)]
        );
        // Only "b" matched in left.
        assert_eq!(la.values()[0], Scalar::Int64(2));
        assert!(la.values()[1].is_missing());
        assert!(la.values()[2].is_missing());
    }

    #[test]
    fn series_align_outer_matches_arithmetic_union() {
        let left = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec![2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(200), Scalar::Int64(300)],
        )
        .unwrap();

        let (la, ra) = left.align(&right, AlignMode::Outer).unwrap();
        assert_eq!(la.len(), 3);
        assert_eq!(
            la.index().labels(),
            &[1_i64.into(), 2_i64.into(), 3_i64.into()]
        );
        assert_eq!(la.values()[1], Scalar::Int64(20));
        assert!(la.values()[2].is_missing());
        assert!(ra.values()[0].is_missing());
        assert_eq!(ra.values()[1], Scalar::Int64(200));
        assert_eq!(ra.values()[2], Scalar::Int64(300));
    }

    #[test]
    fn series_align_identical_indexes() {
        let left = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let right = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(100), Scalar::Int64(200)],
        )
        .unwrap();

        for mode in [
            AlignMode::Inner,
            AlignMode::Left,
            AlignMode::Right,
            AlignMode::Outer,
        ] {
            let (la, ra) = left.align(&right, mode).unwrap();
            assert_eq!(la.len(), 2);
            assert_eq!(la.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
            assert_eq!(ra.values(), &[Scalar::Int64(100), Scalar::Int64(200)]);
        }
    }

    #[test]
    fn series_align_empty_series() {
        let left =
            Series::from_values("a", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();
        let right = Series::from_values("b", vec![1_i64.into()], vec![Scalar::Int64(10)]).unwrap();

        let (la, _) = left.align(&right, AlignMode::Inner).unwrap();
        assert!(la.is_empty());

        let (la, ra) = left.align(&right, AlignMode::Left).unwrap();
        assert!(la.is_empty());
        assert!(ra.is_empty());

        let (la, ra) = left.align(&right, AlignMode::Right).unwrap();
        assert_eq!(ra.len(), 1);
        assert!(la.values()[0].is_missing());
    }

    // ---- Series.combine_first() tests (bd-2gi.15) ----

    #[test]
    fn combine_first_fills_nulls_from_other() {
        let left = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
        )
        .unwrap();

        let result = left.combine_first(&right).unwrap();
        assert_eq!(result.name(), "x");
        assert_eq!(
            result.values(),
            &[Scalar::Int64(10), Scalar::Int64(200), Scalar::Int64(30)]
        );
    }

    #[test]
    fn combine_first_adds_labels_from_other() {
        let left = Series::from_values("x", vec![1_i64.into()], vec![Scalar::Int64(10)]).unwrap();
        let right = Series::from_values(
            "y",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(100), Scalar::Int64(200)],
        )
        .unwrap();

        let result = left.combine_first(&right).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(10)); // self wins
        assert_eq!(result.values()[1], Scalar::Int64(200)); // from other
    }

    #[test]
    fn combine_first_self_has_all_values() {
        let left = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let right = Series::from_values(
            "y",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(100), Scalar::Int64(200)],
        )
        .unwrap();

        let result = left.combine_first(&right).unwrap();
        assert_eq!(result.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
    }

    // ---- Series.reindex() tests (bd-2gi.15) ----

    #[test]
    fn reindex_to_superset_fills_nulls() {
        let s = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let result = s
            .reindex(vec![1_i64.into(), 2_i64.into(), 3_i64.into()])
            .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.values()[0], Scalar::Int64(10));
        assert_eq!(result.values()[1], Scalar::Int64(20));
        assert!(result.values()[2].is_missing());
    }

    #[test]
    fn reindex_to_subset_drops_labels() {
        let s = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        let result = s.reindex(vec![2_i64.into()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.values(), &[Scalar::Int64(20)]);
    }

    #[test]
    fn reindex_reorders_labels() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        let result = s.reindex(vec!["c".into(), "a".into()]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values(), &[Scalar::Int64(3), Scalar::Int64(1)]);
    }

    #[test]
    fn reindex_empty_new_index() {
        let s = Series::from_values("x", vec![1_i64.into()], vec![Scalar::Int64(10)]).unwrap();

        let result = s.reindex(Vec::new()).unwrap();
        assert!(result.is_empty());
    }

    // ---- Series comparison operator tests ----

    #[test]
    fn series_gt_basic() {
        let left = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
        )
        .unwrap();
        let right = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(3), Scalar::Int64(3), Scalar::Int64(3)],
        )
        .unwrap();

        let result = left.gt(&right).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(false));
    }

    #[test]
    fn series_compare_scalar_gt() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
        )
        .unwrap();

        let result = s
            .compare_scalar(&Scalar::Int64(3), fp_columnar::ComparisonOp::Gt)
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(false));
    }

    #[test]
    fn series_comparison_with_alignment() {
        let left = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let right = Series::from_values(
            "b",
            vec![2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(15), Scalar::Int64(25)],
        )
        .unwrap();

        let result = left.gt(&right).unwrap();
        // Union index: [1, 2, 3]
        // Position 0 (label 1): left=10, right=null -> null
        // Position 1 (label 2): left=20, right=15 -> true
        // Position 2 (label 3): left=null, right=25 -> null
        assert!(result.values()[0].is_missing());
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert!(result.values()[2].is_missing());
    }

    #[test]
    fn series_eq_ne_basic() {
        let s1 = Series::from_values(
            "a",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(10)],
        )
        .unwrap();
        let s2 = Series::from_values(
            "b",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(20)],
        )
        .unwrap();

        let eq = s1.eq_series(&s2).unwrap();
        assert_eq!(eq.values()[0], Scalar::Bool(true));
        assert_eq!(eq.values()[1], Scalar::Bool(false));

        let ne = s1.ne_series(&s2).unwrap();
        assert_eq!(ne.values()[0], Scalar::Bool(false));
        assert_eq!(ne.values()[1], Scalar::Bool(true));
    }

    // ---- Series filter tests ----

    #[test]
    fn series_filter_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        let mask = Series::from_values(
            "mask",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
        )
        .unwrap();

        let result = s.filter(&mask).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(10));
        assert_eq!(result.values()[1], Scalar::Int64(30));
    }

    #[test]
    fn series_filter_with_compare() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        let mask = s
            .compare_scalar(&Scalar::Int64(15), fp_columnar::ComparisonOp::Gt)
            .unwrap();
        let result = s.filter(&mask).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(20));
        assert_eq!(result.values()[1], Scalar::Int64(30));
    }

    // ---- Series fillna/dropna tests ----

    #[test]
    fn series_fillna_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        let result = s.fillna(&Scalar::Int64(0)).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(10));
        assert_eq!(result.values()[1], Scalar::Int64(0));
        assert_eq!(result.values()[2], Scalar::Int64(30));
    }

    #[test]
    fn series_dropna_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        let result = s.dropna().unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(10));
        assert_eq!(result.values()[1], Scalar::Int64(30));
        assert_eq!(result.index().labels(), &[1_i64.into(), 3_i64.into()]);
    }

    // ---- Series descriptive statistics tests ----

    #[test]
    fn series_sum_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        assert_eq!(s.sum().unwrap(), Scalar::Float64(60.0));
    }

    #[test]
    fn series_sum_with_nulls() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        assert_eq!(s.sum().unwrap(), Scalar::Float64(40.0));
    }

    #[test]
    fn series_mean_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        assert_eq!(s.mean().unwrap(), Scalar::Float64(20.0));
    }

    #[test]
    fn series_count_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        assert_eq!(s.count(), 2);
    }

    #[test]
    fn series_min_max_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(5), Scalar::Int64(30)],
        )
        .unwrap();

        assert_eq!(s.min().unwrap(), Scalar::Float64(5.0));
        assert_eq!(s.max().unwrap(), Scalar::Float64(30.0));
    }

    #[test]
    fn series_std_var_basic() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(4.0),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        // var = ((2-3.5)^2 + (4-3.5)^2 + (4-3.5)^2 + (4-3.5)^2) / 3 = (2.25+0.25+0.25+0.25)/3 = 1.0
        let var = match s.var().unwrap() {
            Scalar::Float64(v) => v,
            _ => panic!("expected float"),
        };
        assert!((var - 1.0).abs() < 1e-10);

        let std = match s.std().unwrap() {
            Scalar::Float64(v) => v,
            _ => panic!("expected float"),
        };
        assert!((std - 1.0).abs() < 1e-10);
    }

    #[test]
    fn series_median_odd() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();

        assert_eq!(s.median().unwrap(), Scalar::Float64(2.0));
    }

    #[test]
    fn series_median_even() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        assert_eq!(s.median().unwrap(), Scalar::Float64(2.5));
    }

    #[test]
    fn series_stats_empty() {
        let s = Series::from_values("x", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();

        assert_eq!(s.sum().unwrap(), Scalar::Float64(0.0));
        assert_eq!(s.count(), 0);
        match s.mean().unwrap() {
            Scalar::Float64(v) => assert!(v.is_nan()),
            _ => panic!("expected NaN"),
        }
    }

    #[test]
    fn series_loc_selects_labels_in_request_order_with_duplicates() {
        let s = Series::from_values(
            "vals",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        let selected = s.loc(&["c".into(), "a".into(), "c".into()]).unwrap();
        assert_eq!(
            selected.index().labels(),
            &[
                IndexLabel::from("c"),
                IndexLabel::from("a"),
                IndexLabel::from("c")
            ]
        );
        assert_eq!(
            selected.values(),
            &[Scalar::Int64(30), Scalar::Int64(10), Scalar::Int64(30)]
        );
    }

    #[test]
    fn series_loc_missing_label_is_rejected() {
        let s = Series::from_values(
            "vals",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();

        let err = s.loc(&["z".into()]).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("loc label not found"))
        );
    }

    #[test]
    fn series_iloc_selects_positions_in_request_order_with_duplicates() {
        let s = Series::from_values(
            "vals",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
        )
        .unwrap();

        let selected = s.iloc(&[2, 0, 2]).unwrap();
        assert_eq!(
            selected.index().labels(),
            &[
                IndexLabel::from(2_i64),
                IndexLabel::from(0_i64),
                IndexLabel::from(2_i64)
            ]
        );
        assert_eq!(
            selected.values(),
            &[Scalar::Int64(300), Scalar::Int64(100), Scalar::Int64(300)]
        );
    }

    #[test]
    fn series_iloc_out_of_bounds_is_rejected() {
        let s = Series::from_values(
            "vals",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let err = s.iloc(&[2]).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("out of bounds"))
        );
    }

    // ---- DataFrame filter_rows, head, tail, with_column, drop_column, rename tests ----

    #[test]
    fn dataframe_filter_rows_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "b",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
            ],
        )
        .unwrap();

        let mask = Series::from_values(
            "mask",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
        )
        .unwrap();

        let result = df.filter_rows(&mask).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(3)]
        );
        assert_eq!(
            result.column("b").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(30)]
        );
    }

    #[test]
    fn dataframe_head_tail() {
        let df = DataFrame::from_dict(
            &["v"],
            vec![(
                "v",
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                    Scalar::Int64(4),
                    Scalar::Int64(5),
                ],
            )],
        )
        .unwrap();

        let head = df.head(3).unwrap();
        assert_eq!(head.len(), 3);
        assert_eq!(
            head.column("v").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );

        let tail = df.tail(2).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(
            tail.column("v").unwrap().values(),
            &[Scalar::Int64(4), Scalar::Int64(5)]
        );
    }

    #[test]
    fn dataframe_loc_selects_labels_in_request_order_with_duplicates() {
        let df = DataFrame::from_dict_with_index(
            vec![
                (
                    "a",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
                (
                    "b",
                    vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
                ),
            ],
            vec!["x".into(), "y".into(), "z".into()],
        )
        .unwrap();

        let selected = df.loc(&["z".into(), "x".into(), "z".into()]).unwrap();
        assert_eq!(
            selected.index().labels(),
            &[
                IndexLabel::from("z"),
                IndexLabel::from("x"),
                IndexLabel::from("z")
            ]
        );
        assert_eq!(
            selected.column("a").unwrap().values(),
            &[Scalar::Int64(30), Scalar::Int64(10), Scalar::Int64(30)]
        );
        assert_eq!(
            selected.column("b").unwrap().values(),
            &[Scalar::Int64(300), Scalar::Int64(100), Scalar::Int64(300)]
        );
    }

    #[test]
    fn dataframe_loc_missing_label_is_rejected() {
        let df = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec!["x".into(), "y".into()],
        )
        .unwrap();

        let err = df.loc(&["missing".into()]).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("loc label not found"))
        );
    }

    #[test]
    fn dataframe_iloc_selects_positions_in_request_order_with_duplicates() {
        let df = DataFrame::from_dict_with_index(
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1000),
                        Scalar::Int64(1100),
                        Scalar::Int64(1200),
                        Scalar::Int64(1300),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(2000),
                        Scalar::Int64(2100),
                        Scalar::Int64(2200),
                        Scalar::Int64(2300),
                    ],
                ),
            ],
            vec![
                IndexLabel::from(10_i64),
                IndexLabel::from(11_i64),
                IndexLabel::from(12_i64),
                IndexLabel::from(13_i64),
            ],
        )
        .unwrap();

        let selected = df.iloc(&[3, 1, 3]).unwrap();
        assert_eq!(
            selected.index().labels(),
            &[
                IndexLabel::from(13_i64),
                IndexLabel::from(11_i64),
                IndexLabel::from(13_i64)
            ]
        );
        assert_eq!(
            selected.column("a").unwrap().values(),
            &[
                Scalar::Int64(1300),
                Scalar::Int64(1100),
                Scalar::Int64(1300)
            ]
        );
        assert_eq!(
            selected.column("b").unwrap().values(),
            &[
                Scalar::Int64(2300),
                Scalar::Int64(2100),
                Scalar::Int64(2300)
            ]
        );
    }

    #[test]
    fn dataframe_iloc_out_of_bounds_is_rejected() {
        let df = DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(10)])]).unwrap();
        let err = df.iloc(&[1]).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("out of bounds"))
        );
    }

    #[test]
    fn dataframe_with_column() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        use fp_columnar::Column;
        let new_col =
            Column::from_values(vec![Scalar::Float64(10.0), Scalar::Float64(20.0)]).unwrap();

        let result = df.with_column("b", new_col).unwrap();
        assert_eq!(result.num_columns(), 2);
        assert_eq!(
            result.column("b").unwrap().values(),
            &[Scalar::Float64(10.0), Scalar::Float64(20.0)]
        );
    }

    #[test]
    fn dataframe_drop_column() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![("a", vec![Scalar::Int64(1)]), ("b", vec![Scalar::Int64(2)])],
        )
        .unwrap();

        let result = df.drop_column("b").unwrap();
        assert_eq!(result.num_columns(), 1);
        assert!(result.column("a").is_some());
        assert!(result.column("b").is_none());
    }

    #[test]
    fn dataframe_rename_columns() {
        let df = DataFrame::from_dict(&["old_name"], vec![("old_name", vec![Scalar::Int64(1)])])
            .unwrap();

        let result = df.rename_columns(&[("old_name", "new_name")]).unwrap();
        assert!(result.column("new_name").is_some());
        assert!(result.column("old_name").is_none());
    }
}
