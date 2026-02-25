#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use fp_columnar::{ArithmeticOp, Column, ColumnError, ComparisonOp};
use fp_index::{
    AlignMode, DuplicateKeep, Index, IndexError, IndexLabel, align, align_union,
    validate_alignment_plan,
};
use fp_runtime::{DecisionAction, EvidenceLedger, RuntimePolicy};
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

fn scalar_to_value_counts_index_label(value: &Scalar) -> IndexLabel {
    match value {
        Scalar::Int64(v) => IndexLabel::Int64(*v),
        Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
        // Match pandas string representation for boolean index values.
        Scalar::Bool(v) => IndexLabel::Utf8(if *v {
            "True".to_owned()
        } else {
            "False".to_owned()
        }),
        // Keep float labels as textual index labels for current IndexLabel surface.
        Scalar::Float64(v) => IndexLabel::Utf8(format!("{v:?}")),
        Scalar::Null(_) => IndexLabel::Utf8("<null>".to_owned()),
    }
}

fn index_label_to_scalar(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(v) => Scalar::Int64(*v),
        IndexLabel::Utf8(v) => Scalar::Utf8(v.clone()),
    }
}

fn index_label_to_utf8_scalar(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(v) => Scalar::Utf8(v.to_string()),
        IndexLabel::Utf8(v) => Scalar::Utf8(v.clone()),
    }
}

fn index_position_groups(index: &Index) -> BTreeMap<IndexLabel, Vec<usize>> {
    let mut groups: BTreeMap<IndexLabel, Vec<usize>> = BTreeMap::new();
    for (pos, label) in index.labels().iter().enumerate() {
        groups.entry(label.clone()).or_default().push(pos);
    }
    groups
}

/// Duplicate-aware outer alignment for Series arithmetic.
///
/// Mirrors pandas non-unique join behavior for `Series.align(join="outer")`
/// with `sort=False`:
/// - preserve left-then-unseen label order
/// - shared labels materialize cartesian matches (lc * rc rows)
/// - left-only and right-only labels keep their original multiplicity
fn align_union_duplicate_aware(
    left: &Index,
    right: &Index,
) -> (Index, Vec<Option<usize>>, Vec<Option<usize>>) {
    let left_groups = index_position_groups(left);
    let right_groups = index_position_groups(right);

    let mut seen = BTreeSet::new();
    let mut union_labels = Vec::new();
    for label in left.labels().iter().chain(right.labels().iter()) {
        if seen.insert(label.clone()) {
            union_labels.push(label.clone());
        }
    }

    let mut out_labels = Vec::new();
    let mut left_positions = Vec::new();
    let mut right_positions = Vec::new();

    for label in union_labels {
        let left_hits = left_groups.get(&label).map_or(&[][..], Vec::as_slice);
        let right_hits = right_groups.get(&label).map_or(&[][..], Vec::as_slice);

        if left_hits.is_empty() {
            for &rp in right_hits {
                out_labels.push(label.clone());
                left_positions.push(None);
                right_positions.push(Some(rp));
            }
            continue;
        }

        if right_hits.is_empty() {
            for &lp in left_hits {
                out_labels.push(label.clone());
                left_positions.push(Some(lp));
                right_positions.push(None);
            }
            continue;
        }

        for &lp in left_hits {
            for &rp in right_hits {
                out_labels.push(label.clone());
                left_positions.push(Some(lp));
                right_positions.push(Some(rp));
            }
        }
    }

    (Index::new(out_labels), left_positions, right_positions)
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
        let has_duplicate_labels = self.index.has_duplicates() || other.index.has_duplicates();
        let exact_duplicate_fast_path = has_duplicate_labels && self.index == other.index;

        let (union_index, left_positions, right_positions) = if exact_duplicate_fast_path {
            let positions = (0..self.len()).map(Some).collect::<Vec<_>>();
            (self.index.clone(), positions.clone(), positions)
        } else if has_duplicate_labels {
            align_union_duplicate_aware(&self.index, &other.index)
        } else {
            let plan = align_union(&self.index, &other.index);
            validate_alignment_plan(&plan)?;
            (plan.union_index, plan.left_positions, plan.right_positions)
        };

        let left = self.column.reindex_by_positions(&left_positions)?;
        let right = other.column.reindex_by_positions(&right_positions)?;

        let action = policy.decide_join_admission(union_index.len(), ledger);
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

        Self::new(out_name, union_index, column)
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

    fn reorder_by_positions(&self, positions: &[usize]) -> Result<Self, FrameError> {
        let mut out_labels = Vec::with_capacity(positions.len());
        let mut out_values = Vec::with_capacity(positions.len());

        for &position in positions {
            out_labels.push(self.index.labels()[position].clone());
            out_values.push(self.column.values()[position].clone());
        }

        Self::from_values(self.name.clone(), out_labels, out_values)
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

    /// Return a new Series sorted by index labels.
    ///
    /// Matches `s.sort_index(ascending=...)` for the current 1D IndexLabel
    /// model.
    pub fn sort_index(&self, ascending: bool) -> Result<Self, FrameError> {
        let mut order = self.index.argsort();
        if !ascending {
            order.reverse();
        }
        self.reorder_by_positions(&order)
    }

    /// Return a new Series sorted by values.
    ///
    /// Matches `s.sort_values(ascending=...)` with `na_position='last'`.
    pub fn sort_values(&self, ascending: bool) -> Result<Self, FrameError> {
        let mut order = (0..self.len()).collect::<Vec<_>>();
        order.sort_by(|&left_pos, &right_pos| {
            compare_scalars_with_na_last(
                &self.values()[left_pos],
                &self.values()[right_pos],
                ascending,
            )
        });
        self.reorder_by_positions(&order)
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

    /// Count unique non-missing values sorted by descending frequency.
    ///
    /// Matches `pd.Series.value_counts()` default behavior:
    /// - `dropna=True` (missing values excluded)
    /// - sort by count descending
    /// - stable tie ordering by first appearance
    pub fn value_counts(&self) -> Result<Self, FrameError> {
        let mut counts: Vec<(Scalar, usize)> = Vec::new();

        for value in self.column.values() {
            if value.is_missing() {
                continue;
            }

            if let Some((_, count)) = counts
                .iter_mut()
                .find(|(existing, _)| existing.semantic_eq(value))
            {
                *count += 1;
            } else {
                counts.push((value.clone(), 1));
            }
        }

        // Stable descending sort keeps first-seen ordering for tied counts.
        counts.sort_by(|(_, left_count), (_, right_count)| right_count.cmp(left_count));

        let mut labels = Vec::with_capacity(counts.len());
        let mut values = Vec::with_capacity(counts.len());
        for (value, count) in counts {
            labels.push(scalar_to_value_counts_index_label(&value));
            values.push(Scalar::Int64(i64::try_from(count).unwrap_or(i64::MAX)));
        }

        Self::from_values("count", labels, values)
    }

    /// Return unique non-null values in first-seen order.
    ///
    /// Matches `pd.Series.unique()`.
    #[must_use]
    pub fn unique(&self) -> Vec<Scalar> {
        let mut seen = Vec::<Scalar>::new();
        for value in self.column.values() {
            if value.is_missing() {
                continue;
            }
            if !seen.iter().any(|existing| existing.semantic_eq(value)) {
                seen.push(value.clone());
            }
        }
        seen
    }

    /// Count of unique non-null values.
    ///
    /// Matches `pd.Series.nunique()`.
    #[must_use]
    pub fn nunique(&self) -> usize {
        self.unique().len()
    }

    /// Map values using a dict-like mapping. Unmapped values become NaN.
    ///
    /// Matches `pd.Series.map(dict)`.
    pub fn map(&self, mapping: &[(Scalar, Scalar)]) -> Result<Self, FrameError> {
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            let mapped = mapping
                .iter()
                .find(|(k, _)| k.semantic_eq(val))
                .map(|(_, v)| v.clone());
            out.push(mapped.unwrap_or(Scalar::Null(NullKind::NaN)));
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Replace specific values with substitutions.
    ///
    /// Matches `pd.Series.replace(to_replace, value)` for scalar pairs.
    pub fn replace(&self, replacements: &[(Scalar, Scalar)]) -> Result<Self, FrameError> {
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            let replaced = replacements
                .iter()
                .find(|(old, _)| old.semantic_eq(val))
                .map(|(_, new)| new.clone());
            out.push(replaced.unwrap_or_else(|| val.clone()));
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Compute the q-th quantile (0.0 to 1.0) using linear interpolation.
    ///
    /// Matches `pd.Series.quantile(q)`.
    pub fn quantile(&self, q: f64) -> Result<Scalar, FrameError> {
        if !(0.0..=1.0).contains(&q) {
            return Err(FrameError::CompatibilityRejected(format!(
                "quantile must be between 0 and 1, got {q}"
            )));
        }
        let mut nums: Vec<f64> = Vec::new();
        for val in self.column.values() {
            if !val.is_missing() {
                nums.push(val.to_f64().map_err(ColumnError::from)?);
            }
        }
        if nums.is_empty() {
            return Ok(Scalar::Float64(f64::NAN));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Ok(Scalar::Float64(Self::percentile_linear_series(&nums, q)))
    }

    fn percentile_linear_series(sorted: &[f64], q: f64) -> f64 {
        if sorted.len() == 1 {
            return sorted[0];
        }
        let pos = q * (sorted.len() - 1) as f64;
        let lower = pos.floor() as usize;
        let upper = pos.ceil() as usize;
        if lower == upper {
            sorted[lower]
        } else {
            let frac = pos - lower as f64;
            sorted[lower] * (1.0 - frac) + sorted[upper] * frac
        }
    }

    /// Cast values to a target dtype.
    ///
    /// Matches `series.astype(dtype)` for scalar dtypes.
    pub fn astype(&self, dtype: DType) -> Result<Self, FrameError> {
        let column = Column::new(dtype, self.values().to_vec())?;
        Self::new(self.name.clone(), self.index.clone(), column)
    }

    /// Clip values to `[lower, upper]`.
    ///
    /// Matches `pd.Series.clip(lower, upper)`. NaN values pass through unchanged.
    pub fn clip(&self, lower: Option<f64>, upper: Option<f64>) -> Result<Self, FrameError> {
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(val.clone());
                continue;
            }
            let v = val.to_f64().map_err(ColumnError::from)?;
            let mut clamped = v;
            if let Some(lo) = lower
                && clamped < lo
            {
                clamped = lo;
            }
            if let Some(hi) = upper
                && clamped > hi
            {
                clamped = hi;
            }
            out.push(Scalar::Float64(clamped));
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Absolute value of each element.
    ///
    /// Matches `pd.Series.abs()`. NaN values pass through.
    pub fn abs(&self) -> Result<Self, FrameError> {
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(val.clone());
            } else {
                let v = val.to_f64().map_err(ColumnError::from)?;
                out.push(Scalar::Float64(v.abs()));
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Round each element to `decimals` decimal places.
    ///
    /// Matches `pd.Series.round(decimals)`. NaN values pass through.
    pub fn round(&self, decimals: i32) -> Result<Self, FrameError> {
        let factor = 10.0_f64.powi(decimals);
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(val.clone());
            } else {
                let v = val.to_f64().map_err(ColumnError::from)?;
                out.push(Scalar::Float64((v * factor).round() / factor));
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    // --- Descriptive Statistics ---

    #[must_use]
    fn scalar_truthy(value: &Scalar) -> bool {
        match value {
            Scalar::Null(_) => false,
            Scalar::Bool(flag) => *flag,
            Scalar::Int64(v) => *v != 0,
            Scalar::Float64(v) => !v.is_nan() && *v != 0.0,
            Scalar::Utf8(v) => !v.is_empty(),
        }
    }

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

    /// Whether any non-missing value is truthy.
    ///
    /// Matches `pd.Series.any()` with default `skipna=True`.
    pub fn any(&self) -> Result<bool, FrameError> {
        for value in self.column.values() {
            if value.is_missing() {
                continue;
            }
            if Self::scalar_truthy(value) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether all non-missing values are truthy.
    ///
    /// Matches `pd.Series.all()` with default `skipna=True`.
    pub fn all(&self) -> Result<bool, FrameError> {
        for value in self.column.values() {
            if value.is_missing() {
                continue;
            }
            if !Self::scalar_truthy(value) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // --- Shift / Diff / Cumulative ---

    /// Shift values by `periods` positions, filling with NaN.
    ///
    /// Matches `pd.Series.shift(periods)`. Positive shifts move values
    /// downward (earlier positions become NaN); negative shifts move
    /// values upward (later positions become NaN).
    pub fn shift(&self, periods: i64) -> Result<Self, FrameError> {
        let n = self.len();
        let missing = Scalar::Null(NullKind::NaN);
        let vals = self.column.values();
        let mut out = Vec::with_capacity(n);

        if periods >= 0 {
            let p = periods as usize;
            for _ in 0..p.min(n) {
                out.push(missing.clone());
            }
            if p < n {
                out.extend_from_slice(&vals[..n - p]);
            }
        } else {
            let p = periods.unsigned_abs() as usize;
            if p < n {
                out.extend_from_slice(&vals[p..]);
            }
            for _ in 0..p.min(n) {
                out.push(missing.clone());
            }
        }

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Element-wise difference with the previous element.
    ///
    /// Matches `pd.Series.diff(periods)`. Computes `value[i] - value[i - periods]`.
    /// Produces NaN for positions without a valid predecessor and for nulls.
    pub fn diff(&self, periods: i64) -> Result<Self, FrameError> {
        let n = self.len();
        let vals = self.column.values();
        let mut out = Vec::with_capacity(n);

        for i in 0..n {
            let prev_idx = if periods >= 0 {
                i.checked_sub(periods as usize)
            } else {
                let p = periods.unsigned_abs() as usize;
                let j = i + p;
                if j < n { Some(j) } else { None }
            };

            let result = match prev_idx {
                Some(j) if !vals[i].is_missing() && !vals[j].is_missing() => {
                    match (vals[i].to_f64(), vals[j].to_f64()) {
                        (Ok(a), Ok(b)) => Scalar::Float64(a - b),
                        _ => Scalar::Null(NullKind::NaN),
                    }
                }
                _ => Scalar::Null(NullKind::NaN),
            };
            out.push(result);
        }

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Cumulative sum, skipping NaN.
    ///
    /// Matches `pd.Series.cumsum(skipna=True)`.
    pub fn cumsum(&self) -> Result<Self, FrameError> {
        let mut acc = 0.0_f64;
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                acc += val.to_f64().map_err(ColumnError::from)?;
                out.push(Scalar::Float64(acc));
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Cumulative product, skipping NaN.
    ///
    /// Matches `pd.Series.cumprod(skipna=True)`.
    pub fn cumprod(&self) -> Result<Self, FrameError> {
        let mut acc = 1.0_f64;
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                acc *= val.to_f64().map_err(ColumnError::from)?;
                out.push(Scalar::Float64(acc));
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Cumulative minimum, skipping NaN.
    ///
    /// Matches `pd.Series.cummin(skipna=True)`.
    pub fn cummin(&self) -> Result<Self, FrameError> {
        let mut acc = f64::INFINITY;
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                let v = val.to_f64().map_err(ColumnError::from)?;
                if v < acc {
                    acc = v;
                }
                out.push(Scalar::Float64(acc));
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Cumulative maximum, skipping NaN.
    ///
    /// Matches `pd.Series.cummax(skipna=True)`.
    pub fn cummax(&self) -> Result<Self, FrameError> {
        let mut acc = f64::NEG_INFINITY;
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                let v = val.to_f64().map_err(ColumnError::from)?;
                if v > acc {
                    acc = v;
                }
                out.push(Scalar::Float64(acc));
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Keep values where `cond` is True; replace others with `other`.
    ///
    /// Matches `series.where(cond, other)`. If `other` is `None`, replaced
    /// values become `NaN`. The condition Series is aligned to `self` via
    /// outer-index union before masking.
    pub fn where_cond(
        &self,
        cond: &Self,
        other: Option<&Scalar>,
    ) -> Result<Self, FrameError> {
        let plan = align_union(&self.index, &cond.index);
        validate_alignment_plan(&plan)?;

        let aligned_data = self.column.reindex_by_positions(&plan.left_positions)?;
        let aligned_cond = cond.column.reindex_by_positions(&plan.right_positions)?;

        let fill = other.cloned().unwrap_or(Scalar::Null(NullKind::NaN));

        let values: Vec<Scalar> = aligned_data
            .values()
            .iter()
            .zip(aligned_cond.values())
            .map(|(val, c)| match c {
                Scalar::Bool(true) => val.clone(),
                Scalar::Bool(false) => fill.clone(),
                Scalar::Null(_) => Scalar::Null(NullKind::NaN),
                _ => fill.clone(),
            })
            .collect();

        Self::from_values(
            self.name.clone(),
            plan.union_index.labels().to_vec(),
            values,
        )
    }

    /// Replace values where `cond` is True with `other`.
    ///
    /// Matches `series.mask(cond, other)`. This is the inverse of `where`:
    /// values are replaced where the condition IS True, not where it is False.
    pub fn mask(
        &self,
        cond: &Self,
        other: Option<&Scalar>,
    ) -> Result<Self, FrameError> {
        let plan = align_union(&self.index, &cond.index);
        validate_alignment_plan(&plan)?;

        let aligned_data = self.column.reindex_by_positions(&plan.left_positions)?;
        let aligned_cond = cond.column.reindex_by_positions(&plan.right_positions)?;

        let fill = other.cloned().unwrap_or(Scalar::Null(NullKind::NaN));

        let values: Vec<Scalar> = aligned_data
            .values()
            .iter()
            .zip(aligned_cond.values())
            .map(|(val, c)| match c {
                Scalar::Bool(true) => fill.clone(),
                Scalar::Bool(false) => val.clone(),
                Scalar::Null(_) => Scalar::Null(NullKind::NaN),
                _ => val.clone(),
            })
            .collect();

        Self::from_values(
            self.name.clone(),
            plan.union_index.labels().to_vec(),
            values,
        )
    }

    /// Test whether each element is contained in a set of values.
    ///
    /// Matches `series.isin(values)`. Returns a boolean Series with the same
    /// index. Null elements produce `false` (matching pandas behavior).
    pub fn isin(&self, test_values: &[Scalar]) -> Result<Self, FrameError> {
        let values: Vec<Scalar> = self
            .column
            .values()
            .iter()
            .map(|val| {
                if val.is_missing() {
                    // pandas: NaN.isin([NaN]) returns True only when NaN is in values
                    let has_nan = test_values.iter().any(|tv| tv.is_missing());
                    Scalar::Bool(has_nan)
                } else {
                    Scalar::Bool(test_values.iter().any(|tv| {
                        if tv.is_missing() {
                            return false;
                        }
                        // Compare numerically for Int64/Float64 cross-type
                        match (val, tv) {
                            (Scalar::Int64(a), Scalar::Float64(b)) => {
                                (*a as f64) == *b
                            }
                            (Scalar::Float64(a), Scalar::Int64(b)) => {
                                *a == (*b as f64)
                            }
                            _ => val == tv,
                        }
                    }))
                }
            })
            .collect();

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), values)
    }

    /// Test whether each element falls within a range.
    ///
    /// Matches `series.between(left, right, inclusive='both')`. Returns a
    /// boolean Series. The `inclusive` parameter controls boundary behavior:
    /// - `"both"`: `left <= x <= right`
    /// - `"neither"`: `left < x < right`
    /// - `"left"`: `left <= x < right`
    /// - `"right"`: `left < x <= right`
    ///
    /// Null elements produce `false`.
    pub fn between(
        &self,
        left: &Scalar,
        right: &Scalar,
        inclusive: &str,
    ) -> Result<Self, FrameError> {
        let left_f = left.to_f64().map_err(ColumnError::from)?;
        let right_f = right.to_f64().map_err(ColumnError::from)?;

        let values: Vec<Scalar> = self
            .column
            .values()
            .iter()
            .map(|val| {
                if val.is_missing() {
                    return Scalar::Bool(false);
                }
                match val.to_f64() {
                    Ok(v) => {
                        let result = match inclusive {
                            "both" => v >= left_f && v <= right_f,
                            "neither" => v > left_f && v < right_f,
                            "left" => v >= left_f && v < right_f,
                            "right" => v > left_f && v <= right_f,
                            _ => v >= left_f && v <= right_f, // default to "both"
                        };
                        Scalar::Bool(result)
                    }
                    Err(_) => Scalar::Bool(false),
                }
            })
            .collect();

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), values)
    }

    /// Return the label of the minimum value.
    ///
    /// Matches `series.idxmin()`. Skips missing values. Returns an error
    /// if the series is empty or all-null.
    pub fn idxmin(&self) -> Result<IndexLabel, FrameError> {
        let mut best_idx: Option<usize> = None;
        let mut best_val = f64::INFINITY;
        for (i, val) in self.column.values().iter().enumerate() {
            if val.is_missing() {
                continue;
            }
            let v = val.to_f64().map_err(ColumnError::from)?;
            if v < best_val {
                best_val = v;
                best_idx = Some(i);
            }
        }
        best_idx
            .map(|i| self.index.labels()[i].clone())
            .ok_or_else(|| {
                FrameError::CompatibilityRejected("idxmin of empty or all-null series".to_owned())
            })
    }

    /// Return the label of the maximum value.
    ///
    /// Matches `series.idxmax()`. Skips missing values.
    pub fn idxmax(&self) -> Result<IndexLabel, FrameError> {
        let mut best_idx: Option<usize> = None;
        let mut best_val = f64::NEG_INFINITY;
        for (i, val) in self.column.values().iter().enumerate() {
            if val.is_missing() {
                continue;
            }
            let v = val.to_f64().map_err(ColumnError::from)?;
            if v > best_val {
                best_val = v;
                best_idx = Some(i);
            }
        }
        best_idx
            .map(|i| self.index.labels()[i].clone())
            .ok_or_else(|| {
                FrameError::CompatibilityRejected("idxmax of empty or all-null series".to_owned())
            })
    }

    /// Return the `n` largest values as a new Series, sorted descending.
    ///
    /// Matches `series.nlargest(n)`. Missing values are excluded.
    pub fn nlargest(&self, n: usize) -> Result<Self, FrameError> {
        let mut indexed: Vec<(usize, f64)> = self
            .column
            .values()
            .iter()
            .enumerate()
            .filter_map(|(i, val)| {
                if val.is_missing() {
                    None
                } else {
                    val.to_f64().ok().map(|v| (i, v))
                }
            })
            .collect();

        // Sort descending by value, stable by position for ties
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        indexed.truncate(n);

        let labels: Vec<IndexLabel> = indexed
            .iter()
            .map(|(i, _)| self.index.labels()[*i].clone())
            .collect();
        let values: Vec<Scalar> = indexed
            .iter()
            .map(|(i, _)| self.column.values()[*i].clone())
            .collect();

        Self::from_values(self.name.clone(), labels, values)
    }

    /// Return the `n` smallest values as a new Series, sorted ascending.
    ///
    /// Matches `series.nsmallest(n)`. Missing values are excluded.
    pub fn nsmallest(&self, n: usize) -> Result<Self, FrameError> {
        let mut indexed: Vec<(usize, f64)> = self
            .column
            .values()
            .iter()
            .enumerate()
            .filter_map(|(i, val)| {
                if val.is_missing() {
                    None
                } else {
                    val.to_f64().ok().map(|v| (i, v))
                }
            })
            .collect();

        // Sort ascending by value, stable by position for ties
        indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        indexed.truncate(n);

        let labels: Vec<IndexLabel> = indexed
            .iter()
            .map(|(i, _)| self.index.labels()[*i].clone())
            .collect();
        let values: Vec<Scalar> = indexed
            .iter()
            .map(|(i, _)| self.column.values()[*i].clone())
            .collect();

        Self::from_values(self.name.clone(), labels, values)
    }

    /// Percentage change between current and prior element.
    ///
    /// Matches `series.pct_change(periods=1)`. Returns a float Series where
    /// the first `periods` elements are NaN. Missing values propagate as NaN.
    pub fn pct_change(&self, periods: usize) -> Result<Self, FrameError> {
        let len = self.len();
        let mut out = Vec::with_capacity(len);

        for i in 0..len {
            if i < periods {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            let current = &self.column.values()[i];
            let previous = &self.column.values()[i - periods];

            if current.is_missing() || previous.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }

            match (current.to_f64(), previous.to_f64()) {
                (Ok(cur), Ok(prev)) => {
                    if prev == 0.0 {
                        out.push(Scalar::Null(NullKind::NaN));
                    } else {
                        out.push(Scalar::Float64((cur - prev) / prev));
                    }
                }
                _ => out.push(Scalar::Null(NullKind::NaN)),
            }
        }

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Convert Series to a single-column DataFrame.
    ///
    /// Matches `series.to_frame(name=None)`. Uses the series name as the
    /// column name by default.
    pub fn to_frame(&self, name: Option<&str>) -> Result<DataFrame, FrameError> {
        let col_name = name.unwrap_or(&self.name).to_owned();
        let mut columns = BTreeMap::new();
        columns.insert(col_name.clone(), self.column.clone());
        DataFrame::new_with_column_order(self.index.clone(), columns, vec![col_name])
    }

    /// Convert Series to a vector of scalars.
    ///
    /// Matches `series.to_list()`.
    pub fn to_list(&self) -> Vec<Scalar> {
        self.column.values().to_vec()
    }

    /// Convert Series to a vector of (label, scalar) pairs.
    ///
    /// Matches `series.to_dict()`.
    pub fn to_dict(&self) -> Vec<(IndexLabel, Scalar)> {
        self.index
            .labels()
            .iter()
            .zip(self.column.values())
            .map(|(lbl, val)| (lbl.clone(), val.clone()))
            .collect()
    }

    /// Create a rolling window view of this Series.
    ///
    /// Matches `series.rolling(window, min_periods=None)`.
    pub fn rolling(&self, window: usize, min_periods: Option<usize>) -> Rolling<'_> {
        Rolling {
            series: self,
            window,
            min_periods: min_periods.unwrap_or(window),
        }
    }

    /// Create an expanding window view of this Series.
    ///
    /// Matches `series.expanding(min_periods=1)`.
    pub fn expanding(&self, min_periods: Option<usize>) -> Expanding<'_> {
        Expanding {
            series: self,
            min_periods: min_periods.unwrap_or(1),
        }
    }

    /// Rank values in the Series.
    ///
    /// `method`: "average" (default), "min", "max", "first", "dense"
    /// `ascending`: rank direction (default true = smallest gets rank 1)
    /// `na_option`: "keep" (NaN stays NaN), "top" (NaN gets lowest ranks), "bottom" (NaN gets highest ranks)
    pub fn rank(
        &self,
        method: &str,
        ascending: bool,
        na_option: &str,
    ) -> Result<Self, FrameError> {
        let vals = self.column().values();
        let len = vals.len();

        // Separate null and non-null indices
        let mut null_positions = Vec::new();
        let mut sortable: Vec<(usize, f64)> = Vec::new();

        for (i, v) in vals.iter().enumerate() {
            if v.is_missing() {
                null_positions.push(i);
            } else if let Ok(f) = v.to_f64() {
                sortable.push((i, f));
            } else {
                null_positions.push(i);
            }
        }

        // Sort by value; stable sort preserves original order for ties
        if ascending {
            sortable.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            sortable.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Assign ranks based on method
        let mut ranks = vec![f64::NAN; len];

        match method {
            "average" => {
                let mut i = 0;
                while i < sortable.len() {
                    let mut j = i + 1;
                    while j < sortable.len()
                        && (sortable[j].1 - sortable[i].1).abs() < f64::EPSILON
                    {
                        j += 1;
                    }
                    // Positions i..j have equal values; average rank
                    let avg: f64 = ((i + 1)..=(j)).map(|r| r as f64).sum::<f64>()
                        / (j - i) as f64;
                    for item in &sortable[i..j] {
                        ranks[item.0] = avg;
                    }
                    i = j;
                }
            }
            "min" => {
                let mut i = 0;
                while i < sortable.len() {
                    let mut j = i + 1;
                    while j < sortable.len()
                        && (sortable[j].1 - sortable[i].1).abs() < f64::EPSILON
                    {
                        j += 1;
                    }
                    let min_rank = (i + 1) as f64;
                    for item in &sortable[i..j] {
                        ranks[item.0] = min_rank;
                    }
                    i = j;
                }
            }
            "max" => {
                let mut i = 0;
                while i < sortable.len() {
                    let mut j = i + 1;
                    while j < sortable.len()
                        && (sortable[j].1 - sortable[i].1).abs() < f64::EPSILON
                    {
                        j += 1;
                    }
                    let max_rank = j as f64;
                    for item in &sortable[i..j] {
                        ranks[item.0] = max_rank;
                    }
                    i = j;
                }
            }
            "first" => {
                for (rank_idx, item) in sortable.iter().enumerate() {
                    ranks[item.0] = (rank_idx + 1) as f64;
                }
            }
            "dense" => {
                let mut dense_rank = 0u64;
                let mut i = 0;
                while i < sortable.len() {
                    dense_rank += 1;
                    let mut j = i + 1;
                    while j < sortable.len()
                        && (sortable[j].1 - sortable[i].1).abs() < f64::EPSILON
                    {
                        j += 1;
                    }
                    for item in &sortable[i..j] {
                        ranks[item.0] = dense_rank as f64;
                    }
                    i = j;
                }
            }
            _ => {
                return Err(FrameError::CompatibilityRejected(format!(
                    "rank method '{method}' not supported"
                )));
            }
        }

        // Handle na_option
        match na_option {
            "keep" => {
                // nulls stay as NaN — already set
            }
            "top" => {
                // NaN values get the lowest ranks (1, 2, ...)
                // Shift all existing ranks up by null count
                let null_count = null_positions.len();
                for r in &mut ranks {
                    if !r.is_nan() {
                        *r += null_count as f64;
                    }
                }
                for (i, &pos) in null_positions.iter().enumerate() {
                    ranks[pos] = (i + 1) as f64;
                }
            }
            "bottom" => {
                // NaN values get the highest ranks
                let non_null_count = sortable.len();
                for (i, &pos) in null_positions.iter().enumerate() {
                    ranks[pos] = (non_null_count + i + 1) as f64;
                }
            }
            _ => {
                return Err(FrameError::CompatibilityRejected(format!(
                    "na_option '{na_option}' not supported"
                )));
            }
        }

        let out: Vec<Scalar> = ranks
            .into_iter()
            .map(|r| {
                if r.is_nan() {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Float64(r)
                }
            })
            .collect();

        Self::from_values(self.name(), self.index().labels().to_vec(), out)
    }

    /// Compute the Pearson correlation with another Series.
    pub fn corr(&self, other: &Self) -> Result<f64, FrameError> {
        let (cov, var_x, var_y, _) = self.cov_components(other)?;
        let denom = (var_x * var_y).sqrt();
        if denom < f64::EPSILON {
            Ok(f64::NAN)
        } else {
            Ok(cov / denom)
        }
    }

    /// Compute the sample covariance with another Series.
    pub fn cov_with(&self, other: &Self) -> Result<f64, FrameError> {
        let (cov, _, _, _) = self.cov_components(other)?;
        Ok(cov)
    }

    /// Internal: compute covariance, var_x, var_y, count between two Series.
    fn cov_components(&self, other: &Self) -> Result<(f64, f64, f64, usize), FrameError> {
        let a_vals = self.column().values();
        let b_vals = other.column().values();
        let len = a_vals.len().min(b_vals.len());

        let mut sum_x = 0.0_f64;
        let mut sum_y = 0.0_f64;
        let mut sum_xy = 0.0_f64;
        let mut sum_x2 = 0.0_f64;
        let mut sum_y2 = 0.0_f64;
        let mut count = 0_usize;

        for i in 0..len {
            if let (Ok(x), Ok(y)) = (a_vals[i].to_f64(), b_vals[i].to_f64())
                && !x.is_nan()
                && !y.is_nan()
            {
                sum_x += x;
                sum_y += y;
                sum_xy += x * y;
                sum_x2 += x * x;
                sum_y2 += y * y;
                count += 1;
            }
        }

        if count < 2 {
            return Ok((f64::NAN, f64::NAN, f64::NAN, count));
        }

        let n = count as f64;
        let mean_x = sum_x / n;
        let mean_y = sum_y / n;
        let cov = (sum_xy - n * mean_x * mean_y) / (n - 1.0);
        let var_x = (sum_x2 - n * mean_x * mean_x) / (n - 1.0);
        let var_y = (sum_y2 - n * mean_y * mean_y) / (n - 1.0);

        Ok((cov, var_x, var_y, count))
    }

    /// Access string methods on a Utf8 Series (analogous to `pandas.Series.str`).
    pub fn str(&self) -> StringAccessor<'_> {
        StringAccessor { series: self }
    }

    /// Apply a closure element-wise to produce a new Series.
    pub fn apply_fn<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Scalar,
    {
        let new_vals: Vec<Scalar> = self.column().values().iter().map(func).collect();
        Self::from_values(self.name(), self.index().labels().to_vec(), new_vals)
    }

    /// Map values using a dictionary (HashMap).
    ///
    /// Values not found in the map become NaN (like pandas `Series.map(dict)`).
    pub fn map_values(
        &self,
        mapping: &std::collections::HashMap<String, Scalar>,
    ) -> Result<Self, FrameError> {
        let new_vals: Vec<Scalar> = self
            .column()
            .values()
            .iter()
            .map(|v| {
                let key = match v {
                    Scalar::Utf8(s) => s.clone(),
                    Scalar::Int64(i) => i.to_string(),
                    Scalar::Float64(f) => f.to_string(),
                    Scalar::Bool(b) => b.to_string(),
                    Scalar::Null(_) => return Scalar::Null(NullKind::NaN),
                };
                mapping
                    .get(&key)
                    .cloned()
                    .unwrap_or(Scalar::Null(NullKind::NaN))
            })
            .collect();
        Self::from_values(self.name(), self.index().labels().to_vec(), new_vals)
    }
}

/// Rolling window aggregation over a Series.
///
/// Created by `Series::rolling()`. Provides methods for computing
/// rolling statistics over a sliding window.
pub struct Rolling<'a> {
    series: &'a Series,
    window: usize,
    min_periods: usize,
}

impl Rolling<'_> {
    /// Helper: collect non-null f64 values from a window slice.
    fn window_values(vals: &[Scalar]) -> Vec<f64> {
        vals.iter()
            .filter_map(|v| {
                if v.is_missing() {
                    None
                } else {
                    v.to_f64().ok()
                }
            })
            .collect()
    }

    /// Apply a rolling aggregation function to produce a new Series.
    fn apply_rolling<F>(&self, agg: F, name: &str) -> Result<Series, FrameError>
    where
        F: Fn(&[f64]) -> f64,
    {
        let vals = self.series.column().values();
        let len = vals.len();
        let mut out = Vec::with_capacity(len);

        for i in 0..len {
            let start = (i + 1).saturating_sub(self.window);
            let window_slice = &vals[start..=i];
            let nums = Self::window_values(window_slice);

            if i + 1 < self.window || nums.len() < self.min_periods {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                out.push(Scalar::Float64(agg(&nums)));
            }
        }

        Series::from_values(
            name,
            self.series.index().labels().to_vec(),
            out,
        )
    }

    /// Rolling sum.
    pub fn sum(&self) -> Result<Series, FrameError> {
        self.apply_rolling(|nums| nums.iter().sum(), self.series.name())
    }

    /// Rolling mean.
    pub fn mean(&self) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| {
                if nums.is_empty() {
                    f64::NAN
                } else {
                    nums.iter().sum::<f64>() / nums.len() as f64
                }
            },
            self.series.name(),
        )
    }

    /// Rolling minimum.
    pub fn min(&self) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| nums.iter().copied().fold(f64::INFINITY, f64::min),
            self.series.name(),
        )
    }

    /// Rolling maximum.
    pub fn max(&self) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            self.series.name(),
        )
    }

    /// Rolling sample standard deviation (ddof=1).
    pub fn std(&self) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| {
                if nums.len() < 2 {
                    return f64::NAN;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                    / (nums.len() - 1) as f64;
                var.sqrt()
            },
            self.series.name(),
        )
    }

    /// Rolling count of non-null values.
    pub fn count(&self) -> Result<Series, FrameError> {
        let vals = self.series.column().values();
        let len = vals.len();
        let mut out = Vec::with_capacity(len);

        for i in 0..len {
            let start = (i + 1).saturating_sub(self.window);
            let window_slice = &vals[start..=i];
            let count = window_slice.iter().filter(|v| !v.is_missing()).count();

            if i + 1 < self.window {
                // Before full window, still emit count (pandas behavior)
                out.push(Scalar::Float64(count as f64));
            } else {
                out.push(Scalar::Float64(count as f64));
            }
        }

        Series::from_values(
            self.series.name(),
            self.series.index().labels().to_vec(),
            out,
        )
    }
}

/// Expanding window aggregation over a Series.
///
/// Created by `Series::expanding()`. All prior elements are included
/// in each window computation.
pub struct Expanding<'a> {
    series: &'a Series,
    min_periods: usize,
}

impl Expanding<'_> {
    /// Apply an expanding aggregation function.
    fn apply_expanding<F>(&self, agg: F, name: &str) -> Result<Series, FrameError>
    where
        F: Fn(&[f64]) -> f64,
    {
        let vals = self.series.column().values();
        let len = vals.len();
        let mut out = Vec::with_capacity(len);
        let mut nums = Vec::new();

        for val in vals {
            if !val.is_missing()
                && let Ok(v) = val.to_f64()
            {
                nums.push(v);
            }

            if nums.len() < self.min_periods {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                out.push(Scalar::Float64(agg(&nums)));
            }
        }

        Series::from_values(
            name,
            self.series.index().labels().to_vec(),
            out,
        )
    }

    /// Expanding sum.
    pub fn sum(&self) -> Result<Series, FrameError> {
        self.apply_expanding(|nums| nums.iter().sum(), self.series.name())
    }

    /// Expanding mean.
    pub fn mean(&self) -> Result<Series, FrameError> {
        self.apply_expanding(
            |nums| nums.iter().sum::<f64>() / nums.len() as f64,
            self.series.name(),
        )
    }

    /// Expanding minimum.
    pub fn min(&self) -> Result<Series, FrameError> {
        self.apply_expanding(
            |nums| nums.iter().copied().fold(f64::INFINITY, f64::min),
            self.series.name(),
        )
    }

    /// Expanding maximum.
    pub fn max(&self) -> Result<Series, FrameError> {
        self.apply_expanding(
            |nums| nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            self.series.name(),
        )
    }

    /// Expanding sample standard deviation (ddof=1).
    pub fn std(&self) -> Result<Series, FrameError> {
        self.apply_expanding(
            |nums| {
                if nums.len() < 2 {
                    return f64::NAN;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                    / (nums.len() - 1) as f64;
                var.sqrt()
            },
            self.series.name(),
        )
    }
}

/// String accessor for Series containing Utf8 data.
///
/// Created by `Series::str()`. Provides string manipulation methods
/// analogous to pandas `Series.str` namespace.
pub struct StringAccessor<'a> {
    series: &'a Series,
}

impl StringAccessor<'_> {
    /// Helper: apply a string transformation to each value.
    fn apply_str<F>(&self, func: F, name: &str) -> Result<Series, FrameError>
    where
        F: Fn(&str) -> Scalar,
    {
        let vals = self.series.column().values();
        let out: Vec<Scalar> = vals
            .iter()
            .map(|v| match v {
                Scalar::Utf8(s) => func(s),
                _ if v.is_missing() => Scalar::Null(NullKind::NaN),
                _ => v.clone(),
            })
            .collect();
        Series::from_values(name, self.series.index().labels().to_vec(), out)
    }

    /// Convert strings to lowercase.
    pub fn lower(&self) -> Result<Series, FrameError> {
        self.apply_str(|s| Scalar::Utf8(s.to_lowercase()), self.series.name())
    }

    /// Convert strings to uppercase.
    pub fn upper(&self) -> Result<Series, FrameError> {
        self.apply_str(|s| Scalar::Utf8(s.to_uppercase()), self.series.name())
    }

    /// Strip leading and trailing whitespace.
    pub fn strip(&self) -> Result<Series, FrameError> {
        self.apply_str(|s| Scalar::Utf8(s.trim().to_string()), self.series.name())
    }

    /// Strip leading whitespace.
    pub fn lstrip(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.trim_start().to_string()),
            self.series.name(),
        )
    }

    /// Strip trailing whitespace.
    pub fn rstrip(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.trim_end().to_string()),
            self.series.name(),
        )
    }

    /// Check whether each string contains a pattern.
    pub fn contains(&self, pat: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(s.contains(pat)),
            self.series.name(),
        )
    }

    /// Replace occurrences of a pattern with a replacement string.
    pub fn replace(&self, pat: &str, repl: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.replace(pat, repl)),
            self.series.name(),
        )
    }

    /// Check whether each string starts with a prefix.
    pub fn startswith(&self, pat: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(s.starts_with(pat)),
            self.series.name(),
        )
    }

    /// Check whether each string ends with a suffix.
    pub fn endswith(&self, pat: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(s.ends_with(pat)),
            self.series.name(),
        )
    }

    /// Get the length of each string.
    pub fn len(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Int64(s.len() as i64),
            self.series.name(),
        )
    }

    /// Slice each string from start to end.
    pub fn slice(&self, start: usize, end: Option<usize>) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let chars: Vec<char> = s.chars().collect();
                let stop = end.unwrap_or(chars.len()).min(chars.len());
                let begin = start.min(stop);
                Scalar::Utf8(chars[begin..stop].iter().collect())
            },
            self.series.name(),
        )
    }

    /// Split each string by a separator and return the n-th element.
    pub fn split_get(&self, pat: &str, n: usize) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let parts: Vec<&str> = s.split(pat).collect();
                if n < parts.len() {
                    Scalar::Utf8(parts[n].to_string())
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Capitalize the first character of each string.
    pub fn capitalize(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let mut chars = s.chars();
                match chars.next() {
                    None => Scalar::Utf8(String::new()),
                    Some(c) => {
                        let upper: String = c.to_uppercase().collect();
                        let rest: String = chars.collect();
                        Scalar::Utf8(format!("{upper}{rest}"))
                    }
                }
            },
            self.series.name(),
        )
    }

    /// Title case each string.
    pub fn title(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let mut result = String::with_capacity(s.len());
                let mut capitalize_next = true;
                for c in s.chars() {
                    if c.is_whitespace() || c == '_' || c == '-' {
                        result.push(c);
                        capitalize_next = true;
                    } else if capitalize_next {
                        for uc in c.to_uppercase() {
                            result.push(uc);
                        }
                        capitalize_next = false;
                    } else {
                        for lc in c.to_lowercase() {
                            result.push(lc);
                        }
                    }
                }
                Scalar::Utf8(result)
            },
            self.series.name(),
        )
    }

    /// Repeat each string n times.
    pub fn repeat(&self, n: usize) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.repeat(n)),
            self.series.name(),
        )
    }

    /// Pad strings to a minimum width with a fill character.
    pub fn pad(&self, width: usize, side: &str, fillchar: char) -> Result<Series, FrameError> {
        let side_owned = side.to_string();
        self.apply_str(
            |s| {
                if s.len() >= width {
                    return Scalar::Utf8(s.to_string());
                }
                let pad_len = width - s.len();
                let padding: String = std::iter::repeat_n(fillchar, pad_len).collect();
                match side_owned.as_str() {
                    "left" => Scalar::Utf8(format!("{padding}{s}")),
                    "right" => Scalar::Utf8(format!("{s}{padding}")),
                    "both" => {
                        let left = pad_len / 2;
                        let right = pad_len - left;
                        let lpad: String = std::iter::repeat_n(fillchar, left).collect();
                        let rpad: String = std::iter::repeat_n(fillchar, right).collect();
                        Scalar::Utf8(format!("{lpad}{s}{rpad}"))
                    }
                    _ => Scalar::Utf8(s.to_string()),
                }
            },
            self.series.name(),
        )
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
/// - Duplicate input index labels are supported only when every input index is
///   exactly identical (`join='outer'` fast path); duplicate-index reindexing is
///   compatibility-rejected.
fn concat_dataframes_axis1(
    frames: &[&DataFrame],
    join: ConcatJoin,
) -> Result<DataFrame, FrameError> {
    if frames.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new());
    }

    let target_index = match join {
        ConcatJoin::Outer => {
            let all_indexes_equal = frames
                .windows(2)
                .all(|window| window[0].index() == window[1].index());
            if all_indexes_equal {
                frames[0].index().clone()
            } else {
                // Match pandas union behavior for non-identical indexes:
                // deduplicate labels while preserving first-seen order.
                let mut seen = BTreeSet::new();
                let mut labels = Vec::new();
                for frame in frames {
                    for label in frame.index().labels() {
                        if seen.insert(label.clone()) {
                            labels.push(label.clone());
                        }
                    }
                }
                Index::new(labels)
            }
        }
        ConcatJoin::Inner => {
            let mut seen = BTreeSet::new();
            let mut labels = Vec::new();
            for label in frames[0].index().labels() {
                if seen.insert(label.clone()) {
                    labels.push(label.clone());
                }
            }
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
        let positions = if frame.index() == &target_index {
            (0..frame.len()).map(Some).collect::<Vec<_>>()
        } else if frame.index().has_duplicates() {
            return Err(FrameError::CompatibilityRejected(
                "concat(axis=1) cannot reindex duplicate index labels".to_owned(),
            ));
        } else {
            frame.index().get_indexer(&target_index)
        };

        for name in frame.column_names() {
            let column = frame
                .column(name)
                .expect("frame column listed in order must exist");
            if columns.contains_key(name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "duplicate column '{name}' in concat(axis=1) output"
                )));
            }

            columns.insert(name.clone(), column.reindex_by_positions(&positions)?);
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

#[derive(Debug, Clone, PartialEq)]
pub enum DataFrameColumnInput {
    Values(Vec<Scalar>),
    Scalar(Scalar),
}

impl From<Vec<Scalar>> for DataFrameColumnInput {
    fn from(values: Vec<Scalar>) -> Self {
        Self::Values(values)
    }
}

impl From<Scalar> for DataFrameColumnInput {
    fn from(value: Scalar) -> Self {
        Self::Scalar(value)
    }
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

    fn take_rows_by_positions(&self, positions: &[usize]) -> Result<Self, FrameError> {
        for &position in positions {
            if position >= self.len() {
                return Err(FrameError::CompatibilityRejected(format!(
                    "row position {position} out of bounds for length {}",
                    self.len()
                )));
            }
        }

        let labels = positions
            .iter()
            .map(|&position| self.index.labels()[position].clone())
            .collect::<Vec<_>>();
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

        Self::new_with_column_order(Index::new(labels), columns, self.column_order.clone())
    }

    fn rows_equal_on_subset(&self, left: usize, right: usize, subset: &[String]) -> bool {
        subset.iter().all(|name| {
            let column = self.columns.get(name).expect("selected column must exist");
            column.values()[left].semantic_eq(&column.values()[right])
        })
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

    /// Construct a DataFrame from mixed dict inputs (`Vec<Scalar>` and scalar).
    ///
    /// Scalar values are broadcast to the row count inferred from the first
    /// non-scalar column. If all inputs are scalar, an explicit index is
    /// required and this constructor rejects the operation.
    pub fn from_dict_mixed(
        column_order: &[&str],
        data: Vec<(&str, DataFrameColumnInput)>,
    ) -> Result<Self, FrameError> {
        if data.is_empty() {
            return Self::new(Index::new(Vec::new()), BTreeMap::new());
        }

        let row_count = data
            .iter()
            .find_map(|(_, input)| match input {
                DataFrameColumnInput::Values(values) => Some(values.len()),
                DataFrameColumnInput::Scalar(_) => None,
            })
            .ok_or_else(|| {
                FrameError::CompatibilityRejected(
                    "dataframe_from_dict with all-scalar values requires explicit index".to_owned(),
                )
            })?;

        let expanded = data
            .into_iter()
            .map(|(name, input)| {
                let values = match input {
                    DataFrameColumnInput::Values(values) => {
                        if values.len() != row_count {
                            return Err(FrameError::LengthMismatch {
                                index_len: row_count,
                                column_len: values.len(),
                            });
                        }
                        values
                    }
                    DataFrameColumnInput::Scalar(value) => vec![value; row_count],
                };
                Ok((name, values))
            })
            .collect::<Result<Vec<_>, FrameError>>()?;

        Self::from_dict(column_order, expanded)
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

    /// Construct a DataFrame from mixed dict inputs with an explicit index.
    ///
    /// Scalar values are broadcast to the explicit index length.
    pub fn from_dict_with_index_mixed(
        data: Vec<(&str, DataFrameColumnInput)>,
        index_labels: Vec<IndexLabel>,
    ) -> Result<Self, FrameError> {
        let row_count = index_labels.len();
        let expanded = data
            .into_iter()
            .map(|(name, input)| {
                let values = match input {
                    DataFrameColumnInput::Values(values) => {
                        if values.len() != row_count {
                            return Err(FrameError::LengthMismatch {
                                index_len: row_count,
                                column_len: values.len(),
                            });
                        }
                        values
                    }
                    DataFrameColumnInput::Scalar(value) => vec![value; row_count],
                };
                Ok((name, values))
            })
            .collect::<Result<Vec<_>, FrameError>>()?;

        Self::from_dict_with_index(expanded, index_labels)
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
            new_columns.insert(name.clone(), Column::new(col.dtype(), filtered_values)?);
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

    /// Return non-missing counts for each column.
    ///
    /// Matches `df.count()` default behavior (`axis=0`, `skipna=True`).
    pub fn count(&self) -> Result<Series, FrameError> {
        let labels = self
            .column_order
            .iter()
            .map(|name| IndexLabel::Utf8(name.clone()))
            .collect::<Vec<_>>();
        let values = self
            .column_order
            .iter()
            .map(|name| {
                let column = self
                    .columns
                    .get(name)
                    .expect("column name listed in order must exist");
                let count = i64::try_from(column.validity().count_valid()).unwrap_or(i64::MAX);
                Scalar::Int64(count)
            })
            .collect::<Vec<_>>();
        Series::from_values("count", labels, values)
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

    /// Return a boolean Series indicating duplicated rows.
    ///
    /// Matches `df.duplicated(subset=..., keep=...)`.
    pub fn duplicated(
        &self,
        subset: Option<&[String]>,
        keep: DuplicateKeep,
    ) -> Result<Series, FrameError> {
        let selected_columns = self.resolve_column_selector(subset)?;
        let row_count = self.len();
        let mut duplicated = vec![false; row_count];

        match keep {
            DuplicateKeep::First => {
                for (i, is_duplicated) in duplicated.iter_mut().enumerate() {
                    for j in 0..i {
                        if self.rows_equal_on_subset(i, j, &selected_columns) {
                            *is_duplicated = true;
                            break;
                        }
                    }
                }
            }
            DuplicateKeep::Last => {
                for (i, is_duplicated) in duplicated.iter_mut().enumerate() {
                    for j in (i + 1)..row_count {
                        if self.rows_equal_on_subset(i, j, &selected_columns) {
                            *is_duplicated = true;
                            break;
                        }
                    }
                }
            }
            DuplicateKeep::None => {
                for (i, is_duplicated) in duplicated.iter_mut().enumerate() {
                    for j in 0..row_count {
                        if i != j && self.rows_equal_on_subset(i, j, &selected_columns) {
                            *is_duplicated = true;
                            break;
                        }
                    }
                }
            }
        }

        Series::from_values(
            "__duplicated__",
            self.index.labels().to_vec(),
            duplicated.into_iter().map(Scalar::Bool).collect::<Vec<_>>(),
        )
    }

    /// Drop duplicated rows.
    ///
    /// Matches `df.drop_duplicates(subset=..., keep=..., ignore_index=...)`.
    pub fn drop_duplicates(
        &self,
        subset: Option<&[String]>,
        keep: DuplicateKeep,
        ignore_index: bool,
    ) -> Result<Self, FrameError> {
        let duplicated = self.duplicated(subset, keep)?;
        let keep_positions = duplicated
            .values()
            .iter()
            .enumerate()
            .filter_map(|(position, value)| match value {
                Scalar::Bool(false) => Some(position),
                _ => None,
            })
            .collect::<Vec<_>>();

        let out = self.take_rows_by_positions(&keep_positions)?;
        if ignore_index {
            out.reset_index(true)
        } else {
            Ok(out)
        }
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
            columns.insert(name.clone(), Column::new(col.dtype(), values)?);
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
            columns.insert(name.clone(), Column::new(col.dtype(), values)?);
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

        let index_column_name = self.reset_index_column_name()?;
        let index_values = if has_int && has_utf8 {
            self.index
                .labels()
                .iter()
                .map(index_label_to_utf8_scalar)
                .collect::<Vec<_>>()
        } else {
            self.index
                .labels()
                .iter()
                .map(index_label_to_scalar)
                .collect::<Vec<_>>()
        };

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
            columns.insert(name.clone(), Column::new(column.dtype(), values)?);
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
            columns.insert(name.clone(), Column::new(column.dtype(), values)?);
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

    /// Cast one or more columns to target dtypes.
    ///
    /// Matches mapping-style `df.astype({"a": dtype_a, "b": dtype_b})`.
    /// Mapping order is processed deterministically and duplicate selectors
    /// are rejected.
    pub fn astype_columns(&self, mapping: &[(&str, DType)]) -> Result<Self, FrameError> {
        let mut seen = BTreeSet::new();
        for &(name, _) in mapping {
            if !seen.insert(name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "duplicate astype selector: '{name}'"
                )));
            }

            if !self.columns.contains_key(name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "column '{name}' not found"
                )));
            }
        }

        let mut columns = self.columns.clone();
        for &(name, dtype) in mapping {
            let source = self.columns.get(name).ok_or_else(|| {
                FrameError::CompatibilityRejected(format!("column '{name}' not found"))
            })?;
            let casted = Column::new(dtype, source.values().to_vec())?;
            columns.insert(name.to_owned(), casted);
        }

        Self::new_with_column_order(self.index.clone(), columns, self.column_order.clone())
    }

    /// Cast a single column to a target dtype.
    ///
    /// Matches `df.astype({\"col\": dtype})` for single-column mapping.
    pub fn astype_column(&self, name: &str, dtype: DType) -> Result<Self, FrameError> {
        self.astype_columns(&[(name, dtype)])
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

            if columns.contains_key(*new_name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "duplicate column '{}' resulting from rename",
                    new_name
                )));
            }

            column_order.push((*new_name).to_owned());
            columns.insert((*new_name).to_owned(), col.clone());
        }
        Self::new_with_column_order(self.index.clone(), columns, column_order)
    }

    /// Summary statistics for numeric columns.
    ///
    /// Matches `df.describe()` — returns a DataFrame with index
    /// `[count, mean, std, min, 25%, 50%, 75%, max]` and one column per
    /// numeric input column.
    pub fn describe(&self) -> Result<Self, FrameError> {
        let stat_labels = vec![
            IndexLabel::Utf8("count".to_owned()),
            IndexLabel::Utf8("mean".to_owned()),
            IndexLabel::Utf8("std".to_owned()),
            IndexLabel::Utf8("min".to_owned()),
            IndexLabel::Utf8("25%".to_owned()),
            IndexLabel::Utf8("50%".to_owned()),
            IndexLabel::Utf8("75%".to_owned()),
            IndexLabel::Utf8("max".to_owned()),
        ];
        let out_index = Index::new(stat_labels);

        let mut out_columns = BTreeMap::new();
        let mut out_order = Vec::new();

        for name in &self.column_order {
            let col = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");

            if col.dtype() != DType::Int64 && col.dtype() != DType::Float64 {
                continue;
            }

            let mut nums: Vec<f64> = Vec::new();
            for val in col.values() {
                if !val.is_missing()
                    && let Ok(v) = val.to_f64()
                {
                    nums.push(v);
                }
            }

            let count = nums.len() as f64;
            let (mean, std, min, q25, q50, q75, max) = if nums.is_empty() {
                (
                    f64::NAN,
                    f64::NAN,
                    f64::NAN,
                    f64::NAN,
                    f64::NAN,
                    f64::NAN,
                    f64::NAN,
                )
            } else {
                let sum: f64 = nums.iter().sum();
                let mean_val = sum / count;
                let var = if nums.len() > 1 {
                    nums.iter().map(|x| (x - mean_val).powi(2)).sum::<f64>() / (count - 1.0)
                } else {
                    f64::NAN
                };
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let min_val = nums[0];
                let max_val = nums[nums.len() - 1];
                (
                    mean_val,
                    var.sqrt(),
                    min_val,
                    Self::percentile_linear(&nums, 0.25),
                    Self::percentile_linear(&nums, 0.50),
                    Self::percentile_linear(&nums, 0.75),
                    max_val,
                )
            };

            let stats = vec![
                Scalar::Float64(count),
                Scalar::Float64(mean),
                Scalar::Float64(std),
                Scalar::Float64(min),
                Scalar::Float64(q25),
                Scalar::Float64(q50),
                Scalar::Float64(q75),
                Scalar::Float64(max),
            ];

            out_columns.insert(name.clone(), Column::from_values(stats)?);
            out_order.push(name.clone());
        }

        Self::new_with_column_order(out_index, out_columns, out_order)
    }

    /// Compute quantile(s) for each numeric column.
    ///
    /// Matches `df.quantile(q)` for a single quantile value.
    /// Returns a Series indexed by column names.
    pub fn quantile(&self, q: f64) -> Result<Series, FrameError> {
        if !(0.0..=1.0).contains(&q) {
            return Err(FrameError::CompatibilityRejected(format!(
                "quantile must be between 0 and 1, got {q}"
            )));
        }
        let mut labels = Vec::new();
        let mut values = Vec::new();
        for name in &self.column_order {
            let col = self
                .columns
                .get(name)
                .expect("column name listed in order must exist");
            if col.dtype() != DType::Int64 && col.dtype() != DType::Float64 {
                continue;
            }
            let mut nums: Vec<f64> = Vec::new();
            for val in col.values() {
                if !val.is_missing()
                    && let Ok(v) = val.to_f64()
                {
                    nums.push(v);
                }
            }
            labels.push(IndexLabel::Utf8(name.clone()));
            if nums.is_empty() {
                values.push(Scalar::Float64(f64::NAN));
            } else {
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                values.push(Scalar::Float64(Self::percentile_linear(&nums, q)));
            }
        }
        Series::from_values(format!("{q}"), labels, values)
    }

    /// Apply an aggregation function along an axis.
    ///
    /// `axis=0` (default): apply function to each column, returning a Series
    /// indexed by column names.
    /// `axis=1`: apply function to each row, returning a Series indexed by
    /// the original DataFrame index.
    ///
    /// Matches `df.apply(func, axis=...)` for built-in aggregations.
    pub fn apply(&self, func: &str, axis: usize) -> Result<Series, FrameError> {
        if axis == 0 {
            // Column-wise: aggregate each column
            let mut labels = Vec::new();
            let mut values = Vec::new();
            for name in &self.column_order {
                let col = self
                    .columns
                    .get(name)
                    .expect("column name listed in order must exist");
                labels.push(IndexLabel::Utf8(name.clone()));
                let s = Series::new(name.clone(), self.index.clone(), col.clone())?;
                let agg = match func {
                    "sum" => s.sum()?,
                    "mean" => s.mean()?,
                    "min" => s.min()?,
                    "max" => s.max()?,
                    "std" => s.std()?,
                    "var" => s.var()?,
                    "median" => s.median()?,
                    "count" => Scalar::Int64(s.count() as i64),
                    other => {
                        return Err(FrameError::CompatibilityRejected(format!(
                            "unsupported apply function: '{other}'"
                        )));
                    }
                };
                values.push(agg);
            }
            Series::from_values(func, labels, values)
        } else if axis == 1 {
            // Row-wise: aggregate each row across columns
            let mut values = Vec::with_capacity(self.len());
            for row_idx in 0..self.len() {
                let row_vals: Vec<f64> = self
                    .column_order
                    .iter()
                    .filter_map(|name| {
                        let col = self.columns.get(name)?;
                        let val = &col.values()[row_idx];
                        if val.is_missing() {
                            None
                        } else {
                            val.to_f64().ok()
                        }
                    })
                    .collect();
                let sample_var = |vals: &[f64]| -> f64 {
                    if vals.len() < 2 {
                        return f64::NAN;
                    }
                    let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                    let sum_sq_diff = vals
                        .iter()
                        .map(|v| {
                            let diff = *v - mean;
                            diff * diff
                        })
                        .sum::<f64>();
                    sum_sq_diff / (vals.len() as f64 - 1.0)
                };
                let sample_median = |vals: &[f64]| -> f64 {
                    if vals.is_empty() {
                        return f64::NAN;
                    }
                    let mut sorted = vals.to_vec();
                    sorted.sort_unstable_by(|a, b| {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    let mid = sorted.len() / 2;
                    if sorted.len().is_multiple_of(2) {
                        (sorted[mid - 1] + sorted[mid]) / 2.0
                    } else {
                        sorted[mid]
                    }
                };
                let result = match func {
                    "sum" => Scalar::Float64(row_vals.iter().sum::<f64>()),
                    "mean" => Scalar::Float64(if row_vals.is_empty() {
                        f64::NAN
                    } else {
                        row_vals.iter().sum::<f64>() / row_vals.len() as f64
                    }),
                    "min" => Scalar::Float64(
                        row_vals
                            .iter()
                            .copied()
                            .reduce(f64::min)
                            .unwrap_or(f64::NAN),
                    ),
                    "max" => Scalar::Float64(
                        row_vals
                            .iter()
                            .copied()
                            .reduce(f64::max)
                            .unwrap_or(f64::NAN),
                    ),
                    "count" => Scalar::Int64(row_vals.len() as i64),
                    "median" => Scalar::Float64(sample_median(&row_vals)),
                    "var" => Scalar::Float64(sample_var(&row_vals)),
                    "std" => Scalar::Float64(sample_var(&row_vals).sqrt()),
                    other => {
                        return Err(FrameError::CompatibilityRejected(format!(
                            "unsupported apply function: '{other}'"
                        )));
                    }
                };
                values.push(result);
            }
            Series::from_values(func, self.index.labels().to_vec(), values)
        } else {
            Err(FrameError::CompatibilityRejected(format!(
                "axis must be 0 or 1, got {axis}"
            )))
        }
    }

    /// Transpose the DataFrame: rows become columns, columns become rows.
    ///
    /// Matches `df.T` / `df.transpose()`. Column names become the index
    /// of the result, and the original index labels become column names
    /// (converted to strings).
    pub fn transpose(&self) -> Result<Self, FrameError> {
        let n_rows = self.len();
        let n_cols = self.num_columns();

        // New index: original column names
        let new_index_labels: Vec<IndexLabel> = self
            .column_order
            .iter()
            .map(|name| IndexLabel::Utf8(name.clone()))
            .collect();
        let new_index = Index::new(new_index_labels);

        // New columns: one per original row, named by index label
        let mut new_columns = BTreeMap::new();
        let mut new_order = Vec::with_capacity(n_rows);

        for row_idx in 0..n_rows {
            let col_name = match &self.index.labels()[row_idx] {
                IndexLabel::Int64(v) => v.to_string(),
                IndexLabel::Utf8(v) => v.clone(),
            };

            let mut row_values = Vec::with_capacity(n_cols);
            for col_name_src in &self.column_order {
                let col = self
                    .columns
                    .get(col_name_src)
                    .expect("column name listed in order must exist");
                row_values.push(col.values()[row_idx].clone());
            }

            new_columns.insert(col_name.clone(), Column::from_values(row_values)?);
            new_order.push(col_name);
        }

        Self::new_with_column_order(new_index, new_columns, new_order)
    }

    /// Linear interpolation percentile (matching pandas default method).
    fn percentile_linear(sorted: &[f64], q: f64) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        if sorted.len() == 1 {
            return sorted[0];
        }
        let pos = q * (sorted.len() - 1) as f64;
        let lower = pos.floor() as usize;
        let upper = pos.ceil() as usize;
        if lower == upper {
            sorted[lower]
        } else {
            let frac = pos - lower as f64;
            sorted[lower] * (1.0 - frac) + sorted[upper] * frac
        }
    }

    /// Keep values where `cond` is True; replace others with `other`.
    ///
    /// Matches `df.where(cond, other)`. Applies element-wise to each column.
    /// If `other` is `None`, replaced values become NaN.
    pub fn where_cond(
        &self,
        cond: &Self,
        other: Option<&Scalar>,
    ) -> Result<Self, FrameError> {
        let fill = other.cloned().unwrap_or(Scalar::Null(NullKind::NaN));
        let mut new_columns = BTreeMap::new();

        for col_name in &self.column_order {
            let data_col = self
                .columns
                .get(col_name)
                .expect("column in order must exist");
            let cond_col = cond
                .columns
                .get(col_name)
                .ok_or_else(|| {
                    FrameError::CompatibilityRejected(format!(
                        "where: condition missing column '{col_name}'"
                    ))
                })?;

            let values: Vec<Scalar> = data_col
                .values()
                .iter()
                .zip(cond_col.values())
                .map(|(val, c)| match c {
                    Scalar::Bool(true) => val.clone(),
                    Scalar::Bool(false) => fill.clone(),
                    Scalar::Null(_) => Scalar::Null(NullKind::NaN),
                    _ => fill.clone(),
                })
                .collect();

            new_columns.insert(col_name.clone(), Column::from_values(values)?);
        }

        Self::new_with_column_order(
            self.index.clone(),
            new_columns,
            self.column_order.clone(),
        )
    }

    /// Replace values where `cond` is True with `other`.
    ///
    /// Matches `df.mask(cond, other)`. Inverse of `where_cond`.
    pub fn mask(
        &self,
        cond: &Self,
        other: Option<&Scalar>,
    ) -> Result<Self, FrameError> {
        let fill = other.cloned().unwrap_or(Scalar::Null(NullKind::NaN));
        let mut new_columns = BTreeMap::new();

        for col_name in &self.column_order {
            let data_col = self
                .columns
                .get(col_name)
                .expect("column in order must exist");
            let cond_col = cond
                .columns
                .get(col_name)
                .ok_or_else(|| {
                    FrameError::CompatibilityRejected(format!(
                        "mask: condition missing column '{col_name}'"
                    ))
                })?;

            let values: Vec<Scalar> = data_col
                .values()
                .iter()
                .zip(cond_col.values())
                .map(|(val, c)| match c {
                    Scalar::Bool(true) => fill.clone(),
                    Scalar::Bool(false) => val.clone(),
                    Scalar::Null(_) => Scalar::Null(NullKind::NaN),
                    _ => val.clone(),
                })
                .collect();

            new_columns.insert(col_name.clone(), Column::from_values(values)?);
        }

        Self::new_with_column_order(
            self.index.clone(),
            new_columns,
            self.column_order.clone(),
        )
    }

    /// Iterate over rows as `(IndexLabel, Vec<(&str, Scalar)>)` pairs.
    ///
    /// Matches `df.iterrows()`. Returns an iterator over (index_label, row_data)
    /// where row_data is a vector of (column_name, value) pairs.
    pub fn iterrows(&self) -> Vec<(IndexLabel, Vec<(&str, Scalar)>)> {
        let mut rows = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            let label = self.index.labels()[i].clone();
            let row: Vec<(&str, Scalar)> = self
                .column_order
                .iter()
                .map(|col_name| {
                    let val = self.columns.get(col_name).map_or(
                        Scalar::Null(NullKind::Null),
                        |col| col.values()[i].clone(),
                    );
                    (col_name.as_str(), val)
                })
                .collect();
            rows.push((label, row));
        }
        rows
    }

    /// Iterate over rows as tuples of `(IndexLabel, Vec<Scalar>)`.
    ///
    /// Matches `df.itertuples()`. Returns (index_label, values_in_column_order).
    pub fn itertuples(&self) -> Vec<(IndexLabel, Vec<Scalar>)> {
        let mut rows = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            let label = self.index.labels()[i].clone();
            let vals: Vec<Scalar> = self
                .column_order
                .iter()
                .map(|col_name| {
                    self.columns.get(col_name).map_or(
                        Scalar::Null(NullKind::Null),
                        |col| col.values()[i].clone(),
                    )
                })
                .collect();
            rows.push((label, vals));
        }
        rows
    }

    /// Iterate over columns as `(column_name, Series)` pairs.
    ///
    /// Matches `df.items()` / `df.iteritems()`.
    pub fn items(&self) -> Result<Vec<(String, Series)>, FrameError> {
        let mut result = Vec::with_capacity(self.column_order.len());
        for col_name in &self.column_order {
            let col = self
                .columns
                .get(col_name)
                .expect("column in order must exist");
            let series = Series::new(col_name.clone(), self.index.clone(), col.clone())?;
            result.push((col_name.clone(), series));
        }
        Ok(result)
    }

    /// Assign new or overwrite existing columns.
    ///
    /// Matches `df.assign(**kwargs)`. Takes a list of (column_name, Column) pairs.
    /// Existing columns are replaced; new columns are appended.
    pub fn assign(&self, assignments: Vec<(&str, Column)>) -> Result<Self, FrameError> {
        let mut new_columns = self.columns.clone();
        let mut new_order = self.column_order.clone();

        for (name, col) in assignments {
            if col.len() != self.len() {
                return Err(FrameError::LengthMismatch {
                    index_len: self.len(),
                    column_len: col.len(),
                });
            }
            let name_str = name.to_owned();
            if !new_columns.contains_key(&name_str) {
                new_order.push(name_str.clone());
            }
            new_columns.insert(name_str, col);
        }

        Self::new_with_column_order(self.index.clone(), new_columns, new_order)
    }

    /// Apply a transformation function to the DataFrame.
    ///
    /// Matches `df.pipe(func)`. The function receives a reference to `self`
    /// and returns a new DataFrame.
    pub fn pipe<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: FnOnce(&Self) -> Result<Self, FrameError>,
    {
        func(self)
    }

    /// Rank values in each column of the DataFrame.
    ///
    /// Applies `Series::rank` independently to each column.
    pub fn rank(
        &self,
        method: &str,
        ascending: bool,
        na_option: &str,
    ) -> Result<Self, FrameError> {
        let mut ranked_cols = BTreeMap::new();
        for col_name in &self.column_order {
            let col = self
                .columns
                .get(col_name)
                .ok_or_else(|| FrameError::CompatibilityRejected(format!("missing column: {col_name}")))?;
            let series = Series::new(col_name, self.index.clone(), col.clone())?;
            let ranked = series.rank(method, ascending, na_option)?;
            ranked_cols.insert(col_name.clone(), ranked.column().clone());
        }
        Ok(Self {
            columns: ranked_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Unpivot (melt) a DataFrame from wide to long format.
    ///
    /// `id_vars`: columns to keep as identifiers
    /// `value_vars`: columns to unpivot (if empty, uses all non-id columns)
    /// `var_name`: name for the variable column (default "variable")
    /// `value_name`: name for the value column (default "value")
    pub fn melt(
        &self,
        id_vars: &[&str],
        value_vars: &[&str],
        var_name: Option<&str>,
        value_name: Option<&str>,
    ) -> Result<Self, FrameError> {
        let var_col_name = var_name.unwrap_or("variable");
        let val_col_name = value_name.unwrap_or("value");

        // Determine value_vars: if empty, use all non-id columns
        let actual_value_vars: Vec<String> = if value_vars.is_empty() {
            self.column_order
                .iter()
                .filter(|c| !id_vars.contains(&c.as_str()))
                .cloned()
                .collect()
        } else {
            value_vars.iter().map(|s| (*s).to_string()).collect()
        };

        // Validate columns exist
        for col in id_vars {
            if !self.columns.contains_key(*col) {
                return Err(FrameError::CompatibilityRejected(format!("missing column: {col}")));
            }
        }
        for col in &actual_value_vars {
            if !self.columns.contains_key(col) {
                return Err(FrameError::CompatibilityRejected(format!("missing column: {col}")));
            }
        }

        let n_rows = self.index.len();
        let n_value_vars = actual_value_vars.len();
        let total_rows = n_rows * n_value_vars;

        // Build id columns (repeated for each value var)
        let mut result_cols: BTreeMap<String, Column> = BTreeMap::new();
        let mut col_order = Vec::new();

        for &id_col in id_vars {
            let src = &self.columns[id_col];
            let mut repeated = Vec::with_capacity(total_rows);
            for _ in 0..n_value_vars {
                repeated.extend_from_slice(src.values());
            }
            result_cols.insert(
                id_col.to_string(),
                Column::new(src.dtype(), repeated)?,
            );
            col_order.push(id_col.to_string());
        }

        // Build the "variable" column
        let mut var_vals = Vec::with_capacity(total_rows);
        for vv in &actual_value_vars {
            for _ in 0..n_rows {
                var_vals.push(Scalar::Utf8(vv.clone()));
            }
        }
        result_cols.insert(
            var_col_name.to_string(),
            Column::new(DType::Utf8, var_vals)?,
        );
        col_order.push(var_col_name.to_string());

        // Build the "value" column
        let mut value_vals = Vec::with_capacity(total_rows);
        for vv in &actual_value_vars {
            let src = &self.columns[vv];
            value_vals.extend_from_slice(src.values());
        }
        // Determine the common dtype for the value column
        let value_dtype = if actual_value_vars.is_empty() {
            DType::Float64
        } else {
            self.columns[&actual_value_vars[0]].dtype()
        };
        result_cols.insert(
            val_col_name.to_string(),
            Column::new(value_dtype, value_vals)?,
        );
        col_order.push(val_col_name.to_string());

        // Build the new index (0..total_rows)
        let new_labels: Vec<IndexLabel> =
            (0..total_rows as i64).map(IndexLabel::Int64).collect();
        let new_index = Index::new(new_labels);

        Ok(Self {
            columns: result_cols,
            column_order: col_order,
            index: new_index,
        })
    }

    /// Pivot table: aggregate values grouped by index and column keys.
    ///
    /// `values`: column containing values to aggregate
    /// `index_col`: column to use as the new row index
    /// `columns_col`: column whose unique values become new columns
    /// `aggfunc`: aggregation function ("mean", "sum", "count", "min", "max", "first")
    pub fn pivot_table(
        &self,
        values: &str,
        index_col: &str,
        columns_col: &str,
        aggfunc: &str,
    ) -> Result<Self, FrameError> {
        // Validate columns
        for col in [values, index_col, columns_col] {
            if !self.columns.contains_key(col) {
                return Err(FrameError::CompatibilityRejected(format!("missing column: {col}")));
            }
        }

        let val_col = &self.columns[values];
        let idx_col = &self.columns[index_col];
        let cols_col = &self.columns[columns_col];
        let n_rows = self.index.len();

        // Collect unique index values (preserving first-seen order)
        let mut idx_order: Vec<String> = Vec::new();
        let mut idx_set = std::collections::HashSet::new();
        for i in 0..n_rows {
            let key = format!("{:?}", idx_col.values()[i]);
            if idx_set.insert(key.clone()) {
                idx_order.push(key);
            }
        }

        // Collect unique column values (preserving first-seen order)
        let mut col_order_unique: Vec<String> = Vec::new();
        let mut col_set = std::collections::HashSet::new();
        for i in 0..n_rows {
            let key = format!("{:?}", cols_col.values()[i]);
            if col_set.insert(key.clone()) {
                col_order_unique.push(key);
            }
        }

        // Build groups: (idx_key, col_key) -> Vec<f64>
        let mut groups: std::collections::HashMap<(String, String), Vec<f64>> =
            std::collections::HashMap::new();
        for i in 0..n_rows {
            let ik = format!("{:?}", idx_col.values()[i]);
            let ck = format!("{:?}", cols_col.values()[i]);
            if let Ok(v) = val_col.values()[i].to_f64() {
                groups.entry((ik, ck)).or_default().push(v);
            }
        }

        // Aggregate
        let agg_fn: fn(&[f64]) -> f64 = match aggfunc {
            "sum" => |vals: &[f64]| vals.iter().sum(),
            "mean" => |vals: &[f64]| {
                if vals.is_empty() {
                    f64::NAN
                } else {
                    vals.iter().sum::<f64>() / vals.len() as f64
                }
            },
            "count" => |vals: &[f64]| vals.len() as f64,
            "min" => |vals: &[f64]| {
                vals.iter().copied().fold(f64::INFINITY, f64::min)
            },
            "max" => |vals: &[f64]| {
                vals.iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max)
            },
            "first" => |vals: &[f64]| {
                vals.first().copied().unwrap_or(f64::NAN)
            },
            _ => {
                return Err(FrameError::CompatibilityRejected(format!(
                    "pivot_table aggfunc '{aggfunc}' not supported"
                )));
            }
        };

        // Build result columns
        let mut result_cols = BTreeMap::new();
        let mut result_col_order = Vec::new();

        // Extract clean column names from debug format
        let clean_col_name = |s: &str| -> String {
            // Strip Utf8("...") or Int64(...) wrappers
            if let Some(inner) = s.strip_prefix("Utf8(\"") {
                inner.strip_suffix("\")").unwrap_or(inner).to_string()
            } else if let Some(inner) = s.strip_prefix("Int64(") {
                inner.strip_suffix(')').unwrap_or(inner).to_string()
            } else if let Some(inner) = s.strip_prefix("Float64(") {
                inner.strip_suffix(')').unwrap_or(inner).to_string()
            } else {
                s.to_string()
            }
        };

        for ck in &col_order_unique {
            let col_name = clean_col_name(ck);
            let mut col_vals = Vec::with_capacity(idx_order.len());
            for ik in &idx_order {
                if let Some(vals) = groups.get(&(ik.clone(), ck.clone())) {
                    col_vals.push(Scalar::Float64(agg_fn(vals)));
                } else {
                    col_vals.push(Scalar::Null(NullKind::NaN));
                }
            }
            result_cols.insert(
                col_name.clone(),
                Column::new(DType::Float64, col_vals)?,
            );
            result_col_order.push(col_name);
        }

        // Build index from idx_order
        let new_labels: Vec<IndexLabel> = idx_order
            .iter()
            .map(|s| {
                let cleaned = clean_col_name(s);
                if let Ok(i) = cleaned.parse::<i64>() {
                    IndexLabel::Int64(i)
                } else {
                    IndexLabel::Utf8(cleaned)
                }
            })
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: result_col_order,
            index: Index::new(new_labels),
        })
    }

    /// Aggregate using one or more operations over the specified axis.
    ///
    /// `funcs`: mapping of column_name → list of aggregation function names.
    /// Returns a DataFrame with index = function names and columns = original column names.
    pub fn agg(
        &self,
        funcs: &std::collections::HashMap<String, Vec<String>>,
    ) -> Result<Self, FrameError> {
        // Collect all unique function names in insertion order
        let mut all_funcs: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for fns in funcs.values() {
            for f in fns {
                if seen.insert(f.clone()) {
                    all_funcs.push(f.clone());
                }
            }
        }

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &self.column_order {
            if let Some(col_funcs) = funcs.get(col_name) {
                let col = &self.columns[col_name];
                let series = Series::new(col_name, self.index.clone(), col.clone())?;
                let mut vals = Vec::with_capacity(all_funcs.len());
                for func_name in &all_funcs {
                    if col_funcs.contains(func_name) {
                        let v = match func_name.as_str() {
                            "sum" => series.sum()?,
                            "mean" => series.mean()?,
                            "min" => series.min()?,
                            "max" => series.max()?,
                            "std" => series.std()?,
                            "var" => series.var()?,
                            "median" => series.median()?,
                            "count" => Scalar::Int64(series.count() as i64),
                            other => {
                                return Err(FrameError::CompatibilityRejected(
                                    format!("unsupported agg function: '{other}'"),
                                ));
                            }
                        };
                        vals.push(v);
                    } else {
                        vals.push(Scalar::Null(NullKind::NaN));
                    }
                }
                result_cols.insert(
                    col_name.clone(),
                    Column::new(DType::Float64, vals)?,
                );
                col_order.push(col_name.clone());
            }
        }

        let labels: Vec<IndexLabel> = all_funcs
            .iter()
            .map(|f| IndexLabel::Utf8(f.clone()))
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(labels),
        })
    }

    /// Apply a function element-wise to every value in the DataFrame.
    ///
    /// Returns a new DataFrame with the same shape and index.
    pub fn applymap<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Scalar,
    {
        let mut result_cols = BTreeMap::new();
        for col_name in &self.column_order {
            let col = &self.columns[col_name];
            let new_vals: Vec<Scalar> = col.values().iter().map(&func).collect();
            let dtype = if new_vals.is_empty() {
                col.dtype()
            } else {
                new_vals[0].dtype()
            };
            result_cols.insert(col_name.clone(), Column::new(dtype, new_vals)?);
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Apply a function element-wise, preserving the shape of the DataFrame.
    ///
    /// Similar to `applymap` but validates that the output has the same shape.
    pub fn transform<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Scalar,
    {
        self.applymap(func)
    }

    /// Compute the Pearson correlation matrix between numeric columns.
    pub fn corr(&self) -> Result<Self, FrameError> {
        self.pairwise_stat("corr")
    }

    /// Compute the pairwise covariance matrix between numeric columns.
    pub fn cov(&self) -> Result<Self, FrameError> {
        self.pairwise_stat("cov")
    }

    /// Internal helper for corr/cov pairwise matrix computation.
    fn pairwise_stat(&self, stat: &str) -> Result<Self, FrameError> {
        let n = self.column_order.len();
        let len = self.index.len();

        // Extract f64 columns
        let col_data: Vec<Vec<f64>> = self
            .column_order
            .iter()
            .map(|name| {
                let col = &self.columns[name];
                (0..len)
                    .map(|i| col.values()[i].to_f64().unwrap_or(f64::NAN))
                    .collect()
            })
            .collect();

        let mut result_cols = BTreeMap::new();
        for (j, col_j_name) in self.column_order.iter().enumerate() {
            let mut vals = Vec::with_capacity(n);
            for (i, _col_i_name) in self.column_order.iter().enumerate() {
                // Compute pairwise stat between col_i and col_j
                let mut sum_x = 0.0_f64;
                let mut sum_y = 0.0_f64;
                let mut sum_xy = 0.0_f64;
                let mut sum_x2 = 0.0_f64;
                let mut sum_y2 = 0.0_f64;
                let mut count = 0_usize;

                for (&x, &y) in col_data[i].iter().zip(col_data[j].iter()) {
                    if x.is_nan() || y.is_nan() {
                        continue;
                    }
                    sum_x += x;
                    sum_y += y;
                    sum_xy += x * y;
                    sum_x2 += x * x;
                    sum_y2 += y * y;
                    count += 1;
                }

                let val = if count < 2 {
                    f64::NAN
                } else {
                    let n_f = count as f64;
                    let mean_x = sum_x / n_f;
                    let mean_y = sum_y / n_f;
                    let cov_xy =
                        (sum_xy - n_f * mean_x * mean_y) / (n_f - 1.0);

                    match stat {
                        "cov" => cov_xy,
                        "corr" => {
                            let var_x =
                                (sum_x2 - n_f * mean_x * mean_x) / (n_f - 1.0);
                            let var_y =
                                (sum_y2 - n_f * mean_y * mean_y) / (n_f - 1.0);
                            let denom = (var_x * var_y).sqrt();
                            if denom < f64::EPSILON {
                                f64::NAN
                            } else {
                                cov_xy / denom
                            }
                        }
                        _ => f64::NAN,
                    }
                };
                vals.push(Scalar::Float64(val));
            }
            result_cols.insert(
                col_j_name.clone(),
                Column::new(DType::Float64, vals)?,
            );
        }

        let labels: Vec<IndexLabel> = self
            .column_order
            .iter()
            .map(|s| IndexLabel::Utf8(s.clone()))
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: Index::new(labels),
        })
    }

    /// Compute the number of unique values per column.
    pub fn value_counts_per_column(&self) -> Result<Self, FrameError> {
        let mut labels = Vec::new();
        let mut counts = Vec::new();
        for col_name in &self.column_order {
            let col = &self.columns[col_name];
            let mut seen = std::collections::HashSet::new();
            for v in col.values() {
                if !v.is_missing() {
                    seen.insert(format!("{v:?}"));
                }
            }
            labels.push(IndexLabel::Utf8(col_name.clone()));
            counts.push(Scalar::Int64(seen.len() as i64));
        }
        let mut result_cols = BTreeMap::new();
        result_cols.insert(
            "nunique".to_string(),
            Column::new(DType::Int64, counts)?,
        );
        Ok(Self {
            columns: result_cols,
            column_order: vec!["nunique".to_string()],
            index: Index::new(labels),
        })
    }

    /// Get the top N rows ordered by a column.
    pub fn nlargest(&self, n: usize, column: &str) -> Result<Self, FrameError> {
        let sorted = self.sort_values(column, false)?;
        sorted.head(n as i64)
    }

    /// Get the bottom N rows ordered by a column.
    pub fn nsmallest(&self, n: usize, column: &str) -> Result<Self, FrameError> {
        let sorted = self.sort_values(column, true)?;
        sorted.head(n as i64)
    }

    /// Reindex the DataFrame to a new set of index labels.
    ///
    /// Missing rows are filled with NaN.
    pub fn reindex(&self, new_labels: Vec<IndexLabel>) -> Result<Self, FrameError> {
        // Build a lookup from old label -> row index
        let mut old_lookup: std::collections::HashMap<&IndexLabel, usize> =
            std::collections::HashMap::new();
        for (i, label) in self.index.labels().iter().enumerate() {
            old_lookup.entry(label).or_insert(i);
        }

        let new_len = new_labels.len();
        let mut result_cols = BTreeMap::new();

        for col_name in &self.column_order {
            let col = &self.columns[col_name];
            let mut new_vals = Vec::with_capacity(new_len);
            for label in &new_labels {
                if let Some(&idx) = old_lookup.get(label) {
                    new_vals.push(col.values()[idx].clone());
                } else {
                    new_vals.push(Scalar::Null(NullKind::NaN));
                }
            }
            result_cols.insert(
                col_name.clone(),
                Column::new(col.dtype(), new_vals)?,
            );
        }

        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: Index::new(new_labels),
        })
    }

    /// Group by one or more columns and aggregate.
    ///
    /// Returns a `DataFrameGroupBy` for deferred aggregation.
    pub fn groupby(&self, by: &[&str]) -> Result<DataFrameGroupBy<'_>, FrameError> {
        // Validate columns exist
        for col in by {
            if !self.columns.contains_key(*col) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "missing column: {col}"
                )));
            }
        }
        Ok(DataFrameGroupBy {
            df: self,
            by: by.iter().map(|s| (*s).to_string()).collect(),
        })
    }

    /// Get summary info about the DataFrame: dtypes, non-null counts, memory.
    pub fn info(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("DataFrame: {} rows x {} columns", self.len(), self.column_order.len()));
        lines.push(format!("Index: {} entries", self.index.len()));
        lines.push(String::new());
        lines.push(format!("{:<20} {:<10} {:<10}", "Column", "Non-Null", "Dtype"));
        lines.push(format!("{:<20} {:<10} {:<10}", "------", "--------", "-----"));
        for col_name in &self.column_order {
            let col = &self.columns[col_name];
            let non_null = col.values().iter().filter(|v| !v.is_missing()).count();
            lines.push(format!(
                "{:<20} {:<10} {:?}",
                col_name,
                non_null,
                col.dtype()
            ));
        }
        lines.join("\n")
    }

    /// Randomly sample rows from the DataFrame.
    ///
    /// `n`: number of rows to sample (mutually exclusive with `frac`).
    /// `frac`: fraction of rows to sample.
    /// `replace`: whether to sample with replacement.
    /// `seed`: optional deterministic seed.
    pub fn sample(
        &self,
        n: Option<usize>,
        frac: Option<f64>,
        replace: bool,
        seed: Option<u64>,
    ) -> Result<Self, FrameError> {
        let total = self.len();
        let sample_n = match (n, frac) {
            (Some(count), None) => count,
            (None, Some(f)) => (total as f64 * f).ceil() as usize,
            (None, None) => 1,
            (Some(_), Some(_)) => {
                return Err(FrameError::CompatibilityRejected(
                    "cannot specify both n and frac".to_string(),
                ));
            }
        };

        if !replace && sample_n > total {
            return Err(FrameError::CompatibilityRejected(format!(
                "cannot sample {sample_n} rows from {total} without replacement"
            )));
        }

        // Simple LCG for deterministic sampling
        let mut rng_state = seed.unwrap_or(42);
        let mut next_rand = || -> usize {
            rng_state = rng_state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (rng_state >> 33) as usize
        };

        let indices: Vec<usize> = if replace {
            (0..sample_n).map(|_| next_rand() % total).collect()
        } else {
            // Fisher-Yates shuffle on index array, take first sample_n
            let mut pool: Vec<usize> = (0..total).collect();
            for i in 0..sample_n {
                let j = i + (next_rand() % (total - i));
                pool.swap(i, j);
            }
            pool[..sample_n].to_vec()
        };

        // Build result
        let mut result_cols = BTreeMap::new();
        for col_name in &self.column_order {
            let col = &self.columns[col_name];
            let vals: Vec<Scalar> = indices.iter().map(|&i| col.values()[i].clone()).collect();
            result_cols.insert(col_name.clone(), Column::new(col.dtype(), vals)?);
        }

        let new_labels: Vec<IndexLabel> = indices
            .iter()
            .map(|&i| self.index.labels()[i].clone())
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: Index::new(new_labels),
        })
    }

    /// Stack: pivot columns into rows (wide-to-long).
    ///
    /// Converts a DataFrame with columns [A, B, ...] into a two-column
    /// DataFrame with a multi-level index (original_index, column_name)
    /// and a single value column. Similar to pandas `DataFrame.stack()`.
    pub fn stack(&self) -> Result<Self, FrameError> {
        let n_rows = self.len();
        let n_cols = self.column_order.len();
        let total = n_rows * n_cols;

        let mut new_labels = Vec::with_capacity(total);
        let mut values = Vec::with_capacity(total);

        for row in 0..n_rows {
            let row_label = &self.index.labels()[row];
            for col_name in &self.column_order {
                // Create composite label: "row_label|col_name"
                let label_str = match row_label {
                    IndexLabel::Int64(v) => format!("{v}|{col_name}"),
                    IndexLabel::Utf8(v) => format!("{v}|{col_name}"),
                };
                new_labels.push(IndexLabel::Utf8(label_str));
                values.push(self.columns[col_name].values()[row].clone());
            }
        }

        let value_col = Column::new(DType::Float64, values)?;
        let mut cols = BTreeMap::new();
        cols.insert("value".to_string(), value_col);

        Ok(Self {
            columns: cols,
            column_order: vec!["value".to_string()],
            index: Index::new(new_labels),
        })
    }

    /// Unstack: pivot rows into columns (long-to-wide).
    ///
    /// Assumes the index contains composite labels in "row|col" format
    /// (as produced by `stack()`). Produces a DataFrame with unique row
    /// keys as index and unique column keys as columns.
    pub fn unstack(&self) -> Result<Self, FrameError> {
        if self.column_order.len() != 1 {
            return Err(FrameError::CompatibilityRejected(
                "unstack requires exactly one value column".to_string(),
            ));
        }
        let val_col_name = &self.column_order[0];
        let val_col = &self.columns[val_col_name];

        // Parse composite keys
        let mut row_order: Vec<String> = Vec::new();
        let mut row_set = std::collections::HashSet::new();
        let mut col_order: Vec<String> = Vec::new();
        let mut col_set = std::collections::HashSet::new();
        let mut entries: Vec<(String, String, Scalar)> = Vec::new();

        for (i, label) in self.index.labels().iter().enumerate() {
            let label_str = match label {
                IndexLabel::Utf8(s) => s.clone(),
                IndexLabel::Int64(v) => v.to_string(),
            };
            if let Some(sep_pos) = label_str.rfind('|') {
                let row_key = label_str[..sep_pos].to_string();
                let col_key = label_str[sep_pos + 1..].to_string();
                if row_set.insert(row_key.clone()) {
                    row_order.push(row_key.clone());
                }
                if col_set.insert(col_key.clone()) {
                    col_order.push(col_key.clone());
                }
                entries.push((row_key, col_key, val_col.values()[i].clone()));
            }
        }

        // Build lookup
        let mut lookup: std::collections::HashMap<(String, String), Scalar> =
            std::collections::HashMap::new();
        for (rk, ck, val) in entries {
            lookup.insert((rk, ck), val);
        }

        // Build result columns
        let mut result_cols = BTreeMap::new();
        for ck in &col_order {
            let mut vals = Vec::with_capacity(row_order.len());
            for rk in &row_order {
                if let Some(val) = lookup.get(&(rk.clone(), ck.clone())) {
                    vals.push(val.clone());
                } else {
                    vals.push(Scalar::Null(NullKind::NaN));
                }
            }
            result_cols.insert(ck.clone(), Column::new(DType::Float64, vals)?);
        }

        let new_labels: Vec<IndexLabel> = row_order
            .iter()
            .map(|s| {
                if let Ok(i) = s.parse::<i64>() {
                    IndexLabel::Int64(i)
                } else {
                    IndexLabel::Utf8(s.clone())
                }
            })
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(new_labels),
        })
    }

    /// Apply a closure to each column (axis=0) or each row (axis=1).
    ///
    /// Unlike the string-based `apply()`, this takes a Rust closure.
    pub fn apply_fn<F>(&self, func: F, axis: usize) -> Result<Self, FrameError>
    where
        F: Fn(&[Scalar]) -> Scalar,
    {
        if axis == 0 {
            // Column-wise: apply func to each column's values
            let mut labels = Vec::new();
            let mut values = Vec::new();
            for name in &self.column_order {
                let col = &self.columns[name];
                labels.push(IndexLabel::Utf8(name.clone()));
                values.push(func(col.values()));
            }
            let s = Series::from_values("result", labels, values)?;
            // Return as single-column DataFrame
            let mut cols = BTreeMap::new();
            cols.insert("result".to_string(), s.column().clone());
            Ok(Self {
                columns: cols,
                column_order: vec!["result".to_string()],
                index: s.index().clone(),
            })
        } else {
            // Row-wise: apply func to each row
            let mut values = Vec::with_capacity(self.len());
            for row_idx in 0..self.len() {
                let row_vals: Vec<Scalar> = self
                    .column_order
                    .iter()
                    .map(|name| self.columns[name].values()[row_idx].clone())
                    .collect();
                values.push(func(&row_vals));
            }
            let s = Series::from_values("result", self.index.labels().to_vec(), values)?;
            let mut cols = BTreeMap::new();
            cols.insert("result".to_string(), s.column().clone());
            Ok(Self {
                columns: cols,
                column_order: vec!["result".to_string()],
                index: self.index.clone(),
            })
        }
    }
}

/// Deferred GroupBy object for DataFrame.
///
/// Created by `DataFrame::groupby()`. Call aggregation methods to produce results.
pub struct DataFrameGroupBy<'a> {
    df: &'a DataFrame,
    by: Vec<String>,
}

impl DataFrameGroupBy<'_> {
    /// Internal: build groups as (composite_key -> Vec<row_index>).
    fn build_groups(&self) -> (Vec<String>, std::collections::HashMap<String, Vec<usize>>) {
        let n = self.df.len();
        let mut group_order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();

        for row in 0..n {
            let key: String = self
                .by
                .iter()
                .map(|col_name| {
                    let val = &self.df.columns[col_name].values()[row];
                    format!("{val:?}")
                })
                .collect::<Vec<_>>()
                .join("|");

            if !groups.contains_key(&key) {
                group_order.push(key.clone());
            }
            groups.entry(key).or_default().push(row);
        }

        (group_order, groups)
    }

    /// Internal: extract the group key label for a given row index.
    fn group_key_label(&self, row: usize) -> IndexLabel {
        if self.by.len() == 1 {
            let val = &self.df.columns[&self.by[0]].values()[row];
            match val {
                Scalar::Int64(v) => IndexLabel::Int64(*v),
                Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
                other => IndexLabel::Utf8(format!("{other:?}")),
            }
        } else {
            let parts: Vec<String> = self
                .by
                .iter()
                .map(|col_name| {
                    let val = &self.df.columns[col_name].values()[row];
                    match val {
                        Scalar::Int64(v) => v.to_string(),
                        Scalar::Utf8(v) => v.clone(),
                        other => format!("{other:?}"),
                    }
                })
                .collect();
            IndexLabel::Utf8(parts.join(", "))
        }
    }

    /// Aggregate each value column per group with the given function.
    fn aggregate(&self, func_name: &str) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();

        // Determine value columns (all columns not in group-by keys)
        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let n_groups = group_order.len();

        // Build index from group keys
        let mut labels = Vec::with_capacity(n_groups);
        for gkey in &group_order {
            let first_row = groups[gkey][0];
            labels.push(self.group_key_label(first_row));
        }

        // Aggregate each value column
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut agg_vals = Vec::with_capacity(n_groups);

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let group_vals: Vec<Scalar> = row_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();

                let agg_val = match func_name {
                    "sum" => fp_types::nansum(&group_vals),
                    "mean" => fp_types::nanmean(&group_vals),
                    "count" => fp_types::nancount(&group_vals),
                    "min" => fp_types::nanmin(&group_vals),
                    "max" => fp_types::nanmax(&group_vals),
                    "std" => fp_types::nanstd(&group_vals, 1),
                    "var" => fp_types::nanvar(&group_vals, 1),
                    "median" => fp_types::nanmedian(&group_vals),
                    "first" => {
                        if group_vals.is_empty() {
                            Scalar::Null(NullKind::NaN)
                        } else {
                            group_vals[0].clone()
                        }
                    }
                    "last" => {
                        if group_vals.is_empty() {
                            Scalar::Null(NullKind::NaN)
                        } else {
                            group_vals[group_vals.len() - 1].clone()
                        }
                    }
                    other => {
                        return Err(FrameError::CompatibilityRejected(format!(
                            "unsupported groupby aggregation: '{other}'"
                        )));
                    }
                };
                agg_vals.push(agg_val);
            }

            result_cols.insert(col_name.clone(), Column::new(DType::Float64, agg_vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(labels),
        })
    }

    /// GroupBy sum.
    pub fn sum(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("sum")
    }

    /// GroupBy mean.
    pub fn mean(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("mean")
    }

    /// GroupBy count.
    pub fn count(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("count")
    }

    /// GroupBy min.
    pub fn min(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("min")
    }

    /// GroupBy max.
    pub fn max(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("max")
    }

    /// GroupBy std.
    pub fn std(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("std")
    }

    /// GroupBy var.
    pub fn var(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("var")
    }

    /// GroupBy median.
    pub fn median(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("median")
    }

    /// GroupBy first.
    pub fn first(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("first")
    }

    /// GroupBy last.
    pub fn last(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("last")
    }

    /// GroupBy size (number of rows per group).
    pub fn size(&self) -> Result<Series, FrameError> {
        let (group_order, groups) = self.build_groups();
        let mut labels = Vec::with_capacity(group_order.len());
        let mut values = Vec::with_capacity(group_order.len());

        for gkey in &group_order {
            let first_row = groups[gkey][0];
            labels.push(self.group_key_label(first_row));
            values.push(Scalar::Int64(groups[gkey].len() as i64));
        }

        Series::from_values("size", labels, values)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::{DType, NullKind, Scalar};

    use super::{
        DataFrame, DataFrameColumnInput, DropNaHow, DuplicateKeep, FrameError, IndexLabel, Series,
    };

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
    fn strict_mode_add_aligns_non_identical_duplicate_indices() {
        let left = Series::from_values(
            "left",
            vec![IndexLabel::from("a"), IndexLabel::from("a")],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right =
            Series::from_values("right", vec![IndexLabel::from("a")], vec![Scalar::Int64(3)])
                .expect("right");

        let out = left.add(&right).expect("strict mode should align");
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from("a"), IndexLabel::from("a")]
        );
        assert_eq!(out.values(), &[Scalar::Int64(4), Scalar::Int64(5)]);
    }

    #[test]
    fn strict_mode_allows_duplicate_indices_when_exactly_aligned() {
        let left = Series::from_values(
            "left",
            vec![IndexLabel::from("a"), IndexLabel::from("a")],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec![IndexLabel::from("a"), IndexLabel::from("a")],
            vec![Scalar::Int64(3), Scalar::Int64(4)],
        )
        .expect("right");

        let out = left
            .add(&right)
            .expect("strict mode should allow exact duplicate alignment");
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from("a"), IndexLabel::from("a")]
        );
        assert_eq!(out.values(), &[Scalar::Int64(4), Scalar::Int64(6)]);
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
    fn series_sub_strict_aligns_non_identical_duplicates() {
        let left = Series::from_values(
            "a",
            vec!["x".into(), "x".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let right = Series::from_values("b", vec!["x".into()], vec![Scalar::Int64(3)]).unwrap();

        let out = left.sub(&right).expect("strict should align");
        assert_eq!(out.index().labels(), &["x".into(), "x".into()]);
        assert_eq!(out.values(), &[Scalar::Int64(-2), Scalar::Int64(-1)]);
    }

    #[test]
    fn series_add_non_identical_duplicate_indices_cross_products_shared_labels() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();
        let right = Series::from_values(
            "right",
            vec!["a".into(), "a".into(), "c".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        let out = left.add(&right).expect("strict should align");
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from("a"),
                IndexLabel::from("a"),
                IndexLabel::from("a"),
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c")
            ]
        );
        assert_eq!(
            out.values(),
            &[
                Scalar::Int64(11),
                Scalar::Int64(21),
                Scalar::Int64(12),
                Scalar::Int64(22),
                Scalar::Null(NullKind::Null),
                Scalar::Null(NullKind::Null)
            ]
        );
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
    fn dataframe_from_dict_mixed_broadcasts_scalar_against_vector_length() {
        let df = DataFrame::from_dict_mixed(
            &["a", "b"],
            vec![
                (
                    "a",
                    DataFrameColumnInput::from(vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ),
                ("b", DataFrameColumnInput::from(Scalar::Int64(9))),
            ],
        )
        .unwrap();

        assert_eq!(df.index().labels(), &[0_i64.into(), 1_i64.into()]);
        assert_eq!(
            df.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            df.column("b").unwrap().values(),
            &[Scalar::Int64(9), Scalar::Int64(9)]
        );
    }

    #[test]
    fn dataframe_from_dict_mixed_all_scalars_without_index_is_rejected() {
        let err = DataFrame::from_dict_mixed(
            &["a", "b"],
            vec![
                ("a", DataFrameColumnInput::from(Scalar::Int64(1))),
                ("b", DataFrameColumnInput::from(Scalar::Int64(2))),
            ],
        )
        .expect_err("all-scalar constructor without index should be rejected");
        assert_eq!(
            err.to_string(),
            "compatibility gate rejected operation: dataframe_from_dict with all-scalar values requires explicit index"
        );
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
    fn dataframe_from_dict_with_index_mixed_broadcasts_scalar() {
        let df = DataFrame::from_dict_with_index_mixed(
            vec![
                (
                    "a",
                    DataFrameColumnInput::from(vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ),
                (
                    "b",
                    DataFrameColumnInput::from(Scalar::Utf8("x".to_owned())),
                ),
            ],
            vec!["r1".into(), "r2".into()],
        )
        .unwrap();

        assert_eq!(df.index().labels(), &["r1".into(), "r2".into()]);
        assert_eq!(
            df.column("b").unwrap().values(),
            &[Scalar::Utf8("x".to_owned()), Scalar::Utf8("x".to_owned())]
        );
    }

    #[test]
    fn dataframe_from_dict_with_index_mixed_rejects_shape_mismatch() {
        let err = DataFrame::from_dict_with_index_mixed(
            vec![
                ("a", DataFrameColumnInput::from(vec![Scalar::Int64(1)])),
                ("b", DataFrameColumnInput::from(Scalar::Int64(5))),
            ],
            vec!["x".into(), "y".into()],
        )
        .expect_err("constructor should reject non-scalar mismatched length");
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
    fn concat_dataframes_axis1_duplicate_index_outer_exact_match_preserves_rows() {
        let left = DataFrame::from_dict_with_index(
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![0_i64.into(), 0_i64.into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![("y", vec![Scalar::Int64(3), Scalar::Int64(4)])],
            vec![0_i64.into(), 0_i64.into()],
        )
        .unwrap();

        let out = concat_dataframes_with_axis(&[&left, &right], 1).unwrap();
        assert_eq!(out.index().labels(), &[0_i64.into(), 0_i64.into()]);
        assert_eq!(
            out.column("x").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            out.column("y").unwrap().values(),
            &[Scalar::Int64(3), Scalar::Int64(4)]
        );
    }

    #[test]
    fn concat_dataframes_axis1_duplicate_index_reindex_rejects() {
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
    fn concat_dataframes_axis1_inner_duplicate_index_reindex_rejects() {
        let left = DataFrame::from_dict_with_index(
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![0_i64.into(), 0_i64.into()],
        )
        .unwrap();
        let right = DataFrame::from_dict_with_index(
            vec![("y", vec![Scalar::Int64(3), Scalar::Int64(4)])],
            vec![0_i64.into(), 0_i64.into()],
        )
        .unwrap();

        let err = concat_dataframes_with_axis_join(&[&left, &right], 1, ConcatJoin::Inner)
            .expect_err("should reject");
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
    fn series_value_counts_sorts_desc_and_drops_missing() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
                IndexLabel::from("r5"),
                IndexLabel::from("r6"),
                IndexLabel::from("r7"),
            ],
            vec![
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let out = s.value_counts().unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::Int64(2),
                IndexLabel::Int64(1),
                IndexLabel::Int64(3)
            ]
        );
        assert_eq!(
            out.values(),
            &[Scalar::Int64(3), Scalar::Int64(2), Scalar::Int64(1)]
        );
    }

    #[test]
    fn series_value_counts_preserves_first_seen_order_for_ties() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
                IndexLabel::from("r5"),
            ],
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("c".to_owned()),
            ],
        )
        .unwrap();

        let out = s.value_counts().unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from("b"),
                IndexLabel::from("a"),
                IndexLabel::from("c")
            ]
        );
        assert_eq!(
            out.values(),
            &[Scalar::Int64(2), Scalar::Int64(2), Scalar::Int64(1)]
        );
    }

    #[test]
    fn series_value_counts_bool_labels_use_python_style_strings() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
                IndexLabel::from("r5"),
                IndexLabel::from("r6"),
            ],
            vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Null(NullKind::Null),
                Scalar::Bool(false),
            ],
        )
        .unwrap();

        let out = s.value_counts().unwrap();
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from("True"), IndexLabel::from("False")]
        );
        assert_eq!(out.values(), &[Scalar::Int64(3), Scalar::Int64(2)]);
    }

    #[test]
    fn series_value_counts_all_missing_returns_empty() {
        let s = Series::from_values(
            "vals",
            vec![IndexLabel::from("r1"), IndexLabel::from("r2")],
            vec![Scalar::Null(NullKind::Null), Scalar::Float64(f64::NAN)],
        )
        .unwrap();

        let out = s.value_counts().unwrap();
        assert!(out.index().labels().is_empty());
        assert!(out.values().is_empty());
    }

    #[test]
    fn series_unique_returns_first_seen_order_skipping_nulls() {
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
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let uniq = s.unique();
        assert_eq!(
            uniq,
            vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)]
        );
    }

    #[test]
    fn series_unique_empty_series() {
        let s = Series::from_values("empty", vec![], vec![]).unwrap();
        assert!(s.unique().is_empty());
        assert_eq!(s.nunique(), 0);
    }

    #[test]
    fn series_nunique_counts_distinct_non_null() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
                IndexLabel::from("d"),
            ],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();

        assert_eq!(s.nunique(), 2);
    }

    #[test]
    fn series_shift_positive() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        let shifted = s.shift(1).unwrap();
        assert!(shifted.values()[0].is_missing());
        assert_eq!(shifted.values()[1], Scalar::Int64(10));
        assert_eq!(shifted.values()[2], Scalar::Int64(20));
    }

    #[test]
    fn series_shift_negative() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();

        let shifted = s.shift(-1).unwrap();
        assert_eq!(shifted.values()[0], Scalar::Int64(20));
        assert_eq!(shifted.values()[1], Scalar::Int64(30));
        assert!(shifted.values()[2].is_missing());
    }

    #[test]
    fn series_diff_basic() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let d = s.diff(1).unwrap();
        assert!(d.values()[0].is_missing()); // no predecessor
        assert_eq!(d.values()[1], Scalar::Float64(2.0));
        assert_eq!(d.values()[2], Scalar::Float64(3.0));
    }

    #[test]
    fn series_cumsum_skips_nan() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
                IndexLabel::from("d"),
            ],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
                Scalar::Float64(5.0),
            ],
        )
        .unwrap();

        let cs = s.cumsum().unwrap();
        assert_eq!(cs.values()[0], Scalar::Float64(1.0));
        assert!(cs.values()[1].is_missing());
        assert_eq!(cs.values()[2], Scalar::Float64(4.0));
        assert_eq!(cs.values()[3], Scalar::Float64(9.0));
    }

    #[test]
    fn series_cumprod_basic() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        let cp = s.cumprod().unwrap();
        assert_eq!(cp.values()[0], Scalar::Float64(2.0));
        assert_eq!(cp.values()[1], Scalar::Float64(6.0));
        assert_eq!(cp.values()[2], Scalar::Float64(24.0));
    }

    #[test]
    fn series_cummin_cummax_basic() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();

        let cmin = s.cummin().unwrap();
        assert_eq!(cmin.values()[0], Scalar::Float64(3.0));
        assert_eq!(cmin.values()[1], Scalar::Float64(1.0));
        assert_eq!(cmin.values()[2], Scalar::Float64(1.0));

        let cmax = s.cummax().unwrap();
        assert_eq!(cmax.values()[0], Scalar::Float64(3.0));
        assert_eq!(cmax.values()[1], Scalar::Float64(3.0));
        assert_eq!(cmax.values()[2], Scalar::Float64(3.0));
    }

    #[test]
    fn series_map_replaces_and_fills_nan() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        let mapped = s
            .map(&[
                (Scalar::Int64(1), Scalar::Utf8("one".to_owned())),
                (Scalar::Int64(2), Scalar::Utf8("two".to_owned())),
            ])
            .unwrap();

        assert_eq!(mapped.values()[0], Scalar::Utf8("one".to_owned()));
        assert_eq!(mapped.values()[1], Scalar::Utf8("two".to_owned()));
        assert!(mapped.values()[2].is_missing()); // 3 not in mapping
    }

    #[test]
    fn series_replace_substitutes_values() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(1)],
        )
        .unwrap();

        let replaced = s.replace(&[(Scalar::Int64(1), Scalar::Int64(99))]).unwrap();

        assert_eq!(replaced.values()[0], Scalar::Int64(99));
        assert_eq!(replaced.values()[1], Scalar::Int64(2)); // unchanged
        assert_eq!(replaced.values()[2], Scalar::Int64(99));
    }

    #[test]
    fn series_quantile_median() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();

        let q50 = s.quantile(0.5).unwrap();
        assert_eq!(q50, Scalar::Float64(2.0));
    }

    #[test]
    fn dataframe_quantile_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let q = df.quantile(0.5).unwrap();
        assert_eq!(q.values()[0], Scalar::Float64(2.0)); // a median
        assert_eq!(q.values()[1], Scalar::Float64(20.0)); // b median
    }

    #[test]
    fn series_clip_bounds_values() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("a"),
                IndexLabel::from("b"),
                IndexLabel::from("c"),
            ],
            vec![
                Scalar::Float64(-5.0),
                Scalar::Float64(3.0),
                Scalar::Float64(10.0),
            ],
        )
        .unwrap();

        let clipped = s.clip(Some(0.0), Some(5.0)).unwrap();
        assert_eq!(clipped.values()[0], Scalar::Float64(0.0));
        assert_eq!(clipped.values()[1], Scalar::Float64(3.0));
        assert_eq!(clipped.values()[2], Scalar::Float64(5.0));
    }

    #[test]
    fn series_abs_and_round() {
        let neg_pi = -std::f64::consts::PI;
        let e = std::f64::consts::E;
        let s = Series::from_values(
            "vals",
            vec![IndexLabel::from("a"), IndexLabel::from("b")],
            vec![Scalar::Float64(neg_pi), Scalar::Float64(e)],
        )
        .unwrap();

        let a = s.abs().unwrap();
        assert!(
            matches!(a.values()[0], Scalar::Float64(v) if (v - std::f64::consts::PI).abs() < 1e-12)
        );

        let r = s.round(2).unwrap();
        let expected_neg_pi_2dp = (neg_pi * 100.0).round() / 100.0;
        let expected_e_2dp = (e * 100.0).round() / 100.0;
        assert!(
            matches!(r.values()[0], Scalar::Float64(v) if (v - expected_neg_pi_2dp).abs() < 1e-12)
        );
        assert!(matches!(r.values()[1], Scalar::Float64(v) if (v - expected_e_2dp).abs() < 1e-12));
    }

    #[test]
    fn dataframe_apply_column_wise() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(4.0),
                        Scalar::Float64(5.0),
                        Scalar::Float64(6.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let sums = df.apply("sum", 0).unwrap();
        assert_eq!(sums.values()[0], Scalar::Float64(6.0)); // sum(a)
        assert_eq!(sums.values()[1], Scalar::Float64(15.0)); // sum(b)
    }

    #[test]
    fn dataframe_apply_row_wise() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();

        let row_sums = df.apply("sum", 1).unwrap();
        assert_eq!(row_sums.values()[0], Scalar::Float64(4.0)); // 1+3
        assert_eq!(row_sums.values()[1], Scalar::Float64(6.0)); // 2+4
    }

    #[test]
    fn dataframe_apply_row_wise_std_var() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
                (
                    "c",
                    vec![Scalar::Null(NullKind::Null), Scalar::Float64(6.0)],
                ),
            ],
        )
        .unwrap();

        let row_vars = df.apply("var", 1).unwrap();
        let row_stds = df.apply("std", 1).unwrap();

        assert!(matches!(row_vars.values()[0], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
        assert!(matches!(row_vars.values()[1], Scalar::Float64(v) if (v - 4.0).abs() < 1e-10));
        assert!(
            matches!(row_stds.values()[0], Scalar::Float64(v) if (v - 2.0_f64.sqrt()).abs() < 1e-10)
        );
        assert!(matches!(row_stds.values()[1], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
    }

    #[test]
    fn dataframe_apply_row_wise_std_var_insufficient_values() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                (
                    "b",
                    vec![Scalar::Null(NullKind::Null), Scalar::Float64(4.0)],
                ),
            ],
        )
        .unwrap();

        let row_vars = df.apply("var", 1).unwrap();
        let row_stds = df.apply("std", 1).unwrap();

        assert!(matches!(row_vars.values()[0], Scalar::Float64(v) if v.is_nan()));
        assert!(matches!(row_stds.values()[0], Scalar::Float64(v) if v.is_nan()));
        assert!(matches!(row_vars.values()[1], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
        assert!(
            matches!(row_stds.values()[1], Scalar::Float64(v) if (v - 2.0_f64.sqrt()).abs() < 1e-10)
        );
    }

    #[test]
    fn dataframe_apply_row_wise_median() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(3.0),
                        Scalar::Float64(4.0),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "c",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(6.0),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
            ],
        )
        .unwrap();

        let row_medians = df.apply("median", 1).unwrap();
        assert!(matches!(row_medians.values()[0], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
        assert!(matches!(row_medians.values()[1], Scalar::Float64(v) if (v - 4.0).abs() < 1e-10));
        assert!(matches!(row_medians.values()[2], Scalar::Float64(v) if v.is_nan()));
    }

    #[test]
    fn dataframe_apply_row_wise_count_int64() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Null(NullKind::Null),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(3.0),
                        Scalar::Float64(4.0),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "c",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(6.0),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
            ],
        )
        .unwrap();

        let row_counts = df.apply("count", 1).unwrap();
        assert_eq!(row_counts.values()[0], Scalar::Int64(2));
        assert_eq!(row_counts.values()[1], Scalar::Int64(2));
        assert_eq!(row_counts.values()[2], Scalar::Int64(0));
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
        assert!(!s.any().unwrap());
        assert!(s.all().unwrap());
    }

    #[test]
    fn series_any_all_numeric_with_missing() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Int64(0),
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
            ],
        )
        .unwrap();

        assert!(s.any().unwrap());
        assert!(!s.all().unwrap());
    }

    #[test]
    fn series_any_all_all_missing_matches_pandas_skipna_defaults() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Null(NullKind::Null), Scalar::Float64(f64::NAN)],
        )
        .unwrap();

        assert!(!s.any().unwrap());
        assert!(s.all().unwrap());
    }

    #[test]
    fn series_any_all_falsy_values() {
        let s = Series::from_values(
            "vals",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Bool(false),
                Scalar::Bool(false),
                Scalar::Null(NullKind::Null),
            ],
        )
        .unwrap();

        assert!(!s.any().unwrap());
        assert!(!s.all().unwrap());
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
    fn series_sort_index_ascending_orders_labels_and_keeps_values_aligned() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("z"),
                IndexLabel::from("a"),
                IndexLabel::from("m"),
            ],
            vec![Scalar::Int64(30), Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let out = s.sort_index(true).unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from("a"),
                IndexLabel::from("m"),
                IndexLabel::from("z")
            ]
        );
        assert_eq!(
            out.values(),
            &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]
        );
    }

    #[test]
    fn series_sort_index_descending_orders_labels_and_keeps_values_aligned() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::Int64(3),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
            ],
            vec![Scalar::Int64(30), Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let out = s.sort_index(false).unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::Int64(3),
                IndexLabel::Int64(2),
                IndexLabel::Int64(1)
            ]
        );
        assert_eq!(
            out.values(),
            &[Scalar::Int64(30), Scalar::Int64(20), Scalar::Int64(10)]
        );
    }

    #[test]
    fn series_sort_values_numeric_ascending_keeps_missing_last() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
                IndexLabel::from("r5"),
            ],
            vec![
                Scalar::Int64(3),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Float64(f64::NAN),
            ],
        )
        .unwrap();

        let out = s.sort_values(true).unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r5")
            ]
        );
        let expected = [
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(f64::NAN),
        ];
        assert_eq!(out.values().len(), expected.len());
        for (actual, expected) in out.values().iter().zip(expected.iter()) {
            assert!(
                actual.semantic_eq(expected),
                "series_sort_values_numeric_ascending mismatch: actual={actual:?}, expected={expected:?}"
            );
        }
    }

    #[test]
    fn series_sort_values_numeric_descending_keeps_missing_last() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
            ],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let out = s.sort_values(false).unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from("r2"),
                IndexLabel::from("r4"),
                IndexLabel::from("r1"),
                IndexLabel::from("r3")
            ]
        );
        assert_eq!(
            out.values(),
            &[
                Scalar::Int64(3),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null)
            ]
        );
    }

    #[test]
    fn series_sort_values_utf8_ties_preserve_input_order() {
        let s = Series::from_values(
            "vals",
            vec![
                IndexLabel::from("r1"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r4"),
                IndexLabel::from("r5"),
            ],
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("c".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();

        let asc = s.sort_values(true).unwrap();
        assert_eq!(
            asc.index().labels(),
            &[
                IndexLabel::from("r2"),
                IndexLabel::from("r3"),
                IndexLabel::from("r1"),
                IndexLabel::from("r5"),
                IndexLabel::from("r4")
            ]
        );

        let desc = s.sort_values(false).unwrap();
        assert_eq!(
            desc.index().labels(),
            &[
                IndexLabel::from("r4"),
                IndexLabel::from("r1"),
                IndexLabel::from("r5"),
                IndexLabel::from("r2"),
                IndexLabel::from("r3")
            ]
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
    fn dataframe_duplicated_keep_variants() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("y".to_owned()),
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("z".to_owned()),
                    ],
                ),
            ],
        )
        .unwrap();

        let first = df.duplicated(None, DuplicateKeep::First).unwrap();
        assert_eq!(
            first.values(),
            &[
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );

        let last = df.duplicated(None, DuplicateKeep::Last).unwrap();
        assert_eq!(
            last.values(),
            &[
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(false),
                Scalar::Bool(false)
            ]
        );

        let none = df.duplicated(None, DuplicateKeep::None).unwrap();
        assert_eq!(
            none.values(),
            &[
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(false)
            ]
        );
    }

    #[test]
    fn dataframe_duplicated_subset_uses_semantic_missing_equality() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![Scalar::Float64(f64::NAN), Scalar::Null(NullKind::NaN)],
                ),
                ("b", vec![Scalar::Int64(1), Scalar::Int64(1)]),
            ],
        )
        .unwrap();

        let duplicated = df.duplicated(None, DuplicateKeep::First).unwrap();
        assert_eq!(
            duplicated.values(),
            &[Scalar::Bool(false), Scalar::Bool(true)]
        );
    }

    #[test]
    fn dataframe_drop_duplicates_keep_first_preserves_index_order() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("y".to_owned()),
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("z".to_owned()),
                    ],
                ),
            ],
        )
        .unwrap();

        let out = df
            .drop_duplicates(None, DuplicateKeep::First, false)
            .unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from(0_i64),
                IndexLabel::from(2_i64),
                IndexLabel::from(4_i64)
            ]
        );
        assert_eq!(
            out.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(2)]
        );
        assert_eq!(
            out.column("b").unwrap().values(),
            &[
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("z".to_owned())
            ]
        );
    }

    #[test]
    fn dataframe_drop_duplicates_keep_last_with_ignore_index_resets_labels() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("y".to_owned()),
                        Scalar::Utf8("x".to_owned()),
                        Scalar::Utf8("z".to_owned()),
                    ],
                ),
            ],
        )
        .unwrap();

        let out = df.drop_duplicates(None, DuplicateKeep::Last, true).unwrap();
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::from(0_i64),
                IndexLabel::from(1_i64),
                IndexLabel::from(2_i64)
            ]
        );
        assert_eq!(
            out.column("a").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            out.column("b").unwrap().values(),
            &[
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("z".to_owned())
            ]
        );
    }

    #[test]
    fn dataframe_drop_duplicates_subset_keep_none_removes_all_duplicates() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(1),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                    ],
                ),
            ],
        )
        .unwrap();

        let subset = vec!["a".to_owned()];
        let out = df
            .drop_duplicates(Some(&subset), DuplicateKeep::None, false)
            .unwrap();
        assert_eq!(out.index().labels(), &[IndexLabel::from(3_i64)]);
        assert_eq!(out.column("a").unwrap().values(), &[Scalar::Int64(2)]);
        assert_eq!(out.column("b").unwrap().values(), &[Scalar::Int64(20)]);
    }

    #[test]
    fn dataframe_drop_duplicates_rejects_missing_subset_column() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(1)])],
        )
        .unwrap();

        let subset = vec!["missing".to_owned()];
        let err = df
            .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
            .unwrap_err();
        assert!(
            matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("column 'missing' not found"))
        );
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
    fn dataframe_count_counts_non_missing_per_column() {
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
                        Scalar::Null(NullKind::NaN),
                        Scalar::Int64(2),
                    ],
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
        )
        .unwrap();

        let counts = df.count().unwrap();
        assert_eq!(
            counts.index().labels(),
            &[
                IndexLabel::Utf8("a".to_owned()),
                IndexLabel::Utf8("b".to_owned()),
                IndexLabel::Utf8("c".to_owned())
            ]
        );
        assert_eq!(
            counts.values(),
            &[Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(3)]
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
    fn dataframe_reset_index_supports_mixed_index_label_types() {
        let df = DataFrame::from_dict_with_index(
            vec![("v", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![IndexLabel::from("row-1"), IndexLabel::from(2_i64)],
        )
        .unwrap();

        let out = df.reset_index(false).unwrap();
        assert_eq!(
            out.index().labels(),
            &[IndexLabel::from(0_i64), IndexLabel::from(1_i64)]
        );
        assert_eq!(
            out.column("index").unwrap().values(),
            &[
                Scalar::Utf8("row-1".to_owned()),
                Scalar::Utf8("2".to_owned())
            ]
        );
        assert_eq!(
            out.column("v").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
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
    fn dataframe_astype_columns_casts_multiple_targets_and_preserves_order() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                (
                    "c",
                    vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())],
                ),
            ],
        )
        .unwrap();

        let casted = df
            .astype_columns(&[("a", DType::Float64), ("b", DType::Float64)])
            .unwrap();

        let names = casted
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        assert_eq!(casted.column("a").unwrap().dtype(), DType::Float64);
        assert_eq!(casted.column("b").unwrap().dtype(), DType::Float64);
        assert_eq!(casted.column("c").unwrap().dtype(), DType::Utf8);
        assert_eq!(
            casted.column("b").unwrap().values(),
            &[Scalar::Float64(10.0), Scalar::Float64(20.0)]
        );
    }

    #[test]
    fn dataframe_astype_columns_rejects_duplicate_or_missing_selectors() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![(
                "a",
                vec![Scalar::Utf8("1".to_owned()), Scalar::Utf8("2".to_owned())],
            )],
        )
        .unwrap();

        let duplicate = df
            .astype_columns(&[("a", DType::Int64), ("a", DType::Float64)])
            .unwrap_err();
        assert!(
            matches!(duplicate, FrameError::CompatibilityRejected(msg) if msg.contains("duplicate astype selector"))
        );

        let missing = df.astype_columns(&[("missing", DType::Int64)]).unwrap_err();
        assert!(
            matches!(missing, FrameError::CompatibilityRejected(msg) if msg.contains("column 'missing' not found"))
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

    #[test]
    fn dataframe_describe_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.0),
                        Scalar::Float64(3.0),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Float64(4.0),
                        Scalar::Float64(5.0),
                        Scalar::Float64(6.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let desc = df.describe().unwrap();
        assert_eq!(desc.len(), 8); // count, mean, std, min, 25%, 50%, 75%, max
        assert_eq!(desc.num_columns(), 2);

        // Check count row for column "a"
        let a_col = desc.column("a").unwrap();
        assert_eq!(a_col.values()[0], Scalar::Float64(3.0)); // count
        assert_eq!(a_col.values()[1], Scalar::Float64(2.0)); // mean
        assert_eq!(a_col.values()[3], Scalar::Float64(1.0)); // min
        assert_eq!(a_col.values()[7], Scalar::Float64(3.0)); // max
    }

    #[test]
    fn dataframe_describe_skips_non_numeric() {
        let df = DataFrame::from_dict(
            &["nums", "strs"],
            vec![
                ("nums", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                (
                    "strs",
                    vec![Scalar::Utf8("a".to_owned()), Scalar::Utf8("b".to_owned())],
                ),
            ],
        )
        .unwrap();

        let desc = df.describe().unwrap();
        assert_eq!(desc.num_columns(), 1); // only "nums"
        assert!(desc.column("strs").is_none());
    }

    #[test]
    fn dataframe_transpose_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let t = df.transpose().unwrap();
        // Original: 2 rows x 2 cols -> Transposed: 2 rows (a, b) x 2 cols (0, 1)
        assert_eq!(t.len(), 2);
        assert_eq!(t.num_columns(), 2);
        assert_eq!(
            t.index().labels(),
            &[
                IndexLabel::Utf8("a".to_owned()),
                IndexLabel::Utf8("b".to_owned())
            ]
        );
        let col0 = t.column("0").unwrap();
        assert_eq!(col0.values(), &[Scalar::Int64(1), Scalar::Int64(3)]);
        let col1 = t.column("1").unwrap();
        assert_eq!(col1.values(), &[Scalar::Int64(2), Scalar::Int64(4)]);
    }

    // --- Series::where_cond tests ---

    #[test]
    fn series_where_cond_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        let cond = Series::from_values(
            "c",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
        )
        .unwrap();

        let result = s.where_cond(&cond, None).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1));
        assert!(result.values()[1].is_missing());
        assert_eq!(result.values()[2], Scalar::Int64(3));
    }

    #[test]
    fn series_where_cond_with_other() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let cond = Series::from_values(
            "c",
            vec!["a".into(), "b".into()],
            vec![Scalar::Bool(false), Scalar::Bool(true)],
        )
        .unwrap();

        let result = s.where_cond(&cond, Some(&Scalar::Int64(-1))).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(-1));
        assert_eq!(result.values()[1], Scalar::Int64(20));
    }

    #[test]
    fn series_where_cond_null_propagation() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();

        let cond = Series::from_values(
            "c",
            vec!["a".into(), "b".into()],
            vec![Scalar::Null(NullKind::Null), Scalar::Bool(true)],
        )
        .unwrap();

        let result = s.where_cond(&cond, None).unwrap();
        assert!(result.values()[0].is_missing());
        assert_eq!(result.values()[1], Scalar::Int64(2));
    }

    // --- Series::mask tests ---

    #[test]
    fn series_mask_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        let cond = Series::from_values(
            "c",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
        )
        .unwrap();

        let result = s.mask(&cond, None).unwrap();
        assert!(result.values()[0].is_missing()); // True -> replaced
        assert_eq!(result.values()[1], Scalar::Int64(2)); // False -> kept
        assert!(result.values()[2].is_missing()); // True -> replaced
    }

    #[test]
    fn series_mask_with_other() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let cond = Series::from_values(
            "c",
            vec!["a".into(), "b".into()],
            vec![Scalar::Bool(true), Scalar::Bool(false)],
        )
        .unwrap();

        let result = s.mask(&cond, Some(&Scalar::Int64(0))).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(0)); // True -> replaced with 0
        assert_eq!(result.values()[1], Scalar::Int64(20)); // False -> kept
    }

    // --- Series::isin tests ---

    #[test]
    fn series_isin_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into(), "d".into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .unwrap();

        let result = s.isin(&[Scalar::Int64(2), Scalar::Int64(4)]).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(false));
        assert_eq!(result.values()[3], Scalar::Bool(true));
    }

    #[test]
    fn series_isin_with_strings() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Utf8("apple".to_owned()),
                Scalar::Utf8("banana".to_owned()),
                Scalar::Utf8("cherry".to_owned()),
            ],
        )
        .unwrap();

        let result = s
            .isin(&[
                Scalar::Utf8("banana".to_owned()),
                Scalar::Utf8("date".to_owned()),
            ])
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(false));
    }

    #[test]
    fn series_isin_null_handling() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
            ],
        )
        .unwrap();

        // NaN not in test values -> False for null element
        let result = s.isin(&[Scalar::Int64(1)]).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
        assert_eq!(result.values()[2], Scalar::Bool(false));

        // NaN in test values -> True for null element
        let result_with_nan = s.isin(&[Scalar::Int64(1), Scalar::Null(NullKind::NaN)]).unwrap();
        assert_eq!(result_with_nan.values()[1], Scalar::Bool(true));
    }

    #[test]
    fn series_isin_cross_type() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(2), Scalar::Float64(3.0)],
        )
        .unwrap();

        let result = s.isin(&[Scalar::Float64(2.0), Scalar::Int64(3)]).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(true));
    }

    // --- Series::between tests ---

    #[test]
    fn series_between_both_inclusive() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
                Scalar::Int64(5),
            ],
        )
        .unwrap();

        let result = s
            .between(&Scalar::Int64(2), &Scalar::Int64(4), "both")
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false)); // 1
        assert_eq!(result.values()[1], Scalar::Bool(true)); // 2 (boundary)
        assert_eq!(result.values()[2], Scalar::Bool(true)); // 3
        assert_eq!(result.values()[3], Scalar::Bool(true)); // 4 (boundary)
        assert_eq!(result.values()[4], Scalar::Bool(false)); // 5
    }

    #[test]
    fn series_between_neither_inclusive() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
        )
        .unwrap();

        let result = s
            .between(&Scalar::Int64(2), &Scalar::Int64(4), "neither")
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false)); // 2 == left
        assert_eq!(result.values()[1], Scalar::Bool(true)); // 3 is strictly between
        assert_eq!(result.values()[2], Scalar::Bool(false)); // 4 == right
    }

    #[test]
    fn series_between_left_inclusive() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
        )
        .unwrap();

        let result = s
            .between(&Scalar::Int64(2), &Scalar::Int64(4), "left")
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true)); // 2 == left -> included
        assert_eq!(result.values()[1], Scalar::Bool(true)); // 3 in range
        assert_eq!(result.values()[2], Scalar::Bool(false)); // 4 == right -> excluded
    }

    #[test]
    fn series_between_null_produces_false() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Null(NullKind::NaN), Scalar::Int64(3)],
        )
        .unwrap();

        let result = s
            .between(&Scalar::Int64(1), &Scalar::Int64(5), "both")
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false)); // NaN -> false
        assert_eq!(result.values()[1], Scalar::Bool(true));
    }

    #[test]
    fn series_between_float_range() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Float64(1.5),
                Scalar::Float64(2.5),
                Scalar::Float64(3.5),
            ],
        )
        .unwrap();

        let result = s
            .between(&Scalar::Float64(2.0), &Scalar::Float64(3.0), "both")
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(false));
    }

    // --- Series::idxmin / idxmax tests ---

    #[test]
    fn series_idxmin_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        assert_eq!(s.idxmin().unwrap(), IndexLabel::Utf8("b".to_owned()));
    }

    #[test]
    fn series_idxmax_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        assert_eq!(s.idxmax().unwrap(), IndexLabel::Utf8("a".to_owned()));
    }

    #[test]
    fn series_idxmin_skips_nulls() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(5),
                Scalar::Int64(2),
            ],
        )
        .unwrap();
        assert_eq!(s.idxmin().unwrap(), IndexLabel::Utf8("c".to_owned()));
    }

    #[test]
    fn series_idxmin_all_null() {
        let s = Series::from_values(
            "x",
            vec!["a".into()],
            vec![Scalar::Null(NullKind::NaN)],
        )
        .unwrap();
        assert!(s.idxmin().is_err());
    }

    // --- Series::nlargest / nsmallest tests ---

    #[test]
    fn series_nlargest_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into(), "d".into()],
            vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(4),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let result = s.nlargest(2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(4)); // largest
        assert_eq!(result.values()[1], Scalar::Int64(3)); // second largest
        assert_eq!(
            result.index().labels()[0],
            IndexLabel::Utf8("c".to_owned())
        );
        assert_eq!(
            result.index().labels()[1],
            IndexLabel::Utf8("a".to_owned())
        );
    }

    #[test]
    fn series_nsmallest_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into(), "d".into()],
            vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(4),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let result = s.nsmallest(2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(1));
        assert_eq!(result.values()[1], Scalar::Int64(2));
    }

    #[test]
    fn series_nlargest_with_nulls() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(5),
                Scalar::Int64(3),
            ],
        )
        .unwrap();

        let result = s.nlargest(2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(5));
        assert_eq!(result.values()[1], Scalar::Int64(3));
    }

    #[test]
    fn series_nlargest_n_exceeds_length() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(2), Scalar::Int64(1)],
        )
        .unwrap();

        let result = s.nlargest(10).unwrap();
        assert_eq!(result.len(), 2);
    }

    // --- Series::pct_change tests ---

    #[test]
    fn series_pct_change_basic() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into(), "d".into()],
            vec![
                Scalar::Float64(100.0),
                Scalar::Float64(110.0),
                Scalar::Float64(99.0),
                Scalar::Float64(120.0),
            ],
        )
        .unwrap();

        let result = s.pct_change(1).unwrap();
        assert!(result.values()[0].is_missing()); // first is NaN
        let v1 = result.values()[1].to_f64().unwrap();
        assert!((v1 - 0.1).abs() < 1e-10); // (110-100)/100 = 0.1
        let v2 = result.values()[2].to_f64().unwrap();
        assert!((v2 - (-0.1)).abs() < 1e-10); // (99-110)/110 = -0.1
    }

    #[test]
    fn series_pct_change_periods_2() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Float64(100.0),
                Scalar::Float64(200.0),
                Scalar::Float64(150.0),
            ],
        )
        .unwrap();

        let result = s.pct_change(2).unwrap();
        assert!(result.values()[0].is_missing());
        assert!(result.values()[1].is_missing());
        let v2 = result.values()[2].to_f64().unwrap();
        assert!((v2 - 0.5).abs() < 1e-10); // (150-100)/100 = 0.5
    }

    #[test]
    fn series_pct_change_with_nulls() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                Scalar::Float64(100.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(200.0),
            ],
        )
        .unwrap();

        let result = s.pct_change(1).unwrap();
        assert!(result.values()[1].is_missing()); // null previous -> NaN
        assert!(result.values()[2].is_missing()); // null at i-1 -> NaN
    }

    // --- Series::to_frame / to_list / to_dict tests ---

    #[test]
    fn series_to_frame() {
        let s = Series::from_values(
            "vals",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();

        let df = s.to_frame(None).unwrap();
        assert_eq!(df.num_columns(), 1);
        assert!(df.column("vals").is_some());
        assert_eq!(df.column("vals").unwrap().values(), s.values());
    }

    #[test]
    fn series_to_frame_custom_name() {
        let s = Series::from_values(
            "vals",
            vec!["a".into()],
            vec![Scalar::Int64(42)],
        )
        .unwrap();

        let df = s.to_frame(Some("custom")).unwrap();
        assert!(df.column("custom").is_some());
        assert!(df.column("vals").is_none());
    }

    #[test]
    fn series_to_list() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let list = s.to_list();
        assert_eq!(list, vec![Scalar::Int64(10), Scalar::Int64(20)]);
    }

    #[test]
    fn series_to_dict() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let dict = s.to_dict();
        assert_eq!(dict.len(), 2);
        assert_eq!(dict[0], (IndexLabel::Utf8("a".to_owned()), Scalar::Int64(10)));
        assert_eq!(dict[1], (IndexLabel::Utf8("b".to_owned()), Scalar::Int64(20)));
    }

    // --- DataFrame::where_cond / mask tests ---

    #[test]
    fn dataframe_where_cond_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let cond = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Bool(true), Scalar::Bool(false)]),
                ("b", vec![Scalar::Bool(false), Scalar::Bool(true)]),
            ],
        )
        .unwrap();

        let result = df.where_cond(&cond, None).unwrap();
        assert_eq!(result.column("a").unwrap().values()[0], Scalar::Int64(1));
        assert!(result.column("a").unwrap().values()[1].is_missing());
        assert!(result.column("b").unwrap().values()[0].is_missing());
        assert_eq!(result.column("b").unwrap().values()[1], Scalar::Int64(4));
    }

    #[test]
    fn dataframe_where_cond_with_fill() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(10), Scalar::Int64(20)])],
        )
        .unwrap();

        let cond = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Bool(false), Scalar::Bool(true)])],
        )
        .unwrap();

        let result = df.where_cond(&cond, Some(&Scalar::Int64(-1))).unwrap();
        assert_eq!(result.column("x").unwrap().values()[0], Scalar::Int64(-1));
        assert_eq!(result.column("x").unwrap().values()[1], Scalar::Int64(20));
    }

    #[test]
    fn dataframe_mask_basic() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let cond = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Bool(true), Scalar::Bool(false)])],
        )
        .unwrap();

        let result = df.mask(&cond, Some(&Scalar::Int64(0))).unwrap();
        assert_eq!(result.column("a").unwrap().values()[0], Scalar::Int64(0)); // True -> replaced
        assert_eq!(result.column("a").unwrap().values()[1], Scalar::Int64(2)); // False -> kept
    }

    // --- DataFrame::iterrows / itertuples / items tests ---

    #[test]
    fn dataframe_iterrows_basic() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("y", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let rows = df.iterrows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1.len(), 2); // two columns
        assert_eq!(rows[0].1[0].0, "x");
        assert_eq!(rows[0].1[0].1, Scalar::Int64(1));
        assert_eq!(rows[0].1[1].0, "y");
        assert_eq!(rows[0].1[1].1, Scalar::Int64(3));
    }

    #[test]
    fn dataframe_itertuples_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ("b", vec![Scalar::Int64(30), Scalar::Int64(40)]),
            ],
        )
        .unwrap();

        let tuples = df.itertuples();
        assert_eq!(tuples.len(), 2);
        assert_eq!(tuples[0].1, vec![Scalar::Int64(10), Scalar::Int64(30)]);
        assert_eq!(tuples[1].1, vec![Scalar::Int64(20), Scalar::Int64(40)]);
    }

    #[test]
    fn dataframe_items_basic() {
        let df = DataFrame::from_dict(
            &["col1", "col2"],
            vec![
                ("col1", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("col2", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let items = df.items().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, "col1");
        assert_eq!(items[0].1.values(), &[Scalar::Int64(1), Scalar::Int64(2)]);
        assert_eq!(items[1].0, "col2");
    }

    // --- DataFrame::assign / pipe tests ---

    #[test]
    fn dataframe_assign_new_column() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let new_col = fp_columnar::Column::from_values(vec![
            Scalar::Int64(10),
            Scalar::Int64(20),
        ])
        .unwrap();

        let result = df.assign(vec![("b", new_col)]).unwrap();
        assert_eq!(result.num_columns(), 2);
        assert!(result.column("b").is_some());
        assert_eq!(
            result.column("b").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
    }

    #[test]
    fn dataframe_assign_overwrite_column() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let new_col = fp_columnar::Column::from_values(vec![
            Scalar::Int64(99),
            Scalar::Int64(100),
        ])
        .unwrap();

        let result = df.assign(vec![("a", new_col)]).unwrap();
        assert_eq!(result.num_columns(), 1);
        assert_eq!(
            result.column("a").unwrap().values(),
            &[Scalar::Int64(99), Scalar::Int64(100)]
        );
    }

    #[test]
    fn dataframe_assign_length_mismatch() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let bad_col = fp_columnar::Column::from_values(vec![Scalar::Int64(1)]).unwrap();
        assert!(df.assign(vec![("b", bad_col)]).is_err());
    }

    #[test]
    fn dataframe_pipe_basic() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let result = df
            .pipe(|d| {
                let col = fp_columnar::Column::from_values(vec![
                    Scalar::Int64(10),
                    Scalar::Int64(20),
                ])?;
                d.assign(vec![("b", col)])
            })
            .unwrap();

        assert_eq!(result.num_columns(), 2);
        assert!(result.column("b").is_some());
    }

    // --- Rolling window tests ---

    #[test]
    fn rolling_mean_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ],
        )
        .unwrap();

        let result = s.rolling(3, None).mean().unwrap();
        // First 2 should be NaN (window not full)
        assert!(result.values()[0].is_missing());
        assert!(result.values()[1].is_missing());
        // rolling(3).mean() at index 2 = (1+2+3)/3 = 2.0
        assert_eq!(result.values()[2], Scalar::Float64(2.0));
        // at index 3 = (2+3+4)/3 = 3.0
        assert_eq!(result.values()[3], Scalar::Float64(3.0));
        // at index 4 = (3+4+5)/3 = 4.0
        assert_eq!(result.values()[4], Scalar::Float64(4.0));
    }

    #[test]
    fn rolling_sum_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
            ],
        )
        .unwrap();

        let result = s.rolling(2, None).sum().unwrap();
        assert!(result.values()[0].is_missing());
        assert_eq!(result.values()[1], Scalar::Float64(30.0)); // 10+20
        assert_eq!(result.values()[2], Scalar::Float64(50.0)); // 20+30
    }

    #[test]
    fn rolling_min_max() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(4.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();

        let mins = s.rolling(2, None).min().unwrap();
        assert!(mins.values()[0].is_missing());
        assert_eq!(mins.values()[1], Scalar::Float64(1.0)); // min(3,1)
        assert_eq!(mins.values()[2], Scalar::Float64(1.0)); // min(1,4)
        assert_eq!(mins.values()[3], Scalar::Float64(2.0)); // min(4,2)

        let maxs = s.rolling(2, None).max().unwrap();
        assert_eq!(maxs.values()[1], Scalar::Float64(3.0)); // max(3,1)
        assert_eq!(maxs.values()[2], Scalar::Float64(4.0)); // max(1,4)
        assert_eq!(maxs.values()[3], Scalar::Float64(4.0)); // max(4,2)
    }

    #[test]
    fn rolling_with_nulls() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        // min_periods=2 means we need at least 2 non-null in the window
        let result = s.rolling(3, Some(2)).mean().unwrap();
        assert!(result.values()[0].is_missing()); // window too small
        assert!(result.values()[1].is_missing()); // window too small
        // window [1, NaN, 3] -> non-null: [1,3] -> mean = 2.0
        assert_eq!(result.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn rolling_std() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let result = s.rolling(3, None).std().unwrap();
        assert!(result.values()[0].is_missing());
        assert!(result.values()[1].is_missing());
        // std of [2,4,6] = 2.0
        let std_val = result.values()[2].to_f64().unwrap();
        assert!((std_val - 2.0).abs() < 1e-10);
    }

    // --- Expanding window tests ---

    #[test]
    fn expanding_sum_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        let result = s.expanding(None).sum().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(3.0)); // 1+2
        assert_eq!(result.values()[2], Scalar::Float64(6.0)); // 1+2+3
        assert_eq!(result.values()[3], Scalar::Float64(10.0)); // 1+2+3+4
    }

    #[test]
    fn expanding_mean_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let result = s.expanding(None).mean().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(2.0)); // 2/1
        assert_eq!(result.values()[1], Scalar::Float64(3.0)); // (2+4)/2
        assert_eq!(result.values()[2], Scalar::Float64(4.0)); // (2+4+6)/3
    }

    #[test]
    fn expanding_min_periods() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();

        let result = s.expanding(Some(2)).sum().unwrap();
        assert!(result.values()[0].is_missing()); // only 1 element, need 2
        assert_eq!(result.values()[1], Scalar::Float64(3.0)); // 1+2
        assert_eq!(result.values()[2], Scalar::Float64(6.0)); // 1+2+3
    }

    #[test]
    fn expanding_with_nulls() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();

        let result = s.expanding(None).sum().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(1.0)); // null skipped
        assert_eq!(result.values()[2], Scalar::Float64(4.0)); // 1+3
    }

    // ── rank tests ──

    #[test]
    fn series_rank_average() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("average", true, "keep").unwrap();
        // 1.0 ties at positions 1,2 → average rank (1+2)/2 = 1.5
        assert_eq!(ranked.values()[0], Scalar::Float64(4.0)); // 3.0 → rank 4
        assert_eq!(ranked.values()[1], Scalar::Float64(1.5)); // 1.0 → avg rank
        assert_eq!(ranked.values()[2], Scalar::Float64(1.5)); // 1.0 → avg rank
        assert_eq!(ranked.values()[3], Scalar::Float64(3.0)); // 2.0 → rank 3
    }

    #[test]
    fn series_rank_min_max() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(1.0),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();
        let min_ranked = s.rank("min", true, "keep").unwrap();
        assert_eq!(min_ranked.values()[0], Scalar::Float64(3.0)); // 2.0 → rank 3
        assert_eq!(min_ranked.values()[1], Scalar::Float64(1.0)); // 1.0 → min rank 1
        assert_eq!(min_ranked.values()[2], Scalar::Float64(1.0)); // 1.0 → min rank 1

        let max_ranked = s.rank("max", true, "keep").unwrap();
        assert_eq!(max_ranked.values()[0], Scalar::Float64(3.0)); // 2.0 → rank 3
        assert_eq!(max_ranked.values()[1], Scalar::Float64(2.0)); // 1.0 → max rank 2
        assert_eq!(max_ranked.values()[2], Scalar::Float64(2.0)); // 1.0 → max rank 2
    }

    #[test]
    fn series_rank_first() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(1.0),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("first", true, "keep").unwrap();
        assert_eq!(ranked.values()[0], Scalar::Float64(3.0)); // 2.0 → rank 3
        assert_eq!(ranked.values()[1], Scalar::Float64(1.0)); // first 1.0 → rank 1
        assert_eq!(ranked.values()[2], Scalar::Float64(2.0)); // second 1.0 → rank 2
    }

    #[test]
    fn series_rank_dense() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("dense", true, "keep").unwrap();
        assert_eq!(ranked.values()[0], Scalar::Float64(3.0)); // 3.0 → dense rank 3
        assert_eq!(ranked.values()[1], Scalar::Float64(1.0)); // 1.0 → dense rank 1
        assert_eq!(ranked.values()[2], Scalar::Float64(1.0)); // 1.0 → dense rank 1
        assert_eq!(ranked.values()[3], Scalar::Float64(2.0)); // 2.0 → dense rank 2
    }

    #[test]
    fn series_rank_descending() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("average", false, "keep").unwrap();
        assert_eq!(ranked.values()[0], Scalar::Float64(3.0)); // 1.0 → rank 3 (desc)
        assert_eq!(ranked.values()[1], Scalar::Float64(2.0)); // 2.0 → rank 2
        assert_eq!(ranked.values()[2], Scalar::Float64(1.0)); // 3.0 → rank 1
    }

    #[test]
    fn series_rank_na_keep() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("average", true, "keep").unwrap();
        assert_eq!(ranked.values()[0], Scalar::Float64(2.0));
        assert!(ranked.values()[1].is_missing()); // NaN stays NaN
        assert_eq!(ranked.values()[2], Scalar::Float64(1.0));
    }

    #[test]
    fn series_rank_na_top() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("average", true, "top").unwrap();
        // NaN gets rank 1 (top), others shift up
        assert_eq!(ranked.values()[0], Scalar::Float64(3.0)); // 2.0 → rank 3
        assert_eq!(ranked.values()[1], Scalar::Float64(1.0)); // NaN → rank 1 (top)
        assert_eq!(ranked.values()[2], Scalar::Float64(2.0)); // 1.0 → rank 2
    }

    #[test]
    fn series_rank_na_bottom() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();
        let ranked = s.rank("average", true, "bottom").unwrap();
        assert_eq!(ranked.values()[0], Scalar::Float64(2.0)); // 2.0 → rank 2
        assert_eq!(ranked.values()[1], Scalar::Float64(3.0)); // NaN → rank 3 (bottom)
        assert_eq!(ranked.values()[2], Scalar::Float64(1.0)); // 1.0 → rank 1
    }

    #[test]
    fn dataframe_rank_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(3.0),
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(20.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let ranked = df.rank("average", true, "keep").unwrap();
        // Column "a": 3.0→3, 1.0→1, 2.0→2
        assert_eq!(ranked.columns["a"].values()[0], Scalar::Float64(3.0));
        assert_eq!(ranked.columns["a"].values()[1], Scalar::Float64(1.0));
        assert_eq!(ranked.columns["a"].values()[2], Scalar::Float64(2.0));
        // Column "b": 10.0→1, 30.0→3, 20.0→2
        assert_eq!(ranked.columns["b"].values()[0], Scalar::Float64(1.0));
        assert_eq!(ranked.columns["b"].values()[1], Scalar::Float64(3.0));
        assert_eq!(ranked.columns["b"].values()[2], Scalar::Float64(2.0));
    }

    // ── melt tests ──

    #[test]
    fn dataframe_melt_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "id",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
            )
            .unwrap(),
            Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
            Series::from_values(
                "y",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(3.0), Scalar::Float64(4.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let melted = df.melt(&["id"], &["x", "y"], None, None).unwrap();
        assert_eq!(melted.index.len(), 4); // 2 rows * 2 value_vars
        assert_eq!(melted.column_order.len(), 3); // id, variable, value

        // id column repeated: a,b,a,b
        assert_eq!(melted.columns["id"].values()[0], Scalar::Utf8("a".into()));
        assert_eq!(melted.columns["id"].values()[1], Scalar::Utf8("b".into()));
        assert_eq!(melted.columns["id"].values()[2], Scalar::Utf8("a".into()));
        assert_eq!(melted.columns["id"].values()[3], Scalar::Utf8("b".into()));

        // variable column: x,x,y,y
        assert_eq!(
            melted.columns["variable"].values()[0],
            Scalar::Utf8("x".into())
        );
        assert_eq!(
            melted.columns["variable"].values()[1],
            Scalar::Utf8("x".into())
        );
        assert_eq!(
            melted.columns["variable"].values()[2],
            Scalar::Utf8("y".into())
        );
        assert_eq!(
            melted.columns["variable"].values()[3],
            Scalar::Utf8("y".into())
        );

        // value column: 1,2,3,4
        assert_eq!(melted.columns["value"].values()[0], Scalar::Float64(1.0));
        assert_eq!(melted.columns["value"].values()[1], Scalar::Float64(2.0));
        assert_eq!(melted.columns["value"].values()[2], Scalar::Float64(3.0));
        assert_eq!(melted.columns["value"].values()[3], Scalar::Float64(4.0));
    }

    #[test]
    fn dataframe_melt_auto_value_vars() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "id",
                vec![0_i64.into()],
                vec![Scalar::Utf8("a".into())],
            )
            .unwrap(),
            Series::from_values(
                "val1",
                vec![0_i64.into()],
                vec![Scalar::Float64(10.0)],
            )
            .unwrap(),
            Series::from_values(
                "val2",
                vec![0_i64.into()],
                vec![Scalar::Float64(20.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        // Empty value_vars → uses all non-id columns
        let melted = df.melt(&["id"], &[], None, None).unwrap();
        assert_eq!(melted.index.len(), 2); // 1 row * 2 value_vars
    }

    #[test]
    fn dataframe_melt_custom_names() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "key",
                vec![0_i64.into()],
                vec![Scalar::Utf8("k".into())],
            )
            .unwrap(),
            Series::from_values(
                "a",
                vec![0_i64.into()],
                vec![Scalar::Float64(1.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let melted = df
            .melt(&["key"], &["a"], Some("col_name"), Some("col_value"))
            .unwrap();
        assert!(melted.columns.contains_key("col_name"));
        assert!(melted.columns.contains_key("col_value"));
    }

    // ── pivot_table tests ──

    #[test]
    fn dataframe_pivot_table_sum() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "row",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Utf8("r1".into()),
                    Scalar::Utf8("r1".into()),
                    Scalar::Utf8("r2".into()),
                    Scalar::Utf8("r2".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "col",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Utf8("c1".into()),
                    Scalar::Utf8("c2".into()),
                    Scalar::Utf8("c1".into()),
                    Scalar::Utf8("c2".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let pivoted = df.pivot_table("val", "row", "col", "sum").unwrap();
        assert_eq!(pivoted.index.len(), 2); // r1, r2
        assert_eq!(pivoted.column_order.len(), 2); // c1, c2

        // r1, c1 → 1.0; r1, c2 → 2.0; r2, c1 → 3.0; r2, c2 → 4.0
        assert_eq!(pivoted.columns["c1"].values()[0], Scalar::Float64(1.0));
        assert_eq!(pivoted.columns["c2"].values()[0], Scalar::Float64(2.0));
        assert_eq!(pivoted.columns["c1"].values()[1], Scalar::Float64(3.0));
        assert_eq!(pivoted.columns["c2"].values()[1], Scalar::Float64(4.0));
    }

    #[test]
    fn dataframe_pivot_table_mean() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "row",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Utf8("r1".into()),
                    Scalar::Utf8("r1".into()),
                    Scalar::Utf8("r1".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "col",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Utf8("c1".into()),
                    Scalar::Utf8("c1".into()),
                    Scalar::Utf8("c2".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let pivoted = df.pivot_table("val", "row", "col", "mean").unwrap();
        // r1, c1 → mean(10, 20) = 15.0; r1, c2 → 30.0
        assert_eq!(pivoted.columns["c1"].values()[0], Scalar::Float64(15.0));
        assert_eq!(pivoted.columns["c2"].values()[0], Scalar::Float64(30.0));
    }

    #[test]
    fn dataframe_pivot_table_missing_cell() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "row",
                vec![0_i64.into(), 1_i64.into()],
                vec![
                    Scalar::Utf8("r1".into()),
                    Scalar::Utf8("r2".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "col",
                vec![0_i64.into(), 1_i64.into()],
                vec![
                    Scalar::Utf8("c1".into()),
                    Scalar::Utf8("c2".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(5.0), Scalar::Float64(10.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let pivoted = df.pivot_table("val", "row", "col", "sum").unwrap();
        // r1 has c1 but not c2; r2 has c2 but not c1
        assert_eq!(pivoted.columns["c1"].values()[0], Scalar::Float64(5.0));
        assert!(pivoted.columns["c2"].values()[0].is_missing()); // missing cell → NaN
        assert!(pivoted.columns["c1"].values()[1].is_missing()); // missing cell → NaN
        assert_eq!(pivoted.columns["c2"].values()[1], Scalar::Float64(10.0));
    }

    // ── agg tests ──

    #[test]
    fn dataframe_agg_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let mut funcs = std::collections::HashMap::new();
        funcs.insert("a".to_string(), vec!["sum".to_string(), "mean".to_string()]);
        funcs.insert("b".to_string(), vec!["sum".to_string(), "mean".to_string()]);

        let result = df.agg(&funcs).unwrap();
        assert_eq!(result.index.len(), 2); // sum, mean
        // a: sum=6, mean=2
        assert_eq!(result.columns["a"].values()[0], Scalar::Float64(6.0));
        assert_eq!(result.columns["a"].values()[1], Scalar::Float64(2.0));
        // b: sum=60, mean=20
        assert_eq!(result.columns["b"].values()[0], Scalar::Float64(60.0));
        assert_eq!(result.columns["b"].values()[1], Scalar::Float64(20.0));
    }

    // ── applymap tests ──

    #[test]
    fn dataframe_applymap_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(4.0)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(9.0), Scalar::Float64(16.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df
            .applymap(|v| match v.to_f64() {
                Ok(f) => Scalar::Float64(f.sqrt()),
                Err(_) => v.clone(),
            })
            .unwrap();

        assert_eq!(result.columns["a"].values()[0], Scalar::Float64(1.0));
        assert_eq!(result.columns["a"].values()[1], Scalar::Float64(2.0));
        assert_eq!(result.columns["b"].values()[0], Scalar::Float64(3.0));
        assert_eq!(result.columns["b"].values()[1], Scalar::Float64(4.0));
    }

    // ── transform tests ──

    #[test]
    fn dataframe_transform_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(2.0), Scalar::Float64(3.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df
            .transform(|v| match v.to_f64() {
                Ok(f) => Scalar::Float64(f * 10.0),
                Err(_) => v.clone(),
            })
            .unwrap();
        assert_eq!(result.columns["x"].values()[0], Scalar::Float64(20.0));
        assert_eq!(result.columns["x"].values()[1], Scalar::Float64(30.0));
    }

    // ── corr/cov tests ──

    #[test]
    fn series_corr_perfect() {
        let a = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();
        let b = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let r = a.corr(&b).unwrap();
        assert!((r - 1.0).abs() < 1e-10);
    }

    #[test]
    fn series_corr_negative() {
        let a = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();
        let b = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(2.0),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();

        let r = a.corr(&b).unwrap();
        assert!((r - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn series_cov_basic() {
        let a = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();
        let b = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let c = a.cov_with(&b).unwrap();
        // cov(a,b) = sum((ai-mean_a)*(bi-mean_b))/(n-1) = ((-1)*(-2)+0*0+1*2)/2 = 4/2 = 2
        assert!((c - 2.0).abs() < 1e-10);
    }

    #[test]
    fn dataframe_corr_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(2.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(6.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let corr_matrix = df.corr().unwrap();
        assert_eq!(corr_matrix.index.len(), 2);
        // Diagonal should be 1.0
        assert!((corr_matrix.columns["a"].values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
        assert!((corr_matrix.columns["b"].values()[1].to_f64().unwrap() - 1.0).abs() < 1e-10);
        // Off-diagonal: perfect positive corr
        assert!((corr_matrix.columns["b"].values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn dataframe_cov_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(2.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(6.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let cov_matrix = df.cov().unwrap();
        // cov(a,a) = var(a) = 1.0; cov(b,b) = var(b) = 4.0; cov(a,b) = 2.0
        assert!((cov_matrix.columns["a"].values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
        assert!((cov_matrix.columns["b"].values()[1].to_f64().unwrap() - 4.0).abs() < 1e-10);
        assert!((cov_matrix.columns["a"].values()[1].to_f64().unwrap() - 2.0).abs() < 1e-10);
    }

    // ── nlargest/nsmallest tests ──

    #[test]
    fn dataframe_nlargest_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(40.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let top2 = df.nlargest(2, "val").unwrap();
        assert_eq!(top2.len(), 2);
        assert_eq!(top2.columns["val"].values()[0], Scalar::Float64(40.0));
        assert_eq!(top2.columns["val"].values()[1], Scalar::Float64(30.0));
    }

    #[test]
    fn dataframe_nsmallest_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(40.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let bottom2 = df.nsmallest(2, "val").unwrap();
        assert_eq!(bottom2.len(), 2);
        assert_eq!(bottom2.columns["val"].values()[0], Scalar::Float64(10.0));
        assert_eq!(bottom2.columns["val"].values()[1], Scalar::Float64(20.0));
    }

    // ── reindex test ──

    #[test]
    fn dataframe_reindex_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let new_labels = vec![2_i64.into(), 0_i64.into(), 5_i64.into()];
        let reindexed = df.reindex(new_labels).unwrap();
        assert_eq!(reindexed.len(), 3);
        assert_eq!(reindexed.columns["val"].values()[0], Scalar::Float64(30.0)); // label 2
        assert_eq!(reindexed.columns["val"].values()[1], Scalar::Float64(10.0)); // label 0
        assert!(reindexed.columns["val"].values()[2].is_missing()); // label 5 → NaN
    }

    // ── str accessor tests ──

    #[test]
    fn str_lower() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("Hello".into()), Scalar::Utf8("WORLD".into())],
        )
        .unwrap();
        let result = s.str().lower().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hello".into()));
        assert_eq!(result.values()[1], Scalar::Utf8("world".into()));
    }

    #[test]
    fn str_upper() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("hello".into()), Scalar::Utf8("World".into())],
        )
        .unwrap();
        let result = s.str().upper().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("HELLO".into()));
        assert_eq!(result.values()[1], Scalar::Utf8("WORLD".into()));
    }

    #[test]
    fn str_strip() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("  hello  ".into()),
                Scalar::Utf8("world\n".into()),
            ],
        )
        .unwrap();
        let result = s.str().strip().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hello".into()));
        assert_eq!(result.values()[1], Scalar::Utf8("world".into()));
    }

    #[test]
    fn str_contains() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("foo bar".into()),
                Scalar::Utf8("baz".into()),
                Scalar::Utf8("foobar".into()),
            ],
        )
        .unwrap();
        let result = s.str().contains("foo").unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
        assert_eq!(result.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn str_replace() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello world".into())],
        )
        .unwrap();
        let result = s.str().replace("world", "rust").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hello rust".into()));
    }

    #[test]
    fn str_startswith_endswith() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("prefix_data".into()),
                Scalar::Utf8("data_suffix".into()),
            ],
        )
        .unwrap();
        let starts = s.str().startswith("prefix").unwrap();
        assert_eq!(starts.values()[0], Scalar::Bool(true));
        assert_eq!(starts.values()[1], Scalar::Bool(false));

        let ends = s.str().endswith("suffix").unwrap();
        assert_eq!(ends.values()[0], Scalar::Bool(false));
        assert_eq!(ends.values()[1], Scalar::Bool(true));
    }

    #[test]
    fn str_len() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("hi".into()), Scalar::Utf8("hello".into())],
        )
        .unwrap();
        let result = s.str().len().unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(2));
        assert_eq!(result.values()[1], Scalar::Int64(5));
    }

    #[test]
    fn str_slice() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello world".into())],
        )
        .unwrap();
        let result = s.str().slice(0, Some(5)).unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hello".into()));
    }

    #[test]
    fn str_split_get() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("a-b-c".into())],
        )
        .unwrap();
        let result = s.str().split_get("-", 1).unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("b".into()));
    }

    #[test]
    fn str_capitalize() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello".into())],
        )
        .unwrap();
        let result = s.str().capitalize().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("Hello".into()));
    }

    #[test]
    fn str_with_nulls() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("hello".into()),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();
        let result = s.str().upper().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("HELLO".into()));
        assert!(result.values()[1].is_missing());
    }

    // ── DataFrame groupby tests ──

    #[test]
    fn dataframe_groupby_sum() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.groupby(&["grp"]).unwrap().sum().unwrap();
        assert_eq!(result.len(), 2); // groups: a, b
        // Group "a": 1+3=4; Group "b": 2+4=6
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(4.0));
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(6.0));
    }

    #[test]
    fn dataframe_groupby_mean() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("b".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(40.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.groupby(&["grp"]).unwrap().mean().unwrap();
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(15.0)); // mean(10,20)
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(35.0)); // mean(30,40)
    }

    #[test]
    fn dataframe_groupby_count() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.groupby(&["grp"]).unwrap().count().unwrap();
        // Group "a": 1 non-null (NaN doesn't count); Group "b": 1
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(1.0)); // count skips NaN
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(1.0));
    }

    #[test]
    fn dataframe_groupby_size() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.groupby(&["grp"]).unwrap().size().unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(2)); // group "a" has 2 rows
        assert_eq!(result.values()[1], Scalar::Int64(1)); // group "b" has 1 row
    }

    // ── sample tests ──

    #[test]
    fn dataframe_sample_n() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(40.0),
                    Scalar::Float64(50.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let sampled = df.sample(Some(3), None, false, Some(123)).unwrap();
        assert_eq!(sampled.len(), 3);
    }

    #[test]
    fn dataframe_sample_frac() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let sampled = df.sample(None, Some(0.5), false, Some(42)).unwrap();
        assert_eq!(sampled.len(), 2); // 50% of 4 rows
    }

    #[test]
    fn dataframe_sample_deterministic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let s1 = df.sample(Some(2), None, false, Some(99)).unwrap();
        let s2 = df.sample(Some(2), None, false, Some(99)).unwrap();
        // Same seed → same result
        assert_eq!(s1.columns["val"].values(), s2.columns["val"].values());
    }

    // ── info test ──

    #[test]
    fn dataframe_info_basic() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let info = df.info();
        assert!(info.contains("2 rows"));
        assert!(info.contains("1 columns"));
        assert!(info.contains("Float64"));
    }

    // ── stack/unstack tests ──

    #[test]
    fn dataframe_stack_unstack_roundtrip() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(3.0), Scalar::Float64(4.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let stacked = df.stack().unwrap();
        assert_eq!(stacked.len(), 4); // 2 rows * 2 cols
        assert_eq!(stacked.column_order, vec!["value"]);

        let unstacked = stacked.unstack().unwrap();
        assert_eq!(unstacked.len(), 2);
        assert_eq!(unstacked.column_order.len(), 2);
        // Values should round-trip
        assert_eq!(unstacked.columns["a"].values()[0], Scalar::Float64(1.0));
        assert_eq!(unstacked.columns["a"].values()[1], Scalar::Float64(2.0));
        assert_eq!(unstacked.columns["b"].values()[0], Scalar::Float64(3.0));
        assert_eq!(unstacked.columns["b"].values()[1], Scalar::Float64(4.0));
    }

    // ── apply_fn tests ──

    #[test]
    fn series_apply_fn() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();
        let result = s
            .apply_fn(|v| match v.to_f64() {
                Ok(f) => Scalar::Float64(f * f),
                Err(_) => v.clone(),
            })
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(4.0));
        assert_eq!(result.values()[2], Scalar::Float64(9.0));
    }

    #[test]
    fn dataframe_apply_fn_column_wise() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(3.0)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(10.0), Scalar::Float64(20.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df
            .apply_fn(
                |vals| {
                    let sum: f64 = vals
                        .iter()
                        .filter_map(|v| v.to_f64().ok())
                        .sum();
                    Scalar::Float64(sum)
                },
                0,
            )
            .unwrap();
        // Column "a" sum=4, column "b" sum=30
        assert_eq!(result.columns["result"].values()[0], Scalar::Float64(4.0));
        assert_eq!(result.columns["result"].values()[1], Scalar::Float64(30.0));
    }

    #[test]
    fn dataframe_apply_fn_row_wise() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(3.0)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(10.0), Scalar::Float64(20.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df
            .apply_fn(
                |vals| {
                    let sum: f64 = vals
                        .iter()
                        .filter_map(|v| v.to_f64().ok())
                        .sum();
                    Scalar::Float64(sum)
                },
                1,
            )
            .unwrap();
        // Row 0: 1+10=11, Row 1: 3+20=23
        assert_eq!(result.columns["result"].values()[0], Scalar::Float64(11.0));
        assert_eq!(result.columns["result"].values()[1], Scalar::Float64(23.0));
    }

    // ── map_values test ──

    #[test]
    fn series_map_values() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
            ],
        )
        .unwrap();

        let mut mapping = std::collections::HashMap::new();
        mapping.insert("a".to_string(), Scalar::Int64(1));
        mapping.insert("b".to_string(), Scalar::Int64(2));
        // "c" is not in mapping → should become NaN

        let result = s.map_values(&mapping).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1));
        assert_eq!(result.values()[1], Scalar::Int64(2));
        assert!(result.values()[2].is_missing()); // unmapped → NaN
    }
}
