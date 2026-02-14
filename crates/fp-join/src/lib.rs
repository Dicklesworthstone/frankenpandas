#![forbid(unsafe_code)]

use std::{collections::HashMap, mem::size_of};

use bumpalo::{Bump, collections::Vec as BumpVec};
use fp_columnar::{Column, ColumnError};
use fp_frame::{FrameError, Series};
use fp_index::{Index, IndexLabel};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Outer,
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
        JoinType::Outer => {
            // Left-matched rows + unmatched right rows
            let right_unmatched: usize = right
                .index()
                .labels()
                .iter()
                .filter(|label| !left.index().labels().contains(label))
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
                for (right_pos, label) in right.index().labels().iter().enumerate() {
                    if !left.index().labels().contains(label) {
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
    }

    let left_values = left.column().reindex_by_positions(&left_positions)?;
    let right_values = right.column().reindex_by_positions(&right_positions)?;

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
                for (right_pos, label) in right.index().labels().iter().enumerate() {
                    if !left.index().labels().contains(label) {
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
    }

    let left_values = left
        .column()
        .reindex_by_positions(left_positions.as_slice())?;
    let right_values = right
        .column()
        .reindex_by_positions(right_positions.as_slice())?;

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

/// Convert a Scalar to an IndexLabel for use as a hash key.
/// Returns None for missing/NaN values (which cannot be join keys).
fn scalar_to_key(s: &fp_types::Scalar) -> Option<IndexLabel> {
    match s {
        fp_types::Scalar::Int64(v) => Some(IndexLabel::Int64(*v)),
        fp_types::Scalar::Float64(v) if !v.is_nan() => Some(IndexLabel::Int64(*v as i64)),
        fp_types::Scalar::Utf8(v) => Some(IndexLabel::Utf8(v.clone())),
        fp_types::Scalar::Bool(b) => Some(IndexLabel::Int64(i64::from(*b))),
        _ => None, // Null, NaN
    }
}

/// Merge two DataFrames on a specified key column.
///
/// Matches `pd.merge(left, right, on=key, how=join_type)`.
/// - The key column is used for matching rows (hash join).
/// - Non-key columns are carried through and reindexed.
/// - Column name conflicts get `_left`/`_right` suffixes.
/// - The output index is auto-generated (0..n RangeIndex-style).
pub fn merge_dataframes(
    left: &fp_frame::DataFrame,
    right: &fp_frame::DataFrame,
    on: &str,
    join_type: JoinType,
) -> Result<MergedDataFrame, JoinError> {
    let left_key = left.column(on).ok_or_else(|| {
        JoinError::Frame(FrameError::CompatibilityRejected(format!(
            "left DataFrame missing key column '{on}'"
        )))
    })?;
    let right_key = right.column(on).ok_or_else(|| {
        JoinError::Frame(FrameError::CompatibilityRejected(format!(
            "right DataFrame missing key column '{on}'"
        )))
    })?;

    // Convert key columns to IndexLabel for hashing.
    let left_keys: Vec<Option<IndexLabel>> = left_key.values().iter().map(scalar_to_key).collect();
    let right_keys: Vec<Option<IndexLabel>> =
        right_key.values().iter().map(scalar_to_key).collect();

    // Build hash map from right key → row positions.
    let mut right_map = HashMap::<&IndexLabel, Vec<usize>>::new();
    for (pos, key) in right_keys.iter().enumerate() {
        if let Some(k) = key {
            right_map.entry(k).or_default().push(pos);
        }
    }

    let left_map = if matches!(join_type, JoinType::Right | JoinType::Outer) {
        let mut m = HashMap::<&IndexLabel, Vec<usize>>::new();
        for (pos, key) in left_keys.iter().enumerate() {
            if let Some(k) = key {
                m.entry(k).or_default().push(pos);
            }
        }
        Some(m)
    } else {
        None
    };

    // Compute row position mappings.
    let mut left_positions = Vec::<Option<usize>>::new();
    let mut right_positions = Vec::<Option<usize>>::new();
    let mut out_keys = Vec::<fp_types::Scalar>::new();

    match join_type {
        JoinType::Inner | JoinType::Left | JoinType::Outer => {
            for (left_pos, key) in left_keys.iter().enumerate() {
                if let Some(k) = key
                    && let Some(matches) = right_map.get(k)
                {
                    for &right_pos in matches {
                        out_keys.push(left_key.values()[left_pos].clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(right_pos));
                    }
                    continue;
                }

                if matches!(join_type, JoinType::Left | JoinType::Outer) {
                    out_keys.push(left_key.values()[left_pos].clone());
                    left_positions.push(Some(left_pos));
                    right_positions.push(None);
                }
            }

            if matches!(join_type, JoinType::Outer) {
                // Track which right keys already appeared via left.
                let left_key_set: std::collections::HashSet<&IndexLabel> =
                    left_keys.iter().filter_map(|k| k.as_ref()).collect();
                for (right_pos, key) in right_keys.iter().enumerate() {
                    if let Some(k) = key
                        && !left_key_set.contains(k)
                    {
                        out_keys.push(right_key.values()[right_pos].clone());
                        left_positions.push(None);
                        right_positions.push(Some(right_pos));
                    }
                }
            }
        }
        JoinType::Right => {
            let left_map = left_map.as_ref().expect("left_map required for Right join");
            for (right_pos, key) in right_keys.iter().enumerate() {
                if let Some(k) = key
                    && let Some(matches) = left_map.get(k)
                {
                    for &left_pos in matches {
                        out_keys.push(right_key.values()[right_pos].clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(right_pos));
                    }
                    continue;
                }

                out_keys.push(right_key.values()[right_pos].clone());
                left_positions.push(None);
                right_positions.push(Some(right_pos));
            }
        }
    }

    // Build output columns by reindexing.
    let n = left_positions.len();
    let index = Index::new((0..n as i64).map(IndexLabel::from).collect());
    let mut columns = std::collections::BTreeMap::new();

    // Collect column names to handle conflicts.
    let left_col_names: std::collections::HashSet<&String> = left.columns().keys().collect();
    let right_col_names: std::collections::HashSet<&String> = right.columns().keys().collect();

    // Add key column from output keys.
    columns.insert(on.to_owned(), Column::from_values(out_keys)?);

    // Add left non-key columns.
    for (name, col) in left.columns() {
        if name == on {
            continue;
        }
        let reindexed = col.reindex_by_positions(&left_positions)?;
        let out_name = if right_col_names.contains(name) && name != on {
            format!("{name}_left")
        } else {
            name.clone()
        };
        columns.insert(out_name, reindexed);
    }

    // Add right non-key columns.
    for (name, col) in right.columns() {
        if name == on {
            continue;
        }
        let reindexed = col.reindex_by_positions(&right_positions)?;
        let out_name = if left_col_names.contains(name) && name != on {
            format!("{name}_right")
        } else {
            name.clone()
        };
        columns.insert(out_name, reindexed);
    }

    Ok(MergedDataFrame { index, columns })
}

#[cfg(test)]
mod tests {
    use fp_index::IndexLabel;
    use fp_types::{NullKind, Scalar};

    use super::{
        JoinExecutionOptions, JoinType, join_series, join_series_with_options,
        join_series_with_trace,
    };
    use fp_frame::Series;

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
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null)
            ]
        );
        assert_eq!(
            out.right_values.values(),
            &[
                Scalar::Null(NullKind::Null),
                Scalar::Int64(20),
                Scalar::Int64(30)
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

    use super::merge_dataframes;
    use fp_frame::DataFrame;

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
}
