#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

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
use fp_types::{DType, NullKind, Scalar, TypeError};
// Join build maps key on &IndexLabel / &CompositeJoinKey and are LOOKUP-only:
// output row order comes from probe-side iteration and per-key insertion-order
// position Vecs, never from map iteration. So SipHash -> FxHash (rustc-hash,
// pure safe Rust) is observationally invisible. SipHash over these label/key
// byte images is pathologically slow (cf. fp-index dedup 3-4x); FxHash collapses
// the build+probe hashing cost on the merge hot path.
use rustc_hash::{FxHashMap, FxHashSet};
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
    let right_map = if matches!(join_type, JoinType::Right) {
        FxHashMap::<&IndexLabel, Vec<usize>>::default()
    } else {
        let mut m = FxHashMap::<&IndexLabel, Vec<usize>>::default();
        for (pos, label) in right.index().labels().iter().enumerate() {
            m.entry(label).or_default().push(pos);
        }
        m
    };

    // For Right/Outer joins, also build a left_map.
    let left_map = if matches!(join_type, JoinType::Right | JoinType::Outer) {
        let mut m = FxHashMap::<&IndexLabel, Vec<usize>>::default();
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
    right_map: &FxHashMap<&IndexLabel, Vec<usize>>,
    left_map: Option<&FxHashMap<&IndexLabel, Vec<usize>>>,
    join_type: JoinType,
) -> usize {
    if matches!(join_type, JoinType::Right) {
        return right
            .index()
            .labels()
            .iter()
            .map(|label| match left_map.as_ref().and_then(|m| m.get(label)) {
                Some(matches) => matches.len(),
                None => 1,
            })
            .sum();
    }

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
        JoinType::Right => left_matched,
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
    right_map: &FxHashMap<&IndexLabel, Vec<usize>>,
    left_map: Option<&FxHashMap<&IndexLabel, Vec<usize>>>,
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

    // Per br-frankenpandas-wp0n6: pandas Series.join preserves shared
    // index name (preserved when both operands agree, None when they differ).
    let shared_name = if left.index().name().eq(&right.index().name()) {
        left.index().name().map(str::to_owned)
    } else {
        None
    };
    Ok(JoinedSeries {
        index: Index::new(out_labels).rename_index(shared_name.as_deref()),
        left_values,
        right_values,
    })
}

fn join_series_with_arena(
    left: &Series,
    right: &Series,
    join_type: JoinType,
    right_map: &FxHashMap<&IndexLabel, Vec<usize>>,
    left_map: Option<&FxHashMap<&IndexLabel, Vec<usize>>>,
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

    // Per br-frankenpandas-ceces: pandas Series.join preserves shared
    // index name. Sister to join_series fix (wp0n6).
    let shared_name = if left.index().name().eq(&right.index().name()) {
        left.index().name().map(str::to_owned)
    } else {
        None
    };
    Ok(JoinedSeries {
        index: Index::new(out_labels).rename_index(shared_name.as_deref()),
        left_values,
        right_values,
    })
}

/// Result of a DataFrame merge operation.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedDataFrame {
    pub index: Index,
    pub columns: std::collections::BTreeMap<String, Column>,
    /// Output column order in pandas' convention (key/left columns in their
    /// source order, then right non-key columns). The `columns` map is sorted,
    /// so this Vec carries the real order for materialization — br-691lh.
    pub column_order: Vec<String>,
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

/// A row's composite join key. Inline storage for the single-key case (the
/// overwhelmingly common join shape) avoids a heap allocation per row during
/// key extraction; multi-column keys spill to the heap as before. `SmallVec`
/// shares `Vec`'s `Hash`/`Eq`/`Ord`/`Deref<[T]>` semantics, so the join result
/// (key equality, grouping, output order) is bit-identical.
type CompositeJoinKey = smallvec::SmallVec<[JoinKeyComponent; 1]>;

/// `(left_positions, right_positions, out_row_keys)` produced by the merge
/// position-computation phase. `out_row_keys` is `Some` only when the rows must
/// be reordered by join key (sort or outer join).
type MergeRowPositions = (
    Vec<Option<usize>>,
    Vec<Option<usize>>,
    Option<Vec<CompositeJoinKey>>,
);
type JoinPositionBucket = smallvec::SmallVec<[usize; 1]>;

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
        let mut parts: CompositeJoinKey = smallvec::SmallVec::with_capacity(key_columns.len());
        for column in key_columns {
            parts.push(scalar_to_key_component(&column.values()[row]));
        }
        out.push(parts);
    }

    out
}

fn collect_single_join_keys(column: &Column) -> Vec<JoinKeyComponent> {
    column
        .values()
        .iter()
        .map(scalar_to_key_component)
        .collect()
}

fn has_duplicate_composite_keys(keys: &[CompositeJoinKey]) -> bool {
    let mut seen = FxHashSet::with_capacity_and_hasher(keys.len(), Default::default());
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

fn validate_mode_allows_fast_positions(validate_mode: Option<MergeValidateMode>) -> bool {
    matches!(validate_mode, None | Some(MergeValidateMode::ManyToMany))
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

fn push_merge_row_key(out_row_keys: &mut Option<Vec<CompositeJoinKey>>, key: &CompositeJoinKey) {
    if let Some(out_row_keys) = out_row_keys {
        out_row_keys.push(key.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedMergeSuffixes {
    left: Option<String>,
    right: Option<String>,
}

impl Default for ResolvedMergeSuffixes {
    fn default() -> Self {
        Self {
            left: Some("_x".to_owned()),
            right: Some("_y".to_owned()),
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
    order: &mut Vec<String>,
    name: String,
    column: Column,
) -> Result<(), JoinError> {
    if output_columns.contains_key(&name) {
        return Err(JoinError::Frame(FrameError::CompatibilityRejected(
            format!("merge suffixes cause duplicate output column '{name}'"),
        )));
    }
    order.push(name.clone());
    output_columns.insert(name, column);
    Ok(())
}

fn ordered_identity_int64_keys_match(left_key: &Column, right_key: &Column) -> bool {
    if left_key.len() != right_key.len()
        || left_key.dtype() != DType::Int64
        || right_key.dtype() != DType::Int64
        || !left_key.validity().all()
        || !right_key.validity().all()
    {
        return false;
    }

    if let (Some(left), Some(right)) = (left_key.as_i64_slice(), right_key.as_i64_slice()) {
        let mut previous = None;
        let mut strictly_increasing = true;
        for (&left_value, &right_value) in left.iter().zip(right) {
            if left_value != right_value {
                return false;
            }
            if previous.is_some_and(|previous| left_value <= previous) {
                strictly_increasing = false;
                break;
            }
            previous = Some(left_value);
        }
        if strictly_increasing {
            return true;
        }
    }

    // The identity fast path emits a 1:1 positional merge, which is only correct
    // when keys are UNIQUE. With duplicate keys pandas multiplies cardinality
    // (each duplicate on the left cross-joins every duplicate on the right), so
    // identical-but-duplicated keys must fall through to the general cartesian
    // path. (br-frankenpandas-jdupk)
    let mut seen = FxHashSet::<i64>::with_capacity_and_hasher(left_key.len(), Default::default());
    for (left, right) in left_key.values().iter().zip(right_key.values()) {
        match (left, right) {
            (Scalar::Int64(left), Scalar::Int64(right))
                if matches!(left.cmp(right), Ordering::Equal) =>
            {
                if !seen.insert(*left) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn strictly_increasing_int64_key_values(column: &Column) -> Option<&[Scalar]> {
    if column.dtype() != DType::Int64 || !column.validity().all() {
        return None;
    }

    let values = column.values();
    let mut previous = None;
    for value in values {
        let current = match value {
            Scalar::Int64(value) => *value,
            _ => return None,
        };
        if previous.is_some_and(|previous| current <= previous) {
            return None;
        }
        previous = Some(current);
    }
    Some(values)
}

fn all_valid_int64_key_values(column: &Column) -> Option<&[Scalar]> {
    if column.dtype() != DType::Int64 || !column.validity().all() {
        return None;
    }

    let values = column.values();
    if values.iter().all(|value| matches!(value, Scalar::Int64(_))) {
        Some(values)
    } else {
        None
    }
}

fn ordered_unique_int64_inner_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let left_values = strictly_increasing_int64_key_values(left_key)?;
    let right_values = strictly_increasing_int64_key_values(right_key)?;

    let mut left_positions = Vec::<usize>::with_capacity(left_values.len().min(right_values.len()));
    let mut right_positions = Vec::<usize>::with_capacity(left_positions.capacity());
    let mut left_idx = 0usize;
    let mut right_idx = 0usize;

    while left_idx < left_values.len() && right_idx < right_values.len() {
        let left_value = match &left_values[left_idx] {
            Scalar::Int64(value) => *value,
            _ => return None,
        };
        let right_value = match &right_values[right_idx] {
            Scalar::Int64(value) => *value,
            _ => return None,
        };

        match left_value.cmp(&right_value) {
            Ordering::Equal => {
                left_positions.push(left_idx);
                right_positions.push(right_idx);
                left_idx += 1;
                right_idx += 1;
            }
            Ordering::Less => left_idx += 1,
            Ordering::Greater => right_idx += 1,
        }
    }

    Some((left_positions, right_positions))
}

fn ordered_unique_int64_left_match_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<Vec<Option<usize>>> {
    let left_values = strictly_increasing_int64_key_values(left_key)?;
    let right_values = strictly_increasing_int64_key_values(right_key)?;

    let mut right_positions = Vec::<Option<usize>>::with_capacity(left_values.len());
    let mut right_idx = 0usize;

    for left_value in left_values {
        let Scalar::Int64(left_value) = left_value else {
            return None;
        };
        while let Some(Scalar::Int64(right_value)) = right_values.get(right_idx)
            && right_value < left_value
        {
            right_idx += 1;
        }

        let matched = match right_values.get(right_idx) {
            Some(Scalar::Int64(right_value)) if right_value == left_value => Some(right_idx),
            Some(Scalar::Int64(_)) | None => None,
            Some(_) => return None,
        };
        right_positions.push(matched);
    }

    Some(right_positions)
}

fn ordered_unique_int64_right_match_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<Vec<Option<usize>>> {
    let left_values = strictly_increasing_int64_key_values(left_key)?;
    let right_values = strictly_increasing_int64_key_values(right_key)?;

    let mut left_positions = Vec::<Option<usize>>::with_capacity(right_values.len());
    let mut left_idx = 0usize;

    for right_value in right_values {
        let Scalar::Int64(right_value) = right_value else {
            return None;
        };
        while let Some(Scalar::Int64(left_value)) = left_values.get(left_idx)
            && left_value < right_value
        {
            left_idx += 1;
        }

        let matched = match left_values.get(left_idx) {
            Some(Scalar::Int64(left_value)) if left_value == right_value => Some(left_idx),
            Some(Scalar::Int64(_)) | None => None,
            Some(_) => return None,
        };
        left_positions.push(matched);
    }

    Some(left_positions)
}

type OptionalJoinPositions = (Vec<Option<usize>>, Vec<Option<usize>>);

fn dense_int64_left_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<OptionalJoinPositions> {
    let left_values = all_valid_int64_key_values(left_key)?;
    let right_values = all_valid_int64_key_values(right_key)?;

    if right_values.is_empty() {
        let left_positions = (0..left_values.len()).map(Some).collect::<Vec<_>>();
        let right_positions = vec![None; left_values.len()];
        return Some((left_positions, right_positions));
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    for value in right_values {
        let Scalar::Int64(key) = value else {
            return None;
        };
        min_key = min_key.min(*key);
        max_key = max_key.max(*key);
    }

    let span = i128::from(max_key)
        .checked_sub(i128::from(min_key))?
        .checked_add(1)?;
    let row_count = left_values.len().saturating_add(right_values.len());
    let max_dense_span = row_count.saturating_mul(4).max(1024);
    if span > max_dense_span as i128 {
        return None;
    }
    let span = usize::try_from(span).ok()?;
    let span_i128 = i128::try_from(span).ok()?;

    let mut right_counts = vec![0usize; span];
    for value in right_values {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        right_counts[bucket] = right_counts[bucket].checked_add(1)?;
    }

    let mut right_offsets = Vec::<usize>::with_capacity(span + 1);
    right_offsets.push(0);
    let mut running = 0usize;
    for count in &right_counts {
        running = running.checked_add(*count)?;
        right_offsets.push(running);
    }

    let mut write_offsets = right_offsets[..span].to_vec();
    let mut right_positions_by_bucket = vec![0usize; right_values.len()];
    for (pos, value) in right_values.iter().enumerate() {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        let write_idx = write_offsets[bucket];
        right_positions_by_bucket[write_idx] = pos;
        write_offsets[bucket] = write_offsets[bucket].checked_add(1)?;
    }

    let mut output_len = 0usize;
    for value in left_values {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let offset = i128::from(*key) - i128::from(min_key);
        let bucket_rows = if (0..span_i128).contains(&offset) {
            let bucket = usize::try_from(offset).ok()?;
            right_counts[bucket].max(1)
        } else {
            1
        };
        output_len = output_len.checked_add(bucket_rows)?;
    }

    let mut left_positions = Vec::<Option<usize>>::with_capacity(output_len);
    let mut right_positions = Vec::<Option<usize>>::with_capacity(output_len);
    for (left_pos, value) in left_values.iter().enumerate() {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let offset = i128::from(*key) - i128::from(min_key);
        if (0..span_i128).contains(&offset) {
            let bucket = usize::try_from(offset).ok()?;
            let start = right_offsets[bucket];
            let end = right_offsets[bucket + 1];
            if start != end {
                for &right_pos in &right_positions_by_bucket[start..end] {
                    left_positions.push(Some(left_pos));
                    right_positions.push(Some(right_pos));
                }
                continue;
            }
        }
        left_positions.push(Some(left_pos));
        right_positions.push(None);
    }

    Some((left_positions, right_positions))
}

fn dense_int64_right_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<OptionalJoinPositions> {
    let left_values = all_valid_int64_key_values(left_key)?;
    let right_values = all_valid_int64_key_values(right_key)?;

    if left_values.is_empty() {
        let left_positions = vec![None; right_values.len()];
        let right_positions = (0..right_values.len()).map(Some).collect::<Vec<_>>();
        return Some((left_positions, right_positions));
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    for value in left_values {
        let Scalar::Int64(key) = value else {
            return None;
        };
        min_key = min_key.min(*key);
        max_key = max_key.max(*key);
    }

    let span = i128::from(max_key)
        .checked_sub(i128::from(min_key))?
        .checked_add(1)?;
    let row_count = left_values.len().saturating_add(right_values.len());
    let max_dense_span = row_count.saturating_mul(4).max(1024);
    if span > max_dense_span as i128 {
        return None;
    }
    let span = usize::try_from(span).ok()?;
    let span_i128 = i128::try_from(span).ok()?;

    let mut left_counts = vec![0usize; span];
    for value in left_values {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        left_counts[bucket] = left_counts[bucket].checked_add(1)?;
    }

    let mut left_offsets = Vec::<usize>::with_capacity(span + 1);
    left_offsets.push(0);
    let mut running = 0usize;
    for count in &left_counts {
        running = running.checked_add(*count)?;
        left_offsets.push(running);
    }

    let mut write_offsets = left_offsets[..span].to_vec();
    let mut left_positions_by_bucket = vec![0usize; left_values.len()];
    for (pos, value) in left_values.iter().enumerate() {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        let write_idx = write_offsets[bucket];
        left_positions_by_bucket[write_idx] = pos;
        write_offsets[bucket] = write_offsets[bucket].checked_add(1)?;
    }

    let mut output_len = 0usize;
    for value in right_values {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let offset = i128::from(*key) - i128::from(min_key);
        let bucket_rows = if (0..span_i128).contains(&offset) {
            let bucket = usize::try_from(offset).ok()?;
            left_counts[bucket].max(1)
        } else {
            1
        };
        output_len = output_len.checked_add(bucket_rows)?;
    }

    let mut left_positions = Vec::<Option<usize>>::with_capacity(output_len);
    let mut right_positions = Vec::<Option<usize>>::with_capacity(output_len);
    for (right_pos, value) in right_values.iter().enumerate() {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let offset = i128::from(*key) - i128::from(min_key);
        if (0..span_i128).contains(&offset) {
            let bucket = usize::try_from(offset).ok()?;
            let start = left_offsets[bucket];
            let end = left_offsets[bucket + 1];
            if start != end {
                for &left_pos in &left_positions_by_bucket[start..end] {
                    left_positions.push(Some(left_pos));
                    right_positions.push(Some(right_pos));
                }
                continue;
            }
        }
        left_positions.push(None);
        right_positions.push(Some(right_pos));
    }

    Some((left_positions, right_positions))
}

fn dense_int64_outer_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<OptionalJoinPositions> {
    let left_values = all_valid_int64_key_values(left_key)?;
    let right_values = all_valid_int64_key_values(right_key)?;

    let mut min_key = None::<i64>;
    let mut max_key = None::<i64>;
    for value in left_values.iter().chain(right_values.iter()) {
        let Scalar::Int64(key) = value else {
            return None;
        };
        min_key = Some(min_key.map_or(*key, |current| current.min(*key)));
        max_key = Some(max_key.map_or(*key, |current| current.max(*key)));
    }

    let Some(min_key) = min_key else {
        return Some((Vec::new(), Vec::new()));
    };
    let max_key = max_key.expect("max key exists when min key exists");
    let span = i128::from(max_key)
        .checked_sub(i128::from(min_key))?
        .checked_add(1)?;
    let row_count = left_values.len().saturating_add(right_values.len());
    let max_dense_span = row_count.saturating_mul(4).max(1024);
    if span > max_dense_span as i128 {
        return None;
    }
    let span = usize::try_from(span).ok()?;

    let mut left_buckets = (0..span).map(|_| Vec::<usize>::new()).collect::<Vec<_>>();
    let mut right_buckets = (0..span).map(|_| Vec::<usize>::new()).collect::<Vec<_>>();

    for (pos, value) in left_values.iter().enumerate() {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        left_buckets[bucket].push(pos);
    }
    for (pos, value) in right_values.iter().enumerate() {
        let Scalar::Int64(key) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        right_buckets[bucket].push(pos);
    }

    let mut output_len = 0usize;
    for (left_bucket, right_bucket) in left_buckets.iter().zip(right_buckets.iter()) {
        let bucket_rows = match (left_bucket.is_empty(), right_bucket.is_empty()) {
            (false, false) => left_bucket.len().checked_mul(right_bucket.len())?,
            (false, true) => left_bucket.len(),
            (true, false) => right_bucket.len(),
            (true, true) => 0,
        };
        output_len = output_len.checked_add(bucket_rows)?;
    }

    let mut left_positions = Vec::<Option<usize>>::with_capacity(output_len);
    let mut right_positions = Vec::<Option<usize>>::with_capacity(output_len);
    for (left_bucket, right_bucket) in left_buckets.into_iter().zip(right_buckets) {
        match (left_bucket.is_empty(), right_bucket.is_empty()) {
            (false, false) => {
                for left_pos in left_bucket {
                    for &right_pos in &right_bucket {
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(right_pos));
                    }
                }
            }
            (false, true) => {
                for left_pos in left_bucket {
                    left_positions.push(Some(left_pos));
                    right_positions.push(None);
                }
            }
            (true, false) => {
                for right_pos in right_bucket {
                    left_positions.push(None);
                    right_positions.push(Some(right_pos));
                }
            }
            (true, true) => {}
        }
    }

    Some((left_positions, right_positions))
}

fn ordered_unique_int64_outer_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<OptionalJoinPositions> {
    let left_values = strictly_increasing_int64_key_values(left_key)?;
    let right_values = strictly_increasing_int64_key_values(right_key)?;

    let mut left_positions =
        Vec::<Option<usize>>::with_capacity(left_values.len().saturating_add(right_values.len()));
    let mut right_positions = Vec::<Option<usize>>::with_capacity(left_positions.capacity());
    let mut left_idx = 0usize;
    let mut right_idx = 0usize;

    while left_idx < left_values.len() && right_idx < right_values.len() {
        let left_value = match &left_values[left_idx] {
            Scalar::Int64(value) => *value,
            _ => return None,
        };
        let right_value = match &right_values[right_idx] {
            Scalar::Int64(value) => *value,
            _ => return None,
        };

        match left_value.cmp(&right_value) {
            Ordering::Equal => {
                left_positions.push(Some(left_idx));
                right_positions.push(Some(right_idx));
                left_idx += 1;
                right_idx += 1;
            }
            Ordering::Less => {
                left_positions.push(Some(left_idx));
                right_positions.push(None);
                left_idx += 1;
            }
            Ordering::Greater => {
                left_positions.push(None);
                right_positions.push(Some(right_idx));
                right_idx += 1;
            }
        }
    }

    while left_idx < left_values.len() {
        left_positions.push(Some(left_idx));
        right_positions.push(None);
        left_idx += 1;
    }
    while right_idx < right_values.len() {
        left_positions.push(None);
        right_positions.push(Some(right_idx));
        right_idx += 1;
    }

    Some((left_positions, right_positions))
}

fn identity_merge_column(
    column: &Column,
    row_count: usize,
    identity_positions: &mut Option<Vec<usize>>,
) -> Column {
    if column.validity().all() {
        return column.clone();
    }

    let positions = identity_positions.get_or_insert_with(|| (0..row_count).collect());
    column.take_positions(positions)
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

/// Positional gather that keeps an all-valid Int64 output column TYPED.
///
/// `Column::take_positions` already typed-fast-paths all-valid Float64 (it
/// gathers into a contiguous `Vec<f64>` behind `lazy_all_valid_float64`), but
/// for all-valid Int64 it materializes a `Vec<Scalar::Int64>` (32 B/elem). On
/// the inner-merge output build — one gather per carried column — that Scalar
/// rebuild dominates for integer payloads. Here we gather the contiguous `i64`
/// buffer directly and hand it to `from_i64_values`, which yields the same
/// lazy-typed all-valid Int64 column.
///
/// Bit-identical to `take_positions`: for an all-valid Int64 column the latter
/// produces `Scalar::Int64(slice[pos])` at every output slot with an all-valid
/// mask; `from_i64_values(gathered)` materializes exactly those scalars with the
/// same all-valid mask (no NaN coercion path for i64), and `Column` equality
/// ignores the internal data cache. `as_i64_slice` returns `Some` only for an
/// all-valid, typed-backed Int64 column, so every other dtype / any-missing
/// column falls through to the unchanged `take_positions`.
fn take_positions_typed(column: &Column, positions: &[usize]) -> Column {
    if let Some(slice) = column.as_i64_slice() {
        let mut data = Vec::with_capacity(positions.len());
        for &pos in positions {
            data.push(slice[pos]);
        }
        return Column::from_i64_values(data);
    }
    column.take_positions(positions)
}

enum FusedInt64Side {
    Left,
    Right,
}

struct FusedInt64OutputColumn<'a> {
    name: String,
    side: FusedInt64Side,
    values: &'a [i64],
}

struct FusedInt64MergeOptions<'a> {
    suffixes: &'a ResolvedMergeSuffixes,
    require_all_left_keys_matched: bool,
}

fn build_single_key_dense_i64_inner_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_key: &Column,
    right_key: &Column,
    options: FusedInt64MergeOptions<'_>,
) -> Result<Option<MergedDataFrame>, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);

    let Some(left_keys) = left_key.as_i64_slice() else {
        return Ok(None);
    };
    let Some(right_keys) = right_key.as_i64_slice() else {
        return Ok(None);
    };

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);

    let mut specs = Vec::<FusedInt64OutputColumn<'_>>::new();
    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let Some(values) = col.as_i64_slice() else {
            return Ok(None);
        };
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, options.suffixes.left.as_deref())
        } else {
            name.clone()
        };
        specs.push(FusedInt64OutputColumn {
            name: out_name,
            side: FusedInt64Side::Left,
            values,
        });
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }
        let Some(values) = col.as_i64_slice() else {
            return Ok(None);
        };
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, options.suffixes.right.as_deref())
        } else {
            name.clone()
        };
        specs.push(FusedInt64OutputColumn {
            name: out_name,
            side: FusedInt64Side::Right,
            values,
        });
    }

    ensure_merge_suffixes_for_overlaps(&overlapping_names, options.suffixes)?;

    if left_keys.is_empty() || right_keys.is_empty() {
        if options.require_all_left_keys_matched && !left_keys.is_empty() {
            return Ok(None);
        }
        let mut columns = std::collections::BTreeMap::new();
        let mut column_order = Vec::with_capacity(specs.len());
        for spec in specs {
            insert_merged_output_column(
                &mut columns,
                &mut column_order,
                spec.name,
                Column::from_i64_values(Vec::new()),
            )?;
        }
        return Ok(Some(MergedDataFrame {
            index: Index::new(Vec::new()),
            columns,
            column_order,
        }));
    }

    let mut min = right_keys[0];
    let mut max = right_keys[0];
    for &v in right_keys {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    let span = (max as i128) - (min as i128) + 1;
    if span > (1i128 << 24) || span > 16 * (right_keys.len() as i128) {
        return Ok(None);
    }
    let range = span as usize;

    let mut offsets = vec![0usize; range + 1];
    for &v in right_keys {
        offsets[(v - min) as usize + 1] += 1;
    }
    for i in 0..range {
        offsets[i + 1] += offsets[i];
    }
    let mut positions = vec![0usize; right_keys.len()];
    let mut cursor = offsets.clone();
    for (pos, &v) in right_keys.iter().enumerate() {
        let bucket = (v - min) as usize;
        positions[cursor[bucket]] = pos;
        cursor[bucket] += 1;
    }

    let mut output_len = 0usize;
    for &v in left_keys {
        if v < min || v > max {
            if options.require_all_left_keys_matched {
                return Ok(None);
            }
            continue;
        }
        let bucket = (v - min) as usize;
        let bucket_len = offsets[bucket + 1] - offsets[bucket];
        if bucket_len == 0 {
            if options.require_all_left_keys_matched {
                return Ok(None);
            }
            continue;
        }
        let Some(new_output_len) = output_len.checked_add(bucket_len) else {
            return Ok(None);
        };
        output_len = new_output_len;
    }

    let mut output_data = specs
        .iter()
        .map(|_| Vec::<i64>::with_capacity(output_len))
        .collect::<Vec<_>>();
    for (left_pos, &v) in left_keys.iter().enumerate() {
        if v < min || v > max {
            continue;
        }
        let bucket = (v - min) as usize;
        if offsets[bucket] == offsets[bucket + 1] {
            continue;
        }
        for &right_pos in &positions[offsets[bucket]..offsets[bucket + 1]] {
            for (out, spec) in output_data.iter_mut().zip(specs.iter()) {
                match spec.side {
                    FusedInt64Side::Left => out.push(spec.values[left_pos]),
                    FusedInt64Side::Right => out.push(spec.values[right_pos]),
                }
            }
        }
    }

    let index = Index::new((0..output_len as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order = Vec::with_capacity(specs.len());
    for (spec, data) in specs.into_iter().zip(output_data) {
        debug_assert_eq!(data.len(), output_len);
        insert_merged_output_column(
            &mut columns,
            &mut column_order,
            spec.name,
            Column::from_i64_values(data),
        )?;
    }

    Ok(Some(MergedDataFrame {
        index,
        columns,
        column_order,
    }))
}

fn build_single_key_dense_i64_right_all_matched_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_key: &Column,
    right_key: &Column,
    suffixes: &ResolvedMergeSuffixes,
) -> Result<Option<MergedDataFrame>, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);

    let Some(left_keys) = left_key.as_i64_slice() else {
        return Ok(None);
    };
    let Some(right_keys) = right_key.as_i64_slice() else {
        return Ok(None);
    };

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);

    let mut specs = Vec::<FusedInt64OutputColumn<'_>>::new();
    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let (values, side) = if left_key_name_set.contains(name.as_str())
            && shared_key_names.contains(name.as_str())
        {
            let right_key_col = right
                .columns()
                .get(right_on[0])
                .expect("right key column must exist");
            let Some(values) = right_key_col.as_i64_slice() else {
                return Ok(None);
            };
            (values, FusedInt64Side::Right)
        } else {
            let Some(values) = col.as_i64_slice() else {
                return Ok(None);
            };
            (values, FusedInt64Side::Left)
        };
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        specs.push(FusedInt64OutputColumn {
            name: out_name,
            side,
            values,
        });
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }
        let Some(values) = col.as_i64_slice() else {
            return Ok(None);
        };
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        specs.push(FusedInt64OutputColumn {
            name: out_name,
            side: FusedInt64Side::Right,
            values,
        });
    }

    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    if right_keys.is_empty() {
        let mut columns = std::collections::BTreeMap::new();
        let mut column_order = Vec::with_capacity(specs.len());
        for spec in specs {
            insert_merged_output_column(
                &mut columns,
                &mut column_order,
                spec.name,
                Column::from_i64_values(Vec::new()),
            )?;
        }
        return Ok(Some(MergedDataFrame {
            index: Index::new(Vec::new()),
            columns,
            column_order,
        }));
    }
    if left_keys.is_empty() {
        return Ok(None);
    }

    let mut min = left_keys[0];
    let mut max = left_keys[0];
    for &v in left_keys {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    let span = (max as i128) - (min as i128) + 1;
    if span > (1i128 << 24) || span > 16 * (left_keys.len() as i128) {
        return Ok(None);
    }
    let range = span as usize;

    let mut offsets = vec![0usize; range + 1];
    for &v in left_keys {
        offsets[(v - min) as usize + 1] += 1;
    }
    for i in 0..range {
        offsets[i + 1] += offsets[i];
    }
    let mut positions = vec![0usize; left_keys.len()];
    let mut cursor = offsets.clone();
    for (pos, &v) in left_keys.iter().enumerate() {
        let bucket = (v - min) as usize;
        positions[cursor[bucket]] = pos;
        cursor[bucket] += 1;
    }

    let mut output_len = 0usize;
    for &v in right_keys {
        if v < min || v > max {
            return Ok(None);
        }
        let bucket = (v - min) as usize;
        let bucket_len = offsets[bucket + 1] - offsets[bucket];
        if bucket_len == 0 {
            return Ok(None);
        }
        let Some(new_output_len) = output_len.checked_add(bucket_len) else {
            return Ok(None);
        };
        output_len = new_output_len;
    }

    let mut output_data = specs
        .iter()
        .map(|_| Vec::<i64>::with_capacity(output_len))
        .collect::<Vec<_>>();
    for (right_pos, &v) in right_keys.iter().enumerate() {
        let bucket = (v - min) as usize;
        for &left_pos in &positions[offsets[bucket]..offsets[bucket + 1]] {
            for (out, spec) in output_data.iter_mut().zip(specs.iter()) {
                match spec.side {
                    FusedInt64Side::Left => out.push(spec.values[left_pos]),
                    FusedInt64Side::Right => out.push(spec.values[right_pos]),
                }
            }
        }
    }

    let index = Index::new((0..output_len as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order = Vec::with_capacity(specs.len());
    for (spec, data) in specs.into_iter().zip(output_data) {
        debug_assert_eq!(data.len(), output_len);
        insert_merged_output_column(
            &mut columns,
            &mut column_order,
            spec.name,
            Column::from_i64_values(data),
        )?;
    }

    Ok(Some(MergedDataFrame {
        index,
        columns,
        column_order,
    }))
}

fn build_single_key_dense_i64_outer_all_matched_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_key: &Column,
    right_key: &Column,
    suffixes: &ResolvedMergeSuffixes,
) -> Result<Option<MergedDataFrame>, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);

    let Some(left_keys) = left_key.as_i64_slice() else {
        return Ok(None);
    };
    let Some(right_keys) = right_key.as_i64_slice() else {
        return Ok(None);
    };

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);

    let mut specs = Vec::<FusedInt64OutputColumn<'_>>::new();
    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let Some(values) = col.as_i64_slice() else {
            return Ok(None);
        };
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        specs.push(FusedInt64OutputColumn {
            name: out_name,
            side: FusedInt64Side::Left,
            values,
        });
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }
        let Some(values) = col.as_i64_slice() else {
            return Ok(None);
        };
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        specs.push(FusedInt64OutputColumn {
            name: out_name,
            side: FusedInt64Side::Right,
            values,
        });
    }

    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    if left_keys.is_empty() || right_keys.is_empty() {
        if !left_keys.is_empty() || !right_keys.is_empty() {
            return Ok(None);
        }
        let mut columns = std::collections::BTreeMap::new();
        let mut column_order = Vec::with_capacity(specs.len());
        for spec in specs {
            insert_merged_output_column(
                &mut columns,
                &mut column_order,
                spec.name,
                Column::from_i64_values(Vec::new()),
            )?;
        }
        return Ok(Some(MergedDataFrame {
            index: Index::new(Vec::new()),
            columns,
            column_order,
        }));
    }

    let mut min_key = left_keys[0];
    let mut max_key = left_keys[0];
    for &key in left_keys.iter().chain(right_keys.iter()) {
        min_key = min_key.min(key);
        max_key = max_key.max(key);
    }

    let span = i128::from(max_key)
        .checked_sub(i128::from(min_key))
        .and_then(|span| span.checked_add(1));
    let Some(span) = span else {
        return Ok(None);
    };
    let row_count = left_keys.len().saturating_add(right_keys.len());
    let max_dense_span = row_count.saturating_mul(4).max(1024);
    if span > max_dense_span as i128 {
        return Ok(None);
    }
    let Ok(span) = usize::try_from(span) else {
        return Ok(None);
    };

    let mut left_buckets = (0..span).map(|_| Vec::<usize>::new()).collect::<Vec<_>>();
    let mut right_buckets = (0..span).map(|_| Vec::<usize>::new()).collect::<Vec<_>>();
    for (pos, &key) in left_keys.iter().enumerate() {
        let Ok(bucket) = usize::try_from(i128::from(key) - i128::from(min_key)) else {
            return Ok(None);
        };
        left_buckets[bucket].push(pos);
    }
    for (pos, &key) in right_keys.iter().enumerate() {
        let Ok(bucket) = usize::try_from(i128::from(key) - i128::from(min_key)) else {
            return Ok(None);
        };
        right_buckets[bucket].push(pos);
    }

    let mut output_len = 0usize;
    for (left_bucket, right_bucket) in left_buckets.iter().zip(right_buckets.iter()) {
        match (left_bucket.is_empty(), right_bucket.is_empty()) {
            (false, false) => {
                let Some(bucket_rows) = left_bucket.len().checked_mul(right_bucket.len()) else {
                    return Ok(None);
                };
                let Some(new_output_len) = output_len.checked_add(bucket_rows) else {
                    return Ok(None);
                };
                output_len = new_output_len;
            }
            (false, true) | (true, false) => return Ok(None),
            (true, true) => {}
        }
    }

    let index = Index::new((0..output_len as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order = Vec::with_capacity(specs.len());
    for spec in specs {
        let FusedInt64OutputColumn { name, side, values } = spec;
        let mut data = Vec::<i64>::with_capacity(output_len);
        match side {
            FusedInt64Side::Left => {
                for (left_bucket, right_bucket) in left_buckets.iter().zip(right_buckets.iter()) {
                    if left_bucket.is_empty() {
                        continue;
                    }
                    let repeat_count = right_bucket.len();
                    for &left_pos in left_bucket {
                        let value = values[left_pos];
                        for _ in 0..repeat_count {
                            data.push(value);
                        }
                    }
                }
            }
            FusedInt64Side::Right => {
                for (left_bucket, right_bucket) in left_buckets.iter().zip(right_buckets.iter()) {
                    if left_bucket.is_empty() {
                        continue;
                    }
                    for _ in left_bucket {
                        for &right_pos in right_bucket {
                            data.push(values[right_pos]);
                        }
                    }
                }
            }
        }
        debug_assert_eq!(data.len(), output_len);
        insert_merged_output_column(
            &mut columns,
            &mut column_order,
            name,
            Column::from_i64_values(data),
        )?;
    }

    Ok(Some(MergedDataFrame {
        index,
        columns,
        column_order,
    }))
}

fn build_single_key_inner_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_positions: &[usize],
    right_positions: &[usize],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(left_positions.len(), right_positions.len());

    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();

    let mut shared_name_positions = HashMap::<&str, (usize, usize)>::new();
    if left_on[0] == right_on[0] {
        shared_name_positions.insert(left_on[0], (0, 0));
    }
    let shared_key_names = shared_name_positions
        .keys()
        .copied()
        .collect::<HashSet<&str>>();
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        if left_key_name_set.contains(name.as_str()) {
            let key_column = take_positions_typed(col, left_positions);
            insert_merged_output_column(&mut columns, &mut column_order, name.clone(), key_column)?;
            continue;
        }

        let reindexed = take_positions_typed(col, left_positions);
        let out_name = if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str())
            && shared_name_positions.contains_key(name.as_str())
        {
            continue;
        }

        let reindexed = take_positions_typed(col, right_positions);
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

fn build_single_key_dense_left_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_positions: &[Option<usize>],
    right_positions: &[Option<usize>],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(left_positions.len(), right_positions.len());

    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        let output = col.reindex_by_positions(left_positions)?;
        insert_merged_output_column(&mut columns, &mut column_order, out_name, output)?;
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }

        let reindexed = col.reindex_by_positions(right_positions)?;
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

fn build_single_key_ordered_unique_left_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    right_positions: &[Option<usize>],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(left.len(), right_positions.len());

    let n = left.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();
    let mut identity_positions = None;

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        let output = identity_merge_column(col, n, &mut identity_positions);
        insert_merged_output_column(&mut columns, &mut column_order, out_name, output)?;
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }

        let reindexed = col.reindex_by_positions(right_positions)?;
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

fn build_single_key_ordered_unique_right_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_positions: &[Option<usize>],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(right.len(), left_positions.len());

    let n = right.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();
    let mut identity_positions = None;

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        let output = if left_key_name_set.contains(name.as_str())
            && shared_key_names.contains(name.as_str())
        {
            let right_key_col = right
                .columns()
                .get(right_on[0])
                .expect("right key column must exist");
            identity_merge_column(right_key_col, n, &mut identity_positions)
        } else {
            col.reindex_by_positions(left_positions)?
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, output)?;
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }

        let reindexed = identity_merge_column(col, n, &mut identity_positions);
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

fn build_single_key_dense_right_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_positions: &[Option<usize>],
    right_positions: &[Option<usize>],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(left_positions.len(), right_positions.len());

    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = if left_on[0] == right_on[0] {
        [left_on[0]].into_iter().collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        let output = if left_key_name_set.contains(name.as_str())
            && shared_key_names.contains(name.as_str())
        {
            let right_key_col = right
                .columns()
                .get(right_on[0])
                .expect("right key column must exist");
            right_key_col.reindex_by_positions(right_positions)?
        } else {
            col.reindex_by_positions(left_positions)?
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, output)?;
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str()) && shared_key_names.contains(name.as_str()) {
            continue;
        }

        let reindexed = col.reindex_by_positions(right_positions)?;
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

fn build_single_key_ordered_unique_outer_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_positions: &[Option<usize>],
    right_positions: &[Option<usize>],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(left_positions.len(), right_positions.len());

    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();

    let mut shared_name_positions = HashMap::<&str, (usize, usize)>::new();
    if left_on[0] == right_on[0] {
        shared_name_positions.insert(left_on[0], (0, 0));
    }
    let shared_key_names = shared_name_positions
        .keys()
        .copied()
        .collect::<HashSet<&str>>();
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    let mut left_take_positions = Vec::with_capacity(n);
    let mut right_take_positions = Vec::with_capacity(n);
    let mut all_positions_present = true;
    for (left_pos, right_pos) in left_positions.iter().zip(right_positions.iter()) {
        let (Some(left_pos), Some(right_pos)) = (*left_pos, *right_pos) else {
            all_positions_present = false;
            break;
        };
        left_take_positions.push(left_pos);
        right_take_positions.push(right_pos);
    }
    let all_present_take_positions =
        all_positions_present.then_some((left_take_positions, right_take_positions));
    let all_present_left_positions = all_present_take_positions
        .as_ref()
        .map(|(positions, _)| positions.as_slice());
    let all_present_right_positions = all_present_take_positions
        .as_ref()
        .map(|(_, positions)| positions.as_slice());

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        if left_key_name_set.contains(name.as_str()) {
            if let Some((left_key_idx, right_key_idx)) = shared_name_positions.get(name.as_str()) {
                let left_key_col = left
                    .columns()
                    .get(left_on[*left_key_idx])
                    .expect("left key column must exist");
                let key_column = if let Some(positions) = all_present_left_positions {
                    take_positions_typed(left_key_col, positions)
                } else {
                    let right_key_col = right
                        .columns()
                        .get(right_on[*right_key_idx])
                        .expect("right key column must exist");
                    let values = left_positions
                        .iter()
                        .zip(right_positions.iter())
                        .map(|(left_pos, right_pos)| match (left_pos, right_pos) {
                            (Some(pos), _) => left_key_col.values()[*pos].clone(),
                            (None, Some(pos)) => right_key_col.values()[*pos].clone(),
                            (None, None) => fp_types::Scalar::Null(fp_types::NullKind::Null),
                        })
                        .collect::<Vec<_>>();
                    Column::from_values(values)?
                };
                insert_merged_output_column(
                    &mut columns,
                    &mut column_order,
                    name.clone(),
                    key_column,
                )?;
            } else {
                let key_column = if let Some(positions) = all_present_left_positions {
                    take_positions_typed(col, positions)
                } else {
                    reindex_outer_join_column(col, left_positions)?
                };
                insert_merged_output_column(
                    &mut columns,
                    &mut column_order,
                    name.clone(),
                    key_column,
                )?;
            }
            continue;
        }

        let reindexed = if let Some(positions) = all_present_left_positions {
            take_positions_typed(col, positions)
        } else {
            reindex_outer_join_column(col, left_positions)?
        };
        let out_name = if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str())
            && shared_name_positions.contains_key(name.as_str())
        {
            continue;
        }

        let reindexed = if let Some(positions) = all_present_right_positions {
            take_positions_typed(col, positions)
        } else {
            reindex_outer_join_column(col, right_positions)?
        };
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

fn build_single_key_ordered_identity_inner_merge_output(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_on.len(), 1);
    debug_assert_eq!(right_on.len(), 1);
    debug_assert_eq!(left_on[0], right_on[0]);

    let n = left.len();
    debug_assert_eq!(n, right.len());
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();
    let mut identity_positions = None;

    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();
    let left_key_name_set: HashSet<&str> = left_on.iter().copied().collect();
    let right_key_name_set: HashSet<&str> = right_on.iter().copied().collect();
    let shared_key_names = left_on.iter().copied().collect::<HashSet<&str>>();
    let overlapping_names =
        collect_overlapping_column_names(&left_col_names, &right_col_names, &shared_key_names);
    ensure_merge_suffixes_for_overlaps(&overlapping_names, suffixes)?;

    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        let out_name = if left_key_name_set.contains(name.as_str()) {
            name.clone()
        } else if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        let output = identity_merge_column(col, n, &mut identity_positions);
        insert_merged_output_column(&mut columns, &mut column_order, out_name, output)?;
    }

    for name in right.column_names() {
        if right_key_name_set.contains(name.as_str()) {
            continue;
        }

        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        let output = identity_merge_column(col, n, &mut identity_positions);
        insert_merged_output_column(&mut columns, &mut column_order, out_name, output)?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
}

/// Hash-free inner-join position build for two all-valid, bounded-range Int64
/// key columns. Replaces the `FxHashMap<&JoinKeyComponent, _>` build+probe with
/// a counting-sort / CSR direct-address table indexed by `key - min`.
///
/// Bit-identical to the hash path's emitted `(left_pos, right_pos)` pairs: the
/// hash path buckets right positions in *insertion* order (ascending position),
/// and this CSR fill also writes right positions in ascending-position order,
/// so for every left row the matched right rows are emitted in the same order;
/// left rows are still probed in ascending order. All-valid Int64 keys map to
/// `Present(IndexLabel::Int64(v))` in the hash path, whose equality is plain
/// `i64 ==`, so membership/equality semantics are identical. A left key outside
/// the right key span matches nothing in either path. Returns `None` (caller
/// falls back to the hash path) for non-Int64, any-missing, or wide-span keys.
fn dense_int64_inner_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let left = left_key.as_i64_slice()?;
    let right = right_key.as_i64_slice()?;
    dense_i64_inner_positions_slices(left, right)
}

/// Core counting-sort/CSR inner-join over two `i64` key slices (see
/// [`dense_int64_inner_positions`]). Emits `(left_pos, right_pos)` pairs with
/// right positions ascending per left key, left probed ascending — matching the
/// hash path's bucket-insertion order. Returns `None` for a span wider than the
/// bounded direct-address gate so the caller falls back to hashing.
fn dense_i64_inner_positions_slices(
    left: &[i64],
    right: &[i64],
) -> Option<(Vec<usize>, Vec<usize>)> {
    if right.is_empty() || left.is_empty() {
        return Some((Vec::new(), Vec::new()));
    }

    let mut min = right[0];
    let mut max = right[0];
    for &v in right {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    // Bounded-span gate (mirrors the fp-columnar direct-address tally gates):
    // the dense table costs O(span) memory, so only take it when the span is
    // small in absolute terms AND relative to the build (right) side.
    let span = (max as i128) - (min as i128) + 1;
    if span > (1i128 << 24) || span > 16 * (right.len() as i128) {
        return None;
    }
    let range = span as usize;

    // CSR build over the right side: counts -> exclusive-prefix offsets ->
    // positions filled in ascending right-position order.
    let mut offsets = vec![0usize; range + 1];
    for &v in right {
        offsets[(v - min) as usize + 1] += 1;
    }
    for i in 0..range {
        offsets[i + 1] += offsets[i];
    }
    let mut positions = vec![0usize; right.len()];
    let mut cursor = offsets.clone();
    for (pos, &v) in right.iter().enumerate() {
        let bucket = (v - min) as usize;
        positions[cursor[bucket]] = pos;
        cursor[bucket] += 1;
    }

    let mut left_positions = Vec::<usize>::with_capacity(left.len().min(right.len()));
    let mut right_positions = Vec::<usize>::with_capacity(left_positions.capacity());
    for (left_pos, &v) in left.iter().enumerate() {
        if v < min || v > max {
            continue;
        }
        let bucket = (v - min) as usize;
        for &right_pos in &positions[offsets[bucket]..offsets[bucket + 1]] {
            left_positions.push(left_pos);
            right_positions.push(right_pos);
        }
    }
    Some((left_positions, right_positions))
}

/// Hash inner-join position build for the remaining all-valid Int64 single-key
/// shape after ordered/dense witnesses reject. This keeps the generic hash
/// path's left-major probe order and right-bucket insertion order, but hashes
/// raw `i64` keys instead of materializing `JoinKeyComponent::Present` wrappers.
fn hash_int64_inner_positions(
    left_key: &Column,
    right_key: &Column,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let left = left_key.as_i64_slice()?;
    let right = right_key.as_i64_slice()?;
    Some(hash_i64_inner_positions_slices(left, right))
}

fn hash_i64_inner_positions_slices(left: &[i64], right: &[i64]) -> (Vec<usize>, Vec<usize>) {
    if right.is_empty() || left.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut right_map = FxHashMap::<i64, JoinPositionBucket>::with_capacity_and_hasher(
        right.len(),
        Default::default(),
    );
    for (pos, &key) in right.iter().enumerate() {
        right_map.entry(key).or_default().push(pos);
    }

    let mut left_positions = Vec::<usize>::with_capacity(left.len().min(right.len()));
    let mut right_positions = Vec::<usize>::with_capacity(left_positions.capacity());
    for (left_pos, &key) in left.iter().enumerate() {
        if let Some(matches) = right_map.get(&key) {
            for &right_pos in matches {
                left_positions.push(left_pos);
                right_positions.push(right_pos);
            }
        }
    }
    (left_positions, right_positions)
}

/// Hash-free inner-join positions for a MULTI-column key whose every component
/// is an all-valid, bounded-range Int64 column. Packs each row's composite key
/// into a single `i64` via a mixed-radix code over the per-column spans
/// (`packed = Σ_c (val_c - min_c) · stride_c`, `stride_0 = 1`, `stride_c =
/// stride_{c-1}·range_{c-1}`), then runs the dense CSR core on the packed keys.
///
/// The packing is a bijection on the joint key box (mins/ranges computed over
/// BOTH sides so left- and right-side codes are comparable), so two composite
/// keys collide under packing IFF every component is equal — identical to the
/// `CompositeJoinKey` equality the FxHashMap path uses. Pair order is therefore
/// byte-for-byte the same as the hash path. Returns `None` (caller falls back to
/// the composite-hash path) for any non-Int64 / any-missing component or when
/// the packed span exceeds the bounded direct-address gate.
fn dense_packed_int64_inner_positions(
    left_cols: &[&Column],
    right_cols: &[&Column],
) -> Option<(Vec<usize>, Vec<usize>)> {
    let k = left_cols.len();
    if k == 0 || k != right_cols.len() {
        return None;
    }

    let left_slices: Vec<&[i64]> = left_cols
        .iter()
        .map(|c| c.as_i64_slice())
        .collect::<Option<Vec<_>>>()?;
    let right_slices: Vec<&[i64]> = right_cols
        .iter()
        .map(|c| c.as_i64_slice())
        .collect::<Option<Vec<_>>>()?;

    let n_left = left_slices[0].len();
    let n_right = right_slices[0].len();
    if n_left == 0 || n_right == 0 {
        return Some((Vec::new(), Vec::new()));
    }

    // Per-column min and span over BOTH sides; product = joint packed span.
    let mut mins = vec![0i64; k];
    let mut strides = vec![1i128; k];
    let mut total: i128 = 1;
    for c in 0..k {
        let mut mn = right_slices[c][0];
        let mut mx = right_slices[c][0];
        for &v in left_slices[c].iter().chain(right_slices[c].iter()) {
            if v < mn {
                mn = v;
            }
            if v > mx {
                mx = v;
            }
        }
        mins[c] = mn;
        let range = (mx as i128) - (mn as i128) + 1;
        strides[c] = total;
        total = total.checked_mul(range)?;
        // Gate the packed span the same way the single-key core does, up front
        // (the product can overflow the bounded table for wide multi-keys).
        if total > (1i128 << 24) {
            return None;
        }
    }
    if total > 16 * ((n_left + n_right) as i128) {
        return None;
    }

    let pack = |slices: &[&[i64]], n: usize| -> Vec<i64> {
        (0..n)
            .map(|row| {
                let mut acc: i128 = 0;
                for c in 0..k {
                    acc += ((slices[c][row] as i128) - (mins[c] as i128)) * strides[c];
                }
                acc as i64
            })
            .collect()
    };
    let packed_left = pack(&left_slices, n_left);
    let packed_right = pack(&right_slices, n_right);

    dense_i64_inner_positions_slices(&packed_left, &packed_right)
}

fn merge_single_key_inner_unsorted(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    left_on: &[&str],
    right_on: &[&str],
    left_key_columns: &[&Column],
    right_key_columns: &[&Column],
    suffixes: &ResolvedMergeSuffixes,
) -> Result<MergedDataFrame, JoinError> {
    debug_assert_eq!(left_key_columns.len(), 1);
    debug_assert_eq!(right_key_columns.len(), 1);

    if left_on[0] == right_on[0]
        && ordered_identity_int64_keys_match(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_ordered_identity_inner_merge_output(
            left, right, left_on, right_on, suffixes,
        );
    }
    if let Some((left_positions, right_positions)) =
        ordered_unique_int64_inner_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_inner_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            suffixes,
        );
    }

    if let Some(merged) = build_single_key_dense_i64_inner_merge_output(
        left,
        right,
        left_on,
        right_on,
        left_key_columns[0],
        right_key_columns[0],
        FusedInt64MergeOptions {
            suffixes,
            require_all_left_keys_matched: false,
        },
    )? {
        return Ok(merged);
    }

    // Hash-free dense direct-address build+probe for bounded all-valid Int64
    // keys (the common low-cardinality join-key shape). Bit-identical pairs.
    if let Some((left_positions, right_positions)) =
        dense_int64_inner_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_inner_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            suffixes,
        );
    }
    if let Some((left_positions, right_positions)) =
        hash_int64_inner_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_inner_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            suffixes,
        );
    }

    let left_keys = collect_single_join_keys(left_key_columns[0]);
    let right_keys = collect_single_join_keys(right_key_columns[0]);
    let mut right_map =
        FxHashMap::<&JoinKeyComponent, JoinPositionBucket>::with_capacity_and_hasher(
            right_keys.len(),
            Default::default(),
        );
    for (pos, key) in right_keys.iter().enumerate() {
        right_map.entry(key).or_default().push(pos);
    }

    let mut left_positions = Vec::<usize>::with_capacity(left_keys.len().min(right_keys.len()));
    let mut right_positions = Vec::<usize>::with_capacity(left_positions.capacity());
    for (left_pos, key) in left_keys.iter().enumerate() {
        if let Some(matches) = right_map.get(key) {
            for &right_pos in matches {
                left_positions.push(left_pos);
                right_positions.push(right_pos);
            }
        }
    }

    build_single_key_inner_merge_output(
        left,
        right,
        left_on,
        right_on,
        &left_positions,
        &right_positions,
        suffixes,
    )
}

/// Merge two DataFrames on one or more key columns.
///
/// Matches `pd.merge(left, right, left_on=left_keys, right_on=right_keys, how=join_type)`.
/// - Key columns are used for matching rows (hash join).
/// - Non-key columns are carried through and reindexed.
/// - Column name conflicts use configurable suffixes (default `_x`/`_y`).
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
    let validate_allows_fast_positions = validate_mode_allows_fast_positions(validate_mode);

    if matches!(join_type, JoinType::Inner)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
    {
        return merge_single_key_inner_unsorted(
            left,
            right,
            left_on,
            right_on,
            &left_key_columns,
            &right_key_columns,
            &suffixes,
        );
    }
    if matches!(join_type, JoinType::Left)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some(right_positions) =
            ordered_unique_int64_left_match_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_ordered_unique_left_merge_output(
            left,
            right,
            left_on,
            right_on,
            &right_positions,
            &suffixes,
        );
    }
    if matches!(join_type, JoinType::Left)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some(merged) = build_single_key_dense_i64_inner_merge_output(
            left,
            right,
            left_on,
            right_on,
            left_key_columns[0],
            right_key_columns[0],
            FusedInt64MergeOptions {
                suffixes: &suffixes,
                require_all_left_keys_matched: true,
            },
        )?
    {
        return Ok(merged);
    }
    if matches!(join_type, JoinType::Left)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some((left_positions, right_positions)) =
            dense_int64_left_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_dense_left_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            &suffixes,
        );
    }
    if matches!(join_type, JoinType::Right)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some(left_positions) =
            ordered_unique_int64_right_match_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_ordered_unique_right_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &suffixes,
        );
    }
    if matches!(join_type, JoinType::Right)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some(merged) = build_single_key_dense_i64_right_all_matched_merge_output(
            left,
            right,
            left_on,
            right_on,
            left_key_columns[0],
            right_key_columns[0],
            &suffixes,
        )?
    {
        return Ok(merged);
    }
    if matches!(join_type, JoinType::Right)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some((left_positions, right_positions)) =
            dense_int64_right_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_dense_right_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            &suffixes,
        );
    }
    if matches!(join_type, JoinType::Outer)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some((left_positions, right_positions)) =
            ordered_unique_int64_outer_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_ordered_unique_outer_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            &suffixes,
        );
    }
    if matches!(join_type, JoinType::Outer)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some(merged) = build_single_key_dense_i64_outer_all_matched_merge_output(
            left,
            right,
            left_on,
            right_on,
            left_key_columns[0],
            right_key_columns[0],
            &suffixes,
        )?
    {
        return Ok(merged);
    }
    if matches!(join_type, JoinType::Outer)
        && left_on.len() == 1
        && right_on.len() == 1
        && !sort
        && indicator_name.is_none()
        && validate_allows_fast_positions
        && let Some((left_positions, right_positions)) =
            dense_int64_outer_positions(left_key_columns[0], right_key_columns[0])
    {
        return build_single_key_ordered_unique_outer_merge_output(
            left,
            right,
            left_on,
            right_on,
            &left_positions,
            &right_positions,
            &suffixes,
        );
    }

    let needs_key_order = sort || matches!(join_type, JoinType::Outer);

    // Hash-free fast path: a plain inner join on all-valid bounded-Int64 key
    // column(s) packs the composite key into one i64 and runs the dense CSR
    // core, skipping CompositeJoinKey materialization + FxHashMap build/probe.
    // Gated to inner joins with no sort/indicator/enforcing validate, where the
    // emitted (left,right) pairs are byte-for-byte identical to the hash path.
    let packed_inner = if matches!(join_type, JoinType::Inner)
        && !sort
        && validate_allows_fast_positions
        && indicator_name.is_none()
    {
        dense_packed_int64_inner_positions(&left_key_columns, &right_key_columns)
    } else {
        None
    };

    let (mut left_positions, mut right_positions, out_row_keys): MergeRowPositions =
        if let Some((lp, rp)) = packed_inner {
            (
                lp.into_iter().map(Some).collect(),
                rp.into_iter().map(Some).collect(),
                None,
            )
        } else {
            // Convert key columns to hashable composite keys.
            let left_keys = collect_composite_keys(&left_key_columns);
            let right_keys = collect_composite_keys(&right_key_columns);
            if let Some(validate_mode) = validate_mode {
                validate_merge_cardinality(validate_mode, &left_keys, &right_keys)?;
            }

            // Build only the probe maps required by the selected join direction.
            let right_map = if matches!(
                join_type,
                JoinType::Inner | JoinType::Left | JoinType::Outer
            ) {
                let mut m =
                    FxHashMap::<&CompositeJoinKey, JoinPositionBucket>::with_capacity_and_hasher(
                        right_keys.len(),
                        Default::default(),
                    );
                for (pos, key) in right_keys.iter().enumerate() {
                    m.entry(key).or_default().push(pos);
                }
                Some(m)
            } else {
                None
            };

            let left_map = if matches!(join_type, JoinType::Right | JoinType::Outer) {
                let mut m =
                    FxHashMap::<&CompositeJoinKey, JoinPositionBucket>::with_capacity_and_hasher(
                        left_keys.len(),
                        Default::default(),
                    );
                for (pos, key) in left_keys.iter().enumerate() {
                    m.entry(key).or_default().push(pos);
                }
                Some(m)
            } else {
                None
            };

            // Compute row position mappings.
            let row_capacity = match join_type {
                JoinType::Inner => left_keys.len().min(right_keys.len()),
                JoinType::Left => left_keys.len(),
                JoinType::Right => right_keys.len(),
                JoinType::Outer => left_keys.len().saturating_add(right_keys.len()),
                JoinType::Cross => left_keys.len().saturating_mul(right_keys.len()),
            };
            let mut left_positions = Vec::<Option<usize>>::with_capacity(row_capacity);
            let mut right_positions = Vec::<Option<usize>>::with_capacity(row_capacity);
            let mut out_row_keys =
                needs_key_order.then(|| Vec::<CompositeJoinKey>::with_capacity(row_capacity));

            match join_type {
                JoinType::Inner | JoinType::Left | JoinType::Outer => {
                    let right_map = right_map
                        .as_ref()
                        .expect("right_map required for Inner, Left, and Outer joins");
                    for (left_pos, key) in left_keys.iter().enumerate() {
                        if let Some(matches) = right_map.get(key) {
                            for &right_pos in matches {
                                push_merge_row_key(&mut out_row_keys, key);
                                left_positions.push(Some(left_pos));
                                right_positions.push(Some(right_pos));
                            }
                            continue;
                        }

                        if matches!(join_type, JoinType::Left | JoinType::Outer) {
                            push_merge_row_key(&mut out_row_keys, key);
                            left_positions.push(Some(left_pos));
                            right_positions.push(None);
                        }
                    }

                    if matches!(join_type, JoinType::Outer) {
                        let left_map = left_map.as_ref().expect("left_map required for Outer join");
                        for (right_pos, key) in right_keys.iter().enumerate() {
                            if !left_map.contains_key(key) {
                                push_merge_row_key(&mut out_row_keys, key);
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
                                push_merge_row_key(&mut out_row_keys, key);
                                left_positions.push(Some(left_pos));
                                right_positions.push(Some(right_pos));
                            }
                            continue;
                        }

                        push_merge_row_key(&mut out_row_keys, key);
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

            (left_positions, right_positions, out_row_keys)
        };

    if needs_key_order {
        let out_row_keys = out_row_keys
            .as_ref()
            .expect("merge row keys required when sorting output rows");
        sort_merge_rows_by_join_keys(out_row_keys, &mut left_positions, &mut right_positions);
    }

    let all_positions_present = matches!(join_type, JoinType::Inner)
        || (matches!(
            join_type,
            JoinType::Left | JoinType::Right | JoinType::Outer
        ) && left_positions.iter().all(Option::is_some)
            && right_positions.iter().all(Option::is_some));
    let all_present_take_positions = all_positions_present.then(|| {
        let left = left_positions
            .iter()
            .map(|position| position.expect("all-present join emits left positions for every row"))
            .collect::<Vec<_>>();
        let right = right_positions
            .iter()
            .map(|position| position.expect("all-present join emits right positions for every row"))
            .collect::<Vec<_>>();
        (left, right)
    });
    let all_present_left_positions = all_present_take_positions
        .as_ref()
        .map(|(positions, _)| positions.as_slice());
    let all_present_right_positions = all_present_take_positions
        .as_ref()
        .map(|(_, positions)| positions.as_slice());

    // Build output columns by reindexing.
    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();
    let mut column_order: Vec<String> = Vec::new();

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

    // Add left columns (including left key columns) in the left frame's own
    // column order (pandas preserves it; the BTreeMap would re-sort) — br-691lh.
    for name in left.column_names() {
        let col = left
            .columns()
            .get(name)
            .expect("left column listed in column_names must exist");
        if left_key_name_set.contains(name.as_str()) {
            if let Some((left_key_idx, right_key_idx)) = shared_name_positions.get(name.as_str()) {
                // Shared key name: for rows emitted from right-only keys, source key values from
                // the right frame instead of leaving them null.
                let left_key_col = left_key_columns[*left_key_idx];
                let key_column = if let Some(positions) = all_present_left_positions {
                    take_positions_typed(left_key_col, positions)
                } else {
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
                    Column::from_values(values)?
                };
                insert_merged_output_column(
                    &mut columns,
                    &mut column_order,
                    name.clone(),
                    key_column,
                )?;
            } else {
                let key_column = if let Some(positions) = all_present_left_positions {
                    take_positions_typed(col, positions)
                } else if matches!(join_type, JoinType::Outer) {
                    reindex_outer_join_column(col, &left_positions)?
                } else {
                    col.reindex_by_positions(&left_positions)?
                };
                insert_merged_output_column(
                    &mut columns,
                    &mut column_order,
                    name.clone(),
                    key_column,
                )?;
            }
            continue;
        }
        let reindexed = if let Some(positions) = all_present_left_positions {
            take_positions_typed(col, positions)
        } else if matches!(join_type, JoinType::Outer) {
            reindex_outer_join_column(col, &left_positions)?
        } else {
            col.reindex_by_positions(&left_positions)?
        };
        let out_name = if right_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.left.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    // Add right non-key columns in the right frame's own column order — br-691lh.
    for name in right.column_names() {
        let col = right
            .columns()
            .get(name)
            .expect("right column listed in column_names must exist");
        if right_key_name_set.contains(name.as_str())
            && shared_name_positions.contains_key(name.as_str())
        {
            // Shared same-name key already emitted from the left side.
            continue;
        }
        let reindexed = if let Some(positions) = all_present_right_positions {
            take_positions_typed(col, positions)
        } else if matches!(join_type, JoinType::Outer) {
            reindex_outer_join_column(col, &right_positions)?
        } else {
            col.reindex_by_positions(&right_positions)?
        };
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    if let Some(indicator_name) = indicator_name.as_deref() {
        ensure_indicator_name_available(
            indicator_name,
            &left_col_names,
            &right_col_names,
            &columns,
        )?;
        let indicator_col = build_merge_indicator_column(&left_positions, &right_positions)?;
        insert_merged_output_column(
            &mut columns,
            &mut column_order,
            indicator_name.to_owned(),
            indicator_col,
        )?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
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
    let mut column_order: Vec<String> = Vec::new();

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
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    for (name, col) in right.columns() {
        let reindexed = col.reindex_by_positions(&right_positions)?;
        let out_name = if left_col_names.contains(name) {
            apply_merge_suffix(name, suffixes.right.as_deref())
        } else {
            name.clone()
        };
        insert_merged_output_column(&mut columns, &mut column_order, out_name, reindexed)?;
    }

    if let Some(indicator_name) = indicator_name {
        ensure_indicator_name_available(
            indicator_name,
            &left_col_names,
            &right_col_names,
            &columns,
        )?;
        let indicator_col = build_merge_indicator_column(&left_positions, &right_positions)?;
        insert_merged_output_column(
            &mut columns,
            &mut column_order,
            indicator_name.to_owned(),
            indicator_col,
        )?;
    }

    Ok(MergedDataFrame {
        index,
        columns,
        column_order,
    })
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

    /// pandas-named alias for [`Self::join_on_index`]. Matches
    /// `pd.DataFrame.join(other, how=...)` directly. Per br-frankenpandas-nk54a.
    ///
    /// Default impl delegates to `join_on_index`. Implementors that already
    /// have an optimized index-based join can leave this default.
    fn join(
        &self,
        other: &fp_frame::DataFrame,
        how: JoinType,
    ) -> Result<MergedDataFrame, JoinError> {
        self.join_on_index(other, how)
    }

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
        let mut df = fp_frame::DataFrame::new_with_column_order(
            merged.index.clone(),
            merged.columns.clone(),
            merged.column_order.clone(),
        )
        .map_err(JoinError::Frame)?;

        df = match method {
            "ffill" => df.ffill(None).map_err(JoinError::Frame)?,
            _ => {
                return Err(JoinError::Frame(FrameError::CompatibilityRejected(
                    format!("merge_ordered: fill_method must be 'ffill' or None, got '{method}'"),
                )));
            }
        };

        let column_order = df.column_names().into_iter().cloned().collect();
        Ok(MergedDataFrame {
            index: df.index().clone(),
            columns: df.columns().clone(),
            column_order,
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
        // pandas merge_asof accepts Timedelta64 (and datetime) `on` columns
        // and orders them by ns count. Scalar::Timedelta64.to_f64() returns
        // NonNumericValue, so the default branch would reject the join.
        // Extract ns directly for non-NaT Timedelta values; treat NaT as NaN
        // (matches the ValueIsMissing arm below).
        if let Scalar::Timedelta64(ns) = value {
            if *ns == fp_types::Timedelta::NAT {
                out.push(f64::NAN);
            } else {
                out.push(*ns as f64);
            }
            continue;
        }
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

    // pandas pd.merge_asof accepts NaN keys: the NaN row gets a null in
    // the joined columns (no match). The strict "reject null keys" check
    // previously enforced here diverged from the FP-P2D-056 oracle.
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

    // pandas pd.merge_asof accepts NaN keys: the NaN row gets a null in
    // the joined columns (no match). The strict "reject null keys" check
    // previously enforced here diverged from the FP-P2D-056 oracle.
    ensure_sorted_non_decreasing(&left_vals, "left", on)?;
    ensure_sorted_non_decreasing(&right_vals, "right", on)?;

    // Dense u32 group ids over the `by` columns (shared id space for left and
    // right). The single non-float `by` column case factorizes the column's
    // scalars with a borrowed typed key — no per-row `format!`/`Vec<String>`
    // allocation — and is bit-identical to the Debug-string grouping (see
    // `build_group_ids`). Everything else falls back to the string path.
    let (left_ids, right_ids) = build_group_ids(left, right, by_cols)?;

    // Group right rows by their `by` group id for asof matching (looked up by
    // `.get()` per left group below). No per-group sort validation is needed:
    // the global `ensure_sorted_non_decreasing` checks above already require the
    // `on` column to be non-decreasing over the whole frame, and every
    // per-group subsequence of a sorted column is itself sorted — so the old
    // per-group checks were unreachable (the global check fires first on any
    // unsorted input). FxHashMap (vs std SipHash) speeds the build.
    let mut right_groups: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (idx, &id) in right_ids.iter().enumerate() {
        right_groups.entry(id).or_default().push(idx);
    }

    // Group LEFT rows by their `by` group id (preserving the globally-sorted
    // order within each group), then run the monotonic two-pointer asof sweep
    // ONCE per group over all of the group's left values — instead of rebuilding
    // the group's right-value vector and re-scanning it from scratch for every
    // individual left row (the old O(L·R) per group). compute_asof_matches
    // already drives a single left array with a monotonic cursor and handles
    // NaN left keys (→ None, cursor unchanged); because every per-group left
    // subsequence is sorted (the global `on` column is non-decreasing), the
    // grouped sweep yields the identical match for each left row as the old
    // fresh-per-row scan. Results are scattered by absolute left index, so the
    // output order is independent of group-iteration order.
    let mut left_groups: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (left_idx, &id) in left_ids.iter().enumerate() {
        left_groups.entry(id).or_default().push(left_idx);
    }

    let mut right_matches: Vec<Option<usize>> = vec![None; left_vals.len()];
    for (group_id, left_positions) in &left_groups {
        let Some(group_indices) = right_groups.get(group_id) else {
            continue; // no right rows for this group -> all matches stay None
        };
        let group_right_vals: Vec<f64> = group_indices.iter().map(|&i| right_vals[i]).collect();
        let group_left_vals: Vec<f64> = left_positions.iter().map(|&i| left_vals[i]).collect();

        let group_matches = compute_asof_matches(
            &group_left_vals,
            &group_right_vals,
            direction,
            options.allow_exact_matches,
            options.tolerance,
        );

        for (k, &left_idx) in left_positions.iter().enumerate() {
            if let Some(Some(group_idx)) = group_matches.get(k) {
                right_matches[left_idx] = Some(group_indices[*group_idx]);
            }
        }
    }

    build_asof_output(left, right, on, &right_matches, Some(by_cols))
}

/// Borrowed, allocation-free `by`-key for the single-column factorize fast
/// path. Each variant is the typed payload of a `Scalar` whose *derived*
/// `Debug` is injective and equality-consistent — so two scalars share a
/// `ByKey` iff their `format!("{val:?}")` strings (the legacy group key) are
/// equal. `Float64` (`-0.0` vs `0.0`, `NaN`) and `Interval` Debug strings
/// disagree with value equality, so `from_scalar` returns `None` for them and
/// the caller falls back to the string path.
#[derive(PartialEq, Eq, Hash)]
enum ByKey<'a> {
    Null(u8),
    Bool(bool),
    Int(i64),
    Str(&'a str),
    Timedelta(i64),
    Datetime(i64),
    Period(i64),
}

impl<'a> ByKey<'a> {
    fn from_scalar(s: &'a Scalar) -> Option<Self> {
        Some(match s {
            Scalar::Null(NullKind::Null) => ByKey::Null(0),
            Scalar::Null(NullKind::NaN) => ByKey::Null(1),
            Scalar::Null(NullKind::NaT) => ByKey::Null(2),
            Scalar::Bool(b) => ByKey::Bool(*b),
            Scalar::Int64(v) => ByKey::Int(*v),
            Scalar::Utf8(s) => ByKey::Str(s.as_str()),
            Scalar::Timedelta64(v) => ByKey::Timedelta(*v),
            Scalar::Datetime64(v) => ByKey::Datetime(*v),
            Scalar::Period(v) => ByKey::Period(*v),
            Scalar::Float64(_) | Scalar::Interval(_) => return None,
        })
    }
}

/// Factorize a single `by` column over both frames into a shared u32 id space.
/// Returns `None` (caller falls back to the string path) if any value is a
/// `Float64`/`Interval` whose Debug string would not agree with typed equality.
fn try_factorize_typed<'a>(
    left: &'a [Scalar],
    right: &'a [Scalar],
) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut codes: FxHashMap<ByKey<'a>, u32> = FxHashMap::default();
    let mut next = 0u32;
    let mut left_ids = Vec::with_capacity(left.len());
    for v in left {
        let id = *codes.entry(ByKey::from_scalar(v)?).or_insert_with(|| {
            let i = next;
            next += 1;
            i
        });
        left_ids.push(id);
    }
    let mut right_ids = Vec::with_capacity(right.len());
    for v in right {
        let id = *codes.entry(ByKey::from_scalar(v)?).or_insert_with(|| {
            let i = next;
            next += 1;
            i
        });
        right_ids.push(id);
    }
    Some((left_ids, right_ids))
}

/// Build dense u32 group ids for the left and right frames over the `by`
/// columns, sharing one id space. Returns `(left_ids, right_ids)`.
///
/// Fast path (a single non-float `by` column): factorize the column's scalars
/// directly with the borrowed typed [`ByKey`] — zero per-row string formatting
/// or allocation. Bit-identical to the `format!("{val:?}")` grouping because
/// the typed key induces the exact same equality classes. Multi-column or
/// float/interval `by` keys route to the original Debug-string builder, whose
/// per-row composite keys are then interned to u32 (one clone per *distinct*
/// group, not per row).
fn build_group_ids(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    by_cols: &[String],
) -> Result<(Vec<u32>, Vec<u32>), JoinError> {
    if by_cols.len() == 1
        && let (Some(lcol), Some(rcol)) = (
            left.columns().get(&by_cols[0]),
            right.columns().get(&by_cols[0]),
        )
        && let Some(ids) = try_factorize_typed(lcol.values(), rcol.values())
    {
        return Ok(ids);
    }

    let left_keys = build_group_keys(left, by_cols)?;
    let right_keys = build_group_keys(right, by_cols)?;
    let mut codes: FxHashMap<Vec<String>, u32> = FxHashMap::default();
    let mut next = 0u32;
    let mut intern = |k: &Vec<String>, codes: &mut FxHashMap<Vec<String>, u32>| -> u32 {
        if let Some(&id) = codes.get(k) {
            id
        } else {
            let id = next;
            next += 1;
            codes.insert(k.clone(), id);
            id
        }
    };
    let left_ids = left_keys.iter().map(|k| intern(k, &mut codes)).collect();
    let right_ids = right_keys.iter().map(|k| intern(k, &mut codes)).collect();
    Ok((left_ids, right_ids))
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

                // For nearest, pick the closest neighbor on each side. The
                // candidate window depends on whether exact (equal) keys may
                // match:
                //   allow_exact:  lower = last value <= lv, upper = first value >= lv
                //   !allow_exact: lower = last value <  lv, upper = first value >  lv
                // Selecting the window up front (rather than excluding exact
                // matches after the fact) skips over runs of exact-equal keys
                // when they are disallowed, so a nearer non-equal key beyond
                // the run is still found — matching pandas merge_asof.
                let (lower, upper) = if allow_exact_matches {
                    let lo = right_non_nan_values.partition_point(|rv| *rv <= lv);
                    let up = right_non_nan_values.partition_point(|rv| *rv < lv);
                    (
                        if lo > 0 { Some(lo - 1) } else { None },
                        if up < right_non_nan_values.len() {
                            Some(up)
                        } else {
                            None
                        },
                    )
                } else {
                    let lo = right_non_nan_values.partition_point(|rv| *rv < lv);
                    let up = right_non_nan_values.partition_point(|rv| *rv <= lv);
                    (
                        if lo > 0 { Some(lo - 1) } else { None },
                        if up < right_non_nan_values.len() {
                            Some(up)
                        } else {
                            None
                        },
                    )
                };

                let chosen = match (lower, upper) {
                    (Some(l), Some(u)) => {
                        let lower_dist = (right_non_nan_values[l] - lv).abs();
                        let upper_dist = (right_non_nan_values[u] - lv).abs();
                        // Ties go to the lower (backward) neighbor, like pandas.
                        if upper_dist < lower_dist {
                            Some(u)
                        } else {
                            Some(l)
                        }
                    }
                    (Some(l), None) => Some(l),
                    (None, Some(u)) => Some(u),
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
    let mut column_order: Vec<String> = Vec::new();

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
        insert_merged_output_column(
            &mut out_columns,
            &mut column_order,
            output_name,
            col.clone(),
        )?;
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
            &mut column_order,
            output_name,
            Column::new(right_col.dtype(), vals)?,
        )?;
    }

    Ok(MergedDataFrame {
        index: left.index().clone(),
        columns: out_columns,
        column_order,
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

    use fp_frame::{DataFrame, FrameError};

    use super::{
        JoinError, MergeExecutionOptions, MergeValidateMode, MergedDataFrame,
        build_single_key_dense_right_merge_output, dense_int64_right_positions, merge_dataframes,
        merge_dataframes_on, merge_dataframes_on_with, merge_dataframes_on_with_options,
        ordered_identity_int64_keys_match, resolve_merge_suffixes,
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
    fn merge_preserves_pandas_column_order_for_nonalphabetical_names() {
        // br-frankenpandas-691lh: output columns follow pandas order — key/left
        // columns in the LEFT frame's source order, then right non-key columns —
        // not alphabetical (the columns map is a BTreeMap). Verified vs pandas
        // 2.2.3: merge(left[k,zebra,apple], right[k,mango], on=k)
        //   -> [k, zebra, apple, mango].
        let left = DataFrame::from_dict(
            &["k", "zebra", "apple"],
            vec![
                ("k", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("zebra", vec![Scalar::Int64(10), Scalar::Int64(20)]),
                ("apple", vec![Scalar::Int64(30), Scalar::Int64(40)]),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["k", "mango"],
            vec![
                ("k", vec![Scalar::Int64(1), Scalar::Int64(2)]),
                ("mango", vec![Scalar::Int64(50), Scalar::Int64(60)]),
            ],
        )
        .unwrap();
        let merged = merge_dataframes(&left, &right, "k", JoinType::Inner).unwrap();
        let order: Vec<&str> = merged.column_order.iter().map(String::as_str).collect();
        assert_eq!(order, ["k", "zebra", "apple", "mango"]);
    }

    fn merged_values<'a>(
        merged: &'a MergedDataFrame,
        name: &str,
    ) -> Result<&'a [Scalar], JoinError> {
        Ok(merged
            .columns
            .get(name)
            .ok_or_else(|| {
                JoinError::Frame(FrameError::CompatibilityRejected(format!(
                    "missing merged column '{name}'"
                )))
            })?
            .values())
    }

    fn merged_values_where_indicator(
        merged: &MergedDataFrame,
        column_name: &str,
        indicator_name: &str,
        indicator_value: &str,
    ) -> Result<Vec<Scalar>, JoinError> {
        let values = merged_values(merged, column_name)?;
        let indicators = merged_values(merged, indicator_name)?;
        Ok(values
            .iter()
            .zip(indicators.iter())
            .filter_map(|(value, indicator)| {
                if matches!(indicator, Scalar::Utf8(v) if v == indicator_value) {
                    Some(value.clone())
                } else {
                    None
                }
            })
            .collect())
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
    fn merge_inner_single_key_preserves_duplicate_probe_order() {
        let left = DataFrame::from_dict(
            &["id", "left_value"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(2)],
                ),
                (
                    "left_value",
                    vec![Scalar::Int64(20), Scalar::Int64(10), Scalar::Int64(21)],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "right_value"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(2), Scalar::Int64(2), Scalar::Int64(1)],
                ),
                (
                    "right_value",
                    vec![Scalar::Int64(200), Scalar::Int64(201), Scalar::Int64(100)],
                ),
            ],
        )
        .unwrap();

        let merged = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();

        assert_eq!(
            merged.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ]
        );
        assert_eq!(
            merged.columns.get("left_value").unwrap().values(),
            &[
                Scalar::Int64(20),
                Scalar::Int64(20),
                Scalar::Int64(10),
                Scalar::Int64(21),
                Scalar::Int64(21),
            ]
        );
        assert_eq!(
            merged.columns.get("right_value").unwrap().values(),
            &[
                Scalar::Int64(200),
                Scalar::Int64(201),
                Scalar::Int64(100),
                Scalar::Int64(200),
                Scalar::Int64(201),
            ]
        );
    }

    #[test]
    fn merge_inner_wide_sparse_int64_hash_matches_generic_validated_route() {
        let stride = 1_i64 << 30;
        let left = DataFrame::from_dict(
            &["id", "left_value"],
            vec![
                (
                    "id",
                    vec![3, 1, 3, 0, 2]
                        .into_iter()
                        .map(|v| Scalar::Int64(v * stride))
                        .collect(),
                ),
                (
                    "left_value",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(12),
                        Scalar::Int64(13),
                        Scalar::Int64(14),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "right_value"],
            vec![
                (
                    "id",
                    vec![3, 2, 3, 1]
                        .into_iter()
                        .map(|v| Scalar::Int64(v * stride))
                        .collect(),
                ),
                (
                    "right_value",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(101),
                        Scalar::Int64(102),
                        Scalar::Int64(103),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
        let generic = merge_dataframes_on_with_options(
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
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(3 * stride),
                Scalar::Int64(3 * stride),
                Scalar::Int64(stride),
                Scalar::Int64(3 * stride),
                Scalar::Int64(3 * stride),
                Scalar::Int64(2 * stride),
            ]
        );
        assert_eq!(
            fast.columns.get("left_value").unwrap().values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(10),
                Scalar::Int64(11),
                Scalar::Int64(12),
                Scalar::Int64(12),
                Scalar::Int64(14),
            ]
        );
        assert_eq!(
            fast.columns.get("right_value").unwrap().values(),
            &[
                Scalar::Int64(100),
                Scalar::Int64(102),
                Scalar::Int64(103),
                Scalar::Int64(100),
                Scalar::Int64(102),
                Scalar::Int64(101),
            ]
        );
    }

    #[test]
    fn merge_inner_ordered_identity_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "w",
                    vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
        let generic = merge_dataframes_on_with_options(
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
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
    }

    #[test]
    fn merge_inner_dense_int64_fused_materialization_matches_generic_route() {
        let left = DataFrame::from_dict(
            &["id", "v", "overlap"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(20),
                        Scalar::Int64(10),
                        Scalar::Int64(21),
                        Scalar::Int64(30),
                    ],
                ),
                (
                    "overlap",
                    vec![
                        Scalar::Int64(200),
                        Scalar::Int64(100),
                        Scalar::Int64(201),
                        Scalar::Int64(300),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w", "overlap"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(4),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(2000),
                        Scalar::Int64(2001),
                        Scalar::Int64(1000),
                        Scalar::Int64(4000),
                    ],
                ),
                (
                    "overlap",
                    vec![
                        Scalar::Int64(20_000),
                        Scalar::Int64(20_001),
                        Scalar::Int64(10_000),
                        Scalar::Int64(40_000),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
        let generic = merge_dataframes_on_with_options(
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
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.column_order,
            ["id", "v", "overlap_x", "w", "overlap_y"]
        );
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ]
        );
        assert_eq!(
            fast.columns.get("w").unwrap().values(),
            &[
                Scalar::Int64(2000),
                Scalar::Int64(2001),
                Scalar::Int64(1000),
                Scalar::Int64(2000),
                Scalar::Int64(2001),
            ]
        );
        assert!(
            fast.columns
                .values()
                .all(|column| column.as_i64_slice().is_some())
        );
    }

    #[test]
    fn merge_inner_ordered_unique_int64_subset_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(12),
                        Scalar::Int64(13),
                        Scalar::Int64(14),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(4),
                        Scalar::Int64(6),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(300),
                        Scalar::Int64(400),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
        let generic = merge_dataframes_on_with_options(
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
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[Scalar::Int64(0), Scalar::Int64(2), Scalar::Int64(4)]
        );
        assert_eq!(
            fast.columns.get("v").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(12), Scalar::Int64(14)]
        );
        assert_eq!(
            fast.columns.get("w").unwrap().values(),
            &[Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)]
        );
    }

    #[test]
    fn merge_inner_ordered_identity_falls_back_for_non_identity_keys() {
        let cases = [
            (
                vec![Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(4)],
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            ),
            (
                vec![
                    Scalar::Int64(1),
                    Scalar::Null(NullKind::Null),
                    Scalar::Int64(3),
                ],
                vec![
                    Scalar::Int64(1),
                    Scalar::Null(NullKind::Null),
                    Scalar::Int64(3),
                ],
            ),
        ];

        for (left_keys, right_keys) in cases {
            let left = DataFrame::from_dict(
                &["id", "v"],
                vec![
                    ("id", left_keys),
                    (
                        "v",
                        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                    ),
                ],
            )
            .unwrap();
            let right = DataFrame::from_dict(
                &["id", "w"],
                vec![
                    ("id", right_keys),
                    (
                        "w",
                        vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
                    ),
                ],
            )
            .unwrap();

            let maybe_fast = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();
            let generic = merge_dataframes_on_with_options(
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
            .unwrap();

            assert_eq!(maybe_fast.index, generic.index);
            assert_eq!(maybe_fast.column_order, generic.column_order);
            assert_eq!(maybe_fast.columns, generic.columns);
        }
    }

    #[test]
    fn merge_default_suffixes_match_pandas() {
        // Overlapping non-key column "v" gets pandas default _x/_y suffixes.
        // Verified vs pandas 2.2.3: left{k:[1,2,3],v:[10,20,30]} merged inner on
        // k with right{k:[2,3,4],v:[200,300,400]} -> cols [k,v_x,v_y],
        // k=[2,3], v_x=[20,30], v_y=[200,300].
        let idx: Vec<_> = (0..3).map(|i| (i as i64).into()).collect();
        let left = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "k",
                idx.clone(),
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "v",
                idx.clone(),
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        ])
        .unwrap();
        let right = fp_frame::DataFrame::from_series(vec![
            fp_frame::Series::from_values(
                "k",
                idx.clone(),
                vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
            )
            .unwrap(),
            fp_frame::Series::from_values(
                "v",
                idx,
                vec![Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(400)],
            )
            .unwrap(),
        ])
        .unwrap();
        let merged = merge_dataframes(&left, &right, "k", JoinType::Inner).unwrap();

        assert_eq!(
            merged.columns.get("k").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(3)],
            "k"
        );
        assert_eq!(
            merged.columns.get("v_x").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(30)],
            "v_x"
        );
        assert_eq!(
            merged.columns.get("v_y").unwrap().values(),
            &[Scalar::Int64(200), Scalar::Int64(300)],
            "v_y"
        );
        assert!(
            !merged.columns.contains_key("v"),
            "bare 'v' should be suffixed away"
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
    fn merge_left_ordered_unique_int64_subset_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(12),
                        Scalar::Int64(13),
                        Scalar::Int64(14),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(4),
                        Scalar::Int64(6),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(300),
                        Scalar::Int64(400),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Left).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Left,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4)
            ]
        );
        let right_values = fast.columns.get("w").unwrap().values();
        assert_eq!(right_values[0], Scalar::Int64(100));
        assert!(right_values[1].is_missing());
        assert_eq!(right_values[2], Scalar::Int64(200));
        assert!(right_values[3].is_missing());
        assert_eq!(right_values[4], Scalar::Int64(300));
    }

    #[test]
    fn merge_left_dense_int64_duplicates_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(12),
                        Scalar::Int64(13),
                        Scalar::Int64(15),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(0),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(200),
                        Scalar::Int64(201),
                        Scalar::Int64(300),
                        Scalar::Int64(400),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Left).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Left,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(0),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(5),
            ]
        );
        let left_values = fast.columns.get("v").unwrap().values();
        assert_eq!(left_values[0], Scalar::Int64(10));
        assert_eq!(left_values[1], Scalar::Int64(10));
        assert_eq!(left_values[2], Scalar::Int64(11));
        assert_eq!(left_values[4], Scalar::Int64(12));
        assert_eq!(left_values[6], Scalar::Int64(15));
        let right_values = fast.columns.get("w").unwrap().values();
        assert_eq!(right_values[0], Scalar::Int64(200));
        assert_eq!(right_values[1], Scalar::Int64(201));
        assert_eq!(right_values[2], Scalar::Int64(400));
        assert!(right_values[5].is_missing());
        assert!(right_values[6].is_missing());
    }

    #[test]
    fn merge_left_all_matched_dense_int64_fused_output_matches_sorted_generic_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(0),
                        Scalar::Int64(1),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(20),
                        Scalar::Int64(21),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(0),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(101),
                        Scalar::Int64(200),
                        Scalar::Int64(300),
                        Scalar::Int64(301),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Left).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Left,
            MergeExecutionOptions {
                sort: true,
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(0),
                Scalar::Int64(0),
                Scalar::Int64(0),
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ]
        );
        assert_eq!(
            fast.columns.get("v").unwrap().values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(10),
                Scalar::Int64(11),
                Scalar::Int64(11),
                Scalar::Int64(20),
                Scalar::Int64(21),
                Scalar::Int64(30),
                Scalar::Int64(30),
            ]
        );
        assert_eq!(
            fast.columns.get("w").unwrap().values(),
            &[
                Scalar::Int64(100),
                Scalar::Int64(101),
                Scalar::Int64(100),
                Scalar::Int64(101),
                Scalar::Int64(200),
                Scalar::Int64(200),
                Scalar::Int64(300),
                Scalar::Int64(301),
            ]
        );
    }

    #[test]
    fn merge_outer_ordered_unique_int64_subset_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(0), Scalar::Int64(1), Scalar::Int64(3)],
                ),
                (
                    "v",
                    vec![Scalar::Int64(10), Scalar::Int64(11), Scalar::Int64(13)],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(300),
                        Scalar::Int64(500),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Outer).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(5)
            ]
        );
        let left_values = fast.columns.get("v").unwrap().values();
        assert_eq!(left_values[0], Scalar::Float64(10.0));
        assert_eq!(left_values[1], Scalar::Float64(11.0));
        assert!(left_values[2].is_missing());
        assert_eq!(left_values[3], Scalar::Float64(13.0));
        assert!(left_values[4].is_missing());
        let right_values = fast.columns.get("w").unwrap().values();
        assert_eq!(right_values[0], Scalar::Float64(100.0));
        assert!(right_values[1].is_missing());
        assert_eq!(right_values[2], Scalar::Float64(200.0));
        assert_eq!(right_values[3], Scalar::Float64(300.0));
        assert_eq!(right_values[4], Scalar::Float64(500.0));
    }

    #[test]
    fn merge_outer_dense_int64_duplicates_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(12),
                        Scalar::Int64(13),
                        Scalar::Int64(15),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(0),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(200),
                        Scalar::Int64(201),
                        Scalar::Int64(300),
                        Scalar::Int64(400),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Outer).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(5),
            ]
        );
        let left_values = fast.columns.get("v").unwrap().values();
        assert_eq!(left_values[0], Scalar::Float64(11.0));
        assert_eq!(left_values[1], Scalar::Float64(13.0));
        assert_eq!(left_values[2], Scalar::Float64(10.0));
        assert_eq!(left_values[5], Scalar::Float64(12.0));
        assert!(left_values[6].is_missing());
        assert_eq!(left_values[7], Scalar::Float64(15.0));
        let right_values = fast.columns.get("w").unwrap().values();
        assert_eq!(right_values[0], Scalar::Float64(400.0));
        assert!(right_values[1].is_missing());
        assert_eq!(right_values[2], Scalar::Float64(200.0));
        assert_eq!(right_values[3], Scalar::Float64(201.0));
        assert!(right_values[7].is_missing());
    }

    #[test]
    fn merge_outer_all_matched_dense_int64_duplicates_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(20),
                        Scalar::Int64(10),
                        Scalar::Int64(21),
                        Scalar::Int64(11),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(101),
                        Scalar::Int64(201),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Outer).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            generic.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ]
        );
        assert!(
            generic
                .columns
                .values()
                .all(|column| column.as_i64_slice().is_some())
        );
    }

    #[test]
    fn merge_outer_all_matched_dense_int64_fused_output_matches_sorted_generic_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(20),
                        Scalar::Int64(10),
                        Scalar::Int64(21),
                        Scalar::Int64(11),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(101),
                        Scalar::Int64(201),
                    ],
                ),
            ],
        )
        .unwrap();

        let fused = merge_dataframes(&left, &right, "id", JoinType::Outer).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                sort: true,
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fused.index, generic.index);
        assert_eq!(fused.column_order, generic.column_order);
        assert_eq!(fused.columns, generic.columns);
    }

    #[test]
    fn merge_right_dense_int64_duplicates_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(11),
                        Scalar::Int64(12),
                        Scalar::Int64(13),
                        Scalar::Int64(15),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(0),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(200),
                        Scalar::Int64(201),
                        Scalar::Int64(300),
                        Scalar::Int64(400),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Right).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Right,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(0),
            ]
        );
        let left_values = fast.columns.get("v").unwrap().values();
        assert_eq!(left_values[0], Scalar::Int64(10));
        assert_eq!(left_values[1], Scalar::Int64(12));
        assert_eq!(left_values[2], Scalar::Int64(10));
        assert_eq!(left_values[3], Scalar::Int64(12));
        assert!(left_values[4].is_missing());
        assert_eq!(left_values[5], Scalar::Int64(11));
        let right_values = fast.columns.get("w").unwrap().values();
        assert_eq!(right_values[0], Scalar::Int64(200));
        assert_eq!(right_values[1], Scalar::Int64(200));
        assert_eq!(right_values[2], Scalar::Int64(201));
        assert_eq!(right_values[3], Scalar::Int64(201));
        assert_eq!(right_values[4], Scalar::Int64(300));
        assert_eq!(right_values[5], Scalar::Int64(400));
    }

    #[test]
    fn merge_left_right_all_matched_dense_int64_duplicates_match_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(20),
                        Scalar::Int64(10),
                        Scalar::Int64(21),
                        Scalar::Int64(11),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(101),
                        Scalar::Int64(201),
                    ],
                ),
            ],
        )
        .unwrap();

        for join_type in [JoinType::Left, JoinType::Right] {
            let fast = merge_dataframes(&left, &right, "id", join_type).unwrap();
            let generic = merge_dataframes_on_with_options(
                &left,
                &right,
                &["id"],
                &["id"],
                join_type,
                MergeExecutionOptions {
                    validate_mode: Some(MergeValidateMode::ManyToMany),
                    ..MergeExecutionOptions::default()
                },
            )
            .unwrap();

            assert_eq!(fast.index, generic.index);
            assert_eq!(fast.column_order, generic.column_order);
            assert_eq!(fast.columns, generic.columns);
            assert!(
                generic
                    .columns
                    .values()
                    .all(|column| column.as_i64_slice().is_some())
            );
        }
    }

    #[test]
    fn merge_right_all_matched_dense_int64_fused_output_matches_dense_materialization() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                    ],
                ),
                (
                    "v",
                    vec![
                        Scalar::Int64(20),
                        Scalar::Int64(10),
                        Scalar::Int64(21),
                        Scalar::Int64(11),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(101),
                        Scalar::Int64(201),
                    ],
                ),
            ],
        )
        .unwrap();

        let fused = merge_dataframes(&left, &right, "id", JoinType::Right).unwrap();
        let left_key = left.columns().get("id").unwrap();
        let right_key = right.columns().get("id").unwrap();
        let (left_positions, right_positions) =
            dense_int64_right_positions(left_key, right_key).unwrap();
        let suffixes = resolve_merge_suffixes(None);
        let materialized = build_single_key_dense_right_merge_output(
            &left,
            &right,
            &["id"],
            &["id"],
            &left_positions,
            &right_positions,
            &suffixes,
        )
        .unwrap();

        assert_eq!(fused.index, materialized.index);
        assert_eq!(fused.column_order, materialized.column_order);
        assert_eq!(fused.columns, materialized.columns);
        assert_eq!(
            fused.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ]
        );
        assert!(
            fused
                .columns
                .values()
                .all(|column| column.as_i64_slice().is_some())
        );
    }

    #[test]
    fn merge_right_ordered_unique_int64_subset_matches_generic_validated_route() {
        let left = DataFrame::from_dict(
            &["id", "v"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(0), Scalar::Int64(1), Scalar::Int64(3)],
                ),
                (
                    "v",
                    vec![Scalar::Int64(10), Scalar::Int64(11), Scalar::Int64(13)],
                ),
            ],
        )
        .unwrap();
        let right = DataFrame::from_dict(
            &["id", "w"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(0),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "w",
                    vec![
                        Scalar::Int64(100),
                        Scalar::Int64(200),
                        Scalar::Int64(300),
                        Scalar::Int64(500),
                    ],
                ),
            ],
        )
        .unwrap();

        let fast = merge_dataframes(&left, &right, "id", JoinType::Right).unwrap();
        let generic = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Right,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap();

        assert_eq!(fast.index, generic.index);
        assert_eq!(fast.column_order, generic.column_order);
        assert_eq!(fast.columns, generic.columns);
        assert_eq!(
            fast.columns.get("id").unwrap().values(),
            &[
                Scalar::Int64(0),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(5)
            ]
        );
        let left_values = fast.columns.get("v").unwrap().values();
        assert_eq!(left_values[0], Scalar::Int64(10));
        assert!(left_values[1].is_missing());
        assert_eq!(left_values[2], Scalar::Int64(13));
        assert!(left_values[3].is_missing());
        let right_values = fast.columns.get("w").unwrap().values();
        assert_eq!(right_values[0], Scalar::Int64(100));
        assert_eq!(right_values[1], Scalar::Int64(200));
        assert_eq!(right_values[2], Scalar::Int64(300));
        assert_eq!(right_values[3], Scalar::Int64(500));
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
        assert!(merged.columns.contains_key("val_x"));
        assert!(merged.columns.contains_key("val_y"));
        assert_eq!(
            merged.columns.get("val_x").unwrap().values(),
            &[Scalar::Int64(10)]
        );
        assert_eq!(
            merged.columns.get("val_y").unwrap().values(),
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
    fn ordered_identity_int64_guard_uses_strict_unique_certificate() {
        let left = fp_columnar::Column::from_i64_values(vec![1, 2, 3, 4]);
        let right = fp_columnar::Column::from_i64_values(vec![1, 2, 3, 4]);
        assert!(ordered_identity_int64_keys_match(&left, &right));

        let duplicate_left = fp_columnar::Column::from_i64_values(vec![1, 1, 2]);
        let duplicate_right = fp_columnar::Column::from_i64_values(vec![1, 1, 2]);
        assert!(!ordered_identity_int64_keys_match(
            &duplicate_left,
            &duplicate_right
        ));

        let unsorted_left = fp_columnar::Column::from_i64_values(vec![2, 1, 3]);
        let unsorted_right = fp_columnar::Column::from_i64_values(vec![2, 1, 3]);
        assert!(ordered_identity_int64_keys_match(
            &unsorted_left,
            &unsorted_right
        ));
    }

    #[test]
    fn merge_identical_duplicate_keys_cross_join_values_jdupk() {
        // Identical left/right key columns that contain duplicates must NOT take
        // the identity 1:1 fast path: pandas cross-joins each duplicate, so a=10
        // and a=20 each pair with b=100 and b=200. (br-frankenpandas-jdupk)
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
        assert_eq!(
            merged.columns.get("a").unwrap().values(),
            &[
                Scalar::Int64(10),
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(20)
            ]
        );
        assert_eq!(
            merged.columns.get("b").unwrap().values(),
            &[
                Scalar::Int64(100),
                Scalar::Int64(200),
                Scalar::Int64(100),
                Scalar::Int64(200)
            ]
        );
    }

    #[test]
    fn merge_inner_int64_fast_paths_match_naive_reference_fuzz_jdupk() {
        // Differential guard: merge_dataframes dispatches int64 single-key inner
        // merges to ordered/identity/dense fast paths. Each MUST equal a naive
        // left-major nested-loop join for arbitrary int64 keys (sorted/unsorted,
        // unique/duplicate, overlapping/disjoint, negative). This is the
        // isomorphism proof those perf paths owe; it caught the identity-path
        // row-loss bug. (br-frankenpandas-rw0r5)
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };
        for _ in 0..400 {
            let nl = (next() % 6 + 1) as usize;
            let nr = (next() % 6 + 1) as usize;
            // small key range forces collisions; offset gives negatives too
            let lk: Vec<i64> = (0..nl).map(|_| (next() % 5) as i64 - 2).collect();
            let rk: Vec<i64> = (0..nr).map(|_| (next() % 5) as i64 - 2).collect();

            let left = DataFrame::from_dict(
                &["id", "a"],
                vec![
                    ("id", lk.iter().map(|&k| Scalar::Int64(k)).collect()),
                    (
                        "a",
                        (0..nl).map(|i| Scalar::Int64(i as i64 + 1000)).collect(),
                    ),
                ],
            )
            .unwrap();
            let right = DataFrame::from_dict(
                &["id", "b"],
                vec![
                    ("id", rk.iter().map(|&k| Scalar::Int64(k)).collect()),
                    (
                        "b",
                        (0..nr).map(|j| Scalar::Int64(j as i64 + 2000)).collect(),
                    ),
                ],
            )
            .unwrap();

            let merged = merge_dataframes(&left, &right, "id", JoinType::Inner).unwrap();

            // Naive reference: pandas inner merge is left-major, right-minor.
            let mut exp_a = Vec::new();
            let mut exp_b = Vec::new();
            for (i, &left_key) in lk.iter().enumerate() {
                for (j, &right_key) in rk.iter().enumerate() {
                    if left_key == right_key {
                        exp_a.push(Scalar::Int64(i as i64 + 1000));
                        exp_b.push(Scalar::Int64(j as i64 + 2000));
                    }
                }
            }
            assert_eq!(
                merged.columns.get("a").unwrap().values(),
                exp_a.as_slice(),
                "left payload mismatch for lk={lk:?} rk={rk:?}"
            );
            assert_eq!(
                merged.columns.get("b").unwrap().values(),
                exp_b.as_slice(),
                "right payload mismatch for lk={lk:?} rk={rk:?}"
            );

            // Cardinality guard for left/right/outer (dtype/order independent —
            // catches the row-loss class regardless of int->float promotion).
            let inner_pairs = exp_a.len();
            let left_only = lk.iter().filter(|&key| !rk.contains(key)).count();
            let right_only = rk.iter().filter(|&key| !lk.contains(key)).count();
            for (how, expected_rows) in [
                (JoinType::Left, inner_pairs + left_only),
                (JoinType::Right, inner_pairs + right_only),
                (JoinType::Outer, inner_pairs + left_only + right_only),
            ] {
                let m = merge_dataframes(&left, &right, "id", how).unwrap();
                assert_eq!(
                    m.columns.get("id").unwrap().len(),
                    expected_rows,
                    "{how:?} cardinality mismatch for lk={lk:?} rk={rk:?}"
                );
            }
        }
    }

    #[test]
    fn merge_null_nan_keys_metamorphic_unmatched_rows_do_not_disturb_inner_tn6qb8()
    -> Result<(), JoinError> {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Null(NullKind::NaN),
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(2.0),
                    ],
                ),
                (
                    "left_v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(21),
                        Scalar::Int64(30),
                    ],
                ),
            ],
        )?;
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(2.0),
                        Scalar::Float64(f64::NAN),
                    ],
                ),
                (
                    "right_v",
                    vec![Scalar::Int64(200), Scalar::Int64(300), Scalar::Int64(201)],
                ),
            ],
        )?;

        let baseline = merge_dataframes(&left, &right, "id", JoinType::Inner)?;

        let left_with_unmatched = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Null(NullKind::NaN),
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(2.0),
                        Scalar::Float64(99.0),
                    ],
                ),
                (
                    "left_v",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(21),
                        Scalar::Int64(30),
                        Scalar::Int64(990),
                    ],
                ),
            ],
        )?;
        let right_with_unmatched = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(2.0),
                        Scalar::Float64(f64::NAN),
                        Scalar::Float64(100.0),
                    ],
                ),
                (
                    "right_v",
                    vec![
                        Scalar::Int64(200),
                        Scalar::Int64(300),
                        Scalar::Int64(201),
                        Scalar::Int64(1000),
                    ],
                ),
            ],
        )?;

        let transformed = merge_dataframes(
            &left_with_unmatched,
            &right_with_unmatched,
            "id",
            JoinType::Inner,
        )?;

        assert_eq!(
            merged_values(&baseline, "left_v")?,
            merged_values(&transformed, "left_v")?
        );
        assert_eq!(
            merged_values(&baseline, "right_v")?,
            merged_values(&transformed, "right_v")?
        );
        assert_eq!(
            merged_values(&baseline, "left_v")?,
            &[
                Scalar::Int64(20),
                Scalar::Int64(20),
                Scalar::Int64(21),
                Scalar::Int64(21),
                Scalar::Int64(30),
            ]
        );
        assert_eq!(
            merged_values(&baseline, "right_v")?,
            &[
                Scalar::Int64(200),
                Scalar::Int64(201),
                Scalar::Int64(200),
                Scalar::Int64(201),
                Scalar::Int64(300),
            ]
        );

        Ok(())
    }

    #[test]
    fn merge_mixed_dtype_missing_keys_metamorphic_outer_both_equals_inner_tn6qb8()
    -> Result<(), JoinError> {
        let left = DataFrame::from_dict(
            &["id", "left_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Null(NullKind::NaN),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "left_v",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                        Scalar::Float64(40.0),
                    ],
                ),
            ],
        )?;
        let right = DataFrame::from_dict(
            &["id", "right_v"],
            vec![
                (
                    "id",
                    vec![
                        Scalar::Float64(1.0),
                        Scalar::Float64(2.5),
                        Scalar::Null(NullKind::Null),
                        Scalar::Float64(3.0),
                    ],
                ),
                (
                    "right_v",
                    vec![
                        Scalar::Float64(100.0),
                        Scalar::Float64(250.0),
                        Scalar::Float64(300.0),
                        Scalar::Float64(400.0),
                    ],
                ),
            ],
        )?;
        let options = MergeExecutionOptions {
            indicator_name: Some("_merge".to_owned()),
            sort: true,
            ..MergeExecutionOptions::default()
        };

        let inner = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Inner,
            options.clone(),
        )?;
        let outer = merge_dataframes_on_with_options(
            &left,
            &right,
            &["id"],
            &["id"],
            JoinType::Outer,
            options,
        )?;

        assert_eq!(
            merged_values(&inner, "left_v")?,
            merged_values_where_indicator(&outer, "left_v", "_merge", "both")?
        );
        assert_eq!(
            merged_values(&inner, "right_v")?,
            merged_values_where_indicator(&outer, "right_v", "_merge", "both")?
        );
        assert_eq!(
            merged_values(&inner, "left_v")?,
            &[
                Scalar::Float64(10.0),
                Scalar::Float64(40.0),
                Scalar::Float64(30.0)
            ]
        );
        assert_eq!(
            merged_values(&inner, "right_v")?,
            &[
                Scalar::Float64(100.0),
                Scalar::Float64(400.0),
                Scalar::Float64(300.0)
            ]
        );
        assert_eq!(
            merged_values(&outer, "_merge")?,
            &[
                Scalar::Utf8("both".to_owned()),
                Scalar::Utf8("left_only".to_owned()),
                Scalar::Utf8("right_only".to_owned()),
                Scalar::Utf8("both".to_owned()),
                Scalar::Utf8("both".to_owned()),
            ]
        );

        Ok(())
    }

    #[test]
    fn merge_cross_cartesian_product() {
        let left = make_left_df();
        let right = make_right_df();

        let merged = merge_dataframes(&left, &right, "unused_key", JoinType::Cross).unwrap();

        assert_eq!(merged.columns.get("id_x").unwrap().len(), 9);
        assert_eq!(
            &merged.columns.get("id_x").unwrap().values()[0..3],
            &[Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(1)]
        );
        assert_eq!(
            &merged.columns.get("id_y").unwrap().values()[0..3],
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
        assert_eq!(merged.columns.get("id_x").unwrap().len(), 9);
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

        assert!(merged.columns.contains_key("val_x"));
        assert!(merged.columns.contains_key("val_y"));
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
        // Restored after br-frankenpandas-glsam revert: pandas 2.2.3
        // supports `how='cross', indicator=...` and emits "both" for every
        // cross-product row (verified against FP-P2D-039 fixture provenance
        // pandas_version=2.2.3). The Explore-agent claim that pandas raises
        // was incorrect; this test locks in the actual pandas behavior.
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
    fn merge_asof_nearest_excludes_exact_finds_neighbor() {
        // Regression: with direction=nearest and allow_exact_matches=false,
        // an exact-equal key must be skipped over to reach the nearest
        // non-equal key, rather than yielding no match. Verified against
        // pandas 2.2.3: merge_asof(left=5, right=[3,5,5], nearest,
        // allow_exact_matches=False) -> matches value 3.
        use super::{AsofDirection, MergeAsofOptions};

        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                ("time", vec![Scalar::Int64(5)]),
                ("val", vec![Scalar::Int64(1)]),
            ],
        )
        .unwrap();
        let right = fp_frame::DataFrame::from_dict(
            &["time", "ref_val"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(3), Scalar::Int64(5), Scalar::Int64(5)],
                ),
                (
                    "ref_val",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let opts = MergeAsofOptions::new().allow_exact_matches(false);
        let result =
            super::merge_asof_with_options(&left, &right, "time", AsofDirection::Nearest, opts)
                .unwrap();
        let ref_col = result.columns.get("ref_val").unwrap();
        assert_eq!(ref_col.values()[0], Scalar::Float64(10.0)); // value 3 is nearest non-exact
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

        // pandas merge_asof accepts NaN keys: the NaN row gets no match
        // (null in joined columns) — verified against FP-P2D-056
        // backward_nan_left_key_strict (pandas 2.2.3 oracle).
        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        assert!(result.columns.get("quote").unwrap().values()[1].is_missing());
        // time=5 → backward match to time=4 → quote=400
        assert_eq!(
            result.columns.get("quote").unwrap().values()[2],
            Scalar::Float64(400.0)
        );
    }

    #[test]
    fn merge_asof_nan_in_right_key() {
        use super::AsofDirection;

        // Symmetric to merge_asof_nan_in_key: NaN keys on the right side
        // also get treated as no-candidate (pandas does NOT raise). Other
        // left rows still match to non-NaN right rows.
        let left = fp_frame::DataFrame::from_dict(
            &["time", "val"],
            vec![
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(3), Scalar::Int64(5)],
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
                (
                    "time",
                    vec![
                        Scalar::Int64(2),
                        Scalar::Null(NullKind::NaN),
                        Scalar::Int64(4),
                    ],
                ),
                (
                    "quote",
                    vec![
                        Scalar::Float64(200.0),
                        Scalar::Float64(999.0),
                        Scalar::Float64(400.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let result = super::merge_asof(&left, &right, "time", AsofDirection::Backward).unwrap();
        // left time=1 → no right ≤ 1 → null
        assert!(result.columns.get("quote").unwrap().values()[0].is_missing());
        // left time=3 → right time=2 → 200.0 (NaN right row never matches)
        assert_eq!(
            result.columns.get("quote").unwrap().values()[1],
            Scalar::Float64(200.0)
        );
        // left time=5 → right time=4 → 400.0
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
        let shared_left = result.columns.get("shared_x").unwrap().values();
        let shared_right = result.columns.get("shared_y").unwrap().values();

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
        assert!(result.columns.contains_key("val_x"));
        assert!(result.columns.contains_key("val_y"));
        assert_eq!(
            result.columns.get("val_x").unwrap().values()[0],
            Scalar::Float64(10.0)
        );
        assert_eq!(
            result.columns.get("val_y").unwrap().values()[0],
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
                        Scalar::Int64(2),
                        Scalar::Int64(3),
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
                        Scalar::Utf8("B".into()),
                        Scalar::Utf8("A".into()),
                    ],
                ),
                (
                    "time",
                    vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)],
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
        // Left after 2d3338b1's reorder is (A,1), (A,2), (B,3), (B,4) — globally
        // sorted by time. Right is (A,2), (B,3), (A,4). Backward grouped asof:
        // Row 0: (A,1) → no right A with time <= 1 → null
        assert!(matches!(quote_col.values()[0], Scalar::Null(_)));
        // Row 1: (A,2) → right A time=2 → 200.0
        assert_eq!(quote_col.values()[1], Scalar::Float64(200.0));
        // Row 2: (B,3) → right B time=3 → 400.0
        assert_eq!(quote_col.values()[2], Scalar::Float64(400.0));
        // Row 3: (B,4) → right B time=3 (latest <= 4) → 400.0
        assert_eq!(quote_col.values()[3], Scalar::Float64(400.0));
    }

    #[test]
    fn merge_asof_by_column_requires_globally_sorted_left_keys() {
        use super::{AsofDirection, MergeAsofOptions, merge_asof_with_options};

        let left = fp_frame::DataFrame::from_dict(
            &["group", "time"],
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
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                    ],
                ),
            ],
        )
        .unwrap();
        let right = fp_frame::DataFrame::from_dict(
            &["group", "time", "quote"],
            vec![
                (
                    "group",
                    vec![
                        Scalar::Utf8("A".into()),
                        Scalar::Utf8("B".into()),
                        Scalar::Utf8("A".into()),
                    ],
                ),
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "quote",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(20.0),
                        Scalar::Float64(30.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let err = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new().by(vec!["group".to_string()]),
        )
        .expect_err("pandas requires globally sorted left keys even with by=");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(message))
                if message.contains("left") && message.contains("sorted")
        ));
    }

    #[test]
    fn merge_asof_by_column_requires_globally_sorted_right_keys() {
        use super::{AsofDirection, MergeAsofOptions, merge_asof_with_options};

        let left = fp_frame::DataFrame::from_dict(
            &["group", "time"],
            vec![
                (
                    "group",
                    vec![
                        Scalar::Utf8("A".into()),
                        Scalar::Utf8("B".into()),
                        Scalar::Utf8("A".into()),
                    ],
                ),
                (
                    "time",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
            ],
        )
        .unwrap();
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
                    vec![Scalar::Int64(1), Scalar::Int64(3), Scalar::Int64(2)],
                ),
                (
                    "quote",
                    vec![
                        Scalar::Float64(10.0),
                        Scalar::Float64(30.0),
                        Scalar::Float64(20.0),
                    ],
                ),
            ],
        )
        .unwrap();

        let err = merge_asof_with_options(
            &left,
            &right,
            "time",
            AsofDirection::Backward,
            MergeAsofOptions::new().by(vec!["group".to_string()]),
        )
        .expect_err("pandas requires globally sorted right keys even with by=");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(message))
                if message.contains("right") && message.contains("sorted")
        ));
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
                if message.contains("fill_method must be 'ffill' or None")
        ));
    }

    #[test]
    fn merge_ordered_rejects_bfill_like_pandas() {
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

        let err = super::merge_ordered(&left, &right, &["date"], Some("bfill"))
            .expect_err("pandas only accepts fill_method='ffill' or None");
        assert!(matches!(
            err,
            super::JoinError::Frame(fp_frame::FrameError::CompatibilityRejected(message))
                if message.contains("fill_method must be 'ffill' or None")
        ));
    }

    // ── join() pandas-named alias (br-frankenpandas-nk54a) ──────────────

    #[test]
    fn dataframe_join_aliases_join_on_index() {
        use std::collections::BTreeMap;

        use fp_columnar::Column;
        use fp_frame::DataFrame;
        use fp_index::Index;
        use fp_types::{DType, Scalar};

        use super::DataFrameMergeExt;

        fn build(name1: &str, name2: &str, vals1: &[i64], vals2: &[i64]) -> DataFrame {
            let mut cols = BTreeMap::new();
            cols.insert(
                name1.to_owned(),
                Column::new(
                    DType::Int64,
                    vals1.iter().map(|v| Scalar::Int64(*v)).collect(),
                )
                .unwrap(),
            );
            cols.insert(
                name2.to_owned(),
                Column::new(
                    DType::Int64,
                    vals2.iter().map(|v| Scalar::Int64(*v)).collect(),
                )
                .unwrap(),
            );
            DataFrame::new_with_column_order(
                Index::new(
                    (0..vals1.len() as i64)
                        .map(fp_index::IndexLabel::Int64)
                        .collect(),
                ),
                cols,
                vec![name1.to_owned(), name2.to_owned()],
            )
            .unwrap()
        }

        let left = build("a", "b", &[10, 20, 30], &[1, 2, 3]);
        let right = build("c", "d", &[100, 200, 300], &[7, 8, 9]);

        let via_join = left.join(&right, super::JoinType::Inner).unwrap();
        let via_join_on_index = left.join_on_index(&right, super::JoinType::Inner).unwrap();

        // Both produce the same MergedDataFrame shape & values.
        let join_keys: Vec<&String> = via_join.columns.keys().collect();
        let idx_keys: Vec<&String> = via_join_on_index.columns.keys().collect();
        assert_eq!(join_keys, idx_keys);
        assert_eq!(via_join.index, via_join_on_index.index);
        for k in via_join.columns.keys() {
            assert_eq!(
                via_join.columns.get(k).unwrap().values(),
                via_join_on_index.columns.get(k).unwrap().values(),
                "column {k} differs"
            );
        }
    }
}
