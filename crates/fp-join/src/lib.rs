#![forbid(unsafe_code)]

//! Merge / join engine for **frankenpandas** — implements
//! pandas-shape `pd.merge` / `pd.merge_ordered` / `pd.merge_asof`
//! / `Series.join` for DataFrame and Series operands.
//!
//! ## Why a separate crate
//!
//! Joins have substantial machinery — hash-build phases, asof
//! search, validate-mode integrity checks, bumpalo-backed
//! intermediate buffers — that doesn't belong inside fp-frame's
//! row/column primitives. fp-join layers on top of fp-columnar /
//! fp-index / fp-frame and exposes a merge surface matching the
//! pandas API shape.
//!
//! ## Top-level entry points
//!
//! - [`join_series`] / [`join_series_with_options`]: pandas
//!   `Series.join(other, how=...)`. Returns a [`JoinedSeries`].
//! - [`merge_dataframes`]: index-on-index merge (pandas
//!   `df.merge(other, left_index=True, right_index=True)`).
//! - [`merge_dataframes_on`] / [`merge_dataframes_on_with`] /
//!   [`merge_dataframes_on_with_options`]: column-key merge
//!   (`pd.merge(left, right, on=...)`). Returns a
//!   [`MergedDataFrame`].
//! - [`merge_ordered`]: pandas `pd.merge_ordered(left, right,
//!   on=...)` — preserves ordering for time-series merges.
//! - [`merge_asof`] / [`merge_asof_with_options`]: pandas
//!   `pd.merge_asof(left, right, on=..., direction=...)` for
//!   nearest-key time-aware merges. [`AsofDirection`] /
//!   [`MergeAsofOptions`] tune the search.
//!
//! ## DataFrame extension trait
//!
//! [`DataFrameMergeExt`] adds `df.merge(...)` / `df.join(...)`
//! method-style entry points so callers get fluent chaining
//! after `use fp_join::DataFrameMergeExt;`.
//!
//! ## Tunables
//!
//! - [`JoinType`]: `Inner` / `Left` / `Right` / `Outer` /
//!   `Cross`.
//! - [`MergeValidateMode`]: pandas `validate='one_to_one' |
//!   'one_to_many' | 'many_to_one' | 'many_to_many'` integrity
//!   check before producing the result.
//! - [`JoinExecutionOptions`] / [`MergeExecutionOptions`]:
//!   per-call knobs (suffixes for overlapping columns, indicator
//!   column, sort policy, ...).
//!
//! ## Error reporting
//!
//! [`JoinError`] enumerates the failure modes (key column
//! mismatch, validate-mode violation, dtype mismatch on key
//! column, ...).
//!
//! ## Cross-crate relationships
//!
//! - **fp-columnar** ([`Column`], [`ColumnError`]) for column
//!   storage operations.
//! - **fp-frame** for the `DataFrame` / `Series` value types.
//! - **fp-index** for row alignment plans + label types.
//! - **fp-types** for the underlying scalar machinery.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    mem::size_of,
};

use bumpalo::{Bump, collections::Vec as BumpVec};
use fp_columnar::{Column, ColumnError};
use fp_frame::{FrameError, Series};
use fp_index::{Index, IndexLabel};
use fp_types::TypeError;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Outer,
    Cross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeValidateMode {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
}

impl MergeValidateMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::OneToOne => "one_to_one",
            Self::OneToMany => "one_to_many",
            Self::ManyToOne => "many_to_one",
            Self::ManyToMany => "many_to_many",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinedSeries {
    pub index: Index,
    pub left_values: Column,
    pub right_values: Column,
}

#[derive(Debug, Error)]
pub enum JoinError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Column(#[from] ColumnError),
}

pub const DEFAULT_ARENA_BUDGET_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinExecutionOptions {
    pub use_arena: bool,
    pub arena_budget_bytes: usize,
}

impl Default for JoinExecutionOptions {
    fn default() -> Self {
        Self {
            use_arena: true,
            arena_budget_bytes: DEFAULT_ARENA_BUDGET_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeExecutionOptions {
    pub indicator_name: Option<String>,
    pub validate_mode: Option<MergeValidateMode>,
    pub suffixes: Option<[Option<String>; 2]>,
    pub sort: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JoinExecutionTrace {
    used_arena: bool,
    output_rows: usize,
    estimated_bytes: usize,
}

pub fn join_series(
    left: &Series,
    right: &Series,
    join_type: JoinType,
) -> Result<JoinedSeries, JoinError> {
    join_series_with_options(left, right, join_type, JoinExecutionOptions::default())
}

pub fn join_series_with_options(
    left: &Series,
    right: &Series,
    join_type: JoinType,
    options: JoinExecutionOptions,
) -> Result<JoinedSeries, JoinError> {
    let (joined, _) = join_series_with_trace(left, right, join_type, options)?;
    Ok(joined)
}

fn join_series_with_trace(
    left: &Series,
    right: &Series,
    join_type: JoinType,
    options: JoinExecutionOptions,
) -> Result<(JoinedSeries, JoinExecutionTrace), JoinError> {
    // AG-02: borrowed-key HashMap eliminates right-index label clones during build phase.
    let mut right_map = HashMap::<&IndexLabel, Vec<usize>>::new();
    for (pos, label) in right.index().labels().iter().enumerate() {
        right_map.entry(label).or_default().push(pos);
    }

    // For Right/Outer joins, also build a left_map.
    let left_map = if matches!(join_type, JoinType::Right | JoinType::Outer) {
        let mut m = HashMap::<&IndexLabel, Vec<usize>>::new();
        for (pos, label) in left.index().labels().iter().enumerate() {
            m.entry(label).or_default().push(pos);
        }
        Some(m)
    } else {
        None
    };

    let output_rows = estimate_output_rows(left, right, &right_map, left_map.as_ref(), join_type);
    let estimated_bytes = estimate_intermediate_bytes(output_rows);
    let use_arena = options.use_arena && estimated_bytes <= options.arena_budget_bytes;

    let joined = if use_arena {
        join_series_with_arena(
            left,
            right,
            join_type,
            &right_map,
            left_map.as_ref(),
            output_rows,
        )?
    } else {
        join_series_with_global_allocator(
            left,
            right,
            join_type,
            &right_map,
            left_map.as_ref(),
            output_rows,
        )?
    };

    Ok((
        joined,
        JoinExecutionTrace {
            used_arena: use_arena,
            output_rows,
            estimated_bytes,
        },
    ))
}

fn estimate_output_rows(
    left: &Series,
    right: &Series,
    right_map: &HashMap<&IndexLabel, Vec<usize>>,
    left_map: Option<&HashMap<&IndexLabel, Vec<usize>>>,
    join_type: JoinType,
) -> usize {
    let left_matched: usize = left
        .index()
        .labels()
        .iter()
        .map(|label| match right_map.get(label) {
            Some(matches) => matches.len(),
            None if matches!(join_type, JoinType::Left | JoinType::Outer) => 1,
            None => 0,
        })
        .sum();

    match join_type {
        JoinType::Inner | JoinType::Left => left_matched,
        JoinType::Right => {
            // All right labels, with matches from left
            right
                .index()
                .labels()
                .iter()
                .map(|label| match left_map.as_ref().and_then(|m| m.get(label)) {
                    Some(matches) => matches.len(),
                    None => 1,
                })
                .sum()
        }
        JoinType::Cross => left
            .index()
            .labels()
            .len()
            .saturating_mul(right.index().labels().len()),
        JoinType::Outer => {
            // Left-matched rows + unmatched right rows
            let right_unmatched: usize = right
                .index()
                .labels()
                .iter()
                .filter(|label| {
                    !left_map
                        .as_ref()
                        .expect("Outer needs left_map")
                        .contains_key(*label)
                })
                .count();
            left_matched + right_unmatched
        }
    }
}

fn estimate_intermediate_bytes(output_rows: usize) -> usize {
    output_rows.saturating_mul(
        size_of::<Option<usize>>()
            .saturating_mul(2)
            .saturating_add(size_of::<IndexLabel>()),
    )
}

fn reindex_outer_join_column(
    column: &Column,
    positions: &[Option<usize>],
) -> Result<Column, JoinError> {
    if !matches!(
        column.dtype(),
        fp_types::DType::Int64 | fp_types::DType::Float64
    ) {
        return Ok(column.reindex_by_positions(positions)?);
    }

    let has_missing = positions
        .iter()
        .any(|slot| slot.is_none_or(|idx| idx >= column.len()));
    if !has_missing {
        return Ok(column.reindex_by_positions(positions)?);
    }

    let values = positions
        .iter()
        .map(|slot| match slot {
            Some(idx) => column
                .values()
                .get(*idx)
                .cloned()
                .unwrap_or(fp_types::Scalar::Null(fp_types::NullKind::NaN)),
            None => fp_types::Scalar::Null(fp_types::NullKind::NaN),
        })
        .map(|value| {
            fp_types::cast_scalar_owned(value, fp_types::DType::Float64).map_err(ColumnError::from)
        })
        .collect::<Result<Vec<_>, ColumnError>>()?;

    Ok(Column::new(fp_types::DType::Float64, values)?)
}

fn sort_outer_join_rows(
    out_labels: &mut Vec<IndexLabel>,
    left_positions: &mut [Option<usize>],
    right_positions: &mut [Option<usize>],
) {
    if out_labels.len() <= 1 {
        return;
    }

    let mut rows = out_labels
        .drain(..)
        .zip(left_positions.iter().copied())
        .zip(right_positions.iter().copied())
        .map(|((label, left_pos), right_pos)| (label, left_pos, right_pos))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(&right.0));

    for (idx, (label, left_pos, right_pos)) in rows.into_iter().enumerate() {
        out_labels.push(label);
        left_positions[idx] = left_pos;
        right_positions[idx] = right_pos;
    }
}

fn join_series_with_global_allocator(
    left: &Series,
    right: &Series,
    join_type: JoinType,
    right_map: &HashMap<&IndexLabel, Vec<usize>>,
    left_map: Option<&HashMap<&IndexLabel, Vec<usize>>>,
    output_rows: usize,
) -> Result<JoinedSeries, JoinError> {
    let mut out_labels = Vec::with_capacity(output_rows);
    let mut left_positions = Vec::<Option<usize>>::with_capacity(output_rows);
    let mut right_positions = Vec::<Option<usize>>::with_capacity(output_rows);

    match join_type {
        JoinType::Inner | JoinType::Left | JoinType::Outer => {
            // Iterate left labels: emit matches and (for Left/Outer) unmatched left rows.
            for (left_pos, label) in left.index().labels().iter().enumerate() {
                if let Some(matches) = right_map.get(label) {
                    for right_pos in matches {
                        out_labels.push(label.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(*right_pos));
                    }
                    continue;
                }

                if matches!(join_type, JoinType::Left | JoinType::Outer) {
                    out_labels.push(label.clone());
                    left_positions.push(Some(left_pos));
                    right_positions.push(None);
                }
            }

            // For Outer: append right labels that have no match in left.
            if matches!(join_type, JoinType::Outer) {
                let left_map = left_map.as_ref().expect("left_map required for Outer join");
                for (right_pos, label) in right.index().labels().iter().enumerate() {
                    if !left_map.contains_key(label) {
                        out_labels.push(label.clone());
                        left_positions.push(None);
                        right_positions.push(Some(right_pos));
                    }
                }
            }
        }
        JoinType::Right => {
            // Iterate right labels: emit matches and unmatched right rows.
            let left_map = left_map.expect("left_map required for Right join");
            for (right_pos, label) in right.index().labels().iter().enumerate() {
                if let Some(matches) = left_map.get(label) {
                    for left_pos in matches {
                        out_labels.push(label.clone());
                        left_positions.push(Some(*left_pos));
                        right_positions.push(Some(right_pos));
                    }
                    continue;
                }

                out_labels.push(label.clone());
                left_positions.push(None);
                right_positions.push(Some(right_pos));
            }
        }
        JoinType::Cross => {
            for (left_pos, left_label) in left.index().labels().iter().enumerate() {
                for (right_pos, _) in right.index().labels().iter().enumerate() {
                    out_labels.push(left_label.clone());
                    left_positions.push(Some(left_pos));
                    right_positions.push(Some(right_pos));
                }
            }
        }
    }

    if matches!(join_type, JoinType::Outer) {
        sort_outer_join_rows(&mut out_labels, &mut left_positions, &mut right_positions);
    }

    let left_values = if matches!(join_type, JoinType::Outer) {
        reindex_outer_join_column(left.column(), &left_positions)?
    } else {
        left.column().reindex_by_positions(&left_positions)?
    };
    let right_values = if matches!(join_type, JoinType::Outer) {
        reindex_outer_join_column(right.column(), &right_positions)?
    } else {
        right.column().reindex_by_positions(&right_positions)?
    };

    Ok(JoinedSeries {
        index: Index::new(out_labels),
        left_values,
        right_values,
    })
}

fn join_series_with_arena(
    left: &Series,
    right: &Series,
    join_type: JoinType,
    right_map: &HashMap<&IndexLabel, Vec<usize>>,
    left_map: Option<&HashMap<&IndexLabel, Vec<usize>>>,
    output_rows: usize,
) -> Result<JoinedSeries, JoinError> {
    let arena = Bump::new();
    let mut out_labels = Vec::with_capacity(output_rows);
    let mut left_positions = BumpVec::<Option<usize>>::with_capacity_in(output_rows, &arena);
    let mut right_positions = BumpVec::<Option<usize>>::with_capacity_in(output_rows, &arena);

    match join_type {
        JoinType::Inner | JoinType::Left | JoinType::Outer => {
            for (left_pos, label) in left.index().labels().iter().enumerate() {
                if let Some(matches) = right_map.get(label) {
                    for right_pos in matches {
                        out_labels.push(label.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(*right_pos));
                    }
                    continue;
                }

                if matches!(join_type, JoinType::Left | JoinType::Outer) {
                    out_labels.push(label.clone());
                    left_positions.push(Some(left_pos));
                    right_positions.push(None);
                }
            }

            if matches!(join_type, JoinType::Outer) {
                let left_map = left_map.as_ref().expect("left_map required for Outer join");
                for (right_pos, label) in right.index().labels().iter().enumerate() {
                    if !left_map.contains_key(label) {
                        out_labels.push(label.clone());
                        left_positions.push(None);
                        right_positions.push(Some(right_pos));
                    }
                }
            }
        }
        JoinType::Right => {
            let left_map = left_map.expect("left_map required for Right join");
            for (right_pos, label) in right.index().labels().iter().enumerate() {
                if let Some(matches) = left_map.get(label) {
                    for left_pos in matches {
                        out_labels.push(label.clone());
                        left_positions.push(Some(*left_pos));
                        right_positions.push(Some(right_pos));
                    }
                    continue;
                }

                out_labels.push(label.clone());
                left_positions.push(None);
                right_positions.push(Some(right_pos));
            }
        }
        JoinType::Cross => {
            for (left_pos, left_label) in left.index().labels().iter().enumerate() {
                for (right_pos, _) in right.index().labels().iter().enumerate() {
                    out_labels.push(left_label.clone());
                    left_positions.push(Some(left_pos));
                    right_positions.push(Some(right_pos));
                }
            }
        }
    }

    if matches!(join_type, JoinType::Outer) {
        sort_outer_join_rows(
            &mut out_labels,
            left_positions.as_mut_slice(),
            right_positions.as_mut_slice(),
        );
    }

    let left_values = if matches!(join_type, JoinType::Outer) {
        reindex_outer_join_column(left.column(), left_positions.as_slice())?
    } else {
        left.column()
            .reindex_by_positions(left_positions.as_slice())?
    };
    let right_values = if matches!(join_type, JoinType::Outer) {
        reindex_outer_join_column(right.column(), right_positions.as_slice())?
    } else {
        right
            .column()
            .reindex_by_positions(right_positions.as_slice())?
    };

    Ok(JoinedSeries {
        index: Index::new(out_labels),
        left_values,
        right_values,
    })
}

/// Result of a DataFrame merge operation.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedDataFrame {
    pub index: Index,
    pub columns: std::collections::BTreeMap<String, Column>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum JoinKeyComponent {
    Present(IndexLabel),
    FloatBits(u64),
    Missing,
}

impl Ord for JoinKeyComponent {
    fn cmp(&self, other: &Self) -> Ordering {
        use JoinKeyComponent::{FloatBits, Missing, Present};
        match (self, other) {
            (Missing, Missing) => Ordering::Equal,
            (Missing, _) => Ordering::Greater,
            (_, Missing) => Ordering::Less,
            (Present(IndexLabel::Int64(a)), Present(IndexLabel::Int64(b))) => a.cmp(b),
            (Present(IndexLabel::Utf8(a)), Present(IndexLabel::Utf8(b))) => a.cmp(b),
            (Present(IndexLabel::Timedelta64(a)), Present(IndexLabel::Timedelta64(b))) => a.cmp(b),
            (Present(IndexLabel::Datetime64(a)), Present(IndexLabel::Datetime64(b))) => a.cmp(b),
            (Present(IndexLabel::Int64(_)), Present(IndexLabel::Utf8(_))) => Ordering::Less,
            (Present(IndexLabel::Utf8(_)), Present(IndexLabel::Int64(_))) => Ordering::Greater,
            (Present(IndexLabel::Timedelta64(_)), Present(IndexLabel::Int64(_))) => {
                Ordering::Greater
            }
            (Present(IndexLabel::Timedelta64(_)), Present(IndexLabel::Utf8(_))) => {
                Ordering::Greater
            }
            (Present(IndexLabel::Int64(_)), Present(IndexLabel::Timedelta64(_))) => Ordering::Less,
            (Present(IndexLabel::Utf8(_)), Present(IndexLabel::Timedelta64(_))) => Ordering::Less,
            (Present(IndexLabel::Datetime64(_)), Present(IndexLabel::Int64(_))) => {
                Ordering::Greater
            }
            (Present(IndexLabel::Datetime64(_)), Present(IndexLabel::Utf8(_))) => Ordering::Greater,
            (Present(IndexLabel::Datetime64(_)), Present(IndexLabel::Timedelta64(_))) => {
                Ordering::Greater
            }
            (Present(IndexLabel::Int64(_)), Present(IndexLabel::Datetime64(_))) => Ordering::Less,
            (Present(IndexLabel::Utf8(_)), Present(IndexLabel::Datetime64(_))) => Ordering::Less,
            (Present(IndexLabel::Timedelta64(_)), Present(IndexLabel::Datetime64(_))) => {
                Ordering::Less
            }
            (Present(IndexLabel::Int64(a)), FloatBits(bits)) => {
                let ord = (*a as f64).total_cmp(&f64::from_bits(*bits));
                if ord == Ordering::Equal {
                    Ordering::Less
                } else {
                    ord
                }
            }
            (FloatBits(bits), Present(IndexLabel::Int64(a))) => {
                let ord = f64::from_bits(*bits).total_cmp(&(*a as f64));
                if ord == Ordering::Equal {
                    Ordering::Greater
                } else {
                    ord
                }
            }
            (Present(IndexLabel::Timedelta64(a)), FloatBits(bits)) => {
                let ord = (*a as f64).total_cmp(&f64::from_bits(*bits));
                if ord == Ordering::Equal {
                    Ordering::Less
                } else {
                    ord
                }
            }
            (FloatBits(bits), Present(IndexLabel::Timedelta64(a))) => {
                let ord = f64::from_bits(*bits).total_cmp(&(*a as f64));
                if ord == Ordering::Equal {
                    Ordering::Greater
                } else {
                    ord
                }
            }
            (Present(IndexLabel::Datetime64(a)), FloatBits(bits)) => {
                let ord = (*a as f64).total_cmp(&f64::from_bits(*bits));
                if ord == Ordering::Equal {
                    Ordering::Less
                } else {
                    ord
                }
            }
            (FloatBits(bits), Present(IndexLabel::Datetime64(a))) => {
                let ord = f64::from_bits(*bits).total_cmp(&(*a as f64));
                if ord == Ordering::Equal {
                    Ordering::Greater
                } else {
                    ord
                }
            }
            (FloatBits(a_bits), FloatBits(b_bits)) => {
                f64::from_bits(*a_bits).total_cmp(&f64::from_bits(*b_bits))
            }
            (FloatBits(_), Present(IndexLabel::Utf8(_))) => Ordering::Less,
            (Present(IndexLabel::Utf8(_)), FloatBits(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for JoinKeyComponent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn scalar_to_key_component(s: &fp_types::Scalar) -> JoinKeyComponent {
    match s {
        fp_types::Scalar::Int64(v) => JoinKeyComponent::Present(IndexLabel::Int64(*v)),
        fp_types::Scalar::Float64(v) if !v.is_nan() => {
            if *v == v.trunc() && *v >= i64::MIN as f64 && *v < 9223372036854775808.0 {
                JoinKeyComponent::Present(IndexLabel::Int64(*v as i64))
            } else {
                JoinKeyComponent::FloatBits(v.to_bits())
            }
        }
        fp_types::Scalar::Utf8(v) => JoinKeyComponent::Present(IndexLabel::Utf8(v.clone())),
        fp_types::Scalar::Bool(b) => JoinKeyComponent::Present(IndexLabel::Int64(i64::from(*b))),
        _ => JoinKeyComponent::Missing, // Null, NaN, NaT
    }
}

type CompositeJoinKey = Vec<JoinKeyComponent>;

fn collect_join_key_columns<'a>(
    frame: &'a fp_frame::DataFrame,
    on: &[&str],
    side: &str,
) -> Result<Vec<&'a Column>, JoinError> {
    let mut key_columns = Vec::with_capacity(on.len());
    for key_name in on {
        let key_column = frame.column(key_name).ok_or_else(|| {
            JoinError::Frame(FrameError::CompatibilityRejected(format!(
                "{side} DataFrame missing key column '{key_name}'"
            )))
        })?;
        key_columns.push(key_column);
    }
    Ok(key_columns)
}

fn collect_composite_keys(key_columns: &[&Column]) -> Vec<CompositeJoinKey> {
    let row_count = key_columns.first().map_or(0, |column| column.len());
    let mut out = Vec::with_capacity(row_count);

    for row in 0..row_count {
        let mut parts = Vec::with_capacity(key_columns.len());
        for column in key_columns {
            parts.push(scalar_to_key_component(&column.values()[row]));
        }
        out.push(parts);
    }

    out
}

fn has_duplicate_composite_keys(keys: &[CompositeJoinKey]) -> bool {
    let mut seen = HashSet::with_capacity(keys.len());
    for key in keys {
        if !seen.insert(key) {
            return true;
        }
    }
    false
}

fn validate_merge_cardinality(
    validate_mode: MergeValidateMode,
    left_keys: &[CompositeJoinKey],
    right_keys: &[CompositeJoinKey],
) -> Result<(), JoinError> {
    let left_has_duplicates = has_duplicate_composite_keys(left_keys);
    let right_has_duplicates = has_duplicate_composite_keys(right_keys);

    let fail = |message: &str| {
        Err(JoinError::Frame(FrameError::CompatibilityRejected(
            message.to_owned(),
        )))
    };

    match validate_mode {
        MergeValidateMode::OneToOne => {
            if left_has_duplicates {
                return fail("merge validate='one_to_one' failed: left keys are not unique");
            }
            if right_has_duplicates {
                return fail("merge validate='one_to_one' failed: right keys are not unique");
            }
        }
        MergeValidateMode::OneToMany => {
            if left_has_duplicates {
                return fail("merge validate='one_to_many' failed: left keys are not unique");
            }
        }
        MergeValidateMode::ManyToOne => {
            if right_has_duplicates {
                return fail("merge validate='many_to_one' failed: right keys are not unique");
            }
        }
        MergeValidateMode::ManyToMany => {}
    }

    Ok(())
}

fn reorder_vec_by_index<T: Clone>(values: &mut Vec<T>, order: &[usize]) {
    let existing = values.clone();
    values.clear();
    values.reserve(order.len());
    for &idx in order {
        values.push(existing[idx].clone());
    }
}

fn sort_merge_rows_by_join_keys(
    out_row_keys: &[CompositeJoinKey],
    left_positions: &mut Vec<Option<usize>>,
    right_positions: &mut Vec<Option<usize>>,
) {
    if out_row_keys.is_empty() {
        return;
    }
    let mut order: Vec<usize> = (0..out_row_keys.len()).collect();
    order.sort_by(|a, b| out_row_keys[*a].cmp(&out_row_keys[*b]));
    reorder_vec_by_index(left_positions, &order);
    reorder_vec_by_index(right_positions, &order);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedMergeSuffixes {
    left: Option<String>,
    right: Option<String>,
}

impl Default for ResolvedMergeSuffixes {
    fn default() -> Self {
        Self {
            left: Some("_left".to_owned()),
            right: Some("_right".to_owned()),
        }
    }
}

fn resolve_merge_suffixes(suffixes: Option<[Option<String>; 2]>) -> ResolvedMergeSuffixes {
    match suffixes {
        Some([left, right]) => ResolvedMergeSuffixes { left, right },
        None => ResolvedMergeSuffixes::default(),
    }
}

fn suffix_is_present(suffix: Option<&str>) -> bool {
    suffix.is_some_and(|value| !value.is_empty())
}

fn apply_merge_suffix(name: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(value) if !value.is_empty() => format!("{name}{value}"),
        _ => name.to_owned(),
    }
}

fn collect_overlapping_column_names(
    left_col_names: &HashSet<&String>,
    right_col_names: &HashSet<&String>,
    excluded_names: &HashSet<&str>,
) -> Vec<String> {
    let mut overlaps = left_col_names
        .iter()
        .filter(|name| right_col_names.contains(*name) && !excluded_names.contains(name.as_str()))
        .map(|name| (*name).clone())
        .collect::<Vec<_>>();
    overlaps.sort();
    overlaps
}

fn ensure_merge_suffixes_for_overlaps(
    overlap_names: &[String],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<(), JoinError> {
    if overlap_names.is_empty() {
        return Ok(());
    }
    if !suffix_is_present(suffixes.left.as_deref()) && !suffix_is_present(suffixes.right.as_deref())
    {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            format!(
                "columns overlap but no suffix specified: {}",
                overlap_names.join(", ")
            ),
        )));
    }
    Ok(())
}

fn insert_merged_output_column(
    output_columns: &mut std::collections::BTreeMap<String, Column>,
    name: String,
    column: Column,
) -> Result<(), JoinError> {
    if output_columns.contains_key(&name) {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            format!("merge suffixes cause duplicate output column '{name}'"),
        )));
    }
    output_columns.insert(name, column);
    Ok(())
}

fn resolve_merge_indicator_name(indicator_name: Option<&str>) -> Result<Option<String>, JoinError> {
    let Some(name) = indicator_name else {
        return Ok(None);
    };
    if name.trim().is_empty() {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            "merge indicator column name must be non-empty".to_owned(),
        )));
    }
    Ok(Some(name.to_owned()))
}

fn build_merge_indicator_column(
    left_positions: &[Option<usize>],
    right_positions: &[Option<usize>],
) -> Result<Column, JoinError> {
    let values = left_positions
        .iter()
        .zip(right_positions.iter())
        .map(|(left_pos, right_pos)| match (left_pos, right_pos) {
            (Some(_), Some(_)) => fp_types::Scalar::Utf8("both".to_owned()),
            (Some(_), None) => fp_types::Scalar::Utf8("left_only".to_owned()),
            (None, Some(_)) => fp_types::Scalar::Utf8("right_only".to_owned()),
            (None, None) => fp_types::Scalar::Null(fp_types::NullKind::Null),
        })
        .collect::<Vec<_>>();
    Column::from_values(values).map_err(JoinError::from)
}

fn ensure_indicator_name_available(
    indicator_name: &str,
    left_col_names: &HashSet<&String>,
    right_col_names: &HashSet<&String>,
    output_columns: &std::collections::BTreeMap<String, Column>,
) -> Result<(), JoinError> {
    if left_col_names
        .iter()
        .any(|name| name.as_str() == indicator_name)
        || right_col_names
            .iter()
            .any(|name| name.as_str() == indicator_name)
        || output_columns.contains_key(indicator_name)
    {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            format!("merge indicator column '{indicator_name}' conflicts with an existing column"),
        )));
    }
    Ok(())
}

/// Merge two DataFrames on one or more key columns.
///
/// Matches `pd.merge(left, right, left_on=left_keys, right_on=right_keys, how=join_type)`.
/// - Key columns are used for matching rows (hash join).
/// - Non-key columns are carried through and reindexed.
/// - Column name conflicts use configurable suffixes (default `_left`/`_right`).
/// - The output index is auto-generated (0..n RangeIndex-style).
pub fn merge_dataframes_on_with(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    join_type: JoinType,
) -> Result<MergedDataFrame, JoinError> {
    merge_dataframes_on_with_options(
        left,
        right,
        left_on,
        right_on,
        join_type,
        MergeExecutionOptions::default(),
    )
}

/// Merge two DataFrames on one or more key columns with execution options.
pub fn merge_dataframes_on_with_options(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    join_type: JoinType,
    options: MergeExecutionOptions,
) -> Result<MergedDataFrame, JoinError> {
    let MergeExecutionOptions {
        indicator_name,
        validate_mode,
        suffixes,
        sort,
    } = options;
    let indicator_name = resolve_merge_indicator_name(indicator_name.as_deref())?;
    let suffixes = resolve_merge_suffixes(suffixes);

    if matches!(join_type, JoinType::Cross) {
        if let Some(validate_mode) = validate_mode
            && !matches!(validate_mode, MergeValidateMode::ManyToMany)
        {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                format!(
                    "merge validate='{}' is not supported for cross join",
                    validate_mode.as_str()
                ),
            )));
        }
        return merge_dataframes_cross(left, right, indicator_name.as_deref(), &suffixes);
    }

    if left_on.is_empty() || right_on.is_empty() {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            "merge requires at least one key column".to_owned(),
        )));
    }
    if left_on.len() != right_on.len() {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            "merge requires left_on and right_on key lists with equal length".to_owned(),
        )));
    }

    let mut seen_left_keys = HashSet::new();
    for key_name in left_on {
        if !seen_left_keys.insert(*key_name) {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                format!("merge left key column '{key_name}' is duplicated"),
            )));
        }
    }
    let mut seen_right_keys = HashSet::new();
    for key_name in right_on {
        if !seen_right_keys.insert(*key_name) {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                format!("merge right key column '{key_name}' is duplicated"),
            )));
        }
    }

    let left_key_columns = collect_join_key_columns(left, left_on, "left")?;
    let right_key_columns = collect_join_key_columns(right, right_on, "right")?;

    // Convert key columns to hashable composite keys.
    let left_keys = collect_composite_keys(&left_key_columns);
    let right_keys = collect_composite_keys(&right_key_columns);
    if let Some(validate_mode) = validate_mode {
        validate_merge_cardinality(validate_mode, &left_keys, &right_keys)?;
    }

    // Build hash map from right key → row positions.
    let mut right_map = HashMap::<&CompositeJoinKey, Vec<usize>>::new();
    for (pos, key) in right_keys.iter().enumerate() {
        right_map.entry(key).or_default().push(pos);
    }

    let left_map = if matches!(join_type, JoinType::Right | JoinType::Outer) {
        let mut m = HashMap::<&CompositeJoinKey, Vec<usize>>::new();
        for (pos, key) in left_keys.iter().enumerate() {
            m.entry(key).or_default().push(pos);
        }
        Some(m)
    } else {
        None
    };

    // Compute row position mappings.
    let mut left_positions = Vec::<Option<usize>>::new();
    let mut right_positions = Vec::<Option<usize>>::new();
    let mut out_row_keys = Vec::<CompositeJoinKey>::new();

    match join_type {
        JoinType::Inner | JoinType::Left | JoinType::Outer => {
            for (left_pos, key) in left_keys.iter().enumerate() {
                if let Some(matches) = right_map.get(key) {
                    for &right_pos in matches {
                        out_row_keys.push(key.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(right_pos));
                    }
                    continue;
                }

                if matches!(join_type, JoinType::Left | JoinType::Outer) {
                    out_row_keys.push(key.clone());
                    left_positions.push(Some(left_pos));
                    right_positions.push(None);
                }
            }

            if matches!(join_type, JoinType::Outer) {
                let left_map = left_map.as_ref().expect("left_map required for Outer join");
                for (right_pos, key) in right_keys.iter().enumerate() {
                    if !left_map.contains_key(key) {
                        out_row_keys.push(key.clone());
                        left_positions.push(None);
                        right_positions.push(Some(right_pos));
                    }
                }
            }
        }
        JoinType::Right => {
            let left_map = left_map.as_ref().expect("left_map required for Right join");
            for (right_pos, key) in right_keys.iter().enumerate() {
                if let Some(matches) = left_map.get(key) {
                    for &left_pos in matches {
                        out_row_keys.push(key.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(right_pos));
                    }
                    continue;
                }

                out_row_keys.push(key.clone());
                left_positions.push(None);
                right_positions.push(Some(right_pos));
            }
        }
        JoinType::Cross => {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                "cross join must be handled by merge_dataframes_cross".to_owned(),
            )));
        }
    }

    if sort || matches!(join_type, JoinType::Outer) {
        sort_merge_rows_by_join_keys(&out_row_keys, &mut left_positions, &mut right_positions);
    }

    // Build output columns by reindexing.
    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();

    // Collect column names to handle conflicts.
    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();

    // Track positional pairs where left_on[i] and right_on[i] share the same key name.
    let mut shared_name_positions = HashMap::<&str, (usize, usize)>::new();
    for (idx, (left_key_name, right_key_name)) in left_on.iter().zip(right_on.iter()).enumerate() {
        if left_key_name == right_key_name {
            shared_name_positions.insert(*left_key_name, (idx, idx));
        }
    }
    let shared_key_names = shared_name_positions
        .keys()
        .copied()
        .collect::<HashSet<&str>>();
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, &suffixes)?;

    // Add left columns (including left key columns).
    for (name, col) in left.columns() {
        if left_key_name_set.contains(name.as_str()) {
            if let Some((left_key_idx, right_key_idx)) = shared_name_positions.get(name.as_str()) {
                // Shared key name: for rows emitted from right-only keys, source key values from
                // the right frame instead of leaving them null.
                let left_key_col = left_key_columns[*left_key_idx];
                let right_key_col = right_key_columns[*right_key_idx];
                let values = left_positions
                    .iter()
                    .zip(right_positions.iter())
                    .map(|(left_pos, right_pos)| match (left_pos, right_pos) {
                        (Some(pos), _) => left_key_col.values()[*pos].clone(),
                        (None, Some(pos)) => right_key_col.values()[*pos].clone(),
                        (None, None) => fp_types::Scalar::Null(fp_types::NullKind::Null),
                    })
                    .collect::<Vec<_>>();
                insert_merged_output_column(
                    &mut columns,
                    name.clone(),
                    Column::from_values(values)?,
                )?;
            } else {
                let key_column = if matches!(join_type, JoinType::Outer) {
                    reindex_outer_join_column(col, &left_positions)?
                } else {
                    col.reindex_by_positions(&left_positions)?
                };
                insert_merged_output_column(&mut columns, name.clone(), key_column)?;
            }
            continue;
        }
        let reindexed = if matches!(join_type, JoinType::Outer) {
            reindex_outer_join_column(col, &left_positions)?
        } else {
            col.reindex_by_positions(&left_positions)?
        };
        let out_name = if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, out_name, reindexed)?;
    }

    // Add right columns (including right key alias columns).
    for (name, col) in right.columns() {
        if right_key_name_set.contains(name.as_str())
            && shared_name_positions.contains_key(name.as_str())
        {
            // Shared same-name key already emitted from the left side.
            continue;
        }
        let reindexed = if matches!(join_type, JoinType::Outer) {
            reindex_outer_join_column(col, &right_positions)?
        } else {
            col.reindex_by_positions(&right_positions)?
        };
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, out_name, reindexed)?;
    }

    if let Some(indicator_name) = indicator_name.as_deref() {
        ensure_indicator_name_available(
            indicator_name,
            &left_col_names,
            &right_col_names,
            &columns,
        )?;
        let indicator_col = build_merge_indicator_column(&left_positions, &right_positions)?;
        insert_merged_output_column(&mut columns, indicator_name.to_owned(), indicator_col)?;
    }

    Ok(MergedDataFrame { index, columns })
}

/// Merge two DataFrames on one or more key columns with identical key names.
///
/// Matches `pd.merge(left, right, on=keys, how=join_type)`.
pub fn merge_dataframes_on(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &[&str],
    join_type: JoinType,
) -> Result<MergedDataFrame, JoinError> {
    merge_dataframes_on_with(left, right, on, on, join_type)
}

/// Merge two DataFrames on a single key column.
pub fn merge_dataframes(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    join_type: JoinType,
) -> Result<MergedDataFrame, JoinError> {
    merge_dataframes_on(left, right, &[on], join_type)
}

fn merge_dataframes_cross(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    indicator_name: Option<&str>,
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    let left_rows = left.index().len();
    let right_rows = right.index().len();
    let out_rows = left_rows.saturating_mul(right_rows);

    let mut left_positions = Vec::<Option<usize>>::with_capacity(out_rows);
    let mut right_positions = Vec::<Option<usize>>::with_capacity(out_rows);
    for left_pos in 0..left_rows {
        for right_pos in 0..right_rows {
            left_positions.push(Some(left_pos));
            right_positions.push(Some(right_pos));
        }
    }

    let index = Index::new((0..out_rows as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &HashSet::new());
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for (name, col) in left.columns() {
        let reindexed = col.reindex_by_positions(&left_positions)?;
        let out_name = if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, out_name, reindexed)?;
    }

    for (name, col) in right.columns() {
        let reindexed = col.reindex_by_positions(&right_positions)?;
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, out_name, reindexed)?;
    }

    if let Some(indicator_name) = indicator_name {
        ensure_indicator_name_available(
            indicator_name,
            &left_col_names,
            &right_col_names,
            &columns,
        )?;
        let indicator_col = build_merge_indicator_column(&left_positions, &right_positions)?;
        insert_merged_output_column(&mut columns, indicator_name.to_owned(), indicator_col)?;
    }

    Ok(MergedDataFrame { index, columns })
}

/// Extension trait adding `.merge()` and `.join()` instance methods to `DataFrame`.
///
/// Import this trait to get pandas-style `df.merge(other, ...)` and `df.join(other, ...)`
/// as instance methods.
pub trait DataFrameMergeExt {
    /// Merge this DataFrame with another on shared key columns.
    ///
    /// Matches `pd.DataFrame.merge(other, on=keys, how=join_type)`.
    fn merge(
        &self,
        other: &fp_frame::DataFrame,
        on: &[&str],
        how: JoinType,
    ) -> Result<MergedDataFrame, JoinError>;

    /// Merge with separate left/right key columns and execution options.
    ///
    /// Matches `pd.DataFrame.merge(other, left_on=..., right_on=..., how=...)`.
    fn merge_with_options(
        &self,
        other: &fp_frame::DataFrame,
        left_on: &[&str],
        right_on: &[&str],
        how: JoinType,
        options: MergeExecutionOptions,
    ) -> Result<MergedDataFrame, JoinError>;

    /// Join another DataFrame on the index.
    ///
    /// Matches `pd.DataFrame.join(other, how=join_type)` (index-based join).
    fn join_on_index(
        &self,
        other: &fp_frame::DataFrame,
        how: JoinType,
    ) -> Result<MergedDataFrame, JoinError>;

    /// Perform an asof merge (nearest-match join) on a sorted key column.
    ///
    /// Matches `pd.merge_asof(left, right, on=key)`. Both DataFrames must
    /// be sorted by the key column. For each row in `left`, finds the last
    /// row in `right` where `right[on] <= left[on]` (backward direction)
    /// or the nearest row in the specified direction.
    ///
    /// `direction`: "backward" (default), "forward", "nearest".
    fn merge_asof(
        &self,
        other: &fp_frame::DataFrame,
        on: &str,
        direction: &str,
    ) -> Result<MergedDataFrame, JoinError>;

    /// Perform an asof merge with additional options.
    ///
    /// Matches `pd.merge_asof(left, right, on=key, direction=...,
    /// allow_exact_matches=..., tolerance=..., by=...)`.
    ///
    /// # Parameters
    /// - `other`: Right DataFrame to merge with
    /// - `on`: Column name to merge on (must be numeric and sorted)
    /// - `direction`: "backward", "forward", or "nearest"
    /// - `options`: Additional options (allow_exact_matches, tolerance, by)
    fn merge_asof_with_options(
        &self,
        other: &fp_frame::DataFrame,
        on: &str,
        direction: &str,
        options: MergeAsofOptions,
    ) -> Result<MergedDataFrame, JoinError>;

    /// Perform an ordered merge (outer join + sort by keys).
    ///
    /// Matches `pd.merge_ordered(left, right, on=keys, fill_method=...)`.
    fn merge_ordered(
        &self,
        other: &fp_frame::DataFrame,
        on: &[&str],
        fill_method: Option<&str>,
    ) -> Result<MergedDataFrame, JoinError>;
}

/// Perform an ordered merge of two DataFrames.
///
/// Matches `pd.merge_ordered(left, right, on=keys, fill_method=...)`.
/// Performs an outer join and ensures the result is sorted by the join keys.
pub fn merge_ordered(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &[&str],
    fill_method: Option<&str>,
) -> Result<MergedDataFrame, JoinError> {
    let options = MergeExecutionOptions {
        sort: true,
        ..MergeExecutionOptions::default()
    };
    let merged = merge_dataframes_on_with_options(left, right, on, on, JoinType::Outer, options)?;

    if let Some(method) = fill_method {
        // Convert to DataFrame to apply fill
        let mut df = fp_frame::DataFrame::new(merged.index.clone(), merged.columns.clone())
            .map_err(JoinError::Frame)?;

        df = match method {
            "ffill" => df.ffill(None).map_err(JoinError::Frame)?,
            "bfill" => df.bfill(None).map_err(JoinError::Frame)?,
            _ => {
                return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                    format!("merge_ordered: unknown fill_method '{method}'"),
                )));
            }
        };

        Ok(MergedDataFrame {
            index: df.index().clone(),
            columns: df.columns().clone(),
        })
    } else {
        Ok(merged)
    }
}

/// Direction for asof merge matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsofDirection {
    Backward,
    Forward,
    Nearest,
}

/// Options for asof merge operations.
///
/// Matches pandas `merge_asof` parameters:
/// - `allow_exact_matches`: If true (default), allow matching with same key value
/// - `tolerance`: Maximum distance between keys for a match
/// - `by`: Columns to match exactly before asof matching
#[derive(Debug, Clone, Default)]
pub struct MergeAsofOptions {
    /// If true (default), allow matching with the same 'on' value.
    /// If false, don't match rows where left key == right key exactly.
    pub allow_exact_matches: bool,
    /// Maximum distance between left and right keys for a valid match.
    /// None means no tolerance limit.
    pub tolerance: Option<f64>,
    /// Columns to match exactly before performing asof merge.
    /// Acts like an equi-join on these columns first.
    pub by: Option<Vec<String>>,
}

impl MergeAsofOptions {
    /// Create options with defaults matching pandas behavior.
    pub fn new() -> Self {
        Self {
            allow_exact_matches: true,
            tolerance: None,
            by: None,
        }
    }

    /// Set whether exact matches are allowed.
    pub fn allow_exact_matches(mut self, allow: bool) -> Self {
        self.allow_exact_matches = allow;
        self
    }

    /// Set the tolerance for matching.
    pub fn tolerance(mut self, tol: f64) -> Self {
        self.tolerance = Some(tol);
        self
    }

    /// Set the columns to match exactly before asof merge.
    pub fn by(mut self, cols: Vec<String>) -> Self {
        self.by = Some(cols);
        self
    }
}

fn asof_numeric_values(column: &Column, side: &str, on: &str) -> Result<Vec<f64>, JoinError> {
    let mut out = Vec::with_capacity(column.len());
    for value in column.values() {
        match value.to_f64() {
            Ok(v) => out.push(v),
            Err(TypeError::ValueIsMissing { .. }) => out.push(f64::NAN),
            Err(_) => {
                return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                    format!("merge_asof: {side} column '{on}' must be numeric"),
                )));
            }
        }
    }
    Ok(out)
}

fn ensure_sorted_non_decreasing(values: &[f64], side: &str, on: &str) -> Result<(), JoinError> {
    let mut prev: Option<f64> = None;
    for &value in values {
        if value.is_nan() {
            continue;
        }
        if let Some(prev_value) = prev
            && value < prev_value
        {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                format!("merge_asof: {side} column '{on}' must be sorted"),
            )));
        }
        prev = Some(value);
    }
    Ok(())
}

/// Perform an asof merge between two DataFrames.
pub fn merge_asof(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    direction: AsofDirection,
) -> Result<MergedDataFrame, JoinError> {
    merge_asof_with_options(left, right, on, direction, MergeAsofOptions::new())
}

/// Perform an asof merge between two DataFrames with additional options.
///
/// # Parameters
/// - `left`: Left DataFrame
/// - `right`: Right DataFrame
/// - `on`: Column name to merge on (must be numeric and sorted)
/// - `direction`: Match direction (Backward, Forward, Nearest)
/// - `options`: Additional merge options (allow_exact_matches, tolerance, by)
pub fn merge_asof_with_options(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    direction: AsofDirection,
    options: MergeAsofOptions,
) -> Result<MergedDataFrame, JoinError> {
    // If `by` columns are specified, we need to do grouped asof merge
    if let Some(ref by_cols) = options.by {
        return merge_asof_grouped(left, right, on, direction, &options, by_cols);
    }

    merge_asof_simple(left, right, on, direction, &options)
}

/// Simple asof merge without grouping.
fn merge_asof_simple(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    direction: AsofDirection,
    options: &MergeAsofOptions,
) -> Result<MergedDataFrame, JoinError> {
    let left_key = left.columns().get(on).ok_or_else(|| {
        JoinError::Frame(FrameError::CompatibilityRejected(format!(
            "merge_asof: column '{on}' not found in left"
        )))
    })?;
    let right_key = right.columns().get(on).ok_or_else(|| {
        JoinError::Frame(FrameError::CompatibilityRejected(format!(
            "merge_asof: column '{on}' not found in right"
        )))
    })?;

    let left_vals = asof_numeric_values(left_key, "left", on)?;
    let right_vals = asof_numeric_values(right_key, "right", on)?;

    ensure_sorted_non_decreasing(&left_vals, "left", on)?;
    ensure_sorted_non_decreasing(&right_vals, "right", on)?;

    let right_matches = compute_asof_matches(
        &left_vals,
        &right_vals,
        direction,
        options.allow_exact_matches,
        options.tolerance,
    );

    build_asof_output(left, right, on, &right_matches, None)
}

/// Grouped asof merge: match on `by` columns first, then asof within groups.
fn merge_asof_grouped(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    direction: AsofDirection,
    options: &MergeAsofOptions,
    by_cols: &[String],
) -> Result<MergedDataFrame, JoinError> {
    // Validate by columns exist
    for col in by_cols {
        if left.columns().get(col).is_none() {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                format!("merge_asof: 'by' column '{col}' not found in left"),
            )));
        }
        if right.columns().get(col).is_none() {
            return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                format!("merge_asof: 'by' column '{col}' not found in right"),
            )));
        }
    }

    let left_key = left.columns().get(on).ok_or_else(|| {
        JoinError::Frame(FrameError::CompatibilityRejected(format!(
            "merge_asof: column '{on}' not found in left"
        )))
    })?;
    let right_key = right.columns().get(on).ok_or_else(|| {
        JoinError::Frame(FrameError::CompatibilityRejected(format!(
            "merge_asof: column '{on}' not found in right"
        )))
    })?;

    let left_vals = asof_numeric_values(left_key, "left", on)?;
    let right_vals = asof_numeric_values(right_key, "right", on)?;

    // Build group keys for left and right
    let left_group_keys = build_group_keys(left, by_cols)?;
    let right_group_keys = build_group_keys(right, by_cols)?;

    // Group left rows by their group key (to check sorting per-group)
    let mut left_groups: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for (idx, key) in left_group_keys.iter().enumerate() {
        left_groups.entry(key.clone()).or_default().push(idx);
    }

    // Group right rows by their group key
    let mut right_groups: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for (idx, key) in right_group_keys.iter().enumerate() {
        right_groups.entry(key.clone()).or_default().push(idx);
    }

    // Check sorting within each left group
    for (group_key, indices) in &left_groups {
        let group_vals: Vec<f64> = indices.iter().map(|&i| left_vals[i]).collect();
        ensure_sorted_non_decreasing(&group_vals, "left", on).map_err(|_| {
            JoinError::Frame(FrameError::CompatibilityRejected(format!(
                "merge_asof: left column '{on}' must be sorted within group {:?}",
                group_key
            )))
        })?;
    }

    // Check sorting within each right group
    for (group_key, indices) in &right_groups {
        let group_vals: Vec<f64> = indices.iter().map(|&i| right_vals[i]).collect();
        ensure_sorted_non_decreasing(&group_vals, "right", on).map_err(|_| {
            JoinError::Frame(FrameError::CompatibilityRejected(format!(
                "merge_asof: right column '{on}' must be sorted within group {:?}",
                group_key
            )))
        })?;
    }

    // For each left row, find match within its group
    let mut right_matches: Vec<Option<usize>> = Vec::with_capacity(left_vals.len());

    for (left_idx, left_group_key) in left_group_keys.iter().enumerate() {
        let lv = left_vals[left_idx];

        if lv.is_nan() {
            right_matches.push(None);
            continue;
        }

        // Get right indices in same group
        let group_indices = match right_groups.get(left_group_key) {
            Some(indices) => indices,
            None => {
                right_matches.push(None);
                continue;
            }
        };

        // Extract right values for this group
        let group_right_vals: Vec<f64> = group_indices.iter().map(|&i| right_vals[i]).collect();

        // Compute match within group
        let group_matches = compute_asof_matches(
            &[lv],
            &group_right_vals,
            direction,
            options.allow_exact_matches,
            options.tolerance,
        );

        // Map back to original right index
        match group_matches.first() {
            Some(Some(group_idx)) => {
                right_matches.push(Some(group_indices[*group_idx]));
            }
            _ => {
                right_matches.push(None);
            }
        }
    }

    build_asof_output(left, right, on, &right_matches, Some(by_cols))
}

/// Build group keys from the `by` columns.
fn build_group_keys(
    df: &fp_frame::DataFrame,
    by_cols: &[String],
) -> Result<Vec<Vec<String>>, JoinError> {
    let n = df.len();
    let mut keys = vec![Vec::with_capacity(by_cols.len()); n];

    for col_name in by_cols {
        let col = df.columns().get(col_name).ok_or_else(|| {
            JoinError::Frame(FrameError::CompatibilityRejected(format!(
                "merge_asof: column '{col_name}' not found"
            )))
        })?;
        for (i, val) in col.values().iter().enumerate() {
            keys[i].push(format!("{val:?}"));
        }
    }

    Ok(keys)
}

/// Compute asof matches for a single group or the whole dataset.
fn compute_asof_matches(
    left_vals: &[f64],
    right_vals: &[f64],
    direction: AsofDirection,
    allow_exact_matches: bool,
    tolerance: Option<f64>,
) -> Vec<Option<usize>> {
    let mut right_non_nan_values = Vec::new();
    let mut right_non_nan_positions = Vec::new();
    for (pos, &rv) in right_vals.iter().enumerate() {
        if !rv.is_nan() {
            right_non_nan_values.push(rv);
            right_non_nan_positions.push(pos);
        }
    }

    let mut right_matches: Vec<Option<usize>> = Vec::with_capacity(left_vals.len());

    match direction {
        AsofDirection::Backward => {
            let mut best: Option<usize> = None;
            let mut best_val: Option<f64> = None;
            let mut j = 0usize;
            for &lv in left_vals {
                if lv.is_nan() {
                    right_matches.push(None);
                    continue;
                }
                // Advance while right <= left (or right < left if !allow_exact_matches)
                while j < right_non_nan_values.len() {
                    let rv = right_non_nan_values[j];
                    let should_include = if allow_exact_matches {
                        rv <= lv
                    } else {
                        rv < lv
                    };
                    if !should_include {
                        break;
                    }
                    best = Some(right_non_nan_positions[j]);
                    best_val = Some(rv);
                    j += 1;
                }

                // Check tolerance
                let matched = match (best, best_val, tolerance) {
                    (Some(idx), Some(rv), Some(tol)) if (lv - rv).abs() <= tol => Some(idx),
                    (Some(idx), _, None) => Some(idx),
                    _ => None,
                };
                right_matches.push(matched);
            }
        }
        AsofDirection::Forward => {
            let mut j = 0usize;
            for &lv in left_vals {
                if lv.is_nan() {
                    right_matches.push(None);
                    continue;
                }
                // Advance while right < left (or right <= left if !allow_exact_matches)
                while j < right_non_nan_values.len() {
                    let rv = right_non_nan_values[j];
                    let should_skip = if allow_exact_matches {
                        rv < lv
                    } else {
                        rv <= lv
                    };
                    if !should_skip {
                        break;
                    }
                    j += 1;
                }

                if j < right_non_nan_values.len() {
                    let rv = right_non_nan_values[j];
                    // Check tolerance
                    let matched = match tolerance {
                        Some(tol) if (rv - lv).abs() <= tol => Some(right_non_nan_positions[j]),
                        None => Some(right_non_nan_positions[j]),
                        _ => None,
                    };
                    right_matches.push(matched);
                } else {
                    right_matches.push(None);
                }
            }
        }
        AsofDirection::Nearest => {
            for &lv in left_vals {
                if lv.is_nan() {
                    right_matches.push(None);
                    continue;
                }
                if right_non_nan_values.is_empty() {
                    right_matches.push(None);
                    continue;
                }

                // For nearest, find the closest value
                // If !allow_exact_matches, exclude exact matches from consideration
                let pos = right_non_nan_values.partition_point(|rv| *rv <= lv);

                let lower = if pos > 0 { Some(pos - 1) } else { None };
                let upper = if pos < right_non_nan_values.len() {
                    Some(pos)
                } else {
                    None
                };

                let chosen = match (lower, upper) {
                    (Some(l), Some(u)) => {
                        let lower_val = right_non_nan_values[l];
                        let upper_val = right_non_nan_values[u];
                        let lower_exact = (lower_val - lv).abs() < f64::EPSILON;
                        let upper_exact = (upper_val - lv).abs() < f64::EPSILON;

                        if !allow_exact_matches {
                            // Exclude exact matches
                            if lower_exact && upper_exact {
                                None
                            } else if lower_exact {
                                Some(u)
                            } else if upper_exact {
                                Some(l)
                            } else {
                                let lower_dist = (lower_val - lv).abs();
                                let upper_dist = (upper_val - lv).abs();
                                if upper_dist < lower_dist {
                                    Some(u)
                                } else {
                                    Some(l)
                                }
                            }
                        } else {
                            let lower_dist = (lower_val - lv).abs();
                            let upper_dist = (upper_val - lv).abs();
                            if upper_dist < lower_dist {
                                Some(u)
                            } else {
                                Some(l)
                            }
                        }
                    }
                    (Some(l), None) => {
                        let val = right_non_nan_values[l];
                        let exact = (val - lv).abs() < f64::EPSILON;
                        if !allow_exact_matches && exact {
                            None
                        } else {
                            Some(l)
                        }
                    }
                    (None, Some(u)) => {
                        let val = right_non_nan_values[u];
                        let exact = (val - lv).abs() < f64::EPSILON;
                        if !allow_exact_matches && exact {
                            None
                        } else {
                            Some(u)
                        }
                    }
                    (None, None) => None,
                };

                // Apply tolerance check
                let matched = match chosen {
                    Some(idx) => {
                        let rv = right_non_nan_values[idx];
                        match tolerance {
                            Some(tol) if (lv - rv).abs() <= tol => {
                                Some(right_non_nan_positions[idx])
                            }
                            None => Some(right_non_nan_positions[idx]),
                            _ => None,
                        }
                    }
                    None => None,
                };
                right_matches.push(matched);
            }
        }
    }

    right_matches
}

/// Build the output DataFrame from computed matches.
fn build_asof_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    right_matches: &[Option<usize>],
    by_cols: Option<&[String]>,
) -> Result<MergedDataFrame, JoinError> {
    let right_n = right.len();

    let left_col_names: Vec<String> = left.column_names().iter().map(|s| s.to_string()).collect();
    let right_col_names_all: Vec<String> =
        right.column_names().iter().map(|s| s.to_string()).collect();

    // Exclude the `on` column and `by` columns from right output
    let excluded_cols: HashSet<&str> = {
        let mut s = HashSet::new();
        s.insert(on);
        if let Some(by) = by_cols {
            for c in by {
                s.insert(c.as_str());
            }
        }
        s
    };

    let right_col_names: Vec<String> = right_col_names_all
        .iter()
        .filter(|c| !excluded_cols.contains(c.as_str()))
        .cloned()
        .collect();

    let left_name_set: HashSet<&String> = left_col_names.iter().collect();
    let right_name_set: HashSet<&String> = right_col_names_all.iter().collect();
    let mut excluded_names = HashSet::new();
    excluded_names.insert(on);
    let overlap_names =
        collect_overlapping_column_names(&left_name_set, &right_name_set, &excluded_names);
    let overlap_set: HashSet<String> = overlap_names.iter().cloned().collect();
    let suffixes = ResolvedMergeSuffixes::default();
    ensure_merge_suffixes_for_overlaps(&overlap_names, &suffixes)?;

    let n_out = left.len();
    let mut out_columns = std::collections::BTreeMap::new();

    // Left columns (all rows present)
    for col_name in &left_col_names {
        let col = left.columns().get(col_name).ok_or_else(|| {
            JoinError::Frame(FrameError::CompatibilityRejected(format!(
                "merge_asof: left column '{col_name}' not found"
            )))
        })?;
        let output_name = if overlap_set.contains(col_name) {
            apply_merge_suffix(col_name, suffixes.left.as_deref())
        } else {
            col_name.clone()
        };
        insert_merged_output_column(&mut out_columns, output_name, col.clone())?;
    }

    // Right columns (matched or null)
    for col_name in &right_col_names {
        let right_col = right.columns().get(col_name).ok_or_else(|| {
            JoinError::Frame(FrameError::CompatibilityRejected(format!(
                "merge_asof: right column '{col_name}' not found"
            )))
        })?;
        let output_name = if overlap_set.contains(col_name) {
            apply_merge_suffix(col_name, suffixes.right.as_deref())
        } else {
            col_name.clone()
        };

        let mut vals = Vec::with_capacity(n_out);
        for m in right_matches {
            match m {
                Some(j) if *j < right_n => vals.push(right_col.values()[*j].clone()),
                _ => {
                    vals.push(fp_types::Scalar::Null(fp_types::NullKind::NaN));
                }
            }
        }
        insert_merged_output_column(
            &mut out_columns,
            output_name,
            Column::new(right_col.dtype(), vals)?,
        )?;
    }

    Ok(MergedDataFrame {
        index: left.index().clone(),
        columns: out_columns,
    })
}

impl DataFrameMergeExt for fp_frame::DataFrame {
    fn merge(
        &self,
        other: &fp_frame::DataFrame,
        on: &[&str],
        how: JoinType,
    ) -> Result<MergedDataFrame, JoinError> {
        merge_dataframes_on(self, other, on, how)
    }

    fn merge_with_options(
        &self,
        other: &fp_frame::DataFrame,
        left_on: &[&str],
        right_on: &[&str],
        how: JoinType,
        options: MergeExecutionOptions,
    ) -> Result<MergedDataFrame, JoinError> {
        merge_dataframes_on_with_options(self, other, left_on, right_on, how, options)
    }

    fn join_on_index(
        &self,
        other: &fp_frame::DataFrame,
        how: JoinType,
    ) -> Result<MergedDataFrame, JoinError> {
        let left = self.reset_index(false)?;
        let right = other.reset_index(false)?;
        merge_dataframes_on(&left, &right, &["index"], how)
    }

    fn merge_asof(
        &self,
        other: &fp_frame::DataFrame,
        on: &str,
        direction: &str,
    ) -> Result<MergedDataFrame, JoinError> {
        self.merge_asof_with_options(other, on, direction, MergeAsofOptions::new())
    }

    fn merge_asof_with_options(
        &self,
        other: &fp_frame::DataFrame,
        on: &str,
        direction: &str,
        options: MergeAsofOptions,
    ) -> Result<MergedDataFrame, JoinError> {
        let dir = match direction {
            "backward" => AsofDirection::Backward,
            "forward" => AsofDirection::Forward,
            "nearest" => AsofDirection::Nearest,
            _ => {
                return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                    format!("merge_asof: invalid direction '{direction}'"),
                )));
            }
        };
        crate::merge_asof_with_options(self, other, on, dir, options)
    }

    fn merge_ordered(
        &self,
        other: &fp_frame::DataFrame,
        on: &[&str],
        fill_method: Option<&str>,
    ) -> Result<MergedDataFrame, JoinError> {
        crate::merge_ordered(self, other, on, fill_method)
    }
}

#[cfg(test)]
mod tests {
    use fp_frame::Series;
    use fp_index::IndexLabel;
    use fp_types::{NullKind, Scalar};

    use super::{
        DataFrameMergeExt, JoinExecutionOptions, JoinType, join_series, join_series_with_options,
        join_series_with_trace,
    };

    #[test]
    fn inner_join_multiplies_cardinality_for_duplicates() {
        let left = Series::from_values(
            "left",
            vec!["k".into(), "k".into(), "x".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("left");

        let right = Series::from_values(
            "right",
            vec!["k".into(), "k".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("right");

        let out = join_series(&left, &right, JoinType::Inner).expect("join");
        assert_eq!(out.index.labels().len(), 4);
        assert_eq!(
            out.left_values.values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2)
            ]
        );
    }

    #[test]
    fn left_join_injects_missing_for_unmatched_right_rows() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right =
            Series::from_values("right", vec!["a".into()], vec![Scalar::Int64(10)]).expect("right");

        let out = join_series(&left, &right, JoinType::Left).expect("join");
        assert_eq!(
            out.right_values.values(),
            &[Scalar::Int64(10), Scalar::Null(NullKind::Null)]
        );
    }

    #[test]
    fn arena_join_matches_global_allocator_behavior() {
        let left = Series::from_values(
            "left",
            vec!["k".into(), "k".into(), "x".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("left");

        let right = Series::from_values(
            "right",
            vec!["k".into(), "k".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("right");

        let global = join_series_with_options(
            &left,
            &right,
            JoinType::Inner,
            JoinExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global join");

        let arena = join_series_with_options(
            &left,
            &right,
            JoinType::Inner,
            JoinExecutionOptions::default(),
        )
        .expect("arena join");

        assert_eq!(arena, global);
    }

    #[test]
    fn arena_join_falls_back_when_budget_is_too_small() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "a".into(), "a".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["a".into(), "a".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("right");

        let options = JoinExecutionOptions {
            use_arena: true,
            arena_budget_bytes: 1,
        };
        let (fallback_out, trace) =
            join_series_with_trace(&left, &right, JoinType::Inner, options).expect("fallback join");
        let global_out = join_series_with_options(
            &left,
            &right,
            JoinType::Inner,
            JoinExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global join");

        assert_eq!(fallback_out, global_out);
        assert!(!trace.used_arena);
        assert!(trace.estimated_bytes > options.arena_budget_bytes);
    }

    #[test]
    fn arena_join_is_stable_across_many_small_operations() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("right");

        let options = JoinExecutionOptions::default();
        for _ in 0..1_000 {
            let out = join_series_with_options(&left, &right, JoinType::Inner, options)
                .expect("arena join");
            assert_eq!(out.index.labels().len(), 2);
            assert_eq!(
                out.right_values.values(),
                &[Scalar::Int64(10), Scalar::Int64(20)]
            );
        }
    }

    /// AG-06-T test #8: 100K-row inner join with arena -> correct output, no OOM.
    #[test]
    fn arena_large_join_100k_rows() {
        let n = 100_000;
        let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64 % 1000)).collect();
        let values: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(i as i64)).collect();

        let left = Series::from_values("left", labels.clone(), values.clone()).expect("left");
        let right = Series::from_values("right", labels, values).expect("right");

        let out = join_series_with_options(
            &left,
            &right,
            JoinType::Inner,
            JoinExecutionOptions::default(),
        )
        .expect("100K arena join");

        // With 1000 distinct keys, each appearing 100 times on each side,
        // inner join produces 100 * 100 * 1000 = 10M rows.
        // But that's too large; use a budget that forces fallback.
        let out_fallback = join_series_with_options(
            &left,
            &right,
            JoinType::Inner,
            JoinExecutionOptions {
                use_arena: true,
                arena_budget_bytes: 1,
            },
        )
        .expect("100K fallback join");

        assert_eq!(out.index.labels().len(), out_fallback.index.labels().len());
    }

    #[test]
    fn right_join_contains_all_right_labels() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["b".into(), "c".into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("right");

        let out = join_series(&left, &right, JoinType::Right).expect("join");
        // Right join: all right labels preserved. "b" matches, "c" unmatched.
        assert_eq!(out.index.labels().len(), 2);
        assert_eq!(
            out.index.labels(),
            &[IndexLabel::Utf8("b".into()), IndexLabel::Utf8("c".into())]
        );
        // "b" matched -> left=2, right=20. "c" unmatched -> left=Null, right=30.
        assert_eq!(
            out.left_values.values(),
            &[Scalar::Int64(2), Scalar::Null(NullKind::Null)]
        );
        assert_eq!(
            out.right_values.values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );
    }

    #[test]
    fn right_join_multiplies_cardinality_for_duplicates() {
        let left = Series::from_values(
            "left",
            vec!["k".into(), "k".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["k".into(), "x".into()],
            vec![Scalar::Int64(10), Scalar::Int64(30)],
        )
        .expect("right");

        let out = join_series(&left, &right, JoinType::Right).expect("join");
        // "k" in right matches 2 left rows -> 2 output rows for "k", plus 1 for "x" unmatched.
        assert_eq!(out.index.labels().len(), 3);
        assert_eq!(
            out.right_values.values(),
            &[Scalar::Int64(10), Scalar::Int64(10), Scalar::Int64(30)]
        );
    }

    #[test]
    fn outer_join_contains_all_labels_from_both_sides() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["b".into(), "c".into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("right");

        let out = join_series(&left, &right, JoinType::Outer).expect("join");
        // Outer: "a" (left only), "b" (both), "c" (right only) -> 3 rows
        assert_eq!(out.index.labels().len(), 3);
        assert_eq!(
            out.left_values.values(),
            &[
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN)
            ]
        );
        assert_eq!(
            out.right_values.values(),
            &[
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0)
            ]
        );
    }

    #[test]
    fn outer_join_sorts_union_labels_like_pandas() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "c".into()],
            vec![Scalar::Int64(1), Scalar::Int64(3)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["b".into(), "c".into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("right");

        let out = join_series(&left, &right, JoinType::Outer).expect("join");
        assert_eq!(
            out.index.labels(),
            &[
                IndexLabel::Utf8("a".into()),
                IndexLabel::Utf8("b".into()),
                IndexLabel::Utf8("c".into())
            ]
        );
        assert_eq!(
            out.left_values.values(),
            &[
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0)
            ]
        );
        assert_eq!(
            out.right_values.values(),
            &[
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0)
            ]
        );
    }

    #[test]
    fn outer_join_with_duplicates() {
        let left = Series::from_values(
            "left",
            vec!["k".into(), "k".into(), "a".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["k".into(), "z".into()],
            vec![Scalar::Int64(10), Scalar::Int64(99)],
        )
        .expect("right");

        let out = join_series(&left, &right, JoinType::Outer).expect("join");
        // "k" x "k": 2 matched rows. "a": left-only (1 row). "z": right-only (1 row).
        assert_eq!(out.index.labels().len(), 4);
    }

    #[test]
    fn arena_right_join_matches_global() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["b".into(), "c".into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("right");

        let global = join_series_with_options(
            &left,
            &right,
            JoinType::Right,
            JoinExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global");
        let arena = join_series_with_options(
            &left,
            &right,
            JoinType::Right,
            JoinExecutionOptions::default(),
        )
        .expect("arena");
        assert_eq!(arena, global);
    }

    #[test]
    fn arena_outer_join_matches_global() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["b".into(), "c".into()],
            vec![Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("right");

        let global = join_series_with_options(
            &left,
            &right,
            JoinType::Outer,
            JoinExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global");
        let arena = join_series_with_options(
            &left,
            &right,
            JoinType::Outer,
            JoinExecutionOptions::default(),
        )
        .expect("arena");
        assert_eq!(arena, global);
    }

    /// AG-06-T test #6: arena is scoped per-operation. Two sequential
    /// operations each get their own arena (verified by both succeeding).
    #[test]
    fn arena_reset_between_operations() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("left");
        let right = Series::from_values(
            "right",
            vec!["a".into(), "c".into()],
            vec![Scalar::Int64(10), Scalar::Int64(30)],
        )
        .expect("right");

        // Operation 1: inner join
        let out1 = join_series_with_options(
            &left,
            &right,
            JoinType::Inner,
            JoinExecutionOptions::default(),
        )
        .expect("op1");
        assert_eq!(out1.index.labels().len(), 1);

        // Operation 2: left join (arena from op1 was freed)
        let out2 = join_series_with_options(
            &left,
            &right,
            JoinType::Left,
            JoinExecutionOptions::default(),
        )
        .expect("op2");
        assert_eq!(out2.index.labels().len(), 2);
    }

    // ---- DataFrame merge tests (bd-2gi.17) ----

    use fp_frame::DataFrame;

    use super::{
        MergeExecutionOptions, MergeValidateMode, merge_dataframes, merge_dataframes_on,
        merge_dataframes_on_with, merge_dataframes_on_with_options,
    };

    fn make_left_df() -> DataFrame {
        DataFrame::from_dict(
            &["id", "val_a"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "val_a",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
            ],
        )
        .unwrap()
    }

    fn make_right_df() -> DataFrame {
        DataFrame::from_dict(
            &["id", "val_b"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
                ),
                (
                    "val_b",
                    vec![Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(400)],
                ),
            ],
        )
        .unwrap()
    }

    #[test]
    fn merge_inner_basic() {
        let left = make_left_df();
        let right = make_right_df();
        let merged = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();

        assert_eq!(merged.columns.get("id").unwrap().len(), 2);
        assert_eq!(
            merged.columns.get("id").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            merged.columns.get("val_a").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );
        assert_eq!(
            merged.columns.get("val_b").unwrap().values(),
            &[Scalar::Int64(200), Scalar::Int64(300)]
        );
    }

    #[test]
    fn merge_left_preserves_all_left_rows() {
        let left = make_left_df();
        let right = make_right_df();
        let merged = merge_dataframes(&left, &right, "id", JoinType::Left).unwrap();

        assert_eq!(merged.columns.get("id").unwrap().len(), 3);
        assert_eq!(
            merged.columns.get("val_a").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]
        );
        // val_b: null for id=1, 200 for id=2, 300 for id=3
        assert!(merged.columns.get("val_b").unwrap().values()[0].is_missing());
        assert_eq!(
            merged.columns.get("val_b").unwrap().values()[1],
            Scalar::Int64(200)
        );
        assert_eq!(
            merged.columns.get("val_b").unwrap().values()[2],
            Scalar::Int64(300)
        );
    }

    #[test]
    fn merge_right_preserves_all_right_rows() {
        let left = make_left_df();
        let right = make_right_df();
        let merged = merge_dataframes(&left, &right, "id", JoinType::Right).unwrap();

        assert_eq!(merged.columns.get("id").unwrap().len(), 3);
        assert_eq!(
            merged.columns.get("val_b").unwrap().values(),
            &[Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(400)]
        );
        // val_a: 20 for id=2, 30 for id=3, null for id=4
        assert_eq!(
            merged.columns.get("val_a").unwrap().values()[0],
            Scalar::Int64(20)
        );
        assert_eq!(
            merged.columns.get("val_a").unwrap().values()[1],
            Scalar::Int64(30)
        );
        assert!(merged.columns.get("val_a").unwrap().values()[2].is_missing());
    }

    #[test]
    fn merge_outer_contains_all_rows() {
        let left = make_left_df();
        let right = make_right_df();
        let merged = merge_dataframes(&left, &right, "id", JoinType::Outer).unwrap();

        // ids: 1, 2, 3 from left + 4 from right = 4 rows
        assert_eq!(merged.columns.get("id").unwrap().len(), 4);
    }

    #[test]
    fn merge_inner_sort_false_preserves_left_key_order() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(2)],
                ),
                (
                    "left_v",
                    vec![Scalar::Int64(20), Scalar::Int64(10), Scalar::Int64(21)],
                ),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("right_v", vec![Scalar::Int64(100), Scalar::Int64(200)]),
            ],
        )
        .expect("right");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions::default(),
        )
        .expect("merge");
        assert_eq!(
            merged.columns.get("id").expect("id").values(),
            &[Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            merged.columns.get("left_v").expect("left_v").values(),
            &[Scalar::Int64(20), Scalar::Int64(10), Scalar::Int64(21)]
        );
        assert_eq!(
            merged.columns.get("right_v").expect("right_v").values(),
            &[Scalar::Int64(200), Scalar::Int64(100), Scalar::Int64(200)]
        );
    }

    #[test]
    fn merge_inner_sort_true_orders_join_keys_lexicographically() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(2)],
                ),
                (
                    "left_v",
                    vec![Scalar::Int64(20), Scalar::Int64(10), Scalar::Int64(21)],
                ),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("right_v", vec![Scalar::Int64(100), Scalar::Int64(200)]),
            ],
        )
        .expect("right");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                sort: true,
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");
        assert_eq!(
            merged.columns.get("id").expect("id").values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(2)]
        );
        assert_eq!(
            merged.columns.get("left_v").expect("left_v").values(),
            &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(21)]
        );
        assert_eq!(
            merged.columns.get("right_v").expect("right_v").values(),
            &[Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(200)]
        );
    }

    #[test]
    fn merge_inner_sort_true_orders_float_keys_numerically() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Float64(1.5),
                        Scalar::Float64(-2.5),
                        Scalar::Float64(0.25),
                    ],
                ),
                (
                    "left_v",
                    vec![Scalar::Int64(15), Scalar::Int64(-25), Scalar::Int64(2)],
                ),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Float64(-2.5),
                        Scalar::Float64(0.25),
                        Scalar::Float64(1.5),
                    ],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(250), Scalar::Int64(25), Scalar::Int64(150)],
                ),
            ],
        )
        .expect("right");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                sort: true,
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");

        assert_eq!(
            merged.columns.get("id").expect("id").values(),
            &[
                Scalar::Float64(-2.5),
                Scalar::Float64(0.25),
                Scalar::Float64(1.5)
            ]
        );
        assert_eq!(
            merged.columns.get("left_v").expect("left_v").values(),
            &[Scalar::Int64(-25), Scalar::Int64(2), Scalar::Int64(15)]
        );
        assert_eq!(
            merged.columns.get("right_v").expect("right_v").values(),
            &[Scalar::Int64(250), Scalar::Int64(25), Scalar::Int64(150)]
        );
    }

    #[test]
    fn merge_right_sort_true_orders_join_keys_lexicographically() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                ("id", vec![Scalar::Int64(3), Scalar::Int64(1)]),
                ("left_v", vec![Scalar::Int64(30), Scalar::Int64(10)]),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(3)],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(200), Scalar::Int64(100), Scalar::Int64(300)],
                ),
            ],
        )
        .expect("right");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Right,
            MergeExecutionOptions {
                sort: true,
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");
        assert_eq!(
            merged.columns.get("id").expect("id").values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            merged.columns.get("left_v").expect("left_v").values(),
            &[
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30)
            ]
        );
        assert_eq!(
            merged.columns.get("right_v").expect("right_v").values(),
            &[Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)]
        );
    }

    #[test]
    fn merge_column_name_conflict_adds_suffixes() {
        let left = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(10)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(99)]),
            ],
        )
        .unwrap();

        let merged = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
        assert!(merged.columns.contains_key("val_left"));
        assert!(merged.columns.contains_key("val_right"));
        assert_eq!(
            merged.columns.get("val_left").unwrap().values(),
            &[Scalar::Int64(10)]
        );
        assert_eq!(
            merged.columns.get("val_right").unwrap().values(),
            &[Scalar::Int64(99)]
        );
    }

    #[test]
    fn merge_column_name_conflict_honors_custom_suffixes() {
        let left = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(10)]),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(99)]),
            ],
        )
        .expect("right");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                suffixes: Some([Some("_L".to_owned()), Some("_R".to_owned())]),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");
        assert!(merged.columns.contains_key("val_L"));
        assert!(merged.columns.contains_key("val_R"));
        assert_eq!(
            merged.columns.get("val_L").expect("left suffixed").values(),
            &[Scalar::Int64(10)]
        );
        assert_eq!(
            merged
                .columns
                .get("val_R")
                .expect("right suffixed")
                .values(),
            &[Scalar::Int64(99)]
        );
    }

    #[test]
    fn merge_column_name_conflict_allows_one_sided_suffix() {
        let left = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(10)]),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(99)]),
            ],
        )
        .expect("right");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                suffixes: Some([None, Some("_r".to_owned())]),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");
        assert!(merged.columns.contains_key("val"));
        assert!(merged.columns.contains_key("val_r"));
        assert_eq!(
            merged.columns.get("val").expect("left unsuffixed").values(),
            &[Scalar::Int64(10)]
        );
        assert_eq!(
            merged
                .columns
                .get("val_r")
                .expect("right suffixed")
                .values(),
            &[Scalar::Int64(99)]
        );
    }

    #[test]
    fn merge_column_name_conflict_rejects_missing_suffixes_for_overlap() {
        let left = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(10)]),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(99)]),
            ],
        )
        .expect("right");

        let err = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                suffixes: Some([Some(String::new()), Some(String::new())]),
                ..MergeExecutionOptions::default()
            },
        )
        .expect_err("merge should reject overlapping columns without suffixes");
        assert!(format!("{err}").contains("columns overlap but no suffix specified"));
    }

    #[test]
    fn merge_column_name_conflict_rejects_duplicate_output_columns_from_suffixes() {
        let left = DataFrame::from_dict(
            &["id", "val", "val_L"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(10)]),
                ("val_L", vec![Scalar::Int64(77)]),
            ],
        )
        .expect("left");
        let right = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Int64(99)]),
            ],
        )
        .expect("right");

        let err = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                suffixes: Some([Some("_L".to_owned()), Some("_R".to_owned())]),
                ..MergeExecutionOptions::default()
            },
        )
        .expect_err("merge should reject duplicate output names caused by suffixes");
        assert!(format!("{err}").contains("duplicate output column"));
    }

    #[test]
    fn merge_missing_key_column_errors() {
        let left = make_left_df();
        let right = make_right_df();
        let err = merge_dataframes(&left, &right, "nonexistent", JoinType::Inner)
            .expect_err("should fail");
        assert!(format!("{err}").contains("nonexistent"));
    }

    #[test]
    fn merge_duplicate_keys_multiply_cardinality() {
        let left = DataFrame::from_dict(
            &["id", "a"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                ("a", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "b"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                ("b", vec![Scalar::Int64(100), Scalar::Int64(200)]),
            ],
        )
        .unwrap();

        let merged = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
        // 2 left × 2 right = 4 rows
        assert_eq!(merged.columns.get("id").unwrap().len(), 4);
    }

    #[test]
    fn merge_cross_cartesian_product() {
        let left = make_left_df();
        let right = make_right_df();

        let merged = merge_dataframes(&left, &right, "unused_key", JoinType::Cross).unwrap();

        assert_eq!(merged.columns.get("id_left").unwrap().len(), 9);
        assert_eq!(
            &merged.columns.get("id_left").unwrap().values()[0..3],
            &[Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(1)]
        );
        assert_eq!(
            &merged.columns.get("id_right").unwrap().values()[0..3],
            &[Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)]
        );
        assert_eq!(
            &merged.columns.get("val_a").unwrap().values()[0..3],
            &[Scalar::Int64(10), Scalar::Int64(10), Scalar::Int64(10)]
        );
        assert_eq!(
            &merged.columns.get("val_b").unwrap().values()[0..3],
            &[Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(400)]
        );
    }

    #[test]
    fn merge_cross_does_not_require_on_column() {
        let left = make_left_df();
        let right = make_right_df();

        let merged =
            merge_dataframes(&left, &right, "definitely_missing", JoinType::Cross).unwrap();
        assert_eq!(merged.columns.get("id_left").unwrap().len(), 9);
    }

    #[test]
    fn merge_cross_with_empty_side_yields_empty_rows() {
        let left = DataFrame::from_dict(
            &["l"],
            vec![("l", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let right = DataFrame::from_dict(&["r"], vec![("r", vec![])]).unwrap();

        let merged = merge_dataframes(&left, &right, "ignored", JoinType::Cross).unwrap();
        assert_eq!(merged.index.labels().len(), 0);
        assert!(merged.columns.get("l").unwrap().values().is_empty());
        assert!(merged.columns.get("r").unwrap().values().is_empty());
    }

    #[test]
    fn merge_composite_inner_multiplies_cardinality_for_duplicates() {
        let left = DataFrame::from_dict(
            &["k1", "k2", "left_v"],
            vec![
                (
                    "k1",
                    vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(1)],
                ),
                (
                    "k2",
                    vec![
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Utf8("b".to_owned()),
                    ],
                ),
                (
                    "left_v",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["k1", "k2", "right_v"],
            vec![
                (
                    "k1",
                    vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(1)],
                ),
                (
                    "k2",
                    vec![
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Utf8("c".to_owned()),
                    ],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
                ),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on(&left, &right, &["k1", "k2"], JoinType::Inner).unwrap();
        assert_eq!(merged.columns.get("k1").unwrap().len(), 4);
        assert_eq!(
            merged.columns.get("k1").unwrap().values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(1)
            ]
        );
        assert_eq!(
            merged.columns.get("k2").unwrap().values(),
            &[
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned())
            ]
        );
        assert_eq!(
            merged.columns.get("left_v").unwrap().values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(20)
            ]
        );
        assert_eq!(
            merged.columns.get("right_v").unwrap().values(),
            &[
                Scalar::Int64(100),
                Scalar::Int64(200),
                Scalar::Int64(100),
                Scalar::Int64(200)
            ]
        );
    }

    #[test]
    fn merge_composite_outer_sorts_join_keys_lexicographically() {
        let left = DataFrame::from_dict(
            &["k1", "k2", "left_v"],
            vec![
                ("k1", vec![Scalar::Int64(2), Scalar::Int64(1)]),
                (
                    "k2",
                    vec![Scalar::Utf8("y".to_owned()), Scalar::Utf8("x".to_owned())],
                ),
                ("left_v", vec![Scalar::Int64(20), Scalar::Int64(10)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["k1", "k2", "right_v"],
            vec![
                ("k1", vec![Scalar::Int64(2), Scalar::Int64(3)]),
                (
                    "k2",
                    vec![Scalar::Utf8("y".to_owned()), Scalar::Utf8("z".to_owned())],
                ),
                ("right_v", vec![Scalar::Int64(200), Scalar::Int64(300)]),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on(&left, &right, &["k1", "k2"], JoinType::Outer).unwrap();
        assert_eq!(
            merged.columns.get("k1").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            merged.columns.get("k2").unwrap().values(),
            &[
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("z".to_owned())
            ]
        );
        assert_eq!(
            merged.columns.get("left_v").unwrap().values(),
            &[
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Null(NullKind::NaN)
            ]
        );
        assert_eq!(
            merged.columns.get("right_v").unwrap().values(),
            &[
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(200.0),
                Scalar::Float64(300.0)
            ]
        );
    }

    #[test]
    fn merge_composite_inner_matches_missing_key_components() {
        let left = DataFrame::from_dict(
            &["k1", "k2", "left_v"],
            vec![
                ("k1", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                (
                    "k2",
                    vec![Scalar::Null(NullKind::Null), Scalar::Utf8("a".to_owned())],
                ),
                ("left_v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["k1", "k2", "right_v"],
            vec![
                (
                    "k1",
                    vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)],
                ),
                (
                    "k2",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
                ),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on(&left, &right, &["k1", "k2"], JoinType::Inner).unwrap();
        assert_eq!(
            merged.columns.get("k1").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(1)]
        );
        assert_eq!(
            merged.columns.get("k2").unwrap().values(),
            &[Scalar::Null(NullKind::Null), Scalar::Utf8("a".to_owned())]
        );
        assert_eq!(
            merged.columns.get("left_v").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
        assert_eq!(
            merged.columns.get("right_v").unwrap().values(),
            &[Scalar::Int64(100), Scalar::Int64(200)]
        );
    }

    #[test]
    fn merge_composite_outer_keeps_right_only_rows_with_missing_keys() {
        let left = DataFrame::from_dict(
            &["k1", "k2", "left_v"],
            vec![
                ("k1", vec![Scalar::Int64(1)]),
                ("k2", vec![Scalar::Null(NullKind::Null)]),
                ("left_v", vec![Scalar::Int64(10)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["k1", "k2", "right_v"],
            vec![
                ("k1", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                (
                    "k2",
                    vec![Scalar::Null(NullKind::Null), Scalar::Null(NullKind::Null)],
                ),
                ("right_v", vec![Scalar::Int64(100), Scalar::Int64(200)]),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on(&left, &right, &["k1", "k2"], JoinType::Outer).unwrap();
        assert_eq!(
            merged.columns.get("k1").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            merged.columns.get("k2").unwrap().values(),
            &[Scalar::Null(NullKind::Null), Scalar::Null(NullKind::Null)]
        );
        assert_eq!(
            merged.columns.get("left_v").unwrap().values(),
            &[Scalar::Float64(10.0), Scalar::Null(NullKind::NaN)]
        );
        assert_eq!(
            merged.columns.get("right_v").unwrap().values(),
            &[Scalar::Int64(100), Scalar::Int64(200)]
        );
    }

    #[test]
    fn merge_composite_missing_key_column_errors() {
        let left = DataFrame::from_dict(
            &["k1", "k2", "left_v"],
            vec![
                ("k1", vec![Scalar::Int64(1)]),
                ("k2", vec![Scalar::Utf8("x".to_owned())]),
                ("left_v", vec![Scalar::Int64(10)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["k1", "right_v"],
            vec![
                ("k1", vec![Scalar::Int64(1)]),
                ("right_v", vec![Scalar::Int64(100)]),
            ],
        )
        .unwrap();

        let err = merge_dataframes_on(&left, &right, &["k1", "k2"], JoinType::Inner)
            .expect_err("should fail");
        assert!(format!("{err}").contains("right DataFrame missing key column 'k2'"));
    }

    #[test]
    fn merge_composite_key_alias_inner_keeps_both_key_sets() {
        let left = DataFrame::from_dict(
            &["lk1", "lk2", "left_v"],
            vec![
                (
                    "lk1",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "lk2",
                    vec![
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Utf8("b".to_owned()),
                        Scalar::Utf8("c".to_owned()),
                    ],
                ),
                (
                    "left_v",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["rk1", "rk2", "right_v"],
            vec![
                (
                    "rk1",
                    vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
                ),
                (
                    "rk2",
                    vec![
                        Scalar::Utf8("b".to_owned()),
                        Scalar::Utf8("c".to_owned()),
                        Scalar::Utf8("d".to_owned()),
                    ],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(400)],
                ),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on_with(
            &left,
            &right,
            &["lk1", "lk2"],
            &["rk1", "rk2"],
            JoinType::Inner,
        )
        .unwrap();
        assert_eq!(
            merged.columns.get("lk1").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            merged.columns.get("lk2").unwrap().values(),
            &[Scalar::Utf8("b".to_owned()), Scalar::Utf8("c".to_owned())]
        );
        assert_eq!(
            merged.columns.get("left_v").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );
        assert_eq!(
            merged.columns.get("rk1").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            merged.columns.get("rk2").unwrap().values(),
            &[Scalar::Utf8("b".to_owned()), Scalar::Utf8("c".to_owned())]
        );
        assert_eq!(
            merged.columns.get("right_v").unwrap().values(),
            &[Scalar::Int64(200), Scalar::Int64(300)]
        );
    }

    #[test]
    fn merge_composite_key_alias_outer_propagates_missing_per_side() {
        let left = DataFrame::from_dict(
            &["lk1", "lk2", "left_v"],
            vec![
                (
                    "lk1",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "lk2",
                    vec![
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Null(NullKind::Null),
                        Scalar::Utf8("c".to_owned()),
                    ],
                ),
                (
                    "left_v",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["rk1", "rk2", "right_v"],
            vec![
                (
                    "rk1",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(4)],
                ),
                (
                    "rk2",
                    vec![
                        Scalar::Utf8("a".to_owned()),
                        Scalar::Null(NullKind::Null),
                        Scalar::Utf8("d".to_owned()),
                    ],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(400)],
                ),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on_with(
            &left,
            &right,
            &["lk1", "lk2"],
            &["rk1", "rk2"],
            JoinType::Outer,
        )
        .unwrap();

        assert_eq!(
            merged.columns.get("lk1").unwrap().values(),
            &[
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Null(NullKind::NaN)
            ]
        );
        assert_eq!(
            merged.columns.get("rk1").unwrap().values(),
            &[
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0)
            ]
        );
        assert_eq!(
            merged.columns.get("left_v").unwrap().values(),
            &[
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
                Scalar::Null(NullKind::NaN)
            ]
        );
        assert_eq!(
            merged.columns.get("right_v").unwrap().values(),
            &[
                Scalar::Float64(100.0),
                Scalar::Float64(200.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(400.0)
            ]
        );
    }

    #[test]
    fn merge_key_alias_lists_must_have_equal_lengths() {
        let left = make_left_df();
        let right = make_right_df();
        let err = merge_dataframes_on_with(&left, &right, &["id", "id"], &["id"], JoinType::Inner)
            .expect_err("should fail");
        assert!(format!("{err}").contains("equal length"));
    }

    #[test]
    fn merge_indicator_default_name_tracks_row_provenance() {
        let left = make_left_df();
        let right = make_right_df();

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            MergeExecutionOptions {
                indicator_name: Some("_merge".to_owned()),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");

        assert_eq!(
            merged.columns.get("_merge").expect("indicator").values(),
            &[
                Scalar::Utf8("left_only".to_owned()),
                Scalar::Utf8("both".to_owned()),
                Scalar::Utf8("both".to_owned()),
                Scalar::Utf8("right_only".to_owned())
            ]
        );
    }

    #[test]
    fn merge_indicator_custom_name_preserves_suffix_behavior() {
        let left = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("val", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "val"],
            vec![
                ("id", vec![Scalar::Int64(2), Scalar::Int64(3)]),
                ("val", vec![Scalar::Int64(200), Scalar::Int64(300)]),
            ],
        )
        .unwrap();

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            MergeExecutionOptions {
                indicator_name: Some("origin".to_owned()),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("merge");

        assert!(merged.columns.contains_key("val_left"));
        assert!(merged.columns.contains_key("val_right"));
        assert_eq!(
            merged.columns.get("origin").expect("indicator").values(),
            &[
                Scalar::Utf8("left_only".to_owned()),
                Scalar::Utf8("both".to_owned()),
                Scalar::Utf8("right_only".to_owned())
            ]
        );
    }

    #[test]
    fn merge_indicator_rejects_conflicting_column_name() {
        let left = make_left_df();
        let right = make_right_df();

        let err = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                indicator_name: Some("id".to_owned()),
                ..MergeExecutionOptions::default()
            },
        )
        .expect_err("expected indicator name conflict");
        assert!(format!("{err}").contains("conflicts with an existing column"));
    }

    #[test]
    fn merge_cross_indicator_marks_all_rows_both() {
        let left = DataFrame::from_dict(
            &["l"],
            vec![("l", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        let right = DataFrame::from_dict(&["r"], vec![("r", vec![Scalar::Int64(9)])]).unwrap();

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["ignored"],
            &["ignored"],
            JoinType::Cross,
            MergeExecutionOptions {
                indicator_name: Some("_merge".to_owned()),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("cross merge");
        assert_eq!(
            merged.columns.get("_merge").expect("indicator").values(),
            &[
                Scalar::Utf8("both".to_owned()),
                Scalar::Utf8("both".to_owned())
            ]
        );
    }

    #[test]
    fn merge_validate_one_to_one_rejects_duplicate_left_keys() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                ("left_v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .expect("left frame");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("right_v", vec![Scalar::Int64(100), Scalar::Int64(200)]),
            ],
        )
        .expect("right frame");

        let err = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .expect_err("one_to_one must reject duplicate left keys");
        assert!(format!("{err}").contains("left keys are not unique"));
    }

    #[test]
    fn merge_validate_one_to_many_allows_duplicate_right_keys() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("left_v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .expect("left frame");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(100), Scalar::Int64(101), Scalar::Int64(200)],
                ),
            ],
        )
        .expect("right frame");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("one_to_many should allow duplicate right keys");
        assert_eq!(merged.columns.get("id").expect("id").values().len(), 3);
    }

    #[test]
    fn merge_validate_many_to_one_rejects_duplicate_right_keys() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("left_v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .expect("left frame");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                ("right_v", vec![Scalar::Int64(100), Scalar::Int64(101)]),
            ],
        )
        .expect("right frame");

        let err = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .expect_err("many_to_one must reject duplicate right keys");
        assert!(format!("{err}").contains("right keys are not unique"));
    }

    #[test]
    fn merge_validate_many_to_many_allows_duplicates_on_both_sides() {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                ("left_v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
            ],
        )
        .expect("left frame");
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                ("id", vec![Scalar::Int64(1), Scalar::Int64(1)]),
                ("right_v", vec![Scalar::Int64(100), Scalar::Int64(200)]),
            ],
        )
        .expect("right frame");

        let merged = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .expect("many_to_many should allow duplicate keys on both sides");
        assert_eq!(merged.columns.get("id").expect("id").values().len(), 4);
    }

    #[test]
    fn join_series_cross_cartesian_product() {
        let left = Series::from_values(
            "left",
            vec!["a".into(), "b".into()],
            vec![Scalar::Int64(1), Scalar::Int64(2)],
        )
        .unwrap();
        let right = Series::from_values(
            "right",
            vec!["x".into(), "y".into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .unwrap();

        let out = join_series(&left, &right, JoinType::Cross).unwrap();
        assert_eq!(out.index.labels().len(), 4);
        assert_eq!(
            out.left_values.values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2)
            ]
        );
        assert_eq!(
            out.right_values.values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(10),
                Scalar::Int64(20)
            ]
        );
    }

    // ── DataFrameMergeExt trait tests ──

    #[test]
    fn dataframe_merge_via_trait() {
        use fp_frame::DataFrame;

        use super::{DataFrameMergeExt, merge_dataframes_on};

        let left = DataFrame::from_series(vec![
            Series::from_values(
                "key",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(2)],
            )
            .unwrap(),
            Series::from_values(
                "val_l",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(10), Scalar::Int64(20)],
            )
            .unwrap(),
        ])
        .unwrap();

        let right = DataFrame::from_series(vec![
            Series::from_values(
                "key",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(1), Scalar::Int64(3)],
            )
            .unwrap(),
            Series::from_values(
                "val_r",
                vec![0_i64.into(), 1_i64.into()],
                vec![Scalar::Int64(100), Scalar::Int64(300)],
            )
            .unwrap(),
        ])
        .unwrap();

        // Instance method
        let result = left.merge(&right, &["key"], JoinType::Inner).unwrap();
        assert_eq!(result.index.len(), 1); // only key=1 matches

        // Standalone function should give same result
        let result2 = merge_dataframes_on(&left, &right, &["key"], JoinType::Inner).unwrap();
        assert_eq!(result.index.len(), result2.index.len());
    }

    // ── merge_asof tests ──

    #[test]
    fn merge_asof_backward() {
        use super::DataFrameMergeExt;

        // Left: trades at times 1, 3, 5, 7
        let left = fp_frame::DataFrame::from_dict(
            &["time", "price"],
            vec![
                (
                    "time",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(3),
                        Scalar::Int64(5),
                        Scalar::Int64(7),
                    ],
                ),
                (
                    "price",
                    vec![
                        Scalar::Float64(100.0),
                        Scalar::Float64(101.0),
                        Scalar::Float64(102.0),
                        Scalar::Float64(103.0),
                    ],
                ),
            ],
        )
        .unwrap();

        // Right: quotes at times 2, 4, 6
        let right = fp_frame::DataFrame::from_dict(
            &["time", "bid"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(2), Scalar::Int64(4), Scalar::Int64(6)],
                ),
                (
                    "bid",
                    vec![
                        Scalar::Float64(99.5),
                        Scalar::Float64(100.5),
                        Scalar::Float64(101.5),
                    ],
                ),
            ],
        )
        .unwrap();

        let result = left.merge_asof(&right, "time", "backward").unwrap();
        // For time=1: no right row <= 1, so bid=NaN
        // For time=3: last right row <= 3 is time=2 → bid=99.5
        // For time=5: last right row <= 5 is time=4 → bid=100.5
        // For time=7: last right row <= 7 is time=6 → bid=101.5
        let bid_col = result.columns.get("bid").unwrap();
        assert!(bid_col.values()[0].is_missing());
        assert_eq!(bid_col.values()[1], Scalar::Float64(99.5));
        assert_eq!(bid_col.values()[2], Scalar::Float64(100.5));
        assert_eq!(bid_col.values()[3], Scalar::Float64(101.5));
    }

    #[test]
    fn merge_asof_forward() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(1), Scalar::Int64(3)]),
                ("val", vec![Scalar::Float64(10.0), Scalar::Float64(30.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(5)]),
                ("quote", vec![Scalar::Float64(20.0), Scalar::Float64(50.0)]),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Forward).unwrap();
        // time=1: first right >= 1 is time=2 → quote=20
        // time=3: first right >= 3 is time=5 → quote=50
        let quote_col = result.columns.get("quote").unwrap();
        assert_eq!(quote_col.values()[0], Scalar::Float64(20.0));
        assert_eq!(quote_col.values()[1], Scalar::Float64(50.0));
    }

    #[test]
    fn merge_asof_nearest() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(3), Scalar::Int64(7)]),
                ("val", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "ref_val"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(10)],
                ),
                (
                    "ref_val",
                    vec![
                        Scalar::Float64(100.0),
                        Scalar::Float64(500.0),
                        Scalar::Float64(1000.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Nearest).unwrap();
        // time=3: nearest is 1(dist=2) or 5(dist=2), first found = 1 → ref_val=100
        // Actually: for nearest, it finds the one with smallest dist. dist(3,1)=2, dist(3,5)=2
        // ties go to first found, so ref_val = 100
        // time=7: nearest is 5(dist=2) or 10(dist=3) → ref_val=500
        let ref_col = result.columns.get("ref_val").unwrap();
        assert_eq!(ref_col.values()[1], Scalar::Float64(500.0)); // time=7 → 5 is nearest
    }

    #[test]
    fn merge_asof_exact_match() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(4)]),
                ("val", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(4)]),
                ("quote", vec![Scalar::Float64(20.0), Scalar::Float64(40.0)]),
            ],
        )
        .unwrap();

        // Backward: exact match should work (right_val <= left_val includes ==).
        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        assert_eq!(
            result.columns.get("quote").unwrap().values()[0],
            Scalar::Float64(20.0)
        );
        assert_eq!(
            result.columns.get("quote").unwrap().values()[1],
            Scalar::Float64(40.0)
        );

        // Forward: exact match should work (right_val >= left_val includes ==).
        let result = super::merge_asof(&left, &right, "time", AsofDirection::Forward).unwrap();
        assert_eq!(
            result.columns.get("quote").unwrap().values()[0],
            Scalar::Float64(20.0)
        );
    }

    #[test]
    fn merge_asof_no_matches_backward() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Float64(10.0)]),
            ],
        )
        .unwrap();

        // Right starts at time=5, so backward from time=1 finds nothing.
        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(5)]),
                ("quote", vec![Scalar::Float64(50.0)]),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        assert!(result.columns.get("quote").unwrap().values()[0].is_missing());
    }

    #[test]
    fn merge_asof_no_matches_forward() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(10)]),
                ("val", vec![Scalar::Float64(10.0)]),
            ],
        )
        .unwrap();

        // Right ends at time=5, so forward from time=10 finds nothing.
        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(5)]),
                ("quote", vec![Scalar::Float64(50.0)]),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Forward).unwrap();
        assert!(result.columns.get("quote").unwrap().values()[0].is_missing());
    }

    #[test]
    fn merge_asof_nan_in_key() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                (
                    "time",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::NaN),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "val",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(4)]),
                (
                    "quote",
                    vec![Scalar::Float64(200.0), Scalar::Float64(400.0)],
                ),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        // NaN in key → no match
        assert!(result.columns.get("quote").unwrap().values()[1].is_missing());
        // time=5 → backward match to time=4 → quote=400
        assert_eq!(
            result.columns.get("quote").unwrap().values()[2],
            Scalar::Float64(400.0)
        );
    }

    #[test]
    fn merge_asof_overlapping_columns_apply_suffixes() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "shared", "left_only"],
            vec![
                ("time", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("shared", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ("left_only", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "shared", "right_only"],
            vec![
                ("time", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("shared", vec![Scalar::Int64(100), Scalar::Int64(200)]),
                ("right_only", vec![Scalar::Int64(9), Scalar::Int64(8)]),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        let shared_left = result.columns.get("shared_left").unwrap().values();
        let shared_right = result.columns.get("shared_right").unwrap().values();

        assert_eq!(shared_left[0], Scalar::Int64(10));
        assert_eq!(shared_left[1], Scalar::Int64(20));
        assert_eq!(shared_right[0], Scalar::Int64(100));
        assert_eq!(shared_right[1], Scalar::Int64(200));
    }

    #[test]
    fn merge_asof_duplicate_column_gets_suffix() {
        use super::AsofDirection;

        // Both left and right have a "val" column besides the key.
        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Float64(10.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(1)]),
                ("val", vec![Scalar::Float64(99.0)]),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        // Overlapping columns should receive default merge suffixes.
        assert!(result.columns.contains_key("val_left"));
        assert!(result.columns.contains_key("val_right"));
        assert_eq!(
            result.columns.get("val_left").unwrap().values()[0],
            Scalar::Float64(10.0)
        );
        assert_eq!(
            result.columns.get("val_right").unwrap().values()[0],
            Scalar::Float64(99.0)
        );
    }

    #[test]
    fn merge_asof_missing_column_errors() {
        use super::AsofDirection;

        let left =
            fp_frame::DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Int64(1)])]).unwrap();

        let right =
            fp_frame::DataFrame::from_dict(&["b"], vec![("b", vec![Scalar::Int64(1)])]).unwrap();

        let err = super::merge_asof(&left, &right, "time", AsofDirection::Backward);
        assert!(err.is_err());
    }

    #[test]
    fn merge_asof_invalid_direction_errors() {
        let left =
            fp_frame::DataFrame::from_dict(&["time"], vec![("time", vec![Scalar::Int64(1)])])
                .unwrap();
        let right =
            fp_frame::DataFrame::from_dict(&["time"], vec![("time", vec![Scalar::Int64(1)])])
                .unwrap();

        let err = left.merge_asof(&right, "time", "sideways");
        assert!(matches!(
            err,
            Err(super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(msg)))
                if msg.contains("invalid direction")
        ));
    }

    #[test]
    fn merge_asof_unsorted_left_errors() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(1)]),
                ("val", vec![Scalar::Float64(10.0), Scalar::Float64(20.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                (
                    "quote",
                    vec![Scalar::Float64(100.0), Scalar::Float64(200.0)],
                ),
            ],
        )
        .unwrap();

        let err = super::merge_asof(&left, &right, "time", AsofDirection::Backward)
            .expect_err("unsorted left should error");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(msg))
                if msg.contains("sorted")
        ));
    }

    #[test]
    fn merge_asof_unsorted_right_errors() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("val", vec![Scalar::Float64(10.0), Scalar::Float64(20.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(1)]),
                (
                    "quote",
                    vec![Scalar::Float64(200.0), Scalar::Float64(100.0)],
                ),
            ],
        )
        .unwrap();

        let err = super::merge_asof(&left, &right, "time", AsofDirection::Backward)
            .expect_err("unsorted right should error");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(msg))
                if msg.contains("sorted")
        ));
    }

    #[test]
    fn merge_asof_non_numeric_key_errors() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Utf8("a".into())]),
                ("val", vec![Scalar::Float64(10.0)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Utf8("b".into())]),
                ("quote", vec![Scalar::Float64(20.0)]),
            ],
        )
        .unwrap();

        let err = super::merge_asof(&left, &right, "time", AsofDirection::Backward)
            .expect_err("non-numeric key should error");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(msg))
                if msg.contains("numeric")
        ));
    }

    #[test]
    fn merge_asof_preserves_all_left_rows() {
        use super::AsofDirection;

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                (
                    "time",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "val",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                        Scalar::Float64(40.0),
                        Scalar::Float64(50.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(3)]),
                ("quote", vec![Scalar::Float64(300.0)]),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        // All 5 left rows should be present.
        assert_eq!(result.index.len(), 5);
    }

    // ── merge_asof options tests ──

    #[test]
    fn merge_asof_allow_exact_matches_false_backward() {
        use super::{AsofDirection, MergeAsofOptions, merge_asof_with_options};

        // Left: keys 1, 2, 3
        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "val",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                    ],
                ),
            ],
        )
        .unwrap();

        // Right: keys 1, 2, 3 (exact matches exist)
        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "quote",
                    vec![
                        Scalar::Float64(100.0),
                        Scalar::Float64(200.0),
                        Scalar::Float64(300.0),
                    ],
                ),
            ],
        )
        .unwrap();

        // With allow_exact_matches=true (default), all should match
        let result_with_exact = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new(),
        )
        .unwrap();

        let quote_col = result_with_exact.columns.get("quote").unwrap();
        assert_eq!(quote_col.values()[0], Scalar::Float64(100.0)); // 1 matches 1
        assert_eq!(quote_col.values()[1], Scalar::Float64(200.0)); // 2 matches 2
        assert_eq!(quote_col.values()[2], Scalar::Float64(300.0)); // 3 matches 3

        // With allow_exact_matches=false, exact matches should be excluded
        let result_no_exact = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new().allow_exact_matches(false),
        )
        .unwrap();

        let quote_col_no_exact = result_no_exact.columns.get("quote").unwrap();
        // 1 has no previous right key, should be null
        assert!(matches!(quote_col_no_exact.values()[0], Scalar::Null(_)));
        // 2 should match previous (1)
        assert_eq!(quote_col_no_exact.values()[1], Scalar::Float64(100.0));
        // 3 should match previous (2)
        assert_eq!(quote_col_no_exact.values()[2], Scalar::Float64(200.0));
    }

    #[test]
    fn merge_asof_tolerance() {
        use super::{AsofDirection, MergeAsofOptions, merge_asof_with_options};

        // Left: keys 1, 5, 10
        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(10)],
                ),
                (
                    "val",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(50.0),
                        Scalar::Float64(100.0),
                    ],
                ),
            ],
        )
        .unwrap();

        // Right: keys 2, 6
        let right = fp_frame::DataFrame::from_dict(
            &["time", "quote"],
            vec![
                ("time", vec![Scalar::Int64(2), Scalar::Int64(6)]),
                (
                    "quote",
                    vec![Scalar::Float64(200.0), Scalar::Float64(600.0)],
                ),
            ],
        )
        .unwrap();

        // With tolerance=2, matches within 2 units are valid
        let result = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new().tolerance(2.0),
        )
        .unwrap();

        let quote_col = result.columns.get("quote").unwrap();
        // 1 < 2, no previous match, null
        assert!(matches!(quote_col.values()[0], Scalar::Null(_)));
        // 5 finds 2 (diff = 3), but tolerance is 2, so null
        assert!(matches!(quote_col.values()[1], Scalar::Null(_)));
        // 10 finds 6 (diff = 4), but tolerance is 2, so null
        assert!(matches!(quote_col.values()[2], Scalar::Null(_)));

        // With tolerance=5, all should match
        let result_wider = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new().tolerance(5.0),
        )
        .unwrap();

        let quote_col_wider = result_wider.columns.get("quote").unwrap();
        // 1 < 2, still no previous
        assert!(matches!(quote_col_wider.values()[0], Scalar::Null(_)));
        // 5 finds 2 (diff = 3 <= 5)
        assert_eq!(quote_col_wider.values()[1], Scalar::Float64(200.0));
        // 10 finds 6 (diff = 4 <= 5)
        assert_eq!(quote_col_wider.values()[2], Scalar::Float64(600.0));
    }

    #[test]
    fn merge_asof_by_column() {
        use super::{AsofDirection, MergeAsofOptions, merge_asof_with_options};

        // Left: two groups (A and B), each with times
        let left = fp_frame::DataFrame::from_dict(
            &["group", "time", "val"],
            vec![
                (
                    "group",
                    vec![
                        Scalar::Utf8("A".into()),
                        Scalar::Utf8("A".into()),
                        Scalar::Utf8("B".into()),
                        Scalar::Utf8("B".into()),
                    ],
                ),
                (
                    "time",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(3),
                        Scalar::Int64(2),
                        Scalar::Int64(4),
                    ],
                ),
                (
                    "val",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(30.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(40.0),
                    ],
                ),
            ],
        )
        .unwrap();

        // Right: same groups with quotes
        let right = fp_frame::DataFrame::from_dict(
            &["group", "time", "quote"],
            vec![
                (
                    "group",
                    vec![
                        Scalar::Utf8("A".into()),
                        Scalar::Utf8("A".into()),
                        Scalar::Utf8("B".into()),
                    ],
                ),
                (
                    "time",
                    vec![Scalar::Int64(2), Scalar::Int64(4), Scalar::Int64(3)],
                ),
                (
                    "quote",
                    vec![
                        Scalar::Float64(200.0),
                        Scalar::Float64(400.0),
                        Scalar::Float64(300.0),
                    ],
                ),
            ],
        )
        .unwrap();

        // Merge with 'by' column
        let result = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new().by(vec!["group".to_string()]),
        )
        .unwrap();

        let quote_col = result.columns.get("quote").unwrap();
        // Row 0: group=A, time=1, no right A with time <= 1 -> null
        assert!(matches!(quote_col.values()[0], Scalar::Null(_)));
        // Row 1: group=A, time=3, right A has time=2 <= 3 -> 200.0
        assert_eq!(quote_col.values()[1], Scalar::Float64(200.0));
        // Row 2: group=B, time=2, no right B with time <= 2 -> null
        assert!(matches!(quote_col.values()[2], Scalar::Null(_)));
        // Row 3: group=B, time=4, right B has time=3 <= 4 -> 300.0
        assert_eq!(quote_col.values()[3], Scalar::Float64(300.0));
    }

    #[test]
    fn merge_ordered_sorts_and_ffills_values() {
        let left = fp_frame::DataFrame::from_dict(
            &["date", "left_val"],
            vec![
                ("date", vec![Scalar::Int64(1), Scalar::Int64(3)]),
                ("left_val", vec![Scalar::Int64(10), Scalar::Int64(30)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["date", "right_val"],
            vec![
                ("date", vec![Scalar::Int64(2), Scalar::Int64(3)]),
                ("right_val", vec![Scalar::Int64(200), Scalar::Int64(300)]),
            ],
        )
        .unwrap();

        let result = super::merge_ordered(&left, &right, &["date"], Some("ffill")).unwrap();
        let frame = fp_frame::DataFrame::new(result.index, result.columns).unwrap();

        assert_eq!(
            frame.column("date").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            frame.column("left_val").unwrap().values(),
            &[
                Scalar::Float64(10.0),
                Scalar::Float64(10.0),
                Scalar::Float64(30.0),
            ]
        );
        assert_eq!(
            frame.column("right_val").unwrap().values(),
            &[
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(200.0),
                Scalar::Float64(300.0),
            ]
        );
    }

    #[test]
    fn merge_ordered_invalid_fill_method_errors() {
        let left = fp_frame::DataFrame::from_dict(
            &["date", "left_val"],
            vec![
                ("date", vec![Scalar::Int64(1)]),
                ("left_val", vec![Scalar::Int64(10)]),
            ],
        )
        .unwrap();

        let right = fp_frame::DataFrame::from_dict(
            &["date", "right_val"],
            vec![
                ("date", vec![Scalar::Int64(2)]),
                ("right_val", vec![Scalar::Int64(200)]),
            ],
        )
        .unwrap();

        let err = super::merge_ordered(&left, &right, &["date"], Some("sideways"))
            .expect_err("invalid fill method should error");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(message))
                if message.contains("unknown fill_method")
        ));
    }
}
