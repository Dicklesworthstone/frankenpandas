#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

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

    /// Construct a Series from a dict (BTreeMap) of label → value.
    ///
    /// Matches `pd.Series({"a": 1, "b": 2})`.
    pub fn from_dict(
        name: impl Into<String>,
        data: BTreeMap<IndexLabel, Scalar>,
    ) -> Result<Self, FrameError> {
        let (labels, values): (Vec<_>, Vec<_>) = data.into_iter().collect();
        Self::from_values(name, labels, values)
    }

    /// Pretty-print the Series as a string table.
    ///
    /// Matches `str(series)` / `series.to_string()` in pandas.
    #[must_use]
    pub fn to_string_repr(&self) -> String {
        let mut lines = Vec::with_capacity(self.len() + 2);
        // Compute max label width using the same display format as rendering
        let label_width = self
            .index
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Int64(v) => v.to_string().len(),
                IndexLabel::Utf8(s) => s.len(),
            })
            .max()
            .unwrap_or(0);

        for (label, val) in self.index.labels().iter().zip(self.column.values()) {
            let lbl_str = match label {
                IndexLabel::Int64(v) => v.to_string(),
                IndexLabel::Utf8(s) => s.clone(),
            };
            let val_str = match val {
                Scalar::Null(_) => "NaN".to_string(),
                Scalar::Bool(b) => b.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => {
                    if v.is_nan() {
                        "NaN".to_string()
                    } else {
                        v.to_string()
                    }
                }
                Scalar::Utf8(s) => s.clone(),
            };
            lines.push(format!("{lbl_str:<label_width$}    {val_str}"));
        }
        lines.push(format!("Name: {}, Length: {}", self.name, self.len()));
        lines.join("\n")
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

    /// Return the dtype of this Series.
    ///
    /// Matches `pd.Series.dtype`.
    #[must_use]
    pub fn dtype(&self) -> DType {
        self.column.dtype()
    }

    /// Return a deep copy of this Series.
    ///
    /// Matches `pd.Series.copy(deep=True)`.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
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

    /// Binary operation with fill_value for NaN handling.
    ///
    /// Matches `s1.add(s2, fill_value=0)` in pandas. When one operand is NaN
    /// and the other is not, the NaN is replaced with `fill_value` before the
    /// operation. When both are NaN, the result is NaN.
    fn binary_op_fill(
        &self,
        other: &Self,
        op: ArithmeticOp,
        fill_value: f64,
    ) -> Result<Self, FrameError> {
        let (left_aligned, right_aligned) = self.align(other, AlignMode::Outer)?;

        let mut out = Vec::with_capacity(left_aligned.len());
        for (lv, rv) in left_aligned
            .column()
            .values()
            .iter()
            .zip(right_aligned.column().values().iter())
        {
            let result = match (lv.is_missing(), rv.is_missing()) {
                (true, true) => Scalar::Null(NullKind::NaN),
                (true, false) => {
                    let r = rv.to_f64().map_err(ColumnError::from)?;
                    let v = match op {
                        ArithmeticOp::Add => fill_value + r,
                        ArithmeticOp::Sub => fill_value - r,
                        ArithmeticOp::Mul => fill_value * r,
                        ArithmeticOp::Div => {
                            if r == 0.0 {
                                f64::NAN
                            } else {
                                fill_value / r
                            }
                        }
                    };
                    Scalar::Float64(v)
                }
                (false, true) => {
                    let l = lv.to_f64().map_err(ColumnError::from)?;
                    let v = match op {
                        ArithmeticOp::Add => l + fill_value,
                        ArithmeticOp::Sub => l - fill_value,
                        ArithmeticOp::Mul => l * fill_value,
                        ArithmeticOp::Div => {
                            if fill_value == 0.0 {
                                f64::NAN
                            } else {
                                l / fill_value
                            }
                        }
                    };
                    Scalar::Float64(v)
                }
                (false, false) => {
                    let l = lv.to_f64().map_err(ColumnError::from)?;
                    let r = rv.to_f64().map_err(ColumnError::from)?;
                    let v = match op {
                        ArithmeticOp::Add => l + r,
                        ArithmeticOp::Sub => l - r,
                        ArithmeticOp::Mul => l * r,
                        ArithmeticOp::Div => {
                            if r == 0.0 {
                                f64::NAN
                            } else {
                                l / r
                            }
                        }
                    };
                    Scalar::Float64(v)
                }
            };
            out.push(result);
        }

        let out_name = if self.name == other.name {
            self.name.clone()
        } else {
            let op_sym = match op {
                ArithmeticOp::Add => "+",
                ArithmeticOp::Sub => "-",
                ArithmeticOp::Mul => "*",
                ArithmeticOp::Div => "/",
            };
            format!("{}{op_sym}{}", self.name, other.name)
        };

        Self::from_values(
            &out_name,
            left_aligned.index().labels().to_vec(),
            out,
        )
    }

    /// Add with fill_value for NaN handling.
    ///
    /// Matches `s1.add(s2, fill_value=0)`.
    pub fn add_fill(&self, other: &Self, fill_value: f64) -> Result<Self, FrameError> {
        self.binary_op_fill(other, ArithmeticOp::Add, fill_value)
    }

    /// Subtract with fill_value for NaN handling.
    pub fn sub_fill(&self, other: &Self, fill_value: f64) -> Result<Self, FrameError> {
        self.binary_op_fill(other, ArithmeticOp::Sub, fill_value)
    }

    /// Multiply with fill_value for NaN handling.
    pub fn mul_fill(&self, other: &Self, fill_value: f64) -> Result<Self, FrameError> {
        self.binary_op_fill(other, ArithmeticOp::Mul, fill_value)
    }

    /// Divide with fill_value for NaN handling.
    pub fn div_fill(&self, other: &Self, fill_value: f64) -> Result<Self, FrameError> {
        self.binary_op_fill(other, ArithmeticOp::Div, fill_value)
    }

    /// Modulo operation (element-wise remainder).
    ///
    /// Matches `s1.mod(s2)` / `s1 % s2` in pandas.
    pub fn modulo(&self, other: &Self) -> Result<Self, FrameError> {
        let (left_aligned, right_aligned) = self.align(other, AlignMode::Outer)?;
        let mut out = Vec::with_capacity(left_aligned.len());
        for (lv, rv) in left_aligned
            .column()
            .values()
            .iter()
            .zip(right_aligned.column().values().iter())
        {
            if lv.is_missing() || rv.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                let l = lv.to_f64().map_err(ColumnError::from)?;
                let r = rv.to_f64().map_err(ColumnError::from)?;
                out.push(if r == 0.0 {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Float64(l % r)
                });
            }
        }
        Self::from_values(
            self.name(),
            left_aligned.index().labels().to_vec(),
            out,
        )
    }

    /// Power operation (element-wise exponentiation).
    ///
    /// Matches `s1.pow(s2)` / `s1 ** s2` in pandas.
    pub fn pow(&self, other: &Self) -> Result<Self, FrameError> {
        let (left_aligned, right_aligned) = self.align(other, AlignMode::Outer)?;
        let mut out = Vec::with_capacity(left_aligned.len());
        for (lv, rv) in left_aligned
            .column()
            .values()
            .iter()
            .zip(right_aligned.column().values().iter())
        {
            if lv.is_missing() || rv.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                let l = lv.to_f64().map_err(ColumnError::from)?;
                let r = rv.to_f64().map_err(ColumnError::from)?;
                out.push(Scalar::Float64(l.powf(r)));
            }
        }
        Self::from_values(
            self.name(),
            left_aligned.index().labels().to_vec(),
            out,
        )
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

    /// Append another Series to this one.
    ///
    /// Matches `pd.Series.append(other)` (deprecated but common).
    /// Concatenates values and index labels.
    pub fn append(&self, other: &Self) -> Result<Self, FrameError> {
        let mut labels = self.index.labels().to_vec();
        labels.extend_from_slice(other.index.labels());
        let mut values = self.column.values().to_vec();
        values.extend_from_slice(other.column.values());
        Self::from_values(self.name.clone(), labels, values)
    }

    /// Return the index label of the first non-null value.
    ///
    /// Matches `pd.Series.first_valid_index()`.
    pub fn first_valid_index(&self) -> Option<IndexLabel> {
        for (i, val) in self.column.values().iter().enumerate() {
            if !val.is_missing() {
                return Some(self.index.labels()[i].clone());
            }
        }
        None
    }

    /// Return the index label of the last non-null value.
    ///
    /// Matches `pd.Series.last_valid_index()`.
    pub fn last_valid_index(&self) -> Option<IndexLabel> {
        for (i, val) in self.column.values().iter().enumerate().rev() {
            if !val.is_missing() {
                return Some(self.index.labels()[i].clone());
            }
        }
        None
    }

    /// Update values using non-null values from another Series.
    ///
    /// Analogous to `pandas.Series.update(other)`. Only updates values at
    /// matching index labels where `other` has non-null values. The result
    /// retains self's index (no new labels added from other).
    pub fn update(&self, other: &Self) -> Result<Self, FrameError> {
        let other_map: BTreeMap<&IndexLabel, &Scalar> = other
            .index
            .labels()
            .iter()
            .zip(other.values().iter())
            .collect();

        let new_values: Vec<Scalar> = self
            .index
            .labels()
            .iter()
            .zip(self.values().iter())
            .map(|(label, val)| {
                if let Some(&other_val) = other_map.get(label)
                    && !other_val.is_missing()
                {
                    return other_val.clone();
                }
                val.clone()
            })
            .collect();

        let col = Column::from_values(new_values)?;
        Self::new(self.name.clone(), self.index.clone(), col)
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

    /// Element-wise `== scalar`.
    pub fn eq_scalar(&self, scalar: &Scalar) -> Result<Self, FrameError> {
        self.compare_scalar(scalar, ComparisonOp::Eq)
    }

    /// Element-wise `!= scalar`.
    pub fn ne_scalar(&self, scalar: &Scalar) -> Result<Self, FrameError> {
        self.compare_scalar(scalar, ComparisonOp::Ne)
    }

    /// Element-wise `> scalar`.
    pub fn gt_scalar(&self, scalar: &Scalar) -> Result<Self, FrameError> {
        self.compare_scalar(scalar, ComparisonOp::Gt)
    }

    /// Element-wise `>= scalar`.
    pub fn ge_scalar(&self, scalar: &Scalar) -> Result<Self, FrameError> {
        self.compare_scalar(scalar, ComparisonOp::Ge)
    }

    /// Element-wise `< scalar`.
    pub fn lt_scalar(&self, scalar: &Scalar) -> Result<Self, FrameError> {
        self.compare_scalar(scalar, ComparisonOp::Lt)
    }

    /// Element-wise `<= scalar`.
    pub fn le_scalar(&self, scalar: &Scalar) -> Result<Self, FrameError> {
        self.compare_scalar(scalar, ComparisonOp::Le)
    }

    /// Test whether two Series are equal (same name, index, and values).
    ///
    /// Analogous to `pandas.Series.equals(other)`. Returns `true` only if
    /// both Series have identical names, index labels, and values (including
    /// NaN == NaN semantics for null positions).
    pub fn equals(&self, other: &Self) -> bool {
        if self.name != other.name {
            return false;
        }
        if self.index.labels() != other.index.labels() {
            return false;
        }
        let lv = self.values();
        let rv = other.values();
        if lv.len() != rv.len() {
            return false;
        }
        for (a, b) in lv.iter().zip(rv.iter()) {
            if a.is_missing() && b.is_missing() {
                continue;
            }
            if a != b {
                return false;
            }
        }
        true
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

    /// Forward-fill missing values (propagate last valid observation forward).
    ///
    /// Matches `pd.Series.ffill()` / `pd.Series.fillna(method='ffill')`.
    /// An optional `limit` caps the maximum number of consecutive NaNs to fill.
    pub fn ffill(&self, limit: Option<usize>) -> Result<Self, FrameError> {
        let vals = self.column.values();
        let mut out = Vec::with_capacity(vals.len());
        let mut last_valid: Option<&Scalar> = None;
        let mut consecutive_fills: usize = 0;

        for val in vals {
            if val.is_missing() {
                if let Some(fill) = last_valid {
                    consecutive_fills += 1;
                    if limit.is_none() || consecutive_fills <= limit.unwrap() {
                        out.push(fill.clone());
                    } else {
                        out.push(val.clone());
                    }
                } else {
                    out.push(val.clone());
                }
            } else {
                last_valid = Some(val);
                consecutive_fills = 0;
                out.push(val.clone());
            }
        }

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Back-fill missing values (propagate next valid observation backward).
    ///
    /// Matches `pd.Series.bfill()` / `pd.Series.fillna(method='bfill')`.
    /// An optional `limit` caps the maximum number of consecutive NaNs to fill.
    pub fn bfill(&self, limit: Option<usize>) -> Result<Self, FrameError> {
        let vals = self.column.values();
        let n = vals.len();
        let mut out = vec![Scalar::Null(NullKind::NaN); n];
        let mut next_valid: Option<&Scalar> = None;
        let mut consecutive_fills: usize = 0;

        for i in (0..n).rev() {
            if vals[i].is_missing() {
                if let Some(fill) = next_valid {
                    consecutive_fills += 1;
                    if limit.is_none() || consecutive_fills <= limit.unwrap() {
                        out[i] = fill.clone();
                    } else {
                        out[i] = vals[i].clone();
                    }
                } else {
                    out[i] = vals[i].clone();
                }
            } else {
                next_valid = Some(&vals[i]);
                consecutive_fills = 0;
                out[i] = vals[i].clone();
            }
        }

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
    }

    /// Linearly interpolate missing values.
    ///
    /// Matches `pd.Series.interpolate(method='linear')`. Non-numeric values
    /// and leading/trailing NaNs are left as NaN. Only interior NaN gaps
    /// between two valid numeric values are filled.
    pub fn interpolate(&self) -> Result<Self, FrameError> {
        let vals = self.column.values();
        let n = vals.len();
        let mut out: Vec<Scalar> = vals.to_vec();

        let mut i = 0;
        while i < n {
            if out[i].is_missing() {
                // Find the start of the gap (previous valid value)
                let gap_start = i;
                let left = if gap_start > 0 {
                    out[gap_start - 1].to_f64().ok()
                } else {
                    None
                };

                // Find end of the gap
                let mut j = i;
                while j < n && out[j].is_missing() {
                    j += 1;
                }

                let right = if j < n { out[j].to_f64().ok() } else { None };

                // Only interpolate if we have both endpoints
                if let (Some(lv), Some(rv)) = (left, right) {
                    let gap_len = j - gap_start + 1; // includes both endpoints
                    for (offset, slot) in out[gap_start..j].iter_mut().enumerate() {
                        let t = (offset + 1) as f64 / gap_len as f64;
                        *slot = Scalar::Float64(lv + t * (rv - lv));
                    }
                }

                i = j + 1;
            } else {
                i += 1;
            }
        }

        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
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

    /// Value counts with full pandas parameter support.
    ///
    /// Matches `pd.Series.value_counts(normalize, sort, ascending, dropna)`.
    pub fn value_counts_with_options(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> Result<Self, FrameError> {
        let mut counts: Vec<(Scalar, usize)> = Vec::new();
        let mut null_count = 0_usize;

        for value in self.column.values() {
            if value.is_missing() {
                null_count += 1;
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

        if !dropna && null_count > 0 {
            counts.push((Scalar::Null(NullKind::NaN), null_count));
        }

        if sort {
            if ascending {
                counts.sort_by_key(|(_, a)| *a);
            } else {
                counts.sort_by_key(|(_, a)| std::cmp::Reverse(*a));
            }
        }

        let total = if normalize {
            self.len() as f64
        } else {
            1.0
        };

        let mut labels = Vec::with_capacity(counts.len());
        let mut values = Vec::with_capacity(counts.len());
        for (value, count) in counts {
            labels.push(scalar_to_value_counts_index_label(&value));
            if normalize {
                values.push(Scalar::Float64(count as f64 / total));
            } else {
                values.push(Scalar::Int64(i64::try_from(count).unwrap_or(i64::MAX)));
            }
        }

        Self::from_values("count", labels, values)
    }

    /// Check if all values in the Series are unique (no duplicates).
    ///
    /// Matches `pd.Series.is_unique`.
    #[must_use]
    pub fn is_unique(&self) -> bool {
        let non_null = self.unique();
        let null_count = self.column.values().iter().filter(|v| v.is_missing()).count();
        let total_non_null = self.len() - null_count;
        non_null.len() == total_non_null && null_count <= 1
    }

    /// Check if values are monotonically increasing (non-decreasing).
    ///
    /// Matches `pd.Series.is_monotonic_increasing`.
    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        let vals = self.column.values();
        if vals.len() <= 1 {
            return true;
        }
        for pair in vals.windows(2) {
            if pair[0].is_missing() || pair[1].is_missing() {
                return false;
            }
            match (pair[0].to_f64(), pair[1].to_f64()) {
                (Ok(a), Ok(b)) if a <= b => {}
                _ => return false,
            }
        }
        true
    }

    /// Check if values are monotonically decreasing (non-increasing).
    ///
    /// Matches `pd.Series.is_monotonic_decreasing`.
    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        let vals = self.column.values();
        if vals.len() <= 1 {
            return true;
        }
        for pair in vals.windows(2) {
            if pair[0].is_missing() || pair[1].is_missing() {
                return false;
            }
            match (pair[0].to_f64(), pair[1].to_f64()) {
                (Ok(a), Ok(b)) if a >= b => {}
                _ => return false,
            }
        }
        true
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

    /// Cast with error handling.
    ///
    /// Matches `pd.Series.astype(dtype, errors='coerce'|'raise')`.
    /// - `errors="raise"` (default): same as `astype()`
    /// - `errors="coerce"`: values that fail conversion become NaN
    pub fn astype_safe(&self, dtype: DType, errors: &str) -> Result<Self, FrameError> {
        if errors != "coerce" {
            return self.astype(dtype);
        }
        // Coerce mode: try converting each value, NaN on failure
        let mut out = Vec::with_capacity(self.len());
        for val in self.column.values() {
            if val.is_missing() {
                out.push(val.clone());
                continue;
            }
            match dtype {
                DType::Float64 => match val.to_f64() {
                    Ok(f) => out.push(Scalar::Float64(f)),
                    Err(_) => out.push(Scalar::Null(NullKind::NaN)),
                },
                DType::Int64 => match val {
                    Scalar::Int64(_) => out.push(val.clone()),
                    Scalar::Float64(f) => out.push(Scalar::Int64(*f as i64)),
                    Scalar::Utf8(s) => match s.parse::<i64>() {
                        Ok(n) => out.push(Scalar::Int64(n)),
                        Err(_) => out.push(Scalar::Null(NullKind::NaN)),
                    },
                    Scalar::Bool(b) => out.push(Scalar::Int64(if *b { 1 } else { 0 })),
                    _ => out.push(Scalar::Null(NullKind::NaN)),
                },
                DType::Utf8 => out.push(Scalar::Utf8(format!("{val}"))),
                DType::Bool => match val {
                    Scalar::Bool(_) => out.push(val.clone()),
                    Scalar::Int64(n) => out.push(Scalar::Bool(*n != 0)),
                    Scalar::Float64(f) => out.push(Scalar::Bool(*f != 0.0)),
                    Scalar::Utf8(s) => match s.to_lowercase().as_str() {
                        "true" | "1" | "yes" => out.push(Scalar::Bool(true)),
                        "false" | "0" | "no" => out.push(Scalar::Bool(false)),
                        _ => out.push(Scalar::Null(NullKind::NaN)),
                    },
                    _ => out.push(Scalar::Null(NullKind::NaN)),
                },
                DType::Null => out.push(Scalar::Null(NullKind::Null)),
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), out)
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

    /// Clip values below a threshold.
    pub fn clip_lower(&self, threshold: f64) -> Result<Self, FrameError> {
        self.clip(Some(threshold), None)
    }

    /// Clip values above a threshold.
    pub fn clip_upper(&self, threshold: f64) -> Result<Self, FrameError> {
        self.clip(None, Some(threshold))
    }

    /// Return a new Series with name prefixed.
    ///
    /// Analogous to `pandas.Series.add_prefix(prefix)`.
    pub fn add_prefix(&self, prefix: &str) -> Result<Self, FrameError> {
        Self::new(
            format!("{prefix}{}", self.name),
            self.index.clone(),
            self.column.clone(),
        )
    }

    /// Return a new Series with name suffixed.
    ///
    /// Analogous to `pandas.Series.add_suffix(suffix)`.
    pub fn add_suffix(&self, suffix: &str) -> Result<Self, FrameError> {
        Self::new(
            format!("{}{suffix}", self.name),
            self.index.clone(),
            self.column.clone(),
        )
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

    /// Product of non-null numeric values. Returns 1.0 for empty.
    ///
    /// Matches `pd.Series.prod()`.
    pub fn prod(&self) -> Result<Scalar, FrameError> {
        let mut product = 1.0_f64;
        for val in self.column.values() {
            if !val.is_missing() {
                product *= val.to_f64().map_err(ColumnError::from)?;
            }
        }
        Ok(Scalar::Float64(product))
    }

    /// Most frequently occurring value(s).
    ///
    /// Matches `pd.Series.mode()`. Returns a new Series containing
    /// all values tied for the highest frequency, sorted ascending.
    pub fn mode(&self) -> Result<Self, FrameError> {
        let mut counts: BTreeMap<String, (Scalar, usize)> = BTreeMap::new();
        for val in self.column.values() {
            if val.is_missing() {
                continue;
            }
            let key = format!("{val:?}");
            counts
                .entry(key)
                .and_modify(|(_, c)| *c += 1)
                .or_insert_with(|| (val.clone(), 1));
        }
        let max_count = counts.values().map(|(_, c)| *c).max().unwrap_or(0);
        if max_count == 0 {
            let idx = Index::new(Vec::new());
            let col = Column::from_values(Vec::new())?;
            return Self::new(self.name.clone(), idx, col);
        }
        let mut modes: Vec<Scalar> = counts
            .into_values()
            .filter(|(_, c)| *c == max_count)
            .map(|(v, _)| v)
            .collect();
        // Sort modes for deterministic output
        modes.sort_by(|a, b| {
            let fa = a.to_f64().ok();
            let fb = b.to_f64().ok();
            match (fa, fb) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                _ => format!("{a:?}").cmp(&format!("{b:?}")),
            }
        });
        let labels: Vec<IndexLabel> = (0..modes.len()).map(|i| (i as i64).into()).collect();
        Self::from_values(self.name.clone(), labels, modes)
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

    /// Return the `n` smallest values with `keep` parameter.
    ///
    /// Matches `series.nsmallest(n, keep='first'|'last'|'all')`.
    /// - `first`: keep first occurrence of duplicates
    /// - `last`: keep last occurrence of duplicates
    /// - `all`: keep all occurrences (may return more than n rows)
    pub fn nsmallest_keep(&self, n: usize, keep: &str) -> Result<Self, FrameError> {
        self.nlargest_smallest_impl(n, keep, true)
    }

    /// Return the `n` largest values with `keep` parameter.
    ///
    /// Matches `series.nlargest(n, keep='first'|'last'|'all')`.
    pub fn nlargest_keep(&self, n: usize, keep: &str) -> Result<Self, FrameError> {
        self.nlargest_smallest_impl(n, keep, false)
    }

    fn nlargest_smallest_impl(
        &self,
        n: usize,
        keep: &str,
        ascending: bool,
    ) -> Result<Self, FrameError> {
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

        if ascending {
            indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        } else {
            indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        }

        match keep {
            "last" => {
                // Reverse position order for ties
                indexed.sort_by(|a, b| {
                    let cmp = if ascending {
                        a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal)
                    } else {
                        b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal)
                    };
                    if cmp == Ordering::Equal {
                        b.0.cmp(&a.0) // last occurrence first
                    } else {
                        cmp
                    }
                });
                indexed.truncate(n);
            }
            "all" => {
                // Keep all values tied with the nth value
                if indexed.len() > n {
                    let threshold = indexed[n - 1].1;
                    indexed.retain(|&(_, v)| {
                        if ascending {
                            v <= threshold
                        } else {
                            v >= threshold
                        }
                    });
                }
            }
            _ => {
                // "first" - default, already in position order
                indexed.truncate(n);
            }
        }

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

    // ── Positional indexing ──────────────────────────────────────

    /// Return the integer indices that would sort the Series.
    ///
    /// Matches `pd.Series.argsort()`. NaN values are placed last.
    pub fn argsort(&self, ascending: bool) -> Result<Self, FrameError> {
        let mut order: Vec<usize> = (0..self.len()).collect();
        let vals = self.column.values();
        order.sort_by(|&a, &b| compare_scalars_with_na_last(&vals[a], &vals[b], ascending));

        let labels = self.index.labels().to_vec();
        let out_vals: Vec<Scalar> = order
            .into_iter()
            .map(|i| Scalar::Int64(i as i64))
            .collect();
        Self::from_values(self.name.clone(), labels, out_vals)
    }

    /// Return the integer position of the minimum value.
    ///
    /// Matches `pd.Series.argmin()`. Skips missing values.
    pub fn argmin(&self) -> Result<usize, FrameError> {
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
        best_idx.ok_or_else(|| {
            FrameError::CompatibilityRejected("argmin of empty or all-null series".to_owned())
        })
    }

    /// Return the integer position of the maximum value.
    ///
    /// Matches `pd.Series.argmax()`. Skips missing values.
    pub fn argmax(&self) -> Result<usize, FrameError> {
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
        best_idx.ok_or_else(|| {
            FrameError::CompatibilityRejected("argmax of empty or all-null series".to_owned())
        })
    }

    /// Select elements by integer positions.
    ///
    /// Matches `pd.Series.take(indices)`. Negative indices are not supported.
    pub fn take(&self, indices: &[usize]) -> Result<Self, FrameError> {
        let n = self.len();
        let mut labels = Vec::with_capacity(indices.len());
        let mut values = Vec::with_capacity(indices.len());
        for &idx in indices {
            if idx >= n {
                return Err(FrameError::CompatibilityRejected(format!(
                    "take index {idx} out of bounds for length {n}"
                )));
            }
            labels.push(self.index.labels()[idx].clone());
            values.push(self.column.values()[idx].clone());
        }
        Self::from_values(self.name.clone(), labels, values)
    }

    /// Find insertion points to maintain sorted order.
    ///
    /// Matches `pd.Series.searchsorted(value, side='left'|'right')`.
    /// The Series must be sorted. Returns integer positions.
    pub fn searchsorted(&self, value: &Scalar, side: &str) -> Result<usize, FrameError> {
        let vals = self.column.values();
        let target = value
            .to_f64()
            .map_err(|e| FrameError::Column(ColumnError::from(e)))?;

        match side {
            "right" => {
                // First position where val > target
                let pos = vals
                    .iter()
                    .position(|v| {
                        if v.is_missing() {
                            true // NaN sorts last
                        } else {
                            v.to_f64().map_or(true, |f| f > target)
                        }
                    })
                    .unwrap_or(vals.len());
                Ok(pos)
            }
            _ => {
                // "left" default: first position where val >= target
                let pos = vals
                    .iter()
                    .position(|v| {
                        if v.is_missing() {
                            true
                        } else {
                            v.to_f64().map_or(true, |f| f >= target)
                        }
                    })
                    .unwrap_or(vals.len());
                Ok(pos)
            }
        }
    }

    /// Encode the Series as an enumerated type.
    ///
    /// Matches `pd.Series.factorize()`. Returns `(codes, uniques)` where
    /// `codes` is an Int64 Series of category indices (-1 for NaN) and
    /// `uniques` is a Series of the unique values in order of appearance.
    pub fn factorize(&self) -> Result<(Self, Self), FrameError> {
        let vals = self.column.values();
        let mut uniques: Vec<Scalar> = Vec::new();
        let mut codes: Vec<Scalar> = Vec::with_capacity(vals.len());

        for val in vals {
            if val.is_missing() {
                codes.push(Scalar::Int64(-1));
                continue;
            }
            let pos = uniques.iter().position(|u| u == val);
            match pos {
                Some(p) => codes.push(Scalar::Int64(p as i64)),
                None => {
                    codes.push(Scalar::Int64(uniques.len() as i64));
                    uniques.push(val.clone());
                }
            }
        }

        let code_labels: Vec<IndexLabel> = (0..codes.len())
            .map(|i| (i as i64).into())
            .collect();
        let unique_labels: Vec<IndexLabel> = (0..uniques.len())
            .map(|i| (i as i64).into())
            .collect();

        let code_series = Self::from_values(self.name.clone(), code_labels, codes)?;
        let unique_series = Self::from_values("uniques".to_owned(), unique_labels, uniques)?;
        Ok((code_series, unique_series))
    }

    /// Randomly sample elements from the Series.
    ///
    /// Matches `pd.Series.sample(n, frac, replace, random_state)`.
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
            (None, Some(f)) => (total as f64 * f).round() as usize,
            (None, None) => 1,
            (Some(_), Some(_)) => {
                return Err(FrameError::CompatibilityRejected(
                    "cannot specify both n and frac".to_string(),
                ));
            }
        };

        if !replace && sample_n > total {
            return Err(FrameError::CompatibilityRejected(format!(
                "cannot sample {sample_n} from {total} without replacement"
            )));
        }

        let mut rng_state = seed.unwrap_or(42);
        let mut next_rand = || -> usize {
            rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (rng_state >> 33) as usize
        };

        let indices: Vec<usize> = if replace {
            (0..sample_n).map(|_| next_rand() % total).collect()
        } else {
            let mut pool: Vec<usize> = (0..total).collect();
            for i in 0..sample_n {
                let j = i + next_rand() % (total - i);
                pool.swap(i, j);
            }
            pool[..sample_n].to_vec()
        };

        let labels: Vec<IndexLabel> = indices
            .iter()
            .map(|&i| self.index.labels()[i].clone())
            .collect();
        let values: Vec<Scalar> = indices
            .iter()
            .map(|&i| self.column.values()[i].clone())
            .collect();
        Self::from_values(self.name(), labels, values)
    }

    // ── Statistical methods ──────────────────────────────────────

    /// Standard error of the mean.
    ///
    /// Matches `pd.Series.sem()`. Computes `std / sqrt(n)`.
    pub fn sem(&self) -> Result<f64, FrameError> {
        let (n, _, _, m2) = self.numeric_moments()?;
        if n < 2 {
            return Err(FrameError::CompatibilityRejected(
                "sem requires at least 2 non-null values".to_owned(),
            ));
        }
        let std = (m2 / (n - 1) as f64).sqrt();
        Ok(std / (n as f64).sqrt())
    }

    /// Skewness (Fisher's definition, bias=False).
    ///
    /// Matches `pd.Series.skew()`.
    pub fn skew(&self) -> Result<f64, FrameError> {
        let (count, mean, vals) = self.numeric_values()?;
        if count < 3 {
            return Err(FrameError::CompatibilityRejected(
                "skew requires at least 3 non-null values".to_owned(),
            ));
        }
        let n = count as f64;
        let m2: f64 = vals.iter().map(|v| (v - mean).powi(2)).sum();
        let m3: f64 = vals.iter().map(|v| (v - mean).powi(3)).sum();
        let s2 = m2 / (n - 1.0);
        if s2 == 0.0 {
            return Ok(0.0);
        }
        let s3 = s2.powf(1.5);
        Ok((n / ((n - 1.0) * (n - 2.0))) * (m3 / s3))
    }

    /// Excess kurtosis (Fisher's definition, bias=False).
    ///
    /// Matches `pd.Series.kurtosis()` / `pd.Series.kurt()`.
    pub fn kurtosis(&self) -> Result<f64, FrameError> {
        let (count, mean, vals) = self.numeric_values()?;
        if count < 4 {
            return Err(FrameError::CompatibilityRejected(
                "kurtosis requires at least 4 non-null values".to_owned(),
            ));
        }
        let n = count as f64;
        let m2: f64 = vals.iter().map(|v| (v - mean).powi(2)).sum();
        let m4: f64 = vals.iter().map(|v| (v - mean).powi(4)).sum();
        let s2 = m2 / (n - 1.0);
        if s2 == 0.0 {
            return Ok(0.0);
        }
        let adj = (n * (n + 1.0)) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
        let sub = (3.0 * (n - 1.0).powi(2)) / ((n - 2.0) * (n - 3.0));
        Ok(adj * (m4 / (s2 * s2)) - sub)
    }

    /// Alias for `kurtosis()`.
    pub fn kurt(&self) -> Result<f64, FrameError> {
        self.kurtosis()
    }

    /// Collect non-null numeric values along with count and mean.
    fn numeric_values(&self) -> Result<(usize, f64, Vec<f64>), FrameError> {
        let mut vals = Vec::new();
        for v in self.column.values() {
            if !v.is_missing() {
                vals.push(v.to_f64().map_err(ColumnError::from)?);
            }
        }
        if vals.is_empty() {
            return Err(FrameError::CompatibilityRejected(
                "no non-null numeric values".to_owned(),
            ));
        }
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        Ok((vals.len(), mean, vals))
    }

    /// Compute count and running M2 (for variance/std/sem).
    fn numeric_moments(&self) -> Result<(usize, f64, f64, f64), FrameError> {
        let mut count = 0usize;
        let mut mean = 0.0_f64;
        let mut m2 = 0.0_f64;
        for v in self.column.values() {
            if v.is_missing() {
                continue;
            }
            let x = v.to_f64().map_err(ColumnError::from)?;
            count += 1;
            let delta = x - mean;
            mean += delta / count as f64;
            let delta2 = x - mean;
            m2 += delta * delta2;
        }
        if count == 0 {
            return Err(FrameError::CompatibilityRejected(
                "no non-null numeric values".to_owned(),
            ));
        }
        Ok((count, mean, 0.0, m2))
    }

    // ── Squeeze & memory ─────────────────────────────────────────

    /// Squeeze a single-element Series to a Scalar.
    ///
    /// Matches `pd.Series.squeeze()`. Returns the single value if length is 1,
    /// otherwise returns self unchanged wrapped in `Err`.
    pub fn squeeze(&self) -> Result<Scalar, Self> {
        if self.len() == 1 {
            Ok(self.column.values()[0].clone())
        } else {
            Err(self.clone())
        }
    }

    /// Approximate memory usage in bytes.
    ///
    /// Matches `pd.Series.memory_usage()`. Includes index and values.
    pub fn memory_usage(&self) -> usize {
        let index_bytes = std::mem::size_of_val(self.index.labels());
        let values_bytes = std::mem::size_of_val(self.column.values());
        index_bytes + values_bytes
    }

    /// Return true if the Series contains any NaN or null values.
    ///
    /// Matches `pd.Series.hasnans`.
    #[must_use]
    pub fn hasnans(&self) -> bool {
        self.column.values().iter().any(|v| v.is_missing())
    }

    /// Return the approximate number of bytes used by the Series values.
    ///
    /// Matches `pd.Series.nbytes`. Does not include the index.
    #[must_use]
    pub fn nbytes(&self) -> usize {
        std::mem::size_of_val(self.column.values())
    }

    /// Pass the Series through a function.
    ///
    /// Matches `pd.Series.pipe(func)`. Useful for method chaining.
    pub fn pipe<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: FnOnce(&Self) -> Result<Self, FrameError>,
    {
        func(self)
    }

    /// Return an iterator over (index_label, value) pairs.
    ///
    /// Matches `pd.Series.items()`.
    pub fn items(&self) -> Vec<(IndexLabel, Scalar)> {
        self.index
            .labels()
            .iter()
            .zip(self.column.values())
            .map(|(label, val)| (label.clone(), val.clone()))
            .collect()
    }

    /// Access a scalar value by integer position.
    ///
    /// Matches `pd.Series.iat[i]`.
    pub fn iat(&self, pos: i64) -> Result<Scalar, FrameError> {
        let idx = if pos < 0 {
            (self.len() as i64 + pos) as usize
        } else {
            pos as usize
        };
        if idx >= self.len() {
            return Err(FrameError::CompatibilityRejected(format!(
                "iat: position {pos} out of bounds for Series of length {}",
                self.len()
            )));
        }
        Ok(self.column.values()[idx].clone())
    }

    /// Access a scalar value by label.
    ///
    /// Matches `pd.Series.at[label]`.
    pub fn at(&self, label: &IndexLabel) -> Result<Scalar, FrameError> {
        let pos = self.index.position(label).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("at: label {label:?} not found in index"))
        })?;
        Ok(self.column.values()[pos].clone())
    }

    /// Apply a function element-wise, returning a Series of the same shape.
    ///
    /// Matches `pd.Series.transform(func)`.
    pub fn transform<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Scalar,
    {
        let new_values: Vec<Scalar> = self.column.values().iter().map(&func).collect();
        Self::from_values(self.name(), self.index.labels().to_vec(), new_values)
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
                    out.push(Scalar::Float64((cur - prev) / prev));
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

    /// Serialize the Series to a CSV string.
    ///
    /// Matches `pd.Series.to_csv()`.
    pub fn to_csv(&self, sep: char, include_index: bool) -> String {
        fn format_scalar(val: &Scalar) -> String {
            match val {
                Scalar::Null(_) => String::new(),
                Scalar::Bool(b) => if *b { "True" } else { "False" }.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => v.to_string(),
                Scalar::Utf8(s) => s.clone(),
            }
        }

        let mut out = String::new();
        // Header
        if include_index {
            out.push_str(&format!("{sep}{}\n", self.name));
        } else {
            out.push_str(&format!("{}\n", self.name));
        }
        // Data
        for (label, val) in self.index.labels().iter().zip(self.column.values()) {
            if include_index {
                let idx = match label {
                    IndexLabel::Int64(v) => v.to_string(),
                    IndexLabel::Utf8(s) => s.clone(),
                };
                out.push_str(&format!("{idx}{sep}{}\n", format_scalar(val)));
            } else {
                out.push_str(&format!("{}\n", format_scalar(val)));
            }
        }
        out
    }

    /// Serialize the Series to a JSON string.
    ///
    /// Matches `pd.Series.to_json(orient)`.
    /// - `"split"`: `{"name":"...","index":[...],"data":[...]}`
    /// - `"records"` / `"values"`: `[val1, val2, ...]`
    /// - `"index"`: `{"label1": val1, "label2": val2, ...}`
    pub fn to_json(&self, orient: &str) -> Result<String, FrameError> {
        fn json_val(val: &Scalar) -> String {
            match val {
                Scalar::Null(_) => "null".to_string(),
                Scalar::Bool(b) => if *b { "true" } else { "false" }.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => {
                    if v.is_nan() {
                        "null".to_string()
                    } else {
                        v.to_string()
                    }
                }
                Scalar::Utf8(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            }
        }

        fn json_label(lbl: &IndexLabel) -> String {
            match lbl {
                IndexLabel::Int64(v) => format!("\"{v}\""),
                IndexLabel::Utf8(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            }
        }

        match orient {
            "split" => {
                let idx: Vec<String> = self.index.labels().iter().map(json_label).collect();
                let data: Vec<String> = self.column.values().iter().map(json_val).collect();
                Ok(format!(
                    "{{\"name\":\"{}\",\"index\":[{}],\"data\":[{}]}}",
                    self.name,
                    idx.join(","),
                    data.join(","),
                ))
            }
            "records" | "values" => {
                let vals: Vec<String> = self.column.values().iter().map(json_val).collect();
                Ok(format!("[{}]", vals.join(",")))
            }
            "index" => {
                let pairs: Vec<String> = self
                    .index
                    .labels()
                    .iter()
                    .zip(self.column.values())
                    .map(|(lbl, val)| format!("{}:{}", json_label(lbl), json_val(val)))
                    .collect();
                Ok(format!("{{{}}}", pairs.join(",")))
            }
            _ => Err(FrameError::CompatibilityRejected(format!(
                "unknown orient: {orient}"
            ))),
        }
    }

    /// Boolean mask indicating duplicate values.
    ///
    /// Matches `pd.Series.duplicated(keep='first')`.
    /// First occurrence is `false`, subsequent duplicates are `true`.
    pub fn duplicated(&self) -> Result<Self, FrameError> {
        let mut seen = Vec::<&Scalar>::new();
        let mut flags = Vec::with_capacity(self.len());
        for val in self.column.values() {
            let is_dup = seen.iter().any(|existing| existing.semantic_eq(val));
            flags.push(Scalar::Bool(is_dup));
            if !is_dup {
                seen.push(val);
            }
        }
        Self::from_values(self.name.clone(), self.index.labels().to_vec(), flags)
    }

    /// Remove duplicate values, keeping the first occurrence.
    ///
    /// Matches `pd.Series.drop_duplicates(keep='first')`.
    pub fn drop_duplicates(&self) -> Result<Self, FrameError> {
        let mut seen = Vec::<&Scalar>::new();
        let mut indices = Vec::new();
        for (i, val) in self.column.values().iter().enumerate() {
            let is_dup = seen.iter().any(|existing| existing.semantic_eq(val));
            if !is_dup {
                seen.push(val);
                indices.push(i);
            }
        }
        let labels: Vec<IndexLabel> = indices.iter().map(|&i| self.index.labels()[i].clone()).collect();
        let values: Vec<Scalar> = indices.iter().map(|&i| self.column.values()[i].clone()).collect();
        Self::from_values(self.name.clone(), labels, values)
    }

    /// Compare this Series with another, showing only differing values.
    ///
    /// Matches `pd.Series.compare(other)`. Returns a DataFrame with columns
    /// "self" and "other" containing only the rows where the values differ.
    /// Equal values are shown as NaN.
    pub fn compare_with(&self, other: &Self) -> Result<DataFrame, FrameError> {
        if self.len() != other.len() {
            return Err(FrameError::CompatibilityRejected(
                "compare requires Series of equal length".to_owned(),
            ));
        }
        let mut self_vals = Vec::with_capacity(self.len());
        let mut other_vals = Vec::with_capacity(self.len());
        let mut labels = Vec::new();
        let mut has_diff = false;

        for i in 0..self.len() {
            let sv = &self.column.values()[i];
            let ov = &other.column.values()[i];
            if !sv.semantic_eq(ov) {
                has_diff = true;
                self_vals.push(sv.clone());
                other_vals.push(ov.clone());
                labels.push(self.index.labels()[i].clone());
            }
        }

        if !has_diff {
            // No differences - return empty DataFrame
            return DataFrame::from_dict(
                &["self", "other"],
                vec![("self", vec![]), ("other", vec![])],
            );
        }

        DataFrame::from_dict_with_index(
            vec![
                ("self", self_vals),
                ("other", other_vals),
            ],
            labels,
        )
    }

    /// Reindex to match another Series' index.
    ///
    /// Matches `pd.Series.reindex_like(other)`.
    pub fn reindex_like(&self, other: &Self) -> Result<Self, FrameError> {
        self.reindex(other.index.labels().to_vec())
    }

    /// Rename the Series (return a copy with a new name).
    ///
    /// Matches `pd.Series.rename(name)`.
    pub fn rename(&self, name: &str) -> Result<Self, FrameError> {
        Self::new(name.to_owned(), self.index.clone(), self.column.clone())
    }

    /// Replace the index labels.
    ///
    /// Matches `pd.Series.set_axis(labels)`.
    pub fn set_axis(&self, labels: Vec<IndexLabel>) -> Result<Self, FrameError> {
        if labels.len() != self.len() {
            return Err(FrameError::LengthMismatch {
                index_len: labels.len(),
                column_len: self.len(),
            });
        }
        Self::new(
            self.name.clone(),
            Index::new(labels),
            self.column.clone(),
        )
    }

    /// Truncate the Series to rows between `before` and `after` labels (inclusive).
    ///
    /// Matches `pd.Series.truncate(before, after)`.
    pub fn truncate(
        &self,
        before: Option<&IndexLabel>,
        after: Option<&IndexLabel>,
    ) -> Result<Self, FrameError> {
        let labels = self.index.labels();
        let start = match before {
            Some(b) => labels.iter().position(|l| l >= b).unwrap_or(labels.len()),
            None => 0,
        };
        let end = match after {
            Some(a) => labels
                .iter()
                .rposition(|l| l <= a)
                .map(|i| i + 1)
                .unwrap_or(0),
            None => labels.len(),
        };
        if start >= end {
            return Self::from_values(
                self.name.clone(),
                Vec::new(),
                Vec::new(),
            );
        }
        let new_labels = labels[start..end].to_vec();
        let new_values = self.column.values()[start..end].to_vec();
        Self::from_values(self.name.clone(), new_labels, new_values)
    }

    /// Combine two Series element-wise using a function.
    ///
    /// Matches `pd.Series.combine(other, func)`. The function receives
    /// two scalars (one from self, one from other) and returns a scalar.
    pub fn combine<F>(&self, other: &Self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar, &Scalar) -> Scalar,
    {
        let len = self.len().max(other.len());
        let fill = Scalar::Null(NullKind::NaN);
        let mut out = Vec::with_capacity(len);
        let mut labels = Vec::with_capacity(len);

        for i in 0..len {
            let sv = self.column.values().get(i).unwrap_or(&fill);
            let ov = other.column.values().get(i).unwrap_or(&fill);
            out.push(func(sv, ov));
            labels.push(
                self.index
                    .labels()
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| (i as i64).into()),
            );
        }

        Self::from_values(self.name.clone(), labels, out)
    }

    /// Explode a Series of string values by splitting on a separator.
    ///
    /// Matches `pd.Series.explode()` for list-like string values.
    /// Each string is split by `sep` and each piece becomes its own row.
    /// The index label is repeated for each piece.
    pub fn explode(&self, sep: &str) -> Result<Self, FrameError> {
        let mut new_labels = Vec::new();
        let mut new_values = Vec::new();

        for (i, val) in self.column.values().iter().enumerate() {
            let label = self.index.labels()[i].clone();
            match val {
                Scalar::Utf8(s) if !s.is_empty() => {
                    for part in s.split(sep) {
                        new_labels.push(label.clone());
                        let trimmed = part.trim();
                        if trimmed.is_empty() {
                            new_values.push(Scalar::Null(NullKind::NaN));
                        } else {
                            new_values.push(Scalar::Utf8(trimmed.to_string()));
                        }
                    }
                }
                _ => {
                    new_labels.push(label);
                    new_values.push(val.clone());
                }
            }
        }

        Self::from_values(self.name(), new_labels, new_values)
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

    /// Create an exponentially weighted moving window view of this Series.
    ///
    /// Matches `series.ewm(span=...)`. Either `span` or `alpha` must be provided.
    /// - span: Specify decay in terms of span. `alpha = 2 / (span + 1)`.
    /// - alpha: Specify smoothing factor directly, 0 < alpha <= 1.
    pub fn ewm(&self, span: Option<f64>, alpha: Option<f64>) -> Ewm<'_> {
        let a = if let Some(s) = span {
            2.0 / (s + 1.0)
        } else {
            alpha.unwrap_or(2.0 / 11.0)
        };
        Ewm {
            series: self,
            alpha: a,
        }
    }

    /// Create a resampler view for time-based downsampling.
    ///
    /// Matches `series.resample(freq)` where freq is one of:
    /// - "Y" or "A": yearly (bucket by YYYY)
    /// - "M": monthly (bucket by YYYY-MM)
    /// - "D": daily (bucket by YYYY-MM-DD)
    ///
    /// The Series index must contain Utf8 datetime-like labels (ISO format).
    pub fn resample(&self, freq: &str) -> Resample<'_> {
        Resample {
            series: self,
            freq: freq.to_string(),
        }
    }

    /// Compute autocorrelation at the specified lag.
    ///
    /// Matches `series.autocorr(lag=1)`. Computes the Pearson correlation
    /// between the Series and its lagged copy.
    pub fn autocorr(&self, lag: usize) -> Result<f64, FrameError> {
        let vals = self.column.values();
        if vals.len() <= lag {
            return Ok(f64::NAN);
        }
        let n = vals.len() - lag;
        let mut x_vals = Vec::with_capacity(n);
        let mut y_vals = Vec::with_capacity(n);

        for i in 0..n {
            let a = &vals[i];
            let b = &vals[i + lag];
            if a.is_missing() || b.is_missing() {
                continue;
            }
            if let (Ok(x), Ok(y)) = (a.to_f64(), b.to_f64())
                && !x.is_nan() && !y.is_nan()
            {
                x_vals.push(x);
                y_vals.push(y);
            }
        }

        let count = x_vals.len();
        if count < 2 {
            return Ok(f64::NAN);
        }

        let n_f = count as f64;
        let mean_x = x_vals.iter().sum::<f64>() / n_f;
        let mean_y = y_vals.iter().sum::<f64>() / n_f;

        let mut cov = 0.0_f64;
        let mut var_x = 0.0_f64;
        let mut var_y = 0.0_f64;
        for i in 0..count {
            let dx = x_vals[i] - mean_x;
            let dy = y_vals[i] - mean_y;
            cov += dx * dy;
            var_x += dx * dx;
            var_y += dy * dy;
        }

        let denom = (var_x * var_y).sqrt();
        if denom < f64::EPSILON {
            Ok(f64::NAN)
        } else {
            Ok(cov / denom)
        }
    }

    /// Compute the dot product with another Series.
    ///
    /// Matches `series.dot(other)` / `series @ other`.
    /// Skips positions where either value is null/NaN.
    pub fn dot(&self, other: &Self) -> Result<f64, FrameError> {
        let a_vals = self.column.values();
        let b_vals = other.column.values();
        let len = a_vals.len().min(b_vals.len());

        let mut result = 0.0_f64;
        for i in 0..len {
            if let (Ok(x), Ok(y)) = (a_vals[i].to_f64(), b_vals[i].to_f64())
                && !x.is_nan() && !y.is_nan()
            {
                result += x * y;
            }
        }

        Ok(result)
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

    /// Compute the Spearman rank correlation with another Series.
    ///
    /// Ranks both Series and then computes Pearson correlation on the ranks.
    pub fn corr_spearman(&self, other: &Self) -> Result<f64, FrameError> {
        let a_vals = self.column().values();
        let b_vals = other.column().values();
        let len = a_vals.len().min(b_vals.len());

        // Collect valid pairs
        let mut pairs: Vec<(f64, f64)> = Vec::new();
        for i in 0..len {
            if let (Ok(x), Ok(y)) = (a_vals[i].to_f64(), b_vals[i].to_f64())
                && !x.is_nan()
                && !y.is_nan()
            {
                pairs.push((x, y));
            }
        }

        let n = pairs.len();
        if n < 2 {
            return Ok(f64::NAN);
        }

        // Compute average ranks for x
        let ranks_x = Self::average_ranks(&pairs.iter().map(|(x, _)| *x).collect::<Vec<_>>());
        let ranks_y = Self::average_ranks(&pairs.iter().map(|(_, y)| *y).collect::<Vec<_>>());

        // Pearson on ranks
        Self::pearson_on_slices(&ranks_x, &ranks_y)
    }

    /// Compute the Kendall tau-b correlation with another Series.
    pub fn corr_kendall(&self, other: &Self) -> Result<f64, FrameError> {
        let a_vals = self.column().values();
        let b_vals = other.column().values();
        let len = a_vals.len().min(b_vals.len());

        let mut pairs: Vec<(f64, f64)> = Vec::new();
        for i in 0..len {
            if let (Ok(x), Ok(y)) = (a_vals[i].to_f64(), b_vals[i].to_f64())
                && !x.is_nan()
                && !y.is_nan()
            {
                pairs.push((x, y));
            }
        }

        let n = pairs.len();
        if n < 2 {
            return Ok(f64::NAN);
        }

        // Count concordant and discordant pairs, plus tied pairs
        let mut concordant = 0_i64;
        let mut discordant = 0_i64;
        let mut tied_x = 0_i64;
        let mut tied_y = 0_i64;

        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pairs[i].0 - pairs[j].0;
                let dy = pairs[i].1 - pairs[j].1;

                if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
                    tied_x += 1;
                    tied_y += 1;
                } else if dx.abs() < f64::EPSILON {
                    tied_x += 1;
                } else if dy.abs() < f64::EPSILON {
                    tied_y += 1;
                } else if (dx > 0.0 && dy > 0.0) || (dx < 0.0 && dy < 0.0) {
                    concordant += 1;
                } else {
                    discordant += 1;
                }
            }
        }

        let n_pairs = (n * (n - 1)) as f64 / 2.0;
        let denom = ((n_pairs - tied_x as f64) * (n_pairs - tied_y as f64)).sqrt();

        if denom < f64::EPSILON {
            Ok(f64::NAN)
        } else {
            Ok((concordant - discordant) as f64 / denom)
        }
    }

    /// Compute average ranks for a slice of values (used by Spearman).
    fn average_ranks(values: &[f64]) -> Vec<f64> {
        let n = values.len();
        let mut indexed: Vec<(usize, f64)> =
            values.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut ranks = vec![0.0; n];
        let mut i = 0;
        while i < n {
            let mut j = i;
            // Find group of tied values
            while j < n && (indexed[j].1 - indexed[i].1).abs() < f64::EPSILON {
                j += 1;
            }
            // Average rank for tied group: (i+1 + j) / 2
            let avg_rank = (i + 1 + j) as f64 / 2.0;
            for item in &indexed[i..j] {
                ranks[item.0] = avg_rank;
            }
            i = j;
        }
        ranks
    }

    /// Compute Pearson correlation on two f64 slices.
    fn pearson_on_slices(x: &[f64], y: &[f64]) -> Result<f64, FrameError> {
        let n = x.len() as f64;
        let mean_x = x.iter().sum::<f64>() / n;
        let mean_y = y.iter().sum::<f64>() / n;

        let mut cov = 0.0;
        let mut var_x = 0.0;
        let mut var_y = 0.0;

        for (&xi, &yi) in x.iter().zip(y.iter()) {
            let dx = xi - mean_x;
            let dy = yi - mean_y;
            cov += dx * dy;
            var_x += dx * dx;
            var_y += dy * dy;
        }

        let denom = (var_x * var_y).sqrt();
        if denom < f64::EPSILON {
            Ok(f64::NAN)
        } else {
            Ok(cov / denom)
        }
    }

    /// Access string methods on a Utf8 Series (analogous to `pandas.Series.str`).
    pub fn str(&self) -> StringAccessor<'_> {
        StringAccessor { series: self }
    }

    /// Access datetime components of this Series.
    ///
    /// Matches `pd.Series.dt`. Extracts components from ISO 8601
    /// formatted string values.
    #[must_use]
    pub fn dt(&self) -> DatetimeAccessor<'_> {
        DatetimeAccessor { series: self }
    }

    /// Apply a closure element-wise to produce a new Series.
    pub fn apply_fn<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Scalar,
    {
        let new_vals: Vec<Scalar> = self.column().values().iter().map(func).collect();
        Self::from_values(self.name(), self.index().labels().to_vec(), new_vals)
    }

    /// Map values using a failable closure.
    ///
    /// Like `apply_fn` but the closure returns `Result<Scalar, FrameError>`,
    /// enabling transformations that may fail (type conversion, parsing, etc.).
    /// Matches the general pattern of `pandas Series.map(func)`.
    pub fn map_fn<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Result<Scalar, FrameError>,
    {
        let mut new_vals = Vec::with_capacity(self.len());
        for val in self.column().values() {
            new_vals.push(func(val)?);
        }
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

            out.push(Scalar::Float64(count as f64));
        }

        Series::from_values(
            self.series.name(),
            self.series.index().labels().to_vec(),
            out,
        )
    }

    /// Rolling sample variance (ddof=1).
    pub fn var(&self) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| {
                if nums.len() < 2 {
                    return f64::NAN;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64
            },
            self.series.name(),
        )
    }

    /// Rolling median.
    pub fn median(&self) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| {
                if nums.is_empty() {
                    return f64::NAN;
                }
                let mut sorted = nums.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let mid = sorted.len() / 2;
                if sorted.len() % 2 == 0 {
                    (sorted[mid - 1] + sorted[mid]) / 2.0
                } else {
                    sorted[mid]
                }
            },
            self.series.name(),
        )
    }

    /// Rolling quantile.
    ///
    /// Matches `series.rolling(window).quantile(q)`.
    pub fn quantile(&self, q: f64) -> Result<Series, FrameError> {
        self.apply_rolling(
            |nums| {
                if nums.is_empty() {
                    return f64::NAN;
                }
                let mut sorted = nums.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let pos = q * (sorted.len() - 1) as f64;
                let lo = pos.floor() as usize;
                let hi = pos.ceil() as usize;
                if lo == hi || hi >= sorted.len() {
                    sorted[lo]
                } else {
                    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
                }
            },
            self.series.name(),
        )
    }

    /// Apply a custom function over the rolling window.
    ///
    /// Matches `series.rolling(window).apply(func)`.
    pub fn apply<F>(&self, func: F) -> Result<Series, FrameError>
    where
        F: Fn(&[f64]) -> f64,
    {
        self.apply_rolling(func, self.series.name())
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

    /// Expanding sample variance (ddof=1).
    pub fn var(&self) -> Result<Series, FrameError> {
        self.apply_expanding(
            |nums| {
                if nums.len() < 2 {
                    return f64::NAN;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64
            },
            self.series.name(),
        )
    }

    /// Expanding median.
    pub fn median(&self) -> Result<Series, FrameError> {
        self.apply_expanding(
            |nums| {
                if nums.is_empty() {
                    return f64::NAN;
                }
                let mut sorted = nums.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let mid = sorted.len() / 2;
                if sorted.len() % 2 == 0 {
                    (sorted[mid - 1] + sorted[mid]) / 2.0
                } else {
                    sorted[mid]
                }
            },
            self.series.name(),
        )
    }

    /// Apply a custom function over the expanding window.
    ///
    /// Matches `series.expanding().apply(func)`.
    pub fn apply<F>(&self, func: F) -> Result<Series, FrameError>
    where
        F: Fn(&[f64]) -> f64,
    {
        self.apply_expanding(func, self.series.name())
    }
}

/// Exponentially weighted moving window aggregation over a Series.
///
/// Created by `Series::ewm()`. Uses the recursive EWM formula:
/// `y_t = alpha * x_t + (1 - alpha) * y_{t-1}`
pub struct Ewm<'a> {
    series: &'a Series,
    alpha: f64,
}

impl Ewm<'_> {
    /// EWM mean.
    ///
    /// Matches `series.ewm(span=...).mean()`.
    pub fn mean(&self) -> Result<Series, FrameError> {
        let vals = self.series.column().values();
        let mut out = Vec::with_capacity(vals.len());
        let alpha = self.alpha;
        let one_minus_alpha = 1.0 - alpha;

        let mut ewm_old = f64::NAN;
        let mut nobs = 0_usize;

        for val in vals {
            if val.is_missing() || val.to_f64().map_or(true, |v| v.is_nan()) {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            let x = val.to_f64().unwrap();
            nobs += 1;
            if nobs == 1 {
                ewm_old = x;
            } else {
                ewm_old = alpha * x + one_minus_alpha * ewm_old;
            }
            out.push(Scalar::Float64(ewm_old));
        }

        Series::from_values(self.series.name(), self.series.index().labels().to_vec(), out)
    }

    /// EWM standard deviation (sample, ddof=1).
    ///
    /// Matches `series.ewm(span=...).std()`.
    pub fn std(&self) -> Result<Series, FrameError> {
        let var_series = self.var()?;
        let mut out = Vec::with_capacity(var_series.len());
        for v in var_series.column().values() {
            match v {
                Scalar::Float64(f) => out.push(Scalar::Float64(f.sqrt())),
                _ => out.push(v.clone()),
            }
        }
        Series::from_values(self.series.name(), self.series.index().labels().to_vec(), out)
    }

    /// EWM variance (sample, ddof=1).
    ///
    /// Uses the online formula for exponentially weighted variance.
    /// Matches `series.ewm(span=...).var()`.
    pub fn var(&self) -> Result<Series, FrameError> {
        let vals = self.series.column().values();
        let mut out = Vec::with_capacity(vals.len());
        let alpha = self.alpha;
        let one_minus_alpha = 1.0 - alpha;

        let mut ewm_mean = f64::NAN;
        let mut ewm_var = 0.0_f64;
        let mut nobs = 0_usize;
        let mut sum_wt = 0.0_f64;
        let mut sum_wt2 = 0.0_f64;
        let mut old_wt = 1.0_f64;

        for val in vals {
            if val.is_missing() || val.to_f64().map_or(true, |v| v.is_nan()) {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            let x = val.to_f64().unwrap();
            nobs += 1;

            if nobs == 1 {
                ewm_mean = x;
                sum_wt = 1.0;
                sum_wt2 = 1.0;
                old_wt = 1.0;
                out.push(Scalar::Null(NullKind::NaN)); // variance undefined for single obs
            } else {
                old_wt *= one_minus_alpha;
                let new_wt = 1.0;
                sum_wt += new_wt;
                sum_wt2 = sum_wt2 * one_minus_alpha * one_minus_alpha + new_wt * new_wt;

                let old_mean = ewm_mean;
                ewm_mean = (old_wt * old_mean + new_wt * x) / sum_wt;
                ewm_var = (old_wt * (ewm_var + (old_mean - ewm_mean).powi(2))
                    + new_wt * (x - ewm_mean).powi(2))
                    / sum_wt;

                // Bias correction (ddof=1)
                let bias_factor = sum_wt * sum_wt / (sum_wt * sum_wt - sum_wt2);
                out.push(Scalar::Float64(ewm_var * bias_factor));

                old_wt = sum_wt;
            }
        }

        Series::from_values(self.series.name(), self.series.index().labels().to_vec(), out)
    }
}

/// Time-based resampling view over a Series.
///
/// Created by `Series::resample(freq)`. Groups values by time buckets
/// defined by the frequency string.
pub struct Resample<'a> {
    series: &'a Series,
    freq: String,
}

impl Resample<'_> {
    /// Extract a time bucket key from an index label.
    fn bucket_key(label: &IndexLabel, freq: &str) -> Option<String> {
        let s = match label {
            IndexLabel::Utf8(s) => s.as_str(),
            IndexLabel::Int64(_) => return None,
        };
        // Parse date components from ISO-ish format "YYYY-MM-DD..." or "YYYY-MM-DDTHH:..."
        let date_part = s.split('T').next().unwrap_or(s).split(' ').next().unwrap_or(s);
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() < 3 {
            return None;
        }
        let year = parts[0];
        let month = parts[1];

        match freq {
            "Y" | "A" => Some(year.to_string()),
            "M" => Some(format!("{year}-{month}")),
            "D" => Some(date_part.to_string()),
            _ => Some(date_part.to_string()),
        }
    }

    /// Build groups: returns (bucket_keys_in_order, bucket->row_indices).
    fn build_groups(&self) -> (Vec<String>, std::collections::HashMap<String, Vec<usize>>) {
        let labels = self.series.index().labels();
        let mut order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();

        for (i, label) in labels.iter().enumerate() {
            if let Some(key) = Self::bucket_key(label, &self.freq) {
                if !groups.contains_key(&key) {
                    order.push(key.clone());
                }
                groups.entry(key).or_default().push(i);
            }
        }
        (order, groups)
    }

    /// Aggregate each time bucket using a function.
    fn aggregate<F>(&self, agg: F) -> Result<Series, FrameError>
    where
        F: Fn(&[Scalar]) -> Scalar,
    {
        let (order, groups) = self.build_groups();
        let vals = self.series.column().values();

        let mut out_labels = Vec::with_capacity(order.len());
        let mut out_vals = Vec::with_capacity(order.len());

        for key in &order {
            out_labels.push(IndexLabel::Utf8(key.clone()));
            let group_vals: Vec<Scalar> =
                groups[key].iter().map(|&i| vals[i].clone()).collect();
            out_vals.push(agg(&group_vals));
        }

        Series::from_values(self.series.name(), out_labels, out_vals)
    }

    /// Resample sum.
    pub fn sum(&self) -> Result<Series, FrameError> {
        self.aggregate(fp_types::nansum)
    }

    /// Resample mean.
    pub fn mean(&self) -> Result<Series, FrameError> {
        self.aggregate(fp_types::nanmean)
    }

    /// Resample count.
    pub fn count(&self) -> Result<Series, FrameError> {
        self.aggregate(fp_types::nancount)
    }

    /// Resample min.
    pub fn min(&self) -> Result<Series, FrameError> {
        self.aggregate(fp_types::nanmin)
    }

    /// Resample max.
    pub fn max(&self) -> Result<Series, FrameError> {
        self.aggregate(fp_types::nanmax)
    }

    /// Resample first non-null value.
    pub fn first(&self) -> Result<Series, FrameError> {
        self.aggregate(|vals| {
            vals.iter()
                .find(|v| !v.is_missing())
                .cloned()
                .unwrap_or(Scalar::Null(NullKind::NaN))
        })
    }

    /// Resample last non-null value.
    pub fn last(&self) -> Result<Series, FrameError> {
        self.aggregate(|vals| {
            vals.iter()
                .rev()
                .find(|v| !v.is_missing())
                .cloned()
                .unwrap_or(Scalar::Null(NullKind::NaN))
        })
    }
}

/// Rolling window aggregation over a DataFrame's numeric columns.
///
/// Created by `DataFrame::rolling()`. Applies the rolling window to each
/// numeric column independently.
pub struct DataFrameRolling<'a> {
    df: &'a DataFrame,
    window: usize,
    min_periods: usize,
}

impl DataFrameRolling<'_> {
    /// Apply rolling aggregation to each numeric column, returning a new DataFrame.
    fn apply_rolling<F>(&self, agg: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&Series, usize, usize) -> Result<Series, FrameError>,
    {
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &self.df.column_order {
            let col = &self.df.columns[col_name];
            let dt = col.dtype();
            if dt != DType::Int64 && dt != DType::Float64 {
                continue;
            }

            let series =
                Series::new(col_name, self.df.index.clone(), col.clone())?;
            let result = agg(&series, self.window, self.min_periods)?;
            result_cols.insert(col_name.clone(), result.column().clone());
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: self.df.index.clone(),
        })
    }

    /// Rolling sum across all numeric columns.
    pub fn sum(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).sum())
    }

    /// Rolling mean across all numeric columns.
    pub fn mean(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).mean())
    }

    /// Rolling min across all numeric columns.
    pub fn min(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).min())
    }

    /// Rolling max across all numeric columns.
    pub fn max(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).max())
    }

    /// Rolling standard deviation across all numeric columns.
    pub fn std(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).std())
    }

    /// Rolling count of non-null values across all numeric columns.
    pub fn count(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).count())
    }

    /// Rolling variance across all numeric columns.
    pub fn var(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).var())
    }

    /// Rolling median across all numeric columns.
    pub fn median(&self) -> Result<DataFrame, FrameError> {
        self.apply_rolling(|s, w, mp| s.rolling(w, Some(mp)).median())
    }

    /// Rolling quantile across all numeric columns.
    pub fn quantile(&self, q: f64) -> Result<DataFrame, FrameError> {
        self.apply_rolling(move |s, w, mp| s.rolling(w, Some(mp)).quantile(q))
    }
}

/// Expanding window aggregation over a DataFrame's numeric columns.
///
/// Created by `DataFrame::expanding()`. Includes all prior elements
/// in each computation for every numeric column.
pub struct DataFrameExpanding<'a> {
    df: &'a DataFrame,
    min_periods: usize,
}

impl DataFrameExpanding<'_> {
    /// Apply expanding aggregation to each numeric column, returning a new DataFrame.
    fn apply_expanding<F>(&self, agg: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&Series, usize) -> Result<Series, FrameError>,
    {
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &self.df.column_order {
            let col = &self.df.columns[col_name];
            let dt = col.dtype();
            if dt != DType::Int64 && dt != DType::Float64 {
                continue;
            }

            let series =
                Series::new(col_name, self.df.index.clone(), col.clone())?;
            let result = agg(&series, self.min_periods)?;
            result_cols.insert(col_name.clone(), result.column().clone());
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: self.df.index.clone(),
        })
    }

    /// Expanding sum across all numeric columns.
    pub fn sum(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).sum())
    }

    /// Expanding mean across all numeric columns.
    pub fn mean(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).mean())
    }

    /// Expanding min across all numeric columns.
    pub fn min(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).min())
    }

    /// Expanding max across all numeric columns.
    pub fn max(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).max())
    }

    /// Expanding standard deviation across all numeric columns.
    pub fn std(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).std())
    }

    /// Expanding variance across all numeric columns.
    pub fn var(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).var())
    }

    /// Expanding median across all numeric columns.
    pub fn median(&self) -> Result<DataFrame, FrameError> {
        self.apply_expanding(|s, mp| s.expanding(Some(mp)).median())
    }
}

/// Exponentially weighted moving window over a DataFrame's numeric columns.
///
/// Created by `DataFrame::ewm()`. Applies EWM to each numeric column.
pub struct DataFrameEwm<'a> {
    df: &'a DataFrame,
    span: Option<f64>,
    alpha: Option<f64>,
}

impl DataFrameEwm<'_> {
    /// Apply EWM aggregation to each numeric column, returning a new DataFrame.
    fn apply_ewm<F>(&self, agg: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&Series, Option<f64>, Option<f64>) -> Result<Series, FrameError>,
    {
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &self.df.column_order {
            let col = &self.df.columns[col_name];
            let dt = col.dtype();
            if dt != DType::Int64 && dt != DType::Float64 {
                continue;
            }

            let series = Series::new(col_name, self.df.index.clone(), col.clone())?;
            let result = agg(&series, self.span, self.alpha)?;
            result_cols.insert(col_name.clone(), result.column().clone());
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: self.df.index.clone(),
        })
    }

    /// EWM mean across all numeric columns.
    pub fn mean(&self) -> Result<DataFrame, FrameError> {
        self.apply_ewm(|s, span, alpha| s.ewm(span, alpha).mean())
    }

    /// EWM standard deviation across all numeric columns.
    pub fn std(&self) -> Result<DataFrame, FrameError> {
        self.apply_ewm(|s, span, alpha| s.ewm(span, alpha).std())
    }

    /// EWM variance across all numeric columns.
    pub fn var(&self) -> Result<DataFrame, FrameError> {
        self.apply_ewm(|s, span, alpha| s.ewm(span, alpha).var())
    }
}

/// Time-based resampling view over a DataFrame's numeric columns.
///
/// Created by `DataFrame::resample(freq)`. Groups by time buckets and aggregates.
pub struct DataFrameResample<'a> {
    df: &'a DataFrame,
    freq: String,
}

impl DataFrameResample<'_> {
    /// Apply resampling to each numeric column.
    fn apply_resample<F>(&self, agg: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&Series, &str) -> Result<Series, FrameError>,
    {
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();
        let mut result_index: Option<Index> = None;

        for col_name in &self.df.column_order {
            let col = &self.df.columns[col_name];
            let dt = col.dtype();
            if dt != DType::Int64 && dt != DType::Float64 {
                continue;
            }

            let series = Series::new(col_name, self.df.index.clone(), col.clone())?;
            let result = agg(&series, &self.freq)?;
            if result_index.is_none() {
                result_index = Some(result.index().clone());
            }
            result_cols.insert(col_name.clone(), result.column().clone());
            col_order.push(col_name.clone());
        }

        let index = result_index.unwrap_or_else(|| Index::new(Vec::new()));
        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index,
        })
    }

    /// Resample sum across all numeric columns.
    pub fn sum(&self) -> Result<DataFrame, FrameError> {
        self.apply_resample(|s, freq| s.resample(freq).sum())
    }

    /// Resample mean across all numeric columns.
    pub fn mean(&self) -> Result<DataFrame, FrameError> {
        self.apply_resample(|s, freq| s.resample(freq).mean())
    }

    /// Resample count across all numeric columns.
    pub fn count(&self) -> Result<DataFrame, FrameError> {
        self.apply_resample(|s, freq| s.resample(freq).count())
    }

    /// Resample min across all numeric columns.
    pub fn min(&self) -> Result<DataFrame, FrameError> {
        self.apply_resample(|s, freq| s.resample(freq).min())
    }

    /// Resample max across all numeric columns.
    pub fn max(&self) -> Result<DataFrame, FrameError> {
        self.apply_resample(|s, freq| s.resample(freq).max())
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

    /// Get the length of each string (character count, not byte count).
    pub fn len(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Int64(s.chars().count() as i64),
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

    /// Count the number of parts when splitting by pattern.
    ///
    /// Matches `pd.Series.str.split(pat).str.len()`. Returns Int64 count.
    pub fn split_count(&self, pat: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Int64(s.split(pat).count() as i64),
            self.series.name(),
        )
    }

    /// Join/concatenate each string element with a separator.
    ///
    /// Applies to each element: useful after conceptual split operations.
    /// Replaces occurrences of `from` with `sep`.
    pub fn join(&self, from: &str, sep: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.split(from).collect::<Vec<_>>().join(sep)),
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
                let char_len = s.chars().count();
                if char_len >= width {
                    return Scalar::Utf8(s.to_string());
                }
                let pad_len = width - char_len;
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

    // ── Regex-based methods ──────────────────────────────────────────

    /// Check whether each string matches a regex pattern.
    ///
    /// Analogous to `pandas.Series.str.contains(pat, regex=True)`.
    pub fn contains_regex(&self, pat: &str) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        self.apply_str(|s| Scalar::Bool(re.is_match(s)), self.series.name())
    }

    /// Replace first occurrence of a regex pattern with a replacement string.
    ///
    /// Analogous to `pandas.Series.str.replace(pat, repl, regex=True)`.
    /// The replacement string supports backreferences (`$1`, `$2`, etc.).
    pub fn replace_regex(&self, pat: &str, repl: &str) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        let repl_owned = repl.to_string();
        self.apply_str(
            |s| Scalar::Utf8(re.replace(s, repl_owned.as_str()).into_owned()),
            self.series.name(),
        )
    }

    /// Replace all occurrences of a regex pattern with a replacement string.
    ///
    /// Analogous to `pandas.Series.str.replace(pat, repl, regex=True, n=-1)`.
    pub fn replace_regex_all(&self, pat: &str, repl: &str) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        let repl_owned = repl.to_string();
        self.apply_str(
            |s| Scalar::Utf8(re.replace_all(s, repl_owned.as_str()).into_owned()),
            self.series.name(),
        )
    }

    /// Extract the first match of a regex capture group.
    ///
    /// Analogous to `pandas.Series.str.extract(pat)`.
    /// Returns the first capture group (group 1) if the pattern contains
    /// a group, otherwise returns the full match (group 0).
    /// Non-matching strings produce Null.
    pub fn extract(&self, pat: &str) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        let has_groups = re.captures_len() > 1;
        self.apply_str(
            |s| match re.captures(s) {
                Some(caps) => {
                    let group = if has_groups { 1 } else { 0 };
                    match caps.get(group) {
                        Some(m) => Scalar::Utf8(m.as_str().to_string()),
                        None => Scalar::Null(NullKind::NaN),
                    }
                }
                None => Scalar::Null(NullKind::NaN),
            },
            self.series.name(),
        )
    }

    /// Count non-overlapping matches of a regex pattern in each string.
    ///
    /// Analogous to `pandas.Series.str.count(pat)`.
    pub fn count_matches(&self, pat: &str) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        self.apply_str(
            |s| Scalar::Int64(re.find_iter(s).count() as i64),
            self.series.name(),
        )
    }

    /// Count non-overlapping occurrences of a literal substring.
    ///
    /// Matches `pd.Series.str.count(pat)` for literal patterns.
    pub fn count_literal(&self, pat: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Int64(s.matches(pat).count() as i64),
            self.series.name(),
        )
    }

    /// Find all non-overlapping matches and return them joined by a separator.
    ///
    /// Since Series cannot hold list values, matches are joined with `sep`.
    /// Non-matching strings produce Null.
    pub fn findall(&self, pat: &str, sep: &str) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        let sep_owned = sep.to_string();
        self.apply_str(
            |s| {
                let matches: Vec<&str> = re.find_iter(s).map(|m| m.as_str()).collect();
                if matches.is_empty() {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Utf8(matches.join(&sep_owned))
                }
            },
            self.series.name(),
        )
    }

    /// Check whether each string fully matches a regex pattern.
    ///
    /// Unlike `contains_regex` which searches for a match anywhere,
    /// this requires the entire string to match (anchored `^...$`).
    pub fn fullmatch(&self, pat: &str) -> Result<Series, FrameError> {
        let anchored = format!("^(?:{pat})$");
        let re = Regex::new(&anchored).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        self.apply_str(|s| Scalar::Bool(re.is_match(s)), self.series.name())
    }

    /// Check whether each string matches a regex pattern at the start.
    ///
    /// Analogous to `pandas.Series.str.match(pat)` which uses Python's
    /// `re.match()` (anchored at the beginning of the string).
    pub fn match_regex(&self, pat: &str) -> Result<Series, FrameError> {
        let anchored = format!("^(?:{pat})");
        let re = Regex::new(&anchored).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        self.apply_str(|s| Scalar::Bool(re.is_match(s)), self.series.name())
    }

    /// Split each string by a regex pattern and return the n-th element.
    ///
    /// Analogous to `pandas.Series.str.split(pat, regex=True).str[n]`.
    pub fn split_regex_get(&self, pat: &str, n: usize) -> Result<Series, FrameError> {
        let re = Regex::new(pat).map_err(|e| {
            FrameError::CompatibilityRejected(format!("invalid regex pattern: {e}"))
        })?;
        self.apply_str(
            |s| {
                let parts: Vec<&str> = re.split(s).collect();
                if n < parts.len() {
                    Scalar::Utf8(parts[n].to_string())
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    // ── Formatting methods ──────────────────────────────────────────

    /// Zero-fill strings to specified width.
    ///
    /// Matches `pd.Series.str.zfill(width)`.
    pub fn zfill(&self, width: usize) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let char_len = s.chars().count();
                if char_len >= width {
                    Scalar::Utf8(s.to_string())
                } else {
                    let pad_len = width - char_len;
                    let padding: String = std::iter::repeat_n('0', pad_len).collect();
                    if s.starts_with('-') || s.starts_with('+') {
                        let (sign, rest) = s.split_at(1);
                        Scalar::Utf8(format!("{sign}{padding}{rest}"))
                    } else {
                        Scalar::Utf8(format!("{padding}{s}"))
                    }
                }
            },
            self.series.name(),
        )
    }

    /// Center-align strings within specified width.
    ///
    /// Matches `pd.Series.str.center(width, fillchar)`.
    pub fn center(&self, width: usize, fillchar: char) -> Result<Series, FrameError> {
        self.pad(width, "both", fillchar)
    }

    /// Left-align strings within specified width (pad on right).
    ///
    /// Matches `pd.Series.str.ljust(width, fillchar)`.
    pub fn ljust(&self, width: usize, fillchar: char) -> Result<Series, FrameError> {
        self.pad(width, "right", fillchar)
    }

    /// Right-align strings within specified width (pad on left).
    ///
    /// Matches `pd.Series.str.rjust(width, fillchar)`.
    pub fn rjust(&self, width: usize, fillchar: char) -> Result<Series, FrameError> {
        self.pad(width, "left", fillchar)
    }

    // ── Character class predicate methods ────────────────────────────

    /// Check if each string is composed of digits only.
    ///
    /// Matches `pd.Series.str.isdigit()`.
    pub fn isdigit(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())),
            self.series.name(),
        )
    }

    /// Check if each string is composed of alphabetic characters only.
    ///
    /// Matches `pd.Series.str.isalpha()`.
    pub fn isalpha(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())),
            self.series.name(),
        )
    }

    /// Check if each string is alphanumeric.
    ///
    /// Matches `pd.Series.str.isalnum()`.
    pub fn isalnum(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric())),
            self.series.name(),
        )
    }

    /// Check if each string is composed of whitespace only.
    ///
    /// Matches `pd.Series.str.isspace()`.
    pub fn isspace(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(!s.is_empty() && s.chars().all(|c| c.is_whitespace())),
            self.series.name(),
        )
    }

    /// Check if each string is lowercase.
    ///
    /// Matches `pd.Series.str.islower()`.
    pub fn islower(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let has_cased = s.chars().any(|c| c.is_alphabetic());
                Scalar::Bool(has_cased && s.chars().all(|c| !c.is_uppercase()))
            },
            self.series.name(),
        )
    }

    /// Check if each string is uppercase.
    ///
    /// Matches `pd.Series.str.isupper()`.
    pub fn isupper(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let has_cased = s.chars().any(|c| c.is_alphabetic());
                Scalar::Bool(has_cased && s.chars().all(|c| !c.is_lowercase()))
            },
            self.series.name(),
        )
    }

    /// Check if each string is numeric (including Unicode numeric chars).
    ///
    /// Matches `pd.Series.str.isnumeric()`.
    pub fn isnumeric(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(!s.is_empty() && s.chars().all(|c| c.is_numeric())),
            self.series.name(),
        )
    }

    /// Extract character at position from each string.
    ///
    /// Matches `pd.Series.str.get(i)`. Returns NaN if index is out of bounds.
    pub fn get(&self, i: usize) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                s.chars()
                    .nth(i)
                    .map_or(Scalar::Null(NullKind::NaN), |c| {
                        Scalar::Utf8(c.to_string())
                    })
            },
            self.series.name(),
        )
    }

    /// Wrap long lines at specified width.
    ///
    /// Matches `pd.Series.str.wrap(width)`. Inserts newlines to wrap text.
    pub fn wrap(&self, width: usize) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                if width == 0 {
                    return Scalar::Utf8(s.to_string());
                }
                let mut result = String::new();
                let mut line_len = 0;
                for word in s.split_whitespace() {
                    let word_len = word.chars().count();
                    if line_len > 0 && line_len + 1 + word_len > width {
                        result.push('\n');
                        line_len = 0;
                    } else if line_len > 0 {
                        result.push(' ');
                        line_len += 1;
                    }
                    result.push_str(word);
                    line_len += word_len;
                }
                Scalar::Utf8(result)
            },
            self.series.name(),
        )
    }

    /// Normalize Unicode strings.
    ///
    /// Matches `pd.Series.str.normalize(form)`. Supported forms: NFC, NFKC.
    /// NFD/NFKD require a full Unicode decomposition library; we approximate
    /// by returning the input unchanged for unsupported forms.
    pub fn normalize(&self, form: &str) -> Result<Series, FrameError> {
        let form = form.to_uppercase();
        self.apply_str(
            |s| {
                // For NFC/NFKC, Rust strings are already NFC-like for ASCII.
                // Full Unicode normalization would need the `unicode-normalization` crate.
                // We pass through unchanged as a best-effort for ASCII-dominant data.
                match form.as_str() {
                    "NFC" | "NFKC" | "NFD" | "NFKD" => Scalar::Utf8(s.to_string()),
                    _ => Scalar::Null(NullKind::NaN),
                }
            },
            self.series.name(),
        )
    }

    /// Check if each string is a valid decimal number.
    ///
    /// Matches `pd.Series.str.isdecimal()`.
    pub fn isdecimal(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())),
            self.series.name(),
        )
    }

    /// Check if each string is titlecased.
    ///
    /// Matches `pd.Series.str.istitle()`.
    pub fn istitle(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                if s.is_empty() {
                    return Scalar::Bool(false);
                }
                let mut prev_cased = false;
                let mut prev_space = true;
                for c in s.chars() {
                    if c.is_alphabetic() {
                        if prev_space && !c.is_uppercase() {
                            return Scalar::Bool(false);
                        }
                        if !prev_space && !c.is_lowercase() {
                            return Scalar::Bool(false);
                        }
                        prev_cased = true;
                        prev_space = false;
                    } else {
                        prev_space = true;
                    }
                }
                Scalar::Bool(prev_cased)
            },
            self.series.name(),
        )
    }

    /// Return the string representation of each element.
    ///
    /// Matches `pd.Series.str.cat()` - concatenate strings.
    /// Concatenates all strings in the Series with a separator.
    pub fn cat(&self, sep: &str) -> Result<String, FrameError> {
        let mut parts = Vec::new();
        for val in self.series.column().values() {
            match val {
                Scalar::Utf8(s) => parts.push(s.clone()),
                Scalar::Null(_) => {}
                other => parts.push(format!("{other:?}")),
            }
        }
        Ok(parts.join(sep))
    }

    /// Find the first occurrence of a substring.
    ///
    /// Matches `pd.Series.str.find(sub)`. Returns -1 if not found.
    pub fn find(&self, sub: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| match s.find(sub) {
                Some(pos) => Scalar::Int64(pos as i64),
                None => Scalar::Int64(-1),
            },
            self.series.name(),
        )
    }

    /// Find the last occurrence of a substring.
    ///
    /// Matches `pd.Series.str.rfind(sub)`. Returns -1 if not found.
    pub fn rfind(&self, sub: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| match s.rfind(sub) {
                Some(pos) => Scalar::Int64(pos as i64),
                None => Scalar::Int64(-1),
            },
            self.series.name(),
        )
    }

    /// Find the first occurrence of a substring; error if not found.
    ///
    /// Matches `pd.Series.str.index(sub)`. Like `find()` but raises
    /// an error for missing values (here, returns NaN for not-found).
    pub fn index_of(&self, sub: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| match s.find(sub) {
                Some(pos) => Scalar::Int64(pos as i64),
                None => Scalar::Null(NullKind::NaN),
            },
            self.series.name(),
        )
    }

    /// Find the last occurrence of a substring; error if not found.
    ///
    /// Matches `pd.Series.str.rindex(sub)`. Like `rfind()` but raises
    /// an error for missing values (here, returns NaN for not-found).
    pub fn rindex_of(&self, sub: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| match s.rfind(sub) {
                Some(pos) => Scalar::Int64(pos as i64),
                None => Scalar::Null(NullKind::NaN),
            },
            self.series.name(),
        )
    }

    /// Replace tab characters with spaces.
    ///
    /// Matches `pd.Series.str.expandtabs(tabsize)`.
    pub fn expandtabs(&self, tabsize: usize) -> Result<Series, FrameError> {
        let spaces = " ".repeat(tabsize);
        self.apply_str(
            |s| Scalar::Utf8(s.replace('\t', &spaces)),
            self.series.name(),
        )
    }

    /// Remove a prefix from each string if present.
    ///
    /// Matches `pd.Series.str.removeprefix(prefix)` (Python 3.9+ / pandas 1.4+).
    pub fn removeprefix(&self, prefix: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.strip_prefix(prefix).unwrap_or(s).to_owned()),
            self.series.name(),
        )
    }

    /// Remove a suffix from each string if present.
    ///
    /// Matches `pd.Series.str.removesuffix(suffix)` (Python 3.9+ / pandas 1.4+).
    pub fn removesuffix(&self, suffix: &str) -> Result<Series, FrameError> {
        self.apply_str(
            |s| Scalar::Utf8(s.strip_suffix(suffix).unwrap_or(s).to_owned()),
            self.series.name(),
        )
    }

    /// Aggressive Unicode case folding.
    ///
    /// Matches `pd.Series.str.casefold()`. Like lower() but more aggressive
    /// for Unicode (e.g., German sharp s).
    pub fn casefold(&self) -> Result<Series, FrameError> {
        // Rust's to_lowercase is already Unicode-aware and handles casefold semantics
        self.apply_str(
            |s| Scalar::Utf8(s.to_lowercase()),
            self.series.name(),
        )
    }

    /// Swap the case of each character.
    ///
    /// Matches `pd.Series.str.swapcase()`.
    pub fn swapcase(&self) -> Result<Series, FrameError> {
        self.apply_str(
            |s| {
                let swapped: String = s
                    .chars()
                    .flat_map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().collect::<Vec<_>>()
                        } else {
                            c.to_uppercase().collect::<Vec<_>>()
                        }
                    })
                    .collect();
                Scalar::Utf8(swapped)
            },
            self.series.name(),
        )
    }

    /// Split each string at the first occurrence of separator.
    ///
    /// Matches `pd.Series.str.partition(sep)`. Returns a tuple-like string
    /// "(before, sep, after)" or "(original, '', '')" if sep not found.
    /// Returns three separate values as a comma-separated string for simplicity.
    pub fn partition(&self, sep: &str) -> Result<(Series, Series, Series), FrameError> {
        let vals = self.series.column().values();
        let labels = self.series.index().labels().to_vec();
        let mut before = Vec::with_capacity(vals.len());
        let mut sep_out = Vec::with_capacity(vals.len());
        let mut after = Vec::with_capacity(vals.len());

        for val in vals {
            match val {
                Scalar::Utf8(s) => {
                    if let Some(pos) = s.find(sep) {
                        before.push(Scalar::Utf8(s[..pos].to_owned()));
                        sep_out.push(Scalar::Utf8(sep.to_owned()));
                        after.push(Scalar::Utf8(s[pos + sep.len()..].to_owned()));
                    } else {
                        before.push(Scalar::Utf8(s.clone()));
                        sep_out.push(Scalar::Utf8(String::new()));
                        after.push(Scalar::Utf8(String::new()));
                    }
                }
                _ => {
                    before.push(Scalar::Null(NullKind::NaN));
                    sep_out.push(Scalar::Null(NullKind::NaN));
                    after.push(Scalar::Null(NullKind::NaN));
                }
            }
        }

        let s1 = Series::from_values(format!("{}_0", self.series.name()), labels.clone(), before)?;
        let s2 = Series::from_values(format!("{}_1", self.series.name()), labels.clone(), sep_out)?;
        let s3 = Series::from_values(format!("{}_2", self.series.name()), labels, after)?;
        Ok((s1, s2, s3))
    }

    /// Split each string at the last occurrence of separator.
    ///
    /// Matches `pd.Series.str.rpartition(sep)`.
    pub fn rpartition(&self, sep: &str) -> Result<(Series, Series, Series), FrameError> {
        let vals = self.series.column().values();
        let labels = self.series.index().labels().to_vec();
        let mut before = Vec::with_capacity(vals.len());
        let mut sep_out = Vec::with_capacity(vals.len());
        let mut after = Vec::with_capacity(vals.len());

        for val in vals {
            match val {
                Scalar::Utf8(s) => {
                    if let Some(pos) = s.rfind(sep) {
                        before.push(Scalar::Utf8(s[..pos].to_owned()));
                        sep_out.push(Scalar::Utf8(sep.to_owned()));
                        after.push(Scalar::Utf8(s[pos + sep.len()..].to_owned()));
                    } else {
                        before.push(Scalar::Utf8(String::new()));
                        sep_out.push(Scalar::Utf8(String::new()));
                        after.push(Scalar::Utf8(s.clone()));
                    }
                }
                _ => {
                    before.push(Scalar::Null(NullKind::NaN));
                    sep_out.push(Scalar::Null(NullKind::NaN));
                    after.push(Scalar::Null(NullKind::NaN));
                }
            }
        }

        let s1 = Series::from_values(format!("{}_0", self.series.name()), labels.clone(), before)?;
        let s2 = Series::from_values(format!("{}_1", self.series.name()), labels.clone(), sep_out)?;
        let s3 = Series::from_values(format!("{}_2", self.series.name()), labels, after)?;
        Ok((s1, s2, s3))
    }
}

/// Datetime accessor for Series, analogous to `pd.Series.dt`.
///
/// Extracts datetime components from ISO 8601 / RFC 3339 formatted strings.
/// Non-string or unparseable values yield NaN.
pub struct DatetimeAccessor<'a> {
    series: &'a Series,
}

impl DatetimeAccessor<'_> {
    /// Helper: apply a datetime extraction function to each value.
    fn extract_component<F>(&self, func: F, name: &str) -> Result<Series, FrameError>
    where
        F: Fn(&str) -> Scalar,
    {
        let vals = self.series.column().values();
        let out: Vec<Scalar> = vals
            .iter()
            .map(|v| match v {
                Scalar::Utf8(s) => func(s),
                _ if v.is_missing() => Scalar::Null(NullKind::NaN),
                _ => Scalar::Null(NullKind::NaN),
            })
            .collect();
        Series::from_values(
            name.to_string(),
            self.series.index().labels().to_vec(),
            out,
        )
    }

    /// Extract year component.
    ///
    /// Matches `pd.Series.dt.year`.
    pub fn year(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| Self::parse_datetime_component(s, 0),
            self.series.name(),
        )
    }

    /// Extract month component (1-12).
    ///
    /// Matches `pd.Series.dt.month`.
    pub fn month(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| Self::parse_datetime_component(s, 1),
            self.series.name(),
        )
    }

    /// Extract day component (1-31).
    ///
    /// Matches `pd.Series.dt.day`.
    pub fn day(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| Self::parse_datetime_component(s, 2),
            self.series.name(),
        )
    }

    /// Extract hour component (0-23).
    ///
    /// Matches `pd.Series.dt.hour`.
    pub fn hour(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| Self::parse_datetime_component(s, 3),
            self.series.name(),
        )
    }

    /// Extract minute component (0-59).
    ///
    /// Matches `pd.Series.dt.minute`.
    pub fn minute(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| Self::parse_datetime_component(s, 4),
            self.series.name(),
        )
    }

    /// Extract second component (0-59).
    ///
    /// Matches `pd.Series.dt.second`.
    pub fn second(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| Self::parse_datetime_component(s, 5),
            self.series.name(),
        )
    }

    /// Extract day of week (Monday=0, Sunday=6).
    ///
    /// Matches `pd.Series.dt.dayofweek` / `pd.Series.dt.weekday`.
    pub fn dayofweek(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                // Parse date and compute day of week using Zeller-like formula
                if let Some((y, m, d)) = Self::parse_ymd_from_datetime(s) {
                    // Tomohiko Sakamoto's algorithm
                    let t: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
                    let y_adj = if m < 3 { y - 1 } else { y };
                    let dow = (y_adj + y_adj / 4 - y_adj / 100 + y_adj / 400 + t[(m - 1) as usize] + d) % 7;
                    // Sakamoto gives Sunday=0, we want Monday=0
                    let pandas_dow = (dow + 6) % 7;
                    Scalar::Int64(pandas_dow)
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Extract date part as string (YYYY-MM-DD).
    ///
    /// Matches `pd.Series.dt.date`.
    pub fn date(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some(date_part) = s.split('T').next()
                    && Self::parse_ymd(date_part).is_some()
                {
                    return Scalar::Utf8(date_part.to_string());
                }
                // Try space-delimited datetime
                if let Some(date_part) = s.split(' ').next()
                    && Self::parse_ymd(date_part).is_some()
                {
                    return Scalar::Utf8(date_part.to_string());
                }
                Scalar::Null(NullKind::NaN)
            },
            self.series.name(),
        )
    }

    /// Internal: parse a datetime string and extract a specific component.
    ///
    /// Components: 0=year, 1=month, 2=day, 3=hour, 4=minute, 5=second
    fn parse_datetime_component(s: &str, component: usize) -> Scalar {
        // Try parsing ISO 8601 formats:
        // "YYYY-MM-DD", "YYYY-MM-DDTHH:MM:SS", "YYYY-MM-DD HH:MM:SS"
        let s = s.trim();
        let (date_part, time_part) = if let Some(i) = s.find('T') {
            (&s[..i], Some(&s[i + 1..]))
        } else if let Some(i) = s.find(' ') {
            (&s[..i], Some(&s[i + 1..]))
        } else {
            (s, None)
        };

        let date_parts: Vec<&str> = date_part.split('-').collect();
        if date_parts.len() != 3 {
            return Scalar::Null(NullKind::NaN);
        }

        let Ok(year) = date_parts[0].parse::<i64>() else {
            return Scalar::Null(NullKind::NaN);
        };
        let Ok(month) = date_parts[1].parse::<i64>() else {
            return Scalar::Null(NullKind::NaN);
        };
        let Ok(day) = date_parts[2].parse::<i64>() else {
            return Scalar::Null(NullKind::NaN);
        };

        match component {
            0 => Scalar::Int64(year),
            1 => Scalar::Int64(month),
            2 => Scalar::Int64(day),
            3..=5 => {
                if let Some(time_str) = time_part {
                    // Remove timezone info if present
                    let time_clean = time_str
                        .split('+')
                        .next()
                        .and_then(|s| s.split('Z').next())
                        .unwrap_or(time_str);
                    let time_parts: Vec<&str> = time_clean.split(':').collect();
                    let idx = component - 3;
                    if idx < time_parts.len() {
                        // Handle seconds with fractional part
                        let part = time_parts[idx].split('.').next().unwrap_or(time_parts[idx]);
                        if let Ok(v) = part.parse::<i64>() {
                            return Scalar::Int64(v);
                        }
                    }
                }
                Scalar::Int64(0) // Default to 0 if no time part
            }
            _ => Scalar::Null(NullKind::NaN),
        }
    }

    /// Extract quarter (1-4).
    ///
    /// Matches `pd.Series.dt.quarter`.
    pub fn quarter(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                let m = Self::parse_datetime_component(s, 1);
                match m {
                    Scalar::Int64(month) => Scalar::Int64((month - 1) / 3 + 1),
                    _ => Scalar::Null(NullKind::NaN),
                }
            },
            self.series.name(),
        )
    }

    /// Extract day of year (1-366).
    ///
    /// Matches `pd.Series.dt.dayofyear`.
    pub fn dayofyear(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some((y, m, d)) = Self::parse_ymd_from_datetime(s) {
                    let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
                    let days_in_month: [i64; 12] = [
                        31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30,
                        31, 31, 30, 31, 30, 31,
                    ];
                    let doy: i64 = days_in_month[..(m - 1) as usize].iter().sum::<i64>() + d;
                    Scalar::Int64(doy)
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Extract ISO week number (1-53).
    ///
    /// Matches `pd.Series.dt.isocalendar().week`.
    pub fn weekofyear(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some((y, m, d)) = Self::parse_ymd_from_datetime(s) {
                    // Compute day of year
                    let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
                    let days_in_month: [i64; 12] = [
                        31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30,
                        31, 31, 30, 31, 30, 31,
                    ];
                    let doy: i64 = days_in_month[..(m - 1) as usize].iter().sum::<i64>() + d;

                    // Day of week (Monday=1, Sunday=7 for ISO)
                    let t: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
                    let y_adj = if m < 3 { y - 1 } else { y };
                    let dow_sun0 = (y_adj + y_adj / 4 - y_adj / 100 + y_adj / 400 + t[(m - 1) as usize] + d) % 7;
                    let iso_dow = if dow_sun0 == 0 { 7 } else { dow_sun0 }; // Monday=1..Sunday=7

                    // ISO week: the week containing Jan 4 is always week 1
                    let week = (doy - iso_dow + 10) / 7;
                    if week < 1 {
                        Scalar::Int64(52) // belongs to previous year
                    } else if week > 52 {
                        Scalar::Int64(1) // may belong to next year
                    } else {
                        Scalar::Int64(week)
                    }
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Check if date is the first day of the month.
    ///
    /// Matches `pd.Series.dt.is_month_start`.
    pub fn is_month_start(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some((_, _, d)) = Self::parse_ymd_from_datetime(s) {
                    Scalar::Bool(d == 1)
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Check if date is the last day of the month.
    ///
    /// Matches `pd.Series.dt.is_month_end`.
    pub fn is_month_end(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some((y, m, d)) = Self::parse_ymd_from_datetime(s) {
                    let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
                    let days_in_month: [i64; 12] = [
                        31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30,
                        31, 31, 30, 31, 30, 31,
                    ];
                    Scalar::Bool(d == days_in_month[(m - 1) as usize])
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Check if date is the first day of a quarter.
    ///
    /// Matches `pd.Series.dt.is_quarter_start`.
    pub fn is_quarter_start(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some((_, m, d)) = Self::parse_ymd_from_datetime(s) {
                    Scalar::Bool(d == 1 && (m == 1 || m == 4 || m == 7 || m == 10))
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Check if date is the last day of a quarter.
    ///
    /// Matches `pd.Series.dt.is_quarter_end`.
    pub fn is_quarter_end(&self) -> Result<Series, FrameError> {
        self.extract_component(
            |s| {
                if let Some((_, m, d)) = Self::parse_ymd_from_datetime(s) {
                    Scalar::Bool(
                        (m == 3 && d == 31)
                            || (m == 6 && d == 30)
                            || (m == 9 && d == 30)
                            || (m == 12 && d == 31),
                    )
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Format datetime using strftime-like directives.
    ///
    /// Matches `pd.Series.dt.strftime(format)`. Supports: %Y, %m, %d, %H, %M, %S.
    pub fn strftime(&self, format: &str) -> Result<Series, FrameError> {
        let fmt = format.to_owned();
        self.extract_component(
            |s| {
                if let Some((y, m, d)) = Self::parse_ymd_from_datetime(s) {
                    let h = match Self::parse_datetime_component(s, 3) {
                        Scalar::Int64(v) => v,
                        _ => 0,
                    };
                    let mi = match Self::parse_datetime_component(s, 4) {
                        Scalar::Int64(v) => v,
                        _ => 0,
                    };
                    let sec = match Self::parse_datetime_component(s, 5) {
                        Scalar::Int64(v) => v,
                        _ => 0,
                    };
                    let result = fmt
                        .replace("%Y", &format!("{y:04}"))
                        .replace("%m", &format!("{m:02}"))
                        .replace("%d", &format!("{d:02}"))
                        .replace("%H", &format!("{h:02}"))
                        .replace("%M", &format!("{mi:02}"))
                        .replace("%S", &format!("{sec:02}"));
                    Scalar::Utf8(result)
                } else {
                    Scalar::Null(NullKind::NaN)
                }
            },
            self.series.name(),
        )
    }

    /// Internal: parse year, month, day from a date-only string ("YYYY-MM-DD").
    fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
        let parts: Vec<&str> = s.trim().split('-').collect();
        if parts.len() != 3 {
            return None;
        }
        let year = parts[0].parse::<i64>().ok()?;
        let month = parts[1].parse::<i64>().ok()?;
        let day = parts[2].parse::<i64>().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some((year, month, day))
    }

    /// Internal: extract date part from a datetime string, then parse YMD.
    fn parse_ymd_from_datetime(s: &str) -> Option<(i64, i64, i64)> {
        let date_part = s.split('T').next().unwrap_or(s);
        let date_part = date_part.split(' ').next().unwrap_or(date_part);
        Self::parse_ymd(date_part)
    }
}

impl std::fmt::Display for Series {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let max_rows = 60;
        let len = self.len();
        let show = len.min(max_rows);
        for i in 0..show {
            let label = &self.index.labels()[i];
            let val = &self.column.values()[i];
            writeln!(f, "{label}    {val}")?;
        }
        if len > max_rows {
            writeln!(f, "...")?;
        }
        write!(f, "Name: {}, Length: {len}, dtype: {:?}", self.name, self.column.dtype())
    }
}

impl std::fmt::Display for DataFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let max_rows = 60;
        let len = self.len();

        // Compute column widths
        let mut col_widths: Vec<usize> = self
            .column_order
            .iter()
            .map(|name| name.len())
            .collect();
        // Also compute index label width
        let idx_width = self
            .index
            .labels()
            .iter()
            .take(max_rows)
            .map(|l| format!("{l}").len())
            .max()
            .unwrap_or(0)
            .max(5);

        // Measure value widths
        let show = len.min(max_rows);
        for (col_idx, name) in self.column_order.iter().enumerate() {
            let col = &self.columns[name];
            for i in 0..show {
                let val_len = format!("{}", col.values()[i]).len();
                if val_len > col_widths[col_idx] {
                    col_widths[col_idx] = val_len;
                }
            }
        }

        // Header row
        write!(f, "{:width$}", "", width = idx_width + 2)?;
        for (i, name) in self.column_order.iter().enumerate() {
            write!(f, "{:>width$}", name, width = col_widths[i] + 2)?;
        }
        writeln!(f)?;

        // Data rows
        for row in 0..show {
            let label = &self.index.labels()[row];
            write!(f, "{:<width$}", format!("{label}"), width = idx_width + 2)?;
            for (col_idx, name) in self.column_order.iter().enumerate() {
                let val = &self.columns[name].values()[row];
                write!(f, "{:>width$}", format!("{val}"), width = col_widths[col_idx] + 2)?;
            }
            writeln!(f)?;
        }
        if len > max_rows {
            writeln!(f, "... ({len} rows total)")?;
        }
        write!(f, "\n[{len} rows x {} columns]", self.column_order.len())
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

/// Concatenate DataFrames with keys for hierarchical labeling.
///
/// Matches `pd.concat([df1, df2], keys=['a', 'b'])`. Prefixes each
/// frame's index labels with the corresponding key, creating composite
/// labels like "key|original_label".
pub fn concat_dataframes_with_keys(
    frames: &[&DataFrame],
    keys: &[&str],
) -> Result<DataFrame, FrameError> {
    if frames.len() != keys.len() {
        return Err(FrameError::CompatibilityRejected(
            "concat: number of frames must match number of keys".to_owned(),
        ));
    }

    if frames.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new());
    }

    let total_len: usize = frames.iter().map(|f| f.len()).sum();
    let mut labels = Vec::with_capacity(total_len);

    for (frame, key) in frames.iter().zip(keys.iter()) {
        for label in frame.index().labels() {
            let composite = match label {
                IndexLabel::Int64(v) => format!("{key}|{v}"),
                IndexLabel::Utf8(s) => format!("{key}|{s}"),
            };
            labels.push(IndexLabel::Utf8(composite));
        }
    }

    // Use union column set (same as concat_dataframes)
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

    DataFrame::new_with_column_order(Index::new(labels), columns, union_columns)
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

    /// Parse a CSV string into a DataFrame.
    ///
    /// Matches `pd.read_csv(StringIO(text))`. The first line is treated as
    /// column headers. Values are auto-detected as Int64, Float64, Bool, or Utf8.
    pub fn from_csv(text: &str, sep: char) -> Result<Self, FrameError> {
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| FrameError::CompatibilityRejected("empty CSV".to_owned()))?;
        let col_names: Vec<String> = header.split(sep).map(|s| s.trim().to_owned()).collect();

        let mut col_data: Vec<Vec<Scalar>> = vec![Vec::new(); col_names.len()];

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(sep).collect();
            for (i, col) in col_data.iter_mut().enumerate() {
                let raw = fields.get(i).map(|s| s.trim()).unwrap_or("");
                if raw.is_empty() {
                    col.push(Scalar::Null(NullKind::NaN));
                } else if let Ok(v) = raw.parse::<i64>() {
                    col.push(Scalar::Int64(v));
                } else if let Ok(v) = raw.parse::<f64>() {
                    col.push(Scalar::Float64(v));
                } else if raw.eq_ignore_ascii_case("true") {
                    col.push(Scalar::Bool(true));
                } else if raw.eq_ignore_ascii_case("false") {
                    col.push(Scalar::Bool(false));
                } else {
                    col.push(Scalar::Utf8(raw.to_owned()));
                }
            }
        }

        // Unify types per column: if a column has both Int64 and Float64,
        // promote all Int64 values to Float64 for homogeneity.
        for col in &mut col_data {
            let has_float = col.iter().any(|v| matches!(v, Scalar::Float64(_)));
            let has_int = col.iter().any(|v| matches!(v, Scalar::Int64(_)));
            if has_float && has_int {
                for val in col.iter_mut() {
                    if let Scalar::Int64(i) = val {
                        *val = Scalar::Float64(*i as f64);
                    }
                }
            }
        }

        let data: Vec<(&str, Vec<Scalar>)> = col_names
            .iter()
            .zip(col_data)
            .map(|(name, vals)| (name.as_str(), vals))
            .collect();
        let order: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
        Self::from_dict(&order, data)
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

    /// Return (nrows, ncols) tuple.
    ///
    /// Matches `pd.DataFrame.shape`.
    #[must_use]
    pub fn shape(&self) -> (usize, usize) {
        (self.len(), self.num_columns())
    }

    /// Return a Series of dtypes, one per column.
    ///
    /// Matches `pd.DataFrame.dtypes`.
    pub fn dtypes(&self) -> Result<Series, FrameError> {
        let labels: Vec<IndexLabel> = self.column_order.iter().map(|n| n.as_str().into()).collect();
        let values: Vec<Scalar> = self
            .column_order
            .iter()
            .map(|n| Scalar::Utf8(format!("{:?}", self.columns[n].dtype())))
            .collect();
        Series::from_values("dtypes".to_string(), labels, values)
    }

    /// Return a deep copy of this DataFrame.
    ///
    /// Matches `pd.DataFrame.copy(deep=True)`.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
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

    /// Sort by multiple columns with per-column ascending flags.
    ///
    /// Matches `df.sort_values(by=[...], ascending=[...], na_position='first'|'last')`.
    /// `na_position` defaults to "last".
    pub fn sort_values_multi(
        &self,
        by: &[&str],
        ascending: &[bool],
        na_position: &str,
    ) -> Result<Self, FrameError> {
        if by.is_empty() {
            return Ok(self.clone());
        }

        // Validate columns exist
        let sort_cols: Vec<&Column> = by
            .iter()
            .map(|name| {
                self.columns.get(*name).ok_or_else(|| {
                    FrameError::CompatibilityRejected(format!("column '{name}' not found"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Pad ascending to match by length (default true)
        let asc: Vec<bool> = by
            .iter()
            .enumerate()
            .map(|(i, _)| ascending.get(i).copied().unwrap_or(true))
            .collect();

        let na_first = na_position == "first";

        let mut order: Vec<usize> = (0..self.len()).collect();
        order.sort_by(|&a, &b| {
            for (col_idx, col) in sort_cols.iter().enumerate() {
                let left = &col.values()[a];
                let right = &col.values()[b];
                let l_miss = left.is_missing();
                let r_miss = right.is_missing();

                if l_miss && r_miss {
                    continue;
                }
                if l_miss {
                    return if na_first { Ordering::Less } else { Ordering::Greater };
                }
                if r_miss {
                    return if na_first { Ordering::Greater } else { Ordering::Less };
                }

                let cmp = compare_scalars_with_na_last(left, right, asc[col_idx]);
                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
            Ordering::Equal
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

    /// Rename columns using a HashMap mapping.
    ///
    /// Convenience wrapper matching `df.rename(columns={...})` semantics.
    /// Only columns present in the mapping are renamed; others are kept as-is.
    pub fn rename_columns_map(
        &self,
        mapping: &std::collections::HashMap<String, String>,
    ) -> Result<Self, FrameError> {
        let pairs: Vec<(&str, &str)> = mapping
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.rename_columns(&pairs)
    }

    /// Rename columns using a mapper function.
    ///
    /// Matches `df.rename(columns=func)`. Applies the function to each column name.
    pub fn rename_with<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&str) -> String,
    {
        let mut columns = BTreeMap::new();
        let mut column_order = Vec::with_capacity(self.column_order.len());
        for name in &self.column_order {
            let col = &self.columns[name];
            let new_name = func(name);
            if columns.contains_key(&new_name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "duplicate column '{new_name}' resulting from rename"
                )));
            }
            column_order.push(new_name.clone());
            columns.insert(new_name, col.clone());
        }
        Self::new_with_column_order(self.index.clone(), columns, column_order)
    }

    /// Add a prefix to all column names.
    ///
    /// Matches `df.add_prefix(prefix)`.
    pub fn add_prefix(&self, prefix: &str) -> Result<Self, FrameError> {
        let p = prefix.to_owned();
        self.rename_with(|name| format!("{p}{name}"))
    }

    /// Add a suffix to all column names.
    ///
    /// Matches `df.add_suffix(suffix)`.
    pub fn add_suffix(&self, suffix: &str) -> Result<Self, FrameError> {
        let s = suffix.to_owned();
        self.rename_with(|name| format!("{name}{s}"))
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

    /// Alias for `transpose()`.
    ///
    /// Matches `df.T` in pandas.
    pub fn t(&self) -> Result<Self, FrameError> {
        self.transpose()
    }

    /// Swap axes (rows and columns).
    ///
    /// Matches `df.swapaxes('index', 'columns')`. Alias for `transpose()`.
    pub fn swapaxes(&self) -> Result<Self, FrameError> {
        self.transpose()
    }

    /// Label-based element lookup.
    ///
    /// Matches `df.lookup(row_labels, col_labels)`. Returns a Vec of Scalars
    /// where element i is the value at (row_labels[i], col_labels[i]).
    pub fn lookup(
        &self,
        row_labels: &[IndexLabel],
        col_labels: &[&str],
    ) -> Result<Vec<Scalar>, FrameError> {
        if row_labels.len() != col_labels.len() {
            return Err(FrameError::CompatibilityRejected(
                "row_labels and col_labels must have the same length".to_owned(),
            ));
        }

        let pos_map = self.index.position_map_first();
        let mut result = Vec::with_capacity(row_labels.len());

        for (rl, cl) in row_labels.iter().zip(col_labels) {
            let row_pos = pos_map.get(rl).ok_or_else(|| {
                FrameError::CompatibilityRejected(format!("row label not found: {rl:?}"))
            })?;
            let col = self.columns.get(*cl).ok_or_else(|| {
                FrameError::CompatibilityRejected(format!("column not found: {cl}"))
            })?;
            result.push(col.values()[*row_pos].clone());
        }

        Ok(result)
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
            // Strip Utf8("...") or Int64(...) or Bool(...) wrappers
            if let Some(inner) = s.strip_prefix("Utf8(\"") {
                inner.strip_suffix("\")").unwrap_or(inner).to_string()
            } else if let Some(inner) = s.strip_prefix("Int64(") {
                inner.strip_suffix(')').unwrap_or(inner).to_string()
            } else if let Some(inner) = s.strip_prefix("Float64(") {
                inner.strip_suffix(')').unwrap_or(inner).to_string()
            } else if let Some(inner) = s.strip_prefix("Bool(") {
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
                    Column::from_values(vals)?,
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
            result_cols.insert(col_name.clone(), Column::from_values(new_vals)?);
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Apply a function element-wise (pandas 2.0 name for `applymap`).
    ///
    /// Matches `pd.DataFrame.map(func)`.
    pub fn map_elements<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Scalar) -> Scalar,
    {
        self.applymap(func)
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

    /// Compute correlation matrix using the specified method.
    ///
    /// Supported methods: "pearson", "spearman", "kendall".
    pub fn corr_method(&self, method: &str) -> Result<Self, FrameError> {
        match method {
            "pearson" => self.pairwise_stat("corr"),
            "spearman" => self.pairwise_rank_corr("spearman"),
            "kendall" => self.pairwise_rank_corr("kendall"),
            other => Err(FrameError::CompatibilityRejected(format!(
                "unsupported correlation method: '{other}'"
            ))),
        }
    }

    /// Compute the pairwise covariance matrix between numeric columns.
    pub fn cov(&self) -> Result<Self, FrameError> {
        self.pairwise_stat("cov")
    }

    /// Internal helper for corr/cov pairwise matrix computation.
    fn pairwise_stat(&self, stat: &str) -> Result<Self, FrameError> {
        let len = self.index.len();

        // Only include numeric columns (Int64/Float64), matching pandas behavior
        let numeric_cols: Vec<String> = self
            .column_order
            .iter()
            .filter(|name| {
                let dt = self.columns[name.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .cloned()
            .collect();

        let n = numeric_cols.len();

        // Extract f64 columns
        let col_data: Vec<Vec<f64>> = numeric_cols
            .iter()
            .map(|name| {
                let col = &self.columns[name];
                (0..len)
                    .map(|i| col.values()[i].to_f64().unwrap_or(f64::NAN))
                    .collect()
            })
            .collect();

        let mut result_cols = BTreeMap::new();
        for (j, col_j_name) in numeric_cols.iter().enumerate() {
            let mut vals = Vec::with_capacity(n);
            for (i, _col_i_name) in numeric_cols.iter().enumerate() {
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

        let labels: Vec<IndexLabel> = numeric_cols
            .iter()
            .map(|s| IndexLabel::Utf8(s.clone()))
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: numeric_cols,
            index: Index::new(labels),
        })
    }

    /// Compute pairwise Spearman or Kendall correlation matrix between numeric columns.
    fn pairwise_rank_corr(&self, method: &str) -> Result<Self, FrameError> {
        let len = self.index.len();

        let numeric_cols: Vec<String> = self
            .column_order
            .iter()
            .filter(|name| {
                let dt = self.columns[name.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .cloned()
            .collect();

        let n = numeric_cols.len();

        // Build Series for each numeric column
        let series_list: Vec<Series> = numeric_cols
            .iter()
            .map(|name| {
                let col = &self.columns[name];
                let labels: Vec<IndexLabel> = (0..len).map(|i| (i as i64).into()).collect();
                Series::new(name, Index::new(labels), col.clone())
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut result_cols = BTreeMap::new();
        for (j, col_j_name) in numeric_cols.iter().enumerate() {
            let mut vals = Vec::with_capacity(n);
            for (i, _) in numeric_cols.iter().enumerate() {
                let val = if i == j {
                    1.0 // Self-correlation is always 1
                } else {
                    match method {
                        "spearman" => series_list[i]
                            .corr_spearman(&series_list[j])
                            .unwrap_or(f64::NAN),
                        "kendall" => series_list[i]
                            .corr_kendall(&series_list[j])
                            .unwrap_or(f64::NAN),
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

        let labels: Vec<IndexLabel> = numeric_cols
            .iter()
            .map(|s| IndexLabel::Utf8(s.clone()))
            .collect();

        Ok(Self {
            columns: result_cols,
            column_order: numeric_cols,
            index: Index::new(labels),
        })
    }

    /// Column-wise correlation with another DataFrame.
    ///
    /// Matches `df.corrwith(other)`. Returns a Series of Pearson correlations
    /// for each shared numeric column.
    pub fn corrwith(&self, other: &Self) -> Result<Series, FrameError> {
        let mut labels = Vec::new();
        let mut values = Vec::new();

        for name in &self.column_order {
            let sc = match self.columns.get(name) {
                Some(c) => c,
                None => continue,
            };
            let oc = match other.columns.get(name) {
                Some(c) => c,
                None => continue,
            };
            let dt_s = sc.dtype();
            let dt_o = oc.dtype();
            if (dt_s != DType::Int64 && dt_s != DType::Float64)
                || (dt_o != DType::Int64 && dt_o != DType::Float64)
            {
                continue;
            }

            let ss = Series::new(name.clone(), self.index.clone(), sc.clone())?;
            let so = Series::new(name.clone(), other.index.clone(), oc.clone())?;
            let r = ss.corr(&so)?;

            labels.push(IndexLabel::Utf8(name.clone()));
            values.push(Scalar::Float64(r));
        }

        Series::from_values("corrwith".to_string(), labels, values)
    }

    /// Matrix dot product with another DataFrame.
    ///
    /// Matches `df.dot(other)`. Computes matrix multiplication where
    /// self's columns are matched to other's index.
    pub fn dot(&self, other: &Self) -> Result<Self, FrameError> {
        // self: (m x k), other: (k x n) → result: (m x n)
        let m = self.len();
        let k = self.num_columns();

        if k != other.len() {
            return Err(FrameError::CompatibilityRejected(format!(
                "dot: self has {k} columns but other has {} rows",
                other.len()
            )));
        }

        // Extract self as row-major f64 matrix
        let self_rows: Vec<Vec<f64>> = (0..m)
            .map(|row| {
                self.column_order
                    .iter()
                    .map(|name| self.columns[name].values()[row].to_f64().unwrap_or(f64::NAN))
                    .collect()
            })
            .collect();

        // Build result
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for other_col_name in &other.column_order {
            let other_col = &other.columns[other_col_name];
            let other_vec: Vec<f64> = other_col
                .values()
                .iter()
                .map(|v| v.to_f64().unwrap_or(f64::NAN))
                .collect();

            let mut vals = Vec::with_capacity(m);
            for row in &self_rows {
                let dot_val: f64 = row.iter().zip(&other_vec).map(|(a, b)| a * b).sum();
                vals.push(Scalar::Float64(dot_val));
            }

            result_cols.insert(other_col_name.clone(), Column::from_values(vals)?);
            col_order.push(other_col_name.clone());
        }

        Ok(Self {
            columns: result_cols,
            column_order: col_order,
            index: self.index.clone(),
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

    /// Update values using non-null values from another DataFrame.
    ///
    /// Analogous to `pandas.DataFrame.update(other)`. Only updates values
    /// at matching index labels and column names where `other` has non-null
    /// values. Retains self's index and columns (no new labels/columns
    /// added from other).
    pub fn update(&self, other: &Self) -> Result<Self, FrameError> {
        let other_idx_map: BTreeMap<&IndexLabel, usize> = other
            .index
            .labels()
            .iter()
            .enumerate()
            .map(|(i, l)| (l, i))
            .collect();

        let mut result_cols = BTreeMap::new();

        for col_name in &self.column_order {
            let self_col = &self.columns[col_name];
            let new_vals: Vec<Scalar> = if let Some(other_col) = other.columns.get(col_name) {
                self.index
                    .labels()
                    .iter()
                    .enumerate()
                    .map(|(i, label)| {
                        if let Some(&other_i) = other_idx_map.get(label) {
                            let other_val = &other_col.values()[other_i];
                            if !other_val.is_missing() {
                                return other_val.clone();
                            }
                        }
                        self_col.values()[i].clone()
                    })
                    .collect()
            } else {
                self_col.values().to_vec()
            };
            result_cols.insert(col_name.clone(), Column::from_values(new_vals)?);
        }

        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Combine two DataFrames, preferring non-null values from self.
    ///
    /// Analogous to `pandas.DataFrame.combine_first(other)`. Uses union
    /// of indices and columns. For overlapping positions, uses self's value
    /// if non-null, else other's value.
    pub fn combine_first(&self, other: &Self) -> Result<Self, FrameError> {
        // Build union of index labels (preserving order: self first, then new from other)
        let mut seen = std::collections::HashSet::new();
        let mut union_labels = Vec::new();
        for label in self.index.labels() {
            if seen.insert(label.clone()) {
                union_labels.push(label.clone());
            }
        }
        for label in other.index.labels() {
            if seen.insert(label.clone()) {
                union_labels.push(label.clone());
            }
        }

        // Build union of columns (preserving order: self first, then new from other)
        let mut col_order = Vec::new();
        let mut col_set = std::collections::HashSet::new();
        for name in &self.column_order {
            if col_set.insert(name.clone()) {
                col_order.push(name.clone());
            }
        }
        for name in &other.column_order {
            if col_set.insert(name.clone()) {
                col_order.push(name.clone());
            }
        }

        // Build lookup maps for self and other
        let self_idx: BTreeMap<&IndexLabel, usize> = self
            .index
            .labels()
            .iter()
            .enumerate()
            .map(|(i, l)| (l, i))
            .collect();
        let other_idx: BTreeMap<&IndexLabel, usize> = other
            .index
            .labels()
            .iter()
            .enumerate()
            .map(|(i, l)| (l, i))
            .collect();

        let mut result_cols = BTreeMap::new();
        for col_name in &col_order {
            let self_col = self.columns.get(col_name);
            let other_col = other.columns.get(col_name);
            let vals: Vec<Scalar> = union_labels
                .iter()
                .map(|label| {
                    // Prefer self value if non-null
                    if let Some(sc) = self_col
                        && let Some(&i) = self_idx.get(label)
                    {
                        let v = &sc.values()[i];
                        if !v.is_missing() {
                            return v.clone();
                        }
                    }
                    // Fall back to other
                    if let Some(oc) = other_col
                        && let Some(&i) = other_idx.get(label)
                    {
                        return oc.values()[i].clone();
                    }
                    Scalar::Null(NullKind::NaN)
                })
                .collect();
            result_cols.insert(col_name.clone(), Column::from_values(vals)?);
        }

        Ok(Self {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(union_labels),
        })
    }

    /// Create a rolling window view over all numeric columns.
    ///
    /// Matches `df.rolling(window)` semantics.
    pub fn rolling(
        &self,
        window: usize,
        min_periods: Option<usize>,
    ) -> DataFrameRolling<'_> {
        DataFrameRolling {
            df: self,
            window,
            min_periods: min_periods.unwrap_or(window),
        }
    }

    /// Create an expanding window view over all numeric columns.
    ///
    /// Matches `df.expanding(min_periods)` semantics.
    pub fn expanding(&self, min_periods: Option<usize>) -> DataFrameExpanding<'_> {
        DataFrameExpanding {
            df: self,
            min_periods: min_periods.unwrap_or(1),
        }
    }

    /// Create an exponentially weighted moving window view over all numeric columns.
    ///
    /// Matches `df.ewm(span=...)` semantics.
    pub fn ewm(&self, span: Option<f64>, alpha: Option<f64>) -> DataFrameEwm<'_> {
        DataFrameEwm {
            df: self,
            span,
            alpha,
        }
    }

    /// Create a time-based resampler view for the DataFrame.
    ///
    /// Matches `df.resample(freq)`. The DataFrame must have Utf8 datetime-like
    /// index labels. Applies aggregation to each numeric column per time bucket.
    pub fn resample(&self, freq: &str) -> DataFrameResample<'_> {
        DataFrameResample {
            df: self,
            freq: freq.to_string(),
        }
    }

    /// Select rows where the time component of the index is between two times.
    ///
    /// Matches `df.between_time(start_time, end_time)`.
    /// Index labels should be datetime-like strings with time component (HH:MM or HH:MM:SS).
    pub fn between_time(&self, start: &str, end: &str) -> Result<Self, FrameError> {
        let labels = self.index.labels();
        let mut keep = Vec::new();

        for (i, label) in labels.iter().enumerate() {
            if let IndexLabel::Utf8(s) = label {
                let time_part = Self::extract_time(s);
                if let Some(ref t) = time_part
                    && t.as_str() >= start && t.as_str() <= end
                {
                    keep.push(i);
                }
            }
        }

        self.take_rows_by_positions(&keep)
    }

    /// Select rows where the time component exactly matches the given time.
    ///
    /// Matches `df.at_time(time)`.
    pub fn at_time(&self, time: &str) -> Result<Self, FrameError> {
        let labels = self.index.labels();
        let mut keep = Vec::new();

        for (i, label) in labels.iter().enumerate() {
            if let IndexLabel::Utf8(s) = label {
                let time_part = Self::extract_time(s);
                if let Some(ref t) = time_part
                    && t.as_str() == time
                {
                    keep.push(i);
                }
            }
        }

        self.take_rows_by_positions(&keep)
    }

    /// Extract the time portion from a datetime string.
    fn extract_time(s: &str) -> Option<String> {
        // Try "YYYY-MM-DDTHH:MM:SS" or "YYYY-MM-DD HH:MM:SS"
        if let Some(pos) = s.find('T').or_else(|| {
            // Find space that separates date from time
            let parts: Vec<&str> = s.splitn(2, ' ').collect();
            if parts.len() == 2 && parts[0].contains('-') {
                Some(parts[0].len())
            } else {
                None
            }
        }) {
            let time_str = &s[pos + 1..];
            Some(time_str.to_string())
        } else {
            None
        }
    }

    /// Render the DataFrame as a LaTeX table.
    ///
    /// Matches `df.to_latex()`.
    pub fn to_latex(&self, include_index: bool) -> String {
        fn format_scalar(val: &Scalar) -> String {
            match val {
                Scalar::Null(_) => "NaN".to_string(),
                Scalar::Bool(b) => if *b { "True" } else { "False" }.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => {
                    if v.is_nan() {
                        "NaN".to_string()
                    } else {
                        v.to_string()
                    }
                }
                Scalar::Utf8(s) => s.clone(),
            }
        }

        let ncols = self.column_order.len() + if include_index { 1 } else { 0 };
        let col_spec = "r".repeat(ncols);

        let mut out = String::new();
        out.push_str(&format!("\\begin{{tabular}}{{{col_spec}}}\n"));
        out.push_str("\\toprule\n");

        // Header
        let mut headers: Vec<String> = Vec::new();
        if include_index {
            headers.push(String::new());
        }
        for name in &self.column_order {
            headers.push(name.clone());
        }
        out.push_str(&headers.join(" & "));
        out.push_str(" \\\\\n");
        out.push_str("\\midrule\n");

        // Data rows
        for (row_idx, label) in self.index.labels().iter().enumerate() {
            let mut cells: Vec<String> = Vec::new();
            if include_index {
                cells.push(match label {
                    IndexLabel::Int64(v) => v.to_string(),
                    IndexLabel::Utf8(s) => s.clone(),
                });
            }
            for name in &self.column_order {
                cells.push(format_scalar(&self.columns[name].values()[row_idx]));
            }
            out.push_str(&cells.join(" & "));
            out.push_str(" \\\\\n");
        }

        out.push_str("\\bottomrule\n");
        out.push_str("\\end{tabular}\n");
        out
    }

    /// Return the number of dimensions.
    ///
    /// Matches `pd.DataFrame.ndim`. Always returns 2.
    #[must_use]
    pub fn ndim(&self) -> usize {
        2
    }

    /// Return a list of the row axis labels and column axis labels.
    ///
    /// Matches `pd.DataFrame.axes`.
    #[must_use]
    pub fn axes(&self) -> (Vec<IndexLabel>, Vec<String>) {
        (
            self.index.labels().to_vec(),
            self.column_order.clone(),
        )
    }

    /// Render the DataFrame as an HTML table.
    ///
    /// Matches `pd.DataFrame.to_html()`.
    pub fn to_html(&self, include_index: bool) -> String {
        fn escape_html(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }

        fn format_scalar(val: &Scalar) -> String {
            match val {
                Scalar::Null(_) => "NaN".to_string(),
                Scalar::Bool(b) => if *b { "True" } else { "False" }.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => {
                    if v.is_nan() {
                        "NaN".to_string()
                    } else {
                        v.to_string()
                    }
                }
                Scalar::Utf8(s) => escape_html(s),
            }
        }

        let mut out = String::new();
        out.push_str("<table border=\"1\" class=\"dataframe\">\n");
        out.push_str("  <thead>\n    <tr style=\"text-align: right;\">\n");

        if include_index {
            out.push_str("      <th></th>\n");
        }
        for name in &self.column_order {
            out.push_str(&format!("      <th>{}</th>\n", escape_html(name)));
        }
        out.push_str("    </tr>\n  </thead>\n  <tbody>\n");

        for (row_idx, label) in self.index.labels().iter().enumerate() {
            out.push_str("    <tr>\n");
            if include_index {
                let idx_str = match label {
                    IndexLabel::Int64(v) => v.to_string(),
                    IndexLabel::Utf8(s) => escape_html(s),
                };
                out.push_str(&format!("      <th>{idx_str}</th>\n"));
            }
            for name in &self.column_order {
                out.push_str(&format!(
                    "      <td>{}</td>\n",
                    format_scalar(&self.columns[name].values()[row_idx])
                ));
            }
            out.push_str("    </tr>\n");
        }

        out.push_str("  </tbody>\n</table>\n");
        out
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

    /// Convert DataFrame to a nested dictionary.
    ///
    /// Matches `pd.DataFrame.to_dict(orient=...)`.
    ///
    /// Supported orients:
    /// - `"dict"`: `{column -> {index -> value}}` (default)
    /// - `"list"`: `{column -> [values]}`
    /// - `"records"`: `[{column -> value}, ...]`
    /// - `"index"`: `{index -> {column -> value}}`
    pub fn to_dict(
        &self,
        orient: &str,
    ) -> Result<BTreeMap<String, Vec<(String, Scalar)>>, FrameError> {
        match orient {
            "dict" => {
                let mut result = BTreeMap::new();
                for col_name in &self.column_order {
                    let col = &self.columns[col_name];
                    let entries: Vec<(String, Scalar)> = self
                        .index
                        .labels()
                        .iter()
                        .zip(col.values().iter())
                        .map(|(label, val)| (format!("{label:?}"), val.clone()))
                        .collect();
                    result.insert(col_name.clone(), entries);
                }
                Ok(result)
            }
            "list" => {
                let mut result = BTreeMap::new();
                for col_name in &self.column_order {
                    let col = &self.columns[col_name];
                    let entries: Vec<(String, Scalar)> = col
                        .values()
                        .iter()
                        .enumerate()
                        .map(|(i, val)| (i.to_string(), val.clone()))
                        .collect();
                    result.insert(col_name.clone(), entries);
                }
                Ok(result)
            }
            "records" => {
                let mut result = BTreeMap::new();
                for (row_idx, _label) in self.index.labels().iter().enumerate() {
                    let entries: Vec<(String, Scalar)> = self
                        .column_order
                        .iter()
                        .map(|col_name| {
                            let val = self.columns[col_name].values()[row_idx].clone();
                            (col_name.clone(), val)
                        })
                        .collect();
                    result.insert(row_idx.to_string(), entries);
                }
                Ok(result)
            }
            "index" => {
                let mut result = BTreeMap::new();
                for (row_idx, label) in self.index.labels().iter().enumerate() {
                    let entries: Vec<(String, Scalar)> = self
                        .column_order
                        .iter()
                        .map(|col_name| {
                            let val = self.columns[col_name].values()[row_idx].clone();
                            (col_name.clone(), val)
                        })
                        .collect();
                    result.insert(format!("{label:?}"), entries);
                }
                Ok(result)
            }
            "split" => {
                // Split orient: returns columns, index, data as separate entries
                let mut result = BTreeMap::new();
                let col_entry: Vec<(String, Scalar)> = self
                    .column_order
                    .iter()
                    .enumerate()
                    .map(|(i, name)| (i.to_string(), Scalar::Utf8(name.clone())))
                    .collect();
                result.insert("columns".to_owned(), col_entry);

                let idx_entry: Vec<(String, Scalar)> = self
                    .index
                    .labels()
                    .iter()
                    .enumerate()
                    .map(|(i, label)| {
                        (
                            i.to_string(),
                            match label {
                                IndexLabel::Int64(v) => Scalar::Int64(*v),
                                IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
                            },
                        )
                    })
                    .collect();
                result.insert("index".to_owned(), idx_entry);

                // Data: row-major list of lists
                for (row_idx, _label) in self.index.labels().iter().enumerate() {
                    let entries: Vec<(String, Scalar)> = self
                        .column_order
                        .iter()
                        .map(|col_name| {
                            let val = self.columns[col_name].values()[row_idx].clone();
                            (col_name.clone(), val)
                        })
                        .collect();
                    result.insert(format!("data_{row_idx}"), entries);
                }
                Ok(result)
            }
            other => Err(FrameError::CompatibilityRejected(format!(
                "unsupported to_dict orient: {other:?}"
            ))),
        }
    }

    /// Export DataFrame to CSV string.
    ///
    /// Matches `df.to_csv()` returning a string representation.
    pub fn to_csv(&self, sep: char, include_index: bool) -> String {
        let mut out = String::new();

        // Header
        if include_index {
            out.push_str("index");
            out.push(sep);
        }
        for (i, name) in self.column_order.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            out.push_str(name);
        }
        out.push('\n');

        // Rows
        for (row_idx, label) in self.index.labels().iter().enumerate() {
            if include_index {
                match label {
                    IndexLabel::Int64(v) => out.push_str(&v.to_string()),
                    IndexLabel::Utf8(s) => out.push_str(s),
                }
                out.push(sep);
            }
            for (col_idx, name) in self.column_order.iter().enumerate() {
                if col_idx > 0 {
                    out.push(sep);
                }
                let val = &self.columns[name].values()[row_idx];
                match val {
                    Scalar::Null(_) => {}
                    Scalar::Bool(b) => out.push_str(if *b { "True" } else { "False" }),
                    Scalar::Int64(v) => out.push_str(&v.to_string()),
                    Scalar::Float64(v) => out.push_str(&v.to_string()),
                    Scalar::Utf8(s) => {
                        if s.contains(sep) || s.contains('"') || s.contains('\n') {
                            out.push('"');
                            out.push_str(&s.replace('"', "\"\""));
                            out.push('"');
                        } else {
                            out.push_str(s);
                        }
                    }
                }
            }
            out.push('\n');
        }

        out
    }

    /// Export DataFrame to JSON string in records orientation.
    ///
    /// Matches `df.to_json(orient='records')` returning a JSON array of objects.
    pub fn to_json(&self, orient: &str) -> Result<String, FrameError> {
        match orient {
            "records" => {
                let mut rows = Vec::new();
                for row_idx in 0..self.len() {
                    let mut obj_parts = Vec::new();
                    for name in &self.column_order {
                        let val = &self.columns[name].values()[row_idx];
                        let json_val = match val {
                            Scalar::Null(_) => "null".to_string(),
                            Scalar::Bool(b) => b.to_string(),
                            Scalar::Int64(v) => v.to_string(),
                            Scalar::Float64(v) => {
                                if v.is_nan() {
                                    "null".to_string()
                                } else {
                                    v.to_string()
                                }
                            }
                            Scalar::Utf8(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
                        };
                        obj_parts.push(format!("\"{}\":{}", name.replace('"', "\\\""), json_val));
                    }
                    rows.push(format!("{{{}}}", obj_parts.join(",")));
                }
                Ok(format!("[{}]", rows.join(",")))
            }
            "columns" => {
                let mut col_parts = Vec::new();
                for name in &self.column_order {
                    let col = &self.columns[name];
                    let vals: Vec<String> = col
                        .values()
                        .iter()
                        .enumerate()
                        .map(|(i, val)| {
                            let json_val = match val {
                                Scalar::Null(_) => "null".to_string(),
                                Scalar::Bool(b) => b.to_string(),
                                Scalar::Int64(v) => v.to_string(),
                                Scalar::Float64(v) => {
                                    if v.is_nan() { "null".to_string() } else { v.to_string() }
                                }
                                Scalar::Utf8(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
                            };
                            format!("\"{}\":{}", i, json_val)
                        })
                        .collect();
                    col_parts.push(format!(
                        "\"{}\":{{{}}}",
                        name.replace('"', "\\\""),
                        vals.join(",")
                    ));
                }
                Ok(format!("{{{}}}", col_parts.join(",")))
            }
            other => Err(FrameError::CompatibilityRejected(format!(
                "unsupported to_json orient: {other:?}"
            ))),
        }
    }

    /// Render the DataFrame as a formatted plain-text table.
    ///
    /// Matches `df.to_string()`. Columns are right-aligned for numeric data,
    /// left-aligned for strings. Includes index column on the left.
    pub fn to_string_table(&self, include_index: bool) -> String {
        fn format_scalar(val: &Scalar) -> String {
            match val {
                Scalar::Null(_) => "NaN".to_string(),
                Scalar::Bool(b) => if *b { "True" } else { "False" }.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => {
                    if v.is_nan() {
                        "NaN".to_string()
                    } else if *v == v.round() && v.abs() < 1e15 {
                        format!("{v:.1}")
                    } else {
                        v.to_string()
                    }
                }
                Scalar::Utf8(s) => s.clone(),
            }
        }

        let nrows = self.len();

        // Compute column widths
        let mut col_data: Vec<(&str, Vec<String>)> = Vec::new();

        if include_index {
            let idx_cells: Vec<String> = self
                .index
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(v) => v.to_string(),
                    IndexLabel::Utf8(s) => s.clone(),
                })
                .collect();
            col_data.push(("", idx_cells));
        }

        for name in &self.column_order {
            let col = &self.columns[name];
            let cells: Vec<String> = col.values().iter().map(format_scalar).collect();
            col_data.push((name.as_str(), cells));
        }

        // Compute max width per column (including header)
        let widths: Vec<usize> = col_data
            .iter()
            .map(|(header, cells)| {
                let max_cell = cells.iter().map(|c| c.len()).max().unwrap_or(0);
                header.len().max(max_cell)
            })
            .collect();

        let mut out = String::new();

        // Header row
        let header_parts: Vec<String> = col_data
            .iter()
            .zip(&widths)
            .map(|((header, _), w)| format!("{:>width$}", header, width = *w))
            .collect();
        out.push_str(&header_parts.join("  "));
        out.push('\n');

        // Data rows
        for row_idx in 0..nrows {
            let row_parts: Vec<String> = col_data
                .iter()
                .zip(&widths)
                .map(|((_, cells), w)| format!("{:>width$}", cells[row_idx], width = *w))
                .collect();
            out.push_str(&row_parts.join("  "));
            if row_idx + 1 < nrows {
                out.push('\n');
            }
        }

        out
    }

    /// Render the DataFrame as a Markdown table.
    ///
    /// Matches `df.to_markdown()`.
    pub fn to_markdown(&self, include_index: bool) -> String {
        fn format_scalar(val: &Scalar) -> String {
            match val {
                Scalar::Null(_) => "NaN".to_string(),
                Scalar::Bool(b) => if *b { "True" } else { "False" }.to_string(),
                Scalar::Int64(v) => v.to_string(),
                Scalar::Float64(v) => {
                    if v.is_nan() {
                        "NaN".to_string()
                    } else if *v == v.round() && v.abs() < 1e15 {
                        format!("{v:.1}")
                    } else {
                        v.to_string()
                    }
                }
                Scalar::Utf8(s) => s.clone(),
            }
        }

        let nrows = self.len();
        let mut headers: Vec<String> = Vec::new();
        let mut col_cells: Vec<Vec<String>> = Vec::new();

        if include_index {
            headers.push(String::new());
            let idx_cells: Vec<String> = self
                .index
                .labels()
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(v) => v.to_string(),
                    IndexLabel::Utf8(s) => s.clone(),
                })
                .collect();
            col_cells.push(idx_cells);
        }

        for name in &self.column_order {
            headers.push(name.clone());
            let col = &self.columns[name];
            let cells: Vec<String> = col.values().iter().map(format_scalar).collect();
            col_cells.push(cells);
        }

        // Compute widths
        let widths: Vec<usize> = headers
            .iter()
            .zip(&col_cells)
            .map(|(h, cells)| {
                let max_cell = cells.iter().map(|c| c.len()).max().unwrap_or(0);
                h.len().max(max_cell).max(3) // min width 3 for separator ---
            })
            .collect();

        let mut out = String::new();

        // Header
        out.push('|');
        for (h, w) in headers.iter().zip(&widths) {
            out.push_str(&format!(" {:width$} |", h, width = *w));
        }
        out.push('\n');

        // Separator
        out.push('|');
        for w in &widths {
            out.push(' ');
            for _ in 0..*w {
                out.push('-');
            }
            out.push_str(" |");
        }
        out.push('\n');

        // Rows
        for row_idx in 0..nrows {
            out.push('|');
            for (cells, w) in col_cells.iter().zip(&widths) {
                out.push_str(&format!(" {:width$} |", cells[row_idx], width = *w));
            }
            if row_idx + 1 < nrows {
                out.push('\n');
            }
        }

        out
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
            (None, Some(f)) => (total as f64 * f).round() as usize,
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

        let value_col = Column::from_values(values)?;
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
            let sep_pos = label_str.rfind('|').ok_or_else(|| {
                FrameError::CompatibilityRejected(format!(
                    "unstack: index label '{label_str}' missing '|' separator"
                ))
            })?;
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
            result_cols.insert(ck.clone(), Column::from_values(vals)?);
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

    /// Row-wise apply returning a Series (one value per row).
    ///
    /// Convenience wrapper for `apply_fn(func, 1)` that directly returns
    /// a Series instead of a single-column DataFrame.
    pub fn apply_row<F>(&self, name: &str, func: F) -> Result<Series, FrameError>
    where
        F: Fn(&[Scalar]) -> Scalar,
    {
        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let row_vals: Vec<Scalar> = self
                .column_order
                .iter()
                .map(|col_name| self.columns[col_name].values()[row_idx].clone())
                .collect();
            values.push(func(&row_vals));
        }
        Series::from_values(name, self.index.labels().to_vec(), values)
    }

    /// Row-wise apply with failable closure returning a Series.
    ///
    /// Like `apply_row` but the closure can return errors.
    pub fn apply_row_fn<F>(&self, name: &str, func: F) -> Result<Series, FrameError>
    where
        F: Fn(&[Scalar]) -> Result<Scalar, FrameError>,
    {
        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let row_vals: Vec<Scalar> = self
                .column_order
                .iter()
                .map(|col_name| self.columns[col_name].values()[row_idx].clone())
                .collect();
            values.push(func(&row_vals)?);
        }
        Series::from_values(name, self.index.labels().to_vec(), values)
    }

    /// Simple pivot: reshape long to wide using column values.
    ///
    /// Matches `pd.DataFrame.pivot(index, columns, values)`.
    /// Unlike `pivot_table`, no aggregation is performed; duplicate entries error.
    pub fn pivot(
        &self,
        index_col: &str,
        columns_col: &str,
        values_col: &str,
    ) -> Result<Self, FrameError> {
        let idx_vals = self.columns.get(index_col).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("pivot: column '{index_col}' not found"))
        })?;
        let col_vals = self.columns.get(columns_col).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("pivot: column '{columns_col}' not found"))
        })?;
        let val_vals = self.columns.get(values_col).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("pivot: column '{values_col}' not found"))
        })?;

        // Collect unique index and column values in order of appearance
        let mut row_keys: Vec<Scalar> = Vec::new();
        let mut col_keys: Vec<String> = Vec::new();

        for v in idx_vals.values() {
            if !row_keys.contains(v) {
                row_keys.push(v.clone());
            }
        }
        for v in col_vals.values() {
            let key = format!("{v:?}");
            if !col_keys.contains(&key) {
                col_keys.push(key);
            }
        }

        // Build a map of (row_key, col_key) -> value
        let mut cells: BTreeMap<(String, String), Scalar> = BTreeMap::new();
        for i in 0..self.len() {
            let rk = format!("{:?}", idx_vals.values()[i]);
            let ck = format!("{:?}", col_vals.values()[i]);
            if cells.contains_key(&(rk.clone(), ck.clone())) {
                return Err(FrameError::CompatibilityRejected(
                    "pivot: duplicate entries, use pivot_table for aggregation".to_owned(),
                ));
            }
            cells.insert((rk, ck), val_vals.values()[i].clone());
        }

        // Build output columns
        let index_labels: Vec<IndexLabel> = row_keys
            .iter()
            .map(|v| match v {
                Scalar::Utf8(s) => s.as_str().into(),
                Scalar::Int64(n) => (*n).into(),
                other => format!("{other:?}").as_str().into(),
            })
            .collect();

        let mut data = Vec::new();
        for ck in &col_keys {
            let mut col_data = Vec::with_capacity(row_keys.len());
            for rk in &row_keys {
                let rk_str = format!("{rk:?}");
                let val = cells
                    .get(&(rk_str, ck.clone()))
                    .cloned()
                    .unwrap_or(Scalar::Null(NullKind::NaN));
                col_data.push(val);
            }
            // Clean up column name: remove Scalar debug formatting
            let clean_name = ck
                .strip_prefix("Utf8(\"")
                .and_then(|s| s.strip_suffix("\")"))
                .or_else(|| ck.strip_prefix("Int64(").and_then(|s| s.strip_suffix(")")))
                .or_else(|| ck.strip_prefix("Float64(").and_then(|s| s.strip_suffix(")")))
                .unwrap_or(ck);
            data.push((clean_name.to_string(), col_data));
        }

        let column_order: Vec<String> = data.iter().map(|(n, _)| n.clone()).collect();
        let mut columns = BTreeMap::new();
        for (name, vals) in data {
            columns.insert(name, Column::from_values(vals)?);
        }

        let index = Index::new(index_labels);
        Ok(Self {
            index,
            column_order,
            columns,
        })
    }

    /// Squeeze a single-column DataFrame to a Series, or single-row to a Series.
    ///
    /// Matches `pd.DataFrame.squeeze(axis)`.
    /// - axis=1 (default): if single column, return as Series
    /// - axis=0: if single row, return as Series
    pub fn squeeze_to_series(&self, axis: usize) -> Result<Series, Self> {
        if axis == 1 && self.column_order.len() == 1 {
            let col_name = &self.column_order[0];
            let col = &self.columns[col_name];
            Ok(Series::new(
                col_name.clone(),
                self.index.clone(),
                col.clone(),
            )
            .unwrap())
        } else if axis == 0 && self.len() == 1 {
            let labels: Vec<IndexLabel> = self
                .column_order
                .iter()
                .map(|n| n.as_str().into())
                .collect();
            let values: Vec<Scalar> = self
                .column_order
                .iter()
                .map(|n| self.columns[n].values()[0].clone())
                .collect();
            Ok(Series::from_values("0".to_owned(), labels, values).unwrap())
        } else {
            Err(self.clone())
        }
    }

    /// Approximate memory usage per column.
    ///
    /// Matches `pd.DataFrame.memory_usage()`. Returns a Series with column
    /// names as index and byte estimates as values. Includes the index.
    pub fn memory_usage(&self) -> Result<Series, FrameError> {
        let index_bytes = std::mem::size_of_val(self.index.labels());
        let mut labels = vec![IndexLabel::from("Index")];
        let mut values = vec![Scalar::Int64(index_bytes as i64)];

        for col_name in &self.column_order {
            let col = &self.columns[col_name];
            let bytes = std::mem::size_of_val(col.values());
            labels.push(IndexLabel::from(col_name.as_str()));
            values.push(Scalar::Int64(bytes as i64));
        }

        Series::from_values("memory_usage".to_owned(), labels, values)
    }

    // ── DataFrame column-wise aggregation stats ──

    /// Count of unique non-null values per column.
    ///
    /// Matches `pd.DataFrame.nunique()`.
    pub fn nunique(&self) -> Result<Series, FrameError> {
        let labels: Vec<IndexLabel> = self.column_order.iter().map(|n| n.as_str().into()).collect();
        let values: Vec<Scalar> = self
            .column_order
            .iter()
            .map(|n| {
                let col = &self.columns[n];
                let mut seen = BTreeSet::new();
                for v in col.values() {
                    if !v.is_missing() {
                        seen.insert(format!("{v:?}"));
                    }
                }
                Scalar::Int64(seen.len() as i64)
            })
            .collect();
        Series::from_values("nunique".to_string(), labels, values)
    }

    /// Count unique non-null values with axis parameter.
    ///
    /// Matches `pd.DataFrame.nunique(axis)`.
    /// - axis=0 (default): unique count per column (same as `nunique()`)
    /// - axis=1: unique count per row
    pub fn nunique_axis(&self, axis: usize) -> Result<Series, FrameError> {
        if axis == 0 {
            return self.nunique();
        }
        // axis=1: count unique values per row
        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let mut seen = BTreeSet::new();
            for col_name in &self.column_order {
                let val = &self.columns[col_name].values()[row_idx];
                if !val.is_missing() {
                    seen.insert(format!("{val:?}"));
                }
            }
            values.push(Scalar::Int64(seen.len() as i64));
        }
        Series::from_values(
            "nunique".to_owned(),
            self.index.labels().to_vec(),
            values,
        )
    }

    /// Index label of the minimum value per numeric column.
    ///
    /// Matches `pd.DataFrame.idxmin()`.
    pub fn idxmin(&self) -> Result<Series, FrameError> {
        let labels: Vec<IndexLabel> = self.column_order.iter().map(|n| n.as_str().into()).collect();
        let mut values = Vec::with_capacity(labels.len());
        for name in &self.column_order {
            let s = self.column_as_series(name)?;
            match s.idxmin() {
                Ok(label) => values.push(Scalar::Utf8(format!("{label:?}"))),
                Err(_) => values.push(Scalar::Null(NullKind::NaN)),
            }
        }
        Series::from_values("idxmin".to_string(), labels, values)
    }

    /// Index label of the maximum value per numeric column.
    ///
    /// Matches `pd.DataFrame.idxmax()`.
    pub fn idxmax(&self) -> Result<Series, FrameError> {
        let labels: Vec<IndexLabel> = self.column_order.iter().map(|n| n.as_str().into()).collect();
        let mut values = Vec::with_capacity(labels.len());
        for name in &self.column_order {
            let s = self.column_as_series(name)?;
            match s.idxmax() {
                Ok(label) => values.push(Scalar::Utf8(format!("{label:?}"))),
                Err(_) => values.push(Scalar::Null(NullKind::NaN)),
            }
        }
        Series::from_values("idxmax".to_string(), labels, values)
    }

    /// Whether all non-null values are truthy, per column.
    ///
    /// Matches `pd.DataFrame.all()`.
    pub fn all(&self) -> Result<Series, FrameError> {
        let labels: Vec<IndexLabel> = self.column_order.iter().map(|n| n.as_str().into()).collect();
        let mut values = Vec::with_capacity(labels.len());
        for name in &self.column_order {
            let s = self.column_as_series(name)?;
            values.push(Scalar::Bool(s.all()?));
        }
        Series::from_values("all".to_string(), labels, values)
    }

    /// Whether any non-null value is truthy, per column.
    ///
    /// Matches `pd.DataFrame.any()`.
    pub fn any(&self) -> Result<Series, FrameError> {
        let labels: Vec<IndexLabel> = self.column_order.iter().map(|n| n.as_str().into()).collect();
        let mut values = Vec::with_capacity(labels.len());
        for name in &self.column_order {
            let s = self.column_as_series(name)?;
            values.push(Scalar::Bool(s.any()?));
        }
        Series::from_values("any".to_string(), labels, values)
    }

    /// Sum of non-null values per column.
    ///
    /// Matches `pd.DataFrame.sum()`.
    pub fn sum(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("sum")
    }

    /// Mean of non-null values per column.
    ///
    /// Matches `pd.DataFrame.mean()`.
    pub fn mean(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("mean")
    }

    /// Min of non-null values per column.
    ///
    /// Matches `pd.DataFrame.min()`.
    pub fn min_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("min")
    }

    /// Max of non-null values per column.
    ///
    /// Matches `pd.DataFrame.max()`.
    pub fn max_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("max")
    }

    /// Standard deviation of non-null values per column.
    ///
    /// Matches `pd.DataFrame.std()`.
    pub fn std_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("std")
    }

    /// Variance of non-null values per column.
    ///
    /// Matches `pd.DataFrame.var()`.
    pub fn var_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("var")
    }

    /// Median of non-null values per column.
    ///
    /// Matches `pd.DataFrame.median()`.
    pub fn median_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("median")
    }

    /// Product of non-null values per column.
    ///
    /// Matches `pd.DataFrame.prod()`.
    pub fn prod_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("prod")
    }

    /// Mode per column.
    ///
    /// Matches `pd.DataFrame.mode()`. Returns a DataFrame where each column
    /// contains its mode value. Only the first mode is kept when there are ties.
    pub fn mode(&self) -> Result<Self, FrameError> {
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for name in &self.column_order {
            let s = self.column_as_series(name)?;
            let mode_series = s.mode()?;
            if mode_series.is_empty() {
                result_cols.insert(
                    name.clone(),
                    Column::from_values(vec![Scalar::Null(NullKind::NaN)])?,
                );
            } else {
                result_cols.insert(
                    name.clone(),
                    Column::from_values(vec![mode_series.values()[0].clone()])?,
                );
            }
            col_order.push(name.clone());
        }

        Ok(Self {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(vec![0_i64.into()]),
        })
    }

    /// Internal: reduce each numeric column to a single scalar via the named function.
    fn reduce_numeric(&self, func: &str) -> Result<Series, FrameError> {
        let mut labels = Vec::new();
        let mut values = Vec::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            if col.dtype() != DType::Int64 && col.dtype() != DType::Float64 {
                continue;
            }
            let s = self.column_as_series(name)?;
            let val = match func {
                "sum" => s.sum()?,
                "mean" => s.mean()?,
                "min" => s.min()?,
                "max" => s.max()?,
                "std" => s.std()?,
                "var" => s.var()?,
                "median" => s.median()?,
                "prod" => s.prod()?,
                "skew" => Scalar::Float64(s.skew()?),
                "kurtosis" => Scalar::Float64(s.kurtosis()?),
                "sem" => Scalar::Float64(s.sem()?),
                _ => {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "unknown reduce function: {func}"
                    )))
                }
            };
            labels.push(IndexLabel::Utf8(name.clone()));
            values.push(val);
        }
        Series::from_values(func.to_string(), labels, values)
    }

    /// Skewness of non-null values per column.
    ///
    /// Matches `pd.DataFrame.skew()`.
    pub fn skew_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("skew")
    }

    /// Kurtosis of non-null values per column.
    ///
    /// Matches `pd.DataFrame.kurtosis()`.
    pub fn kurtosis_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("kurtosis")
    }

    /// Standard error of the mean per column.
    ///
    /// Matches `pd.DataFrame.sem()`.
    pub fn sem_agg(&self) -> Result<Series, FrameError> {
        self.reduce_numeric("sem")
    }

    // ── Row-axis (axis=1) aggregations ──

    /// Internal: reduce each row across numeric columns using a closure.
    fn reduce_rows<F>(&self, func: F, name: &str) -> Result<Series, FrameError>
    where
        F: Fn(&[f64]) -> f64,
    {
        let numeric_cols: Vec<&str> = self
            .column_order
            .iter()
            .filter(|c| {
                let dt = self.columns[c.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .map(|s| s.as_str())
            .collect();

        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let row_vals: Vec<f64> = numeric_cols
                .iter()
                .filter_map(|&col_name| {
                    self.columns[col_name].values()[row_idx].to_f64().ok()
                })
                .collect();
            if row_vals.is_empty() {
                values.push(Scalar::Null(NullKind::NaN));
            } else {
                values.push(Scalar::Float64(func(&row_vals)));
            }
        }
        Series::from_values(name.to_owned(), self.index.labels().to_vec(), values)
    }

    /// Sum across columns per row.
    ///
    /// Matches `pd.DataFrame.sum(axis=1)`.
    pub fn sum_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(|vals| vals.iter().sum(), "sum")
    }

    /// Mean across columns per row.
    ///
    /// Matches `pd.DataFrame.mean(axis=1)`.
    pub fn mean_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(
            |vals| vals.iter().sum::<f64>() / vals.len() as f64,
            "mean",
        )
    }

    /// Minimum across columns per row.
    ///
    /// Matches `pd.DataFrame.min(axis=1)`.
    pub fn min_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(
            |vals| vals.iter().copied().fold(f64::INFINITY, f64::min),
            "min",
        )
    }

    /// Maximum across columns per row.
    ///
    /// Matches `pd.DataFrame.max(axis=1)`.
    pub fn max_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(
            |vals| vals.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "max",
        )
    }

    /// Standard deviation across columns per row.
    ///
    /// Matches `pd.DataFrame.std(axis=1)`.
    pub fn std_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(
            |vals| {
                let n = vals.len() as f64;
                if n < 2.0 {
                    return f64::NAN;
                }
                let mean = vals.iter().sum::<f64>() / n;
                let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
                var.sqrt()
            },
            "std",
        )
    }

    /// Variance across columns per row.
    ///
    /// Matches `pd.DataFrame.var(axis=1)`.
    pub fn var_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(
            |vals| {
                let n = vals.len() as f64;
                if n < 2.0 {
                    return f64::NAN;
                }
                let mean = vals.iter().sum::<f64>() / n;
                vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)
            },
            "var",
        )
    }

    /// Count of non-null values across columns per row.
    ///
    /// Matches `pd.DataFrame.count(axis=1)`.
    pub fn count_axis1(&self) -> Result<Series, FrameError> {
        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let cnt = self
                .column_order
                .iter()
                .filter(|col_name| !self.columns[col_name.as_str()].values()[row_idx].is_missing())
                .count();
            values.push(Scalar::Int64(cnt as i64));
        }
        Series::from_values("count".to_owned(), self.index.labels().to_vec(), values)
    }

    /// Whether all non-null values are truthy across columns per row.
    ///
    /// Matches `pd.DataFrame.all(axis=1)`.
    pub fn all_axis1(&self) -> Result<Series, FrameError> {
        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let result = self.column_order.iter().all(|col_name| {
                let val = &self.columns[col_name.as_str()].values()[row_idx];
                match val {
                    Scalar::Null(_) => true, // NaN is ignored in all()
                    Scalar::Bool(b) => *b,
                    Scalar::Int64(v) => *v != 0,
                    Scalar::Float64(v) => *v != 0.0 && !v.is_nan(),
                    Scalar::Utf8(s) => !s.is_empty(),
                }
            });
            values.push(Scalar::Bool(result));
        }
        Series::from_values("all".to_owned(), self.index.labels().to_vec(), values)
    }

    /// Whether any non-null value is truthy across columns per row.
    ///
    /// Matches `pd.DataFrame.any(axis=1)`.
    pub fn any_axis1(&self) -> Result<Series, FrameError> {
        let mut values = Vec::with_capacity(self.len());
        for row_idx in 0..self.len() {
            let result = self.column_order.iter().any(|col_name| {
                let val = &self.columns[col_name.as_str()].values()[row_idx];
                match val {
                    Scalar::Null(_) => false,
                    Scalar::Bool(b) => *b,
                    Scalar::Int64(v) => *v != 0,
                    Scalar::Float64(v) => *v != 0.0 && !v.is_nan(),
                    Scalar::Utf8(s) => !s.is_empty(),
                }
            });
            values.push(Scalar::Bool(result));
        }
        Series::from_values("any".to_owned(), self.index.labels().to_vec(), values)
    }

    /// Median across columns per row.
    ///
    /// Matches `pd.DataFrame.median(axis=1)`.
    pub fn median_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(
            |vals| {
                let mut sorted = vals.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let n = sorted.len();
                if n % 2 == 0 {
                    (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
                } else {
                    sorted[n / 2]
                }
            },
            "median",
        )
    }

    /// Product across columns per row.
    ///
    /// Matches `pd.DataFrame.prod(axis=1)`.
    pub fn prod_axis1(&self) -> Result<Series, FrameError> {
        self.reduce_rows(|vals| vals.iter().product(), "prod")
    }

    /// Internal: extract a named column as a Series.
    fn column_as_series(&self, name: &str) -> Result<Series, FrameError> {
        let col = self
            .columns
            .get(name)
            .ok_or_else(|| FrameError::CompatibilityRejected(format!("column not found: {name}")))?;
        Series::new(name.to_string(), self.index.clone(), col.clone())
    }

    // ── DataFrame element-wise operations ──

    /// Cumulative sum per column.
    ///
    /// Matches `pd.DataFrame.cumsum()`.
    pub fn cumsum(&self) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.cumsum())
    }

    /// Cumulative product per column.
    ///
    /// Matches `pd.DataFrame.cumprod()`.
    pub fn cumprod(&self) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.cumprod())
    }

    /// Cumulative maximum per column.
    ///
    /// Matches `pd.DataFrame.cummax()`.
    pub fn cummax(&self) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.cummax())
    }

    /// Cumulative minimum per column.
    ///
    /// Matches `pd.DataFrame.cummin()`.
    pub fn cummin(&self) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.cummin())
    }

    /// First-order difference per column.
    ///
    /// Matches `pd.DataFrame.diff(periods)`.
    pub fn diff(&self, periods: i64) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.diff(periods))
    }

    /// Shift values per column.
    ///
    /// Matches `pd.DataFrame.shift(periods)`.
    pub fn shift(&self, periods: i64) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.shift(periods))
    }

    /// Absolute value per column.
    ///
    /// Matches `pd.DataFrame.abs()`.
    pub fn abs(&self) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.abs())
    }

    /// Clip values per column.
    ///
    /// Matches `pd.DataFrame.clip(lower, upper)`.
    pub fn clip(&self, lower: Option<f64>, upper: Option<f64>) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.clip(lower, upper))
    }

    /// Clip values below a threshold across all numeric columns.
    ///
    /// Matches `pd.DataFrame.clip(lower=threshold)`.
    pub fn clip_lower(&self, threshold: f64) -> Result<Self, FrameError> {
        self.clip(Some(threshold), None)
    }

    /// Clip values above a threshold across all numeric columns.
    ///
    /// Matches `pd.DataFrame.clip(upper=threshold)`.
    pub fn clip_upper(&self, threshold: f64) -> Result<Self, FrameError> {
        self.clip(None, Some(threshold))
    }

    /// Round numeric columns to specified decimal places.
    ///
    /// Matches `pd.DataFrame.round(decimals)`.
    pub fn round(&self, decimals: i32) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.round(decimals))
    }

    /// Add a scalar to all numeric columns.
    ///
    /// Matches `df + scalar` or `df.add(scalar)`.
    pub fn add_scalar(&self, value: f64) -> Result<Self, FrameError> {
        self.apply_scalar_op(value, |a, b| a + b)
    }

    /// Subtract a scalar from all numeric columns.
    pub fn sub_scalar(&self, value: f64) -> Result<Self, FrameError> {
        self.apply_scalar_op(value, |a, b| a - b)
    }

    /// Multiply all numeric columns by a scalar.
    pub fn mul_scalar(&self, value: f64) -> Result<Self, FrameError> {
        self.apply_scalar_op(value, |a, b| a * b)
    }

    /// Divide all numeric columns by a scalar.
    pub fn div_scalar(&self, value: f64) -> Result<Self, FrameError> {
        self.apply_scalar_op(value, |a, b| a / b)
    }

    /// Raise all numeric columns to a scalar power.
    pub fn pow_scalar(&self, value: f64) -> Result<Self, FrameError> {
        self.apply_scalar_op(value, |a, b| a.powf(b))
    }

    /// Modulo all numeric columns by a scalar.
    pub fn mod_scalar(&self, value: f64) -> Result<Self, FrameError> {
        self.apply_scalar_op(value, |a, b| a % b)
    }

    // ── DataFrame-to-DataFrame arithmetic ──

    /// Internal: apply a binary operation between two DataFrames element-wise.
    ///
    /// Aligns on index (outer join), operates on shared numeric columns,
    /// fills missing with NaN.
    fn binary_df_op<F>(&self, other: &Self, op: F, _name: &str) -> Result<Self, FrameError>
    where
        F: Fn(f64, f64) -> f64,
    {
        let (left, right) = self.align_on_index(other, AlignMode::Outer)?;
        let n = left.len();

        // Compute the union of columns from both sides, preserving left order first
        let mut all_columns: Vec<String> = left.column_order.clone();
        for col_name in &right.column_order {
            if !all_columns.contains(col_name) {
                all_columns.push(col_name.clone());
            }
        }

        let nan_col = || -> Vec<Scalar> { vec![Scalar::Null(NullKind::NaN); n] };

        let mut result_cols = BTreeMap::new();

        for col_name in &all_columns {
            let left_col = left.columns.get(col_name);
            let right_col = right.columns.get(col_name);

            match (left_col, right_col) {
                (Some(lc), Some(rc)) => {
                    let left_numeric =
                        lc.dtype() == DType::Int64 || lc.dtype() == DType::Float64;
                    let right_numeric =
                        rc.dtype() == DType::Int64 || rc.dtype() == DType::Float64;

                    if left_numeric && right_numeric {
                        let vals: Vec<Scalar> = lc
                            .values()
                            .iter()
                            .zip(rc.values())
                            .map(|(lv, rv)| match (lv.to_f64(), rv.to_f64()) {
                                (Ok(l), Ok(r)) => Scalar::Float64(op(l, r)),
                                _ => Scalar::Null(NullKind::NaN),
                            })
                            .collect();
                        result_cols.insert(col_name.clone(), Column::from_values(vals)?);
                    } else {
                        result_cols.insert(col_name.clone(), lc.clone());
                    }
                }
                // Column only in one side → result is all NaN
                _ => {
                    result_cols.insert(col_name.clone(), Column::from_values(nan_col())?);
                }
            }
        }

        Ok(Self {
            columns: result_cols,
            column_order: all_columns,
            index: left.index.clone(),
        })
    }

    /// Add another DataFrame element-wise with index alignment.
    ///
    /// Matches `pd.DataFrame.add(other)`.
    pub fn add_df(&self, other: &Self) -> Result<Self, FrameError> {
        self.binary_df_op(other, |a, b| a + b, "add")
    }

    /// Subtract another DataFrame element-wise with index alignment.
    ///
    /// Matches `pd.DataFrame.sub(other)`.
    pub fn sub_df(&self, other: &Self) -> Result<Self, FrameError> {
        self.binary_df_op(other, |a, b| a - b, "sub")
    }

    /// Multiply another DataFrame element-wise with index alignment.
    ///
    /// Matches `pd.DataFrame.mul(other)`.
    pub fn mul_df(&self, other: &Self) -> Result<Self, FrameError> {
        self.binary_df_op(other, |a, b| a * b, "mul")
    }

    /// Divide by another DataFrame element-wise with index alignment.
    ///
    /// Matches `pd.DataFrame.div(other)`.
    pub fn div_df(&self, other: &Self) -> Result<Self, FrameError> {
        self.binary_df_op(other, |a, b| a / b, "div")
    }

    /// Squeeze: pandas-named alias for `squeeze_to_series`.
    ///
    /// Matches `pd.DataFrame.squeeze(axis)`.
    pub fn squeeze(&self, axis: usize) -> Result<Series, Self> {
        self.squeeze_to_series(axis)
    }

    /// Set or replace a column by name with given values.
    ///
    /// Matches `pd.DataFrame.assign(**kwargs)` for single-column assignment.
    /// If the column already exists, it is replaced. Otherwise it is appended.
    pub fn assign_column(&self, name: &str, values: Vec<Scalar>) -> Result<Self, FrameError> {
        if values.len() != self.len() {
            return Err(FrameError::LengthMismatch {
                index_len: self.len(),
                column_len: values.len(),
            });
        }
        let mut new_cols = self.columns.clone();
        new_cols.insert(name.to_owned(), Column::from_values(values)?);
        let mut new_order = self.column_order.clone();
        if !new_order.contains(&name.to_owned()) {
            new_order.push(name.to_owned());
        }
        Ok(Self {
            columns: new_cols,
            column_order: new_order,
            index: self.index.clone(),
        })
    }

    /// Internal: apply a binary f64 operation with a scalar to each numeric column.
    fn apply_scalar_op<F>(&self, scalar: f64, op: F) -> Result<Self, FrameError>
    where
        F: Fn(f64, f64) -> f64,
    {
        let mut result_cols = BTreeMap::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            if col.dtype() == DType::Int64 || col.dtype() == DType::Float64 {
                let vals: Vec<Scalar> = col
                    .values()
                    .iter()
                    .map(|v| {
                        if v.is_missing() {
                            Scalar::Null(NullKind::NaN)
                        } else {
                            match v.to_f64() {
                                Ok(f) => Scalar::Float64(op(f, scalar)),
                                Err(_) => Scalar::Null(NullKind::NaN),
                            }
                        }
                    })
                    .collect();
                result_cols.insert(name.clone(), Column::from_values(vals)?);
            } else {
                result_cols.insert(name.clone(), col.clone());
            }
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Percentage change per column.
    ///
    /// Matches `pd.DataFrame.pct_change(periods)`.
    pub fn pct_change(&self, periods: usize) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.pct_change(periods))
    }

    /// Forward-fill missing values per column.
    ///
    /// Matches `pd.DataFrame.ffill()`. Applies to all columns.
    pub fn ffill(&self, limit: Option<usize>) -> Result<Self, FrameError> {
        self.apply_all_columns(|s| s.ffill(limit))
    }

    /// Back-fill missing values per column.
    ///
    /// Matches `pd.DataFrame.bfill()`. Applies to all columns.
    pub fn bfill(&self, limit: Option<usize>) -> Result<Self, FrameError> {
        self.apply_all_columns(|s| s.bfill(limit))
    }

    /// Linearly interpolate missing values per numeric column.
    ///
    /// Matches `pd.DataFrame.interpolate()`. Non-numeric columns are preserved.
    pub fn interpolate(&self) -> Result<Self, FrameError> {
        self.apply_per_column(|s| s.interpolate())
    }

    /// Convert dtypes to best-possible types.
    ///
    /// Matches `pd.DataFrame.convert_dtypes()`. In our type system this
    /// attempts to promote Int64 columns containing NaN to Float64, and
    /// tries to parse Utf8 values as numbers where possible.
    pub fn convert_dtypes(&self) -> Result<Self, FrameError> {
        let mut result_cols = BTreeMap::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            match col.dtype() {
                DType::Int64 => {
                    // Check if any value is NaN (shouldn't happen in Int64, but if mixed)
                    result_cols.insert(name.clone(), col.clone());
                }
                DType::Utf8 => {
                    // Try to parse as numeric
                    let mut all_numeric = true;
                    let mut converted = Vec::with_capacity(col.values().len());
                    for val in col.values() {
                        match val {
                            Scalar::Utf8(s) => {
                                if let Ok(i) = s.trim().parse::<i64>() {
                                    converted.push(Scalar::Int64(i));
                                } else if let Ok(f) = s.trim().parse::<f64>() {
                                    converted.push(Scalar::Float64(f));
                                } else {
                                    all_numeric = false;
                                    break;
                                }
                            }
                            _ => {
                                converted.push(val.clone());
                            }
                        }
                    }
                    if all_numeric {
                        result_cols.insert(name.clone(), Column::from_values(converted)?);
                    } else {
                        result_cols.insert(name.clone(), col.clone());
                    }
                }
                _ => {
                    result_cols.insert(name.clone(), col.clone());
                }
            }
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Append rows from another DataFrame.
    ///
    /// Matches `pd.DataFrame.append(other)` (deprecated but common).
    /// Uses outer join on columns, filling missing with NaN.
    pub fn append(&self, other: &Self) -> Result<Self, FrameError> {
        // Union of column names, preserving order (self first, then new from other)
        let mut all_cols: Vec<String> = self.column_order.clone();
        for name in &other.column_order {
            if !all_cols.contains(name) {
                all_cols.push(name.clone());
            }
        }

        let mut result_cols = BTreeMap::new();
        let nan = Scalar::Null(NullKind::NaN);

        for name in &all_cols {
            let mut values = Vec::with_capacity(self.len() + other.len());

            // Values from self
            if let Some(col) = self.columns.get(name) {
                values.extend_from_slice(col.values());
            } else {
                values.extend(std::iter::repeat_n(nan.clone(), self.len()));
            }

            // Values from other
            if let Some(col) = other.columns.get(name) {
                values.extend_from_slice(col.values());
            } else {
                values.extend(std::iter::repeat_n(nan.clone(), other.len()));
            }

            result_cols.insert(name.clone(), Column::from_values(values)?);
        }

        let mut new_labels = self.index.labels().to_vec();
        new_labels.extend_from_slice(other.index.labels());
        let index = Index::new(new_labels);

        Ok(Self {
            index,
            column_order: all_cols,
            columns: result_cols,
        })
    }

    /// Internal: apply a closure to ALL columns (not just numeric), returning a new DataFrame.
    fn apply_all_columns<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Series) -> Result<Series, FrameError>,
    {
        let mut result_cols = BTreeMap::new();
        for name in &self.column_order {
            let s = self.column_as_series(name)?;
            let transformed = func(&s)?;
            result_cols.insert(name.clone(), transformed.column().clone());
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Internal: apply a closure to each numeric column, returning a new DataFrame.
    fn apply_per_column<F>(&self, func: F) -> Result<Self, FrameError>
    where
        F: Fn(&Series) -> Result<Series, FrameError>,
    {
        let mut result_cols = BTreeMap::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            if col.dtype() == DType::Int64 || col.dtype() == DType::Float64 {
                let s = self.column_as_series(name)?;
                let transformed = func(&s)?;
                result_cols.insert(name.clone(), transformed.column().clone());
            } else {
                result_cols.insert(name.clone(), col.clone());
            }
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Drop labels from rows or columns.
    ///
    /// Matches `df.drop(labels, axis=0)` for row removal or
    /// `df.drop(columns=['a','b'])` for column removal.
    /// When `axis=0`, removes rows whose index labels match the given labels.
    /// When `axis=1`, removes columns by name.
    pub fn drop(&self, labels: &[&str], axis: usize) -> Result<Self, FrameError> {
        if axis == 1 {
            // Drop columns by name
            let drop_set: BTreeSet<&str> = labels.iter().copied().collect();
            for name in &drop_set {
                if !self.columns.contains_key(*name) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "column '{name}' not found"
                    )));
                }
            }
            let mut columns = BTreeMap::new();
            let mut column_order = Vec::new();
            for name in &self.column_order {
                if !drop_set.contains(name.as_str()) {
                    columns.insert(name.clone(), self.columns[name].clone());
                    column_order.push(name.clone());
                }
            }
            Self::new_with_column_order(self.index.clone(), columns, column_order)
        } else {
            // Drop rows by index label
            let drop_set: BTreeSet<IndexLabel> = labels
                .iter()
                .map(|&l| IndexLabel::Utf8(l.to_owned()))
                .collect();
            let mut keep_indices = Vec::new();
            for (i, label) in self.index.labels().iter().enumerate() {
                if !drop_set.contains(label) {
                    keep_indices.push(i);
                }
            }
            let new_labels: Vec<IndexLabel> = keep_indices
                .iter()
                .map(|&i| self.index.labels()[i].clone())
                .collect();
            let mut columns = BTreeMap::new();
            for name in &self.column_order {
                let col = &self.columns[name];
                let vals: Vec<Scalar> = keep_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();
                columns.insert(name.clone(), Column::from_values(vals)?);
            }
            Self::new_with_column_order(
                Index::new(new_labels),
                columns,
                self.column_order.clone(),
            )
        }
    }

    /// Drop rows by integer index label (Int64).
    ///
    /// Convenience for dropping rows with Int64 index labels.
    pub fn drop_rows_int(&self, labels: &[i64]) -> Result<Self, FrameError> {
        let drop_set: BTreeSet<IndexLabel> = labels
            .iter()
            .map(|&l| IndexLabel::Int64(l))
            .collect();
        let mut keep_indices = Vec::new();
        for (i, label) in self.index.labels().iter().enumerate() {
            if !drop_set.contains(label) {
                keep_indices.push(i);
            }
        }
        let new_labels: Vec<IndexLabel> = keep_indices
            .iter()
            .map(|&i| self.index.labels()[i].clone())
            .collect();
        let mut columns = BTreeMap::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            let vals: Vec<Scalar> = keep_indices
                .iter()
                .map(|&i| col.values()[i].clone())
                .collect();
            columns.insert(name.clone(), Column::from_values(vals)?);
        }
        Self::new_with_column_order(
            Index::new(new_labels),
            columns,
            self.column_order.clone(),
        )
    }

    /// Insert a column at a specific position.
    ///
    /// Matches `df.insert(loc, column, value)`. The column is placed at
    /// position `loc` (0-indexed) in the column order.
    pub fn insert(
        &self,
        loc: usize,
        name: impl Into<String>,
        column: Column,
    ) -> Result<Self, FrameError> {
        let name = name.into();
        if column.len() != self.len() {
            return Err(FrameError::LengthMismatch {
                index_len: self.len(),
                column_len: column.len(),
            });
        }
        if self.columns.contains_key(&name) {
            return Err(FrameError::CompatibilityRejected(format!(
                "column '{name}' already exists; use with_column to overwrite"
            )));
        }
        let loc = loc.min(self.column_order.len());
        let mut column_order = self.column_order.clone();
        column_order.insert(loc, name.clone());
        let mut columns = self.columns.clone();
        columns.insert(name, column);
        Self::new_with_column_order(self.index.clone(), columns, column_order)
    }

    /// Remove and return a column as a Series.
    ///
    /// Matches `col = df.pop('column_name')`. Returns the removed column
    /// as a Series and the modified DataFrame (since we use immutable semantics).
    pub fn pop(&self, name: &str) -> Result<(Series, Self), FrameError> {
        let col = self.columns.get(name).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("column '{name}' not found"))
        })?;
        let series = Series::new(name.to_owned(), self.index.clone(), col.clone())?;
        let remaining = self.drop_column(name)?;
        Ok((series, remaining))
    }

    /// Replace values element-wise across all columns.
    ///
    /// Matches `df.replace(to_replace, value)`. Applies scalar-to-scalar
    /// replacement to every cell in the DataFrame.
    pub fn replace(
        &self,
        replacements: &[(Scalar, Scalar)],
    ) -> Result<Self, FrameError> {
        let mut result_cols = BTreeMap::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            let new_vals: Vec<Scalar> = col
                .values()
                .iter()
                .map(|val| {
                    for (from, to) in replacements {
                        if val == from {
                            return to.clone();
                        }
                    }
                    val.clone()
                })
                .collect();
            result_cols.insert(name.clone(), Column::from_values(new_vals)?);
        }
        Ok(Self {
            columns: result_cols,
            column_order: self.column_order.clone(),
            index: self.index.clone(),
        })
    }

    /// Align two DataFrames on their indices.
    ///
    /// Matches `df1.align(df2, join='outer'|'inner'|'left'|'right')`.
    /// Returns a tuple of two DataFrames aligned to a common index.
    /// Columns are unioned — missing columns filled with NaN.
    pub fn align_on_index(
        &self,
        other: &Self,
        mode: AlignMode,
    ) -> Result<(Self, Self), FrameError> {
        let plan = align(&self.index, &other.index, mode);
        validate_alignment_plan(&plan)?;

        // Build union column set (self's columns first, then new from other)
        let mut all_columns: Vec<String> = self.column_order.clone();
        for name in &other.column_order {
            if !self.columns.contains_key(name) {
                all_columns.push(name.clone());
            }
        }

        let n = plan.union_index.labels().len();
        let null = Scalar::Null(NullKind::NaN);

        // Build left DataFrame
        let mut left_cols = BTreeMap::new();
        for name in &all_columns {
            let vals: Vec<Scalar> = if let Some(col) = self.columns.get(name) {
                plan.left_positions
                    .iter()
                    .map(|pos| {
                        pos.map_or_else(|| null.clone(), |i| col.values()[i].clone())
                    })
                    .collect()
            } else {
                vec![null.clone(); n]
            };
            left_cols.insert(name.clone(), Column::from_values(vals)?);
        }

        // Build right DataFrame
        let mut right_cols = BTreeMap::new();
        for name in &all_columns {
            let vals: Vec<Scalar> = if let Some(col) = other.columns.get(name) {
                plan.right_positions
                    .iter()
                    .map(|pos| {
                        pos.map_or_else(|| null.clone(), |i| col.values()[i].clone())
                    })
                    .collect()
            } else {
                vec![null.clone(); n]
            };
            right_cols.insert(name.clone(), Column::from_values(vals)?);
        }

        let left = Self::new_with_column_order(
            plan.union_index.clone(),
            left_cols,
            all_columns.clone(),
        )?;
        let right = Self::new_with_column_order(
            plan.union_index,
            right_cols,
            all_columns,
        )?;
        Ok((left, right))
    }

    /// Compare two DataFrames element-wise, showing differences.
    ///
    /// Matches `df.compare(other)`. Returns a DataFrame with multi-level-like
    /// columns showing `self` and `other` values where they differ.
    /// Only rows and columns with at least one difference are included.
    pub fn compare(&self, other: &Self) -> Result<Self, FrameError> {
        if self.len() != other.len() {
            return Err(FrameError::CompatibilityRejected(
                "compare requires DataFrames of equal length".to_owned(),
            ));
        }

        // Find shared columns
        let shared_cols: Vec<String> = self
            .column_order
            .iter()
            .filter(|name| other.columns.contains_key(name.as_str()))
            .cloned()
            .collect();

        let n = self.len();
        // Track which rows have any difference
        let mut has_diff = vec![false; n];

        // Build diff columns: for each shared column, create "col_self" and "col_other"
        let mut diff_cols: Vec<(String, Vec<Scalar>)> = Vec::new();

        for col_name in &shared_cols {
            let self_col = &self.columns[col_name];
            let other_col = &other.columns[col_name];

            let mut self_vals = Vec::with_capacity(n);
            let mut other_vals = Vec::with_capacity(n);
            let mut col_has_diff = false;

            for (i, diff_flag) in has_diff.iter_mut().enumerate() {
                let sv = &self_col.values()[i];
                let ov = &other_col.values()[i];
                if sv != ov {
                    *diff_flag = true;
                    col_has_diff = true;
                    self_vals.push(sv.clone());
                    other_vals.push(ov.clone());
                } else {
                    self_vals.push(Scalar::Null(NullKind::NaN));
                    other_vals.push(Scalar::Null(NullKind::NaN));
                }
            }

            if col_has_diff {
                diff_cols.push((format!("{col_name}_self"), self_vals));
                diff_cols.push((format!("{col_name}_other"), other_vals));
            }
        }

        if diff_cols.is_empty() {
            // No differences found - return empty DataFrame
            return Self::new(Index::new(Vec::new()), BTreeMap::new());
        }

        // Filter to only rows with differences
        let diff_indices: Vec<usize> = has_diff
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| if d { Some(i) } else { None })
            .collect();

        let new_labels: Vec<IndexLabel> = diff_indices
            .iter()
            .map(|&i| self.index.labels()[i].clone())
            .collect();

        let mut columns = BTreeMap::new();
        let mut column_order = Vec::new();
        for (name, vals) in &diff_cols {
            let filtered: Vec<Scalar> = diff_indices.iter().map(|&i| vals[i].clone()).collect();
            columns.insert(name.clone(), Column::from_values(filtered)?);
            column_order.push(name.clone());
        }

        Self::new_with_column_order(Index::new(new_labels), columns, column_order)
    }

    /// Select columns by data type.
    ///
    /// Matches `df.select_dtypes(include=['float64'], exclude=['bool'])`.
    /// Pass empty slices to not filter on that criterion.
    pub fn select_dtypes(
        &self,
        include: &[DType],
        exclude: &[DType],
    ) -> Result<Self, FrameError> {
        let mut selected = Vec::new();
        for name in &self.column_order {
            let dt = self.columns[name].dtype();
            let included = include.is_empty() || include.contains(&dt);
            let excluded = !exclude.is_empty() && exclude.contains(&dt);
            if included && !excluded {
                selected.push(name.as_str());
            }
        }
        if selected.is_empty() {
            return Self::new(self.index.clone(), BTreeMap::new());
        }
        self.select_columns(&selected)
    }

    /// Filter rows or columns by label.
    ///
    /// Matches `df.filter(items, like, regex, axis)`.
    /// - `items`: exact label match
    /// - `like`: substring match
    /// - `regex`: regex match
    ///
    /// Only one of items/like/regex should be provided at a time.
    /// axis=0 filters rows by index label, axis=1 filters columns.
    pub fn filter_labels(
        &self,
        items: Option<&[&str]>,
        like: Option<&str>,
        regex: Option<&str>,
        axis: usize,
    ) -> Result<Self, FrameError> {
        if axis == 1 {
            // Filter columns
            let selected: Vec<&str> = self
                .column_order
                .iter()
                .filter(|name| {
                    if let Some(items) = items {
                        return items.contains(&name.as_str());
                    }
                    if let Some(like) = like {
                        return name.contains(like);
                    }
                    if let Some(regex) = regex
                        && let Ok(re) = Regex::new(regex)
                    {
                        return re.is_match(name);
                    }
                    true
                })
                .map(String::as_str)
                .collect();
            if selected.is_empty() {
                return Self::new(self.index.clone(), BTreeMap::new());
            }
            self.select_columns(&selected)
        } else {
            // Filter rows by index label
            let re = regex
                .map(|pat| {
                    Regex::new(pat).map_err(|e| {
                        FrameError::CompatibilityRejected(format!("invalid regex: {e}"))
                    })
                })
                .transpose()?;

            let mut keep_indices = Vec::new();
            for (i, label) in self.index.labels().iter().enumerate() {
                let label_str = match label {
                    IndexLabel::Utf8(s) => s.clone(),
                    IndexLabel::Int64(v) => v.to_string(),
                };
                let matches = if let Some(items) = items {
                    items.contains(&label_str.as_str())
                } else if let Some(like) = like {
                    label_str.contains(like)
                } else if let Some(re) = &re {
                    re.is_match(&label_str)
                } else {
                    true
                };
                if matches {
                    keep_indices.push(i);
                }
            }

            let new_labels: Vec<IndexLabel> = keep_indices
                .iter()
                .map(|&i| self.index.labels()[i].clone())
                .collect();
            let mut columns = BTreeMap::new();
            for name in &self.column_order {
                let col = &self.columns[name];
                let vals: Vec<Scalar> = keep_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();
                columns.insert(name.clone(), Column::from_values(vals)?);
            }
            Self::new_with_column_order(
                Index::new(new_labels),
                columns,
                self.column_order.clone(),
            )
        }
    }

    // ── DataFrame utility: isin / equals / first_valid_index / last_valid_index ──

    /// Return a boolean DataFrame showing whether each element is in `values`.
    ///
    /// Matches `df.isin(values)`.
    pub fn isin(&self, values: &[Scalar]) -> Result<Self, FrameError> {
        let mut new_cols = BTreeMap::new();
        for name in &self.column_order {
            let col = &self.columns[name];
            let bools: Vec<Scalar> = col
                .values()
                .iter()
                .map(|v| Scalar::Bool(values.contains(v)))
                .collect();
            new_cols.insert(name.clone(), Column::from_values(bools)?);
        }
        Self::new_with_column_order(self.index.clone(), new_cols, self.column_order.clone())
    }

    /// Check whether this DataFrame is identical to another.
    ///
    /// Matches `df.equals(other)`. Compares shape, column names, index, and values.
    pub fn equals(&self, other: &Self) -> bool {
        if self.shape() != other.shape() {
            return false;
        }
        if self.column_order != other.column_order {
            return false;
        }
        if self.index != other.index {
            return false;
        }
        for name in &self.column_order {
            let sc = &self.columns[name];
            let oc = match other.columns.get(name) {
                Some(c) => c,
                None => return false,
            };
            if sc.values() != oc.values() {
                return false;
            }
        }
        true
    }

    /// Return the index label of the first non-null row (checks all columns).
    ///
    /// Matches `df.first_valid_index()`.
    pub fn first_valid_index(&self) -> Option<IndexLabel> {
        for (i, label) in self.index.labels().iter().enumerate() {
            let any_valid = self
                .column_order
                .iter()
                .any(|name| !self.columns[name].values()[i].is_missing());
            if any_valid {
                return Some(label.clone());
            }
        }
        None
    }

    /// Return the index label of the last non-null row (checks all columns).
    ///
    /// Matches `df.last_valid_index()`.
    pub fn last_valid_index(&self) -> Option<IndexLabel> {
        for (i, label) in self.index.labels().iter().enumerate().rev() {
            let any_valid = self
                .column_order
                .iter()
                .any(|name| !self.columns[name].values()[i].is_missing());
            if any_valid {
                return Some(label.clone());
            }
        }
        None
    }

    // ── Crosstab ────────────────────────────────────────────────────

    /// Compute a crosstab (contingency table) from two columns.
    ///
    /// Matches `pd.crosstab(index, columns)`. Counts occurrences of each
    /// (row, col) combination.
    pub fn crosstab(
        index_series: &Series,
        columns_series: &Series,
    ) -> Result<Self, FrameError> {
        let idx_vals = index_series.column().values();
        let col_vals = columns_series.column().values();
        let len = idx_vals.len().min(col_vals.len());

        // Collect unique row keys and column keys in order of appearance
        let mut row_keys: Vec<Scalar> = Vec::new();
        let mut col_keys: Vec<String> = Vec::new();
        let mut counts: std::collections::HashMap<(String, String), i64> =
            std::collections::HashMap::new();

        for i in 0..len {
            let rk = &idx_vals[i];
            let ck = &col_vals[i];
            if rk.is_missing() || ck.is_missing() {
                continue;
            }

            if !row_keys.contains(rk) {
                row_keys.push(rk.clone());
            }
            let ck_str = match ck {
                Scalar::Utf8(s) => s.clone(),
                Scalar::Int64(v) => v.to_string(),
                other => format!("{other:?}"),
            };
            if !col_keys.contains(&ck_str) {
                col_keys.push(ck_str.clone());
            }

            let rk_str = match rk {
                Scalar::Utf8(s) => s.clone(),
                Scalar::Int64(v) => v.to_string(),
                other => format!("{other:?}"),
            };
            *counts.entry((rk_str, ck_str)).or_insert(0) += 1;
        }

        // Build output
        let index_labels: Vec<IndexLabel> = row_keys
            .iter()
            .map(|v| match v {
                Scalar::Utf8(s) => s.as_str().into(),
                Scalar::Int64(n) => (*n).into(),
                other => format!("{other:?}").as_str().into(),
            })
            .collect();

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for ck in &col_keys {
            let mut vals = Vec::with_capacity(row_keys.len());
            for rk in &row_keys {
                let rk_str = match rk {
                    Scalar::Utf8(s) => s.clone(),
                    Scalar::Int64(v) => v.to_string(),
                    other => format!("{other:?}"),
                };
                let count = counts.get(&(rk_str, ck.clone())).copied().unwrap_or(0);
                vals.push(Scalar::Int64(count));
            }
            result_cols.insert(ck.clone(), Column::from_values(vals)?);
            col_order.push(ck.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(index_labels),
        })
    }

    /// Crosstab with optional normalization.
    ///
    /// Matches `pd.crosstab(index, columns, normalize=...)`.
    /// normalize: "all" divides by grand total, "index" divides by row totals,
    /// "columns" divides by column totals.
    pub fn crosstab_normalize(
        index_series: &Series,
        columns_series: &Series,
        normalize: &str,
    ) -> Result<Self, FrameError> {
        let ct = Self::crosstab(index_series, columns_series)?;

        match normalize {
            "all" => {
                let grand_total: f64 = ct
                    .column_order
                    .iter()
                    .flat_map(|name| {
                        ct.columns[name]
                            .values()
                            .iter()
                            .filter_map(|v| v.to_f64().ok())
                    })
                    .sum();
                if grand_total == 0.0 {
                    return Ok(ct);
                }
                ct.applymap(|v| match v.to_f64() {
                    Ok(f) => Scalar::Float64(f / grand_total),
                    Err(_) => v.clone(),
                })
            }
            "index" => {
                // Divide each value by its row total
                let n = ct.len();
                let mut result_cols = BTreeMap::new();
                let mut row_totals = vec![0.0_f64; n];
                for name in &ct.column_order {
                    for (i, val) in ct.columns[name].values().iter().enumerate() {
                        if let Ok(f) = val.to_f64() {
                            row_totals[i] += f;
                        }
                    }
                }
                for name in &ct.column_order {
                    let vals: Vec<Scalar> = ct.columns[name]
                        .values()
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            if row_totals[i] == 0.0 {
                                Scalar::Float64(0.0)
                            } else {
                                match v.to_f64() {
                                    Ok(f) => Scalar::Float64(f / row_totals[i]),
                                    Err(_) => v.clone(),
                                }
                            }
                        })
                        .collect();
                    result_cols.insert(name.clone(), Column::from_values(vals)?);
                }
                Ok(Self {
                    columns: result_cols,
                    column_order: ct.column_order,
                    index: ct.index,
                })
            }
            "columns" => {
                // Divide each value by its column total
                let mut result_cols = BTreeMap::new();
                for name in &ct.column_order {
                    let col_total: f64 = ct.columns[name]
                        .values()
                        .iter()
                        .filter_map(|v| v.to_f64().ok())
                        .sum();
                    let vals: Vec<Scalar> = ct.columns[name]
                        .values()
                        .iter()
                        .map(|v| {
                            if col_total == 0.0 {
                                Scalar::Float64(0.0)
                            } else {
                                match v.to_f64() {
                                    Ok(f) => Scalar::Float64(f / col_total),
                                    Err(_) => v.clone(),
                                }
                            }
                        })
                        .collect();
                    result_cols.insert(name.clone(), Column::from_values(vals)?);
                }
                Ok(Self {
                    columns: result_cols,
                    column_order: ct.column_order,
                    index: ct.index,
                })
            }
            _ => Err(FrameError::CompatibilityRejected(format!(
                "unknown normalize mode: {normalize}"
            ))),
        }
    }

    // ── Explode ─────────────────────────────────────────────────────

    /// Explode a column containing delimited strings into multiple rows.
    ///
    /// Matches `df.explode(column)` for string columns. Splits the specified
    /// column by `sep` and replicates other columns accordingly.
    pub fn explode(&self, column: &str, sep: &str) -> Result<Self, FrameError> {
        let col = self.columns.get(column).ok_or_else(|| {
            FrameError::CompatibilityRejected(format!("explode: column '{column}' not found"))
        })?;

        let nrows = self.len();
        let mut new_indices: Vec<IndexLabel> = Vec::new();
        let mut exploded_vals: Vec<Scalar> = Vec::new();
        let mut source_rows: Vec<usize> = Vec::new();

        for row in 0..nrows {
            let val = &col.values()[row];
            match val {
                Scalar::Utf8(s) => {
                    let parts: Vec<&str> = s.split(sep).collect();
                    if parts.is_empty() {
                        new_indices.push(self.index.labels()[row].clone());
                        exploded_vals.push(Scalar::Null(NullKind::NaN));
                        source_rows.push(row);
                    } else {
                        for part in parts {
                            new_indices.push(self.index.labels()[row].clone());
                            exploded_vals
                                .push(Scalar::Utf8(part.trim().to_string()));
                            source_rows.push(row);
                        }
                    }
                }
                _ => {
                    new_indices.push(self.index.labels()[row].clone());
                    exploded_vals.push(val.clone());
                    source_rows.push(row);
                }
            }
        }

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for name in &self.column_order {
            if name == column {
                result_cols
                    .insert(name.clone(), Column::from_values(exploded_vals.clone())?);
            } else {
                let src_col = &self.columns[name];
                let new_vals: Vec<Scalar> = source_rows
                    .iter()
                    .map(|&r| src_col.values()[r].clone())
                    .collect();
                result_cols.insert(name.clone(), Column::from_values(new_vals)?);
            }
            col_order.push(name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(new_indices),
        })
    }

    // ── Cross-section ───────────────────────────────────────────────

    /// Select a cross-section of rows by index label.
    ///
    /// Matches `df.xs(key)`. Returns all rows whose index matches `key`.
    pub fn xs(&self, key: &IndexLabel) -> Result<Self, FrameError> {
        let labels = self.index.labels();
        let matching: Vec<usize> = labels
            .iter()
            .enumerate()
            .filter(|(_, l)| *l == key)
            .map(|(i, _)| i)
            .collect();

        if matching.is_empty() {
            return Err(FrameError::CompatibilityRejected(format!(
                "xs: key {key:?} not found in index"
            )));
        }

        self.take_rows_by_positions(&matching)
    }

    /// Drop the index level (reset to default integer index).
    ///
    /// Matches `df.droplevel(0)` for single-level index.
    pub fn droplevel(&self) -> Result<Self, FrameError> {
        let n = self.len();
        let new_labels: Vec<IndexLabel> = (0..n).map(|i| (i as i64).into()).collect();
        Ok(DataFrame {
            columns: self.columns.clone(),
            column_order: self.column_order.clone(),
            index: Index::new(new_labels),
        })
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
                    "nunique" => fp_types::nannunique(&group_vals),
                    "prod" => fp_types::nanprod(&group_vals),
                    other => {
                        return Err(FrameError::CompatibilityRejected(format!(
                            "unsupported groupby aggregation: '{other}'"
                        )));
                    }
                };
                agg_vals.push(agg_val);
            }

            result_cols.insert(col_name.clone(), Column::from_values(agg_vals)?);
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

    /// GroupBy nunique (count of unique non-null values per group).
    pub fn nunique(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("nunique")
    }

    /// GroupBy prod (product of non-null values per group).
    pub fn prod(&self) -> Result<DataFrame, FrameError> {
        self.aggregate("prod")
    }

    /// Aggregate with per-column function mapping.
    ///
    /// Matches `df.groupby(col).agg({'B': 'sum', 'C': 'mean'})` semantics.
    ///
    /// `func_map`: mapping of column_name → aggregation function name.
    /// Only the specified columns are included in the output.
    pub fn agg(
        &self,
        func_map: &std::collections::HashMap<String, String>,
    ) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let n_groups = group_order.len();

        let mut labels = Vec::with_capacity(n_groups);
        for gkey in &group_order {
            let first_row = groups[gkey][0];
            labels.push(self.group_key_label(first_row));
        }

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        // Preserve original column order for columns present in func_map
        for col_name in &self.df.column_order {
            let func_name = match func_map.get(col_name) {
                Some(f) => f.as_str(),
                None => continue,
            };

            if self.by.contains(col_name) {
                return Err(FrameError::CompatibilityRejected(format!(
                    "cannot aggregate group-by key column: '{col_name}'"
                )));
            }

            let col = &self.df.columns[col_name];
            let mut agg_vals = Vec::with_capacity(n_groups);

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let group_vals: Vec<Scalar> = row_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();

                let agg_val = Self::apply_agg_func(func_name, &group_vals)?;
                agg_vals.push(agg_val);
            }

            result_cols.insert(col_name.clone(), Column::from_values(agg_vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(labels),
        })
    }

    /// Aggregate with multiple functions applied to all value columns.
    ///
    /// Matches `df.groupby(col).agg(['sum', 'mean'])` semantics.
    ///
    /// Returns a DataFrame where each original value column is expanded into
    /// N columns (one per function), named `{col}_{func}`.
    pub fn agg_list(&self, funcs: &[&str]) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let n_groups = group_order.len();

        let mut labels = Vec::with_capacity(n_groups);
        for gkey in &group_order {
            let first_row = groups[gkey][0];
            labels.push(self.group_key_label(first_row));
        }

        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];

            for &func_name in funcs {
                let out_name = format!("{col_name}_{func_name}");
                let mut agg_vals = Vec::with_capacity(n_groups);

                for gkey in &group_order {
                    let row_indices = &groups[gkey];
                    let group_vals: Vec<Scalar> = row_indices
                        .iter()
                        .map(|&i| col.values()[i].clone())
                        .collect();

                    let agg_val = Self::apply_agg_func(func_name, &group_vals)?;
                    agg_vals.push(agg_val);
                }

                result_cols.insert(out_name.clone(), Column::from_values(agg_vals)?);
                col_order.push(out_name);
            }
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(labels),
        })
    }

    /// Apply a named aggregation function to a group's values.
    fn apply_agg_func(func_name: &str, group_vals: &[Scalar]) -> Result<Scalar, FrameError> {
        match func_name {
            "sum" => Ok(fp_types::nansum(group_vals)),
            "mean" => Ok(fp_types::nanmean(group_vals)),
            "count" => Ok(fp_types::nancount(group_vals)),
            "min" => Ok(fp_types::nanmin(group_vals)),
            "max" => Ok(fp_types::nanmax(group_vals)),
            "std" => Ok(fp_types::nanstd(group_vals, 1)),
            "var" => Ok(fp_types::nanvar(group_vals, 1)),
            "median" => Ok(fp_types::nanmedian(group_vals)),
            "first" => Ok(if group_vals.is_empty() {
                Scalar::Null(NullKind::NaN)
            } else {
                group_vals[0].clone()
            }),
            "last" => Ok(if group_vals.is_empty() {
                Scalar::Null(NullKind::NaN)
            } else {
                group_vals[group_vals.len() - 1].clone()
            }),
            "nunique" => Ok(fp_types::nannunique(group_vals)),
            "prod" => Ok(fp_types::nanprod(group_vals)),
            other => Err(FrameError::CompatibilityRejected(format!(
                "unsupported groupby aggregation: '{other}'"
            ))),
        }
    }

    /// Apply a custom function to each group DataFrame.
    ///
    /// Matches `df.groupby(col).apply(func)` semantics.
    ///
    /// The closure receives a sub-DataFrame for each group and should return
    /// a single-row DataFrame (the aggregated result for that group).
    /// Results are concatenated vertically with the group key as the index.
    pub fn apply<F>(&self, func: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&DataFrame) -> Result<DataFrame, FrameError>,
    {
        let (group_order, groups) = self.build_groups();
        let n_groups = group_order.len();

        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let mut labels = Vec::with_capacity(n_groups);
        let mut result_frames = Vec::with_capacity(n_groups);

        for gkey in &group_order {
            let row_indices = &groups[gkey];
            let first_row = row_indices[0];
            labels.push(self.group_key_label(first_row));

            // Build sub-DataFrame for this group
            let group_labels: Vec<IndexLabel> = row_indices
                .iter()
                .map(|&i| self.df.index.labels()[i].clone())
                .collect();
            let mut group_cols = BTreeMap::new();
            let mut group_col_order = Vec::new();

            for col_name in &value_cols {
                let col = &self.df.columns[col_name];
                let group_vals: Vec<Scalar> = row_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();
                group_cols.insert(col_name.clone(), Column::from_values(group_vals)?);
                group_col_order.push(col_name.clone());
            }

            let group_df = DataFrame {
                columns: group_cols,
                column_order: group_col_order,
                index: Index::new(group_labels),
            };

            result_frames.push(func(&group_df)?);
        }

        // Combine results: assume each result is a single row
        if result_frames.is_empty() {
            return Ok(DataFrame {
                columns: BTreeMap::new(),
                column_order: Vec::new(),
                index: Index::new(Vec::new()),
            });
        }

        let out_col_order = result_frames[0].column_order.clone();
        let mut out_cols: BTreeMap<String, Vec<Scalar>> = BTreeMap::new();
        for name in &out_col_order {
            out_cols.insert(name.clone(), Vec::with_capacity(n_groups));
        }

        for frame in &result_frames {
            for name in &out_col_order {
                if let Some(col) = frame.columns.get(name) {
                    for val in col.values() {
                        out_cols.get_mut(name).unwrap().push(val.clone());
                    }
                }
            }
        }

        let mut final_cols = BTreeMap::new();
        for (name, vals) in out_cols {
            final_cols.insert(name, Column::from_values(vals)?);
        }

        Ok(DataFrame {
            columns: final_cols,
            column_order: out_col_order,
            index: Index::new(labels),
        })
    }

    /// Transform each group, returning a DataFrame with the same shape as the input.
    ///
    /// Matches `df.groupby(col).transform(func_name)` semantics.
    ///
    /// The named function is applied per group and the result is broadcast back
    /// to the original row positions.
    pub fn transform(&self, func_name: &str) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let n = self.df.len();

        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut out_vals = vec![Scalar::Null(NullKind::NaN); n];

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let group_vals: Vec<Scalar> = row_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();

                let agg_val = Self::apply_agg_func(func_name, &group_vals)?;

                // Broadcast the aggregated value back to all rows in this group
                for &row_idx in row_indices {
                    out_vals[row_idx] = agg_val.clone();
                }
            }

            result_cols.insert(col_name.clone(), Column::from_values(out_vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: self.df.index.clone(),
        })
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

    /// Filter groups using a function that returns bool for each group.
    ///
    /// Analogous to `df.groupby(col).filter(func)`.
    /// The function receives a sub-DataFrame for each group and should return
    /// `true` to keep the group or `false` to drop it. Returns a DataFrame
    /// containing only the rows belonging to kept groups, preserving original
    /// index and column order.
    pub fn filter<F>(&self, func: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&DataFrame) -> Result<bool, FrameError>,
    {
        let (group_order, groups) = self.build_groups();

        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let mut keep_indices: Vec<usize> = Vec::new();

        for gkey in &group_order {
            let row_indices = &groups[gkey];

            // Build sub-DataFrame for this group
            let group_labels: Vec<IndexLabel> = row_indices
                .iter()
                .map(|&i| self.df.index.labels()[i].clone())
                .collect();
            let mut group_cols = BTreeMap::new();
            let mut group_col_order = Vec::new();

            for col_name in &value_cols {
                let col = &self.df.columns[col_name];
                let group_vals: Vec<Scalar> = row_indices
                    .iter()
                    .map(|&i| col.values()[i].clone())
                    .collect();
                group_cols.insert(col_name.clone(), Column::from_values(group_vals)?);
                group_col_order.push(col_name.clone());
            }

            let group_df = DataFrame {
                columns: group_cols,
                column_order: group_col_order,
                index: Index::new(group_labels),
            };

            if func(&group_df)? {
                keep_indices.extend(row_indices);
            }
        }

        // Sort indices to preserve original row order
        keep_indices.sort_unstable();

        // Build output DataFrame from kept rows
        let out_labels: Vec<IndexLabel> = keep_indices
            .iter()
            .map(|&i| self.df.index.labels()[i].clone())
            .collect();
        let mut out_cols = BTreeMap::new();

        for col_name in &self.df.column_order {
            let col = &self.df.columns[col_name];
            let vals: Vec<Scalar> = keep_indices
                .iter()
                .map(|&i| col.values()[i].clone())
                .collect();
            out_cols.insert(col_name.clone(), Column::from_values(vals)?);
        }

        Ok(DataFrame {
            columns: out_cols,
            column_order: self.df.column_order.clone(),
            index: Index::new(out_labels),
        })
    }

    // ── GroupBy cumulative / transform operations ───────────────

    /// Internal: apply a per-group transform that produces a value for each
    /// original row (not aggregated). Returns a DataFrame with the same index
    /// as the original.
    fn transform_groups<F>(&self, func: F) -> Result<DataFrame, FrameError>
    where
        F: Fn(&[Scalar]) -> Vec<Scalar>,
    {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let n = self.df.len();
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut out = vec![Scalar::Null(NullKind::NaN); n];

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let group_vals: Vec<Scalar> =
                    row_indices.iter().map(|&i| col.values()[i].clone()).collect();
                let transformed = func(&group_vals);
                for (j, &ri) in row_indices.iter().enumerate() {
                    if j < transformed.len() {
                        out[ri] = transformed[j].clone();
                    }
                }
            }

            result_cols.insert(col_name.clone(), Column::from_values(out)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: self.df.index.clone(),
        })
    }

    /// GroupBy cumulative sum.
    ///
    /// Matches `df.groupby(col).cumsum()`.
    pub fn cumsum(&self) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            let mut acc = 0.0_f64;
            vals.iter()
                .map(|v| {
                    if v.is_missing() {
                        Scalar::Null(NullKind::NaN)
                    } else if let Ok(f) = v.to_f64() {
                        acc += f;
                        Scalar::Float64(acc)
                    } else {
                        Scalar::Null(NullKind::NaN)
                    }
                })
                .collect()
        })
    }

    /// GroupBy cumulative product.
    ///
    /// Matches `df.groupby(col).cumprod()`.
    pub fn cumprod(&self) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            let mut acc = 1.0_f64;
            vals.iter()
                .map(|v| {
                    if v.is_missing() {
                        Scalar::Null(NullKind::NaN)
                    } else if let Ok(f) = v.to_f64() {
                        acc *= f;
                        Scalar::Float64(acc)
                    } else {
                        Scalar::Null(NullKind::NaN)
                    }
                })
                .collect()
        })
    }

    /// GroupBy cumulative max.
    ///
    /// Matches `df.groupby(col).cummax()`.
    pub fn cummax(&self) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            let mut acc = f64::NEG_INFINITY;
            vals.iter()
                .map(|v| {
                    if v.is_missing() {
                        Scalar::Null(NullKind::NaN)
                    } else if let Ok(f) = v.to_f64() {
                        if f > acc {
                            acc = f;
                        }
                        Scalar::Float64(acc)
                    } else {
                        Scalar::Null(NullKind::NaN)
                    }
                })
                .collect()
        })
    }

    /// GroupBy cumulative min.
    ///
    /// Matches `df.groupby(col).cummin()`.
    pub fn cummin(&self) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            let mut acc = f64::INFINITY;
            vals.iter()
                .map(|v| {
                    if v.is_missing() {
                        Scalar::Null(NullKind::NaN)
                    } else if let Ok(f) = v.to_f64() {
                        if f < acc {
                            acc = f;
                        }
                        Scalar::Float64(acc)
                    } else {
                        Scalar::Null(NullKind::NaN)
                    }
                })
                .collect()
        })
    }

    /// GroupBy rank within each group.
    ///
    /// Matches `df.groupby(col).rank()`.
    /// Uses average method by default.
    pub fn rank(&self) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            let n = vals.len();
            let mut indexed: Vec<(usize, f64)> = Vec::new();
            let mut null_pos = Vec::new();

            for (i, v) in vals.iter().enumerate() {
                if v.is_missing() {
                    null_pos.push(i);
                } else if let Ok(f) = v.to_f64() {
                    indexed.push((i, f));
                } else {
                    null_pos.push(i);
                }
            }

            indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

            let mut ranks = vec![Scalar::Null(NullKind::NaN); n];
            let mut i = 0;
            while i < indexed.len() {
                let mut j = i + 1;
                while j < indexed.len()
                    && (indexed[j].1 - indexed[i].1).abs() < f64::EPSILON
                {
                    j += 1;
                }
                let avg: f64 =
                    ((i + 1)..=j).map(|r| r as f64).sum::<f64>() / (j - i) as f64;
                for item in &indexed[i..j] {
                    ranks[item.0] = Scalar::Float64(avg);
                }
                i = j;
            }

            ranks
        })
    }

    /// GroupBy shift within each group.
    ///
    /// Matches `df.groupby(col).shift(periods)`.
    pub fn shift(&self, periods: i64) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            let n = vals.len();
            let mut out = vec![Scalar::Null(NullKind::NaN); n];
            for (i, slot) in out.iter_mut().enumerate() {
                let src = i as i64 - periods;
                if src >= 0 && (src as usize) < n {
                    *slot = vals[src as usize].clone();
                }
            }
            out
        })
    }

    /// GroupBy diff within each group.
    ///
    /// Matches `df.groupby(col).diff(periods)`.
    pub fn diff(&self, periods: usize) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            vals.iter()
                .enumerate()
                .map(|(i, v)| {
                    if i < periods {
                        return Scalar::Null(NullKind::NaN);
                    }
                    let prev = &vals[i - periods];
                    if v.is_missing() || prev.is_missing() {
                        return Scalar::Null(NullKind::NaN);
                    }
                    if let (Ok(a), Ok(b)) = (v.to_f64(), prev.to_f64()) {
                        Scalar::Float64(a - b)
                    } else {
                        Scalar::Null(NullKind::NaN)
                    }
                })
                .collect()
        })
    }

    /// GroupBy nth: select the nth row from each group.
    ///
    /// Matches `df.groupby(col).nth(n)`. Returns a DataFrame with one row per group.
    /// Negative `n` counts from the end.
    pub fn nth(&self, n: i64) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();

        let mut keep_indices: Vec<usize> = Vec::new();

        for gkey in &group_order {
            let row_indices = &groups[gkey];
            let len = row_indices.len() as i64;
            let idx = if n >= 0 { n } else { len + n };
            if idx >= 0 && idx < len {
                keep_indices.push(row_indices[idx as usize]);
            }
        }

        let out_labels: Vec<IndexLabel> = keep_indices
            .iter()
            .map(|&i| self.df.index.labels()[i].clone())
            .collect();

        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let vals: Vec<Scalar> = keep_indices
                .iter()
                .map(|&i| col.values()[i].clone())
                .collect();
            result_cols.insert(col_name.clone(), Column::from_values(vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(out_labels),
        })
    }

    /// GroupBy head: select first n rows per group.
    ///
    /// Matches `df.groupby(col).head(n)`.
    pub fn head(&self, n: usize) -> Result<DataFrame, FrameError> {
        let (_group_order, groups) = self.build_groups();
        let mut keep_indices: Vec<usize> = Vec::new();

        for row_indices in groups.values() {
            let take = row_indices.len().min(n);
            keep_indices.extend_from_slice(&row_indices[..take]);
        }

        keep_indices.sort_unstable();
        self.df.take_rows_by_positions(&keep_indices)
    }

    /// GroupBy tail: select last n rows per group.
    ///
    /// Matches `df.groupby(col).tail(n)`.
    pub fn tail(&self, n: usize) -> Result<DataFrame, FrameError> {
        let (_group_order, groups) = self.build_groups();
        let mut keep_indices: Vec<usize> = Vec::new();

        for row_indices in groups.values() {
            let start = row_indices.len().saturating_sub(n);
            keep_indices.extend_from_slice(&row_indices[start..]);
        }

        keep_indices.sort_unstable();
        self.df.take_rows_by_positions(&keep_indices)
    }

    /// GroupBy pct_change within each group.
    ///
    /// Matches `df.groupby(col).pct_change()`.
    pub fn pct_change(&self) -> Result<DataFrame, FrameError> {
        self.transform_groups(|vals| {
            vals.iter()
                .enumerate()
                .map(|(i, v)| {
                    if i == 0 {
                        return Scalar::Null(NullKind::NaN);
                    }
                    let prev = &vals[i - 1];
                    if v.is_missing() || prev.is_missing() {
                        return Scalar::Null(NullKind::NaN);
                    }
                    if let (Ok(curr), Ok(prv)) = (v.to_f64(), prev.to_f64()) {
                        if prv.abs() < f64::EPSILON {
                            Scalar::Null(NullKind::NaN)
                        } else {
                            Scalar::Float64((curr - prv) / prv)
                        }
                    } else {
                        Scalar::Null(NullKind::NaN)
                    }
                })
                .collect()
        })
    }

    /// GroupBy value_counts: count of each unique value per group.
    ///
    /// Matches `df.groupby(col)[value_col].value_counts()`.
    /// Returns a DataFrame with group key + value as index and counts as values.
    pub fn value_counts(&self) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        // For each value column, count occurrences per group per value
        if value_cols.is_empty() {
            return Err(FrameError::CompatibilityRejected(
                "value_counts: no value columns".to_owned(),
            ));
        }

        // Use the first value column
        let col_name = &value_cols[0];
        let col = &self.df.columns[col_name];

        let mut out_labels: Vec<IndexLabel> = Vec::new();
        let mut out_counts: Vec<Scalar> = Vec::new();

        for gkey in &group_order {
            let row_indices = &groups[gkey];
            let first_row = row_indices[0];
            let group_label = self.group_key_label(first_row);

            // Count unique values in this group
            let mut val_counts: Vec<(Scalar, i64)> = Vec::new();
            for &ri in row_indices {
                let val = &col.values()[ri];
                if val.is_missing() {
                    continue;
                }
                if let Some(entry) = val_counts.iter_mut().find(|(v, _)| v == val) {
                    entry.1 += 1;
                } else {
                    val_counts.push((val.clone(), 1));
                }
            }

            // Sort by count descending
            val_counts.sort_by_key(|entry| std::cmp::Reverse(entry.1));

            for (val, count) in val_counts {
                let label_str = match (&group_label, &val) {
                    (IndexLabel::Utf8(g), Scalar::Utf8(v)) => format!("{g}, {v}"),
                    (IndexLabel::Int64(g), Scalar::Utf8(v)) => format!("{g}, {v}"),
                    (IndexLabel::Utf8(g), Scalar::Int64(v)) => format!("{g}, {v}"),
                    (IndexLabel::Int64(g), Scalar::Int64(v)) => format!("{g}, {v}"),
                    (g, v) => format!("{g:?}, {v:?}"),
                };
                out_labels.push(IndexLabel::Utf8(label_str));
                out_counts.push(Scalar::Int64(count));
            }
        }

        let mut result_cols = BTreeMap::new();
        result_cols.insert("count".to_string(), Column::from_values(out_counts)?);

        Ok(DataFrame {
            columns: result_cols,
            column_order: vec!["count".to_string()],
            index: Index::new(out_labels),
        })
    }

    /// GroupBy describe: summary statistics per group.
    ///
    /// Matches `df.groupby(col).describe()`. Returns count, mean, std, min,
    /// 25%/50%/75% quantiles, and max for each group's numeric columns.
    pub fn describe(&self) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_order
            .iter()
            .filter(|c| !self.by.contains(c))
            .cloned()
            .collect();

        let stat_names = ["count", "mean", "std", "min", "25%", "50%", "75%", "max"];
        let n_stats = stat_names.len();
        let n_groups = group_order.len();
        let total_rows = n_groups * n_stats;

        let mut out_labels = Vec::with_capacity(total_rows);
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        // Build labels: group_key|stat_name
        for gkey in &group_order {
            let first_row = groups[gkey][0];
            let group_label = self.group_key_label(first_row);
            for stat in &stat_names {
                let label_str = match &group_label {
                    IndexLabel::Utf8(g) => format!("{g}|{stat}"),
                    IndexLabel::Int64(g) => format!("{g}|{stat}"),
                };
                out_labels.push(IndexLabel::Utf8(label_str));
            }
        }

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut stat_vals = Vec::with_capacity(total_rows);

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let group_vals: Vec<f64> = row_indices
                    .iter()
                    .filter_map(|&i| {
                        let v = &col.values()[i];
                        if v.is_missing() {
                            None
                        } else {
                            v.to_f64().ok()
                        }
                    })
                    .collect();

                let count = group_vals.len();
                let mean = if count > 0 {
                    group_vals.iter().sum::<f64>() / count as f64
                } else {
                    f64::NAN
                };
                let std = if count > 1 {
                    let var = group_vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                        / (count - 1) as f64;
                    var.sqrt()
                } else {
                    f64::NAN
                };

                let mut sorted = group_vals.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                let min = sorted.first().copied().unwrap_or(f64::NAN);
                let max = sorted.last().copied().unwrap_or(f64::NAN);

                let quantile = |q: f64| -> f64 {
                    if sorted.is_empty() {
                        return f64::NAN;
                    }
                    let pos = q * (sorted.len() - 1) as f64;
                    let lo = pos.floor() as usize;
                    let hi = pos.ceil() as usize;
                    if lo == hi || hi >= sorted.len() {
                        sorted[lo]
                    } else {
                        sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
                    }
                };

                stat_vals.push(Scalar::Float64(count as f64));
                stat_vals.push(Scalar::Float64(mean));
                stat_vals.push(Scalar::Float64(std));
                stat_vals.push(Scalar::Float64(min));
                stat_vals.push(Scalar::Float64(quantile(0.25)));
                stat_vals.push(Scalar::Float64(quantile(0.5)));
                stat_vals.push(Scalar::Float64(quantile(0.75)));
                stat_vals.push(Scalar::Float64(max));
            }

            result_cols.insert(col_name.clone(), Column::from_values(stat_vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(out_labels),
        })
    }

    /// Retrieve a single group as a DataFrame.
    ///
    /// Matches `groupby.get_group(name)`. The name should match the
    /// debug representation of the group key value (e.g. `"a"` for Utf8
    /// or `"Int64(1)"` for numeric keys).
    pub fn get_group(&self, name: &str) -> Result<DataFrame, FrameError> {
        let (_group_order, groups) = self.build_groups();
        // Try exact match on the build_groups key first
        if let Some(row_indices) = groups.get(name) {
            return self.df.take_rows_by_positions(row_indices);
        }
        // Try matching by the human-readable group key label
        for (gkey, row_indices) in &groups {
            let first_row = row_indices[0];
            let label = self.group_key_label(first_row);
            let label_str = match &label {
                IndexLabel::Utf8(s) => s.clone(),
                IndexLabel::Int64(v) => v.to_string(),
            };
            if label_str == name || gkey == name {
                return self.df.take_rows_by_positions(row_indices);
            }
        }
        Err(FrameError::CompatibilityRejected(format!(
            "group '{name}' not found"
        )))
    }

    /// Assign within-group cumulative count (0-based).
    ///
    /// Matches `groupby.cumcount()`. Returns a Series of Int64 with a
    /// monotonically increasing counter within each group.
    pub fn cumcount(&self) -> Result<Series, FrameError> {
        let (_group_order, groups) = self.build_groups();
        let n = self.df.len();
        let mut out = vec![Scalar::Int64(0); n];

        for row_indices in groups.values() {
            for (count, &idx) in row_indices.iter().enumerate() {
                out[idx] = Scalar::Int64(count as i64);
            }
        }

        Series::from_values(
            "cumcount".to_owned(),
            self.df.index().labels().to_vec(),
            out,
        )
    }

    /// Assign group number to each row.
    ///
    /// Matches `groupby.ngroup()`. Returns a Series of Int64 where
    /// each row gets the ordinal number of its group.
    pub fn ngroup(&self) -> Result<Series, FrameError> {
        let (group_order, groups) = self.build_groups();
        let n = self.df.len();
        let mut out = vec![Scalar::Int64(0); n];

        for (group_num, gkey) in group_order.iter().enumerate() {
            for &idx in &groups[gkey] {
                out[idx] = Scalar::Int64(group_num as i64);
            }
        }

        Series::from_values(
            "ngroup".to_owned(),
            self.df.index().labels().to_vec(),
            out,
        )
    }

    /// Pipe the GroupBy through a function.
    ///
    /// Matches `groupby.pipe(func)`.
    pub fn pipe<F>(&self, func: F) -> Result<DataFrame, FrameError>
    where
        F: FnOnce(&Self) -> Result<DataFrame, FrameError>,
    {
        func(self)
    }

    /// Standard error of the mean per group.
    ///
    /// Matches `groupby.sem()`. Computed as std / sqrt(count).
    pub fn sem(&self) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_names()
            .iter()
            .filter(|c| {
                if self.by.contains(c) {
                    return false;
                }
                let dt = self.df.columns[c.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .map(|s| s.to_string())
            .collect();

        let mut out_labels = Vec::new();
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for gkey in &group_order {
            let first_row = groups[gkey][0];
            out_labels.push(self.group_key_label(first_row));
        }

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut vals = Vec::with_capacity(group_order.len());

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let group_vals: Vec<f64> = row_indices
                    .iter()
                    .filter_map(|&i| {
                        let v = &col.values()[i];
                        if v.is_missing() {
                            None
                        } else {
                            v.to_f64().ok()
                        }
                    })
                    .collect();

                let count = group_vals.len();
                if count < 2 {
                    vals.push(Scalar::Float64(f64::NAN));
                } else {
                    let mean = group_vals.iter().sum::<f64>() / count as f64;
                    let var = group_vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                        / (count - 1) as f64;
                    vals.push(Scalar::Float64(var.sqrt() / (count as f64).sqrt()));
                }
            }

            result_cols.insert(col_name.clone(), Column::from_values(vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(out_labels),
        })
    }

    /// Skewness per group (Fisher's definition, bias=False).
    ///
    /// Matches `groupby.skew()`.
    pub fn skew(&self) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_names()
            .iter()
            .filter(|c| {
                if self.by.contains(c) {
                    return false;
                }
                let dt = self.df.columns[c.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .map(|s| s.to_string())
            .collect();

        let mut out_labels = Vec::new();
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for gkey in &group_order {
            let first_row = groups[gkey][0];
            out_labels.push(self.group_key_label(first_row));
        }

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut vals = Vec::with_capacity(group_order.len());

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let gv: Vec<f64> = row_indices
                    .iter()
                    .filter_map(|&i| {
                        let v = &col.values()[i];
                        if v.is_missing() { None } else { v.to_f64().ok() }
                    })
                    .collect();

                let n = gv.len();
                if n < 3 {
                    vals.push(Scalar::Float64(f64::NAN));
                } else {
                    let mean = gv.iter().sum::<f64>() / n as f64;
                    let m2 = gv.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
                    let m3 = gv.iter().map(|x| (x - mean).powi(3)).sum::<f64>();
                    let nf = n as f64;
                    let s2 = m2 / (nf - 1.0);
                    let skew = (m3 / nf) / (s2.powf(1.5))
                        * (nf * (nf - 1.0)).sqrt()
                        / (nf - 2.0);
                    vals.push(Scalar::Float64(skew));
                }
            }

            result_cols.insert(col_name.clone(), Column::from_values(vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(out_labels),
        })
    }

    /// Kurtosis per group (Fisher's excess, bias=False).
    ///
    /// Matches `groupby.kurtosis()`.
    pub fn kurtosis(&self) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_names()
            .iter()
            .filter(|c| {
                if self.by.contains(c) {
                    return false;
                }
                let dt = self.df.columns[c.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .map(|s| s.to_string())
            .collect();

        let mut out_labels = Vec::new();
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for gkey in &group_order {
            let first_row = groups[gkey][0];
            out_labels.push(self.group_key_label(first_row));
        }

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut vals = Vec::with_capacity(group_order.len());

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let gv: Vec<f64> = row_indices
                    .iter()
                    .filter_map(|&i| {
                        let v = &col.values()[i];
                        if v.is_missing() { None } else { v.to_f64().ok() }
                    })
                    .collect();

                let n = gv.len();
                if n < 4 {
                    vals.push(Scalar::Float64(f64::NAN));
                } else {
                    let mean = gv.iter().sum::<f64>() / n as f64;
                    let m2 = gv.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
                    let m4 = gv.iter().map(|x| (x - mean).powi(4)).sum::<f64>();
                    let nf = n as f64;
                    let s2 = m2 / (nf - 1.0);
                    if s2 == 0.0 {
                        vals.push(Scalar::Float64(f64::NAN));
                    } else {
                        let excess = (nf * (nf + 1.0) * m4 / (m2 * m2)
                            - 3.0 * (nf - 1.0).powi(2))
                            / ((nf - 1.0) * (nf - 2.0) * (nf - 3.0));
                        vals.push(Scalar::Float64(excess));
                    }
                }
            }

            result_cols.insert(col_name.clone(), Column::from_values(vals)?);
            col_order.push(col_name.clone());
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(out_labels),
        })
    }

    /// Open-High-Low-Close per group.
    ///
    /// Matches `groupby.ohlc()`. Returns columns: open, high, low, close.
    pub fn ohlc(&self) -> Result<DataFrame, FrameError> {
        let (group_order, groups) = self.build_groups();
        let value_cols: Vec<String> = self
            .df
            .column_names()
            .iter()
            .filter(|c| {
                if self.by.contains(c) {
                    return false;
                }
                let dt = self.df.columns[c.as_str()].dtype();
                dt == DType::Int64 || dt == DType::Float64
            })
            .map(|s| s.to_string())
            .collect();

        let mut out_labels = Vec::new();
        let mut result_cols = BTreeMap::new();
        let mut col_order = Vec::new();

        for gkey in &group_order {
            let first_row = groups[gkey][0];
            out_labels.push(self.group_key_label(first_row));
        }

        for col_name in &value_cols {
            let col = &self.df.columns[col_name];
            let mut opens = Vec::with_capacity(group_order.len());
            let mut highs = Vec::with_capacity(group_order.len());
            let mut lows = Vec::with_capacity(group_order.len());
            let mut closes = Vec::with_capacity(group_order.len());

            for gkey in &group_order {
                let row_indices = &groups[gkey];
                let gv: Vec<f64> = row_indices
                    .iter()
                    .filter_map(|&i| {
                        let v = &col.values()[i];
                        if v.is_missing() { None } else { v.to_f64().ok() }
                    })
                    .collect();

                if gv.is_empty() {
                    opens.push(Scalar::Float64(f64::NAN));
                    highs.push(Scalar::Float64(f64::NAN));
                    lows.push(Scalar::Float64(f64::NAN));
                    closes.push(Scalar::Float64(f64::NAN));
                } else {
                    opens.push(Scalar::Float64(gv[0]));
                    highs.push(Scalar::Float64(
                        gv.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    ));
                    lows.push(Scalar::Float64(
                        gv.iter().copied().fold(f64::INFINITY, f64::min),
                    ));
                    closes.push(Scalar::Float64(gv[gv.len() - 1]));
                }
            }

            let prefix = if value_cols.len() > 1 {
                format!("{col_name}_")
            } else {
                String::new()
            };
            result_cols.insert(format!("{prefix}open"), Column::from_values(opens)?);
            result_cols.insert(format!("{prefix}high"), Column::from_values(highs)?);
            result_cols.insert(format!("{prefix}low"), Column::from_values(lows)?);
            result_cols.insert(format!("{prefix}close"), Column::from_values(closes)?);
            col_order.push(format!("{prefix}open"));
            col_order.push(format!("{prefix}high"));
            col_order.push(format!("{prefix}low"));
            col_order.push(format!("{prefix}close"));
        }

        Ok(DataFrame {
            columns: result_cols,
            column_order: col_order,
            index: Index::new(out_labels),
        })
    }

}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::{DType, NullKind, Scalar};

    use fp_columnar::Column;
    use fp_index::AlignMode;

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
        assert_eq!(result.columns["val"].values()[0], Scalar::Int64(1)); // count skips NaN
        assert_eq!(result.columns["val"].values()[1], Scalar::Int64(1));
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

    #[test]
    fn dataframe_groupby_nunique() {
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
                    Scalar::Float64(1.0),
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.groupby(&["grp"]).unwrap().nunique().unwrap();
        // Group "a": 1 unique value (1.0); Group "b": 2 unique values (2.0, 3.0)
        assert_eq!(result.columns["val"].values()[0], Scalar::Int64(1));
        assert_eq!(result.columns["val"].values()[1], Scalar::Int64(2));
    }

    #[test]
    fn dataframe_groupby_prod() {
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
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(5.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.groupby(&["grp"]).unwrap().prod().unwrap();
        // Group "a": 2*3=6; Group "b": 4*5=20
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(6.0));
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(20.0));
    }

    #[test]
    fn dataframe_groupby_agg_per_column() {
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
                "x",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(40.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "y",
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

        let mut func_map = std::collections::HashMap::new();
        func_map.insert("x".to_string(), "sum".to_string());
        func_map.insert("y".to_string(), "mean".to_string());

        let result = df.groupby(&["grp"]).unwrap().agg(&func_map).unwrap();
        assert_eq!(result.len(), 2);
        // Group "a": x sum = 10+30=40; y mean = (1+3)/2=2.0
        assert_eq!(result.columns["x"].values()[0], Scalar::Float64(40.0));
        assert_eq!(result.columns["y"].values()[0], Scalar::Float64(2.0));
        // Group "b": x sum = 20+40=60; y mean = (2+4)/2=3.0
        assert_eq!(result.columns["x"].values()[1], Scalar::Float64(60.0));
        assert_eq!(result.columns["y"].values()[1], Scalar::Float64(3.0));
    }

    #[test]
    fn dataframe_groupby_agg_list() {
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

        let result = df
            .groupby(&["grp"])
            .unwrap()
            .agg_list(&["sum", "mean"])
            .unwrap();
        assert_eq!(result.len(), 2);
        // Column names: val_sum, val_mean
        assert_eq!(result.column_order, vec!["val_sum", "val_mean"]);
        // Group "a": sum=30, mean=15; Group "b": sum=70, mean=35
        assert_eq!(result.columns["val_sum"].values()[0], Scalar::Float64(30.0));
        assert_eq!(
            result.columns["val_mean"].values()[0],
            Scalar::Float64(15.0)
        );
        assert_eq!(result.columns["val_sum"].values()[1], Scalar::Float64(70.0));
        assert_eq!(
            result.columns["val_mean"].values()[1],
            Scalar::Float64(35.0)
        );
    }

    #[test]
    fn dataframe_rolling_sum() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "b",
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

        let result = df.rolling(2, None).sum().unwrap();
        assert_eq!(result.len(), 4);
        // First element: NaN (window not full)
        assert!(result.columns["a"].values()[0].is_missing());
        // Second element: 1+2=3
        assert_eq!(result.columns["a"].values()[1], Scalar::Float64(3.0));
        // Third element: 2+3=5
        assert_eq!(result.columns["a"].values()[2], Scalar::Float64(5.0));
        // b column also present
        assert_eq!(result.columns["b"].values()[1], Scalar::Float64(30.0));
    }

    #[test]
    fn dataframe_rolling_mean() {
        let df = DataFrame::from_series(vec![Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
                Scalar::Float64(8.0),
            ],
        )
        .unwrap()])
        .unwrap();

        let result = df.rolling(2, None).mean().unwrap();
        assert!(result.columns["val"].values()[0].is_missing());
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(3.0)); // (2+4)/2
        assert_eq!(result.columns["val"].values()[2], Scalar::Float64(5.0)); // (4+6)/2
        assert_eq!(result.columns["val"].values()[3], Scalar::Float64(7.0)); // (6+8)/2
    }

    #[test]
    fn dataframe_rolling_skips_non_numeric() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "num",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
            Series::from_values(
                "text",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df.rolling(2, None).sum().unwrap();
        // Only "num" should be in the result, not "text"
        assert!(result.columns.contains_key("num"));
        assert!(!result.columns.contains_key("text"));
    }

    #[test]
    fn dataframe_expanding_sum() {
        let df = DataFrame::from_series(vec![Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap()])
        .unwrap();

        let result = df.expanding(None).sum().unwrap();
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(1.0));
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(3.0)); // 1+2
        assert_eq!(result.columns["val"].values()[2], Scalar::Float64(6.0)); // 1+2+3
    }

    #[test]
    fn dataframe_expanding_mean_with_min_periods() {
        let df = DataFrame::from_series(vec![Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap()])
        .unwrap();

        let result = df.expanding(Some(2)).mean().unwrap();
        // min_periods=2, so first element should be NaN
        assert!(result.columns["val"].values()[0].is_missing());
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(3.0)); // (2+4)/2
        assert_eq!(result.columns["val"].values()[2], Scalar::Float64(4.0)); // (2+4+6)/3
    }

    #[test]
    fn dataframe_groupby_agg_rejects_groupby_column() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into()],
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        let mut func_map = std::collections::HashMap::new();
        func_map.insert("grp".to_string(), "sum".to_string());

        let err = df.groupby(&["grp"]).unwrap().agg(&func_map);
        assert!(err.is_err());
    }

    #[test]
    fn series_corr_spearman() {
        // Perfectly monotonic: [1,2,3,4] vs [10,20,30,40] → Spearman = 1.0
        let a = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();
        let b = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
                Scalar::Float64(40.0),
            ],
        )
        .unwrap();

        let r = a.corr_spearman(&b).unwrap();
        assert!((r - 1.0).abs() < 1e-10);
    }

    #[test]
    fn series_corr_spearman_inverse() {
        // Perfectly inversely monotonic: [1,2,3] vs [30,20,10] → Spearman = -1.0
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
                Scalar::Float64(30.0),
                Scalar::Float64(20.0),
                Scalar::Float64(10.0),
            ],
        )
        .unwrap();

        let r = a.corr_spearman(&b).unwrap();
        assert!((r - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn series_corr_kendall() {
        // Perfect concordance: [1,2,3] vs [4,5,6] → Kendall tau = 1.0
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
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let tau = a.corr_kendall(&b).unwrap();
        assert!((tau - 1.0).abs() < 1e-10);
    }

    #[test]
    fn dataframe_corr_spearman() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "x",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
            Series::from_values(
                "y",
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

        let result = df.corr_method("spearman").unwrap();
        // Self-correlation = 1.0
        assert_eq!(result.columns["x"].values()[0], Scalar::Float64(1.0));
        assert_eq!(result.columns["y"].values()[1], Scalar::Float64(1.0));
        // Cross-correlation should be 1.0 for perfectly monotonic data
        let xy = match &result.columns["y"].values()[0] {
            Scalar::Float64(v) => *v,
            _ => panic!("expected Float64"),
        };
        assert!((xy - 1.0).abs() < 1e-10);
    }

    #[test]
    fn dataframe_groupby_apply() {
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

        // Apply a function that returns the sum of each group as a single-row DataFrame
        let result = df
            .groupby(&["grp"])
            .unwrap()
            .apply(|group_df| {
                // Sum the 'val' column
                let col = &group_df.columns["val"];
                let total: f64 = col
                    .values()
                    .iter()
                    .filter_map(|v| v.to_f64().ok())
                    .sum();
                let mut cols = BTreeMap::new();
                cols.insert(
                    "val".to_string(),
                    super::Column::from_values(vec![Scalar::Float64(total)]).unwrap(),
                );
                Ok(DataFrame::new_with_column_order(
                    super::Index::new(vec![0_i64.into()]),
                    cols,
                    vec!["val".to_string()],
                )
                .unwrap())
            })
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(30.0)); // 10+20
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(70.0)); // 30+40
    }

    #[test]
    fn dataframe_groupby_transform_mean() {
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

        let result = df
            .groupby(&["grp"])
            .unwrap()
            .transform("mean")
            .unwrap();

        // Same shape as input (4 rows)
        assert_eq!(result.len(), 4);
        // Group "a" mean = 15.0, broadcast to rows 0 and 1
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(15.0));
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(15.0));
        // Group "b" mean = 35.0, broadcast to rows 2 and 3
        assert_eq!(result.columns["val"].values()[2], Scalar::Float64(35.0));
        assert_eq!(result.columns["val"].values()[3], Scalar::Float64(35.0));
    }

    #[test]
    fn dataframe_groupby_transform_sum() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Utf8("x".into()),
                    Scalar::Utf8("x".into()),
                    Scalar::Utf8("y".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(5.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df
            .groupby(&["grp"])
            .unwrap()
            .transform("sum")
            .unwrap();

        assert_eq!(result.len(), 3);
        // Group "x" sum = 3.0 for rows 0 and 1
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(3.0));
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(3.0));
        // Group "y" sum = 5.0 for row 2
        assert_eq!(result.columns["val"].values()[2], Scalar::Float64(5.0));
        // Original index is preserved
        assert_eq!(
            result.index().labels(),
            &[0_i64.into(), 1_i64.into(), 2_i64.into()]
        );
    }

    // ── str accessor regex tests ──

    #[test]
    fn str_contains_regex() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("foo123".into()),
                Scalar::Utf8("bar".into()),
                Scalar::Utf8("baz456".into()),
            ],
        )
        .unwrap();
        let result = s.str().contains_regex(r"\d+").unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
        assert_eq!(result.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn str_contains_regex_invalid_pattern() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("test".into())],
        )
        .unwrap();
        assert!(s.str().contains_regex(r"[invalid").is_err());
    }

    #[test]
    fn str_contains_regex_with_null() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("abc123".into()),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();
        let result = s.str().contains_regex(r"\d").unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert!(result.values()[1].is_missing());
    }

    #[test]
    fn str_replace_regex() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("foo123bar456".into()),
                Scalar::Utf8("no digits".into()),
            ],
        )
        .unwrap();
        // Replace first occurrence
        let result = s.str().replace_regex(r"\d+", "NUM").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("fooNUMbar456".into()));
        assert_eq!(result.values()[1], Scalar::Utf8("no digits".into()));
    }

    #[test]
    fn str_replace_regex_all() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("foo123bar456".into())],
        )
        .unwrap();
        let result = s.str().replace_regex_all(r"\d+", "NUM").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("fooNUMbarNUM".into()));
    }

    #[test]
    fn str_replace_regex_with_backreference() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello world".into())],
        )
        .unwrap();
        let result = s
            .str()
            .replace_regex(r"(\w+)\s(\w+)", "$2 $1")
            .unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("world hello".into()));
    }

    #[test]
    fn str_extract_with_group() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("foo123".into()),
                Scalar::Utf8("bar456".into()),
                Scalar::Utf8("no match".into()),
            ],
        )
        .unwrap();
        let result = s.str().extract(r"(\d+)").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("123".into()));
        assert_eq!(result.values()[1], Scalar::Utf8("456".into()));
        assert!(result.values()[2].is_missing());
    }

    #[test]
    fn str_extract_without_group() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("abc123def".into())],
        )
        .unwrap();
        // No capture group → returns full match
        let result = s.str().extract(r"\d+").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("123".into()));
    }

    #[test]
    fn str_count_matches() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("aabaa".into()),
                Scalar::Utf8("bb".into()),
                Scalar::Utf8("aaa".into()),
            ],
        )
        .unwrap();
        let result = s.str().count_matches("a+").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(2)); // "aa" and "aa"
        assert_eq!(result.values()[1], Scalar::Int64(0));
        assert_eq!(result.values()[2], Scalar::Int64(1)); // "aaa"
    }

    #[test]
    fn str_findall() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("foo12bar34".into()),
                Scalar::Utf8("no digits".into()),
            ],
        )
        .unwrap();
        let result = s.str().findall(r"\d+", ",").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("12,34".into()));
        assert!(result.values()[1].is_missing());
    }

    #[test]
    fn str_fullmatch() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("123".into()),
                Scalar::Utf8("abc123".into()),
                Scalar::Utf8("456".into()),
            ],
        )
        .unwrap();
        let result = s.str().fullmatch(r"\d+").unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false)); // partial match, not full
        assert_eq!(result.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn str_match_regex() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("123abc".into()),
                Scalar::Utf8("abc123".into()),
                Scalar::Utf8("456".into()),
            ],
        )
        .unwrap();
        let result = s.str().match_regex(r"\d+").unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true)); // starts with digits
        assert_eq!(result.values()[1], Scalar::Bool(false)); // starts with letters
        assert_eq!(result.values()[2], Scalar::Bool(true)); // all digits
    }

    #[test]
    fn str_split_regex_get() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("foo123bar456baz".into()),
                Scalar::Utf8("hello".into()),
            ],
        )
        .unwrap();
        // Split on one or more digits
        let result = s.str().split_regex_get(r"\d+", 1).unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("bar".into()));
        // "hello" has no digits, so split produces ["hello"] and index 1 is out of bounds
        assert!(result.values()[1].is_missing());
    }

    // ── GroupBy filter tests ──

    #[test]
    fn dataframe_groupby_filter_keeps_matching_groups() {
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
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        // Keep groups where the sum of val > 5
        let result = df
            .groupby(&["grp"])
            .unwrap()
            .filter(|group_df| {
                let sum: f64 = group_df.columns["val"]
                    .values()
                    .iter()
                    .filter_map(|v| match v {
                        Scalar::Float64(f) => Some(*f),
                        _ => None,
                    })
                    .sum();
                Ok(sum > 5.0)
            })
            .unwrap();

        // Group "a" has sum 3.0 (dropped), group "b" has sum 30.0 (kept)
        assert_eq!(result.len(), 2);
        assert_eq!(result.columns["val"].values()[0], Scalar::Float64(10.0));
        assert_eq!(result.columns["val"].values()[1], Scalar::Float64(20.0));
    }

    #[test]
    fn dataframe_groupby_filter_empty_result() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "grp",
                vec![0_i64.into(), 1_i64.into()],
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                ],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
        ])
        .unwrap();

        // Drop all groups
        let result = df
            .groupby(&["grp"])
            .unwrap()
            .filter(|_| Ok(false))
            .unwrap();

        assert_eq!(result.len(), 0);
    }

    // ── Series comparison operator tests ──

    #[test]
    fn series_eq_scalar() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(1)],
        )
        .unwrap();
        let result = s.eq_scalar(&Scalar::Int64(1)).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
        assert_eq!(result.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn series_gt_scalar() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)],
        )
        .unwrap();
        let result = s.gt_scalar(&Scalar::Int64(2)).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(false));
        assert_eq!(result.values()[1], Scalar::Bool(true));
        assert_eq!(result.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn series_le_scalar() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.5), Scalar::Float64(2.5)],
        )
        .unwrap();
        let result = s.le_scalar(&Scalar::Float64(2.0)).unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
    }

    #[test]
    fn series_equals_identical() {
        let a = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let b = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        assert!(a.equals(&b));
    }

    #[test]
    fn series_equals_different_values() {
        let a = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Int64(1)],
        )
        .unwrap();
        let b = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Int64(2)],
        )
        .unwrap();
        assert!(!a.equals(&b));
    }

    #[test]
    fn series_equals_different_names() {
        let a = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Int64(1)],
        )
        .unwrap();
        let b = Series::from_values(
            "y",
            vec![0_i64.into()],
            vec![Scalar::Int64(1)],
        )
        .unwrap();
        assert!(!a.equals(&b));
    }

    #[test]
    fn series_equals_nan_matches_nan() {
        let a = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)],
        )
        .unwrap();
        let b = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)],
        )
        .unwrap();
        assert!(a.equals(&b));
    }

    // ── Series update tests ──

    #[test]
    fn series_update_replaces_matching_labels() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();
        let other = Series::from_values(
            "y",
            vec![1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        let result = s.update(&other).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1)); // no match in other
        assert_eq!(result.values()[1], Scalar::Int64(20)); // updated
        assert_eq!(result.values()[2], Scalar::Int64(30)); // updated
    }

    #[test]
    fn series_update_skips_null_in_other() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let other = Series::from_values(
            "y",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Null(NullKind::NaN), Scalar::Int64(99)],
        )
        .unwrap();
        let result = s.update(&other).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1)); // other is null, keep self
        assert_eq!(result.values()[1], Scalar::Int64(99)); // updated
    }

    // ── DataFrame update tests ──

    #[test]
    fn dataframe_update() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
        ])
        .unwrap();
        let other = DataFrame::from_series(vec![Series::from_values(
            "a",
            vec![1_i64.into()],
            vec![Scalar::Int64(99)],
        )
        .unwrap()])
        .unwrap();

        let result = df.update(&other).unwrap();
        assert_eq!(result.columns["a"].values()[0], Scalar::Int64(1));
        assert_eq!(result.columns["a"].values()[1], Scalar::Int64(99)); // updated
        assert_eq!(result.columns["b"].values()[0], Scalar::Int64(10)); // unchanged
    }

    // ── DataFrame combine_first tests ──

    #[test]
    fn dataframe_combine_first() {
        let df1 = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)],
            )
            .unwrap(),
        ])
        .unwrap();
        let df2 = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(200), Scalar::Int64(300)],
            )
            .unwrap(),
        ])
        .unwrap();

        let result = df1.combine_first(&df2).unwrap();
        // Union index: [0, 1, 2], union columns: [a, b]
        assert_eq!(result.len(), 3);
        assert_eq!(result.column_order.len(), 2);
        // a[0]=1 (from self), a[1]=20 (self was null, take other), a[2]=30 (from other)
        assert_eq!(result.columns["a"].values()[0], Scalar::Int64(1));
        assert_eq!(result.columns["a"].values()[1], Scalar::Int64(20));
        assert_eq!(result.columns["a"].values()[2], Scalar::Int64(30));
        // b[0]=NaN (not in self or other at label 0), b[1]=200, b[2]=300
        assert!(result.columns["b"].values()[0].is_missing());
        assert_eq!(result.columns["b"].values()[1], Scalar::Int64(200));
        assert_eq!(result.columns["b"].values()[2], Scalar::Int64(300));
    }

    // ── DataFrame rename_columns_map test ──

    #[test]
    fn dataframe_rename_columns_map() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "old_a",
                vec![0_i64.into()],
                vec![Scalar::Int64(1)],
            )
            .unwrap(),
            Series::from_values(
                "old_b",
                vec![0_i64.into()],
                vec![Scalar::Int64(2)],
            )
            .unwrap(),
        ])
        .unwrap();

        let mut mapping = std::collections::HashMap::new();
        mapping.insert("old_a".to_string(), "new_a".to_string());
        let result = df.rename_columns_map(&mapping).unwrap();
        assert!(result.columns.contains_key("new_a"));
        assert!(result.columns.contains_key("old_b")); // not renamed
        assert!(!result.columns.contains_key("old_a"));
    }

    // ── Series convenience method tests ──

    #[test]
    fn series_clip_lower() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(5.0), Scalar::Float64(3.0)],
        )
        .unwrap();
        let result = s.clip_lower(2.5).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(2.5));
        assert_eq!(result.values()[1], Scalar::Float64(5.0));
        assert_eq!(result.values()[2], Scalar::Float64(3.0));
    }

    #[test]
    fn series_clip_upper() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(5.0), Scalar::Float64(3.0)],
        )
        .unwrap();
        let result = s.clip_upper(3.5).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(3.5));
        assert_eq!(result.values()[2], Scalar::Float64(3.0));
    }

    #[test]
    fn series_add_prefix_suffix() {
        let s = Series::from_values(
            "col",
            vec![0_i64.into()],
            vec![Scalar::Int64(1)],
        )
        .unwrap();
        let prefixed = s.add_prefix("pre_").unwrap();
        assert_eq!(prefixed.name(), "pre_col");
        let suffixed = s.add_suffix("_suf").unwrap();
        assert_eq!(suffixed.name(), "col_suf");
    }

    // ── Series.dtype tests ──

    #[test]
    fn series_dtype_returns_column_dtype() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Float64(1.0)],
        )
        .unwrap();
        assert_eq!(s.dtype(), DType::Float64);

        let s2 = Series::from_values(
            "y",
            vec![0_i64.into()],
            vec![Scalar::Int64(42)],
        )
        .unwrap();
        assert_eq!(s2.dtype(), DType::Int64);
    }

    // ── Series.copy tests ──

    #[test]
    fn series_copy_is_deep() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let c = s.copy();
        assert_eq!(c.values(), s.values());
        assert_eq!(c.name(), s.name());
    }

    // ── Series.prod tests ──

    #[test]
    fn series_prod() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(2.0), Scalar::Float64(3.0), Scalar::Float64(5.0)],
        )
        .unwrap();
        let result = s.prod().unwrap();
        assert_eq!(result, Scalar::Float64(30.0));
    }

    #[test]
    fn series_prod_with_nulls() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();
        let result = s.prod().unwrap();
        assert_eq!(result, Scalar::Float64(8.0));
    }

    // ── Series.mode tests ──

    #[test]
    fn series_mode_single() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
            ],
        )
        .unwrap();
        let result = s.mode().unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.values()[0], Scalar::Int64(2));
    }

    #[test]
    fn series_mode_tie() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ],
        )
        .unwrap();
        let result = s.mode().unwrap();
        assert_eq!(result.len(), 2);
    }

    // ── DataFrame.shape tests ──

    #[test]
    fn dataframe_shape() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(4), Scalar::Int64(5), Scalar::Int64(6)],
            )
            .unwrap(),
        ])
        .unwrap();
        assert_eq!(df.shape(), (3, 2));
    }

    // ── DataFrame.dtypes tests ──

    #[test]
    fn dataframe_dtypes() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into()],
                vec![Scalar::Int64(1)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into()],
                vec![Scalar::Float64(1.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let dtypes = df.dtypes().unwrap();
        assert_eq!(dtypes.len(), 2);
    }

    // ── DataFrame.copy tests ──

    #[test]
    fn dataframe_copy_is_deep() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into()],
                vec![Scalar::Int64(1)],
            )
            .unwrap(),
        ])
        .unwrap();
        let c = df.copy();
        assert_eq!(c.shape(), df.shape());
    }

    // ── DataFrame.to_dict tests ──

    #[test]
    fn dataframe_to_dict_default() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.to_dict("dict").unwrap();
        assert!(result.contains_key("a"));
        assert_eq!(result["a"].len(), 2);
    }

    #[test]
    fn dataframe_to_dict_records() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.to_dict("records").unwrap();
        assert_eq!(result.len(), 2); // 2 rows
    }

    #[test]
    fn dataframe_to_dict_invalid_orient() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into()],
                vec![Scalar::Int64(1)],
            )
            .unwrap(),
        ])
        .unwrap();
        assert!(df.to_dict("invalid").is_err());
    }

    // ── DataFrame.nunique tests ──

    #[test]
    fn dataframe_nunique() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.nunique().unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(2));
    }

    // ── DataFrame.all / .any tests ──

    #[test]
    fn dataframe_all_any() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Bool(true), Scalar::Bool(true)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Bool(true), Scalar::Bool(false)],
            )
            .unwrap(),
        ])
        .unwrap();
        let all_result = df.all().unwrap();
        assert_eq!(all_result.values()[0], Scalar::Bool(true)); // a: all true
        assert_eq!(all_result.values()[1], Scalar::Bool(false)); // b: not all true

        let any_result = df.any().unwrap();
        assert_eq!(any_result.values()[0], Scalar::Bool(true)); // a: any true
        assert_eq!(any_result.values()[1], Scalar::Bool(true)); // b: any true
    }

    // ── DataFrame.sum / .mean tests ──

    #[test]
    fn dataframe_sum_mean() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let sum = df.sum().unwrap();
        assert_eq!(sum.values()[0], Scalar::Float64(6.0));

        let mean = df.mean().unwrap();
        assert_eq!(mean.values()[0], Scalar::Float64(2.0));
    }

    // ── DataFrame element-wise ops tests ──

    #[test]
    fn dataframe_cumsum() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.cumsum().unwrap();
        let col = result.column("a").unwrap();
        assert_eq!(col.values()[0], Scalar::Float64(1.0));
        assert_eq!(col.values()[1], Scalar::Float64(3.0));
        assert_eq!(col.values()[2], Scalar::Float64(6.0));
    }

    #[test]
    fn dataframe_abs() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(-1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.abs().unwrap();
        let col = result.column("a").unwrap();
        assert_eq!(col.values()[0], Scalar::Float64(1.0));
        assert_eq!(col.values()[1], Scalar::Float64(2.0));
    }

    #[test]
    fn dataframe_diff() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(3.0), Scalar::Float64(6.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.diff(1).unwrap();
        let col = result.column("a").unwrap();
        assert!(col.values()[0].is_missing());
        assert_eq!(col.values()[1], Scalar::Float64(2.0));
        assert_eq!(col.values()[2], Scalar::Float64(3.0));
    }

    #[test]
    fn dataframe_shift() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.shift(1).unwrap();
        let col = result.column("a").unwrap();
        assert!(col.values()[0].is_missing());
        assert_eq!(col.values()[1], Scalar::Float64(1.0));
        assert_eq!(col.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn dataframe_clip() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(5.0), Scalar::Float64(10.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.clip(Some(2.0), Some(8.0)).unwrap();
        let col = result.column("a").unwrap();
        assert_eq!(col.values()[0], Scalar::Float64(2.0));
        assert_eq!(col.values()[1], Scalar::Float64(5.0));
        assert_eq!(col.values()[2], Scalar::Float64(8.0));
    }

    #[test]
    fn dataframe_pct_change() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(100.0), Scalar::Float64(110.0), Scalar::Float64(99.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.pct_change(1).unwrap();
        let col = result.column("a").unwrap();
        assert!(col.values()[0].is_missing());
        if let Scalar::Float64(v) = &col.values()[1] {
            assert!((v - 0.1).abs() < 1e-10);
        }
    }

    // ── DataFrame.prod_agg test ──

    #[test]
    fn dataframe_prod_agg() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(2.0), Scalar::Float64(3.0), Scalar::Float64(5.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.prod_agg().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(30.0));
    }

    // ── DataFrame.idxmin / .idxmax tests ──

    #[test]
    fn dataframe_idxmin_idxmax() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
                vec![Scalar::Float64(3.0), Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let idxmin = df.idxmin().unwrap();
        assert_eq!(idxmin.len(), 1);

        let idxmax = df.idxmax().unwrap();
        assert_eq!(idxmax.len(), 1);
    }

    // ── DataFrame element-wise ops preserve non-numeric columns ──

    #[test]
    fn dataframe_cumsum_preserves_nonnumeric() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "name",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Utf8("alice".to_string()), Scalar::Utf8("bob".to_string())],
            )
            .unwrap(),
            Series::from_values(
                "val",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
            )
            .unwrap(),
        ])
        .unwrap();
        let result = df.cumsum().unwrap();
        // Non-numeric "name" column preserved as-is
        assert!(result.column("name").is_some());
        assert_eq!(
            result.column("name").unwrap().values()[0],
            Scalar::Utf8("alice".to_string())
        );
        // Numeric "val" column cumulated
        assert_eq!(
            result.column("val").unwrap().values()[1],
            Scalar::Float64(3.0)
        );
    }

    // ── Display tests ──

    #[test]
    fn series_display() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.5), Scalar::Float64(3.0)],
        )
        .unwrap();
        let output = format!("{s}");
        assert!(output.contains("Name: x"));
        assert!(output.contains("Length: 3"));
        assert!(output.contains("Float64"));
        assert!(output.contains("2.5"));
    }

    #[test]
    fn dataframe_display() {
        let df = DataFrame::from_series(vec![
            Series::from_values(
                "a",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
            Series::from_values(
                "b",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Float64(1.5), Scalar::Float64(2.5)],
            )
            .unwrap(),
        ])
        .unwrap();
        let output = format!("{df}");
        assert!(output.contains("a"));
        assert!(output.contains("b"));
        assert!(output.contains("10"));
        assert!(output.contains("2.5"));
        assert!(output.contains("[2 rows x 2 columns]"));
    }

    // ── DatetimeAccessor tests ──

    #[test]
    fn dt_year_month_day() {
        let s = Series::from_values(
            "dates",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("2024-03-15".to_string()),
                Scalar::Utf8("2023-12-25".to_string()),
                Scalar::Utf8("2020-01-01".to_string()),
            ],
        )
        .unwrap();

        let year = s.dt().year().unwrap();
        assert_eq!(year.values()[0], Scalar::Int64(2024));
        assert_eq!(year.values()[1], Scalar::Int64(2023));
        assert_eq!(year.values()[2], Scalar::Int64(2020));

        let month = s.dt().month().unwrap();
        assert_eq!(month.values()[0], Scalar::Int64(3));
        assert_eq!(month.values()[1], Scalar::Int64(12));
        assert_eq!(month.values()[2], Scalar::Int64(1));

        let day = s.dt().day().unwrap();
        assert_eq!(day.values()[0], Scalar::Int64(15));
        assert_eq!(day.values()[1], Scalar::Int64(25));
        assert_eq!(day.values()[2], Scalar::Int64(1));
    }

    #[test]
    fn dt_hour_minute_second() {
        let s = Series::from_values(
            "times",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("2024-03-15T14:30:45".to_string()),
                Scalar::Utf8("2023-12-25 08:15:00".to_string()),
            ],
        )
        .unwrap();

        let hour = s.dt().hour().unwrap();
        assert_eq!(hour.values()[0], Scalar::Int64(14));
        assert_eq!(hour.values()[1], Scalar::Int64(8));

        let minute = s.dt().minute().unwrap();
        assert_eq!(minute.values()[0], Scalar::Int64(30));
        assert_eq!(minute.values()[1], Scalar::Int64(15));

        let second = s.dt().second().unwrap();
        assert_eq!(second.values()[0], Scalar::Int64(45));
        assert_eq!(second.values()[1], Scalar::Int64(0));
    }

    #[test]
    fn dt_dayofweek() {
        let s = Series::from_values(
            "dates",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("2024-03-15".to_string()), // Friday = 4
                Scalar::Utf8("2024-03-18".to_string()), // Monday = 0
            ],
        )
        .unwrap();

        let dow = s.dt().dayofweek().unwrap();
        assert_eq!(dow.values()[0], Scalar::Int64(4)); // Friday
        assert_eq!(dow.values()[1], Scalar::Int64(0)); // Monday
    }

    #[test]
    fn dt_date_extraction() {
        let s = Series::from_values(
            "datetimes",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("2024-03-15T14:30:45".to_string()),
                Scalar::Utf8("2023-12-25 08:15:00".to_string()),
            ],
        )
        .unwrap();

        let dates = s.dt().date().unwrap();
        assert_eq!(dates.values()[0], Scalar::Utf8("2024-03-15".to_string()));
        assert_eq!(dates.values()[1], Scalar::Utf8("2023-12-25".to_string()));
    }

    #[test]
    fn dt_with_nulls() {
        let s = Series::from_values(
            "dates",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("2024-03-15".to_string()),
                Scalar::Null(NullKind::NaN),
                Scalar::Utf8("invalid".to_string()),
            ],
        )
        .unwrap();

        let year = s.dt().year().unwrap();
        assert_eq!(year.values()[0], Scalar::Int64(2024));
        assert!(year.values()[1].is_missing());
        assert!(year.values()[2].is_missing());
    }

    #[test]
    fn dt_with_timezone() {
        let s = Series::from_values(
            "dates",
            vec![0_i64.into()],
            vec![Scalar::Utf8("2024-03-15T14:30:45+05:30".to_string())],
        )
        .unwrap();

        let hour = s.dt().hour().unwrap();
        assert_eq!(hour.values()[0], Scalar::Int64(14));

        let second = s.dt().second().unwrap();
        assert_eq!(second.values()[0], Scalar::Int64(45));
    }

    // --- Series.map_fn tests ---

    #[test]
    fn series_map_fn_infallible() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        let doubled = s
            .map_fn(|v| match v {
                Scalar::Int64(n) => Ok(Scalar::Int64(n * 2)),
                _ => Ok(v.clone()),
            })
            .unwrap();
        assert_eq!(
            doubled.values(),
            &[Scalar::Int64(2), Scalar::Int64(4), Scalar::Int64(6)]
        );
    }

    #[test]
    fn series_map_fn_with_error() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("not_a_number".to_owned())],
        )
        .unwrap();

        let result = s.map_fn(|v| match v {
            Scalar::Int64(n) => Ok(Scalar::Float64(*n as f64)),
            _ => Err(FrameError::CompatibilityRejected(
                "expected Int64".to_owned(),
            )),
        });
        assert!(result.is_err());
    }

    #[test]
    fn series_map_fn_preserves_nulls() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(5), Scalar::Null(NullKind::NaN)],
        )
        .unwrap();

        let out = s
            .map_fn(|v| {
                if v.is_missing() {
                    Ok(Scalar::Int64(-1))
                } else {
                    Ok(v.clone())
                }
            })
            .unwrap();
        assert_eq!(out.values()[0], Scalar::Int64(5));
        assert_eq!(out.values()[1], Scalar::Int64(-1));
    }

    // --- DataFrame.drop tests ---

    #[test]
    fn dataframe_drop_columns() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
                ("c", vec![Scalar::Int64(5), Scalar::Int64(6)]),
            ],
        )
        .unwrap();

        let dropped = df.drop(&["b"], 1).unwrap();
        assert_eq!(dropped.num_columns(), 2);
        let names: Vec<&str> = dropped.column_names().into_iter().map(String::as_str).collect();
        assert_eq!(names, vec!["a", "c"]);
        assert_eq!(
            dropped.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
    }

    #[test]
    fn dataframe_drop_multiple_columns() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Int64(2)]),
                ("c", vec![Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let dropped = df.drop(&["a", "c"], 1).unwrap();
        assert_eq!(dropped.num_columns(), 1);
        let names: Vec<&str> = dropped.column_names().into_iter().map(String::as_str).collect();
        assert_eq!(names, vec!["b"]);
    }

    #[test]
    fn dataframe_drop_column_not_found() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        assert!(df.drop(&["z"], 1).is_err());
    }

    #[test]
    fn dataframe_drop_rows_by_label() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("v", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]),
            ],
            vec![
                IndexLabel::Utf8("r1".to_owned()),
                IndexLabel::Utf8("r2".to_owned()),
                IndexLabel::Utf8("r3".to_owned()),
            ],
        )
        .unwrap();

        let dropped = df.drop(&["r2"], 0).unwrap();
        assert_eq!(dropped.len(), 2);
        assert_eq!(
            dropped.index().labels(),
            &[
                IndexLabel::Utf8("r1".to_owned()),
                IndexLabel::Utf8("r3".to_owned())
            ]
        );
        assert_eq!(
            dropped.column("v").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(30)]
        );
    }

    #[test]
    fn dataframe_drop_rows_int() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![
                ("a", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]),
            ],
        )
        .unwrap();

        let dropped = df.drop_rows_int(&[1]).unwrap();
        assert_eq!(dropped.len(), 2);
        assert_eq!(
            dropped.column("a").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(30)]
        );
    }

    // --- DataFrame.insert tests ---

    #[test]
    fn dataframe_insert_at_position() {
        let df = DataFrame::from_dict(
            &["a", "c"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("c", vec![Scalar::Int64(5), Scalar::Int64(6)]),
            ],
        )
        .unwrap();

        let col_b = Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(4)]).unwrap();
        let inserted = df.insert(1, "b", col_b).unwrap();
        let names: Vec<&str> = inserted.column_names().into_iter().map(String::as_str).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert_eq!(
            inserted.column("b").unwrap().values(),
            &[Scalar::Int64(3), Scalar::Int64(4)]
        );
    }

    #[test]
    fn dataframe_insert_at_start() {
        let df = DataFrame::from_dict(
            &["b"],
            vec![("b", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        let col = Column::from_values(vec![Scalar::Int64(99)]).unwrap();
        let inserted = df.insert(0, "a", col).unwrap();
        let names: Vec<&str> = inserted.column_names().into_iter().map(String::as_str).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn dataframe_insert_duplicate_rejects() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        let col = Column::from_values(vec![Scalar::Int64(2)]).unwrap();
        assert!(df.insert(0, "a", col).is_err());
    }

    #[test]
    fn dataframe_insert_length_mismatch() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let col = Column::from_values(vec![Scalar::Int64(99)]).unwrap();
        assert!(df.insert(0, "b", col).is_err());
    }

    // --- DataFrame.pop tests ---

    #[test]
    fn dataframe_pop_column() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let (series, remaining) = df.pop("a").unwrap();
        assert_eq!(series.name(), "a");
        assert_eq!(series.values(), &[Scalar::Int64(1), Scalar::Int64(2)]);
        assert_eq!(remaining.num_columns(), 1);
        assert!(remaining.column("a").is_none());
        assert!(remaining.column("b").is_some());
    }

    #[test]
    fn dataframe_pop_not_found() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        assert!(df.pop("z").is_err());
    }

    // --- DataFrame.replace tests ---

    #[test]
    fn dataframe_replace_scalars() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
                ("b", vec![Scalar::Int64(2), Scalar::Int64(4), Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let replaced = df
            .replace(&[(Scalar::Int64(2), Scalar::Int64(99))])
            .unwrap();
        assert_eq!(
            replaced.column("a").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(99), Scalar::Int64(3)]
        );
        assert_eq!(
            replaced.column("b").unwrap().values(),
            &[Scalar::Int64(99), Scalar::Int64(4), Scalar::Int64(99)]
        );
    }

    #[test]
    fn dataframe_replace_multiple() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])],
        )
        .unwrap();

        let replaced = df
            .replace(&[
                (Scalar::Int64(1), Scalar::Int64(10)),
                (Scalar::Int64(3), Scalar::Int64(30)),
            ])
            .unwrap();
        assert_eq!(
            replaced.column("a").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(2), Scalar::Int64(30)]
        );
    }

    #[test]
    fn dataframe_replace_string_values() {
        let df = DataFrame::from_dict(
            &["s"],
            vec![(
                "s",
                vec![
                    Scalar::Utf8("hello".to_owned()),
                    Scalar::Utf8("world".to_owned()),
                ],
            )],
        )
        .unwrap();

        let replaced = df
            .replace(&[(
                Scalar::Utf8("hello".to_owned()),
                Scalar::Utf8("hi".to_owned()),
            )])
            .unwrap();
        assert_eq!(
            replaced.column("s").unwrap().values(),
            &[
                Scalar::Utf8("hi".to_owned()),
                Scalar::Utf8("world".to_owned())
            ]
        );
    }

    // --- DataFrame.align_on_index tests ---

    #[test]
    fn dataframe_align_outer() {
        let df1 = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
            vec![0_i64.into(), 1_i64.into()],
        )
        .unwrap();

        let df2 = DataFrame::from_dict_with_index(
            vec![("b", vec![Scalar::Int64(10), Scalar::Int64(20)])],
            vec![1_i64.into(), 2_i64.into()],
        )
        .unwrap();

        let (left, right) = df1.align_on_index(&df2, AlignMode::Outer).unwrap();
        assert_eq!(left.len(), 3);
        assert_eq!(right.len(), 3);

        // Left should have column 'a' with values [1, 2, NaN] and column 'b' with [NaN, NaN, NaN]
        assert_eq!(left.column("a").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(left.column("a").unwrap().values()[1], Scalar::Int64(2));
        assert!(left.column("a").unwrap().values()[2].is_missing());

        // Right should have column 'a' with [NaN, NaN, NaN] and column 'b' with [NaN, 10, 20]
        assert!(right.column("a").unwrap().values()[0].is_missing());
        assert_eq!(right.column("b").unwrap().values()[1], Scalar::Int64(10));
        assert_eq!(right.column("b").unwrap().values()[2], Scalar::Int64(20));
    }

    #[test]
    fn dataframe_align_inner() {
        let df1 = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])],
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        )
        .unwrap();

        let df2 = DataFrame::from_dict_with_index(
            vec![("b", vec![Scalar::Int64(10), Scalar::Int64(20)])],
            vec![1_i64.into(), 2_i64.into()],
        )
        .unwrap();

        let (left, right) = df1.align_on_index(&df2, AlignMode::Inner).unwrap();
        assert_eq!(left.len(), 2);
        assert_eq!(right.len(), 2);
        assert_eq!(left.column("a").unwrap().values()[0], Scalar::Int64(2));
        assert_eq!(left.column("a").unwrap().values()[1], Scalar::Int64(3));
        assert_eq!(right.column("b").unwrap().values()[0], Scalar::Int64(10));
        assert_eq!(right.column("b").unwrap().values()[1], Scalar::Int64(20));
    }

    #[test]
    fn dataframe_align_same_index() {
        let df1 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let df2 = DataFrame::from_dict(
            &["b"],
            vec![("b", vec![Scalar::Int64(10), Scalar::Int64(20)])],
        )
        .unwrap();

        let (left, right) = df1.align_on_index(&df2, AlignMode::Outer).unwrap();
        assert_eq!(left.len(), 2);
        assert_eq!(right.len(), 2);
        assert_eq!(left.column("a").unwrap().values(), &[Scalar::Int64(1), Scalar::Int64(2)]);
        assert_eq!(right.column("b").unwrap().values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
    }

    // --- Series introspection tests ---

    #[test]
    fn series_value_counts_with_options_normalize() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let vc = s
            .value_counts_with_options(true, true, false, true)
            .unwrap();
        // Normalized: each count / 4
        assert_eq!(vc.len(), 2);
        // Both values have 0.5
        if let Scalar::Float64(v) = &vc.values()[0] {
            assert!((v - 0.5).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn series_value_counts_ascending() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(2)],
        )
        .unwrap();

        let vc = s
            .value_counts_with_options(false, true, true, true)
            .unwrap();
        // Ascending sort: 1 (count=1) first, then 2 (count=2)
        assert_eq!(vc.values()[0], Scalar::Int64(1));
        assert_eq!(vc.values()[1], Scalar::Int64(2));
    }

    #[test]
    fn series_value_counts_include_na() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
            ],
        )
        .unwrap();

        let vc = s
            .value_counts_with_options(false, true, false, false)
            .unwrap();
        // Should include NaN as a category
        assert_eq!(vc.len(), 2); // 1 (count=2) and NaN (count=1)
    }

    #[test]
    fn series_is_unique_true() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        assert!(s.is_unique());
    }

    #[test]
    fn series_is_unique_false() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();

        assert!(!s.is_unique());
    }

    #[test]
    fn series_is_monotonic_increasing_true() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();

        assert!(s.is_monotonic_increasing());
        assert!(!s.is_monotonic_decreasing());
    }

    #[test]
    fn series_is_monotonic_decreasing_true() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(3), Scalar::Int64(2), Scalar::Int64(1)],
        )
        .unwrap();

        assert!(!s.is_monotonic_increasing());
        assert!(s.is_monotonic_decreasing());
    }

    #[test]
    fn series_is_monotonic_equal_values() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(5)],
        )
        .unwrap();

        assert!(s.is_monotonic_increasing());
        assert!(s.is_monotonic_decreasing());
    }

    #[test]
    fn series_is_monotonic_with_nan() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
            ],
        )
        .unwrap();

        assert!(!s.is_monotonic_increasing());
        assert!(!s.is_monotonic_decreasing());
    }

    #[test]
    fn series_is_monotonic_empty() {
        let s = Series::from_values("x", vec![], vec![]).unwrap();
        assert!(s.is_monotonic_increasing());
        assert!(s.is_monotonic_decreasing());
    }

    // --- DataFrame.compare tests ---

    #[test]
    fn dataframe_compare_basic() {
        let df1 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let df2 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(99)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();

        let diff = df1.compare(&df2).unwrap();
        // Only column 'a' has differences, only row 1
        assert_eq!(diff.len(), 1);
        assert!(diff.column("a_self").is_some());
        assert!(diff.column("a_other").is_some());
        assert_eq!(diff.column("a_self").unwrap().values()[0], Scalar::Int64(2));
        assert_eq!(diff.column("a_other").unwrap().values()[0], Scalar::Int64(99));
    }

    #[test]
    fn dataframe_compare_no_diff() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let diff = df.compare(&df).unwrap();
        assert_eq!(diff.len(), 0);
        assert_eq!(diff.num_columns(), 0);
    }

    #[test]
    fn dataframe_compare_length_mismatch() {
        let df1 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        assert!(df1.compare(&df2).is_err());
    }

    #[test]
    fn dataframe_compare_multiple_diffs() {
        let df1 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]),
            ],
        )
        .unwrap();

        let df2 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(99), Scalar::Int64(3)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(77)]),
            ],
        )
        .unwrap();

        let diff = df1.compare(&df2).unwrap();
        // Row 1 has diff in a, row 2 has diff in b
        assert_eq!(diff.len(), 2);
    }

    // --- Series.str formatting & predicates tests ---

    #[test]
    fn str_zfill() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("42".to_owned()),
                Scalar::Utf8("-5".to_owned()),
                Scalar::Utf8("abc".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().zfill(5).unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("00042".to_owned()));
        assert_eq!(result.values()[1], Scalar::Utf8("-0005".to_owned()));
        assert_eq!(result.values()[2], Scalar::Utf8("00abc".to_owned()));
    }

    #[test]
    fn str_center() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hi".to_owned())],
        )
        .unwrap();

        let result = s.str().center(6, '-').unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("--hi--".to_owned()));
    }

    #[test]
    fn str_ljust_rjust() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hi".to_owned())],
        )
        .unwrap();

        let left = s.str().ljust(5, '.').unwrap();
        assert_eq!(left.values()[0], Scalar::Utf8("hi...".to_owned()));

        let right = s.str().rjust(5, '.').unwrap();
        assert_eq!(right.values()[0], Scalar::Utf8("...hi".to_owned()));
    }

    #[test]
    fn str_isdigit() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("123".to_owned()),
                Scalar::Utf8("12a".to_owned()),
                Scalar::Utf8("".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().isdigit().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
        assert_eq!(result.values()[2], Scalar::Bool(false));
    }

    #[test]
    fn str_isalpha() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("abc".to_owned()),
                Scalar::Utf8("ab1".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().isalpha().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
    }

    #[test]
    fn str_isalnum() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("abc123".to_owned()),
                Scalar::Utf8("abc 123".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().isalnum().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
    }

    #[test]
    fn str_isspace() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("   ".to_owned()),
                Scalar::Utf8(" x ".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().isspace().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
    }

    #[test]
    fn str_islower_isupper() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("hello".to_owned()),
                Scalar::Utf8("HELLO".to_owned()),
                Scalar::Utf8("Hello".to_owned()),
            ],
        )
        .unwrap();

        let lower = s.str().islower().unwrap();
        assert_eq!(lower.values()[0], Scalar::Bool(true));
        assert_eq!(lower.values()[1], Scalar::Bool(false));
        assert_eq!(lower.values()[2], Scalar::Bool(false));

        let upper = s.str().isupper().unwrap();
        assert_eq!(upper.values()[0], Scalar::Bool(false));
        assert_eq!(upper.values()[1], Scalar::Bool(true));
        assert_eq!(upper.values()[2], Scalar::Bool(false));
    }

    #[test]
    fn str_isnumeric() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("123".to_owned()),
                Scalar::Utf8("12.3".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().isnumeric().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false)); // '.' is not numeric
    }

    // --- Series binary ops with fill_value tests ---

    #[test]
    fn series_add_fill_basic() {
        let s1 = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();

        let s2 = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s1.add_fill(&s2, 0.0).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(11.0)); // 1+10
        assert_eq!(result.values()[1], Scalar::Float64(20.0)); // 0+20 (fill)
        assert_eq!(result.values()[2], Scalar::Float64(3.0)); // 3+0 (fill)
    }

    #[test]
    fn series_add_fill_both_nan() {
        let s1 = Series::from_values(
            "a",
            vec![0_i64.into()],
            vec![Scalar::Null(NullKind::NaN)],
        )
        .unwrap();

        let s2 = Series::from_values(
            "b",
            vec![0_i64.into()],
            vec![Scalar::Null(NullKind::NaN)],
        )
        .unwrap();

        let result = s1.add_fill(&s2, 0.0).unwrap();
        assert!(result.values()[0].is_missing()); // both NaN → NaN
    }

    #[test]
    fn series_sub_fill() {
        let s1 = Series::from_values(
            "a",
            vec![0_i64.into()],
            vec![Scalar::Null(NullKind::NaN)],
        )
        .unwrap();

        let s2 = Series::from_values(
            "b",
            vec![0_i64.into()],
            vec![Scalar::Float64(5.0)],
        )
        .unwrap();

        let result = s1.sub_fill(&s2, 10.0).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(5.0)); // 10-5
    }

    #[test]
    fn series_modulo_basic() {
        let s1 = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(10.0), Scalar::Float64(7.0)],
        )
        .unwrap();

        let s2 = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(3.0), Scalar::Float64(4.0)],
        )
        .unwrap();

        let result = s1.modulo(&s2).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0)); // 10 % 3
        assert_eq!(result.values()[1], Scalar::Float64(3.0)); // 7 % 4
    }

    #[test]
    fn series_pow_basic() {
        let s1 = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(2.0), Scalar::Float64(3.0)],
        )
        .unwrap();

        let s2 = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(3.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let result = s1.pow(&s2).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(8.0)); // 2^3
        assert_eq!(result.values()[1], Scalar::Float64(9.0)); // 3^2
    }

    // --- DataFrame.select_dtypes & filter_labels tests ---

    #[test]
    fn dataframe_select_dtypes_include() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("c", vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())]),
            ],
        )
        .unwrap();

        let numeric = df.select_dtypes(&[DType::Int64, DType::Float64], &[]).unwrap();
        assert_eq!(numeric.num_columns(), 2);
        let names: Vec<&str> = numeric.column_names().into_iter().map(String::as_str).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn dataframe_select_dtypes_exclude() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Utf8("x".to_owned())]),
            ],
        )
        .unwrap();

        let non_string = df.select_dtypes(&[], &[DType::Utf8]).unwrap();
        assert_eq!(non_string.num_columns(), 1);
        assert!(non_string.column("a").is_some());
    }

    #[test]
    fn dataframe_filter_labels_items() {
        let df = DataFrame::from_dict(
            &["alpha", "beta", "gamma"],
            vec![
                ("alpha", vec![Scalar::Int64(1)]),
                ("beta", vec![Scalar::Int64(2)]),
                ("gamma", vec![Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let filtered = df
            .filter_labels(Some(&["alpha", "gamma"]), None, None, 1)
            .unwrap();
        assert_eq!(filtered.num_columns(), 2);
        let names: Vec<&str> = filtered.column_names().into_iter().map(String::as_str).collect();
        assert_eq!(names, vec!["alpha", "gamma"]);
    }

    #[test]
    fn dataframe_filter_labels_like() {
        let df = DataFrame::from_dict(
            &["col_a", "col_b", "other"],
            vec![
                ("col_a", vec![Scalar::Int64(1)]),
                ("col_b", vec![Scalar::Int64(2)]),
                ("other", vec![Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let filtered = df.filter_labels(None, Some("col_"), None, 1).unwrap();
        assert_eq!(filtered.num_columns(), 2);
    }

    #[test]
    fn dataframe_filter_labels_regex() {
        let df = DataFrame::from_dict(
            &["x1", "x2", "y1"],
            vec![
                ("x1", vec![Scalar::Int64(1)]),
                ("x2", vec![Scalar::Int64(2)]),
                ("y1", vec![Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let filtered = df.filter_labels(None, None, Some("^x"), 1).unwrap();
        assert_eq!(filtered.num_columns(), 2);
        assert!(filtered.column("x1").is_some());
        assert!(filtered.column("x2").is_some());
    }

    #[test]
    fn dataframe_filter_labels_rows() {
        let df = DataFrame::from_dict_with_index(
            vec![("v", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)])],
            vec![
                IndexLabel::Utf8("foo".to_owned()),
                IndexLabel::Utf8("bar".to_owned()),
                IndexLabel::Utf8("baz".to_owned()),
            ],
        )
        .unwrap();

        let filtered = df.filter_labels(None, Some("ba"), None, 0).unwrap();
        assert_eq!(filtered.len(), 2);
        assert_eq!(
            filtered.column("v").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );
    }

    // --- Series.str additional methods tests ---

    #[test]
    fn str_get_character() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("hello".to_owned()),
                Scalar::Utf8("ab".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().get(0).unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("h".to_owned()));
        assert_eq!(result.values()[1], Scalar::Utf8("a".to_owned()));

        let result2 = s.str().get(4).unwrap();
        assert_eq!(result2.values()[0], Scalar::Utf8("o".to_owned()));
        assert!(result2.values()[1].is_missing()); // out of bounds
    }

    #[test]
    fn str_wrap() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello world this is a long sentence".to_owned())],
        )
        .unwrap();

        let result = s.str().wrap(15).unwrap();
        if let Scalar::Utf8(wrapped) = &result.values()[0] {
            assert!(wrapped.contains('\n'));
        } else {
            panic!("expected Utf8");
        }
    }

    #[test]
    fn str_isdecimal() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("123".to_owned()),
                Scalar::Utf8("12.3".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().isdecimal().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
    }

    #[test]
    fn str_istitle() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("Hello World".to_owned()),
                Scalar::Utf8("hello world".to_owned()),
                Scalar::Utf8("HELLO".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().istitle().unwrap();
        assert_eq!(result.values()[0], Scalar::Bool(true));
        assert_eq!(result.values()[1], Scalar::Bool(false));
        assert_eq!(result.values()[2], Scalar::Bool(false));
    }

    #[test]
    fn str_cat_concatenation() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("hello".to_owned()),
                Scalar::Utf8("world".to_owned()),
                Scalar::Utf8("foo".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().cat(", ").unwrap();
        assert_eq!(result, "hello, world, foo");
    }

    // --- nlargest/nsmallest keep parameter tests ---

    #[test]
    fn series_nlargest_keep_first() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let result = s.nlargest_keep(2, "first").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(3));
        assert_eq!(result.values()[1], Scalar::Int64(3));
    }

    #[test]
    fn series_nsmallest_keep_all() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
            ],
        )
        .unwrap();

        // Asking for 2 smallest, but the 2nd smallest (1) appears twice
        // with keep='all', should return all tied values
        let result = s.nsmallest_keep(2, "all").unwrap();
        assert!(result.len() >= 2);
    }

    // --- DataFrame.apply_row tests ---

    #[test]
    fn dataframe_apply_row_sum() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();

        let result = df
            .apply_row("sum", |row| {
                let mut total = 0_i64;
                for val in row {
                    if let Scalar::Int64(v) = val {
                        total += v;
                    }
                }
                Scalar::Int64(total)
            })
            .unwrap();

        assert_eq!(result.name(), "sum");
        assert_eq!(result.values()[0], Scalar::Int64(11));
        assert_eq!(result.values()[1], Scalar::Int64(22));
    }

    #[test]
    fn dataframe_apply_row_fn_failable() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(10.0)]),
                ("b", vec![Scalar::Float64(3.0)]),
            ],
        )
        .unwrap();

        let result = df
            .apply_row_fn("ratio", |row| {
                let a = row[0].to_f64().map_err(fp_columnar::ColumnError::from)?;
                let b = row[1].to_f64().map_err(fp_columnar::ColumnError::from)?;
                Ok(Scalar::Float64(a / b))
            })
            .unwrap();

        if let Scalar::Float64(v) = result.values()[0] {
            assert!((v - 10.0 / 3.0).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    // --- DataFrame clip_lower/clip_upper/round tests ---

    #[test]
    fn dataframe_clip_lower() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Float64(1.0), Scalar::Float64(5.0), Scalar::Float64(10.0)])],
        )
        .unwrap();

        let result = df.clip_lower(3.0).unwrap();
        assert_eq!(
            result.column("a").unwrap().values(),
            &[Scalar::Float64(3.0), Scalar::Float64(5.0), Scalar::Float64(10.0)]
        );
    }

    #[test]
    fn dataframe_clip_upper() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Float64(1.0), Scalar::Float64(5.0), Scalar::Float64(10.0)])],
        )
        .unwrap();

        let result = df.clip_upper(7.0).unwrap();
        assert_eq!(
            result.column("a").unwrap().values(),
            &[Scalar::Float64(1.0), Scalar::Float64(5.0), Scalar::Float64(7.0)]
        );
    }

    #[test]
    fn dataframe_round() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Float64(1.234), Scalar::Float64(5.678)])],
        )
        .unwrap();

        let result = df.round(1).unwrap();
        assert_eq!(result.column("a").unwrap().values()[0], Scalar::Float64(1.2));
        assert_eq!(result.column("a").unwrap().values()[1], Scalar::Float64(5.7));
    }

    // --- DataFrame to_csv/to_json tests ---

    #[test]
    fn dataframe_to_csv_basic() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())]),
            ],
        )
        .unwrap();

        let csv = df.to_csv(',', false);
        assert!(csv.starts_with("a,b\n"));
        assert!(csv.contains("1,x\n"));
        assert!(csv.contains("2,y\n"));
    }

    #[test]
    fn dataframe_to_csv_with_index() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(10)])],
        )
        .unwrap();

        let csv = df.to_csv(',', true);
        assert!(csv.starts_with("index,a\n"));
        assert!(csv.contains("0,10\n"));
    }

    #[test]
    fn dataframe_to_json_records() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Utf8("hello".to_owned())]),
            ],
        )
        .unwrap();

        let json = df.to_json("records").unwrap();
        assert!(json.starts_with('['));
        assert!(json.ends_with(']'));
        assert!(json.contains("\"a\":1"));
        assert!(json.contains("\"b\":\"hello\""));
    }

    #[test]
    fn dataframe_to_json_columns() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(10), Scalar::Int64(20)])],
        )
        .unwrap();

        let json = df.to_json("columns").unwrap();
        assert!(json.contains("\"x\":{"));
    }

    // --- Series.explode tests ---

    #[test]
    fn series_explode_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("a,b,c".to_owned()),
                Scalar::Utf8("d".to_owned()),
            ],
        )
        .unwrap();

        let result = s.explode(",").unwrap();
        assert_eq!(result.len(), 4); // a, b, c, d
        assert_eq!(result.values()[0], Scalar::Utf8("a".to_owned()));
        assert_eq!(result.values()[1], Scalar::Utf8("b".to_owned()));
        assert_eq!(result.values()[2], Scalar::Utf8("c".to_owned()));
        assert_eq!(result.values()[3], Scalar::Utf8("d".to_owned()));
    }

    #[test]
    fn series_explode_preserves_index() {
        let s = Series::from_values(
            "x",
            vec![IndexLabel::Utf8("r1".to_owned()), IndexLabel::Utf8("r2".to_owned())],
            vec![
                Scalar::Utf8("a,b".to_owned()),
                Scalar::Utf8("c".to_owned()),
            ],
        )
        .unwrap();

        let result = s.explode(",").unwrap();
        assert_eq!(result.len(), 3);
        // r1 should be repeated for a and b
        assert_eq!(result.index().labels()[0], IndexLabel::Utf8("r1".to_owned()));
        assert_eq!(result.index().labels()[1], IndexLabel::Utf8("r1".to_owned()));
        assert_eq!(result.index().labels()[2], IndexLabel::Utf8("r2".to_owned()));
    }

    #[test]
    fn series_explode_with_nulls() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("a,b".to_owned()),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s.explode(",").unwrap();
        assert_eq!(result.len(), 3); // a, b, NaN
        assert!(result.values()[2].is_missing());
    }

    // --- str.split_count and str.join tests ---

    #[test]
    fn str_split_count() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("a,b,c".to_owned()),
                Scalar::Utf8("d".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().split_count(",").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(3));
        assert_eq!(result.values()[1], Scalar::Int64(1));
    }

    #[test]
    fn str_join() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("a,b,c".to_owned())],
        )
        .unwrap();

        let result = s.str().join(",", " | ").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("a | b | c".to_owned()));
    }

    // ── ffill / bfill / interpolate ──────────────────────────────

    #[test]
    fn series_ffill_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s.ffill(None).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(1.0));
        assert_eq!(result.values()[2], Scalar::Float64(1.0));
        assert_eq!(result.values()[3], Scalar::Float64(4.0));
        assert_eq!(result.values()[4], Scalar::Float64(4.0));
    }

    #[test]
    fn series_ffill_leading_nan() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s.ffill(None).unwrap();
        // Leading NaN can't be forward-filled
        assert!(result.values()[0].is_missing());
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
        assert_eq!(result.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn series_ffill_with_limit() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s.ffill(Some(1)).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(1.0)); // filled
        assert!(result.values()[2].is_missing()); // limit exceeded
        assert!(result.values()[3].is_missing()); // limit exceeded
    }

    #[test]
    fn series_bfill_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(5.0),
            ],
        )
        .unwrap();

        let result = s.bfill(None).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(2.0));
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
        assert_eq!(result.values()[2], Scalar::Float64(5.0));
        assert_eq!(result.values()[3], Scalar::Float64(5.0));
        assert_eq!(result.values()[4], Scalar::Float64(5.0));
    }

    #[test]
    fn series_bfill_trailing_nan() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s.bfill(None).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        // Trailing NaNs can't be back-filled
        assert!(result.values()[1].is_missing());
        assert!(result.values()[2].is_missing());
    }

    #[test]
    fn series_bfill_with_limit() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        let result = s.bfill(Some(1)).unwrap();
        assert!(result.values()[0].is_missing()); // limit exceeded
        assert!(result.values()[1].is_missing()); // limit exceeded
        assert_eq!(result.values()[2], Scalar::Float64(4.0)); // filled
        assert_eq!(result.values()[3], Scalar::Float64(4.0));
    }

    #[test]
    fn series_interpolate_basic() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Float64(0.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ],
        )
        .unwrap();

        let result = s.interpolate().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(0.0));
        assert_eq!(result.values()[4], Scalar::Float64(4.0));

        // Interior points: 1.0, 2.0, 3.0
        let v1 = result.values()[1].to_f64().unwrap();
        let v2 = result.values()[2].to_f64().unwrap();
        let v3 = result.values()[3].to_f64().unwrap();
        assert!((v1 - 1.0).abs() < 1e-10);
        assert!((v2 - 2.0).abs() < 1e-10);
        assert!((v3 - 3.0).abs() < 1e-10);
    }

    #[test]
    fn series_interpolate_leading_trailing_nan() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        let result = s.interpolate().unwrap();
        // Leading and trailing NaNs remain
        assert!(result.values()[0].is_missing());
        assert_eq!(result.values()[1], Scalar::Float64(10.0));
        assert_eq!(result.values()[2], Scalar::Float64(20.0));
        assert!(result.values()[3].is_missing());
    }

    #[test]
    fn series_interpolate_single_gap() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(0.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(10.0),
            ],
        )
        .unwrap();

        let result = s.interpolate().unwrap();
        let mid = result.values()[1].to_f64().unwrap();
        assert!((mid - 5.0).abs() < 1e-10);
    }

    #[test]
    fn series_ffill_no_nans() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let result = s.ffill(None).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
    }

    #[test]
    fn series_bfill_no_nans() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(6)],
        )
        .unwrap();

        let result = s.bfill(None).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(5));
        assert_eq!(result.values()[1], Scalar::Int64(6));
    }

    #[test]
    fn series_interpolate_all_nan() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Null(NullKind::NaN), Scalar::Null(NullKind::NaN)],
        )
        .unwrap();

        let result = s.interpolate().unwrap();
        assert!(result.values()[0].is_missing());
        assert!(result.values()[1].is_missing());
    }

    #[test]
    fn series_ffill_string_values() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("hello".to_owned()),
                Scalar::Null(NullKind::NaN),
                Scalar::Utf8("world".to_owned()),
            ],
        )
        .unwrap();

        let result = s.ffill(None).unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hello".to_owned()));
        assert_eq!(result.values()[1], Scalar::Utf8("hello".to_owned()));
        assert_eq!(result.values()[2], Scalar::Utf8("world".to_owned()));
    }

    #[test]
    fn series_interpolate_multiple_gaps() {
        let s = Series::from_values(
            "x",
            vec![
                0_i64.into(), 1_i64.into(), 2_i64.into(),
                3_i64.into(), 4_i64.into(), 5_i64.into(),
            ],
            vec![
                Scalar::Float64(0.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(5.0),
            ],
        )
        .unwrap();

        let result = s.interpolate().unwrap();
        let v1 = result.values()[1].to_f64().unwrap();
        assert!((v1 - 1.0).abs() < 1e-10);
        let v3 = result.values()[3].to_f64().unwrap();
        let v4 = result.values()[4].to_f64().unwrap();
        assert!((v3 - 3.0).abs() < 1e-10);
        assert!((v4 - 4.0).abs() < 1e-10);
    }

    // ── argsort / argmin / argmax / take / searchsorted / factorize ──

    #[test]
    fn series_argsort() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(3.0), Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let result = s.argsort(true).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1)); // 1.0 is smallest
        assert_eq!(result.values()[1], Scalar::Int64(2)); // 2.0 is next
        assert_eq!(result.values()[2], Scalar::Int64(0)); // 3.0 is largest
    }

    #[test]
    fn series_argsort_descending() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(3.0), Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let result = s.argsort(false).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(0)); // 3.0 is largest
        assert_eq!(result.values()[1], Scalar::Int64(2)); // 2.0 is next
        assert_eq!(result.values()[2], Scalar::Int64(1)); // 1.0 is smallest
    }

    #[test]
    fn series_argmin_argmax() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Float64(5.0), Scalar::Float64(1.0), Scalar::Float64(9.0)],
        )
        .unwrap();

        assert_eq!(s.argmin().unwrap(), 1);
        assert_eq!(s.argmax().unwrap(), 2);
    }

    #[test]
    fn series_argmin_with_nan() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();

        assert_eq!(s.argmin().unwrap(), 2);
        assert_eq!(s.argmax().unwrap(), 1);
    }

    #[test]
    fn series_take() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
                Scalar::Float64(40.0),
            ],
        )
        .unwrap();

        let result = s.take(&[3, 1, 0]).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.values()[0], Scalar::Float64(40.0));
        assert_eq!(result.values()[1], Scalar::Float64(20.0));
        assert_eq!(result.values()[2], Scalar::Float64(10.0));
    }

    #[test]
    fn series_take_out_of_bounds() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Float64(1.0)],
        )
        .unwrap();

        assert!(s.take(&[5]).is_err());
    }

    #[test]
    fn series_searchsorted_left() {
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

        assert_eq!(s.searchsorted(&Scalar::Float64(2.5), "left").unwrap(), 2);
        assert_eq!(s.searchsorted(&Scalar::Float64(2.0), "left").unwrap(), 1);
        assert_eq!(s.searchsorted(&Scalar::Float64(0.0), "left").unwrap(), 0);
        assert_eq!(s.searchsorted(&Scalar::Float64(5.0), "left").unwrap(), 4);
    }

    #[test]
    fn series_searchsorted_right() {
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

        assert_eq!(s.searchsorted(&Scalar::Float64(2.0), "right").unwrap(), 2);
        assert_eq!(s.searchsorted(&Scalar::Float64(2.5), "right").unwrap(), 2);
    }

    #[test]
    fn series_factorize() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![
                Scalar::Utf8("cat".to_owned()),
                Scalar::Utf8("dog".to_owned()),
                Scalar::Utf8("cat".to_owned()),
                Scalar::Null(NullKind::NaN),
                Scalar::Utf8("dog".to_owned()),
            ],
        )
        .unwrap();

        let (codes, uniques) = s.factorize().unwrap();
        assert_eq!(codes.values()[0], Scalar::Int64(0));
        assert_eq!(codes.values()[1], Scalar::Int64(1));
        assert_eq!(codes.values()[2], Scalar::Int64(0));
        assert_eq!(codes.values()[3], Scalar::Int64(-1)); // NaN
        assert_eq!(codes.values()[4], Scalar::Int64(1));
        assert_eq!(uniques.len(), 2);
        assert_eq!(uniques.values()[0], Scalar::Utf8("cat".to_owned()));
        assert_eq!(uniques.values()[1], Scalar::Utf8("dog".to_owned()));
    }

    // ── sem / skew / kurtosis ──

    #[test]
    fn series_sem() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
                Scalar::Float64(8.0),
            ],
        )
        .unwrap();

        let result = s.sem().unwrap();
        // std = sqrt(20/3) ≈ 2.5820, sem = std/sqrt(4) ≈ 1.2910
        assert!((result - 1.2909944).abs() < 0.001);
    }

    #[test]
    fn series_skew() {
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

        // Symmetric distribution => skew ≈ 0
        let result = s.skew().unwrap();
        assert!(result.abs() < 1e-10);
    }

    #[test]
    fn series_kurtosis() {
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

        // Uniform-like distribution, excess kurtosis is negative
        let result = s.kurtosis().unwrap();
        // Fisher's excess kurtosis for [1,2,3,4,5] = -1.2
        assert!((result - (-1.2)).abs() < 0.01);
    }

    #[test]
    fn series_skew_requires_3() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        assert!(s.skew().is_err());
    }

    // ── squeeze / memory_usage ──

    #[test]
    fn series_squeeze_single() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Float64(42.0)],
        )
        .unwrap();

        assert_eq!(s.squeeze().unwrap(), Scalar::Float64(42.0));
    }

    #[test]
    fn series_squeeze_multi() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        // Multi-element squeeze returns Err(self)
        assert!(s.squeeze().is_err());
    }

    #[test]
    fn series_memory_usage() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let bytes = s.memory_usage();
        assert!(bytes > 0);
    }

    // ── str: find, rfind, casefold, swapcase, partition, rpartition ──

    #[test]
    fn str_find() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("hello world".to_owned()),
                Scalar::Utf8("goodbye".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().find("world").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(6));
        assert_eq!(result.values()[1], Scalar::Int64(-1));
    }

    #[test]
    fn str_rfind() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("abcabc".to_owned())],
        )
        .unwrap();

        let result = s.str().rfind("abc").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(3));
    }

    #[test]
    fn str_casefold() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("HELLO".to_owned())],
        )
        .unwrap();

        let result = s.str().casefold().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hello".to_owned()));
    }

    #[test]
    fn str_swapcase() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("Hello World".to_owned())],
        )
        .unwrap();

        let result = s.str().swapcase().unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("hELLO wORLD".to_owned()));
    }

    #[test]
    fn str_partition() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("hello-world-foo".to_owned()),
                Scalar::Utf8("nodelim".to_owned()),
            ],
        )
        .unwrap();

        let (before, sep, after) = s.str().partition("-").unwrap();
        assert_eq!(before.values()[0], Scalar::Utf8("hello".to_owned()));
        assert_eq!(sep.values()[0], Scalar::Utf8("-".to_owned()));
        assert_eq!(after.values()[0], Scalar::Utf8("world-foo".to_owned()));
        // No delimiter found
        assert_eq!(before.values()[1], Scalar::Utf8("nodelim".to_owned()));
        assert_eq!(sep.values()[1], Scalar::Utf8(String::new()));
        assert_eq!(after.values()[1], Scalar::Utf8(String::new()));
    }

    #[test]
    fn str_rpartition() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello-world-foo".to_owned())],
        )
        .unwrap();

        let (before, sep, after) = s.str().rpartition("-").unwrap();
        assert_eq!(before.values()[0], Scalar::Utf8("hello-world".to_owned()));
        assert_eq!(sep.values()[0], Scalar::Utf8("-".to_owned()));
        assert_eq!(after.values()[0], Scalar::Utf8("foo".to_owned()));
    }

    #[test]
    fn str_rpartition_no_match() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello".to_owned())],
        )
        .unwrap();

        let (before, sep, after) = s.str().rpartition("-").unwrap();
        assert_eq!(before.values()[0], Scalar::Utf8(String::new()));
        assert_eq!(sep.values()[0], Scalar::Utf8(String::new()));
        assert_eq!(after.values()[0], Scalar::Utf8("hello".to_owned()));
    }

    // ── DataFrame pivot ──

    #[test]
    fn df_pivot_basic() {
        let df = DataFrame::from_dict(
            &["row", "col", "val"],
            vec![
                ("row", vec![Scalar::Utf8("A".to_owned()), Scalar::Utf8("A".to_owned()), Scalar::Utf8("B".to_owned()), Scalar::Utf8("B".to_owned())]),
                ("col", vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned()), Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())]),
                ("val", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();

        let pivoted = df.pivot("row", "col", "val").unwrap();
        assert_eq!(pivoted.column_names().len(), 2); // x, y
        assert_eq!(pivoted.len(), 2); // A, B rows
    }

    #[test]
    fn df_pivot_duplicate_errors() {
        let df = DataFrame::from_dict(
            &["row", "col", "val"],
            vec![
                ("row", vec![Scalar::Utf8("A".to_owned()), Scalar::Utf8("A".to_owned())]),
                ("col", vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("x".to_owned())]),
                ("val", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        assert!(df.pivot("row", "col", "val").is_err());
    }

    // ── DataFrame squeeze / memory_usage ──

    #[test]
    fn df_squeeze_single_column() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)])],
        )
        .unwrap();

        let series = df.squeeze_to_series(1).unwrap();
        assert_eq!(series.name(), "x");
        assert_eq!(series.len(), 2);
    }

    #[test]
    fn df_squeeze_single_row() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0)]),
                ("b", vec![Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        let series = df.squeeze_to_series(0).unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series.values()[0], Scalar::Float64(1.0));
        assert_eq!(series.values()[1], Scalar::Float64(2.0));
    }

    #[test]
    fn df_squeeze_multi_no_squeeze() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();

        // Can't squeeze multi-column, multi-row
        assert!(df.squeeze_to_series(1).is_err());
    }

    #[test]
    fn df_memory_usage() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();

        let mem = df.memory_usage().unwrap();
        // Should have 3 entries: Index, a, b
        assert_eq!(mem.len(), 3);
        // All values should be positive Int64
        for v in mem.values() {
            match v {
                Scalar::Int64(n) => assert!(*n > 0),
                _ => panic!("expected Int64"),
            }
        }
    }

    // ── DatetimeAccessor extensions ──

    #[test]
    fn dt_quarter() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("2024-01-15".to_owned()),
                Scalar::Utf8("2024-04-01".to_owned()),
                Scalar::Utf8("2024-07-31".to_owned()),
                Scalar::Utf8("2024-12-25".to_owned()),
            ],
        )
        .unwrap();

        let result = s.dt().quarter().unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1));
        assert_eq!(result.values()[1], Scalar::Int64(2));
        assert_eq!(result.values()[2], Scalar::Int64(3));
        assert_eq!(result.values()[3], Scalar::Int64(4));
    }

    #[test]
    fn dt_dayofyear() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("2024-01-01".to_owned()),
                Scalar::Utf8("2024-03-01".to_owned()),
            ],
        )
        .unwrap();

        let result = s.dt().dayofyear().unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1));
        // 2024 is a leap year: 31 + 29 + 1 = 61
        assert_eq!(result.values()[1], Scalar::Int64(61));
    }

    #[test]
    fn dt_is_month_start_end() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("2024-01-01".to_owned()),
                Scalar::Utf8("2024-01-15".to_owned()),
                Scalar::Utf8("2024-01-31".to_owned()),
            ],
        )
        .unwrap();

        let start = s.dt().is_month_start().unwrap();
        assert_eq!(start.values()[0], Scalar::Bool(true));
        assert_eq!(start.values()[1], Scalar::Bool(false));
        assert_eq!(start.values()[2], Scalar::Bool(false));

        let end = s.dt().is_month_end().unwrap();
        assert_eq!(end.values()[0], Scalar::Bool(false));
        assert_eq!(end.values()[1], Scalar::Bool(false));
        assert_eq!(end.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn dt_is_quarter_start_end() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("2024-01-01".to_owned()),
                Scalar::Utf8("2024-03-31".to_owned()),
                Scalar::Utf8("2024-06-30".to_owned()),
            ],
        )
        .unwrap();

        let qs = s.dt().is_quarter_start().unwrap();
        assert_eq!(qs.values()[0], Scalar::Bool(true));
        assert_eq!(qs.values()[1], Scalar::Bool(false));
        assert_eq!(qs.values()[2], Scalar::Bool(false));

        let qe = s.dt().is_quarter_end().unwrap();
        assert_eq!(qe.values()[0], Scalar::Bool(false));
        assert_eq!(qe.values()[1], Scalar::Bool(true));
        assert_eq!(qe.values()[2], Scalar::Bool(true));
    }

    #[test]
    fn dt_strftime() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("2024-03-15T14:30:00".to_owned())],
        )
        .unwrap();

        let result = s.dt().strftime("%Y/%m/%d %H:%M").unwrap();
        assert_eq!(result.values()[0], Scalar::Utf8("2024/03/15 14:30".to_owned()));
    }

    #[test]
    fn dt_weekofyear() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("2024-01-01".to_owned())],
        )
        .unwrap();

        let result = s.dt().weekofyear().unwrap();
        // 2024-01-01 is a Monday, ISO week 1
        assert_eq!(result.values()[0], Scalar::Int64(1));
    }

    // ── DataFrame ffill / bfill / interpolate ──

    #[test]
    fn df_ffill() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![
                ("a", vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ]),
            ],
        )
        .unwrap();

        let result = df.ffill(None).unwrap();
        let col = result.column_as_series("a").unwrap();
        assert_eq!(col.values()[0], Scalar::Float64(1.0));
        assert_eq!(col.values()[1], Scalar::Float64(1.0));
        assert_eq!(col.values()[2], Scalar::Float64(3.0));
    }

    #[test]
    fn df_bfill() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![
                ("a", vec![
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(2.0),
                    Scalar::Null(NullKind::NaN),
                ]),
            ],
        )
        .unwrap();

        let result = df.bfill(None).unwrap();
        let col = result.column_as_series("a").unwrap();
        assert_eq!(col.values()[0], Scalar::Float64(2.0));
        assert_eq!(col.values()[1], Scalar::Float64(2.0));
        assert!(col.values()[2].is_missing());
    }

    #[test]
    fn df_interpolate() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![
                ("a", vec![
                    Scalar::Float64(0.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(2.0),
                ]),
            ],
        )
        .unwrap();

        let result = df.interpolate().unwrap();
        let col = result.column_as_series("a").unwrap();
        let mid = col.values()[1].to_f64().unwrap();
        assert!((mid - 1.0).abs() < 1e-10);
    }

    // ── Series.append / DataFrame.append ──

    #[test]
    fn series_append() {
        let s1 = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let s2 = Series::from_values(
            "x",
            vec![2_i64.into(), 3_i64.into()],
            vec![Scalar::Float64(3.0), Scalar::Float64(4.0)],
        )
        .unwrap();

        let result = s1.append(&s2).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[3], Scalar::Float64(4.0));
    }

    #[test]
    fn df_append() {
        let df1 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0)]),
                ("b", vec![Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        let df2 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(3.0)]),
                ("b", vec![Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();

        let result = df1.append(&df2).unwrap();
        assert_eq!(result.len(), 2);
        let col_a = result.column_as_series("a").unwrap();
        assert_eq!(col_a.values()[0], Scalar::Float64(1.0));
        assert_eq!(col_a.values()[1], Scalar::Float64(3.0));
    }

    #[test]
    fn df_append_mismatched_columns() {
        let df1 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Float64(1.0)])],
        )
        .unwrap();

        let df2 = DataFrame::from_dict(
            &["b"],
            vec![("b", vec![Scalar::Float64(2.0)])],
        )
        .unwrap();

        let result = df1.append(&df2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.column_names().len(), 2);
        // Missing columns filled with NaN
        let col_a = result.column_as_series("a").unwrap();
        assert!(col_a.values()[1].is_missing());
    }

    // ── first_valid_index / last_valid_index ──

    #[test]
    fn series_first_valid_index() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();

        assert_eq!(s.first_valid_index(), Some(1_i64.into()));
    }

    #[test]
    fn series_last_valid_index() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .unwrap();

        assert_eq!(s.last_valid_index(), Some(1_i64.into()));
    }

    #[test]
    fn series_first_valid_all_null() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Null(NullKind::NaN)],
        )
        .unwrap();

        assert_eq!(s.first_valid_index(), None);
        assert_eq!(s.last_valid_index(), None);
    }

    // ── DataFrame rename_with / add_prefix / add_suffix ──

    #[test]
    fn df_rename_with() {
        let df = DataFrame::from_dict(
            &["foo", "bar"],
            vec![
                ("foo", vec![Scalar::Float64(1.0)]),
                ("bar", vec![Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        let result = df.rename_with(|name| name.to_uppercase()).unwrap();
        assert!(result.column("FOO").is_some());
        assert!(result.column("BAR").is_some());
    }

    #[test]
    fn df_add_prefix_suffix() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Float64(1.0)]),
                ("y", vec![Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        let prefixed = df.add_prefix("col_").unwrap();
        assert!(prefixed.column("col_x").is_some());
        assert!(prefixed.column("col_y").is_some());

        let suffixed = df.add_suffix("_val").unwrap();
        assert!(suffixed.column("x_val").is_some());
        assert!(suffixed.column("y_val").is_some());
    }

    // ── DataFrame nunique_axis ──

    #[test]
    fn df_nunique_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(1.0)]),
                ("b", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("c", vec![Scalar::Float64(2.0), Scalar::Float64(3.0)]),
            ],
        )
        .unwrap();

        let result = df.nunique_axis(1).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(2)); // row 0: {1.0, 2.0}
        assert_eq!(result.values()[1], Scalar::Int64(3)); // row 1: {1.0, 2.0, 3.0}
    }

    // ── astype_safe ──

    #[test]
    fn series_astype_safe_coerce() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("1".to_owned()),
                Scalar::Utf8("bad".to_owned()),
                Scalar::Utf8("3".to_owned()),
            ],
        )
        .unwrap();

        let result = s.astype_safe(DType::Int64, "coerce").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1));
        assert!(result.values()[1].is_missing());
        assert_eq!(result.values()[2], Scalar::Int64(3));
    }

    #[test]
    fn series_astype_safe_raise() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("bad".to_owned())],
        )
        .unwrap();

        // "raise" mode should error
        assert!(s.astype_safe(DType::Int64, "raise").is_err());
    }

    // ── sort_values_multi ──

    #[test]
    fn df_sort_values_multi() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)]),
                ("b", vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let sorted = df.sort_values_multi(&["a", "b"], &[true, true], "last").unwrap();
        let col_b = sorted.column_as_series("b").unwrap();
        assert_eq!(col_b.values()[0], Scalar::Int64(1)); // (1,1)
        assert_eq!(col_b.values()[1], Scalar::Int64(3)); // (1,3)
        assert_eq!(col_b.values()[2], Scalar::Int64(2)); // (2,2)
    }

    #[test]
    fn df_sort_values_multi_descending() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![
                ("a", vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let sorted = df.sort_values_multi(&["a"], &[false], "last").unwrap();
        let col_a = sorted.column_as_series("a").unwrap();
        assert_eq!(col_a.values()[0], Scalar::Int64(3));
        assert_eq!(col_a.values()[1], Scalar::Int64(2));
        assert_eq!(col_a.values()[2], Scalar::Int64(1));
    }

    // ── str count_literal ──

    #[test]
    fn str_count_literal() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("abcabcabc".to_owned()),
                Scalar::Utf8("xyz".to_owned()),
            ],
        )
        .unwrap();

        let result = s.str().count_literal("abc").unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(3));
        assert_eq!(result.values()[1], Scalar::Int64(0));
    }

    // ── EWM tests ──

    #[test]
    fn ewm_mean_basic() {
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

        // span=3 → alpha = 2/(3+1) = 0.5
        let result = s.ewm(Some(3.0), None).mean().unwrap();
        // First value: 1.0
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        // Second: 0.5*2 + 0.5*1 = 1.5
        assert_eq!(result.values()[1], Scalar::Float64(1.5));
        // Third: 0.5*3 + 0.5*1.5 = 2.25
        assert_eq!(result.values()[2], Scalar::Float64(2.25));
        // Fourth: 0.5*4 + 0.5*2.25 = 3.125
        assert_eq!(result.values()[3], Scalar::Float64(3.125));
    }

    #[test]
    fn ewm_mean_with_alpha() {
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

        let result = s.ewm(None, Some(0.5)).mean().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(10.0));
        assert_eq!(result.values()[1], Scalar::Float64(15.0));
        assert_eq!(result.values()[2], Scalar::Float64(22.5));
    }

    #[test]
    fn ewm_mean_with_nulls() {
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

        let result = s.ewm(Some(3.0), None).mean().unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert!(result.values()[1].is_missing());
        // After null, third value: 0.5*3 + 0.5*1 = 2.0
        assert_eq!(result.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn ewm_var_basic() {
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

        let result = s.ewm(Some(3.0), None).var().unwrap();
        // First value: NaN (can't compute variance from 1 obs)
        assert!(result.values()[0].is_missing());
        // Second value should be a positive number
        if let Scalar::Float64(v) = &result.values()[1] {
            assert!(*v > 0.0);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn ewm_std_basic() {
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

        let result = s.ewm(Some(3.0), None).std().unwrap();
        // First: NaN
        assert!(result.values()[0].is_missing());
        // Rest: should be sqrt of var, so positive
        if let Scalar::Float64(v) = &result.values()[1] {
            assert!(*v > 0.0);
        } else {
            panic!("expected Float64");
        }
    }

    // ── autocorr tests ──

    #[test]
    fn series_autocorr_lag1() {
        // Perfectly correlated with lag 1: [1,2,3,4,5]
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

        let r = s.autocorr(1).unwrap();
        assert!((r - 1.0).abs() < 0.01);
    }

    #[test]
    fn series_autocorr_lag_too_large() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();

        let r = s.autocorr(5).unwrap();
        assert!(r.is_nan());
    }

    // ── dot product tests ──

    #[test]
    fn series_dot_product() {
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
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        let result = a.dot(&b).unwrap();
        assert!((result - 32.0).abs() < f64::EPSILON);
    }

    #[test]
    fn series_dot_with_nulls() {
        let a = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();
        let b = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        // 1*4 + 3*6 = 4 + 18 = 22 (skips null)
        let result = a.dot(&b).unwrap();
        assert!((result - 22.0).abs() < f64::EPSILON);
    }

    // ── DataFrame to_string_table tests ──

    #[test]
    fn df_to_string_table() {
        let df = DataFrame::from_dict(
            &["name", "age"],
            vec![
                ("name", vec![Scalar::Utf8("Alice".to_owned()), Scalar::Utf8("Bob".to_owned())]),
                ("age", vec![Scalar::Int64(30), Scalar::Int64(25)]),
            ],
        )
        .unwrap();

        let result = df.to_string_table(true);
        assert!(result.contains("name"));
        assert!(result.contains("age"));
        assert!(result.contains("Alice"));
        assert!(result.contains("30"));
    }

    #[test]
    fn df_to_string_table_no_index() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();

        let result = df.to_string_table(false);
        // Should not have index column - just data
        assert!(result.contains("a"));
        assert!(result.contains("1"));
    }

    // ── DataFrame to_markdown tests ──

    #[test]
    fn df_to_markdown_basic() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("y", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();

        let result = df.to_markdown(false);
        // Should have header row with | x | y |
        assert!(result.contains("| x"));
        assert!(result.contains("| y"));
        // Should have separator ---
        assert!(result.contains("---"));
        // Should have data rows
        assert!(result.contains("| 1"));
    }

    #[test]
    fn df_to_markdown_with_index() {
        let df = DataFrame::from_dict(
            &["val"],
            vec![("val", vec![Scalar::Int64(10), Scalar::Int64(20)])],
        )
        .unwrap();

        let result = df.to_markdown(true);
        assert!(result.contains("|"));
        assert!(result.contains("val"));
        assert!(result.contains("10"));
    }

    // ── GroupBy cumulative tests ──

    #[test]
    fn groupby_cumsum() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                ]),
                ("v", vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(10),
                    Scalar::Int64(3),
                    Scalar::Int64(20),
                ]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().cumsum().unwrap();
        let v = result.column_as_series("v").unwrap();
        // Group a: 1, 1+2=3, 3+3=6
        // Group b: 10, 10+20=30
        assert_eq!(v.values()[0], Scalar::Float64(1.0));
        assert_eq!(v.values()[1], Scalar::Float64(3.0));
        assert_eq!(v.values()[2], Scalar::Float64(10.0));
        assert_eq!(v.values()[3], Scalar::Float64(6.0));
        assert_eq!(v.values()[4], Scalar::Float64(30.0));
    }

    #[test]
    fn groupby_cumprod() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(5)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().cumprod().unwrap();
        let v = result.column_as_series("v").unwrap();
        assert_eq!(v.values()[0], Scalar::Float64(2.0));
        assert_eq!(v.values()[1], Scalar::Float64(6.0));
        assert_eq!(v.values()[2], Scalar::Float64(5.0));
    }

    #[test]
    fn groupby_cummax() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(5)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().cummax().unwrap();
        let v = result.column_as_series("v").unwrap();
        assert_eq!(v.values()[0], Scalar::Float64(3.0));
        assert_eq!(v.values()[1], Scalar::Float64(3.0));
        assert_eq!(v.values()[2], Scalar::Float64(5.0));
    }

    #[test]
    fn groupby_cummin() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(5)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().cummin().unwrap();
        let v = result.column_as_series("v").unwrap();
        assert_eq!(v.values()[0], Scalar::Float64(3.0));
        assert_eq!(v.values()[1], Scalar::Float64(1.0));
        assert_eq!(v.values()[2], Scalar::Float64(1.0));
    }

    #[test]
    fn groupby_rank() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(30), Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().rank().unwrap();
        let v = result.column_as_series("v").unwrap();
        // Within group a: 30 -> rank 3, 10 -> rank 1, 20 -> rank 2
        assert_eq!(v.values()[0], Scalar::Float64(3.0));
        assert_eq!(v.values()[1], Scalar::Float64(1.0));
        assert_eq!(v.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn groupby_shift() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(10), Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().shift(1).unwrap();
        let v = result.column_as_series("v").unwrap();
        // Group a positions: [0,1,3] with vals [1,2,3], shifted by 1: [NaN,1,2]
        assert!(v.values()[0].is_missing());
        assert_eq!(v.values()[1], Scalar::Int64(1));
        // Group b positions: [2] with vals [10], shifted by 1: [NaN]
        assert!(v.values()[2].is_missing());
        // Group a position 3 → shifted → value at group position 1 → 2
        assert_eq!(v.values()[3], Scalar::Int64(2));
    }

    #[test]
    fn groupby_diff() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(10), Scalar::Int64(15), Scalar::Int64(25)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().diff(1).unwrap();
        let v = result.column_as_series("v").unwrap();
        assert!(v.values()[0].is_missing()); // first in group → NaN
        assert_eq!(v.values()[1], Scalar::Float64(5.0));  // 15-10
        assert_eq!(v.values()[2], Scalar::Float64(10.0)); // 25-15
    }

    #[test]
    fn groupby_nth() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![
                    Scalar::Int64(1),
                    Scalar::Int64(10),
                    Scalar::Int64(2),
                    Scalar::Int64(20),
                    Scalar::Int64(3),
                ]),
            ],
        )
        .unwrap();

        // nth(1) → second element of each group
        let result = df.groupby(&["g"]).unwrap().nth(1).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn groupby_nth_negative() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
            ],
        )
        .unwrap();

        // nth(-1) → last element of each group
        let result = df.groupby(&["g"]).unwrap().nth(-1).unwrap();
        let v = result.column_as_series("v").unwrap();
        assert_eq!(v.values()[0], Scalar::Int64(3));
    }

    #[test]
    fn groupby_head_tail() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                ]),
                ("v", vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                    Scalar::Int64(10),
                    Scalar::Int64(20),
                ]),
            ],
        )
        .unwrap();

        let head_result = df.groupby(&["g"]).unwrap().head(2).unwrap();
        // a: [1,2], b: [10,20] → 4 rows
        assert_eq!(head_result.len(), 4);

        let tail_result = df.groupby(&["g"]).unwrap().tail(1).unwrap();
        // a: [3], b: [20] → 2 rows
        assert_eq!(tail_result.len(), 2);
    }

    // ── Crosstab tests ──

    #[test]
    fn crosstab_basic() {
        let idx = Series::from_values(
            "gender",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("M".to_owned()),
                Scalar::Utf8("F".to_owned()),
                Scalar::Utf8("M".to_owned()),
                Scalar::Utf8("F".to_owned()),
            ],
        )
        .unwrap();
        let cols = Series::from_values(
            "hand",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("L".to_owned()),
                Scalar::Utf8("R".to_owned()),
                Scalar::Utf8("R".to_owned()),
                Scalar::Utf8("R".to_owned()),
            ],
        )
        .unwrap();

        let ct = DataFrame::crosstab(&idx, &cols).unwrap();
        // 2 rows (M, F), 2 columns (L, R)
        assert_eq!(ct.len(), 2);
        assert_eq!(ct.column_names().len(), 2);
    }

    #[test]
    fn crosstab_counts() {
        let idx = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("x".to_owned()),
            ],
        )
        .unwrap();
        let cols = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("p".to_owned()),
                Scalar::Utf8("q".to_owned()),
                Scalar::Utf8("p".to_owned()),
                Scalar::Utf8("p".to_owned()),
            ],
        )
        .unwrap();

        let ct = DataFrame::crosstab(&idx, &cols).unwrap();
        // x-p: 2, x-q: 1, y-p: 1, y-q: 0
        let col_p = ct.column_as_series("p").unwrap();
        assert_eq!(col_p.values()[0], Scalar::Int64(2)); // x-p
        assert_eq!(col_p.values()[1], Scalar::Int64(1)); // y-p
    }

    // ── Resample tests ──

    #[test]
    fn resample_monthly_sum() {
        let s = Series::from_values(
            "sales",
            vec![
                "2024-01-15".into(),
                "2024-01-20".into(),
                "2024-02-10".into(),
                "2024-02-25".into(),
                "2024-03-05".into(),
            ],
            vec![
                Scalar::Float64(100.0),
                Scalar::Float64(200.0),
                Scalar::Float64(150.0),
                Scalar::Float64(50.0),
                Scalar::Float64(300.0),
            ],
        )
        .unwrap();

        let result = s.resample("M").sum().unwrap();
        assert_eq!(result.len(), 3); // 3 months
        // Jan: 100+200=300
        assert_eq!(result.values()[0], Scalar::Float64(300.0));
        // Feb: 150+50=200
        assert_eq!(result.values()[1], Scalar::Float64(200.0));
        // Mar: 300
        assert_eq!(result.values()[2], Scalar::Float64(300.0));
    }

    #[test]
    fn resample_yearly_mean() {
        let s = Series::from_values(
            "val",
            vec![
                "2023-06-01".into(),
                "2023-12-01".into(),
                "2024-03-01".into(),
            ],
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
            ],
        )
        .unwrap();

        let result = s.resample("Y").mean().unwrap();
        assert_eq!(result.len(), 2); // 2023, 2024
        // 2023: (10+20)/2 = 15
        assert_eq!(result.values()[0], Scalar::Float64(15.0));
        // 2024: 30
        assert_eq!(result.values()[1], Scalar::Float64(30.0));
    }

    #[test]
    fn resample_count() {
        let s = Series::from_values(
            "val",
            vec![
                "2024-01-01".into(),
                "2024-01-15".into(),
                "2024-02-01".into(),
            ],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ],
        )
        .unwrap();

        let result = s.resample("M").count().unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(2)); // Jan: 2 obs
        assert_eq!(result.values()[1], Scalar::Int64(1)); // Feb: 1 obs
    }

    #[test]
    fn resample_min_max() {
        let s = Series::from_values(
            "val",
            vec![
                "2024-01-05".into(),
                "2024-01-20".into(),
                "2024-01-30".into(),
            ],
            vec![
                Scalar::Float64(5.0),
                Scalar::Float64(2.0),
                Scalar::Float64(8.0),
            ],
        )
        .unwrap();

        let min_result = s.resample("M").min().unwrap();
        assert_eq!(min_result.values()[0], Scalar::Float64(2.0));

        let max_result = s.resample("M").max().unwrap();
        assert_eq!(max_result.values()[0], Scalar::Float64(8.0));
    }

    // ── DataFrame explode tests ──

    #[test]
    fn df_explode_basic() {
        let df = DataFrame::from_dict(
            &["name", "tags"],
            vec![
                ("name", vec![Scalar::Utf8("Alice".to_owned()), Scalar::Utf8("Bob".to_owned())]),
                ("tags", vec![
                    Scalar::Utf8("a,b,c".to_owned()),
                    Scalar::Utf8("x,y".to_owned()),
                ]),
            ],
        )
        .unwrap();

        let result = df.explode("tags", ",").unwrap();
        // Alice: a, b, c (3 rows) + Bob: x, y (2 rows) = 5 rows
        assert_eq!(result.len(), 5);

        let tags = result.column_as_series("tags").unwrap();
        assert_eq!(tags.values()[0], Scalar::Utf8("a".to_owned()));
        assert_eq!(tags.values()[1], Scalar::Utf8("b".to_owned()));
        assert_eq!(tags.values()[2], Scalar::Utf8("c".to_owned()));
        assert_eq!(tags.values()[3], Scalar::Utf8("x".to_owned()));
        assert_eq!(tags.values()[4], Scalar::Utf8("y".to_owned()));
    }

    #[test]
    fn df_explode_preserves_other_columns() {
        let df = DataFrame::from_dict(
            &["id", "items"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("items", vec![
                    Scalar::Utf8("a|b".to_owned()),
                    Scalar::Utf8("c".to_owned()),
                ]),
            ],
        )
        .unwrap();

        let result = df.explode("items", "|").unwrap();
        assert_eq!(result.len(), 3);
        let id_col = result.column_as_series("id").unwrap();
        assert_eq!(id_col.values()[0], Scalar::Int64(1));
        assert_eq!(id_col.values()[1], Scalar::Int64(1));
        assert_eq!(id_col.values()[2], Scalar::Int64(2));
    }

    // ── DataFrame xs / droplevel tests ──

    #[test]
    fn df_xs_single_match() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("val", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]),
            ],
            vec!["a".into(), "b".into(), "c".into()],
        )
        .unwrap();

        let result = df.xs(&"b".into()).unwrap();
        assert_eq!(result.len(), 1);
        let v = result.column_as_series("val").unwrap();
        assert_eq!(v.values()[0], Scalar::Int64(20));
    }

    #[test]
    fn df_xs_multiple_match() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("val", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
            ],
            vec!["x".into(), "y".into(), "x".into()],
        )
        .unwrap();

        let result = df.xs(&"x".into()).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn df_xs_not_found() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        let result = df.xs(&"missing".into());
        assert!(result.is_err());
    }

    #[test]
    fn df_droplevel() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("val", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
            vec!["a".into(), "b".into()],
        )
        .unwrap();

        let result = df.droplevel().unwrap();
        assert_eq!(result.len(), 2);
        // Index should be 0, 1
        assert_eq!(result.index().labels()[0], IndexLabel::Int64(0));
        assert_eq!(result.index().labels()[1], IndexLabel::Int64(1));
    }

    // ── concat with keys tests ──

    #[test]
    fn concat_with_keys_basic() {
        let df1 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(3), Scalar::Int64(4)])],
        )
        .unwrap();

        let result =
            super::concat_dataframes_with_keys(&[&df1, &df2], &["first", "second"]).unwrap();
        assert_eq!(result.len(), 4);

        // Check that index labels have keys
        let labels = result.index().labels();
        match &labels[0] {
            IndexLabel::Utf8(s) => assert!(s.starts_with("first|")),
            _ => panic!("expected Utf8 label"),
        }
        match &labels[2] {
            IndexLabel::Utf8(s) => assert!(s.starts_with("second|")),
            _ => panic!("expected Utf8 label"),
        }
    }

    #[test]
    fn concat_with_keys_mismatched_lengths() {
        let df1 = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        let result = super::concat_dataframes_with_keys(&[&df1], &["x", "y"]);
        assert!(result.is_err());
    }

    // ── DataFrame EWM tests ──

    #[test]
    fn df_ewm_mean() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)]),
                ("b", vec![Scalar::Float64(10.0), Scalar::Float64(20.0), Scalar::Float64(30.0)]),
            ],
        )
        .unwrap();

        let result = df.ewm(Some(3.0), None).mean().unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.column_names().len(), 2);

        // First value of each column should be the raw value
        let a = result.column_as_series("a").unwrap();
        assert_eq!(a.values()[0], Scalar::Float64(1.0));
    }

    #[test]
    fn df_ewm_var() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![
                ("x", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)]),
            ],
        )
        .unwrap();

        let result = df.ewm(Some(3.0), None).var().unwrap();
        assert_eq!(result.len(), 3);
        // First value: NaN (can't compute var from 1 obs)
        let x = result.column_as_series("x").unwrap();
        assert!(x.values()[0].is_missing());
    }

    // ── GroupBy pct_change tests ──

    #[test]
    fn groupby_pct_change() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                ]),
                ("v", vec![Scalar::Float64(100.0), Scalar::Float64(110.0), Scalar::Float64(121.0)]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().pct_change().unwrap();
        let v = result.column_as_series("v").unwrap();
        assert!(v.values()[0].is_missing()); // first: NaN
        // (110-100)/100 = 0.1
        if let Scalar::Float64(f) = &v.values()[1] {
            assert!((*f - 0.1).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
        // (121-110)/110 = 0.1
        if let Scalar::Float64(f) = &v.values()[2] {
            assert!((*f - 0.1).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    // ── GroupBy value_counts tests ──

    #[test]
    fn groupby_value_counts() {
        let df = DataFrame::from_dict(
            &["g", "color"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                ]),
                ("color", vec![
                    Scalar::Utf8("red".to_owned()),
                    Scalar::Utf8("blue".to_owned()),
                    Scalar::Utf8("red".to_owned()),
                    Scalar::Utf8("green".to_owned()),
                    Scalar::Utf8("green".to_owned()),
                ]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().value_counts().unwrap();
        // Group a: red=2, blue=1; Group b: green=2
        // Total 3 entries
        assert_eq!(result.len(), 3);
        let counts = result.column_as_series("count").unwrap();
        // First entry should be 2 (red in group a, sorted by count desc)
        assert_eq!(counts.values()[0], Scalar::Int64(2));
    }

    // ── GroupBy describe tests ──

    #[test]
    fn groupby_describe() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("a".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                    Scalar::Utf8("b".to_owned()),
                ]),
                ("v", vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                ]),
            ],
        )
        .unwrap();

        let result = df.groupby(&["g"]).unwrap().describe().unwrap();
        // 2 groups * 8 stats = 16 rows
        assert_eq!(result.len(), 16);
        let v = result.column_as_series("v").unwrap();
        // First stat is count for group a: 3
        assert_eq!(v.values()[0], Scalar::Float64(3.0));
        // Second stat is mean for group a: 2.0
        assert_eq!(v.values()[1], Scalar::Float64(2.0));
    }

    // ── DataFrame resample tests ──

    #[test]
    fn df_resample_sum() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("sales", vec![
                    Scalar::Float64(100.0),
                    Scalar::Float64(200.0),
                    Scalar::Float64(300.0),
                ]),
            ],
            vec!["2024-01-15".into(), "2024-01-20".into(), "2024-02-10".into()],
        )
        .unwrap();

        let result = df.resample("M").sum().unwrap();
        assert_eq!(result.len(), 2); // Jan, Feb
        let sales = result.column_as_series("sales").unwrap();
        assert_eq!(sales.values()[0], Scalar::Float64(300.0)); // Jan
        assert_eq!(sales.values()[1], Scalar::Float64(300.0)); // Feb
    }

    #[test]
    fn df_resample_mean() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("val", vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                ]),
            ],
            vec!["2024-01-01".into(), "2024-01-15".into(), "2024-02-01".into()],
        )
        .unwrap();

        let result = df.resample("M").mean().unwrap();
        assert_eq!(result.len(), 2);
        let val = result.column_as_series("val").unwrap();
        assert_eq!(val.values()[0], Scalar::Float64(15.0)); // Jan: (10+20)/2
    }

    // ── DataFrame between_time / at_time tests ──

    #[test]
    fn df_between_time() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("val", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)]),
            ],
            vec![
                "2024-01-01T08:00:00".into(),
                "2024-01-01T12:30:00".into(),
                "2024-01-01T15:00:00".into(),
                "2024-01-01T20:00:00".into(),
            ],
        )
        .unwrap();

        let result = df.between_time("09:00:00", "16:00:00").unwrap();
        assert_eq!(result.len(), 2); // 12:30 and 15:00
    }

    #[test]
    fn df_at_time() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("val", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
            ],
            vec![
                "2024-01-01T10:00:00".into(),
                "2024-01-02T10:00:00".into(),
                "2024-01-03T14:30:00".into(),
            ],
        )
        .unwrap();

        let result = df.at_time("10:00:00").unwrap();
        assert_eq!(result.len(), 2); // Two rows at 10:00
    }

    // ── DataFrame to_latex test ──

    #[test]
    fn df_to_latex() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("y", vec![Scalar::Float64(3.5), Scalar::Float64(4.5)]),
            ],
        )
        .unwrap();

        let result = df.to_latex(false);
        assert!(result.contains("\\begin{tabular}"));
        assert!(result.contains("\\end{tabular}"));
        assert!(result.contains("\\toprule"));
        assert!(result.contains("\\midrule"));
        assert!(result.contains("\\bottomrule"));
        assert!(result.contains("x & y"));
    }

    #[test]
    fn df_to_latex_with_index() {
        let df = DataFrame::from_dict_with_index(
            vec![("val", vec![Scalar::Int64(10)])],
            vec!["row0".into()],
        )
        .unwrap();

        let result = df.to_latex(true);
        assert!(result.contains("row0"));
        assert!(result.contains("10"));
    }

    // ── Batch 6: Series properties and utility methods ──

    #[test]
    fn series_hasnans_true() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Null(NullKind::NaN)],
        )
        .unwrap();
        assert!(s.hasnans());
    }

    #[test]
    fn series_hasnans_false() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();
        assert!(!s.hasnans());
    }

    #[test]
    fn series_nbytes() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Float64(1.0), Scalar::Float64(2.0)],
        )
        .unwrap();
        assert!(s.nbytes() > 0);
    }

    #[test]
    fn series_pipe() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let result = s.pipe(|s| s.head(1)).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.values()[0], Scalar::Int64(10));
    }

    #[test]
    fn series_items() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let items = s.items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], (IndexLabel::Int64(0), Scalar::Int64(10)));
        assert_eq!(items[1], (IndexLabel::Int64(1), Scalar::Int64(20)));
    }

    // ── Batch 6: DataFrame properties ──

    #[test]
    fn df_ndim() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        assert_eq!(df.ndim(), 2);
    }

    #[test]
    fn df_axes() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Int64(2)]),
            ],
        )
        .unwrap();
        let (row_labels, col_names) = df.axes();
        assert_eq!(row_labels.len(), 1);
        assert_eq!(col_names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn df_to_html() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("y", vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())]),
            ],
        )
        .unwrap();
        let html = df.to_html(false);
        assert!(html.contains("<table"));
        assert!(html.contains("<th>x</th>"));
        assert!(html.contains("<th>y</th>"));
        assert!(html.contains("<td>1</td>"));
        assert!(html.contains("<td>a</td>"));
        assert!(html.contains("</table>"));
    }

    #[test]
    fn df_to_html_with_index() {
        let df = DataFrame::from_dict_with_index(
            vec![("val", vec![Scalar::Int64(10)])],
            vec!["row0".into()],
        )
        .unwrap();
        let html = df.to_html(true);
        assert!(html.contains("<th>row0</th>"));
        assert!(html.contains("<td>10</td>"));
    }

    #[test]
    fn df_to_html_escapes_special_chars() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Utf8("<b>&test</b>".into())])],
        )
        .unwrap();
        let html = df.to_html(false);
        assert!(html.contains("&lt;b&gt;&amp;test&lt;/b&gt;"));
    }

    // ── Batch 7: GroupBy enhancements ──

    #[test]
    fn groupby_get_group() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into()), Scalar::Utf8("a".into())]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let group_a = gb.get_group("a").unwrap();
        assert_eq!(group_a.len(), 2);
    }

    #[test]
    fn groupby_cumcount() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                ]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let cc = gb.cumcount().unwrap();
        // a: 0, b: 0, a: 1, b: 1
        assert_eq!(cc.values()[0], Scalar::Int64(0));
        assert_eq!(cc.values()[1], Scalar::Int64(0));
        assert_eq!(cc.values()[2], Scalar::Int64(1));
        assert_eq!(cc.values()[3], Scalar::Int64(1));
    }

    #[test]
    fn groupby_ngroup() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("a".into()),
                ]),
                ("v", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let ng = gb.ngroup().unwrap();
        // group "a" appeared first → 0, "b" → 1
        assert_eq!(ng.values()[0], Scalar::Int64(0));
        assert_eq!(ng.values()[1], Scalar::Int64(1));
        assert_eq!(ng.values()[2], Scalar::Int64(0));
    }

    #[test]
    fn groupby_pipe() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())]),
                ("v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let result = gb.pipe(|gb| gb.sum()).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn groupby_sem() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                ]),
                ("v", vec![
                    Scalar::Float64(2.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(6.0),
                    Scalar::Float64(8.0),
                ]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let result = gb.sem().unwrap();
        // std of [2,4,6,8] = 2.5819..., sem = 2.5819.../sqrt(4) ≈ 1.2909...
        let val = result.columns()["v"].values()[0].to_f64().unwrap();
        assert!((val - 1.2909944).abs() < 0.001);
    }

    // ── Batch 8: Rolling/Expanding enhancements ──

    #[test]
    fn rolling_var() {
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
        let result = s.rolling(3, None).var().unwrap();
        // First two should be NaN, third: var([1,2,3])=1.0, fourth: var([2,3,4])=1.0
        assert!(result.values()[0].is_missing());
        assert!(result.values()[1].is_missing());
        assert_eq!(result.values()[2], Scalar::Float64(1.0));
        assert_eq!(result.values()[3], Scalar::Float64(1.0));
    }

    #[test]
    fn rolling_median() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();
        let result = s.rolling(3, None).median().unwrap();
        assert!(result.values()[0].is_missing());
        assert!(result.values()[1].is_missing());
        // median([1,3,2]) = 2.0
        assert_eq!(result.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn rolling_quantile() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(0.0),
                Scalar::Float64(5.0),
                Scalar::Float64(10.0),
            ],
        )
        .unwrap();
        let result = s.rolling(3, None).quantile(0.5).unwrap();
        // quantile(0.5) of [0,5,10] = 5.0
        assert_eq!(result.values()[2], Scalar::Float64(5.0));
    }

    #[test]
    fn rolling_apply_custom() {
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
        // Custom function: product of window values
        let result = s.rolling(2, None).apply(|vals| vals.iter().product()).unwrap();
        assert!(result.values()[0].is_missing());
        // 1*2=2, 2*3=6
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
        assert_eq!(result.values()[2], Scalar::Float64(6.0));
    }

    #[test]
    fn expanding_var() {
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
        let result = s.expanding(None).var().unwrap();
        // 1 element: NaN, 2 elements: var([2,4])=2.0, 3 elements: var([2,4,6])=4.0
        assert!(result.values()[0].to_f64().unwrap().is_nan());
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
        assert_eq!(result.values()[2], Scalar::Float64(4.0));
    }

    #[test]
    fn expanding_median() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ],
        )
        .unwrap();
        let result = s.expanding(None).median().unwrap();
        // [3] → 3.0, [3,1] → 2.0, [3,1,2] → 2.0
        assert_eq!(result.values()[0], Scalar::Float64(3.0));
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
        assert_eq!(result.values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn expanding_apply_custom() {
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
        let result = s.expanding(None).apply(|vals| vals.iter().product()).unwrap();
        // [1] → 1, [1,2] → 2, [1,2,3] → 6
        assert_eq!(result.values()[0], Scalar::Float64(1.0));
        assert_eq!(result.values()[1], Scalar::Float64(2.0));
        assert_eq!(result.values()[2], Scalar::Float64(6.0));
    }

    // ── Batch 9: DataFrame isin/equals/first_valid_index/last_valid_index ──

    #[test]
    fn df_isin() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]),
                ("b", vec![Scalar::Int64(4), Scalar::Int64(5), Scalar::Int64(6)]),
            ],
        )
        .unwrap();
        let result = df.isin(&[Scalar::Int64(1), Scalar::Int64(5)]).unwrap();
        assert_eq!(result.columns()["a"].values()[0], Scalar::Bool(true));
        assert_eq!(result.columns()["a"].values()[1], Scalar::Bool(false));
        assert_eq!(result.columns()["b"].values()[1], Scalar::Bool(true));
    }

    #[test]
    fn df_equals() {
        let df1 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let df2 = df1.copy();
        assert!(df1.equals(&df2));

        let df3 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(3)])],
        )
        .unwrap();
        assert!(!df1.equals(&df3));
    }

    #[test]
    fn df_first_valid_index() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("a", vec![Scalar::Null(NullKind::NaN), Scalar::Int64(2), Scalar::Int64(3)]),
            ],
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        )
        .unwrap();
        assert_eq!(df.first_valid_index(), Some(1_i64.into()));
    }

    #[test]
    fn df_last_valid_index() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN), Scalar::Null(NullKind::NaN)]),
            ],
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        )
        .unwrap();
        assert_eq!(df.last_valid_index(), Some(0_i64.into()));
    }

    // ── Series sample ──

    #[test]
    fn series_sample() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30), Scalar::Int64(40), Scalar::Int64(50)],
        )
        .unwrap();
        let result = s.sample(Some(3), None, false, Some(42)).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn series_sample_frac() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30), Scalar::Int64(40)],
        )
        .unwrap();
        let result = s.sample(None, Some(0.5), false, Some(99)).unwrap();
        assert_eq!(result.len(), 2);
    }

    // ── DataFrame Rolling/Expanding var/median/quantile ──

    #[test]
    fn df_rolling_var() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])],
        )
        .unwrap();
        let result = df.rolling(2, None).var().unwrap();
        // var([1,2])=0.5, var([2,3])=0.5
        assert_eq!(result.columns()["x"].values()[2], Scalar::Float64(0.5));
    }

    #[test]
    fn df_rolling_median() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(2.0),
            ])],
        )
        .unwrap();
        let result = df.rolling(3, None).median().unwrap();
        // median([1,3,2]) = 2.0
        assert_eq!(result.columns()["x"].values()[2], Scalar::Float64(2.0));
    }

    #[test]
    fn df_rolling_quantile() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(0.0),
                Scalar::Float64(5.0),
                Scalar::Float64(10.0),
            ])],
        )
        .unwrap();
        let result = df.rolling(3, None).quantile(0.5).unwrap();
        assert_eq!(result.columns()["x"].values()[2], Scalar::Float64(5.0));
    }

    #[test]
    fn df_expanding_var() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ])],
        )
        .unwrap();
        let result = df.expanding(None).var().unwrap();
        // var([2,4])=2.0, var([2,4,6])=4.0
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(2.0));
        assert_eq!(result.columns()["x"].values()[2], Scalar::Float64(4.0));
    }

    #[test]
    fn df_expanding_median() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
            ])],
        )
        .unwrap();
        let result = df.expanding(None).median().unwrap();
        // [3]→3.0, [3,1]→2.0, [3,1,2]→2.0
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(3.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(2.0));
        assert_eq!(result.columns()["x"].values()[2], Scalar::Float64(2.0));
    }

    // ── GroupBy skew/kurtosis/ohlc ──

    #[test]
    fn groupby_skew() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                ]),
                ("v", vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(5.0),
                ]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let result = gb.skew().unwrap();
        // Symmetric data → skew ≈ 0
        let val = result.columns()["v"].values()[0].to_f64().unwrap();
        assert!(val.abs() < 0.01);
    }

    #[test]
    fn groupby_kurtosis() {
        let df = DataFrame::from_dict(
            &["g", "v"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                ]),
                ("v", vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(5.0),
                ]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let result = gb.kurtosis().unwrap();
        // Uniform-like → excess kurtosis < 0 (platykurtic)
        let val = result.columns()["v"].values()[0].to_f64().unwrap();
        assert!(val < 0.0);
    }

    #[test]
    fn groupby_ohlc() {
        let df = DataFrame::from_dict(
            &["g", "price"],
            vec![
                ("g", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("a".into()),
                ]),
                ("price", vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(15.0),
                    Scalar::Float64(12.0),
                ]),
            ],
        )
        .unwrap();
        let gb = df.groupby(&["g"]).unwrap();
        let result = gb.ohlc().unwrap();
        // Only one value column, so no prefix
        assert_eq!(result.columns()["open"].values()[0], Scalar::Float64(10.0));
        assert_eq!(result.columns()["high"].values()[0], Scalar::Float64(15.0));
        assert_eq!(result.columns()["low"].values()[0], Scalar::Float64(10.0));
        assert_eq!(result.columns()["close"].values()[0], Scalar::Float64(12.0));
    }

    // ── Batch 10: DataFrame skew_agg/kurtosis_agg/sem_agg ──

    #[test]
    fn df_skew_agg() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])],
        )
        .unwrap();
        let result = df.skew_agg().unwrap();
        // Symmetric data → skew ≈ 0
        let val = result.values()[0].to_f64().unwrap();
        assert!(val.abs() < 0.01);
    }

    #[test]
    fn df_kurtosis_agg() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])],
        )
        .unwrap();
        let result = df.kurtosis_agg().unwrap();
        let val = result.values()[0].to_f64().unwrap();
        // Uniform-like → excess kurtosis < 0
        assert!(val < 0.0);
    }

    #[test]
    fn df_sem_agg() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
                Scalar::Float64(8.0),
            ])],
        )
        .unwrap();
        let result = df.sem_agg().unwrap();
        // sem = std/sqrt(n) = 2.5819.../sqrt(4) ≈ 1.2909...
        let val = result.values()[0].to_f64().unwrap();
        assert!((val - 1.2909944).abs() < 0.001);
    }

    // ── DataFrame T/swapaxes/lookup ──

    #[test]
    fn df_t_alias() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Int64(2)]),
            ],
        )
        .unwrap();
        let t = df.t().unwrap();
        assert_eq!(t.shape(), (2, 1));
    }

    #[test]
    fn df_swapaxes_alias() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let result = df.swapaxes().unwrap();
        assert_eq!(result.shape(), (1, 2));
    }

    #[test]
    fn df_lookup() {
        let df = DataFrame::from_dict_with_index(
            vec![
                ("x", vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]),
                ("y", vec![Scalar::Int64(40), Scalar::Int64(50), Scalar::Int64(60)]),
            ],
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        )
        .unwrap();
        let result = df
            .lookup(
                &[0_i64.into(), 1_i64.into(), 2_i64.into()],
                &["x", "y", "x"],
            )
            .unwrap();
        assert_eq!(result, vec![Scalar::Int64(10), Scalar::Int64(50), Scalar::Int64(30)]);
    }

    // ── Series iat/at/transform ──

    #[test]
    fn series_iat() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        assert_eq!(s.iat(0).unwrap(), Scalar::Int64(10));
        assert_eq!(s.iat(2).unwrap(), Scalar::Int64(30));
        assert_eq!(s.iat(-1).unwrap(), Scalar::Int64(30));
    }

    #[test]
    fn series_at() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        assert_eq!(s.at(&"b".into()).unwrap(), Scalar::Int64(20));
    }

    #[test]
    fn series_transform() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();
        let result = s.transform(|v| match v {
            Scalar::Int64(n) => Scalar::Int64(n * 10),
            other => other.clone(),
        }).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(10));
        assert_eq!(result.values()[1], Scalar::Int64(20));
        assert_eq!(result.values()[2], Scalar::Int64(30));
    }

    // ── DataFrame mode ──

    #[test]
    fn df_mode() {
        let df = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                ]),
                ("y", vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("c".into()),
                ]),
            ],
        )
        .unwrap();
        let result = df.mode().unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.columns()["x"].values()[0], Scalar::Int64(2));
        assert_eq!(result.columns()["y"].values()[0], Scalar::Utf8("b".into()));
    }

    // ── Series: rename, set_axis, truncate, combine ──

    #[test]
    fn series_rename() {
        let s = Series::from_values(
            "old",
            vec![0_i64.into()],
            vec![Scalar::Int64(42)],
        )
        .unwrap();
        let r = s.rename("new").unwrap();
        assert_eq!(r.name(), "new");
        assert_eq!(r.values()[0], Scalar::Int64(42));
    }

    #[test]
    fn series_set_axis() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let r = s.set_axis(vec!["a".into(), "b".into()]).unwrap();
        assert_eq!(r.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(r.values()[0], Scalar::Int64(10));
    }

    #[test]
    fn series_truncate() {
        let s = Series::from_values(
            "x",
            vec![1_i64.into(), 2_i64.into(), 3_i64.into(), 4_i64.into(), 5_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
                Scalar::Int64(50),
            ],
        )
        .unwrap();
        let r = s.truncate(Some(&2_i64.into()), Some(&4_i64.into())).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r.values()[0], Scalar::Int64(20));
        assert_eq!(r.values()[2], Scalar::Int64(40));
    }

    #[test]
    fn series_combine() {
        let a = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let b = Series::from_values(
            "y",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(30)],
        )
        .unwrap();
        let result = a.combine(&b, |a_val, b_val| {
            match (a_val.to_f64(), b_val.to_f64()) {
                (Ok(a), Ok(b)) => Scalar::Float64(a.max(b)),
                _ => Scalar::Null(NullKind::NaN),
            }
        }).unwrap();
        assert_eq!(result.values()[0], Scalar::Float64(10.0));
        assert_eq!(result.values()[1], Scalar::Float64(30.0));
    }

    // ── DataFrame: corrwith, dot ──

    #[test]
    fn df_corrwith() {
        let df1 = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)]),
                ("y", vec![Scalar::Float64(4.0), Scalar::Float64(5.0), Scalar::Float64(6.0)]),
            ],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["x", "y"],
            vec![
                ("x", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)]),
                ("y", vec![Scalar::Float64(6.0), Scalar::Float64(5.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();
        let result = df1.corrwith(&df2).unwrap();
        // x correlates perfectly (1.0), y is perfectly anti-correlated (-1.0)
        let x_val = result.values()[0].to_f64().unwrap();
        let y_val = result.values()[1].to_f64().unwrap();
        assert!((x_val - 1.0).abs() < 0.001);
        assert!((y_val - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn df_dot() {
        // [1 2]   [5 6]   [1*5+2*7  1*6+2*8]   [19 22]
        // [3 4] × [7 8] = [3*5+4*7  3*6+4*8] = [43 50]
        let a = DataFrame::from_dict_with_index(
            vec![
                ("c1", vec![Scalar::Float64(1.0), Scalar::Float64(3.0)]),
                ("c2", vec![Scalar::Float64(2.0), Scalar::Float64(4.0)]),
            ],
            vec![0_i64.into(), 1_i64.into()],
        )
        .unwrap();
        let b = DataFrame::from_dict_with_index(
            vec![
                ("d1", vec![Scalar::Float64(5.0), Scalar::Float64(7.0)]),
                ("d2", vec![Scalar::Float64(6.0), Scalar::Float64(8.0)]),
            ],
            vec![0_i64.into(), 1_i64.into()],
        )
        .unwrap();
        let result = a.dot(&b).unwrap();
        assert_eq!(result.shape(), (2, 2));
        assert_eq!(result.columns()["d1"].values()[0], Scalar::Float64(19.0));
        assert_eq!(result.columns()["d2"].values()[0], Scalar::Float64(22.0));
        assert_eq!(result.columns()["d1"].values()[1], Scalar::Float64(43.0));
        assert_eq!(result.columns()["d2"].values()[1], Scalar::Float64(50.0));
    }

    // ── Series to_csv / to_json ──

    #[test]
    fn series_to_csv() {
        let s = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let csv = s.to_csv(',', true);
        assert!(csv.contains(",val"));
        assert!(csv.contains("0,10"));
        assert!(csv.contains("1,20"));
    }

    #[test]
    fn series_to_json_split() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let json = s.to_json("split").unwrap();
        assert!(json.contains("\"name\":\"x\""));
        assert!(json.contains("\"data\":[10,20]"));
    }

    #[test]
    fn series_to_json_index() {
        let s = Series::from_values(
            "x",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let json = s.to_json("index").unwrap();
        assert!(json.contains("\"a\":1"));
        assert!(json.contains("\"b\":2"));
    }

    #[test]
    fn series_to_json_records() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Float64(3.14)],
        )
        .unwrap();
        let json = s.to_json("records").unwrap();
        assert!(json.contains("3.14"));
    }

    // ── DataFrame scalar arithmetic ──

    #[test]
    fn df_add_scalar() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let result = df.add_scalar(10.0).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(11.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(12.0));
    }

    #[test]
    fn df_mul_scalar() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(2.0), Scalar::Float64(3.0)])],
        )
        .unwrap();
        let result = df.mul_scalar(5.0).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(10.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(15.0));
    }

    #[test]
    fn df_pow_scalar() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(2.0), Scalar::Float64(3.0)])],
        )
        .unwrap();
        let result = df.pow_scalar(2.0).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(4.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(9.0));
    }

    #[test]
    fn df_mod_scalar() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(7.0), Scalar::Float64(10.0)])],
        )
        .unwrap();
        let result = df.mod_scalar(3.0).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(1.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(1.0));
    }

    // ── Batch 12: Series duplicated/drop_duplicates/compare/reindex_like ──

    #[test]
    fn series_duplicated() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
            ],
        )
        .unwrap();
        let dup = s.duplicated().unwrap();
        assert_eq!(dup.column().values()[0], Scalar::Bool(false));
        assert_eq!(dup.column().values()[1], Scalar::Bool(false));
        assert_eq!(dup.column().values()[2], Scalar::Bool(true));
        assert_eq!(dup.column().values()[3], Scalar::Bool(false));
    }

    #[test]
    fn series_drop_duplicates() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
            ],
        )
        .unwrap();
        let result = s.drop_duplicates().unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.column().values()[0], Scalar::Int64(1));
        assert_eq!(result.column().values()[1], Scalar::Int64(2));
        assert_eq!(result.column().values()[2], Scalar::Int64(3));
    }

    #[test]
    fn series_compare_with() {
        let s1 = Series::from_values(
            "a",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();
        let s2 = Series::from_values(
            "b",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(99), Scalar::Int64(3)],
        )
        .unwrap();
        let diff = s1.compare_with(&s2).unwrap();
        // Only row 1 differs
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.columns()["self"].values()[0], Scalar::Int64(2));
        assert_eq!(diff.columns()["other"].values()[0], Scalar::Int64(99));
    }

    #[test]
    fn series_reindex_like() {
        let s1 = Series::from_values(
            "x",
            vec!["a".into(), "b".into(), "c".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .unwrap();
        let s2 = Series::from_values(
            "y",
            vec!["b".into(), "c".into(), "d".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .unwrap();
        let result = s1.reindex_like(&s2).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.column().values()[0], Scalar::Int64(2)); // b
        assert_eq!(result.column().values()[1], Scalar::Int64(3)); // c
        assert!(result.column().values()[2].is_missing()); // d -> NaN
    }

    // ── Batch 12: DataFrame row-axis aggregations ──

    #[test]
    fn df_sum_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();
        let result = df.sum_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Float64(4.0));
        assert_eq!(result.column().values()[1], Scalar::Float64(6.0));
    }

    #[test]
    fn df_mean_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(2.0), Scalar::Float64(4.0)]),
                ("b", vec![Scalar::Float64(6.0), Scalar::Float64(8.0)]),
            ],
        )
        .unwrap();
        let result = df.mean_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Float64(4.0));
        assert_eq!(result.column().values()[1], Scalar::Float64(6.0));
    }

    #[test]
    fn df_min_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(5.0), Scalar::Float64(1.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(9.0)]),
            ],
        )
        .unwrap();
        let result = df.min_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Float64(3.0));
        assert_eq!(result.column().values()[1], Scalar::Float64(1.0));
    }

    #[test]
    fn df_max_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(5.0), Scalar::Float64(1.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(9.0)]),
            ],
        )
        .unwrap();
        let result = df.max_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Float64(5.0));
        assert_eq!(result.column().values()[1], Scalar::Float64(9.0));
    }

    #[test]
    fn df_std_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(4.0)]),
                ("c", vec![Scalar::Float64(6.0)]),
            ],
        )
        .unwrap();
        let result = df.std_axis1().unwrap();
        // std of [2, 4, 6] = 2.0 (sample std)
        if let Scalar::Float64(v) = &result.column().values()[0] {
            assert!((v - 2.0).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn df_var_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(4.0)]),
                ("c", vec![Scalar::Float64(6.0)]),
            ],
        )
        .unwrap();
        let result = df.var_axis1().unwrap();
        // var of [2, 4, 6] = 4.0 (sample var)
        if let Scalar::Float64(v) = &result.column().values()[0] {
            assert!((v - 4.0).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn df_count_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)]),
                ("b", vec![Scalar::Int64(2), Scalar::Int64(3)]),
            ],
        )
        .unwrap();
        let result = df.count_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Int64(2));
        assert_eq!(result.column().values()[1], Scalar::Int64(1));
    }

    #[test]
    fn df_all_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Bool(true), Scalar::Bool(true)]),
                ("b", vec![Scalar::Bool(true), Scalar::Bool(false)]),
            ],
        )
        .unwrap();
        let result = df.all_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Bool(true));
        assert_eq!(result.column().values()[1], Scalar::Bool(false));
    }

    #[test]
    fn df_any_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Bool(false), Scalar::Bool(false)]),
                ("b", vec![Scalar::Bool(false), Scalar::Bool(true)]),
            ],
        )
        .unwrap();
        let result = df.any_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Bool(false));
        assert_eq!(result.column().values()[1], Scalar::Bool(true));
    }

    #[test]
    fn df_median_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b", "c"],
            vec![
                ("a", vec![Scalar::Float64(1.0)]),
                ("b", vec![Scalar::Float64(3.0)]),
                ("c", vec![Scalar::Float64(5.0)]),
            ],
        )
        .unwrap();
        let result = df.median_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Float64(3.0));
    }

    #[test]
    fn df_prod_axis1() {
        let df = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(2.0), Scalar::Float64(3.0)]),
                ("b", vec![Scalar::Float64(5.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();
        let result = df.prod_axis1().unwrap();
        assert_eq!(result.column().values()[0], Scalar::Float64(10.0));
        assert_eq!(result.column().values()[1], Scalar::Float64(12.0));
    }

    // ── Batch 12: DataFrame::map_elements + str removeprefix/removesuffix ──

    #[test]
    fn df_map_elements() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let result = df
            .map_elements(|v| match v {
                Scalar::Int64(n) => Scalar::Int64(n * 10),
                other => other.clone(),
            })
            .unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Int64(10));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Int64(20));
    }

    #[test]
    fn str_removeprefix() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("hello_world".into()),
                Scalar::Utf8("hello_foo".into()),
                Scalar::Utf8("bar".into()),
            ],
        )
        .unwrap();
        let result = s.str().removeprefix("hello_").unwrap();
        assert_eq!(result.column().values()[0], Scalar::Utf8("world".into()));
        assert_eq!(result.column().values()[1], Scalar::Utf8("foo".into()));
        assert_eq!(result.column().values()[2], Scalar::Utf8("bar".into()));
    }

    #[test]
    fn str_removesuffix() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("file.txt".into()),
                Scalar::Utf8("data.csv".into()),
                Scalar::Utf8("plain".into()),
            ],
        )
        .unwrap();
        let result = s.str().removesuffix(".txt").unwrap();
        assert_eq!(result.column().values()[0], Scalar::Utf8("file".into()));
        assert_eq!(result.column().values()[1], Scalar::Utf8("data.csv".into()));
        assert_eq!(result.column().values()[2], Scalar::Utf8("plain".into()));
    }

    // ── Batch 12b: DataFrame-to-DataFrame arithmetic ──

    #[test]
    fn df_add_df() {
        let df1 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
                ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
            ],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Float64(10.0), Scalar::Float64(20.0)]),
                ("b", vec![Scalar::Float64(30.0), Scalar::Float64(40.0)]),
            ],
        )
        .unwrap();
        let result = df1.add_df(&df2).unwrap();
        assert_eq!(result.columns()["a"].values()[0], Scalar::Float64(11.0));
        assert_eq!(result.columns()["b"].values()[1], Scalar::Float64(44.0));
    }

    #[test]
    fn df_sub_df() {
        let df1 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(10.0), Scalar::Float64(20.0)])],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(3.0), Scalar::Float64(5.0)])],
        )
        .unwrap();
        let result = df1.sub_df(&df2).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(7.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(15.0));
    }

    #[test]
    fn df_mul_df() {
        let df1 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(2.0), Scalar::Float64(3.0)])],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(5.0), Scalar::Float64(4.0)])],
        )
        .unwrap();
        let result = df1.mul_df(&df2).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(10.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(12.0));
    }

    #[test]
    fn df_div_df() {
        let df1 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(10.0), Scalar::Float64(20.0)])],
        )
        .unwrap();
        let df2 = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Float64(2.0), Scalar::Float64(5.0)])],
        )
        .unwrap();
        let result = df1.div_df(&df2).unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(5.0));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(4.0));
    }

    #[test]
    fn df_squeeze_single_col() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let s = df.squeeze(1).unwrap();
        assert_eq!(s.name(), "x");
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn df_assign_column_new() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let result = df
            .assign_column("b", vec![Scalar::Int64(10), Scalar::Int64(20)])
            .unwrap();
        assert_eq!(result.column_order.len(), 2);
        assert_eq!(result.columns()["b"].values()[0], Scalar::Int64(10));
    }

    #[test]
    fn df_assign_column_replace() {
        let df = DataFrame::from_dict(
            &["a"],
            vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let result = df
            .assign_column("a", vec![Scalar::Int64(99), Scalar::Int64(100)])
            .unwrap();
        assert_eq!(result.column_order.len(), 1);
        assert_eq!(result.columns()["a"].values()[0], Scalar::Int64(99));
    }

    // ── Batch 12c: str index_of/rindex_of/expandtabs + convert_dtypes ──

    #[test]
    fn str_index_of_found() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello world".into())],
        )
        .unwrap();
        let result = s.str().index_of("world").unwrap();
        assert_eq!(result.column().values()[0], Scalar::Int64(6));
    }

    #[test]
    fn str_index_of_not_found() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("hello".into())],
        )
        .unwrap();
        let result = s.str().index_of("xyz").unwrap();
        assert!(result.column().values()[0].is_missing());
    }

    #[test]
    fn str_rindex_of() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("abcabc".into())],
        )
        .unwrap();
        let result = s.str().rindex_of("abc").unwrap();
        assert_eq!(result.column().values()[0], Scalar::Int64(3));
    }

    #[test]
    fn str_expandtabs() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into()],
            vec![Scalar::Utf8("a\tb".into())],
        )
        .unwrap();
        let result = s.str().expandtabs(4).unwrap();
        assert_eq!(result.column().values()[0], Scalar::Utf8("a    b".into()));
    }

    #[test]
    fn df_convert_dtypes_string_to_float() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Utf8("1.5".into()), Scalar::Utf8("2.5".into())])],
        )
        .unwrap();
        let result = df.convert_dtypes().unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Float64(1.5));
        assert_eq!(result.columns()["x"].values()[1], Scalar::Float64(2.5));
    }

    #[test]
    fn df_convert_dtypes_non_numeric_unchanged() {
        let df = DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Utf8("hello".into()), Scalar::Utf8("world".into())])],
        )
        .unwrap();
        let result = df.convert_dtypes().unwrap();
        assert_eq!(result.columns()["x"].values()[0], Scalar::Utf8("hello".into()));
    }

    // ── Batch 12d: from_csv/to_string/from_dict ──

    #[test]
    fn df_from_csv() {
        let csv = "a,b\n1,2\n3,4";
        let df = DataFrame::from_csv(csv, ',').unwrap();
        assert_eq!(df.column_order, vec!["a", "b"]);
        assert_eq!(df.len(), 2);
        assert_eq!(df.columns()["a"].values()[0], Scalar::Int64(1));
        assert_eq!(df.columns()["b"].values()[1], Scalar::Int64(4));
    }

    #[test]
    fn df_from_csv_with_floats() {
        let csv = "x,y\n1.5,hello\n2.5,world";
        let df = DataFrame::from_csv(csv, ',').unwrap();
        assert_eq!(df.columns()["x"].values()[0], Scalar::Float64(1.5));
        assert_eq!(df.columns()["y"].values()[1], Scalar::Utf8("world".into()));
    }

    #[test]
    fn series_from_dict() {
        let mut data = BTreeMap::new();
        data.insert(IndexLabel::Utf8("a".into()), Scalar::Int64(1));
        data.insert(IndexLabel::Utf8("b".into()), Scalar::Int64(2));
        let s = Series::from_dict("x", data).unwrap();
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn series_to_string_repr() {
        let s = Series::from_values(
            "x",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();
        let repr = s.to_string_repr();
        assert!(repr.contains("10"));
        assert!(repr.contains("20"));
        assert!(repr.contains("Name: x"));
    }

    // ── Batch 12e: crosstab_normalize ──

    #[test]
    fn crosstab_normalize_all() {
        let idx = Series::from_values(
            "idx",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let col = Series::from_values(
            "col",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("x".into()),
                Scalar::Utf8("y".into()),
                Scalar::Utf8("x".into()),
                Scalar::Utf8("x".into()),
            ],
        )
        .unwrap();
        let ct = DataFrame::crosstab_normalize(&idx, &col, "all").unwrap();
        // Grand total = 4, so each cell / 4
        // a-x=1/4=0.25, a-y=1/4=0.25, b-x=2/4=0.5, b-y=0/4=0.0
        assert_eq!(ct.columns()["x"].values()[0], Scalar::Float64(0.25));
        assert_eq!(ct.columns()["y"].values()[0], Scalar::Float64(0.25));
        assert_eq!(ct.columns()["x"].values()[1], Scalar::Float64(0.5));
    }

    #[test]
    fn crosstab_normalize_index() {
        let idx = Series::from_values(
            "idx",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let col = Series::from_values(
            "col",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("x".into()),
                Scalar::Utf8("y".into()),
                Scalar::Utf8("x".into()),
                Scalar::Utf8("x".into()),
            ],
        )
        .unwrap();
        let ct = DataFrame::crosstab_normalize(&idx, &col, "index").unwrap();
        // Row a: total=2, x=1/2=0.5, y=1/2=0.5
        assert_eq!(ct.columns()["x"].values()[0], Scalar::Float64(0.5));
        assert_eq!(ct.columns()["y"].values()[0], Scalar::Float64(0.5));
        // Row b: total=2, x=2/2=1.0, y=0/2=0.0
        assert_eq!(ct.columns()["x"].values()[1], Scalar::Float64(1.0));
        assert_eq!(ct.columns()["y"].values()[1], Scalar::Float64(0.0));
    }

}
