#![forbid(unsafe_code)]

use std::{collections::HashMap, mem::size_of};

use bumpalo::{Bump, collections::Vec as BumpVec};
use fp_columnar::{Column, ColumnError};
use fp_frame::{FrameError, Series};
use fp_index::{Index, IndexError, IndexLabel, align_union, validate_alignment_plan};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{NullKind, Scalar};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupByOptions {
    pub dropna: bool,
}

impl Default for GroupByOptions {
    fn default() -> Self {
        Self { dropna: true }
    }
}

#[derive(Debug, Error)]
pub enum GroupByError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    Column(#[from] ColumnError),
}

pub const DEFAULT_ARENA_BUDGET_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupByExecutionOptions {
    pub use_arena: bool,
    pub arena_budget_bytes: usize,
}

impl Default for GroupByExecutionOptions {
    fn default() -> Self {
        Self {
            use_arena: true,
            arena_budget_bytes: DEFAULT_ARENA_BUDGET_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GroupByExecutionTrace {
    used_arena: bool,
    input_rows: usize,
    estimated_bytes: usize,
}

pub fn groupby_sum(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_sum_with_options(
        keys,
        values,
        options,
        policy,
        ledger,
        GroupByExecutionOptions::default(),
    )
}

pub fn groupby_sum_with_options(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    _policy: &RuntimePolicy,
    _ledger: &mut EvidenceLedger,
    exec_options: GroupByExecutionOptions,
) -> Result<Series, GroupByError> {
    let (result, _trace) =
        groupby_sum_with_trace(keys, values, options, _policy, _ledger, exec_options)?;
    Ok(result)
}

fn groupby_sum_with_trace(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    _policy: &RuntimePolicy,
    _ledger: &mut EvidenceLedger,
    exec_options: GroupByExecutionOptions,
) -> Result<(Series, GroupByExecutionTrace), GroupByError> {
    // Fast path: if indexes already match and are duplicate-free, alignment is identity.
    let aligned_storage = if keys.index() == values.index() && !keys.index().has_duplicates() {
        None
    } else {
        let plan = align_union(keys.index(), values.index());
        validate_alignment_plan(&plan)?;
        let aligned_keys = keys.column().reindex_by_positions(&plan.left_positions)?;
        let aligned_values = values
            .column()
            .reindex_by_positions(&plan.right_positions)?;
        Some((aligned_keys, aligned_values))
    };

    let (aligned_keys_values, aligned_values_values): (&[Scalar], &[Scalar]) =
        if let Some((aligned_keys, aligned_values)) = aligned_storage.as_ref() {
            (aligned_keys.values(), aligned_values.values())
        } else {
            (keys.values(), values.values())
        };

    let input_rows = aligned_keys_values.len();
    let estimated_bytes = estimate_groupby_intermediate_bytes(input_rows);
    let use_arena = exec_options.use_arena && estimated_bytes <= exec_options.arena_budget_bytes;

    let result = if use_arena {
        groupby_sum_with_arena(aligned_keys_values, aligned_values_values, options)?
    } else {
        groupby_sum_with_global_allocator(aligned_keys_values, aligned_values_values, options)?
    };

    Ok((
        result,
        GroupByExecutionTrace {
            used_arena: use_arena,
            input_rows,
            estimated_bytes,
        },
    ))
}

/// Estimate intermediate memory for groupby (dense path intermediates + ordering).
fn estimate_groupby_intermediate_bytes(input_rows: usize) -> usize {
    // Dense path: sums (f64) + seen (bool) + ordering (i64), all up to DENSE_INT_KEY_RANGE_LIMIT.
    // Generic path: ordering (GroupKeyRef ~32 bytes) + HashMap overhead (~64 bytes per entry).
    // Use conservative estimate: assume generic path dominates.
    input_rows.saturating_mul(
        size_of::<f64>()
            .saturating_add(size_of::<bool>())
            .saturating_add(size_of::<i64>())
            .saturating_add(64), // HashMap entry overhead estimate
    )
}

fn groupby_sum_with_global_allocator(
    aligned_keys_values: &[Scalar],
    aligned_values_values: &[Scalar],
    options: GroupByOptions,
) -> Result<Series, GroupByError> {
    if let Some((out_index, out_values)) =
        try_groupby_sum_dense_int64(aligned_keys_values, aligned_values_values, options.dropna)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new("sum", Index::new(out_index), out_column)?);
    }

    // AG-08: Store (source_index, sum) instead of (Scalar, sum) to eliminate
    // per-group key.clone() allocations. Reconstruct IndexLabel at output phase.
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut slot = HashMap::<GroupKeyRef<'_>, (usize, f64)>::new();

    for (pos, (key, value)) in aligned_keys_values
        .iter()
        .zip(aligned_values_values.iter())
        .enumerate()
    {
        if options.dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = slot.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, 0.0)
        });

        if value.is_missing() {
            continue;
        }

        if let Ok(v) = value.to_f64() {
            entry.1 += v;
        }
    }

    emit_groupby_result(aligned_keys_values, &ordering, &mut slot)
}

fn groupby_sum_with_arena(
    aligned_keys_values: &[Scalar],
    aligned_values_values: &[Scalar],
    options: GroupByOptions,
) -> Result<Series, GroupByError> {
    // AG-06: Arena-backed dense path intermediates.
    if let Some((out_index, out_values)) = try_groupby_sum_dense_int64_arena(
        aligned_keys_values,
        aligned_values_values,
        options.dropna,
    ) {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new("sum", Index::new(out_index), out_column)?);
    }

    // AG-06 + AG-08: Arena-back the ordering vector. Store source index
    // instead of cloned Scalar to eliminate per-group allocations.
    let arena = Bump::new();
    let mut ordering = BumpVec::<GroupKeyRef<'_>>::new_in(&arena);
    let mut slot = HashMap::<GroupKeyRef<'_>, (usize, f64)>::new();

    for (pos, (key, value)) in aligned_keys_values
        .iter()
        .zip(aligned_values_values.iter())
        .enumerate()
    {
        if options.dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = slot.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, 0.0)
        });

        if value.is_missing() {
            continue;
        }

        if let Ok(v) = value.to_f64() {
            entry.1 += v;
        }
    }

    emit_groupby_result(aligned_keys_values, ordering.as_slice(), &mut slot)
}

/// Convert accumulated groupby results into the output Series.
/// AG-08: Uses source index to reconstruct IndexLabel without Scalar clones.
fn emit_groupby_result<'a>(
    source_keys: &[Scalar],
    ordering: &[GroupKeyRef<'a>],
    slot: &mut HashMap<GroupKeyRef<'a>, (usize, f64)>,
) -> Result<Series, GroupByError> {
    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());

    for key in ordering {
        let (source_idx, sum) = slot
            .remove(key)
            .expect("ordering references only inserted keys");
        let label = &source_keys[source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Null(NullKind::NaN)
            | Scalar::Null(NullKind::NaT)
            | Scalar::Null(NullKind::Null) => IndexLabel::Utf8("<null>".to_owned()),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
        });
        out_values.push(Scalar::Float64(sum));
    }

    let out_column = Column::from_values(out_values)?;
    Ok(Series::new("sum", Index::new(out_index), out_column)?)
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum GroupKeyRef<'a> {
    Bool(bool),
    Int64(i64),
    FloatBits(u64),
    Utf8(&'a str),
    Null(NullKind),
}

impl<'a> GroupKeyRef<'a> {
    fn from_scalar(key: &'a Scalar) -> Self {
        match key {
            Scalar::Bool(v) => Self::Bool(*v),
            Scalar::Int64(v) => Self::Int64(*v),
            Scalar::Float64(v) => Self::FloatBits(if v.is_nan() {
                f64::NAN.to_bits()
            } else {
                v.to_bits()
            }),
            Scalar::Utf8(v) => Self::Utf8(v.as_str()),
            Scalar::Null(kind) => Self::Null(*kind),
        }
    }
}

const DENSE_INT_KEY_RANGE_LIMIT: i128 = 65_536;

/// Scan keys and return (min, max, saw_any_int). Returns None if a non-Int64,
/// non-droppable-null key is found.
fn dense_int64_range(keys: &[Scalar], dropna: bool) -> Option<(i64, i64, bool)> {
    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    let mut saw_int_key = false;

    for key in keys {
        match key {
            Scalar::Int64(v) => {
                saw_int_key = true;
                min_key = min_key.min(*v);
                max_key = max_key.max(*v);
            }
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        }
    }
    Some((min_key, max_key, saw_int_key))
}

/// Dense-bucket fast path for `Int64` keys.
///
/// Falls back to the generic map path unless every non-dropped key is `Int64`
/// and the key span is within a bounded range budget.
fn try_groupby_sum_dense_int64(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    let (min_key, max_key, saw_int_key) = dense_int64_range(keys, dropna)?;

    if !saw_int_key {
        return Some((Vec::new(), Vec::new()));
    }

    let span = i128::from(max_key) - i128::from(min_key) + 1;
    if span <= 0 || span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }

    let bucket_len = usize::try_from(span).ok()?;
    let mut sums = vec![0.0f64; bucket_len];
    let mut seen = vec![false; bucket_len];
    let mut ordering = Vec::<i64>::new();

    for (key, value) in keys.iter().zip(values.iter()) {
        let key = match key {
            Scalar::Int64(v) => *v,
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        };

        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        if !seen[bucket] {
            seen[bucket] = true;
            ordering.push(key);
        }

        if value.is_missing() {
            continue;
        }
        if let Ok(v) = value.to_f64() {
            sums[bucket] += v;
        }
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in ordering {
        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        out_index.push(IndexLabel::Int64(key));
        out_values.push(Scalar::Float64(sums[bucket]));
    }

    Some((out_index, out_values))
}

/// AG-06: Arena-backed dense bucket fast path. The `sums`, `seen`, and `ordering`
/// vectors live in the arena and are freed in bulk when the arena drops.
fn try_groupby_sum_dense_int64_arena(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    let (min_key, max_key, saw_int_key) = dense_int64_range(keys, dropna)?;

    if !saw_int_key {
        return Some((Vec::new(), Vec::new()));
    }

    let span = i128::from(max_key) - i128::from(min_key) + 1;
    if span <= 0 || span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }

    let bucket_len = usize::try_from(span).ok()?;
    let arena = Bump::new();

    let mut sums = BumpVec::<f64>::with_capacity_in(bucket_len, &arena);
    sums.resize(bucket_len, 0.0f64);
    let mut seen = BumpVec::<bool>::with_capacity_in(bucket_len, &arena);
    seen.resize(bucket_len, false);
    let mut ordering = BumpVec::<i64>::new_in(&arena);

    for (key, value) in keys.iter().zip(values.iter()) {
        let key = match key {
            Scalar::Int64(v) => *v,
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        };

        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        if !seen[bucket] {
            seen[bucket] = true;
            ordering.push(key);
        }

        if value.is_missing() {
            continue;
        }
        if let Ok(v) = value.to_f64() {
            sums[bucket] += v;
        }
    }

    // Copy results out of arena into global-allocated output.
    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in ordering.iter().copied() {
        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        out_index.push(IndexLabel::Int64(key));
        out_values.push(Scalar::Float64(sums[bucket]));
    }

    Some((out_index, out_values))
}

#[cfg(test)]
mod tests {
    use fp_index::IndexLabel;
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::{NullKind, Scalar};

    use super::{
        GroupByExecutionOptions, GroupByOptions, groupby_sum, groupby_sum_with_options,
        groupby_sum_with_trace,
    };
    use fp_frame::Series;

    #[test]
    fn groupby_sum_respects_first_seen_key_order() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &["b".into(), "a".into()]);
        assert_eq!(out.values(), &[Scalar::Float64(4.0), Scalar::Float64(6.0)]);
    }

    #[test]
    fn groupby_sum_duplicate_equal_index_preserves_alignment_behavior() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        // Duplicate-label alignment in current model maps duplicates to first position.
        assert_eq!(out.index().labels(), &["a".into()]);
        assert_eq!(out.values(), &[Scalar::Float64(5.0)]);
    }

    #[test]
    fn groupby_sum_int_dense_path_preserves_first_seen_order() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(5),
                Scalar::Int64(10),
                Scalar::Int64(-2),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(
            out.index().labels(),
            &[10_i64.into(), 5_i64.into(), (-2_i64).into()]
        );
        assert_eq!(
            out.values(),
            &[
                Scalar::Float64(4.0),
                Scalar::Float64(2.0),
                Scalar::Float64(4.0)
            ]
        );
    }

    #[test]
    fn groupby_sum_dropna_false_keeps_null_group_via_generic_fallback() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(10),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions { dropna: false },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &[10_i64.into(), "<null>".into()]);
        assert_eq!(out.values(), &[Scalar::Float64(4.0), Scalar::Float64(2.0)]);
    }

    // --- AG-08-T: GroupBy Clone Elimination Tests ---

    /// AG-08-T #2: Int64 keys with span > 65536 forces generic path.
    #[test]
    fn groupby_sum_int_keys_generic_path_wide_span() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(0),
                Scalar::Int64(100_000), // span > 65536 -> forces generic path
                Scalar::Int64(0),
                Scalar::Int64(100_000),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &[0_i64.into(), 100_000_i64.into()]);
        assert_eq!(out.values(), &[Scalar::Float64(4.0), Scalar::Float64(6.0)]);
    }

    /// AG-08-T #4: All rows have same key -> single output group.
    #[test]
    fn groupby_sum_single_group() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("only".to_owned()),
                Scalar::Utf8("only".to_owned()),
                Scalar::Utf8("only".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &["only".into()]);
        assert_eq!(out.values(), &[Scalar::Float64(60.0)]);
    }

    /// AG-08-T #5: No rows -> empty output Series.
    #[test]
    fn groupby_sum_empty_input() {
        let keys = Series::from_values("key", Vec::<IndexLabel>::new(), Vec::<Scalar>::new())
            .expect("keys");
        let values = Series::from_values("value", Vec::<IndexLabel>::new(), Vec::<Scalar>::new())
            .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels().len(), 0);
        assert_eq!(out.values().len(), 0);
    }

    /// AG-08-T #6: Valid keys but Null/NaN values -> sum ignores missing.
    #[test]
    fn groupby_sum_missing_values_in_sum() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(5),
                Scalar::Null(NullKind::Null),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // "a": 5 + missing = 5.0; "b": missing + missing = 0.0
        assert_eq!(out.values(), &[Scalar::Float64(5.0), Scalar::Float64(0.0)]);
    }

    /// AG-08-T #7: 10000 unique keys -> all groups present, sums correct.
    #[test]
    fn groupby_sum_large_cardinality() {
        let n = 10_000usize;
        let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
        let key_values: Vec<Scalar> = (0..n).map(|i| Scalar::Utf8(format!("key_{}", i))).collect();
        let sum_values: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(i as i64)).collect();

        let keys = Series::from_values("key", labels.clone(), key_values).expect("keys");
        let values = Series::from_values("value", labels, sum_values).expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels().len(), n);
        assert_eq!(out.values().len(), n);
        // Verify a few spot checks
        assert_eq!(out.values()[0], Scalar::Float64(0.0));
        assert_eq!(out.values()[999], Scalar::Float64(999.0));
        assert_eq!(out.values()[9999], Scalar::Float64(9999.0));
    }

    /// AG-08-T #9: Generic path and dense path produce identical output
    /// for Int64 keys within dense range.
    #[test]
    fn groupby_isomorphism_generic_vs_dense() {
        use fp_index::IndexLabel;
        // Keys within dense range (span < 65536) -> dense path
        let dense_keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(5),
                Scalar::Int64(3),
                Scalar::Int64(5),
                Scalar::Int64(3),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();

        // Dense path (span of 5-3=2, within 65536)
        let dense_out = groupby_sum(
            &dense_keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("dense groupby");

        // Force generic by using Utf8 keys with same logical values
        let generic_keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("5".to_owned()),
                Scalar::Utf8("3".to_owned()),
                Scalar::Utf8("5".to_owned()),
                Scalar::Utf8("3".to_owned()),
            ],
        )
        .expect("keys");

        let generic_out = groupby_sum(
            &generic_keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("generic groupby");

        // Both should produce the same sums in the same first-seen order
        assert_eq!(dense_out.values(), generic_out.values());
        // Dense produces IndexLabel::Int64(5), generic produces IndexLabel::Utf8("5")
        // So we verify ordering is the same (first=5/key_5, second=3/key_3)
        assert_eq!(
            dense_out.index().labels().len(),
            generic_out.index().labels().len()
        );
        assert_eq!(
            dense_out.index().labels(),
            &[IndexLabel::Int64(5), IndexLabel::Int64(3)]
        );
        assert_eq!(
            generic_out.index().labels(),
            &[
                IndexLabel::Utf8("5".to_owned()),
                IndexLabel::Utf8("3".to_owned())
            ]
        );
    }

    #[test]
    fn arena_groupby_matches_global_allocator_behavior() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();

        let global = groupby_sum_with_options(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
            GroupByExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global groupby");

        let arena = groupby_sum_with_options(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
            GroupByExecutionOptions::default(),
        )
        .expect("arena groupby");

        assert_eq!(arena.index().labels(), global.index().labels());
        assert_eq!(arena.values(), global.values());
    }

    #[test]
    fn arena_groupby_falls_back_when_budget_too_small() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();

        let options = GroupByExecutionOptions {
            use_arena: true,
            arena_budget_bytes: 1,
        };
        let (fallback_out, trace) = groupby_sum_with_trace(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
            options,
        )
        .expect("fallback groupby");

        let global_out = groupby_sum_with_options(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
            GroupByExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global groupby");

        assert_eq!(fallback_out.index().labels(), global_out.index().labels());
        assert_eq!(fallback_out.values(), global_out.values());
        assert!(!trace.used_arena);
        assert!(trace.estimated_bytes > options.arena_budget_bytes);
    }

    #[test]
    fn arena_groupby_dense_path_matches_global() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(5),
                Scalar::Int64(10),
                Scalar::Int64(-2),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();

        let global = groupby_sum_with_options(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
            GroupByExecutionOptions {
                use_arena: false,
                arena_budget_bytes: 0,
            },
        )
        .expect("global groupby");

        let arena = groupby_sum_with_options(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
            GroupByExecutionOptions::default(),
        )
        .expect("arena groupby");

        assert_eq!(arena.index().labels(), global.index().labels());
        assert_eq!(arena.values(), global.values());
    }

    #[test]
    fn arena_groupby_stable_across_repeated_operations() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("y".to_owned())],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(10), Scalar::Int64(20)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let options = GroupByExecutionOptions::default();

        for _ in 0..1_000 {
            let out = groupby_sum_with_options(
                &keys,
                &values,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut ledger,
                options,
            )
            .expect("arena groupby");
            assert_eq!(out.index().labels().len(), 2);
            assert_eq!(
                out.values(),
                &[Scalar::Float64(10.0), Scalar::Float64(20.0)]
            );
        }
    }
}
