#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use fp_columnar::{ArithmeticOp, Column, ColumnError, ComparisonOp};
use fp_index::{
    AlignMode, Index, IndexError, IndexLabel, align, align_union, validate_alignment_plan,
};
use fp_runtime::{DecisionAction, EvidenceLedger, RuntimeMode, RuntimePolicy};
use fp_types::{DType, NullKind, Scalar};
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

fn normalize_iloc_position(position: i64, len: usize) -> Result<usize, FrameError> {
    let len_i128 = i128::try_from(len).map_err(|_| {
        FrameError::CompatibilityRejected(format!(
            "iloc cannot address length {len} on this platform"
        ))
    })?;
    let position_i128 = i128::from(position);
    let normalized = if position_i128 < 0 {
        len_i128 + position_i128
    } else {
        position_i128
    };

    if normalized < 0 || normalized >= len_i128 {
        return Err(FrameError::CompatibilityRejected(format!(
            "iloc position {position} out of bounds for length {len}"
        )));
    }

    usize::try_from(normalized).map_err(|_| {
        FrameError::CompatibilityRejected(format!(
            "iloc position {position} out of bounds for length {len}"
        ))
    })
}

fn saturating_i64_to_usize(value: i64) -> usize {
    if value < 0 {
        0
    } else {
        usize::try_from(value).unwrap_or(usize::MAX)
    }
}

fn saturating_i64_abs_to_usize(value: i64) -> usize {
    usize::try_from(value.unsigned_abs()).unwrap_or(usize::MAX)
}

fn normalize_head_take(n: i64, len: usize) -> usize {
    if n >= 0 {
        saturating_i64_to_usize(n).min(len)
    } else {
        len.saturating_sub(saturating_i64_abs_to_usize(n))
    }
}

fn normalize_tail_window(n: i64, len: usize) -> (usize, usize) {
    if n >= 0 {
        let take = saturating_i64_to_usize(n).min(len);
        (len - take, take)
    } else {
        let skip = saturating_i64_abs_to_usize(n).min(len);
        (skip, len - skip)
    }
}

fn scalar_to_index_label(value: &Scalar) -> Result<IndexLabel, FrameError> {
    match value {
        Scalar::Int64(v) => Ok(IndexLabel::Int64(*v)),
        Scalar::Utf8(v) => Ok(IndexLabel::Utf8(v.clone())),
        Scalar::Null(_) => Err(FrameError::CompatibilityRejected(
            "set_index does not support missing label values".to_owned(),
        )),
        _ => Err(FrameError::CompatibilityRejected(format!(
            "set_index currently supports Int64/Utf8 labels; found {:?}",
            value.dtype()
        ))),
    }
}

fn index_label_to_scalar(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(v) => Scalar::Int64(*v),
        IndexLabel::Utf8(v) => Scalar::Utf8(v.clone()),
    }
}

fn range_index(len: usize) -> Result<Index, FrameError> {
    let len_i64 = i64::try_from(len).map_err(|_| {
        FrameError::CompatibilityRejected(format!(
            "cannot materialize RangeIndex for length {len} on this platform"
        ))
    })?;
    Ok(Index::new((0..len_i64).map(IndexLabel::from).collect()))
}

fn compare_non_missing_scalars_for_sort(left: &Scalar, right: &Scalar) -> Ordering {
    match (left, right) {
        (Scalar::Bool(lhs), Scalar::Bool(rhs)) => lhs.cmp(rhs),
        (Scalar::Int64(lhs), Scalar::Int64(rhs)) => lhs.cmp(rhs),
        (Scalar::Float64(lhs), Scalar::Float64(rhs)) => {
            lhs.partial_cmp(rhs).unwrap_or(Ordering::Equal)
        }
        (Scalar::Utf8(lhs), Scalar::Utf8(rhs)) => lhs.cmp(rhs),
        // Columns are dtype-homogeneous; this fallback is only for defensive
        // ordering when malformed mixed values leak in.
        _ => left.dtype().cmp(&right.dtype()),
    }
}

fn compare_scalars_with_na_last(left: &Scalar, right: &Scalar, ascending: bool) -> Ordering {
    match (left.is_missing(), right.is_missing()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let order = compare_non_missing_scalars_for_sort(left, right);
            if ascending { order } else { order.reverse() }
        }
    }
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

    // --- Logical Boolean Operators ---

    fn ensure_boolean_series(&self, op_name: &str) -> Result<(), FrameError> {
        if let Some(offending) = self
            .values()
            .iter()
            .find(|value| !matches!(value, Scalar::Bool(_) | Scalar::Null(_)))
        {
            return Err(FrameError::CompatibilityRejected(format!(
                "boolean series required for {op_name}; found dtype {:?}",
                offending.dtype()
            )));
        }
        Ok(())
    }

    /// Element-wise boolean AND with outer index alignment.
    ///
    /// Uses three-valued logic compatible with nullable boolean semantics:
    /// - `false & x == false`
    /// - `true & true == true`
    /// - `true & null == null`, `null & null == null`
    pub fn and(&self, other: &Self) -> Result<Self, FrameError> {
        self.ensure_boolean_series("and")?;
        other.ensure_boolean_series("and")?;

        let plan = align_union(&self.index, &other.index);
        validate_alignment_plan(&plan)?;

        let left = self.column.reindex_by_positions(&plan.left_positions)?;
        let right = other.column.reindex_by_positions(&plan.right_positions)?;

        let values: Vec<Scalar> = left
            .values()
            .iter()
            .zip(right.values())
            .map(|(lhs, rhs)| match (lhs, rhs) {
                (Scalar::Bool(false), _) | (_, Scalar::Bool(false)) => Scalar::Bool(false),
                (Scalar::Bool(true), Scalar::Bool(true)) => Scalar::Bool(true),
                (Scalar::Bool(true), Scalar::Null(_))
                | (Scalar::Null(_), Scalar::Bool(true))
                | (Scalar::Null(_), Scalar::Null(_)) => Scalar::Null(NullKind::Null),
                _ => Scalar::Null(NullKind::Null),
            })
            .collect();

        let out_name = if self.name == other.name {
            self.name.clone()
        } else {
            format!("{}&{}", self.name, other.name)
        };

        Self::new(out_name, plan.union_index, Column::from_values(values)?)
    }

    /// Element-wise boolean OR with outer index alignment.
    ///
    /// Uses three-valued logic compatible with nullable boolean semantics:
    /// - `true | x == true`
    /// - `false | false == false`
    /// - `false | null == null`, `null | null == null`
    pub fn or(&self, other: &Self) -> Result<Self, FrameError> {
        self.ensure_boolean_series("or")?;
        other.ensure_boolean_series("or")?;

        let plan = align_union(&self.index, &other.index);
        validate_alignment_plan(&plan)?;

        let left = self.column.reindex_by_positions(&plan.left_positions)?;
        let right = other.column.reindex_by_positions(&plan.right_positions)?;

        let values: Vec<Scalar> = left
            .values()
            .iter()
            .zip(right.values())
            .map(|(lhs, rhs)| match (lhs, rhs) {
                (Scalar::Bool(true), _) | (_, Scalar::Bool(true)) => Scalar::Bool(true),
                (Scalar::Bool(false), Scalar::Bool(false)) => Scalar::Bool(false),
                (Scalar::Bool(false), Scalar::Null(_))
                | (Scalar::Null(_), Scalar::Bool(false))
                | (Scalar::Null(_), Scalar::Null(_)) => Scalar::Null(NullKind::Null),
                _ => Scalar::Null(NullKind::Null),
            })
            .collect();

        let out_name = if self.name == other.name {
            self.name.clone()
        } else {
            format!("{}|{}", self.name, other.name)
        };

        Self::new(out_name, plan.union_index, Column::from_values(values)?)
    }

    /// Element-wise boolean NOT.
    ///
    /// Missing values remain missing.
    pub fn not(&self) -> Result<Self, FrameError> {
        self.ensure_boolean_series("not")?;

        let values: Vec<Scalar> = self
            .values()
            .iter()
            .map(|value| match value {
                Scalar::Bool(v) => Scalar::Bool(!v),
                Scalar::Null(_) => Scalar::Null(NullKind::Null),
                _ => Scalar::Null(NullKind::Null),
            })
            .collect();

        Self::new(
            format!("~{}", self.name),
            self.index.clone(),
            Column::from_values(values)?,
        )
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
        if let Some(offending) = aligned_mask
            .values()
            .iter()
            .find(|value| !matches!(value, Scalar::Bool(_) | Scalar::Null(_)))
        {
            return Err(FrameError::CompatibilityRejected(format!(
                "boolean mask required for filter; found dtype {:?}",
                offending.dtype()
            )));
        }

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
    /// Negative positions are resolved from the end of the Series.
    pub fn iloc(&self, positions: &[i64]) -> Result<Self, FrameError> {
        let mut out_labels = Vec::with_capacity(positions.len());
        let mut out_values = Vec::with_capacity(positions.len());

        for &position in positions {
            let normalized = normalize_iloc_position(position, self.len())?;
            out_labels.push(self.index.labels()[normalized].clone());
            out_values.push(self.column.values()[normalized].clone());
        }

        Self::from_values(self.name.clone(), out_labels, out_values)
    }

    /// Return the first `n` rows.
    ///
    /// Matches `s.head(n)`. If `n` is negative, this returns all rows except
    /// the last `-n` rows.
    pub fn head(&self, n: i64) -> Result<Self, FrameError> {
        let take = normalize_head_take(n, self.len());
        let labels = self.index.labels()[..take].to_vec();
        let values = self.values()[..take].to_vec();
        Self::from_values(self.name.clone(), labels, values)
    }

    /// Return the last `n` rows.
    ///
    /// Matches `s.tail(n)`. If `n` is negative, this returns all rows except
    /// the first `-n` rows.
    pub fn tail(&self, n: i64) -> Result<Self, FrameError> {
        let (start, _) = normalize_tail_window(n, self.len());
        let labels = self.index.labels()[start..].to_vec();
        let values = self.values()[start..].to_vec();
        Self::from_values(self.name.clone(), labels, values)
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

    /// Return a boolean mask where missing values are `true`.
    ///
    /// Matches `pd.Series.isna()`.
    pub fn isna(&self) -> Result<Self, FrameError> {
        let labels = self.index.labels().to_vec();
        let values = self
            .column
            .values()
            .iter()
            .map(|value| Scalar::Bool(value.is_missing()))
            .collect::<Vec<_>>();
        Self::from_values(self.name.clone(), labels, values)
    }

    /// Alias for `isna`.
    ///
    /// Matches `pd.Series.isnull()`.
    pub fn isnull(&self) -> Result<Self, FrameError> {
        self.isna()
    }

    /// Return a boolean mask where non-missing values are `true`.
    ///
    /// Matches `pd.Series.notna()`.
    pub fn notna(&self) -> Result<Self, FrameError> {
        let labels = self.index.labels().to_vec();
        let values = self
            .column
            .values()
            .iter()
            .map(|value| Scalar::Bool(!value.is_missing()))
            .collect::<Vec<_>>();
        Self::from_values(self.name.clone(), labels, values)
    }

    /// Alias for `notna`.
    ///
    /// Matches `pd.Series.notnull()`.
    pub fn notnull(&self) -> Result<Self, FrameError> {
        self.notna()
    }

    /// Return the number of non-null elements.
    ///
    /// Matches `pd.Series.count()`.
    #[must_use]
    pub fn count(&self) -> usize {
        self.column.validity().count_valid()
    }

    /// Cast values to a target dtype.
    ///
    /// Matches `series.astype(dtype)` for scalar dtypes.
    pub fn astype(&self, dtype: DType) -> Result<Self, FrameError> {
        let column = Column::new(dtype, self.values().to_vec())?;
        Self::new(self.name.clone(), self.index.clone(), column)
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
/// - Output columns are the union of input columns (`join='outer'`).
/// - Index labels are concatenated in order (duplicates preserved).
/// - Missing cells materialize as `Scalar::Null`.
/// - Empty input returns an empty DataFrame.
pub fn concat_dataframes(frames: &[&DataFrame]) -> Result<DataFrame, FrameError> {
    if frames.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new());
    }

    // Concatenate index labels.
    let total_len: usize = frames.iter().map(|f| f.len()).sum();
    let mut labels = Vec::with_capacity(total_len);
    for frame in frames {
        labels.extend_from_slice(frame.index().labels());
    }
    let index = Index::new(labels);

    // Materialize union-column output with null fill for missing frame columns.
    // Preserve first-seen column order (`sort=False` pandas semantics).
    let mut union_columns = Vec::new();
    let mut seen = BTreeSet::new();
    for frame in frames {
        for name in frame.column_names() {
            if seen.insert(name.clone()) {
                union_columns.push(name.clone());
            }
        }
    }

    let mut columns = BTreeMap::new();
    for col_name in &union_columns {
        let mut values = Vec::with_capacity(total_len);
        for frame in frames {
            if let Some(column) = frame.column(col_name) {
                values.extend_from_slice(column.values());
            } else {
                for _ in 0..frame.len() {
                    values.push(Scalar::Null(NullKind::Null));
                }
            }
        }
        columns.insert(col_name.clone(), Column::from_values(values)?);
    }

    DataFrame::new_with_column_order(index, columns, union_columns)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcatJoin {
    Outer,
    Inner,
}

/// Concatenate DataFrames along the requested axis.
///
/// Supported axes:
/// - `0`: row-wise concatenation (`concat_dataframes`)
/// - `1`: column-wise concatenation with outer index alignment
pub fn concat_dataframes_with_axis(
    frames: &[&DataFrame],
    axis: i64,
) -> Result<DataFrame, FrameError> {
    concat_dataframes_with_axis_join(frames, axis, ConcatJoin::Outer)
}

/// Concatenate DataFrames along axis with explicit join policy.
///
/// Supported combinations:
/// - `axis=0, join=outer|inner`: row-wise concatenation
/// - `axis=1, join=outer|inner`: column-wise alignment
pub fn concat_dataframes_with_axis_join(
    frames: &[&DataFrame],
    axis: i64,
    join: ConcatJoin,
) -> Result<DataFrame, FrameError> {
    match axis {
        0 => match join {
            ConcatJoin::Outer => concat_dataframes(frames),
            ConcatJoin::Inner => concat_dataframes_axis0_inner(frames),
        },
        1 => concat_dataframes_axis1(frames, join),
        _ => Err(FrameError::CompatibilityRejected(format!(
            "unsupported concat axis {axis}; expected 0 or 1"
        ))),
    }
}

fn concat_dataframes_axis0_inner(frames: &[&DataFrame]) -> Result<DataFrame, FrameError> {
    if frames.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new());
    }

    let mut shared_columns: Vec<String> = frames[0].column_names().into_iter().cloned().collect();
    for frame in frames.iter().skip(1) {
        let frame_columns: BTreeSet<&str> = frame
            .column_names()
            .into_iter()
            .map(String::as_str)
            .collect();
        shared_columns.retain(|name| frame_columns.contains(name.as_str()));
    }

    let total_len: usize = frames.iter().map(|frame| frame.len()).sum();
    let mut labels = Vec::with_capacity(total_len);
    for frame in frames {
        labels.extend_from_slice(frame.index().labels());
    }
    let index = Index::new(labels);

    let mut columns = BTreeMap::new();
    for name in &shared_columns {
        let mut values = Vec::with_capacity(total_len);
        for frame in frames {
            values.extend_from_slice(
                frame
                    .column(name)
                    .expect("shared concat(axis=0, join='inner') column must exist")
                    .values(),
            );
        }
        columns.insert(name.clone(), Column::from_values(values)?);
    }

    DataFrame::new_with_column_order(index, columns, shared_columns)
}

/// Column-wise DataFrame concat with deterministic outer index alignment.
///
/// Matches `pd.concat([df1, df2, ...], axis=1, join="outer", sort=False)` for
/// the currently supported subset:
/// - Union index preserves left-then-unseen label order.
/// - Missing rows in each input column materialize as `Scalar::Null`.
/// - Duplicate output column names are rejected (fail-closed for MVP storage model).
/// - Duplicate input index labels are rejected in this MVP slice.
fn concat_dataframes_axis1(
    frames: &[&DataFrame],
    join: ConcatJoin,
) -> Result<DataFrame, FrameError> {
    if frames.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new());
    }

    if frames.iter().any(|frame| frame.index().has_duplicates()) {
        return Err(FrameError::CompatibilityRejected(
            "concat(axis=1) does not yet support duplicate index labels".to_owned(),
        ));
    }

    let target_index = match join {
        ConcatJoin::Outer => {
            let mut union_index = frames[0].index().clone();
            for frame in frames.iter().skip(1) {
                let plan = align_union(&union_index, frame.index());
                validate_alignment_plan(&plan)?;
                union_index = plan.union_index;
            }
            union_index
        }
        ConcatJoin::Inner => {
            let mut labels = frames[0].index().labels().to_vec();
            for frame in frames.iter().skip(1) {
                let positions = frame.index().position_map_first();
                labels.retain(|label| positions.contains_key(label));
            }
            Index::new(labels)
        }
    };

    let mut columns = BTreeMap::new();
    let mut output_column_order = Vec::new();
    for frame in frames {
        let positions = frame.index().get_indexer(&target_index);

        for name in frame.column_names() {
            let column = frame
                .column(name)
                .expect("frame column listed in order must exist");
            if columns.contains_key(name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "duplicate column '{name}' in concat(axis=1) output"
                )));
            }

            let values = positions
                .iter()
                .map(|position| match position {
                    Some(pos) => column.values()[*pos].clone(),
                    None => Scalar::Null(NullKind::Null),
                })
                .collect::<Vec<_>>();

            columns.insert(name.clone(), Column::from_values(values)?);
            output_column_order.push(name.clone());
        }
    }

    DataFrame::new_with_column_order(target_index, columns, output_column_order)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataFrame {
    index: Index,
    columns: BTreeMap<String, Column>,
    #[serde(skip)]
    column_order: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropNaHow {
    Any,
    All,
}

impl DataFrame {
    fn validate_column_lengths(
        index: &Index,
        columns: &BTreeMap<String, Column>,
    ) -> Result<(), FrameError> {
        for column in columns.values() {
            if column.len() != index.len() {
                return Err(FrameError::LengthMismatch {
                    index_len: index.len(),
                    column_len: column.len(),
                });
            }
        }
        Ok(())
    }

    fn normalize_column_order(
        columns: &BTreeMap<String, Column>,
        column_order: Vec<String>,
    ) -> Result<Vec<String>, FrameError> {
        if column_order.is_empty() {
            return Ok(columns.keys().cloned().collect());
        }

        let mut normalized = Vec::with_capacity(columns.len());
        for name in column_order {
            if !columns.contains_key(&name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "column '{name}' not found in data"
                )));
            }
            // Match pandas-style selector normalization: if a selector is repeated,
            // keep the last occurrence.
            if let Some(existing_idx) = normalized.iter().position(|entry| entry == &name) {
                normalized.remove(existing_idx);
            }
            normalized.push(name);
        }

        for name in columns.keys() {
            if !normalized.iter().any(|entry| entry == name) {
                normalized.push(name.clone());
            }
        }

        Ok(normalized)
    }

    fn resolve_column_selector(
        &self,
        column_selector: Option<&[String]>,
    ) -> Result<Vec<String>, FrameError> {
        let Some(requested_columns) = column_selector else {
            return Ok(self.column_order.clone());
        };

        let mut selected = Vec::with_capacity(requested_columns.len());
        let mut seen = BTreeSet::new();
        for requested in requested_columns {
            if !self.columns.contains_key(requested) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "column '{requested}' not found"
                )));
            }
            if !seen.insert(requested.clone()) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "duplicate column selector: '{requested}'"
                )));
            }
            selected.push(requested.clone());
        }

        Ok(selected)
    }

    fn resolve_row_label_selector(
        &self,
        row_selector: Option<&[IndexLabel]>,
    ) -> Result<Vec<usize>, FrameError> {
        let Some(requested_rows) = row_selector else {
            return Ok((0..self.len()).collect());
        };

        let mut selected_positions = Vec::new();
        for requested in requested_rows {
            let mut found = false;
            for (position, actual) in self.index.labels().iter().enumerate() {
                if actual == requested {
                    selected_positions.push(position);
                    found = true;
                }
            }
            if !found {
                return Err(FrameError::CompatibilityRejected(format!(
                    "index label '{requested}' not found"
                )));
            }
        }

        Ok(selected_positions)
    }

    fn reorder_rows_by_positions(&self, positions: &[usize]) -> Result<Self, FrameError> {
        if positions.len() != self.len() {
            return Err(FrameError::CompatibilityRejected(format!(
                "row position list length {} does not match dataframe length {}",
                positions.len(),
                self.len()
            )));
        }

        for &position in positions {
            if position >= self.len() {
                return Err(FrameError::CompatibilityRejected(format!(
                    "row position {position} out of bounds for length {}",
                    self.len()
                )));
            }
        }

        let index = self.index.take(positions);
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let column = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let values = positions
                .iter()
                .map(|&position| column.values()[position].clone())
                .collect::<Vec<_>>();
            columns.insert(name.clone(), Column::new(column.dtype(), values)?);
        }

        Self::new_with_column_order(index, columns, self.column_order.clone())
    }

    fn reset_index_column_name(&self) -> Result<String, FrameError> {
        if !self.columns.contains_key("index") {
            return Ok("index".to_owned());
        }
        if !self.columns.contains_key("level_0") {
            return Ok("level_0".to_owned());
        }
        Err(FrameError::CompatibilityRejected(
            "reset_index cannot insert index column because both 'index' and 'level_0' already exist"
                .to_owned(),
        ))
    }

    pub fn new(index: Index, columns: BTreeMap<String, Column>) -> Result<Self, FrameError> {
        Self::validate_column_lengths(&index, &columns)?;
        let column_order = columns.keys().cloned().collect();
        Ok(Self {
            index,
            columns,
            column_order,
        })
    }

    pub fn new_with_column_order(
        index: Index,
        columns: BTreeMap<String, Column>,
        column_order: Vec<String>,
    ) -> Result<Self, FrameError> {
        Self::validate_column_lengths(&index, &columns)?;
        let column_order = Self::normalize_column_order(&columns, column_order)?;
        Ok(Self {
            index,
            columns,
            column_order,
        })
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
        let mut column_order = Vec::new();
        for series in series_list {
            let plan = align_union(&union_index, &series.index);
            // The right_positions map from the union to this series's positions.
            // Since union_index already contains all labels, the union_index in
            // this plan equals our pre-computed union. We use right_positions to
            // locate each series's values within the union.
            let aligned_column = series.column.reindex_by_positions(&plan.right_positions)?;
            column_order.push(series.name.clone());
            columns.insert(series.name, aligned_column);
        }

        Self::new_with_column_order(union_index, columns, column_order)
    }

    /// Construct a DataFrame from a dict of column vectors.
    ///
    /// Matches `pd.DataFrame({"a": [1, 2], "b": [3, 4]})`.
    /// All vectors must have the same length. Index is auto-generated
    /// as 0..n (RangeIndex-style).
    ///
    /// `column_order` controls observable column label order.
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
        let mut input_order = Vec::new();
        for (name, values) in data {
            if values.len() != n {
                return Err(FrameError::LengthMismatch {
                    index_len: n,
                    column_len: values.len(),
                });
            }
            input_order.push(name.to_owned());
            columns.insert(name.to_owned(), Column::from_values(values)?);
        }

        let output_order = if column_order.is_empty() {
            input_order
        } else {
            let mut explicit = Vec::with_capacity(columns.len());
            let mut seen = BTreeSet::new();
            for &name in column_order {
                if !columns.contains_key(name) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "column '{name}' not found in data"
                    )));
                }
                if !seen.insert(name.to_owned()) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "duplicate column selector: '{name}'"
                    )));
                }
                explicit.push(name.to_owned());
            }
            for name in input_order {
                if seen.insert(name.clone()) {
                    explicit.push(name);
                }
            }
            explicit
        };

        Self::new_with_column_order(index, columns, output_order)
    }

    /// Construct a DataFrame from row records.
    ///
    /// Matches `pd.DataFrame.from_records(records, columns=..., index=...)`.
    /// Missing keys are null-filled. If `column_order` is provided, it is used
    /// as the exact output column selector/order and may include labels absent
    /// from the records (materialized as all-null columns).
    pub fn from_records(
        records: Vec<BTreeMap<String, Scalar>>,
        column_order: Option<&[String]>,
        index_labels: Option<Vec<IndexLabel>>,
    ) -> Result<Self, FrameError> {
        let row_count = records.len();

        if let Some(labels) = index_labels.as_ref()
            && labels.len() != row_count
        {
            return Err(FrameError::CompatibilityRejected(format!(
                "dataframe_from_records index length {} does not match records length {}",
                labels.len(),
                row_count
            )));
        }

        let mut discovered_columns = Vec::new();
        let mut discovered_seen = BTreeSet::new();
        for record in &records {
            for key in record.keys() {
                if discovered_seen.insert(key.clone()) {
                    discovered_columns.push(key.clone());
                }
            }
        }

        let output_order = if let Some(requested_order) = column_order {
            let mut explicit = Vec::with_capacity(requested_order.len());
            let mut seen = BTreeSet::new();
            for requested in requested_order {
                if !seen.insert(requested.clone()) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "duplicate column selector: '{requested}'"
                    )));
                }
                explicit.push(requested.clone());
            }
            explicit
        } else {
            discovered_columns
        };

        let mut columns = BTreeMap::new();
        for name in &output_order {
            let values = records
                .iter()
                .map(|record| {
                    record
                        .get(name)
                        .cloned()
                        .unwrap_or(Scalar::Null(NullKind::Null))
                })
                .collect::<Vec<_>>();
            columns.insert(name.clone(), Column::from_values(values)?);
        }

        let index = match index_labels {
            Some(labels) => Index::new(labels),
            None => Index::new((0..row_count as i64).map(IndexLabel::from).collect()),
        };

        Self::new_with_column_order(index, columns, output_order)
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
        let mut input_order = Vec::new();
        for (name, values) in data {
            if values.len() != n {
                return Err(FrameError::LengthMismatch {
                    index_len: n,
                    column_len: values.len(),
                });
            }
            input_order.push(name.to_owned());
            columns.insert(name.to_owned(), Column::from_values(values)?);
        }

        Self::new_with_column_order(index, columns, input_order)
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
        let column_order = names.iter().map(|name| (*name).to_owned()).collect();
        Self::new_with_column_order(self.index.clone(), columns, column_order)
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

    /// Return the column names in observable DataFrame order.
    ///
    /// Matches `pd.DataFrame.columns` (returns the column labels).
    #[must_use]
    pub fn column_names(&self) -> Vec<&String> {
        self.column_order.iter().collect()
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
        if let Some(offending) = aligned_mask
            .values()
            .iter()
            .find(|value| !matches!(value, Scalar::Bool(_) | Scalar::Null(_)))
        {
            return Err(FrameError::CompatibilityRejected(format!(
                "boolean mask required for filter_rows; found dtype {:?}",
                offending.dtype()
            )));
        }

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
        for name in &self.column_order {
            let col = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let aligned = col.reindex_by_positions(&plan.left_positions)?;
            let filtered_values: Vec<Scalar> = aligned
                .values()
                .iter()
                .zip(&keep)
                .filter_map(|(v, &k)| if k { Some(v.clone()) } else { None })
                .collect();
            new_columns.insert(name.clone(), Column::from_values(filtered_values)?);
        }

        Self::new_with_column_order(
            Index::new(new_labels),
            new_columns,
            self.column_order.clone(),
        )
    }

    /// Return a DataFrame of booleans indicating missing values.
    ///
    /// Matches `df.isna()`.
    pub fn isna(&self) -> Result<Self, FrameError> {
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let column = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let mask_values = column
                .values()
                .iter()
                .map(|value| Scalar::Bool(value.is_missing()))
                .collect::<Vec<_>>();
            columns.insert(name.clone(), Column::from_values(mask_values)?);
        }

        Self::new_with_column_order(self.index.clone(), columns, self.column_order.clone())
    }

    /// Return a DataFrame of booleans indicating non-missing values.
    ///
    /// Matches `df.notna()`.
    pub fn notna(&self) -> Result<Self, FrameError> {
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let column = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let mask_values = column
                .values()
                .iter()
                .map(|value| Scalar::Bool(!value.is_missing()))
                .collect::<Vec<_>>();
            columns.insert(name.clone(), Column::from_values(mask_values)?);
        }

        Self::new_with_column_order(self.index.clone(), columns, self.column_order.clone())
    }

    /// Alias for `isna`.
    ///
    /// Matches `df.isnull()`.
    pub fn isnull(&self) -> Result<Self, FrameError> {
        self.isna()
    }

    /// Alias for `notna`.
    ///
    /// Matches `df.notnull()`.
    pub fn notnull(&self) -> Result<Self, FrameError> {
        self.notna()
    }

    /// Fill missing values in each column with `fill_value`.
    ///
    /// Matches `df.fillna(value)` for scalar `value`.
    pub fn fillna(&self, fill_value: &Scalar) -> Result<Self, FrameError> {
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let column = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            columns.insert(name.clone(), column.fillna(fill_value)?);
        }

        Self::new_with_column_order(self.index.clone(), columns, self.column_order.clone())
    }

    /// Drop rows containing missing values in any selected column.
    ///
    /// Matches default `df.dropna()` row-wise behavior (`axis=0`, `how='any'`).
    pub fn dropna(&self) -> Result<Self, FrameError> {
        self.dropna_with_options(DropNaHow::Any, None)
    }

    /// Drop rows by configurable missing-value policy.
    ///
    /// Matches `df.dropna(how=..., subset=...)` for row-wise mode (`axis=0`).
    pub fn dropna_with_options(
        &self,
        how: DropNaHow,
        subset: Option<&[String]>,
    ) -> Result<Self, FrameError> {
        if self.is_empty() {
            return Ok(self.clone());
        }

        let selected_columns = self.resolve_column_selector(subset)?;
        if selected_columns.is_empty() {
            return Ok(self.clone());
        }

        let mut keep = Vec::with_capacity(self.len());
        for row_position in 0..self.len() {
            let mut missing_count = 0_usize;
            for name in &selected_columns {
                let column = self.columns.get(name).expect("selected column must exist");
                if column.values()[row_position].is_missing() {
                    missing_count += 1;
                }
            }
            let row_keep = match how {
                DropNaHow::Any => missing_count == 0,
                DropNaHow::All => missing_count < selected_columns.len(),
            };
            keep.push(row_keep);
        }

        let mask = Series::from_values(
            "__dropna_mask__",
            self.index.labels().to_vec(),
            keep.into_iter().map(Scalar::Bool).collect::<Vec<_>>(),
        )?;
        self.filter_rows(&mask)
    }

    /// Drop rows by minimum non-missing value count.
    ///
    /// Matches `df.dropna(thresh=..., subset=...)` for row-wise mode (`axis=0`).
    pub fn dropna_with_threshold(
        &self,
        thresh: usize,
        subset: Option<&[String]>,
    ) -> Result<Self, FrameError> {
        if self.is_empty() || thresh == 0 {
            return Ok(self.clone());
        }

        let selected_columns = self.resolve_column_selector(subset)?;
        if selected_columns.is_empty() {
            return Ok(self.clone());
        }

        let mut keep = Vec::with_capacity(self.len());
        for row_position in 0..self.len() {
            let mut non_missing_count = 0_usize;
            for name in &selected_columns {
                let column = self.columns.get(name).expect("selected column must exist");
                if !column.values()[row_position].is_missing() {
                    non_missing_count += 1;
                }
            }
            keep.push(non_missing_count >= thresh);
        }

        let mask = Series::from_values(
            "__dropna_thresh_mask__",
            self.index.labels().to_vec(),
            keep.into_iter().map(Scalar::Bool).collect::<Vec<_>>(),
        )?;
        self.filter_rows(&mask)
    }

    /// Drop columns containing missing values in any row.
    ///
    /// Matches default `df.dropna(axis=1)` behavior (`how='any'`).
    pub fn dropna_columns(&self) -> Result<Self, FrameError> {
        self.dropna_columns_with_options(DropNaHow::Any, None)
    }

    /// Drop columns by configurable missing-value policy.
    ///
    /// Matches `df.dropna(axis=1, how=..., subset=...)` where `subset` selects
    /// row labels to consider during missing-value checks.
    pub fn dropna_columns_with_options(
        &self,
        how: DropNaHow,
        subset: Option<&[IndexLabel]>,
    ) -> Result<Self, FrameError> {
        if self.column_order.is_empty() {
            return Ok(self.clone());
        }

        let selected_rows = self.resolve_row_label_selector(subset)?;
        if selected_rows.is_empty() {
            return Ok(self.clone());
        }

        let mut keep_columns = Vec::with_capacity(self.column_order.len());
        for name in &self.column_order {
            let column = self.columns.get(name).expect("selected column must exist");
            let mut missing_count = 0_usize;
            for &row_position in &selected_rows {
                if column.values()[row_position].is_missing() {
                    missing_count += 1;
                }
            }

            let column_keep = match how {
                DropNaHow::Any => missing_count == 0,
                DropNaHow::All => missing_count < selected_rows.len(),
            };
            if column_keep {
                keep_columns.push(name.clone());
            }
        }

        let mut columns = BTreeMap::new();
        for name in &keep_columns {
            let column = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            columns.insert(name.clone(), column.clone());
        }

        Self::new_with_column_order(self.index.clone(), columns, keep_columns)
    }

    /// Drop columns by minimum non-missing value count.
    ///
    /// Matches `df.dropna(axis=1, thresh=..., subset=...)` where `subset`
    /// selects row labels to consider.
    pub fn dropna_columns_with_threshold(
        &self,
        thresh: usize,
        subset: Option<&[IndexLabel]>,
    ) -> Result<Self, FrameError> {
        if self.column_order.is_empty() || thresh == 0 {
            return Ok(self.clone());
        }

        let selected_rows = self.resolve_row_label_selector(subset)?;
        if selected_rows.is_empty() {
            return Ok(self.clone());
        }

        let mut keep_columns = Vec::with_capacity(self.column_order.len());
        for name in &self.column_order {
            let column = self.columns.get(name).expect("selected column must exist");
            let mut non_missing_count = 0_usize;
            for &row_position in &selected_rows {
                if !column.values()[row_position].is_missing() {
                    non_missing_count += 1;
                }
            }
            if non_missing_count >= thresh {
                keep_columns.push(name.clone());
            }
        }

        let mut columns = BTreeMap::new();
        for name in &keep_columns {
            let column = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            columns.insert(name.clone(), column.clone());
        }

        Self::new_with_column_order(self.index.clone(), columns, keep_columns)
    }

    /// Return the first `n` rows.
    ///
    /// Matches `df.head(n)`. If `n` is negative, this returns all rows except
    /// the last `-n` rows.
    pub fn head(&self, n: i64) -> Result<Self, FrameError> {
        let take = normalize_head_take(n, self.len());
        let labels = self.index.labels()[..take].to_vec();
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let col = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let values = col.values()[..take].to_vec();
            columns.insert(name.clone(), Column::from_values(values)?);
        }
        Self::new_with_column_order(Index::new(labels), columns, self.column_order.clone())
    }

    /// Return the last `n` rows.
    ///
    /// Matches `df.tail(n)`. If `n` is negative, this returns all rows except
    /// the first `-n` rows.
    pub fn tail(&self, n: i64) -> Result<Self, FrameError> {
        let (start, _) = normalize_tail_window(n, self.len());
        let labels = self.index.labels()[start..].to_vec();
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let col = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let values = col.values()[start..].to_vec();
            columns.insert(name.clone(), Column::from_values(values)?);
        }
        Self::new_with_column_order(Index::new(labels), columns, self.column_order.clone())
    }

    /// Set the DataFrame index from an existing column.
    ///
    /// Matches `df.set_index(column, drop=...)` for a single column selector.
    pub fn set_index(&self, column: &str, drop: bool) -> Result<Self, FrameError> {
        let source = self.columns.get(column).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("column '{column}' not found"))
        })?;

        let labels = source
            .values()
            .iter()
            .map(scalar_to_index_label)
            .collect::<Result<Vec<_>, _>>()?;
        let index = Index::new(labels);

        if !drop {
            return Self::new_with_column_order(
                index,
                self.columns.clone(),
                self.column_order.clone(),
            );
        }

        let mut columns = self.columns.clone();
        columns.remove(column);
        let column_order = self
            .column_order
            .iter()
            .filter(|name| name.as_str() != column)
            .cloned()
            .collect::<Vec<_>>();
        Self::new_with_column_order(index, columns, column_order)
    }

    /// Reset the index to a default RangeIndex.
    ///
    /// Matches `df.reset_index(drop=...)` for the single-index DataFrame model.
    pub fn reset_index(&self, drop: bool) -> Result<Self, FrameError> {
        let index = range_index(self.len())?;
        if drop {
            return Self::new_with_column_order(
                index,
                self.columns.clone(),
                self.column_order.clone(),
            );
        }

        let has_int = self
            .index
            .labels()
            .iter()
            .any(|label| matches!(label, IndexLabel::Int64(_)));
        let has_utf8 = self
            .index
            .labels()
            .iter()
            .any(|label| matches!(label, IndexLabel::Utf8(_)));
        if has_int && has_utf8 {
            return Err(FrameError::CompatibilityRejected(
                "reset_index does not yet support mixed Int64/Utf8 index labels".to_owned(),
            ));
        }

        let index_column_name = self.reset_index_column_name()?;
        let index_values = self
            .index
            .labels()
            .iter()
            .map(index_label_to_scalar)
            .collect::<Vec<_>>();

        let mut columns = self.columns.clone();
        columns.insert(
            index_column_name.clone(),
            Column::from_values(index_values)?,
        );
        let mut column_order = Vec::with_capacity(self.column_order.len() + 1);
        column_order.push(index_column_name);
        column_order.extend(self.column_order.iter().cloned());

        Self::new_with_column_order(index, columns, column_order)
    }

    /// Return a new DataFrame sorted by index labels.
    ///
    /// Matches `df.sort_index(ascending=...)` for 1D index labels.
    pub fn sort_index(&self, ascending: bool) -> Result<Self, FrameError> {
        let mut order = self.index.argsort();
        if !ascending {
            order.reverse();
        }
        self.reorder_rows_by_positions(&order)
    }

    /// Return a new DataFrame sorted by a column's values.
    ///
    /// Matches `df.sort_values(by=column, ascending=...)` for a single
    /// column key with default `na_position='last'`.
    pub fn sort_values(&self, column: &str, ascending: bool) -> Result<Self, FrameError> {
        let sort_column = self.columns.get(column).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("column '{column}' not found"))
        })?;

        let mut order = (0..self.len()).collect::<Vec<_>>();
        order.sort_by(|&left_pos, &right_pos| {
            compare_scalars_with_na_last(
                &sort_column.values()[left_pos],
                &sort_column.values()[right_pos],
                ascending,
            )
        });

        self.reorder_rows_by_positions(&order)
    }

    /// Label-based row selection for list-like indexers.
    ///
    /// Matches `df.loc[[...]]` for list selectors. Requested labels are
    /// returned in selector order and duplicate labels are preserved.
    /// Missing labels fail closed.
    pub fn loc(&self, labels: &[IndexLabel]) -> Result<Self, FrameError> {
        self.loc_with_columns(labels, None)
    }

    /// Label-based row+column selection for list-like indexers.
    ///
    /// Matches `df.loc[[...], [...]]` for list selectors. Requested rows are
    /// returned in selector order and duplicate row labels are preserved.
    /// Missing labels/columns fail closed.
    pub fn loc_with_columns(
        &self,
        labels: &[IndexLabel],
        column_selector: Option<&[String]>,
    ) -> Result<Self, FrameError> {
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

        let selected_columns = self.resolve_column_selector(column_selector)?;
        let mut columns = BTreeMap::new();
        for name in &selected_columns {
            let column = self.columns.get(name).ok_or_else(|| {
                FrameError::CompatibilityRejected(format!("column '{name}' not found"))
            })?;
            let mut values = Vec::with_capacity(positions.len());
            for &position in &positions {
                values.push(column.values()[position].clone());
            }
            columns.insert(name.clone(), Column::from_values(values)?);
        }

        Self::new_with_column_order(Index::new(out_labels), columns, selected_columns)
    }

    /// Position-based row selection for list-like indexers.
    ///
    /// Matches `df.iloc[[...]]` for list selectors. Requested positions are
    /// returned in selector order and duplicates are preserved.
    /// Negative positions are resolved from the end of the DataFrame.
    pub fn iloc(&self, positions: &[i64]) -> Result<Self, FrameError> {
        self.iloc_with_columns(positions, None)
    }

    /// Position-based row+column selection for list-like indexers.
    ///
    /// Matches `df.iloc[[...], [...]]` for list selectors. Requested rows are
    /// returned in selector order and duplicates are preserved.
    /// Missing columns fail closed.
    pub fn iloc_with_columns(
        &self,
        positions: &[i64],
        column_selector: Option<&[String]>,
    ) -> Result<Self, FrameError> {
        let normalized_positions = positions
            .iter()
            .copied()
            .map(|position| normalize_iloc_position(position, self.len()))
            .collect::<Result<Vec<_>, _>>()?;

        let mut out_labels = Vec::with_capacity(normalized_positions.len());
        for &position in &normalized_positions {
            out_labels.push(self.index.labels()[position].clone());
        }

        let selected_columns = self.resolve_column_selector(column_selector)?;
        let mut columns = BTreeMap::new();
        for name in &selected_columns {
            let column = self.columns.get(name).ok_or_else(|| {
                FrameError::CompatibilityRejected(format!("column '{name}' not found"))
            })?;
            let mut values = Vec::with_capacity(normalized_positions.len());
            for &position in &normalized_positions {
                values.push(column.values()[position].clone());
            }
            columns.insert(name.clone(), Column::from_values(values)?);
        }

        Self::new_with_column_order(Index::new(out_labels), columns, selected_columns)
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
        let name = name.into();
        let mut columns = self.columns.clone();
        columns.insert(name.clone(), column);
        let mut column_order = self.column_order.clone();
        if !column_order.contains(&name) {
            column_order.push(name);
        }
        Self::new_with_column_order(self.index.clone(), columns, column_order)
    }

    /// Cast a single column to a target dtype.
    ///
    /// Matches `df.astype({\"col\": dtype})` for single-column mapping.
    pub fn astype_column(&self, name: &str, dtype: DType) -> Result<Self, FrameError> {
        let source = self.columns.get(name).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("column '{name}' not found"))
        })?;

        let casted = Column::new(dtype, source.values().to_vec())?;
        let mut columns = self.columns.clone();
        columns.insert(name.to_owned(), casted);
        Self::new_with_column_order(self.index.clone(), columns, self.column_order.clone())
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
        let column_order = self
            .column_order
            .iter()
            .filter(|column| column.as_str() != name)
            .cloned()
            .collect();
        Self::new_with_column_order(self.index.clone(), columns, column_order)
    }

    /// Rename columns using a mapping.
    ///
    /// Matches `df.rename(columns={'old': 'new'})`.
    pub fn rename_columns(&self, mapping: &[(&str, &str)]) -> Result<Self, FrameError> {
        let rename_map: BTreeMap<&str, &str> = mapping.iter().copied().collect();
        let mut columns = BTreeMap::new();
        let mut column_order = Vec::with_capacity(self.column_order.len());
        for name in &self.column_order {
            let col = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            let name_str = name.as_str();
            let new_name = rename_map.get(name_str).unwrap_or(&name_str);
            column_order.push((*new_name).to_owned());
            columns.insert((*new_name).to_owned(), col.clone());
        }
        Self::new_with_column_order(self.index.clone(), columns, column_order)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::{DType, NullKind, Scalar};

    use super::{DataFrame, DropNaHow, FrameError, IndexLabel, Series};

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
        assert!(matches!(out.values()[2], Scalar::Float64(_)));
        let v = match out.values()[2] {
            Scalar::Float64(f) => f,
            _ => 0.0,
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
        assert!(matches!(out.values()[0], Scalar::Float64(_)));
        let v = match out.values()[0] {
            Scalar::Float64(f) => f,
            _ => 0.0,
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
        assert!(matches!(add.values()[0], Scalar::Float64(_)));
        let v = match add.values()[0] {
            Scalar::Float64(f) => f,
            _ => 0.0,
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
        assert!(matches!(div.values()[0], Scalar::Float64(_)));
        assert!(matches!(div.values()[1], Scalar::Float64(_)));
        let v0 = match div.values()[0] {
            Scalar::Float64(f) => f,
            _ => 0.0,
        };
        let v1 = match div.values()[1] {
            Scalar::Float64(f) => f,
            _ => 0.0,
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
    fn dataframe_from_records_sparse_keys_null_fill() {
        let records = vec![
            BTreeMap::from([
                ("a".to_owned(), Scalar::Int64(1)),
                ("b".to_owned(), Scalar::Int64(10)),
            ]),
            BTreeMap::from([("a".to_owned(), Scalar::Int64(2))]),
            BTreeMap::from([("b".to_owned(), Scalar::Int64(30))]),
        ];

        let df = DataFrame::from_records(records, None, None).unwrap();
        assert_eq!(
            df.index().labels(),
            &[0_i64.into(), 1_i64.into(), 2_i64.into()]
        );
        assert_eq!(
            df.column("a").unwrap().values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null),
            ]
        );
        assert_eq!(
            df.column("b").unwrap().values(),
            &[
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ]
        );
    }

    #[test]
    fn dataframe_from_records_column_order_allows_new_null_column() {
        let records = vec![
            BTreeMap::from([("a".to_owned(), Scalar::Int64(1))]),
            BTreeMap::from([("a".to_owned(), Scalar::Int64(2))]),
        ];
        let order = vec!["a".to_owned(), "z".to_owned()];

        let df = DataFrame::from_records(records, Some(&order), None).unwrap();
        let names = df
            .column_names()
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "z"]);
        assert_eq!(
            df.column("z").unwrap().values(),
            &[Scalar::Null(NullKind::Null), Scalar::Null(NullKind::Null)]
        );
    }

    #[test]
    fn dataframe_from_records_with_explicit_index() {
        let records = vec![
            BTreeMap::from([("city".to_owned(), Scalar::Utf8("ny".to_owned()))]),
            BTreeMap::from([("city".to_owned(), Scalar::Utf8("la".to_owned()))]),
        ];

        let df = DataFrame::from_records(
            records,
            None,
            Some(vec![
                IndexLabel::Utf8("row-a".to_owned()),
                IndexLabel::Utf8("row-b".to_owned()),
            ]),
        )
        .unwrap();
        assert_eq!(
            df.index().labels(),
            &[
                IndexLabel::Utf8("row-a".to_owned()),
                IndexLabel::Utf8("row-b".to_owned())
            ]
        );
    }

    #[test]
    fn dataframe_from_records_explicit_index_mismatch() {
        let records = vec![
            BTreeMap::from([("a".to_owned(), Scalar::Int64(1))]),
            BTreeMap::from([("a".to_owned(), Scalar::Int64(2))]),
        ];

        let err = DataFrame::from_records(
            records,
            None,
            Some(vec![IndexLabel::Utf8("only-one".to_owned())]),
        )
        .expect_err("should reject index length mismatch");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
        assert_eq!(
            err.to_string(),
            "compatibility gate rejected operation: dataframe_from_records index length 1 does not match records length 2"
        );
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
    fn dataframe_from_dict_preserves_explicit_column_order() {
        let df = DataFrame::from_dict(
            &["b", "a", "c"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Int64(2)]),
                ("c", vec![Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let names = df
            .column_names()
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["b", "a", "c"]);
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

    use super::{
        ConcatJoin, concat_dataframes, concat_dataframes_with_axis,
        concat_dataframes_with_axis_join,
    };

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
    fn concat_dataframes_mismatched_columns_outer_unions_and_null_fills() {
        let df1 = DataFrame::from_dict(
            &["b", "a"],
            vec![
                ("b", vec![Scalar::Int64(10)]),
                ("a", vec![Scalar::Int64(1)]),
            ],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["c", "a"],
            vec![
                ("c", vec![Scalar::Int64(20)]),
                ("a", vec![Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let out = concat_dataframes(&[&df1, &df2]).unwrap();
        let column_names = out
            .column_names()
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(column_names, vec!["b", "a", "c"]);
        assert_eq!(
            out.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            out.column("b").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Null(NullKind::Null)]
        );
        assert_eq!(
            out.column("c").unwrap().values(),
            &[Scalar::Null(NullKind::Null), Scalar::Int64(20)]
        );
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

    #[test]
    fn concat_dataframes_axis1_aligns_and_fills_nulls() {
        let left = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![0_i64.into(), 1_i64.into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![("b", vec![Scalar::Int64(10), Scalar::Int64(20)])],
            vec![1_i64.into(), 2_i64.into()],
        )
        .unwrap();

        let out = concat_dataframes_with_axis(&[&left, &right], 1).unwrap();
        assert_eq!(
            out.index().labels(),
            &[0_i64.into(), 1_i64.into(), 2_i64.into()]
        );
        assert_eq!(
            out.column("a").unwrap().values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null)
            ]
        );
        assert_eq!(
            out.column("b").unwrap().values(),
            &[
                Scalar::Null(NullKind::Null),
                Scalar::Int64(10),
                Scalar::Int64(20)
            ]
        );
    }

    #[test]
    fn concat_dataframes_axis1_duplicate_columns_rejects() {
        let left = DataFrame::from_dict_with_index(
            vec![("x", vec![Scalar::Int64(1)])],
            vec![0_i64.into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![("x", vec![Scalar::Int64(2)])],
            vec![0_i64.into()],
        )
        .unwrap();

        let err = concat_dataframes_with_axis(&[&left, &right], 1).expect_err("should reject");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
        assert!(err.to_string().contains("duplicate column"));
    }

    #[test]
    fn concat_dataframes_axis1_duplicate_index_rejects() {
        let left = DataFrame::from_dict_with_index(
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![0_i64.into(), 0_i64.into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![("y", vec![Scalar::Int64(3)])],
            vec![0_i64.into()],
        )
        .unwrap();

        let err = concat_dataframes_with_axis(&[&left, &right], 1).expect_err("should reject");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
        assert!(err.to_string().contains("duplicate index"));
    }

    #[test]
    fn concat_dataframes_with_axis_rejects_unknown_axis() {
        let frame = DataFrame::from_dict(&["x"], vec![("x", vec![Scalar::Int64(1)])]).unwrap();
        let err = concat_dataframes_with_axis(&[&frame], 2).expect_err("should reject");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
        assert!(err.to_string().contains("unsupported concat axis"));
    }

    #[test]
    fn concat_dataframes_axis1_inner_keeps_intersection_in_left_order() {
        let left = DataFrame::from_dict_with_index(
            vec![(
                "a",
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )],
            vec!["b".into(), "a".into(), "c".into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![(
                "b",
                vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
            )],
            vec!["a".into(), "c".into(), "d".into()],
        )
        .unwrap();

        let out = concat_dataframes_with_axis_join(&[&left, &right], 1, ConcatJoin::Inner).unwrap();
        assert_eq!(out.index().labels(), &["a".into(), "c".into()]);
        assert_eq!(
            out.column("a").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );
        assert_eq!(
            out.column("b").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(3)]
        );
    }

    #[test]
    fn concat_dataframes_axis1_inner_disjoint_yields_empty_index() {
        let left = DataFrame::from_dict_with_index(
            vec![("x", vec![Scalar::Int64(1)])],
            vec![0_i64.into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![("y", vec![Scalar::Int64(2)])],
            vec![1_i64.into()],
        )
        .unwrap();

        let out = concat_dataframes_with_axis_join(&[&left, &right], 1, ConcatJoin::Inner).unwrap();
        assert!(out.index().is_empty());
        assert!(out.column("x").unwrap().values().is_empty());
        assert!(out.column("y").unwrap().values().is_empty());
    }

    #[test]
    fn concat_dataframes_axis0_inner_uses_column_intersection() {
        let left = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["b", "c"],
            vec![
                ("b", vec![Scalar::Int64(100), Scalar::Int64(200)]),
                ("c", vec![Scalar::Int64(7), Scalar::Int64(8)]),
            ],
        )
        .unwrap();

        let out = concat_dataframes_with_axis_join(&[&left, &right], 0, ConcatJoin::Inner).unwrap();
        let column_names = out.column_names();
        assert_eq!(column_names.len(), 1);
        assert_eq!(column_names[0], "b");
        assert_eq!(
            out.column("b").unwrap().values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(100),
                Scalar::Int64(200)
            ]
        );
        assert_eq!(out.index().labels().len(), 4);
    }

    #[test]
    fn concat_dataframes_axis0_inner_no_shared_columns_yields_empty_columns() {
        let left = DataFrame::from_dict(&["x"], vec![("x", vec![Scalar::Int64(1)])]).unwrap();
        let right = DataFrame::from_dict(&["y"], vec![("y", vec![Scalar::Int64(2)])]).unwrap();

        let out = concat_dataframes_with_axis_join(&[&left, &right], 0, ConcatJoin::Inner).unwrap();
        assert!(out.column_names().is_empty());
        assert_eq!(out.index().labels().len(), 2);
    }

    #[test]
    fn concat_dataframes_axis0_inner_all_shared_columns_matches_row_concat() {
        let left = DataFrame::from_dict(&["x"], vec![("x", vec![Scalar::Int64(1)])]).unwrap();
        let right = DataFrame::from_dict(&["x"], vec![("x", vec![Scalar::Int64(2)])]).unwrap();

        let out = concat_dataframes_with_axis_join(&[&left, &right], 0, ConcatJoin::Inner).unwrap();
        assert_eq!(
            out.column("x").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
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

    #[test]
    fn series_logical_ops_with_alignment_and_nulls() {
        let left = Series::from_values(
            "m1",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Null(NullKind::Null),
            ],
        )
        .unwrap();
        let right = Series::from_values(
            "m2",
            vec![2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Bool(true),
                Scalar::Null(NullKind::Null),
                Scalar::Bool(false),
            ],
        )
        .unwrap();

        let and = left.and(&right).unwrap();
        assert_eq!(
            and.values(),
            &[
                Scalar::Null(NullKind::Null),
                Scalar::Bool(false),
                Scalar::Null(NullKind::Null),
                Scalar::Bool(false)
            ]
        );

        let or = left.or(&right).unwrap();
        assert_eq!(
            or.values(),
            &[
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Null(NullKind::Null),
                Scalar::Null(NullKind::Null)
            ]
        );

        let not_left = left.not().unwrap();
        assert_eq!(
            not_left.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Null(NullKind::Null)
            ]
        );
    }

    #[test]
    fn series_logical_ops_reject_non_boolean_input() {
        let left = Series::from_values(
            "left",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(0)],
        )
        .unwrap();
        let right = Series::from_values(
            "right",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Bool(true), Scalar::Bool(false)],
        )
        .unwrap();

        let and_err = left.and(&right).unwrap_err();
        assert!(
            matches!(and_err, FrameError::CompatibilityRejected(msg) if msg.contains("boolean series required for and"))
        );

        let not_err = left.not().unwrap_err();
        assert!(
            matches!(not_err, FrameError::CompatibilityRejected(msg) if msg.contains("boolean series required for not"))
        );
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

    #[test]
    fn series_filter_rejects_non_boolean_mask() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let mask = Series::from_values(
            "mask",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(0)],
        )
        .unwrap();

        let err = s.filter(&mask).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("boolean mask required for filter"))
        );
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

    #[test]
    fn series_isna_notna_flags_null_and_nan() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(f64::NAN),
                Scalar::Bool(false),
            ],
        )
        .unwrap();

        let isna = s.isna().unwrap();
        assert_eq!(
            isna.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );

        let notna = s.notna().unwrap();
        assert_eq!(
            notna.values(),
            &[
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(false),
                Scalar::Bool(true)
            ]
        );

        let isnull = s.isnull().unwrap();
        assert_eq!(isnull.values(), isna.values());

        let notnull = s.notnull().unwrap();
        assert_eq!(notnull.values(), notna.values());
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
    fn series_astype_casts_and_preserves_index() {
        let s = Series::from_values(
            "vals",
            vec![IndexLabel::from("r1"), IndexLabel::from("r2")],
            vec![Scalar::Int64(1), Scalar::Null(NullKind::Null)],
        )
        .unwrap();

        let casted = s.astype(DType::Float64).unwrap();
        assert_eq!(
            casted.index().labels(),
            &[IndexLabel::from("r1"), IndexLabel::from("r2")]
        );
        assert_eq!(casted.column().dtype(), DType::Float64);
        assert_eq!(
            casted.values(),
            &[Scalar::Float64(1.0), Scalar::Null(NullKind::NaN)]
        );
    }

    #[test]
    fn series_astype_rejects_invalid_cast() {
        let s = Series::from_values(
            "vals",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("a".to_owned()), Scalar::Utf8("b".to_owned())],
        )
        .unwrap();

        let err = s.astype(DType::Int64).unwrap_err();
        assert!(matches!(err, FrameError::Column(_)));
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
        assert!(matches!(s.var().unwrap(), Scalar::Float64(_)));
        let var = match s.var().unwrap() {
            Scalar::Float64(v) => v,
            _ => 0.0,
        };
        assert!((var - 1.0).abs() < 1e-10);

        assert!(matches!(s.std().unwrap(), Scalar::Float64(_)));
        let std = match s.std().unwrap() {
            Scalar::Float64(v) => v,
            _ => 0.0,
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
        assert!(matches!(s.mean().unwrap(), Scalar::Float64(v) if v.is_nan()));
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

    #[test]
    fn series_iloc_negative_positions_resolve_from_end() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from(10_i64),
                IndexLabel::from(11_i64),
                IndexLabel::from(12_i64),
                IndexLabel::from(13_i64),
            ],
            vec![
                Scalar::Int64(1000),
                Scalar::Int64(1100),
                Scalar::Int64(1200),
                Scalar::Int64(1300),
            ],
        )
        .unwrap();

        let selected = s.iloc(&[-1, -3, 0]).unwrap();
        assert_eq!(
            selected.index().labels(),
            &[
                IndexLabel::from(13_i64),
                IndexLabel::from(11_i64),
                IndexLabel::from(10_i64)
            ]
        );
        assert_eq!(
            selected.values(),
            &[
                Scalar::Int64(1300),
                Scalar::Int64(1100),
                Scalar::Int64(1000)
            ]
        );
    }

    #[test]
    fn series_iloc_negative_out_of_bounds_is_rejected() {
        let s = Series::from_values(
            "vals",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let err = s.iloc(&[-3]).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("out of bounds"))
        );
    }

    #[test]
    fn series_head_tail() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
                IndexLabel::from("d"),
                IndexLabel::from("e"),
            ],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
                Scalar::Int64(5),
            ],
        )
        .unwrap();

        let head = s.head(3).unwrap();
        assert_eq!(
            head.index().labels(),
            &[
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c")
            ]
        );
        assert_eq!(
            head.values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );

        let tail = s.tail(2).unwrap();
        assert_eq!(
            tail.index().labels(),
            &[IndexLabel::from("d"), IndexLabel::from("e")]
        );
        assert_eq!(tail.values(), &[Scalar::Int64(4), Scalar::Int64(5)]);
    }

    #[test]
    fn series_head_tail_negative_n() {
        let s = Series::from_values(
            "vals",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
            ],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
                Scalar::Int64(50),
            ],
        )
        .unwrap();

        let head = s.head(-2).unwrap();
        assert_eq!(
            head.values(),
            &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]
        );

        let tail = s.tail(-2).unwrap();
        assert_eq!(
            tail.values(),
            &[Scalar::Int64(30), Scalar::Int64(40), Scalar::Int64(50)]
        );
    }

    #[test]
    fn series_head_tail_negative_n_saturates_to_empty() {
        let s = Series::from_values(
            "vals",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        let head = s.head(-10).unwrap();
        assert_eq!(head.len(), 0);
        assert_eq!(head.values(), &[]);

        let tail = s.tail(-10).unwrap();
        assert_eq!(tail.len(), 0);
        assert_eq!(tail.values(), &[]);
    }

    // ---- DataFrame filter_rows/fillna/dropna/head/tail/column-mutation tests ----

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
    fn dataframe_filter_rows_rejects_non_boolean_mask() {
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
            vec![Scalar::Int64(1), Scalar::Int64(0), Scalar::Int64(1)],
        )
        .unwrap();

        let err = df.filter_rows(&mask).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("boolean mask required for filter_rows"))
        );
    }

    #[test]
    fn dataframe_fillna_replaces_nulls() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )
        .unwrap();

        let filled = df.fillna(&Scalar::Int64(0)).unwrap();
        assert_eq!(
            filled.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(0), Scalar::Int64(3)]
        );
        assert_eq!(
            filled.column("b").unwrap().values(),
            &[Scalar::Int64(0), Scalar::Int64(20), Scalar::Int64(30)]
        );
    }

    #[test]
    fn dataframe_dropna_drops_rows_with_any_null() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )
        .unwrap();

        let dropped = df.dropna().unwrap();
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped.index().labels(), &[IndexLabel::from(2_i64)]);
        assert_eq!(dropped.column("a").unwrap().values(), &[Scalar::Int64(3)]);
        assert_eq!(dropped.column("b").unwrap().values(), &[Scalar::Int64(30)]);
    }

    #[test]
    fn dataframe_dropna_with_options_how_all_drops_only_all_missing_rows() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
            ],
        )
        .unwrap();

        let dropped = df.dropna_with_options(DropNaHow::All, None).unwrap();
        assert_eq!(dropped.len(), 2);
        assert_eq!(
            dropped.index().labels(),
            &[IndexLabel::from(0_i64), IndexLabel::from(2_i64)]
        );
    }

    #[test]
    fn dataframe_dropna_with_options_subset_any_scopes_missing_check() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Null(NullKind::Null), Scalar::Int64(2)]),
                ("b", vec![Scalar::Null(NullKind::Null), Scalar::Int64(20)]),
            ],
        )
        .unwrap();

        let subset = vec!["a".to_owned()];
        let dropped = df
            .dropna_with_options(DropNaHow::Any, Some(&subset))
            .unwrap();
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped.index().labels(), &[IndexLabel::from(1_i64)]);
    }

    #[test]
    fn dataframe_dropna_with_options_rejects_invalid_subset() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();

        let missing_subset = vec!["z".to_owned()];
        let err = df
            .dropna_with_options(DropNaHow::Any, Some(&missing_subset))
            .unwrap_err();
        assert!(matches!(
            err,
            FrameError::CompatibilityRejected(msg) if msg.contains("column 'z' not found")
        ));

        let duplicate_subset = vec!["a".to_owned(), "a".to_owned()];
        let err = df
            .dropna_with_options(DropNaHow::Any, Some(&duplicate_subset))
            .unwrap_err();
        assert!(matches!(
            err,
            FrameError::CompatibilityRejected(msg) if msg.contains("duplicate column selector")
        ));
    }

    #[test]
    fn dataframe_dropna_with_threshold_keeps_rows_meeting_non_missing_count() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "c",
                    vec![Scalar::Int64(9), Scalar::Int64(8), Scalar::Int64(7)],
                ),
            ],
        )
        .unwrap();

        let dropped = df.dropna_with_threshold(2, None).unwrap();
        assert_eq!(
            dropped.index().labels(),
            &[IndexLabel::from(0_i64), IndexLabel::from(2_i64)]
        );
    }

    #[test]
    fn dataframe_dropna_with_threshold_subset_scopes_column_checks() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "c",
                    vec![Scalar::Int64(5), Scalar::Int64(6), Scalar::Int64(7)],
                ),
            ],
        )
        .unwrap();

        let subset = vec!["a".to_owned(), "b".to_owned()];
        let dropped = df.dropna_with_threshold(1, Some(&subset)).unwrap();
        assert_eq!(
            dropped.index().labels(),
            &[IndexLabel::from(1_i64), IndexLabel::from(2_i64)]
        );
    }

    #[test]
    fn dataframe_dropna_columns_with_options_how_any_drops_columns_with_any_missing() {
        let df = DataFrame::from_dict(
            &["a", "b", "c", "d"],
            vec![
                (
                    "a",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                    ],
                ),
                (
                    "c",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(30),
                    ],
                ),
                (
                    "d",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )
        .unwrap();

        let dropped = df
            .dropna_columns_with_options(DropNaHow::Any, None)
            .unwrap();
        let column_names = dropped
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(column_names, vec!["a".to_owned()]);
        assert_eq!(
            dropped.column("a").unwrap().values(),
            df.column("a").unwrap().values()
        );
    }

    #[test]
    fn dataframe_dropna_columns_defaults_to_axis1_any() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Null(NullKind::Null), Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let default_drop = df.dropna_columns().unwrap();
        let optioned_drop = df
            .dropna_columns_with_options(DropNaHow::Any, None)
            .unwrap();
        assert_eq!(default_drop, optioned_drop);
    }

    #[test]
    fn dataframe_dropna_columns_with_options_how_all_drops_only_all_missing_columns() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "c",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
            ],
        )
        .unwrap();

        let dropped = df
            .dropna_columns_with_options(DropNaHow::All, None)
            .unwrap();
        let column_names = dropped
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(column_names, vec!["a".to_owned(), "c".to_owned()]);
    }

    #[test]
    fn dataframe_dropna_columns_with_options_subset_scopes_row_checks() {
        let df = DataFrame::from_dict_with_index(
            vec![
                (
                    "a",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(5),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(6),
                    ],
                ),
                (
                    "c",
                    vec![Scalar::Int64(7), Scalar::Int64(8), Scalar::Int64(9)],
                ),
            ],
            vec!["r0".into(), "r1".into(), "r2".into()],
        )
        .unwrap();

        let subset = vec![IndexLabel::from("r0")];
        let dropped = df
            .dropna_columns_with_options(DropNaHow::Any, Some(&subset))
            .unwrap();
        let column_names = dropped
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(column_names, vec!["b".to_owned(), "c".to_owned()]);
    }

    #[test]
    fn dataframe_dropna_columns_with_options_rejects_missing_subset_label() {
        let df = DataFrame::from_dict_with_index(
            vec![(
                "a",
                vec![
                    Scalar::Int64(1),
                    Scalar::Null(NullKind::Null),
                    Scalar::Int64(3),
                ],
            )],
            vec!["r0".into(), "r1".into(), "r2".into()],
        )
        .unwrap();

        let subset = vec![IndexLabel::from("missing")];
        let err = df
            .dropna_columns_with_options(DropNaHow::Any, Some(&subset))
            .unwrap_err();
        assert!(matches!(
            err,
            FrameError::CompatibilityRejected(msg) if msg.contains("index label 'missing' not found")
        ));
    }

    #[test]
    fn dataframe_dropna_columns_with_threshold_keeps_columns_meeting_non_missing_count() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                (
                    "a",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "c",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
            ],
        )
        .unwrap();

        let dropped = df.dropna_columns_with_threshold(2, None).unwrap();
        let column_names = dropped
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(column_names, vec!["a".to_owned(), "c".to_owned()]);
    }

    #[test]
    fn dataframe_dropna_columns_with_threshold_subset_scopes_row_checks() {
        let df = DataFrame::from_dict_with_index(
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(2),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "c",
                    vec![Scalar::Int64(3), Scalar::Int64(3), Scalar::Int64(3)],
                ),
            ],
            vec!["r0".into(), "r1".into(), "r2".into()],
        )
        .unwrap();

        let subset = vec![IndexLabel::from("r0"), IndexLabel::from("r1")];
        let dropped = df.dropna_columns_with_threshold(2, Some(&subset)).unwrap();
        let column_names = dropped
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(column_names, vec!["c".to_owned()]);
    }

    #[test]
    fn dataframe_isna_notna_flags_missing_per_cell() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(f64::NAN),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )
        .unwrap();

        let isna = df.isna().unwrap();
        assert_eq!(
            isna.column("a").unwrap().values(),
            &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(true)]
        );
        assert_eq!(
            isna.column("b").unwrap().values(),
            &[Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(false)]
        );

        let notna = df.notna().unwrap();
        assert_eq!(
            notna.column("a").unwrap().values(),
            &[Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(false)]
        );
        assert_eq!(
            notna.column("b").unwrap().values(),
            &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(true)]
        );

        let isnull = df.isnull().unwrap();
        assert_eq!(isnull, isna);

        let notnull = df.notnull().unwrap();
        assert_eq!(notnull, notna);
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
    fn dataframe_head_tail_negative_n() {
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

        let head = df.head(-2).unwrap();
        assert_eq!(head.len(), 3);
        assert_eq!(
            head.column("v").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );

        let tail = df.tail(-2).unwrap();
        assert_eq!(tail.len(), 3);
        assert_eq!(
            tail.column("v").unwrap().values(),
            &[Scalar::Int64(3), Scalar::Int64(4), Scalar::Int64(5)]
        );
    }

    #[test]
    fn dataframe_head_tail_negative_n_saturates_to_empty() {
        let df = DataFrame::from_dict(
            &["v"],
            vec![(
                "v",
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )],
        )
        .unwrap();

        let head = df.head(-10).unwrap();
        assert_eq!(head.len(), 0);
        assert_eq!(head.column("v").unwrap().values(), &[]);

        let tail = df.tail(-10).unwrap();
        assert_eq!(tail.len(), 0);
        assert_eq!(tail.column("v").unwrap().values(), &[]);
    }

    #[test]
    fn dataframe_set_index_drop_true_moves_column_into_index() {
        let df = DataFrame::from_dict(
            &["id", "v"],
            vec![
                ("id", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let out = df.set_index("id", true).unwrap();
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from(10_i64), IndexLabel::from(20_i64)]
        );
        let names = out.column_names().into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(names, vec!["v".to_owned()]);
        assert!(out.column("id").is_none());
        assert_eq!(
            out.column("v").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
    }

    #[test]
    fn dataframe_set_index_drop_false_preserves_source_column() {
        let df = DataFrame::from_dict(
            &["id", "v"],
            vec![
                ("id", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let out = df.set_index("id", false).unwrap();
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from(10_i64), IndexLabel::from(20_i64)]
        );
        let names = out.column_names().into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(names, vec!["id".to_owned(), "v".to_owned()]);
        assert_eq!(
            out.column("id").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
    }

    #[test]
    fn dataframe_set_index_rejects_missing_or_unsupported_labels() {
        let df = DataFrame::from_dict(
            &["id", "v"],
            vec![
                ("id", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();
        let err = df.set_index("missing", true).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("column 'missing' not found"))
        );

        let df_float = DataFrame::from_dict(
            &["id", "v"],
            vec![
                ("id", vec![Scalar::Float64(1.5), Scalar::Float64(2.5)]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();
        let err = df_float.set_index("id", true).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("Int64/Utf8"))
        );

        let df_null = DataFrame::from_dict(
            &["id", "v"],
            vec![
                ("id", vec![Scalar::Null(NullKind::Null), Scalar::Int64(2)]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();
        let err = df_null.set_index("id", true).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("missing label values"))
        );
    }

    #[test]
    fn dataframe_reset_index_drop_true_replaces_with_range_index() {
        let df = DataFrame::from_dict_with_index(
            vec![("v", vec![Scalar::Int64(3), Scalar::Int64(4)])],
            vec![IndexLabel::from("r1"), IndexLabel::from("r2")],
        )
        .unwrap();

        let out = df.reset_index(true).unwrap();
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from(0_i64), IndexLabel::from(1_i64)]
        );
        let names = out.column_names().into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(names, vec!["v".to_owned()]);
        assert_eq!(
            out.column("v").unwrap().values(),
            &[Scalar::Int64(3), Scalar::Int64(4)]
        );
    }

    #[test]
    fn dataframe_reset_index_drop_false_inserts_index_column_first() {
        let df = DataFrame::from_dict_with_index(
            vec![("v", vec![Scalar::Int64(3), Scalar::Int64(4)])],
            vec![IndexLabel::from("r1"), IndexLabel::from("r2")],
        )
        .unwrap();

        let out = df.reset_index(false).unwrap();
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from(0_i64), IndexLabel::from(1_i64)]
        );
        let names = out.column_names().into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(names, vec!["index".to_owned(), "v".to_owned()]);
        assert_eq!(
            out.column("index").unwrap().values(),
            &[Scalar::Utf8("r1".to_owned()), Scalar::Utf8("r2".to_owned())]
        );
    }

    #[test]
    fn dataframe_reset_index_drop_false_uses_level_0_and_rejects_name_collision() {
        let df = DataFrame::from_dict_with_index(
            vec![
                (
                    "index",
                    vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())],
                ),
                ("v", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
            vec![IndexLabel::from(10_i64), IndexLabel::from(20_i64)],
        )
        .unwrap();

        let out = df.reset_index(false).unwrap();
        let names = out.column_names().into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["level_0".to_owned(), "index".to_owned(), "v".to_owned()]
        );
        assert_eq!(
            out.column("level_0").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );

        let collision = DataFrame::from_dict_with_index(
            vec![
                ("index", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("level_0", vec![Scalar::Int64(3), Scalar::Int64(4)]),
                ("v", vec![Scalar::Int64(5), Scalar::Int64(6)]),
            ],
            vec![IndexLabel::from(0_i64), IndexLabel::from(1_i64)],
        )
        .unwrap();
        let err = collision.reset_index(false).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("both 'index' and 'level_0'"))
        );
    }

    #[test]
    fn dataframe_reset_index_rejects_mixed_index_label_types() {
        let df = DataFrame::from_dict_with_index(
            vec![("v", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![IndexLabel::from("row-1"), IndexLabel::from(2_i64)],
        )
        .unwrap();

        let err = df.reset_index(false).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("mixed Int64/Utf8"))
        );
    }

    #[test]
    fn dataframe_sort_index_ascending_and_descending() {
        let df = DataFrame::from_dict_with_index(
            vec![(
                "v",
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )],
            vec!["b".into(), "a".into(), "c".into()],
        )
        .unwrap();

        let asc = df.sort_index(true).unwrap();
        assert_eq!(
            asc.index().labels(),
            &[
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c")
            ]
        );
        assert_eq!(
            asc.column("v").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(10), Scalar::Int64(30)]
        );

        let desc = df.sort_index(false).unwrap();
        assert_eq!(
            desc.index().labels(),
            &[
                IndexLabel::from("c"),
                IndexLabel::from("b"),
                IndexLabel::from("a")
            ]
        );
        assert_eq!(
            desc.column("v").unwrap().values(),
            &[Scalar::Int64(30), Scalar::Int64(10), Scalar::Int64(20)]
        );
    }

    #[test]
    fn dataframe_sort_values_numeric_keeps_na_last_and_stable_ties() {
        let df = DataFrame::from_dict_with_index(
            vec![
                (
                    "score",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "tag",
                    vec![
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("null".to_owned()),
                        Scalar::Utf8("y".to_owned()),
                        Scalar::Utf8("z".to_owned()),
                    ],
                ),
            ],
            vec!["r1".into(), "r2".into(), "r3".into(), "r4".into()],
        )
        .unwrap();

        let asc = df.sort_values("score", true).unwrap();
        assert_eq!(
            asc.index().labels(),
            &[
                IndexLabel::from("r3"),
                IndexLabel::from("r1"),
                IndexLabel::from("r4"),
                IndexLabel::from("r2")
            ]
        );
        // Stable ordering for equal key value (2): r1 before r4.
        assert_eq!(
            asc.column("tag").unwrap().values(),
            &[
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("z".to_owned()),
                Scalar::Utf8("null".to_owned())
            ]
        );

        let desc = df.sort_values("score", false).unwrap();
        assert_eq!(
            desc.index().labels(),
            &[
                IndexLabel::from("r1"),
                IndexLabel::from("r4"),
                IndexLabel::from("r3"),
                IndexLabel::from("r2")
            ]
        );
    }

    #[test]
    fn dataframe_sort_values_missing_column_is_rejected() {
        let df = DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(1)])]).unwrap();
        let err = df.sort_values("missing", true).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("column 'missing' not found"))
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
    fn dataframe_loc_with_columns_selects_row_and_column_subsets() {
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
                (
                    "c",
                    vec![
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("y".to_owned()),
                        Scalar::Utf8("z".to_owned()),
                    ],
                ),
            ],
            vec!["r1".into(), "r2".into(), "r3".into()],
        )
        .unwrap();

        let selected = df
            .loc_with_columns(
                &["r3".into(), "r1".into()],
                Some(&["c".to_owned(), "a".to_owned()]),
            )
            .unwrap();

        assert_eq!(
            selected.index().labels(),
            &[IndexLabel::from("r3"), IndexLabel::from("r1")]
        );
        assert_eq!(
            selected.column("a").unwrap().values(),
            &[Scalar::Int64(30), Scalar::Int64(10)]
        );
        assert_eq!(
            selected.column("c").unwrap().values(),
            &[Scalar::Utf8("z".to_owned()), Scalar::Utf8("x".to_owned())]
        );
        assert!(selected.column("b").is_none());
    }

    #[test]
    fn dataframe_loc_with_columns_missing_column_is_rejected() {
        let df = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec!["x".into(), "y".into()],
        )
        .unwrap();

        let err = df
            .loc_with_columns(&["x".into()], Some(&["missing".to_owned()]))
            .unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("column 'missing' not found"))
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
    fn dataframe_iloc_negative_positions_resolve_from_end() {
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

        let selected = df.iloc(&[-1, -3, 0]).unwrap();
        assert_eq!(
            selected.index().labels(),
            &[
                IndexLabel::from(13_i64),
                IndexLabel::from(11_i64),
                IndexLabel::from(10_i64)
            ]
        );
        assert_eq!(
            selected.column("a").unwrap().values(),
            &[
                Scalar::Int64(1300),
                Scalar::Int64(1100),
                Scalar::Int64(1000)
            ]
        );
        assert_eq!(
            selected.column("b").unwrap().values(),
            &[
                Scalar::Int64(2300),
                Scalar::Int64(2100),
                Scalar::Int64(2000)
            ]
        );
    }

    #[test]
    fn dataframe_iloc_negative_out_of_bounds_is_rejected() {
        let df = DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(10)])]).unwrap();
        let err = df.iloc(&[-2]).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("out of bounds"))
        );
    }

    #[test]
    fn dataframe_iloc_with_columns_selects_row_and_column_subsets() {
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
                (
                    "c",
                    vec![
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("y".to_owned()),
                        Scalar::Utf8("z".to_owned()),
                    ],
                ),
            ],
            vec!["r1".into(), "r2".into(), "r3".into()],
        )
        .unwrap();

        let selected = df
            .iloc_with_columns(&[-1, 0], Some(&["b".to_owned(), "a".to_owned()]))
            .unwrap();
        assert_eq!(
            selected.index().labels(),
            &[IndexLabel::from("r3"), IndexLabel::from("r1")]
        );
        assert_eq!(
            selected.column("a").unwrap().values(),
            &[Scalar::Int64(30), Scalar::Int64(10)]
        );
        assert_eq!(
            selected.column("b").unwrap().values(),
            &[Scalar::Int64(300), Scalar::Int64(100)]
        );
        assert!(selected.column("c").is_none());
    }

    #[test]
    fn dataframe_iloc_with_columns_duplicate_selector_is_rejected() {
        let df = DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(10)])]).unwrap();
        let err = df
            .iloc_with_columns(&[0], Some(&["a".to_owned(), "a".to_owned()]))
            .unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("duplicate column selector"))
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
    fn dataframe_astype_column_casts_selected_column_and_preserves_order() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                (
                    "b",
                    vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())],
                ),
            ],
        )
        .unwrap();

        let casted = df.astype_column("a", DType::Float64).unwrap();
        let names = casted
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(casted.column("a").unwrap().dtype(), DType::Float64);
        assert_eq!(
            casted.column("a").unwrap().values(),
            &[Scalar::Float64(1.0), Scalar::Float64(2.0)]
        );
        assert_eq!(
            casted.column("b").unwrap().values(),
            &[Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())]
        );
    }

    #[test]
    fn dataframe_astype_column_rejects_missing_or_invalid_cast() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![(
                "a",
                vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())],
            )],
        )
        .unwrap();

        let err = df.astype_column("missing", DType::Int64).unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("column 'missing' not found"))
        );

        let err = df.astype_column("a", DType::Int64).unwrap_err();
        assert!(matches!(err, FrameError::Column(_)));
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
