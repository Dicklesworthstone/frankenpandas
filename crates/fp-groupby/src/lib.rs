#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! Standalone groupby aggregation engine for **frankenpandas** —
//! the per-aggregation kernels that power `Series.groupby(...).sum()`
//! / `.mean()` / `.std()` / etc. on top of fp-frame's groupby
//! façade.
//!
//! ## Why a separate crate
//!
//! fp-frame's `SeriesGroupBy` / DataFrame groupby wrappers expose
//! the user-facing API. The actual hash-build, group-key materialise,
//! per-key reduction, and bumpalo-backed intermediate arena live
//! here — separated so per-agg kernels can be tested, fuzzed, and
//! optimized in isolation without the full fp-frame surface in
//! scope.
//!
//! ## Aggregation kernels (pandas `.groupby(...).<agg>()`)
//!
//! - **Sum-family**: [`groupby_sum`] / [`groupby_sum_with_options`],
//!   [`groupby_prod`].
//! - **Central tendency**: [`groupby_mean`], [`groupby_median`].
//! - **Variability**: [`groupby_std`], [`groupby_var`].
//! - **Extremes**: [`groupby_min`], [`groupby_max`],
//!   [`groupby_first`], [`groupby_last`].
//! - **Counting**: [`groupby_count`], [`groupby_size`],
//!   [`groupby_nunique`].
//! - **Dispatcher**: [`groupby_agg`] takes an [`AggFunc`] enum tag
//!   and dispatches to the matching kernel — useful for callers
//!   that want a uniform entry point (e.g. `df.groupby(k).agg(['sum',
//!   'mean'])` style multi-agg).
//!
//! ## Tunables
//!
//! - [`GroupByOptions`]: per-call shape options (sort group keys,
//!   skipna policy, observed-categorical, ...).
//! - [`GroupByExecutionOptions`]: lower-level execution knobs
//!   (bumpalo arena reuse hints, hash-build seed, ...).
//!
//! ## Approximate primitives
//!
//! - [`HyperLogLog`]: cardinality-estimation sketch. Used internally
//!   by [`groupby_nunique`] when the underlying group has a large
//!   number of unique values; exposed publicly for callers building
//!   their own approximate aggregations.
//!
//! ## Error reporting
//!
//! [`GroupByError`] enumerates failure modes (key column dtype
//! mismatch, length mismatch, alignment-plan violation, ...).
//!
//! ## Cross-crate relationships
//!
//! - **fp-columnar** ([`Column`], [`ColumnError`]) for column ops.
//! - **fp-frame** ([`FrameError`], [`Series`]) for the wrapper
//!   types that expose this crate's kernels via `.groupby()`.
//! - **fp-index** ([`Index`], [`IndexError`], [`IndexLabel`],
//!   [`align_union`], [`validate_alignment_plan`]) for the
//!   group-key index alignment.
//! - **fp-runtime** ([`EvidenceLedger`], [`RuntimePolicy`]) for
//!   optional decision recording.
//! - **fp-types** ([`Scalar`], [`NullKind`], [`Timedelta`]) for
//!   the underlying value machinery.

use std::{cmp::Ordering, mem::size_of};

use bumpalo::{Bump, collections::Vec as BumpVec};
use fp_columnar::{Column, ColumnError};
use fp_frame::{FrameError, Series};
use fp_index::{Index, IndexError, IndexLabel, align_union, validate_alignment_plan};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{DType, IntervalClosed, NullKind, PeriodFreq, Scalar, Timedelta, Timestamp};
// Group accumulation maps key on GroupKeyRef and read group ORDER from a
// separate `ordering` Vec (first-seen order), never from map iteration. So the
// hasher is observationally invisible: swapping SipHash -> FxHash changes only
// bucket placement, not any output value or order. FxHash (rustc-hash) is pure
// safe Rust; on the hot string-key path it is ~2x the std SipHasher.
use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupByOptions {
    pub dropna: bool,
    pub sort: bool,
}

impl Default for GroupByOptions {
    fn default() -> Self {
        Self {
            dropna: true,
            sort: true,
        }
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
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
    exec_options: GroupByExecutionOptions,
) -> Result<Series, GroupByError> {
    let (result, _trace) =
        groupby_sum_with_trace(keys, values, options, policy, ledger, exec_options)?;
    Ok(result)
}

fn groupby_sum_with_trace(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
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

    let input_rows = aligned_storage
        .as_ref()
        .map_or_else(|| keys.len(), |(aligned_keys, _)| aligned_keys.len());
    // Record an admission decision for policy observability without altering
    // the current groupby output behavior.
    let _ = policy.decide_join_admission(input_rows, ledger);
    let estimated_bytes = estimate_groupby_intermediate_bytes(input_rows);
    let use_arena = exec_options.use_arena && estimated_bytes <= exec_options.arena_budget_bytes;

    // Identity-aligned, all-valid Int64 columns already expose contiguous raw
    // buffers. Keep the dense direct-address algorithm, but run it before
    // `values()` materializes two Scalar arrays. The helper preserves the
    // exact i128 accumulator, sorted/first-seen order, and overflow promotion
    // used by `try_groupby_sum_dense_int64_values`; wide key ranges still fall
    // through to the former allocator-selected Scalar route.
    if aligned_storage.is_none()
        && let (Some(raw_keys), Some(raw_values)) =
            (keys.column().as_i64_slice(), values.column().as_i64_slice())
        && let Some((out_index, out_values)) =
            try_groupby_sum_dense_int64_slices(raw_keys, raw_values, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        let result = Series::new("sum", Index::new(out_index), out_column)?;
        return Ok((
            result,
            GroupByExecutionTrace {
                used_arena: false,
                input_rows,
                estimated_bytes,
            },
        ));
    }

    let (aligned_keys_values, aligned_values_values): (&[Scalar], &[Scalar]) =
        if let Some((aligned_keys, aligned_values)) = aligned_storage.as_ref() {
            (aligned_keys.values(), aligned_values.values())
        } else {
            (keys.values(), values.values())
        };

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
    // Per br-frankenpandas-qrn2w: detect uniformly-Timedelta64 input and
    // route to a typed i64-ns accumulator path. The f64 paths below call
    // value.to_f64() which fails on Timedelta64, silently dropping every
    // value and producing all-zero Float64 output. pandas
    // df.groupby(k)["dur"].sum() on Timedelta64 returns Timedelta64.
    if is_timedelta_values(aligned_values_values) {
        return groupby_sum_timedelta64(aligned_keys_values, aligned_values_values, options);
    }
    // Per br-frankenpandas-f031e sister gap: pandas groupby.sum() on object/
    // string concatenates per group (skipna), like Series::sum. The f64 paths
    // below call value.to_f64() which fails on Utf8, silently producing all-zero
    // Float64 output. Route uniformly-Utf8 values to the string-concat path.
    if is_utf8_values(aligned_values_values) {
        return groupby_sum_utf8(aligned_keys_values, aligned_values_values, options);
    }
    // Per br-frankenpandas-l75ms: pandas groupby.sum() preserves the integer
    // dtype — Int64 (and Bool, summed as 0/1) stay Int64, like Series::sum. The
    // f64 accumulator paths below would emit Float64. Route uniformly-Int64/Bool
    // values to a dtype-preserving i128 accumulator (saturating to Float64 only
    // on i64 overflow, matching fp-frame's groupby sum).
    if is_int64_or_bool_values(aligned_values_values) {
        return groupby_sum_int64(aligned_keys_values, aligned_values_values, options);
    }
    if let Some((out_index, out_values)) = try_groupby_sum_dense_int64(
        aligned_keys_values,
        aligned_values_values,
        options.dropna,
        options.sort,
    ) {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new("sum", Index::new(out_index), out_column)?);
    }

    // AG-08: Store (source_index, sum) instead of (Scalar, sum) to eliminate
    // per-group key.clone() allocations. Reconstruct IndexLabel at output phase.
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut slot = FxHashMap::<GroupKeyRef<'_>, (usize, f64)>::default();

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

    if options.sort {
        sort_group_ordering_by(aligned_keys_values, &mut ordering, |key| {
            slot.get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    emit_groupby_result(aligned_keys_values, &ordering, &mut slot)
}

fn groupby_sum_with_arena(
    aligned_keys_values: &[Scalar],
    aligned_values_values: &[Scalar],
    options: GroupByOptions,
) -> Result<Series, GroupByError> {
    // Per br-frankenpandas-qrn2w: Timedelta64 dispatch — see global-allocator
    // path for full justification. Arena-allocated intermediates aren't
    // beneficial for the relatively-uncommon Timedelta64 surface, so reuse
    // the global-allocator typed path here.
    if is_timedelta_values(aligned_values_values) {
        return groupby_sum_timedelta64(aligned_keys_values, aligned_values_values, options);
    }
    // Per br-frankenpandas-f031e sister gap: string columns concatenate per
    // group (see global-allocator path). Reuse the same typed string path.
    if is_utf8_values(aligned_values_values) {
        return groupby_sum_utf8(aligned_keys_values, aligned_values_values, options);
    }
    // Per br-frankenpandas-l75ms: pandas groupby.sum() preserves the integer
    // dtype — Int64 (and Bool, summed as 0/1) stay Int64, like Series::sum. The
    // f64 accumulator paths below would emit Float64. Route uniformly-Int64/Bool
    // values to a dtype-preserving i128 accumulator (saturating to Float64 only
    // on i64 overflow, matching fp-frame's groupby sum).
    if is_int64_or_bool_values(aligned_values_values) {
        return groupby_sum_int64(aligned_keys_values, aligned_values_values, options);
    }
    // AG-06: Arena-backed dense path intermediates.
    if let Some((out_index, out_values)) = try_groupby_sum_dense_int64_arena(
        aligned_keys_values,
        aligned_values_values,
        options.dropna,
        options.sort,
    ) {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new("sum", Index::new(out_index), out_column)?);
    }

    // AG-06 + AG-08: Arena-back the ordering vector. Store source index
    // instead of cloned Scalar to eliminate per-group allocations.
    let arena = Bump::new();
    let mut ordering = BumpVec::<GroupKeyRef<'_>>::new_in(&arena);
    let mut slot = FxHashMap::<GroupKeyRef<'_>, (usize, f64)>::default();

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

    if options.sort {
        sort_group_ordering_by(aligned_keys_values, ordering.as_mut_slice(), |key| {
            slot.get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    emit_groupby_result(aligned_keys_values, ordering.as_slice(), &mut slot)
}

/// Convert accumulated groupby results into the output Series.
/// AG-08: Uses source index to reconstruct IndexLabel without Scalar clones.
fn emit_groupby_result<'a>(
    source_keys: &[Scalar],
    ordering: &[GroupKeyRef<'a>],
    slot: &mut FxHashMap<GroupKeyRef<'a>, (usize, f64)>,
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
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            // Typed null group label (br-frankenpandas-8m6ay): pandas
            // groupby(dropna=False) keeps ONE collapsed null group labeled
            // nan (None and nan merge — UNLIKE value_counts' kind-distinct
            // buckets); NaT keys keep the NaT label. Verified pandas 2.2.3.
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN) | Scalar::Null(NullKind::Null) => {
                IndexLabel::Null(NullKind::NaN)
            }
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
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
    Timedelta64(i64),
    Datetime64(i64),
    Period(i64),
    Interval(u64, u64, fp_types::IntervalClosed),
}

impl<'a> GroupKeyRef<'a> {
    fn from_scalar(key: &'a Scalar) -> Self {
        match key {
            Scalar::Bool(v) => Self::Bool(*v),
            Scalar::Int64(v) => Self::Int64(*v),
            Scalar::Float64(v) => {
                if v.is_nan() {
                    Self::FloatBits(f64::NAN.to_bits())
                } else {
                    let normalized = if *v == 0.0 { 0.0 } else { *v };
                    Self::FloatBits(normalized.to_bits())
                }
            }
            Scalar::Utf8(v) => Self::Utf8(v.as_str()),
            Scalar::Null(kind) => Self::Null(*kind),
            Scalar::Timedelta64(v) => {
                if *v == Timedelta::NAT {
                    Self::Null(NullKind::NaT)
                } else {
                    Self::Timedelta64(*v)
                }
            }
            Scalar::Datetime64(v) => {
                if *v == Timestamp::NAT {
                    Self::Null(NullKind::NaT)
                } else {
                    Self::Datetime64(*v)
                }
            }
            Scalar::Period(v) => {
                if v.ordinal == i64::MIN {
                    Self::Null(NullKind::NaT)
                } else {
                    Self::Period(v.ordinal)
                }
            }
            Scalar::Interval(iv) => {
                Self::Interval(iv.left.to_bits(), iv.right.to_bits(), iv.closed)
            }
        }
    }
}

fn compare_group_labels(left: &Scalar, right: &Scalar) -> Ordering {
    match (left.is_missing(), right.is_missing()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => left.semantic_cmp(right),
    }
}

fn sort_group_ordering_by<'a, F>(
    source_keys: &[Scalar],
    ordering: &mut [GroupKeyRef<'a>],
    source_idx_for: F,
) where
    F: Fn(&GroupKeyRef<'a>) -> usize,
{
    ordering.sort_by(|left, right| {
        compare_group_labels(
            &source_keys[source_idx_for(left)],
            &source_keys[source_idx_for(right)],
        )
    });
}

/// Detect uniformly-Timedelta64 value input (allowing Null/NaT missing
/// markers). Mirrors `fp-types::is_timedelta_input` — returns true only
/// when at least one non-missing value is `Timedelta64` and no other
/// dtype appears. Used by `groupby_sum` to route Timedelta64 sums to a
/// dtype-preserving path instead of silently dropping via the f64 path.
fn is_timedelta_values(values: &[Scalar]) -> bool {
    let mut saw_td = false;
    for v in values {
        if v.is_missing() {
            continue;
        }
        match v {
            Scalar::Timedelta64(_) => saw_td = true,
            _ => return false,
        }
    }
    saw_td
}

/// Per br-frankenpandas-qrn2w: Timedelta64-aware groupby sum. Accumulates
/// in i64 nanoseconds with saturation on overflow (matches `fp-types`
/// Timedelta arithmetic) and emits `Scalar::Timedelta64` per group so
/// the output Series retains Timedelta64 dtype. Matches pandas
/// `df.groupby(k)["dur"].sum()` which returns a Timedelta64 Series.
fn groupby_sum_timedelta64(
    keys: &[Scalar],
    values: &[Scalar],
    options: GroupByOptions,
) -> Result<Series, GroupByError> {
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut slot = FxHashMap::<GroupKeyRef<'_>, (usize, i64)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if options.dropna && key.is_missing() {
            continue;
        }
        let key_id = GroupKeyRef::from_scalar(key);
        let entry = slot.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, 0_i64)
        });
        if value.is_missing() {
            continue;
        }
        if let Scalar::Timedelta64(ns) = value {
            entry.1 = Timedelta::add(entry.1, *ns);
        }
    }

    if options.sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            slot.get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, sum) = slot
            .remove(key)
            .expect("ordering references only inserted keys");
        let label = &keys[source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            // Typed null group label (br-frankenpandas-8m6ay): pandas
            // groupby(dropna=False) keeps ONE collapsed null group labeled
            // nan (None and nan merge — UNLIKE value_counts' kind-distinct
            // buckets); NaT keys keep the NaT label. Verified pandas 2.2.3.
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN) | Scalar::Null(NullKind::Null) => {
                IndexLabel::Null(NullKind::NaN)
            }
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(Scalar::Timedelta64(sum));
    }

    let out_column = Column::new(DType::Timedelta64, out_values)?;
    Ok(Series::new("sum", Index::new(out_index), out_column)?)
}

/// Detect uniformly-Utf8 value input (allowing Null missing markers). Mirrors
/// `is_timedelta_values` — true only when at least one non-missing value is
/// `Utf8` and no other dtype appears. Routes `groupby_sum` to the string-concat
/// path instead of silently dropping every value through the f64 accumulator.
fn is_utf8_values(values: &[Scalar]) -> bool {
    let mut saw_utf8 = false;
    for v in values {
        if v.is_missing() {
            continue;
        }
        match v {
            Scalar::Utf8(_) => saw_utf8 = true,
            _ => return false,
        }
    }
    saw_utf8
}

/// Per br-frankenpandas-f031e sister gap: object/string `groupby.sum()`
/// concatenates each group's non-null values in encounter order, matching
/// pandas and FP `Series::sum`. Default skipna=True, so missing values are
/// skipped; an empty / all-null group yields `Scalar::Utf8("")`.
fn groupby_sum_utf8(
    keys: &[Scalar],
    values: &[Scalar],
    options: GroupByOptions,
) -> Result<Series, GroupByError> {
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut slot = FxHashMap::<GroupKeyRef<'_>, (usize, String)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if options.dropna && key.is_missing() {
            continue;
        }
        let key_id = GroupKeyRef::from_scalar(key);
        let entry = slot.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, String::new())
        });
        if let Scalar::Utf8(s) = value {
            entry.1.push_str(s);
        }
    }

    if options.sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            slot.get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, joined) = slot
            .remove(key)
            .expect("ordering references only inserted keys");
        let label = &keys[source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            // Typed null group label (br-frankenpandas-8m6ay): pandas
            // groupby(dropna=False) keeps ONE collapsed null group labeled
            // nan (None and nan merge — UNLIKE value_counts' kind-distinct
            // buckets); NaT keys keep the NaT label. Verified pandas 2.2.3.
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN) | Scalar::Null(NullKind::Null) => {
                IndexLabel::Null(NullKind::NaN)
            }
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(Scalar::Utf8(joined));
    }

    let out_column = Column::new(DType::Utf8, out_values)?;
    Ok(Series::new("sum", Index::new(out_index), out_column)?)
}

/// Detect uniformly-Int64-or-Bool value input (allowing Null missing markers).
/// Mirrors `is_timedelta_values`/`is_utf8_values`. A pandas integer/boolean
/// column sums dtype-preservingly to Int64, so route it to `groupby_sum_int64`
/// instead of the f64 accumulator paths that emit Float64.
fn is_int64_or_bool_values(values: &[Scalar]) -> bool {
    let mut saw = false;
    for v in values {
        if v.is_missing() {
            // pandas' non-nullable int/bool column promotes to Float64 the moment
            // any value is missing (NaN cannot live in int64) — whether the gap
            // came from alignment or was an explicit null — so groupby.sum() is
            // Float64, NOT Int64. Do not take the Int64-preserving path.
            // (br-frankenpandas-33d1h)
            return false;
        }
        match v {
            Scalar::Int64(_) | Scalar::Bool(_) => saw = true,
            _ => return false,
        }
    }
    saw
}

/// Per br-frankenpandas-l75ms: dtype-preserving groupby sum for Int64/Bool
/// columns. Accumulates each group in i128 (Bool counted as 0/1, matching
/// pandas) and emits `Scalar::Int64`, falling back to `Scalar::Float64` only
/// when a group total overflows i64 — identical to fp-frame's `sum_group_vals`.
fn groupby_sum_int64(
    keys: &[Scalar],
    values: &[Scalar],
    options: GroupByOptions,
) -> Result<Series, GroupByError> {
    if let Some((out_index, out_values)) =
        try_groupby_sum_dense_int64_values(keys, values, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new("sum", Index::new(out_index), out_column)?);
    }

    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut slot = FxHashMap::<GroupKeyRef<'_>, (usize, i128)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if options.dropna && key.is_missing() {
            continue;
        }
        let key_id = GroupKeyRef::from_scalar(key);
        let entry = slot.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, 0_i128)
        });
        match value {
            Scalar::Int64(v) => entry.1 += i128::from(*v),
            Scalar::Bool(b) => entry.1 += i128::from(*b),
            _ => {} // missing / null skipped (skipna=True)
        }
    }

    if options.sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            slot.get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, total) = slot
            .remove(key)
            .expect("ordering references only inserted keys");
        let label = &keys[source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            // Typed null group label (br-frankenpandas-8m6ay): pandas
            // groupby(dropna=False) keeps ONE collapsed null group labeled
            // nan (None and nan merge — UNLIKE value_counts' kind-distinct
            // buckets); NaT keys keep the NaT label. Verified pandas 2.2.3.
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN) | Scalar::Null(NullKind::Null) => {
                IndexLabel::Null(NullKind::NaN)
            }
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(match i64::try_from(total) {
            Ok(v) => Scalar::Int64(v),
            Err(_) => Scalar::Float64(total as f64),
        });
    }

    let out_column = Column::from_values(out_values)?;
    Ok(Series::new("sum", Index::new(out_index), out_column)?)
}

const DENSE_INT_KEY_RANGE_LIMIT: i128 = 65_536;

/// Dense-bucket `groupby_sum` directly over all-valid Int64 buffers.
///
/// This is the raw-slice equivalent of `try_groupby_sum_dense_int64_values`:
/// it uses the same i128 totals, first-seen tape, ascending bucket emission,
/// and i64-overflow promotion while avoiding full-column Scalar materialization.
fn try_groupby_sum_dense_int64_slices(
    keys: &[i64],
    values: &[i64],
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if keys.len() != values.len() {
        return None;
    }
    if keys.is_empty() {
        return Some((Vec::new(), Vec::new()));
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    for &key in keys {
        min_key = min_key.min(key);
        max_key = max_key.max(key);
    }

    let span = i128::from(max_key) - i128::from(min_key) + 1;
    if span <= 0 || span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }

    let bucket_len = usize::try_from(span).ok()?;
    let mut sums = vec![0_i128; bucket_len];
    let mut seen = vec![false; bucket_len];
    let mut ordering = Vec::<i64>::new();
    for (&key, &value) in keys.iter().zip(values) {
        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        if !seen[bucket] {
            seen[bucket] = true;
            ordering.push(key);
        }
        sums[bucket] += i128::from(value);
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    if sort {
        for (bucket, was_seen) in seen.iter().enumerate() {
            if !*was_seen {
                continue;
            }
            let key = min_key.checked_add(i64::try_from(bucket).ok()?)?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(match i64::try_from(sums[bucket]) {
                Ok(value) => Scalar::Int64(value),
                Err(_) => Scalar::Float64(sums[bucket] as f64),
            });
        }
    } else {
        for key in ordering {
            let raw = i128::from(key) - i128::from(min_key);
            let bucket = usize::try_from(raw).ok()?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(match i64::try_from(sums[bucket]) {
                Ok(value) => Scalar::Int64(value),
                Err(_) => Scalar::Float64(sums[bucket] as f64),
            });
        }
    }

    Some((out_index, out_values))
}

/// Dense-bucket `groupby_mean` directly over all-valid Int64 buffers.
///
/// This is the raw-slice equivalent of the `AggFunc::Mean` arm in
/// `try_groupby_agg_dense_int64`: it casts each value to f64, folds in row
/// order, and divides by the same per-bucket count. Keeping it before
/// `Series::values()` avoids materializing both input columns as Scalars.
fn try_groupby_mean_dense_int64_slices(
    keys: &[i64],
    values: &[i64],
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if keys.len() != values.len() {
        return None;
    }
    if keys.is_empty() {
        return Some((Vec::new(), Vec::new()));
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    for &key in keys {
        min_key = min_key.min(key);
        max_key = max_key.max(key);
    }

    let span = i128::from(max_key) - i128::from(min_key) + 1;
    if span <= 0 || span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }

    let bucket_len = usize::try_from(span).ok()?;
    let mut sums = vec![0.0_f64; bucket_len];
    let mut counts = vec![0_i64; bucket_len];
    let mut seen = vec![false; bucket_len];
    let mut ordering = Vec::<i64>::new();
    for (&key, &value) in keys.iter().zip(values) {
        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        if !seen[bucket] {
            seen[bucket] = true;
            ordering.push(key);
        }
        sums[bucket] += value as f64;
        counts[bucket] += 1;
    }

    if sort {
        ordering.sort_unstable();
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in ordering {
        let raw = i128::from(key) - i128::from(min_key);
        let bucket = usize::try_from(raw).ok()?;
        out_index.push(IndexLabel::Int64(key));
        out_values.push(Scalar::Float64(sums[bucket] / counts[bucket] as f64));
    }

    Some((out_index, out_values))
}

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

/// Dense-bucket fast path for dtype-preserving `Int64`/`Bool` sums.
///
/// The generic `groupby_sum_int64` path exists to preserve pandas' integer
/// output dtype, but it still hashes every bounded integer key. This helper
/// keeps that dtype/overflow behavior while using direct-address buckets when
/// every non-dropped key is `Int64` and the key span is bounded.
fn try_groupby_sum_dense_int64_values(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
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
    let mut sums = vec![0_i128; bucket_len];
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

        match value {
            Scalar::Int64(v) => sums[bucket] += i128::from(*v),
            Scalar::Bool(v) => sums[bucket] += i128::from(*v),
            _ => return None,
        }
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    if sort {
        for (bucket, was_seen) in seen.iter().enumerate() {
            if !*was_seen {
                continue;
            }
            let key = min_key.checked_add(i64::try_from(bucket).ok()?)?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(match i64::try_from(sums[bucket]) {
                Ok(value) => Scalar::Int64(value),
                Err(_) => Scalar::Float64(sums[bucket] as f64),
            });
        }
    } else {
        for key in ordering {
            let raw = i128::from(key) - i128::from(min_key);
            let bucket = usize::try_from(raw).ok()?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(match i64::try_from(sums[bucket]) {
                Ok(value) => Scalar::Int64(value),
                Err(_) => Scalar::Float64(sums[bucket] as f64),
            });
        }
    }

    Some((out_index, out_values))
}

/// Dense-bucket fast path for `Int64` keys.
///
/// Falls back to the generic map path unless every non-dropped key is `Int64`
/// and the key span is within a bounded range budget.
fn try_groupby_sum_dense_int64(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
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
    if sort {
        for (bucket, was_seen) in seen.iter().enumerate() {
            if !*was_seen {
                continue;
            }
            let key = min_key.checked_add(i64::try_from(bucket).ok()?)?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(Scalar::Float64(sums[bucket]));
        }
    } else {
        for key in ordering {
            let raw = i128::from(key) - i128::from(min_key);
            let bucket = usize::try_from(raw).ok()?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(Scalar::Float64(sums[bucket]));
        }
    }

    Some((out_index, out_values))
}

/// AG-06: Arena-backed dense bucket fast path. The `sums`, `seen`, and `ordering`
/// vectors live in the arena and are freed in bulk when the arena drops.
fn try_groupby_sum_dense_int64_arena(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
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
    if sort {
        for (bucket, was_seen) in seen.iter().enumerate() {
            if !*was_seen {
                continue;
            }
            let key = min_key.checked_add(i64::try_from(bucket).ok()?)?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(Scalar::Float64(sums[bucket]));
        }
    } else {
        for key in ordering.iter().copied() {
            let raw = i128::from(key) - i128::from(min_key);
            let bucket = usize::try_from(raw).ok()?;
            out_index.push(IndexLabel::Int64(key));
            out_values.push(Scalar::Float64(sums[bucket]));
        }
    }

    Some((out_index, out_values))
}

// ---------------------------------------------------------------------------
// bd-2gi.16: Generic GroupBy Aggregation
// ---------------------------------------------------------------------------

/// Aggregation function selector for groupby operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFunc {
    Sum,
    Mean,
    Count,
    Min,
    Max,
    First,
    Last,
    Std,
    Var,
    Median,
    Nunique,
    Prod,
    Size,
}

/// Generic groupby aggregation supporting all standard aggregation functions.
///
/// Matches `df.groupby(keys).agg(func)` semantics:
/// - Groups by key values, sorting by group label by default.
/// - Applies the specified aggregation to each group's values.
/// - Respects `dropna` and `sort` options for null keys and group ordering.
///
/// For Sum, the optimized `groupby_sum()` with dense Int64 and arena paths
/// is preferred; this function uses the generic HashMap path for all ops.
/// Dense direct-address streaming `groupby_agg` for bounded all-valid `Int64`
/// keys and numeric (`Int64`/`Float64`) values, for the aggregations whose
/// result is a fold over each group's in-order non-missing values:
/// `Mean`, `Count`, `Size`, `First`, `Last`, `Min`, `Max`, `Prod` (single
/// pass), plus `Var`/`Std` (a second pass for the squared deviations).
///
/// Replaces the generic path's `FxHashMap<GroupKeyRef, (usize, Vec<Scalar>,
/// usize)>` — which hashes every row AND clones every value into a per-group
/// `Vec<Scalar>` — with a key-`min` indexed bucket table plus streaming scalar
/// accumulators (no value collection, no hashing).
///
/// Bit-identical to the generic path: rows are scanned in input order so each
/// bucket folds the same values in the same order the group `Vec` would hold,
/// and `nanmean`/`nancount` are themselves in-order folds of the non-missing
/// `to_f64()` values (`mean = Σ/​count`, `count` = non-missing count). `Size`
/// counts all rows; `First`/`Last` take the first/last non-missing value
/// (dtype preserved). Group order is first-seen, or ascending key under `sort`
/// (matching `compare_group_labels` on non-missing `Int64` labels). Returns
/// `None` (caller uses the generic path) for any non-`Int64` key, a wide key
/// span, non-numeric values, or an unsupported aggregation.
#[allow(clippy::too_many_arguments)]
fn try_groupby_agg_dense_int64(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(
        func,
        AggFunc::Mean
            | AggFunc::Count
            | AggFunc::Size
            | AggFunc::First
            | AggFunc::Last
            | AggFunc::Min
            | AggFunc::Max
            | AggFunc::Prod
            | AggFunc::Var
            | AggFunc::Std
    ) {
        return None;
    }

    // Int64/Bool prod must preserve Int64 output (pandas parity, mirroring Sum).
    // The dense path accumulates a Float64 product, so route integer/bool prod to
    // the generic scalar path which keeps an i64 product. Int64 columns are always
    // all-valid (mixed Int64+Null upcasts to Float64 at construction), so the first
    // non-missing value reliably reflects the column dtype. br-frankenpandas-rl25i.
    if matches!(func, AggFunc::Prod)
        && values
            .iter()
            .find(|v| !v.is_missing())
            .is_some_and(|v| matches!(v, Scalar::Int64(_) | Scalar::Bool(_)))
    {
        return None;
    }

    let (min_key, max_key, saw_int_key) = dense_int64_range(keys, dropna)?;
    if !saw_int_key {
        return Some((Vec::new(), Vec::new()));
    }
    let span = i128::from(max_key) - i128::from(min_key) + 1;
    if span <= 0 || span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }
    let bucket_len = usize::try_from(span).ok()?;

    let needs_var = matches!(func, AggFunc::Var | AggFunc::Std);
    // Var/Std need the per-bucket mean first, so they also accumulate `sum`.
    let needs_mean = matches!(func, AggFunc::Mean) || needs_var;
    let needs_prod = matches!(func, AggFunc::Prod);
    // First/Last/Min/Max all retain a representative scalar per bucket.
    let needs_value = matches!(
        func,
        AggFunc::First | AggFunc::Last | AggFunc::Min | AggFunc::Max
    );

    let mut sum = vec![0.0_f64; bucket_len];
    let mut prod = if needs_prod {
        vec![1.0_f64; bucket_len]
    } else {
        Vec::new()
    };
    let mut non_missing = vec![0_i64; bucket_len];
    let mut total = vec![0_i64; bucket_len];
    let mut value_slot: Vec<Option<Scalar>> = if needs_value {
        vec![None; bucket_len]
    } else {
        Vec::new()
    };
    let mut first_seen = vec![false; bucket_len];
    let mut ordering = Vec::<i64>::new();

    for (key, value) in keys.iter().zip(values.iter()) {
        let key = match key {
            Scalar::Int64(v) => *v,
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        };
        let bucket = usize::try_from(i128::from(key) - i128::from(min_key)).ok()?;
        if !first_seen[bucket] {
            first_seen[bucket] = true;
            ordering.push(key);
        }
        total[bucket] += 1;
        if value.is_missing() {
            continue;
        }
        non_missing[bucket] += 1;
        if needs_mean {
            // Bail to the generic path on any non-numeric value (nanmean has a
            // separate Timedelta accumulator we would not reproduce here).
            let x = value.to_f64().ok()?;
            sum[bucket] += x;
        }
        if needs_prod {
            // nanprod multiplies to_f64() of every non-missing value in order.
            let x = value.to_f64().ok()?;
            prod[bucket] *= x;
        }
        if needs_value {
            match func {
                AggFunc::First if value_slot[bucket].is_none() => {
                    value_slot[bucket] = Some(value.clone());
                }
                AggFunc::Last => value_slot[bucket] = Some(value.clone()),
                // nanmin/nanmax: keep the first non-missing on a tie (strict
                // </>), dtype preserved. Only Int64/Float64 columns take this
                // path; any other value dtype bails to the generic nanmin/nanmax.
                AggFunc::Min => {
                    let replace = match (value_slot[bucket].as_ref(), value) {
                        (None, _) => true,
                        (Some(Scalar::Int64(a)), Scalar::Int64(b)) => b < a,
                        (Some(Scalar::Float64(a)), Scalar::Float64(b)) => b < a,
                        _ => return None,
                    };
                    if replace {
                        value_slot[bucket] = Some(value.clone());
                    }
                }
                AggFunc::Max => {
                    let replace = match (value_slot[bucket].as_ref(), value) {
                        (None, _) => true,
                        (Some(Scalar::Int64(a)), Scalar::Int64(b)) => b > a,
                        (Some(Scalar::Float64(a)), Scalar::Float64(b)) => b > a,
                        _ => return None,
                    };
                    if replace {
                        value_slot[bucket] = Some(value.clone());
                    }
                }
                _ => {}
            }
        }
    }

    // Second pass for Var/Std: nanvar is two-pass (mean, then Σ(x-mean)²). With
    // the per-bucket mean known from pass 1, re-scan and accumulate squared
    // deviations per bucket in input order — bit-identical to
    // `nums.iter().map(|x| (x-mean).powi(2)).sum()` (same finite values, same
    // order). No per-group Vec, just one extra O(n) scan.
    let mut sum_sq = Vec::new();
    if needs_var {
        sum_sq = vec![0.0_f64; bucket_len];
        for (key, value) in keys.iter().zip(values.iter()) {
            let key = match key {
                Scalar::Int64(v) => *v,
                Scalar::Null(_) if dropna => continue,
                _ => return None,
            };
            if value.is_missing() {
                continue;
            }
            let bucket = (i128::from(key) - i128::from(min_key)) as usize;
            let mean = sum[bucket] / non_missing[bucket] as f64;
            let x = value.to_f64().ok()?;
            sum_sq[bucket] += (x - mean).powi(2);
        }
    }

    if sort {
        ordering.sort_unstable();
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for &key in &ordering {
        let bucket = (i128::from(key) - i128::from(min_key)) as usize;
        out_index.push(IndexLabel::Int64(key));
        let agg = match func {
            AggFunc::Mean => {
                if non_missing[bucket] == 0 {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Float64(sum[bucket] / non_missing[bucket] as f64)
                }
            }
            AggFunc::Count => Scalar::Int64(non_missing[bucket]),
            AggFunc::Size => Scalar::Int64(total[bucket]),
            AggFunc::First | AggFunc::Last | AggFunc::Min | AggFunc::Max => value_slot[bucket]
                .take()
                .unwrap_or(Scalar::Null(NullKind::NaN)),
            // nanprod over all-missing / empty group is the 1.0 identity.
            AggFunc::Prod => Scalar::Float64(prod[bucket]),
            // nanvar/nanstd with ddof=1: Null(NaN) when n <= 1, else
            // sum_sq/(n-1) (and its sqrt for Std).
            AggFunc::Var | AggFunc::Std => {
                let n = non_missing[bucket];
                if n <= 1 {
                    Scalar::Null(NullKind::NaN)
                } else {
                    let var = sum_sq[bucket] / (n - 1) as f64;
                    Scalar::Float64(if matches!(func, AggFunc::Std) {
                        var.sqrt()
                    } else {
                        var
                    })
                }
            }
            _ => unreachable!("dense path gated to the supported aggregations"),
        };
        out_values.push(agg);
    }
    Some((out_index, out_values))
}

/// Dense `Nunique` for bounded all-valid `Int64` keys AND bounded `Int64`
/// values: counts distinct non-missing values per group with a 2-D
/// `[group × value]` seen-bitset instead of the generic path's per-group
/// `FxHashSet`. Bit-identical — nunique is an order-independent distinct count,
/// and for `Int64` values `nannunique`'s `ScalarKey::Int64` equality is plain
/// `i64 ==`, so the dense cell `(key-min, value-vmin)` collides on exactly the
/// same pairs. Returns `None` (caller uses the generic path) for any non-`Int64`
/// key/value, a wide key span, or a `group × value` cell count over the gate.
fn try_groupby_nunique_dense_int64(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    let (min_key, max_key, saw_int_key) = dense_int64_range(keys, dropna)?;
    if !saw_int_key {
        return Some((Vec::new(), Vec::new()));
    }
    let key_span = i128::from(max_key) - i128::from(min_key) + 1;
    if key_span <= 0 || key_span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }
    let key_span = key_span as usize;

    // Value range over non-missing values; bail on any non-Int64 value (Float64/
    // string/etc. nunique stays on the generic FxHashSet path).
    let mut v_min = i64::MAX;
    let mut v_max = i64::MIN;
    let mut saw_value = false;
    for value in values {
        match value {
            Scalar::Int64(v) => {
                saw_value = true;
                v_min = v_min.min(*v);
                v_max = v_max.max(*v);
            }
            other if other.is_missing() => {}
            _ => return None,
        }
    }
    // value_span is 1 (a dummy, unused) when every value is missing.
    let value_span = if saw_value {
        (i128::from(v_max) - i128::from(v_min) + 1) as usize
    } else {
        1
    };
    let cells = (key_span as i128) * (value_span as i128);
    if cells > (1_i128 << 24) {
        return None;
    }

    let mut seen = vec![false; cells as usize];
    let mut nunique = vec![0_i64; key_span];
    let mut first_seen = vec![false; key_span];
    let mut ordering = Vec::<i64>::new();

    for (key, value) in keys.iter().zip(values.iter()) {
        let key = match key {
            Scalar::Int64(v) => *v,
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        };
        let bucket = (i128::from(key) - i128::from(min_key)) as usize;
        if !first_seen[bucket] {
            first_seen[bucket] = true;
            ordering.push(key);
        }
        let v = match value {
            Scalar::Int64(v) => *v,
            other if other.is_missing() => continue,
            _ => return None,
        };
        let cell = bucket * value_span + (i128::from(v) - i128::from(v_min)) as usize;
        if !seen[cell] {
            seen[cell] = true;
            nunique[bucket] += 1;
        }
    }

    if sort {
        ordering.sort_unstable();
    }
    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for &key in &ordering {
        let bucket = (i128::from(key) - i128::from(min_key)) as usize;
        out_index.push(IndexLabel::Int64(key));
        out_values.push(Scalar::Int64(nunique[bucket]));
    }
    Some((out_index, out_values))
}

/// Dense `Median` for bounded all-valid `Int64` keys and numeric values via a
/// CSR group layout: count non-missing per group, prefix-sum to offsets,
/// scatter every non-missing `to_f64()` value into one flat array grouped by
/// group, then sort each group's contiguous slice and take the middle. Replaces
/// the generic path's per-row hashing + per-group `Vec<Scalar>` + per-group
/// `collect_finite` `Vec<f64>` with two scans and a single flat allocation.
///
/// Bit-identical to `nanmedian`: `collect_finite` is the in-order non-missing
/// `to_f64()` values, the per-group slice holds exactly those values, and
/// `sort_by(partial_cmp)` yields the same ordering — so `nums[mid]` (odd) and
/// `(nums[mid-1]+nums[mid])/2` (even) match element-for-element. Returns `None`
/// (caller uses the generic path) for non-`Int64` keys, a wide key span, or any
/// `Timedelta64`/non-`to_f64` value (which `nanmedian` routes elsewhere).
fn try_groupby_median_dense_int64(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    let (min_key, max_key, saw_int_key) = dense_int64_range(keys, dropna)?;
    if !saw_int_key {
        return Some((Vec::new(), Vec::new()));
    }
    let key_span = i128::from(max_key) - i128::from(min_key) + 1;
    if key_span <= 0 || key_span > DENSE_INT_KEY_RANGE_LIMIT {
        return None;
    }
    let key_span = key_span as usize;

    // Pass 1: per-bucket non-missing count + first-seen group order.
    let mut count = vec![0_usize; key_span];
    let mut first_seen = vec![false; key_span];
    let mut ordering = Vec::<i64>::new();
    for (key, value) in keys.iter().zip(values.iter()) {
        let key = match key {
            Scalar::Int64(v) => *v,
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        };
        let bucket = (i128::from(key) - i128::from(min_key)) as usize;
        if !first_seen[bucket] {
            first_seen[bucket] = true;
            ordering.push(key);
        }
        if value.is_missing() {
            continue;
        }
        // nanmedian has a separate Timedelta64 accumulator; leave those (and any
        // non-f64-coercible value) to the generic path.
        if matches!(value, Scalar::Timedelta64(_)) {
            return None;
        }
        count[bucket] += 1;
    }

    // Exclusive prefix-sum -> per-bucket offset into the flat values array.
    let mut offsets = vec![0_usize; key_span + 1];
    for b in 0..key_span {
        offsets[b + 1] = offsets[b] + count[b];
    }
    let total = offsets[key_span];

    // Pass 2: scatter non-missing f64 values into the flat array, grouped.
    let mut flat = vec![0.0_f64; total];
    let mut cursor = offsets.clone();
    for (key, value) in keys.iter().zip(values.iter()) {
        let key = match key {
            Scalar::Int64(v) => *v,
            Scalar::Null(_) if dropna => continue,
            _ => return None,
        };
        if value.is_missing() {
            continue;
        }
        let x = value.to_f64().ok()?;
        let bucket = (i128::from(key) - i128::from(min_key)) as usize;
        flat[cursor[bucket]] = x;
        cursor[bucket] += 1;
    }

    if sort {
        ordering.sort_unstable();
    }
    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for &key in &ordering {
        let bucket = (i128::from(key) - i128::from(min_key)) as usize;
        out_index.push(IndexLabel::Int64(key));
        let slice = &mut flat[offsets[bucket]..offsets[bucket] + count[bucket]];
        let agg = if slice.is_empty() {
            Scalar::Null(NullKind::NaN)
        } else {
            let cmp = |a: &f64, b: &f64| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
            let mid = slice.len() / 2;
            // O(n) median via select_nth instead of a full O(n log n) sort.
            // Bit-identical to the sort path when the slice is NaN-free: with a
            // finite-float total order, `select_nth_unstable_by(mid)` places the
            // exact mid-th order statistic at `slice[mid]` and partitions so
            // every element in `slice[..mid]` is `<= slice[mid]`. For even
            // lengths the low-middle order statistic (sorted `slice[mid-1]`) is
            // therefore the maximum of `slice[..mid]`, and `(lo + hi) / 2.0`
            // uses the identical operands in the identical order. NaN breaks the
            // total order (`partial_cmp` -> `Equal` makes NaN compare equal to
            // everything), so any NaN-bearing group falls back to the original
            // full sort, which is left untouched. A `-0.0` is likewise routed to
            // the sort fallback: `-0.0` and `+0.0` compare `Equal`, so the
            // even-case max-of-left could pick a `+0.0` where the unstable sort
            // left a `-0.0` at `slice[mid-1]`, and `(-0.0 + hi)` vs `(+0.0 + hi)`
            // can differ in sign when `hi` is also a zero. Gating only on the
            // (rare) negative zero keeps the fast path for all-positive-zero
            // data, where `f64::max` and the sort agree bit-for-bit.
            if slice
                .iter()
                .any(|x| x.is_nan() || (*x == 0.0 && x.is_sign_negative()))
            {
                slice.sort_by(cmp);
                if slice.len().is_multiple_of(2) {
                    Scalar::Float64((slice[mid - 1] + slice[mid]) / 2.0)
                } else {
                    Scalar::Float64(slice[mid])
                }
            } else if slice.len().is_multiple_of(2) {
                let (lo_part, &mut hi, _) = slice.select_nth_unstable_by(mid, cmp);
                let lo = lo_part.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                Scalar::Float64((lo + hi) / 2.0)
            } else {
                let (_, &mut median, _) = slice.select_nth_unstable_by(mid, cmp);
                Scalar::Float64(median)
            }
        };
        out_values.push(agg);
    }
    Some((out_index, out_values))
}

/// Counter-only generic path for `groupby.count()` and `groupby.size()`.
///
/// The full generic aggregation fallback stores every non-missing group value
/// in a per-group `Vec<Scalar>` before calling `nancount` or reading the total
/// row count. Count/size do not need those values: they only need the same
/// group-key map, first-source index, non-missing counter, and total counter.
///
/// Bit-identical to the value-vector path because it uses the same
/// `GroupKeyRef` equality, the same first-seen ordering vector, the same
/// `compare_group_labels` sort, and the same label reconstruction from the
/// first source row. The only removed work is cloning unused values.
fn try_groupby_count_size_counter(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(func, AggFunc::Count | AggFunc::Size) {
        return None;
    }

    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, (usize, i64, i64)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, 0, 0)
        });

        if !value.is_missing() {
            entry.1 += 1;
        }
        entry.2 += 1;
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, non_missing, total) = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[*source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(Scalar::Int64(if matches!(func, AggFunc::Count) {
            *non_missing
        } else {
            *total
        }));
    }

    Some((out_index, out_values))
}

/// Numeric generic-key path for `groupby.mean()`.
///
/// The generic fallback hashes the same keys, clones every non-missing value
/// into a per-group `Vec<Scalar>`, then `nanmean` scans each group. Mean only
/// needs the same group map plus a streaming `to_f64()` sum and count. This
/// helper deliberately accepts only values that `nanmean` would reduce through
/// its numeric `to_f64()` fold; Timedelta64 and other non-numeric groups fall
/// back to the existing vector path so dtype-preserving pandas semantics stay
/// untouched.
fn try_groupby_mean_numeric_counter(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, (usize, f64, usize)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, 0.0, 0)
        });

        if value.is_missing() {
            continue;
        }
        let value = value.to_f64().ok()?;
        entry.1 += value;
        entry.2 += 1;
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, sum, count) = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[*source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(if *count == 0 {
            Scalar::Null(NullKind::NaN)
        } else {
            Scalar::Float64(*sum / *count as f64)
        });
    }

    Some((out_index, out_values))
}

#[derive(Debug, Clone)]
struct VarStdScalarAccumulator {
    source_idx: usize,
    sum: f64,
    count: usize,
    sum_sq: f64,
}

/// Numeric generic-key path for `groupby.var()` and `groupby.std()`.
///
/// The generic fallback hashes the same keys, clones every non-missing value
/// into a per-group `Vec<Scalar>`, then `nanvar`/`nanstd` performs a two-pass
/// numeric scan. This helper keeps the same group map and performs those two
/// passes directly over the input rows, avoiding per-group value vectors while
/// preserving the ddof=1 NaN boundary and output ordering. Timedelta and
/// non-numeric values fall back to the existing vector path so dtype-specific
/// pandas compatibility stays untouched.
fn try_groupby_var_std_numeric_counter(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(func, AggFunc::Var | AggFunc::Std) {
        return None;
    }

    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, VarStdScalarAccumulator>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            VarStdScalarAccumulator {
                source_idx: pos,
                sum: 0.0,
                count: 0,
                sum_sq: 0.0,
            }
        });

        if value.is_missing() {
            continue;
        }
        if matches!(value, Scalar::Timedelta64(_)) {
            return None;
        }
        let value = value.to_f64().ok()?;
        entry.sum += value;
        entry.count += 1;
    }

    for (key, value) in keys.iter().zip(values.iter()) {
        if dropna && key.is_missing() {
            continue;
        }
        if value.is_missing() {
            continue;
        }
        if matches!(value, Scalar::Timedelta64(_)) {
            return None;
        }
        let value = value.to_f64().ok()?;
        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups
            .get_mut(&key_id)
            .expect("second pass references only first-pass groups");
        let mean = entry.sum / entry.count as f64;
        entry.sum_sq += (value - mean).powi(2);
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .source_idx
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let group = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[group.source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(if group.count <= 1 {
            Scalar::Null(NullKind::NaN)
        } else {
            let var = group.sum_sq / (group.count - 1) as f64;
            Scalar::Float64(if matches!(func, AggFunc::Std) {
                var.sqrt()
            } else {
                var
            })
        });
    }

    Some((out_index, out_values))
}

fn scalar_group_label(label: &Scalar) -> IndexLabel {
    match label {
        Scalar::Int64(v) => IndexLabel::Int64(*v),
        Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
        Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
        Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
        Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
        Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
        Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
        Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
        Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
        Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
    }
}

fn numeric_median_scalar(values: &mut [f64]) -> Scalar {
    if values.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }

    let cmp = |a: &f64, b: &f64| a.partial_cmp(b).unwrap_or(Ordering::Equal);
    let mid = values.len() / 2;
    if values
        .iter()
        .any(|x| x.is_nan() || (*x == 0.0 && x.is_sign_negative()))
    {
        values.sort_by(cmp);
        if values.len().is_multiple_of(2) {
            Scalar::Float64((values[mid - 1] + values[mid]) / 2.0)
        } else {
            Scalar::Float64(values[mid])
        }
    } else if values.len().is_multiple_of(2) {
        let (lo_part, &mut hi, _) = values.select_nth_unstable_by(mid, cmp);
        let lo = lo_part.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Scalar::Float64((lo + hi) / 2.0)
    } else {
        let (_, &mut median, _) = values.select_nth_unstable_by(mid, cmp);
        Scalar::Float64(median)
    }
}

/// Numeric generic-key path for `groupby.median()`.
///
/// The generic fallback hashes the same keys, clones every non-missing value
/// into a per-group `Vec<Scalar>`, then `nanmedian` allocates a second
/// `Vec<f64>` via `collect_finite`. Median still needs one sortable vector per
/// group, but for numeric values it can store `f64` directly and reuse the same
/// order-statistic selection as the dense Int64-key path. Timedelta and
/// non-numeric values fall back to the existing vector path so dtype-specific
/// pandas compatibility stays untouched.
fn try_groupby_median_numeric_vectors(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, (usize, Vec<f64>)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, Vec::new())
        });

        if value.is_missing() {
            continue;
        }
        if matches!(value, Scalar::Timedelta64(_)) {
            return None;
        }
        entry.1.push(value.to_f64().ok()?);
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, values) = groups
            .get_mut(key)
            .expect("ordering references only inserted keys");
        out_index.push(scalar_group_label(&keys[*source_idx]));
        out_values.push(numeric_median_scalar(values));
    }

    Some((out_index, out_values))
}

#[derive(Hash, PartialEq, Eq)]
enum NuniqueValueKey<'a> {
    Bool(bool),
    Int64(i64),
    FloatBits(u64),
    Utf8(&'a str),
    Timedelta64(i64),
    Datetime64(i64),
    Period(i64, PeriodFreq),
    Interval(u64, u64, IntervalClosed),
}

fn nunique_value_key(value: &Scalar) -> Option<NuniqueValueKey<'_>> {
    if value.is_missing() {
        return None;
    }
    Some(match value {
        Scalar::Bool(v) => NuniqueValueKey::Bool(*v),
        Scalar::Int64(v) => NuniqueValueKey::Int64(*v),
        Scalar::Float64(v) => {
            let normalized = if *v == 0.0 { 0.0 } else { *v };
            NuniqueValueKey::FloatBits(normalized.to_bits())
        }
        Scalar::Utf8(v) => NuniqueValueKey::Utf8(v.as_str()),
        Scalar::Timedelta64(v) => NuniqueValueKey::Timedelta64(*v),
        Scalar::Datetime64(v) => NuniqueValueKey::Datetime64(*v),
        Scalar::Period(v) => NuniqueValueKey::Period(v.ordinal, v.freq),
        Scalar::Interval(v) => NuniqueValueKey::Interval(
            if v.left == 0.0 { 0.0 } else { v.left }.to_bits(),
            if v.right == 0.0 { 0.0 } else { v.right }.to_bits(),
            v.closed,
        ),
        Scalar::Null(_) => return None,
    })
}

struct NuniqueScalarAccumulator<'a> {
    source_idx: usize,
    seen: FxHashSet<NuniqueValueKey<'a>>,
}

/// Generic string/object-key path for `groupby.nunique()`.
///
/// The fallback hashes the same groups, clones every non-missing value into a
/// per-group `Vec<Scalar>`, and only then calls `nannunique`. Distinct-counting
/// needs no value order and no owned scalars, so this stores the exact borrowed
/// bucket keys that `nannunique` would derive and preserves the same missing
/// skip, `-0.0`/`+0.0` normalization, and dtype-specific equality.
fn try_groupby_nunique_borrowed_sets(
    keys: &[Scalar],
    values: &[Scalar],
    dropna: bool,
    sort: bool,
) -> (Vec<IndexLabel>, Vec<Scalar>) {
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, NuniqueScalarAccumulator<'_>>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            NuniqueScalarAccumulator {
                source_idx: pos,
                seen: FxHashSet::default(),
            }
        });

        if let Some(value_key) = nunique_value_key(value) {
            entry.seen.insert(value_key);
        }
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .source_idx
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let group = groups
            .get(key)
            .expect("ordering references only inserted keys");
        out_index.push(scalar_group_label(&keys[group.source_idx]));
        out_values.push(Scalar::Int64(group.seen.len() as i64));
    }

    (out_index, out_values)
}

fn update_min_max_scalar_slot(
    slot: &mut Option<Scalar>,
    invalid: &mut bool,
    value: &Scalar,
    take_min: bool,
) {
    if value.is_missing() || *invalid {
        return;
    }

    let Some(current) = slot else {
        *slot = Some(value.clone());
        return;
    };

    let replace = match (&*current, value) {
        (Scalar::Int64(a), Scalar::Int64(b)) => {
            if take_min {
                b < a
            } else {
                b > a
            }
        }
        (Scalar::Float64(a), Scalar::Float64(b)) => {
            if take_min {
                b < a
            } else {
                b > a
            }
        }
        (Scalar::Utf8(a), Scalar::Utf8(b)) => {
            if take_min {
                b < a
            } else {
                b > a
            }
        }
        (Scalar::Bool(a), Scalar::Bool(b)) => {
            if take_min {
                b < a
            } else {
                b > a
            }
        }
        (Scalar::Timedelta64(a), Scalar::Timedelta64(b)) => {
            if take_min {
                b < a
            } else {
                b > a
            }
        }
        (a, b) => match (a.to_f64(), b.to_f64()) {
            (Ok(af), Ok(bf)) => {
                if take_min {
                    bf < af
                } else {
                    bf > af
                }
            }
            _ => {
                *invalid = true;
                false
            }
        },
    };

    if replace {
        *current = value.clone();
    }
}

/// Generic string/object-key path for `groupby.min()` and `groupby.max()`.
///
/// The generic fallback hashes the same keys, clones every non-missing value
/// into a per-group `Vec<Scalar>`, then calls `nanmin`/`nanmax`. Min/max only
/// need the current extremum scalar plus the same "incomparable pair => NaN"
/// witness, so this keeps a single slot per group and mirrors
/// `fp_types::nanmin`/`nanmax` comparison order exactly.
fn try_groupby_min_max_scalar_slot(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(func, AggFunc::Min | AggFunc::Max) {
        return None;
    }

    let take_min = matches!(func, AggFunc::Min);
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, (usize, Option<Scalar>, bool)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, None, false)
        });

        update_min_max_scalar_slot(&mut entry.1, &mut entry.2, value, take_min);
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, slot, invalid) = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[*source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(if *invalid {
            Scalar::Null(NullKind::NaN)
        } else {
            slot.clone().unwrap_or(Scalar::Null(NullKind::NaN))
        });
    }

    Some((out_index, out_values))
}

/// Generic string/object-key path for `groupby.first()` and `groupby.last()`.
///
/// The generic fallback stores every non-missing value per group, then selects
/// the first or last slot. This path keeps only that selected scalar while
/// preserving the same group admission, ordering, label, and all-missing
/// `Null(NaN)` behavior.
fn try_groupby_first_last_scalar_slot(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(func, AggFunc::First | AggFunc::Last) {
        return None;
    }

    let take_first = matches!(func, AggFunc::First);
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, (usize, Option<Scalar>)>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, None)
        });

        if value.is_missing() {
            continue;
        }
        if !take_first || entry.1.is_none() {
            entry.1 = Some(value.clone());
        }
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let (source_idx, slot) = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[*source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(slot.clone().unwrap_or(Scalar::Null(NullKind::NaN)));
    }

    Some((out_index, out_values))
}

#[derive(Debug, Clone)]
struct SumProdScalarAccumulator {
    source_idx: usize,
    sum: i128,
    prod_i128: Option<i128>,
    prod_f64: f64,
}

#[derive(Debug, Clone)]
struct SumProdFloatAccumulator {
    source_idx: usize,
    sum: f64,
    prod: f64,
}

/// Generic string/object-key path for integer/bool `groupby.sum()` and
/// `groupby.prod()`.
///
/// The fallback already has dtype-preserving branches for Int64/Bool values,
/// but it reaches them only after building a per-group `Vec<Scalar>`. This
/// helper streams the same i128 sum/product state per group and keeps the same
/// f64 product fallback witness for integer overflow.
fn try_groupby_sum_prod_integer_counter(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    value_dtype: DType,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(func, AggFunc::Sum | AggFunc::Prod)
        || !matches!(value_dtype, DType::Int64 | DType::Bool)
    {
        return None;
    }

    let take_sum = matches!(func, AggFunc::Sum);
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, SumProdScalarAccumulator>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            SumProdScalarAccumulator {
                source_idx: pos,
                sum: 0,
                prod_i128: Some(1),
                prod_f64: 1.0,
            }
        });

        let value_i128 = match value {
            Scalar::Int64(x) => i128::from(*x),
            Scalar::Bool(b) => i128::from(u8::from(*b)),
            _ => continue,
        };
        if take_sum {
            entry.sum += value_i128;
        } else {
            entry.prod_i128 = entry
                .prod_i128
                .and_then(|prod| prod.checked_mul(value_i128));
            entry.prod_f64 *= value_i128 as f64;
        }
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .source_idx
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let group = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[group.source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(if take_sum {
            match i64::try_from(group.sum) {
                Ok(sum) => Scalar::Int64(sum),
                Err(_) => Scalar::Float64(group.sum as f64),
            }
        } else {
            match group.prod_i128.and_then(|prod| i64::try_from(prod).ok()) {
                Some(prod) => Scalar::Int64(prod),
                None => Scalar::Float64(group.prod_f64),
            }
        });
    }

    Some((out_index, out_values))
}

/// Generic string/object-key path for Float64 `groupby.sum()` and
/// `groupby.prod()`.
///
/// The scalar fallback clones every non-missing Float64 into a per-group
/// `Vec<Scalar>` before calling `nansum`/`nanprod`. Those reducers are already
/// streaming f64 folds, so keep only the per-group accumulator state here.
fn try_groupby_sum_prod_float_counter(
    keys: &[Scalar],
    values: &[Scalar],
    func: AggFunc,
    value_dtype: DType,
    dropna: bool,
    sort: bool,
) -> Option<(Vec<IndexLabel>, Vec<Scalar>)> {
    if !matches!(func, AggFunc::Sum | AggFunc::Prod) || !matches!(value_dtype, DType::Float64) {
        return None;
    }

    let take_sum = matches!(func, AggFunc::Sum);
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, SumProdFloatAccumulator>::default();

    for (pos, (key, value)) in keys.iter().zip(values.iter()).enumerate() {
        if dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            SumProdFloatAccumulator {
                source_idx: pos,
                sum: 0.0,
                prod: 1.0,
            }
        });

        if value.is_missing() {
            continue;
        }
        let Scalar::Float64(x) = value else {
            return None;
        };
        if take_sum {
            entry.sum += *x;
        } else {
            entry.prod *= *x;
        }
    }

    if sort {
        sort_group_ordering_by(keys, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .source_idx
        });
    }

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    for key in &ordering {
        let group = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &keys[group.source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });
        out_values.push(Scalar::Float64(if take_sum {
            group.sum
        } else {
            group.prod
        }));
    }

    Some((out_index, out_values))
}

pub fn groupby_agg(
    keys: &Series,
    values: &Series,
    func: AggFunc,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    // Alignment: if indexes differ, align to union.
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

    let input_rows = aligned_storage
        .as_ref()
        .map_or_else(|| keys.len(), |(aligned_keys, _)| aligned_keys.len());
    // Record an admission decision for policy observability without altering
    // the current groupby output behavior.
    let _ = policy.decide_join_admission(input_rows, ledger);

    // Identity-aligned, all-valid Int64 inputs already expose raw buffers.
    // Mean's dense fold is exactly representable over those slices, so run it
    // before `values()` materializes two Scalar arrays. Wide key ranges keep
    // the existing Scalar path, as do aligned, nullable, and non-Int64 inputs.
    if aligned_storage.is_none()
        && matches!(func, AggFunc::Mean)
        && let (Some(raw_keys), Some(raw_values)) =
            (keys.column().as_i64_slice(), values.column().as_i64_slice())
        && let Some((out_index, out_values)) =
            try_groupby_mean_dense_int64_slices(raw_keys, raw_values, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new("mean", Index::new(out_index), out_column)?);
    }

    let (key_vals, val_vals): (&[Scalar], &[Scalar]) =
        if let Some((ref ak, ref av)) = aligned_storage {
            (ak.values(), av.values())
        } else {
            (keys.values(), values.values())
        };
    let value_dtype = values.column().dtype();

    let agg_name = match func {
        AggFunc::Sum => "sum",
        AggFunc::Mean => "mean",
        AggFunc::Count => "count",
        AggFunc::Min => "min",
        AggFunc::Max => "max",
        AggFunc::First => "first",
        AggFunc::Last => "last",
        AggFunc::Std => "std",
        AggFunc::Var => "var",
        AggFunc::Median => "median",
        AggFunc::Nunique => "nunique",
        AggFunc::Prod => "prod",
        AggFunc::Size => "size",
    };

    // Dense direct-address streaming fast path for bounded Int64 keys (folds
    // each group's values without hashing or collecting a per-group Vec).
    if let Some((out_index, out_values)) =
        try_groupby_agg_dense_int64(key_vals, val_vals, func, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key count and size only need counters, not cloned
    // per-group values. Keep this after the dense Int64 path so existing typed
    // direct-address count/size behavior stays unchanged.
    if let Some((out_index, out_values)) =
        try_groupby_count_size_counter(key_vals, val_vals, func, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key mean only needs a streaming sum and count for
    // numeric values. Non-numeric or Timedelta values fall back to the vector
    // path below, preserving dtype-specific pandas compatibility.
    if matches!(func, AggFunc::Mean)
        && let Some((out_index, out_values)) =
            try_groupby_mean_numeric_counter(key_vals, val_vals, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key var/std use the same two-pass numeric formula
    // as nanvar/nanstd, but do not need a cloned Vec<Scalar> per group.
    if matches!(func, AggFunc::Var | AggFunc::Std)
        && let Some((out_index, out_values)) = try_groupby_var_std_numeric_counter(
            key_vals,
            val_vals,
            func,
            options.dropna,
            options.sort,
        )
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Bounded Int64 keys should reach the direct-address CSR median before the
    // generic numeric-key hash path. Both paths use the same numeric median
    // selection, but the dense path avoids per-row hashing and per-group Vecs.
    if matches!(func, AggFunc::Median)
        && let Some((out_index, out_values)) =
            try_groupby_median_dense_int64(key_vals, val_vals, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key median needs sortable values, but it can store
    // numeric f64 values directly instead of cloning Scalar values and then
    // allocating a second collect_finite vector inside nanmedian.
    if matches!(func, AggFunc::Median)
        && let Some((out_index, out_values)) =
            try_groupby_median_numeric_vectors(key_vals, val_vals, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key min/max only need one scalar slot per group,
    // not a cloned value Vec. Keep this after dense Int64 so the typed direct
    // path remains the fastest route for small integer domains.
    if matches!(func, AggFunc::Min | AggFunc::Max)
        && let Some((out_index, out_values)) =
            try_groupby_min_max_scalar_slot(key_vals, val_vals, func, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key first/last only need one selected scalar per
    // group, not a cloned value Vec. The dense Int64 route above remains first
    // chance for small integer key domains.
    if matches!(func, AggFunc::First | AggFunc::Last)
        && let Some((out_index, out_values)) = try_groupby_first_last_scalar_slot(
            key_vals,
            val_vals,
            func,
            options.dropna,
            options.sort,
        )
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key integer sum/prod only need streaming
    // accumulators, not a cloned value Vec. Float64 has the matching fast path
    // below; timedelta, string, and mixed-object semantics stay on fallback.
    if matches!(func, AggFunc::Sum | AggFunc::Prod)
        && let Some((out_index, out_values)) = try_groupby_sum_prod_integer_counter(
            key_vals,
            val_vals,
            func,
            value_dtype,
            options.dropna,
            options.sort,
        )
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key Float64 sum/prod use the same left-to-right f64
    // fold as nansum/nanprod, but avoid cloning per-group Scalar vectors.
    if matches!(func, AggFunc::Sum | AggFunc::Prod)
        && let Some((out_index, out_values)) = try_groupby_sum_prod_float_counter(
            key_vals,
            val_vals,
            func,
            value_dtype,
            options.dropna,
            options.sort,
        )
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Dense 2-D seen-bitset distinct count for bounded Int64 keys+values.
    if matches!(func, AggFunc::Nunique)
        && let Some((out_index, out_values)) =
            try_groupby_nunique_dense_int64(key_vals, val_vals, options.dropna, options.sort)
    {
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Generic string/object-key nunique only needs per-group distinct buckets,
    // not cloned value vectors. Keep this after the dense Int64 seen-bitset
    // route so bounded integer key/value cases retain the direct-address path.
    if matches!(func, AggFunc::Nunique) {
        let (out_index, out_values) =
            try_groupby_nunique_borrowed_sets(key_vals, val_vals, options.dropna, options.sort);
        let out_column = Column::from_values(out_values)?;
        return Ok(Series::new(agg_name, Index::new(out_index), out_column)?);
    }

    // Collect groups: key_ref -> (source_idx, non-null values, total count).
    let mut ordering = Vec::<GroupKeyRef<'_>>::new();
    let mut groups = FxHashMap::<GroupKeyRef<'_>, (usize, Vec<Scalar>, usize)>::default();

    for (pos, (key, value)) in key_vals.iter().zip(val_vals.iter()).enumerate() {
        if options.dropna && key.is_missing() {
            continue;
        }

        let key_id = GroupKeyRef::from_scalar(key);
        let entry = groups.entry(key_id.clone()).or_insert_with(|| {
            ordering.push(key_id.clone());
            (pos, Vec::new(), 0)
        });

        entry.2 += 1; // total count (including nulls)
        if !value.is_missing() {
            entry.1.push(value.clone());
        }
    }

    if options.sort {
        sort_group_ordering_by(key_vals, &mut ordering, |key| {
            groups
                .get(key)
                .expect("ordering references only inserted keys")
                .0
        });
    }

    // Apply aggregation function to each group.

    let mut out_index = Vec::with_capacity(ordering.len());
    let mut out_values = Vec::with_capacity(ordering.len());
    // Per br-frankenpandas-l75ms: keep groupby_agg(Sum) consistent with the
    // dedicated groupby_sum — pandas preserves the integer dtype for sum.

    for key in &ordering {
        let (source_idx, vals, total_count) = groups
            .get(key)
            .expect("ordering references only inserted keys");
        let label = &key_vals[*source_idx];
        out_index.push(match label {
            Scalar::Int64(v) => IndexLabel::Int64(*v),
            Scalar::Utf8(v) => IndexLabel::Utf8(v.clone()),
            Scalar::Bool(v) => IndexLabel::Utf8(if *v { "True" } else { "False" }.to_string()),
            // Typed null group label (br-frankenpandas-8m6ay): collapsed nan
            // group, NaT preserved — see the sum/mean/concat/count maps above.
            Scalar::Null(NullKind::NaT) => IndexLabel::Null(NullKind::NaT),
            Scalar::Null(NullKind::NaN | NullKind::Null) => IndexLabel::Null(NullKind::NaN),
            Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
            Scalar::Timedelta64(v) => IndexLabel::Utf8(Timedelta::format(*v)),
            Scalar::Datetime64(v) => IndexLabel::Datetime64(*v),
            Scalar::Period(v) => IndexLabel::Utf8(v.calendar_string()),
            Scalar::Interval(iv) => IndexLabel::Utf8(format!("{iv}")),
        });

        let agg_value = match func {
            AggFunc::Sum if matches!(value_dtype, DType::Int64 | DType::Bool) => {
                let mut total = 0_i128;
                for v in vals {
                    match v {
                        Scalar::Int64(x) => total += i128::from(*x),
                        Scalar::Bool(b) => total += i128::from(*b),
                        _ => {}
                    }
                }
                match i64::try_from(total) {
                    Ok(x) => Scalar::Int64(x),
                    Err(_) => Scalar::Float64(total as f64),
                }
            }
            AggFunc::Sum => fp_types::nansum(vals),
            AggFunc::Mean => fp_types::nanmean(vals),
            AggFunc::Count => fp_types::nancount(vals),
            // pandas groupby.min()/.max() preserve the source column dtype
            // — Int64 stays Int64 (and Timedelta64 stays Timedelta64 via
            // fp_types::nanmin/nanmax). The earlier promotion to Float64
            // here diverged from pandas, mirroring the same regression that
            // br-frankenpandas-764ys fixed for first/last.
            AggFunc::Min => fp_types::nanmin(vals),
            AggFunc::Max => fp_types::nanmax(vals),
            // Per br-frankenpandas-764ys: pandas groupby.first()/.last()
            // preserve the source column dtype — Int64 stays Int64. The
            // previous promotion to Float64 diverged from pandas (which
            // only auto-promotes when the column already contains NaN).
            // If the source column has mixed Int64+Null, our column model
            // upcasts to Float64 at construction time, so `vals[0]` is
            // already the correct dtype here.
            AggFunc::First => {
                if vals.is_empty() {
                    Scalar::Null(NullKind::NaN)
                } else {
                    vals[0].clone()
                }
            }
            AggFunc::Last => {
                if vals.is_empty() {
                    Scalar::Null(NullKind::NaN)
                } else {
                    vals[vals.len() - 1].clone()
                }
            }
            AggFunc::Var => fp_types::nanvar(vals, 1),
            AggFunc::Std => fp_types::nanstd(vals, 1),
            AggFunc::Median => fp_types::nanmedian(vals),
            AggFunc::Nunique => fp_types::nannunique(vals),
            // pandas groupby.prod() preserves Int64 for integer/bool input,
            // mirroring Sum (the earlier Float64-only path diverged from pandas).
            // Accumulate an i128 product and keep Int64 when it fits; fall back to
            // the Float64 nanprod only on i64 overflow. br-frankenpandas-rl25i.
            AggFunc::Prod if matches!(value_dtype, DType::Int64 | DType::Bool) => {
                let mut total: Option<i128> = Some(1);
                for v in vals {
                    let x = match v {
                        Scalar::Int64(x) => i128::from(*x),
                        Scalar::Bool(b) => i128::from(*b),
                        _ => continue,
                    };
                    total = total.and_then(|t| t.checked_mul(x));
                }
                match total.and_then(|t| i64::try_from(t).ok()) {
                    Some(x) => Scalar::Int64(x),
                    None => fp_types::nanprod(vals),
                }
            }
            AggFunc::Prod => fp_types::nanprod(vals),
            AggFunc::Size => Scalar::Int64(*total_count as i64),
        };

        out_values.push(agg_value);
    }

    let out_column = Column::from_values(out_values)?;
    Ok(Series::new(agg_name, Index::new(out_index), out_column)?)
}

/// Convenience: `groupby_mean`.
pub fn groupby_mean(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Mean, options, policy, ledger)
}

/// Convenience: `groupby_count` (count of non-null values per group).
pub fn groupby_count(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Count, options, policy, ledger)
}

/// Convenience: `groupby_min`.
pub fn groupby_min(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Min, options, policy, ledger)
}

/// Convenience: `groupby_max`.
pub fn groupby_max(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Max, options, policy, ledger)
}

/// Convenience: `groupby_first` (first non-null value per group).
pub fn groupby_first(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::First, options, policy, ledger)
}

/// Convenience: `groupby_last` (last non-null value per group).
pub fn groupby_last(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Last, options, policy, ledger)
}

/// Convenience: `groupby_std` (sample standard deviation per group, ddof=1).
pub fn groupby_std(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Std, options, policy, ledger)
}

/// Convenience: `groupby_var` (sample variance per group, ddof=1).
pub fn groupby_var(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Var, options, policy, ledger)
}

/// Convenience: `groupby_median` (median per group).
pub fn groupby_median(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Median, options, policy, ledger)
}

/// Convenience: `groupby_nunique` (count of unique non-null values per group).
pub fn groupby_nunique(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Nunique, options, policy, ledger)
}

/// Convenience: `groupby_prod` (product of non-null values per group).
pub fn groupby_prod(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Prod, options, policy, ledger)
}

/// Convenience: `groupby_size` (total count per group, including nulls).
pub fn groupby_size(
    keys: &Series,
    values: &Series,
    options: GroupByOptions,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, GroupByError> {
    groupby_agg(keys, values, AggFunc::Size, options, policy, ledger)
}

// ---------------------------------------------------------------------------
// AG-12: Sketching / Streaming Aggregation Data Structures
// ---------------------------------------------------------------------------

/// Hash function for sketching. Uses SplitMix64 finalizer for good avalanche.
fn sketch_hash(value: u64, seed: u64) -> u64 {
    let mut h = value.wrapping_add(seed);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// Hash a Scalar value to u64 for sketching purposes.
fn scalar_to_hash_bits(value: &Scalar) -> u64 {
    match value {
        Scalar::Int64(v) => *v as u64,
        Scalar::Float64(v) => {
            if v.is_nan() {
                return 0xDEAD_BEEF_CAFE_BABE;
            }
            v.to_bits()
        }
        Scalar::Bool(v) => u64::from(*v),
        Scalar::Utf8(s) => {
            let mut h = 0xcbf2_9ce4_8422_2325_u64;
            for b in s.bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            h
        }
        Scalar::Null(_) => 0,
        Scalar::Timedelta64(v) => *v as u64,
        Scalar::Datetime64(v) => *v as u64,
        Scalar::Period(v) => v.ordinal as u64,
        Scalar::Interval(iv) => iv.left.to_bits() ^ iv.right.to_bits(),
    }
}

// --- HyperLogLog ---

/// HyperLogLog sketch for approximate distinct-count estimation.
///
/// Uses 2^p registers (p=14 → 16384 registers → 16KB).
/// Standard error: 1.04 / sqrt(m) ≈ 0.81% for p=14.
pub struct HyperLogLog {
    registers: Vec<u8>,
    p: u32,
}

impl HyperLogLog {
    /// Create a new HLL sketch with precision `p` (6..=18).
    /// Memory usage: 2^p bytes.
    #[must_use]
    pub fn new(p: u32) -> Self {
        let p = p.clamp(6, 18);
        let m = 1_usize << p;
        Self {
            registers: vec![0_u8; m],
            p,
        }
    }

    /// Default precision: p=14 → 16384 registers → 16KB, ~0.81% error.
    #[must_use]
    pub fn default_precision() -> Self {
        Self::new(14)
    }

    /// Insert a pre-hashed value.
    pub fn insert_hash(&mut self, hash: u64) {
        let m = self.registers.len();
        let idx = (hash >> (64 - self.p)) as usize;
        let remaining = hash << self.p;
        // If remaining is 0, leading_zeros is 64. But we only have 64 - p bits.
        // The maximum run of zeros before we hit the 'implicit' 1 is 64 - p.
        // We can place a sentinel 1 at the bit just below the remaining bits:
        let sentinel = if self.p == 0 {
            0
        } else {
            1_u64 << (self.p - 1)
        };
        let rho = (remaining | sentinel).leading_zeros() as u8 + 1;
        if rho > self.registers[idx % m] {
            self.registers[idx % m] = rho;
        }
    }

    /// Insert a Scalar value.
    pub fn insert(&mut self, value: &Scalar) {
        let raw = scalar_to_hash_bits(value);
        let hash = sketch_hash(raw, 0x1234_5678);
        self.insert_hash(hash);
    }

    /// Estimate the number of distinct elements.
    #[must_use]
    pub fn estimate(&self) -> f64 {
        let m = self.registers.len() as f64;

        // Alpha constant for bias correction.
        let alpha = match self.registers.len() {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m),
        };

        let raw_estimate = alpha * m * m
            / self
                .registers
                .iter()
                .map(|&r| 2_f64.powi(i32::from(r)).recip())
                .sum::<f64>();

        // Small range correction (linear counting).
        let zeros = self.registers.iter().filter(|&&r| r == 0).count();
        if raw_estimate <= 2.5 * m && zeros > 0 {
            m * (m / zeros as f64).ln()
        } else {
            raw_estimate
        }
    }

    /// Memory usage in bytes.
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.registers.len()
    }
}

// --- KLL Sketch ---

/// KLL sketch for approximate quantile estimation.
///
/// Maintains sorted compactor levels. When a level exceeds capacity,
/// half its elements are promoted (compacted) to the next level.
pub struct KllSketch {
    compactors: Vec<Vec<f64>>,
    k: usize,
    size: usize,
    compact_count: usize,
}

impl KllSketch {
    /// Create with target capacity `k`. Higher k = more accuracy, more memory.
    /// Error bound: ~1/k.
    #[must_use]
    pub fn new(k: usize) -> Self {
        let k = k.max(8);
        Self {
            compactors: vec![Vec::with_capacity(k * 2)],
            k,
            size: 0,
            compact_count: 0,
        }
    }

    /// Default: k=256 → ~0.4% error.
    #[must_use]
    pub fn default_accuracy() -> Self {
        Self::new(256)
    }

    /// Insert a value into the sketch.
    pub fn insert(&mut self, value: f64) {
        self.compactors[0].push(value);
        self.size += 1;

        // Compact if level 0 exceeds capacity.
        if self.compactors[0].len() >= self.capacity_at_level(0) {
            self.compact(0);
        }
    }

    fn capacity_at_level(&self, _level: usize) -> usize {
        // Uniform capacity across all levels (simplified KLL).
        // Levels are cleared on compaction; only fresh/promoted items remain.
        2 * self.k
    }

    fn compact(&mut self, level: usize) {
        self.compactors[level]
            .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Use compact_count + level to alternate which half gets promoted,
        // avoiding systematic bias across levels and time.
        let offset = (self.compact_count + level) % 2;
        self.compact_count = self.compact_count.wrapping_add(1);

        // Ensure next level exists.
        if level + 1 >= self.compactors.len() {
            self.compactors.push(Vec::with_capacity(self.k * 2));
        }

        // Promote every other element to the next level; discard the rest.
        // The disjoint borrows let us stream directly into the destination,
        // retaining the current level's capacity without a temporary Vec.
        let (through_current, after_current) = self.compactors.split_at_mut(level + 1);
        let current = &mut through_current[level];
        let next = &mut after_current[0];
        next.extend(
            current
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(i, v)| if i % 2 == offset { Some(v) } else { None }),
        );
        current.clear();

        // Recursively compact if next level overflows.
        if self.compactors[level + 1].len() >= self.capacity_at_level(level + 1) {
            self.compact(level + 1);
        }
    }

    /// Estimate the quantile at rank `q` (0.0 = min, 1.0 = max).
    /// Returns `None` if the sketch is empty.
    #[must_use]
    pub fn quantile(&self, q: f64) -> Option<f64> {
        if self.size == 0 {
            return None;
        }

        let q = q.clamp(0.0, 1.0);

        // Collect all items with their weights.
        let mut weighted: Vec<(f64, u64)> = Vec::new();
        for (level, compactor) in self.compactors.iter().enumerate() {
            let weight = 1_u64.checked_shl(level as u32).unwrap_or(u64::MAX);
            for &value in compactor {
                weighted.push((value, weight));
            }
        }

        weighted
            .sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let total_weight: u64 = weighted
            .iter()
            .map(|(_, w)| w)
            .fold(0_u64, |acc, &w| acc.saturating_add(w));
        let target = (q * total_weight as f64).ceil() as u64;
        let target = target.max(1).min(total_weight);

        let mut cumulative = 0_u64;
        for &(value, weight) in &weighted {
            cumulative += weight;
            if cumulative >= target {
                return Some(value);
            }
        }

        weighted.last().map(|&(v, _)| v)
    }

    /// Number of elements inserted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.size
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

// --- Count-Min Sketch ---

/// Count-Min sketch for approximate frequency estimation.
///
/// Uses `depth` independent hash functions and `width` counters per function.
/// For eps=width factor, delta=depth factor:
/// - Overestimates by at most eps*N with probability 1-delta.
/// - Never underestimates.
pub struct CountMinSketch {
    counters: Vec<Vec<u64>>,
    width: usize,
    seeds: Vec<u64>,
}

impl CountMinSketch {
    /// Create with specified width and depth.
    /// Error <= N/width with probability >= 1 - 2^(-depth).
    #[must_use]
    pub fn new(width: usize, depth: usize) -> Self {
        let width = width.max(16);
        let depth = depth.max(2);
        let seeds: Vec<u64> = (0..depth)
            .map(|i| sketch_hash(i as u64, 0xBEEF_CAFE))
            .collect();
        Self {
            counters: vec![vec![0_u64; width]; depth],
            width,
            seeds,
        }
    }

    /// Default: width=1024, depth=5 → error <= N/1024 with prob >= 96.9%.
    #[must_use]
    pub fn default_accuracy() -> Self {
        Self::new(1024, 5)
    }

    /// Increment the count for a Scalar value.
    pub fn insert(&mut self, value: &Scalar) {
        let raw = scalar_to_hash_bits(value);
        for (d, seed) in self.seeds.iter().enumerate() {
            let h = sketch_hash(raw, *seed) as usize % self.width;
            self.counters[d][h] = self.counters[d][h].saturating_add(1);
        }
    }

    /// Estimate the frequency of a Scalar value.
    /// Returns the minimum count across all hash functions (never underestimates).
    #[must_use]
    pub fn estimate(&self, value: &Scalar) -> u64 {
        let raw = scalar_to_hash_bits(value);
        self.seeds
            .iter()
            .enumerate()
            .map(|(d, seed)| {
                let h = sketch_hash(raw, *seed) as usize % self.width;
                self.counters[d][h]
            })
            .min()
            .unwrap_or(0)
    }
}

/// Result type for approximate aggregation methods.
#[derive(Debug, Clone)]
pub struct SketchResult {
    /// The approximate value.
    pub value: f64,
    /// The error bound (absolute or relative depending on method).
    pub error_bound: f64,
    /// Memory used by the sketch in bytes.
    pub memory_bytes: usize,
}

/// Approximate distinct count (nunique) using HyperLogLog.
///
/// Returns a `SketchResult` with the estimated cardinality and error bound.
/// Memory: ~16KB regardless of input size (p=14).
pub fn approx_nunique(values: &[Scalar]) -> SketchResult {
    let mut hll = HyperLogLog::default_precision();
    for v in values {
        if !v.is_missing() {
            hll.insert(v);
        }
    }
    let estimate = hll.estimate();
    let m = hll.registers.len() as f64;
    let std_error = 1.04 / m.sqrt();
    SketchResult {
        value: estimate,
        error_bound: std_error * estimate, // absolute error bound
        memory_bytes: hll.memory_bytes(),
    }
}

/// Approximate quantile estimation using KLL sketch.
///
/// `q` in [0.0, 1.0]: 0.0 = min, 0.5 = median, 1.0 = max.
/// Returns `None` if no valid (non-missing) numeric values exist.
pub fn approx_quantile(values: &[Scalar], q: f64) -> Option<SketchResult> {
    let mut kll = KllSketch::default_accuracy();
    for v in values {
        if let Ok(f) = v.to_f64() {
            kll.insert(f);
        }
    }
    kll.quantile(q).map(|value| SketchResult {
        value,
        error_bound: 1.0 / kll.k as f64, // relative rank error
        memory_bytes: kll.compactors.iter().map(|c| c.len() * 8).sum(),
    })
}

/// Approximate value_counts using Count-Min sketch.
///
/// Returns estimated frequencies for each unique value in the input.
/// Frequencies are guaranteed to never underestimate actual counts.
pub fn approx_value_counts(values: &[Scalar]) -> Vec<(Scalar, u64)> {
    let mut cm = CountMinSketch::default_accuracy();
    let mut seen = Vec::<Scalar>::new();

    for v in values {
        if v.is_missing() {
            continue;
        }
        cm.insert(v);
        if !seen.iter().any(|s| s == v) {
            seen.push(v.clone());
        }
    }

    seen.into_iter()
        .map(|v| {
            let freq = cm.estimate(&v);
            (v, freq)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use fp_types::{NullKind, Scalar};

    use super::{
        GroupByExecutionOptions, GroupByOptions, groupby_nunique, groupby_prod, groupby_size,
        groupby_sum, groupby_sum_with_options, groupby_sum_with_trace, try_groupby_agg_dense_int64,
        try_groupby_mean_dense_int64_slices, try_groupby_median_dense_int64,
        try_groupby_median_numeric_vectors, try_groupby_sum_dense_int64_slices,
        try_groupby_sum_dense_int64_values,
    };

    #[test]
    fn groupby_sum_dense_int64_keys_match_naive_reference_fuzz_xbrt8() {
        use std::collections::BTreeMap;
        // Differential guard for the dense int64-key sum bucket path (d43c1178):
        // int64 keys (negative / null / duplicate / dense span) with Float64
        // values route through try_groupby_sum_dense_int64. The result must equal
        // a naive sorted group-sum reference (default options: dropna=true,
        // sort=true, skipna). Integer-valued floats keep sums order-exact.
        // (br-frankenpandas-xbrt8)
        let mut st: u64 = 0xD1B5_4A32_D192_ED03;
        let mut next = || {
            st = st
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            st >> 33
        };
        for _ in 0..400 {
            let n = (next() % 8 + 1) as usize;
            let raw_keys: Vec<Option<i64>> = (0..n)
                .map(|_| {
                    let r = next() % 7;
                    if r == 6 { None } else { Some(r as i64 - 3) }
                })
                .collect();
            let raw_vals: Vec<Option<f64>> = (0..n)
                .map(|i| {
                    if next() % 5 == 0 {
                        None
                    } else {
                        Some(((i as i64 + 1) * 10) as f64)
                    }
                })
                .collect();

            let keys = Series::from_values(
                "k",
                (0..n).map(|i| (i as i64).into()).collect(),
                raw_keys
                    .iter()
                    .map(|k| k.map_or(Scalar::Null(NullKind::Null), Scalar::Int64))
                    .collect(),
            )
            .unwrap();
            let vals = Series::from_values(
                "v",
                (0..n).map(|i| (i as i64).into()).collect(),
                raw_vals
                    .iter()
                    .map(|v| v.map_or(Scalar::Null(NullKind::NaN), Scalar::Float64))
                    .collect(),
            )
            .unwrap();

            let out = groupby_sum(
                &keys,
                &vals,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut EvidenceLedger::new(),
            )
            .unwrap();

            // Naive reference: drop null keys; a group exists per non-null key
            // (even if all its values are null -> sum 0.0); sum non-null values;
            // sorted ascending by key.
            let mut groups: BTreeMap<i64, f64> = BTreeMap::new();
            for i in 0..n {
                if let Some(k) = raw_keys[i] {
                    let entry = groups.entry(k).or_insert(0.0);
                    if let Some(v) = raw_vals[i] {
                        *entry += v;
                    }
                }
            }
            let exp_idx: Vec<IndexLabel> = groups.keys().map(|&k| IndexLabel::Int64(k)).collect();
            let exp_val: Vec<Scalar> = groups.values().map(|&v| Scalar::Float64(v)).collect();
            assert_eq!(
                out.index().labels(),
                exp_idx.as_slice(),
                "index mismatch for keys={raw_keys:?}"
            );
            assert_eq!(
                out.values(),
                exp_val.as_slice(),
                "values mismatch for keys={raw_keys:?} vals={raw_vals:?}"
            );
        }
    }

    #[test]
    fn groupby_sum_sorts_keys_by_default() {
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

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(out.values(), &[Scalar::Int64(6), Scalar::Int64(4)]);
    }

    #[test]
    fn dense_int64_groupby_matches_handcomputed_oracle_k3zcv() {
        use std::collections::BTreeMap;

        // Oracle differential (br-frankenpandas-k3zcv): the dense Int64-key
        // groupby fast paths (sum 1432b615, count/min/max dense streaming) must
        // equal an INDEPENDENT hand-computed grouping. Deterministic seeded LCG —
        // no rand crate, no mocks.
        let mut state: u64 = 0x1234_5678_9abc_def1;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };

        for iter in 0..1200u32 {
            let n = (next() % 14) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 5) as i64 - 2).collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 21) as i64 - 10).collect();

            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("keys");
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("values");

            // Independent oracle: group rows by key, sorted ascending (pandas
            // default). BTreeMap gives ascending key order.
            let mut groups: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                groups.entry(*k).or_default().push(*v);
            }
            let exp_keys: Vec<IndexLabel> = groups.keys().map(|&k| IndexLabel::Int64(k)).collect();
            let exp_sum: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(vs.iter().sum()))
                .collect();
            let exp_count: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(vs.len() as i64))
                .collect();
            let exp_min: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(*vs.iter().min().unwrap()))
                .collect();
            let exp_max: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(*vs.iter().max().unwrap()))
                .collect();

            let ctx = format!("iter={iter} keys={key_vals:?} vals={val_vals:?}");
            let opts = || GroupByOptions::default();
            let pol = RuntimePolicy::strict();
            let mut led = EvidenceLedger::new();

            let s = groupby_sum(&keys, &values, opts(), &pol, &mut led).expect("sum");
            assert_eq!(s.index().labels(), exp_keys, "sum keys {ctx}");
            assert_eq!(s.values(), exp_sum.as_slice(), "sum vals {ctx}");

            let c = groupby_count(&keys, &values, opts(), &pol, &mut led).expect("count");
            assert_eq!(c.values(), exp_count.as_slice(), "count vals {ctx}");

            let mn = groupby_min(&keys, &values, opts(), &pol, &mut led).expect("min");
            assert_eq!(mn.values(), exp_min.as_slice(), "min vals {ctx}");

            let mx = groupby_max(&keys, &values, opts(), &pol, &mut led).expect("max");
            assert_eq!(mx.values(), exp_max.as_slice(), "max vals {ctx}");
        }
    }

    #[test]
    fn dense_int64_groupby_first_last_matches_handcomputed_oracle_ypgw6() {
        use std::collections::BTreeMap;

        // Oracle differential (br-frankenpandas-ypgw6): groupby_first/last carry
        // ROW-ORDER (positional) semantics distinct from the reductions covered by
        // k3zcv. Assert they equal a hand-computed grouping that preserves row
        // order. Deterministic seeded LCG — no rand crate, no mocks.
        let mut state: u64 = 0x6b1e_55ed_a17a_c0de;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };

        for iter in 0..1200u32 {
            let n = (next() % 14) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 5) as i64 - 2).collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 41) as i64 - 20).collect();

            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("keys");
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("values");

            // Independent oracle: group rows by key (ascending = pandas default),
            // preserving ROW ORDER within each group.
            let mut groups: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                groups.entry(*k).or_default().push(*v);
            }
            let exp_first: Vec<Scalar> = groups.values().map(|vs| Scalar::Int64(vs[0])).collect();
            let exp_last: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(*vs.last().unwrap()))
                .collect();
            let exp_keys: Vec<IndexLabel> = groups.keys().map(|&k| IndexLabel::Int64(k)).collect();

            let ctx = format!("iter={iter} keys={key_vals:?} vals={val_vals:?}");
            let pol = RuntimePolicy::strict();
            let mut led = EvidenceLedger::new();

            let f = groupby_first(&keys, &values, GroupByOptions::default(), &pol, &mut led)
                .expect("first");
            assert_eq!(f.index().labels(), exp_keys, "first keys {ctx}");
            assert_eq!(f.values(), exp_first.as_slice(), "first vals {ctx}");

            let l = groupby_last(&keys, &values, GroupByOptions::default(), &pol, &mut led)
                .expect("last");
            assert_eq!(l.values(), exp_last.as_slice(), "last vals {ctx}");
        }
    }

    #[test]
    fn dense_int64_groupby_nunique_matches_handcomputed_oracle_xnbl7() {
        use std::collections::{BTreeMap, BTreeSet};

        // Oracle differential (br-frankenpandas-xnbl7): groupby_nunique (dense
        // 2-D seen-bitset path b562aef4) must equal a hand-computed per-group
        // distinct-value count. Deterministic seeded LCG — no rand, no mocks.
        let mut state: u64 = 0x09e2_177b_1dea_d5e7_u64.wrapping_mul(3);
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };

        for iter in 0..1200u32 {
            let n = (next() % 14) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 5) as i64 - 2).collect();
            // Small value range so duplicates within a group are common.
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 6) as i64).collect();

            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("keys");
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("values");

            // Independent oracle: distinct non-null value count per key (keys
            // ascending = pandas default).
            let mut groups: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                groups.entry(*k).or_default().insert(*v);
            }
            let exp_keys: Vec<IndexLabel> = groups.keys().map(|&k| IndexLabel::Int64(k)).collect();
            let exp_nunique: Vec<Scalar> = groups
                .values()
                .map(|set| Scalar::Int64(set.len() as i64))
                .collect();

            let ctx = format!("iter={iter} keys={key_vals:?} vals={val_vals:?}");
            let mut led = EvidenceLedger::new();
            let out = groupby_nunique(
                &keys,
                &values,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut led,
            )
            .expect("nunique");
            assert_eq!(out.index().labels(), exp_keys, "nunique keys {ctx}");
            assert_eq!(out.values(), exp_nunique.as_slice(), "nunique vals {ctx}");
        }
    }

    #[test]
    fn dense_int64_groupby_prod_matches_handcomputed_oracle_ybda2() {
        use std::collections::BTreeMap;

        // Oracle differential (br-frankenpandas-ybda2): groupby_prod (dense Prod
        // streaming path ab5a2aba) must equal a hand-computed per-group product.
        // Tiny values (0..=2) keep products well within i64 (no overflow).
        // Deterministic seeded LCG — no rand, no mocks.
        let mut state: u64 = 0x70d_face_b00c_1357u64.wrapping_mul(5);
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };

        for iter in 0..1200u32 {
            let n = (next() % 12) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 5) as i64 - 2).collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 3) as i64).collect();

            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("keys");
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .expect("values");

            let mut groups: BTreeMap<i64, i64> = BTreeMap::new();
            let mut keys_seen: BTreeMap<i64, ()> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                keys_seen.entry(*k).or_default();
                let e = groups.entry(*k).or_insert(1);
                *e *= *v;
            }
            let exp_keys: Vec<IndexLabel> =
                keys_seen.keys().map(|&k| IndexLabel::Int64(k)).collect();
            // groupby_prod preserves Int64 for Int64 input (pandas parity, fixed in
            // br-frankenpandas-rl25i; previously returned Float64).
            let exp_prod: Vec<Scalar> = groups.values().map(|&p| Scalar::Int64(p)).collect();

            let ctx = format!("iter={iter} keys={key_vals:?} vals={val_vals:?}");
            let mut led = EvidenceLedger::new();
            let out = groupby_prod(
                &keys,
                &values,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut led,
            )
            .expect("prod");
            assert_eq!(out.index().labels(), exp_keys, "prod keys {ctx}");
            assert_eq!(out.values(), exp_prod.as_slice(), "prod vals {ctx}");
        }
    }

    #[test]
    fn groupby_order_invariant_aggregations_q60t7() {
        // Metamorphic (br-frankenpandas-q60t7): order-insensitive aggregations
        // must be invariant to input row order. Compute each on the original rows
        // and on the row-reversed rows; assert identical index + values.
        // Deterministic seeded LCG — no rand, no mocks.
        let mut s: u64 = 0x0d2e_4c91_a17a_5e7f;
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as u32
        };

        use super::GroupByError;
        type GbFn = fn(
            &Series,
            &Series,
            GroupByOptions,
            &RuntimePolicy,
            &mut EvidenceLedger,
        ) -> Result<Series, GroupByError>;
        let aggs: [(&str, GbFn); 5] = [
            ("sum", groupby_sum),
            ("count", groupby_count),
            ("min", groupby_min),
            ("max", groupby_max),
            ("prod", groupby_prod),
        ];

        for iter in 0..1000u32 {
            let n = (next() % 12) as usize + 1;
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 5) as i64 - 2).collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 3) as i64).collect();

            let mk = |keyv: &[i64], valv: &[i64]| {
                let idx: Vec<IndexLabel> = (0..keyv.len() as i64).map(IndexLabel::Int64).collect();
                let k = Series::from_values(
                    "k",
                    idx.clone(),
                    keyv.iter().copied().map(Scalar::Int64).collect::<Vec<_>>(),
                )
                .unwrap();
                let v = Series::from_values(
                    "v",
                    idx,
                    valv.iter().copied().map(Scalar::Int64).collect::<Vec<_>>(),
                )
                .unwrap();
                (k, v)
            };

            let (k1, v1) = mk(&key_vals, &val_vals);
            let mut rk = key_vals.clone();
            let mut rv = val_vals.clone();
            rk.reverse();
            rv.reverse();
            let (k2, v2) = mk(&rk, &rv);

            for (name, f) in aggs {
                let mut l1 = EvidenceLedger::new();
                let mut l2 = EvidenceLedger::new();
                let a = f(
                    &k1,
                    &v1,
                    GroupByOptions::default(),
                    &RuntimePolicy::strict(),
                    &mut l1,
                )
                .expect("agg orig");
                let b = f(
                    &k2,
                    &v2,
                    GroupByOptions::default(),
                    &RuntimePolicy::strict(),
                    &mut l2,
                )
                .expect("agg rev");
                assert_eq!(
                    a.index().labels(),
                    b.index().labels(),
                    "{name} keys order-invariance iter={iter} keys={key_vals:?}"
                );
                assert_eq!(
                    a.values(),
                    b.values(),
                    "{name} vals order-invariance iter={iter} keys={key_vals:?} vals={val_vals:?}"
                );
            }
        }
    }

    #[test]
    fn groupby_size_sum_equals_n_5cb23() {
        // Invariant (br-frankenpandas-5cb23): sum of groupby_size == n (all-valid keys).
        // Seeded LCG, no mocks.
        let mut s: u64 = 0x4c0b_0c2d_2d3e_4f50;
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as u32
        };
        let pol = RuntimePolicy::strict();
        for iter in 0..400u32 {
            let n = (next() % 20) as usize + 1;
            let keys: Vec<i64> = (0..n).map(|_| (next() % 4) as i64).collect();
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let ks = Series::from_values(
                "k",
                idx.clone(),
                keys.iter().map(|&x| Scalar::Int64(x)).collect::<Vec<_>>(),
            )
            .unwrap();
            let vs = Series::from_values(
                "v",
                idx,
                (0..n).map(|i| Scalar::Int64(i as i64)).collect::<Vec<_>>(),
            )
            .unwrap();
            let mut l = EvidenceLedger::new();
            let size = groupby_size(&ks, &vs, GroupByOptions::default(), &pol, &mut l).unwrap();
            let total: i64 = size
                .values()
                .iter()
                .map(|c| match c {
                    Scalar::Int64(x) => *x,
                    Scalar::Float64(x) => *x as i64,
                    _ => 0,
                })
                .sum();
            assert_eq!(total, n as i64, "sum(size)==n iter={iter}");
        }
    }

    #[test]
    fn groupby_min_le_mean_le_max_3obe6() {
        // Metamorphic (br-frankenpandas-3obe6): per group, min<=mean<=max. Seeded LCG.
        let mut s: u64 = 0x4c0b_0b2c_2d3e_4f50;
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as u32
        };
        let getf = |c: &[Scalar]| -> Vec<f64> {
            c.iter()
                .map(|v| match v {
                    Scalar::Int64(x) => *x as f64,
                    Scalar::Float64(x) => *x,
                    _ => f64::NAN,
                })
                .collect()
        };
        let pol = RuntimePolicy::strict();
        for iter in 0..400u32 {
            let n = (next() % 16) as usize + 1;
            let keys: Vec<i64> = (0..n).map(|_| (next() % 4) as i64).collect();
            let vals: Vec<f64> = (0..n).map(|_| (next() % 200) as f64 / 3.0 - 33.0).collect();
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let ks = Series::from_values(
                "k",
                idx.clone(),
                keys.iter().map(|&x| Scalar::Int64(x)).collect::<Vec<_>>(),
            )
            .unwrap();
            let vs = Series::from_values(
                "v",
                idx,
                vals.iter().map(|&x| Scalar::Float64(x)).collect::<Vec<_>>(),
            )
            .unwrap();
            let mut l = EvidenceLedger::new();
            let mins = getf(
                groupby_min(&ks, &vs, GroupByOptions::default(), &pol, &mut l)
                    .unwrap()
                    .values(),
            );
            let means = getf(
                groupby_mean(&ks, &vs, GroupByOptions::default(), &pol, &mut l)
                    .unwrap()
                    .values(),
            );
            let maxs = getf(
                groupby_max(&ks, &vs, GroupByOptions::default(), &pol, &mut l)
                    .unwrap()
                    .values(),
            );
            for g in 0..mins.len() {
                assert!(mins[g] <= means[g] + 1e-9, "min<=mean iter={iter} g={g}");
                assert!(means[g] <= maxs[g] + 1e-9, "mean<=max iter={iter} g={g}");
            }
        }
    }

    #[test]
    fn groupby_sum_equals_mean_times_count_0czr8() {
        // Metamorphic (br-frankenpandas-0czr8): per group, sum == mean * count.
        // Seeded LCG, no mocks.
        let mut s: u64 = 0x4c0c_0b1c_2d3e_4f50;
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as u32
        };
        let getf = |c: &[Scalar]| -> Vec<f64> {
            c.iter()
                .map(|v| match v {
                    Scalar::Int64(x) => *x as f64,
                    Scalar::Float64(x) => *x,
                    _ => f64::NAN,
                })
                .collect()
        };
        let pol = RuntimePolicy::strict();
        for iter in 0..400u32 {
            let n = (next() % 16) as usize + 1;
            let keys: Vec<i64> = (0..n).map(|_| (next() % 4) as i64).collect();
            let vals: Vec<f64> = (0..n).map(|_| (next() % 100) as f64 / 3.0).collect();
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let ks = Series::from_values(
                "k",
                idx.clone(),
                keys.iter().map(|&x| Scalar::Int64(x)).collect::<Vec<_>>(),
            )
            .unwrap();
            let vs = Series::from_values(
                "v",
                idx,
                vals.iter().map(|&x| Scalar::Float64(x)).collect::<Vec<_>>(),
            )
            .unwrap();
            let mut l = EvidenceLedger::new();
            let sums = getf(
                groupby_sum(&ks, &vs, GroupByOptions::default(), &pol, &mut l)
                    .unwrap()
                    .values(),
            );
            let means = getf(
                groupby_mean(&ks, &vs, GroupByOptions::default(), &pol, &mut l)
                    .unwrap()
                    .values(),
            );
            let counts = getf(
                groupby_count(&ks, &vs, GroupByOptions::default(), &pol, &mut l)
                    .unwrap()
                    .values(),
            );
            assert_eq!(sums.len(), means.len(), "len iter={iter}");
            assert_eq!(sums.len(), counts.len(), "len iter={iter}");
            for g in 0..sums.len() {
                assert!(
                    (sums[g] - means[g] * counts[g]).abs() < 1e-6,
                    "sum==mean*count iter={iter} g={g}"
                );
            }
        }
    }

    #[test]
    fn groupby_count_vs_size_null_values_z1o82() {
        // br-frankenpandas-z1o82: count excludes null values; size counts all rows.
        let idx: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
        let keys = Series::from_values(
            "k",
            idx.clone(),
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "v",
            idx,
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(f64::NAN),
                Scalar::Float64(20.0),
            ],
        )
        .unwrap();
        let pol = RuntimePolicy::strict();
        let mut l1 = EvidenceLedger::new();
        let mut l2 = EvidenceLedger::new();
        let count =
            groupby_count(&keys, &values, GroupByOptions::default(), &pol, &mut l1).unwrap();
        let size = groupby_size(&keys, &values, GroupByOptions::default(), &pol, &mut l2).unwrap();
        // a: count 1 (NaN excluded), size 2; b: count 1, size 1.
        assert_eq!(
            count.values(),
            &[Scalar::Int64(1), Scalar::Int64(1)],
            "count excludes NaN"
        );
        assert_eq!(
            size.values(),
            &[Scalar::Int64(2), Scalar::Int64(1)],
            "size counts all rows"
        );
    }

    #[test]
    fn groupby_drops_null_key_rows_tr1un() {
        // br-frankenpandas-tr1un: groupby (dropna=true default) excludes null-key
        // rows. Keys [a,null,a,b] vals [10,20,30,40] -> a=40, b=40 (null row dropped).
        let idx: Vec<IndexLabel> = (0..4i64).map(IndexLabel::Int64).collect();
        let keys = Series::from_values(
            "k",
            idx.clone(),
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Null(NullKind::Null),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "v",
            idx,
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ],
        )
        .unwrap();
        let mut led = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut led,
        )
        .unwrap();
        // Null-key group excluded; a and b only.
        assert_eq!(out.len(), 2, "null-key group dropped");
        assert_eq!(
            out.index().labels(),
            &[
                IndexLabel::Utf8("a".to_owned()),
                IndexLabel::Utf8("b".to_owned())
            ]
        );
        assert_eq!(
            out.values(),
            &[Scalar::Int64(40), Scalar::Int64(40)],
            "sums exclude null-key value 20"
        );
    }

    #[test]
    fn groupby_var_std_two_pass_oracle_7io1l() {
        use std::collections::BTreeMap;

        // Oracle differential (br-frankenpandas-7io1l): groupby_var/std == per-group
        // two-pass variance (ddof=1; NaN for n<2), std==sqrt(var). Seeded LCG, no mocks.
        let mut st: u64 = 0x7a20_1c0d_2b3e_4f50;
        let mut next = || {
            st = st
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (st >> 33) as u32
        };
        let num = |s: &Scalar| -> f64 {
            match s {
                Scalar::Float64(x) => *x,
                Scalar::Int64(x) => *x as f64,
                _ => f64::NAN,
            }
        };
        for iter in 0..800u32 {
            let n = (next() % 12) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 4) as i64 - 1).collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 21) as i64 - 10).collect();
            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .unwrap();

            let mut groups: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                groups.entry(*k).or_default().push(*v as f64);
            }
            let exp_var: Vec<f64> = groups
                .values()
                .map(|vs| {
                    if vs.len() < 2 {
                        f64::NAN
                    } else {
                        let m = vs.iter().sum::<f64>() / vs.len() as f64;
                        vs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (vs.len() as f64 - 1.0)
                    }
                })
                .collect();

            let pol = RuntimePolicy::strict();
            let mut led = EvidenceLedger::new();
            let var = groupby_var(&keys, &values, GroupByOptions::default(), &pol, &mut led)
                .expect("var");
            let std = groupby_std(&keys, &values, GroupByOptions::default(), &pol, &mut led)
                .expect("std");
            let ctx = format!("iter={iter} keys={key_vals:?} vals={val_vals:?}");
            for (i, ev) in exp_var.iter().enumerate() {
                let gv = num(&var.values()[i]);
                let gs = num(&std.values()[i]);
                if ev.is_nan() {
                    assert!(gv.is_nan(), "var NaN {ctx} i={i}");
                    assert!(gs.is_nan(), "std NaN {ctx} i={i}");
                } else {
                    assert!((gv - ev).abs() < 1e-7, "var {ctx} i={i}: {gv} vs {ev}");
                    assert!((gs - ev.sqrt()).abs() < 1e-7, "std {ctx} i={i}");
                }
            }
        }
    }

    #[test]
    fn groupby_median_matches_sorted_middle_oracle_k3awo() {
        use std::collections::BTreeMap;

        // Oracle differential (br-frankenpandas-k3awo): groupby_median == per-group
        // sorted-middle (odd -> middle, even -> mean of two middles), ascending
        // keys. Deterministic seeded LCG, no mocks.
        let mut st: u64 = 0x6ed1_a40c_3b2a_1908;
        let mut next = || {
            st = st
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (st >> 33) as u32
        };
        for iter in 0..1000u32 {
            let n = (next() % 12) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n).map(|_| (next() % 4) as i64 - 1).collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 21) as i64 - 10).collect();
            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .unwrap();

            let mut groups: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                groups.entry(*k).or_default().push(*v);
            }
            let exp_keys: Vec<IndexLabel> = groups.keys().map(|&k| IndexLabel::Int64(k)).collect();
            let exp_median: Vec<f64> = groups
                .values()
                .map(|vs| {
                    let mut s = vs.clone();
                    s.sort_unstable();
                    let m = s.len() / 2;
                    if s.len() % 2 == 1 {
                        s[m] as f64
                    } else {
                        (s[m - 1] + s[m]) as f64 / 2.0
                    }
                })
                .collect();

            let ctx = format!("iter={iter} keys={key_vals:?} vals={val_vals:?}");
            let mut led = EvidenceLedger::new();
            let out = groupby_median(
                &keys,
                &values,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut led,
            )
            .expect("median");
            assert_eq!(out.index().labels(), exp_keys, "median keys {ctx}");
            for (i, got) in out.values().iter().enumerate() {
                let g = match got {
                    Scalar::Float64(x) => *x,
                    Scalar::Int64(x) => *x as f64,
                    _ => f64::NAN,
                };
                assert!(
                    (g - exp_median[i]).abs() < 1e-9,
                    "median val {ctx} i={i}: {g} vs {}",
                    exp_median[i]
                );
            }
        }
    }

    #[test]
    fn dense_median_dispatch_matches_generic_bits() {
        let keys = vec![
            Scalar::Int64(2),
            Scalar::Int64(-1),
            Scalar::Int64(2),
            Scalar::Int64(-1),
            Scalar::Int64(0),
            Scalar::Int64(0),
            Scalar::Int64(3),
        ];
        let values = vec![
            Scalar::Float64(9.0),
            Scalar::Float64(-0.0),
            Scalar::Float64(1.0),
            Scalar::Float64(0.0),
            Scalar::Float64(4.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Null(NullKind::Null),
        ];

        for sort in [false, true] {
            let dense = try_groupby_median_dense_int64(&keys, &values, true, sort)
                .expect("bounded Int64 keys take the dense path");
            let generic = try_groupby_median_numeric_vectors(&keys, &values, true, sort)
                .expect("numeric values take the generic path");

            assert_eq!(dense.0, generic.0);
            assert_eq!(dense.1.len(), generic.1.len());
            for (dense_value, generic_value) in dense.1.iter().zip(&generic.1) {
                match (dense_value, generic_value) {
                    (Scalar::Float64(dense), Scalar::Float64(generic)) => {
                        assert_eq!(dense.to_bits(), generic.to_bits());
                    }
                    _ => assert_eq!(dense_value, generic_value),
                }
            }
        }
    }

    #[test]
    fn dense_int64_groupby_sparse_keys_matches_oracle_u4c5a() {
        use std::collections::BTreeMap;

        // Oracle (br-frankenpandas-u4c5a): widely-spread keys exceed the dense
        // histogram threshold, exercising the sparse/hash grouping path. It must
        // match the same BTreeMap oracle as the dense path. Seeded LCG, no mocks.
        const WIDE_KEYS: [i64; 6] = [0, 1, -1, 1_000_000, -500_000, 7];
        let mut st: u64 = 0x5a2e_5e00_4c5a_d00d;
        let mut next = || {
            st = st
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (st >> 33) as u32
        };

        for iter in 0..1000u32 {
            let n = (next() % 14) as usize + 1;
            let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
            let key_vals: Vec<i64> = (0..n)
                .map(|_| WIDE_KEYS[(next() as usize) % WIDE_KEYS.len()])
                .collect();
            let val_vals: Vec<i64> = (0..n).map(|_| (next() % 21) as i64 - 10).collect();

            let keys = Series::from_values(
                "k",
                idx.clone(),
                key_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let values = Series::from_values(
                "v",
                idx,
                val_vals
                    .iter()
                    .copied()
                    .map(Scalar::Int64)
                    .collect::<Vec<_>>(),
            )
            .unwrap();

            let mut groups: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
            for (k, v) in key_vals.iter().zip(val_vals.iter()) {
                groups.entry(*k).or_default().push(*v);
            }
            let exp_keys: Vec<IndexLabel> = groups.keys().map(|&k| IndexLabel::Int64(k)).collect();
            let exp_sum: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(vs.iter().sum()))
                .collect();
            let exp_count: Vec<Scalar> = groups
                .values()
                .map(|vs| Scalar::Int64(vs.len() as i64))
                .collect();

            let ctx = format!("iter={iter} keys={key_vals:?}");
            let pol = RuntimePolicy::strict();
            let mut led = EvidenceLedger::new();
            let sum = groupby_sum(&keys, &values, GroupByOptions::default(), &pol, &mut led)
                .expect("sum");
            assert_eq!(sum.index().labels(), exp_keys, "sparse sum keys {ctx}");
            assert_eq!(sum.values(), exp_sum.as_slice(), "sparse sum vals {ctx}");
            let cnt = groupby_count(&keys, &values, GroupByOptions::default(), &pol, &mut led)
                .expect("count");
            assert_eq!(
                cnt.values(),
                exp_count.as_slice(),
                "sparse count vals {ctx}"
            );
        }
    }

    #[test]
    fn groupby_sum_concatenates_string_values_like_pandas() {
        // pandas df.groupby(k)['s'].sum() concatenates object/string values per
        // group (skipna), matching Series::sum (br-f031e). Previously the f64
        // accumulator dropped every string -> Float64(0.0). Verified vs pandas
        // 2.2.3: groups a -> "xy" (skips the null), b -> "z".
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Null(NullKind::Null),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("z".to_owned()),
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
        assert_eq!(
            out.values(),
            &[Scalar::Utf8("xy".to_owned()), Scalar::Utf8("z".to_owned()),]
        );
    }

    #[test]
    fn groupby_sum_sort_false_preserves_first_seen_key_order() {
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
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &["b".into(), "a".into()]);
        assert_eq!(out.values(), &[Scalar::Int64(4), Scalar::Int64(6)]);
    }

    #[test]
    fn groupby_sum_records_runtime_admission_evidence() {
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

        let policy = RuntimePolicy::strict();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.values(), &[Scalar::Int64(4), Scalar::Int64(2)]);
        assert_eq!(ledger.records().len(), 1);
    }

    #[test]
    fn groupby_sum_merges_negative_zero_and_zero_float_keys() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Float64(-0.0),
                Scalar::Float64(1.0),
                Scalar::Float64(0.0),
            ],
        )
        .expect("keys");

        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(4)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels().len(), 2);
        assert_eq!(out.index().labels()[1], "1".into());
        assert_eq!(out.values(), &[Scalar::Int64(5), Scalar::Int64(2)]);
    }

    #[test]
    fn groupby_agg_records_runtime_admission_evidence() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
            ],
        )
        .expect("keys");
        let values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(5), Scalar::Int64(6), Scalar::Int64(7)],
        )
        .expect("values");

        let policy = RuntimePolicy::strict();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_agg(
            &keys,
            &values,
            AggFunc::Count,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("groupby agg");

        assert_eq!(out.values(), &[Scalar::Int64(2), Scalar::Int64(1)]);
        assert_eq!(ledger.records().len(), 1);
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
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        // Duplicate-label alignment expands to a cartesian product of duplicate positions.
        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(out.values(), &[Scalar::Int64(6), Scalar::Int64(3)]);
    }

    #[test]
    fn groupby_sum_int_dense_path_sorts_keys_by_default() {
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
            &[(-2_i64).into(), 5_i64.into(), 10_i64.into()]
        );
        assert_eq!(
            out.values(),
            &[Scalar::Int64(4), Scalar::Int64(2), Scalar::Int64(4),]
        );
    }

    #[test]
    fn groupby_sum_int_dense_path_preserves_first_seen_order_when_sort_disabled() {
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
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
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
            &[Scalar::Int64(4), Scalar::Int64(2), Scalar::Int64(4)]
        );
    }

    #[test]
    fn dense_int64_value_path_preserves_sort_and_first_seen_order() {
        let keys = vec![
            Scalar::Int64(10),
            Scalar::Int64(5),
            Scalar::Int64(10),
            Scalar::Int64(-2),
        ];
        let values = vec![
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Int64(3),
            Scalar::Int64(4),
        ];

        let (sorted_index, sorted_values) =
            try_groupby_sum_dense_int64_values(&keys, &values, true, true).expect("dense");
        assert_eq!(
            sorted_index,
            vec![(-2_i64).into(), 5_i64.into(), 10_i64.into()]
        );
        assert_eq!(
            sorted_values,
            vec![Scalar::Int64(4), Scalar::Int64(2), Scalar::Int64(4)]
        );

        let (first_seen_index, first_seen_values) =
            try_groupby_sum_dense_int64_values(&keys, &values, true, false).expect("dense");
        assert_eq!(
            first_seen_index,
            vec![10_i64.into(), 5_i64.into(), (-2_i64).into()]
        );
        assert_eq!(
            first_seen_values,
            vec![Scalar::Int64(4), Scalar::Int64(2), Scalar::Int64(4)]
        );
    }

    #[test]
    fn dense_int64_raw_sum_matches_scalar_reference_zjkxd() {
        let raw_keys = vec![10_i64, 5, 10, -2, 5];
        let raw_values = vec![i64::MAX, 2, 1, 4, -3];
        let scalar_keys = raw_keys
            .iter()
            .copied()
            .map(Scalar::Int64)
            .collect::<Vec<_>>();
        let scalar_values = raw_values
            .iter()
            .copied()
            .map(Scalar::Int64)
            .collect::<Vec<_>>();

        for sort in [true, false] {
            let expected =
                try_groupby_sum_dense_int64_values(&scalar_keys, &scalar_values, true, sort)
                    .expect("former Scalar dense path");
            let actual = try_groupby_sum_dense_int64_slices(&raw_keys, &raw_values, sort)
                .expect("raw dense path");
            assert_eq!(actual, expected);

            let index = Index::from_range(0, raw_keys.len() as i64, 1);
            let keys = Series::new(
                "key",
                index.clone(),
                Column::from_i64_values_owned(raw_keys.clone()),
            )
            .expect("typed keys");
            let values = Series::new(
                "value",
                index,
                Column::from_i64_values_owned(raw_values.clone()),
            )
            .expect("typed values");
            let mut ledger = EvidenceLedger::new();
            let public = groupby_sum(
                &keys,
                &values,
                GroupByOptions {
                    sort,
                    ..GroupByOptions::default()
                },
                &RuntimePolicy::strict(),
                &mut ledger,
            )
            .expect("public raw sum");
            let expected_public = Series::new(
                "sum",
                Index::new(expected.0.clone()),
                Column::from_values(expected.1.clone()).expect("former output column"),
            )
            .expect("former public output");
            assert_eq!(public.index(), expected_public.index());
            assert_eq!(public.values(), expected_public.values());
        }

        assert_eq!(
            try_groupby_sum_dense_int64_slices(&[], &[], true),
            Some((Vec::new(), Vec::new()))
        );
        assert!(try_groupby_sum_dense_int64_slices(&[0], &[], true).is_none());
        assert!(try_groupby_sum_dense_int64_slices(&[i64::MIN, i64::MAX], &[1, 2], true).is_none());
    }

    #[test]
    fn dense_int64_raw_mean_matches_scalar_reference_o9svg() {
        let raw_keys = vec![10_i64, 5, 10, -2, 5];
        let raw_values = vec![i64::MAX, 2, i64::MIN + 1, 4, -3];
        let scalar_keys = raw_keys
            .iter()
            .copied()
            .map(Scalar::Int64)
            .collect::<Vec<_>>();
        let scalar_values = raw_values
            .iter()
            .copied()
            .map(Scalar::Int64)
            .collect::<Vec<_>>();

        for sort in [true, false] {
            let expected = try_groupby_agg_dense_int64(
                &scalar_keys,
                &scalar_values,
                AggFunc::Mean,
                true,
                sort,
            )
            .expect("former Scalar dense path");
            let actual = try_groupby_mean_dense_int64_slices(&raw_keys, &raw_values, sort)
                .expect("raw dense path");
            assert_eq!(actual, expected);

            let index = Index::from_range(0, raw_keys.len() as i64, 1);
            let keys = Series::new(
                "key",
                index.clone(),
                Column::from_i64_values_owned(raw_keys.clone()),
            )
            .expect("typed keys");
            let values = Series::new(
                "value",
                index,
                Column::from_i64_values_owned(raw_values.clone()),
            )
            .expect("typed values");
            let mut ledger = EvidenceLedger::new();
            let public = groupby_mean(
                &keys,
                &values,
                GroupByOptions {
                    sort,
                    ..GroupByOptions::default()
                },
                &RuntimePolicy::strict(),
                &mut ledger,
            )
            .expect("public raw mean");
            let expected_public = Series::new(
                "mean",
                Index::new(expected.0.clone()),
                Column::from_values(expected.1.clone()).expect("former output column"),
            )
            .expect("former public output");
            assert_eq!(public.index(), expected_public.index());
            assert_eq!(public.values(), expected_public.values());
        }

        assert_eq!(
            try_groupby_mean_dense_int64_slices(&[], &[], true),
            Some((Vec::new(), Vec::new()))
        );
        assert!(try_groupby_mean_dense_int64_slices(&[0], &[], true).is_none());
        assert!(
            try_groupby_mean_dense_int64_slices(&[i64::MIN, i64::MAX], &[1, 2], true).is_none()
        );
    }

    #[test]
    fn groupby_sum_dense_int64_value_path_preserves_bool_and_overflow() {
        let bool_keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)],
        )
        .expect("keys");
        let bool_values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &bool_keys,
            &bool_values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");
        assert_eq!(out.index().labels(), &[1_i64.into(), 2_i64.into()]);
        assert_eq!(out.values(), &[Scalar::Int64(1), Scalar::Int64(1)]);

        let overflow_keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(7), Scalar::Int64(7)],
        )
        .expect("keys");
        let overflow_values = Series::from_values(
            "value",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(i64::MAX), Scalar::Int64(1)],
        )
        .expect("values");

        let mut ledger = EvidenceLedger::new();
        let out = groupby_sum(
            &overflow_keys,
            &overflow_values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");
        assert_eq!(out.index().labels(), &[7_i64.into()]);
        assert_eq!(
            out.values(),
            &[Scalar::Float64((i128::from(i64::MAX) + 1) as f64)]
        );
    }

    #[test]
    fn groupby_sum_dropna_false_keeps_null_group_sorted_last() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Null(NullKind::Null),
                Scalar::Int64(10),
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
            GroupByOptions {
                dropna: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        // Verified pandas 2.2.3 (br-frankenpandas-8m6ay): the dropna=False
        // null group is labeled nan (None collapses into the nan group) and
        // sorts last: groupby([None, 10, 10]).sum() -> {10: 5, nan: 1}.
        assert_eq!(
            out.index().labels(),
            &[10_i64.into(), IndexLabel::Null(NullKind::NaN)]
        );
        assert_eq!(out.values(), &[Scalar::Int64(5), Scalar::Int64(1)]);
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
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels(), &[0_i64.into(), 100_000_i64.into()]);
        assert_eq!(out.values(), &[Scalar::Int64(4), Scalar::Int64(6)]);
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
        assert_eq!(out.values(), &[Scalar::Int64(60)]);
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
        // pandas promotes an int column with a missing value to Float64, so the
        // group sums are Float64: "a" = 5 + skipna = 5.0; "b" = all-missing = 0.0.
        // (br-frankenpandas-33d1h)
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
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .expect("groupby");

        assert_eq!(out.index().labels().len(), n);
        assert_eq!(out.values().len(), n);
        // Verify a few spot checks
        assert_eq!(out.values()[0], Scalar::Int64(0));
        assert_eq!(out.values()[999], Scalar::Int64(999));
        assert_eq!(out.values()[9999], Scalar::Int64(9999));
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

        // Both should produce the same sums in the same sorted order.
        assert_eq!(dense_out.values(), generic_out.values());
        // Dense produces IndexLabel::Int64(5), generic produces IndexLabel::Utf8("5")
        // So we verify ordering is the same after sorting.
        assert_eq!(
            dense_out.index().labels().len(),
            generic_out.index().labels().len()
        );
        assert_eq!(
            dense_out.index().labels(),
            &[IndexLabel::Int64(3), IndexLabel::Int64(5)]
        );
        assert_eq!(
            generic_out.index().labels(),
            &[
                IndexLabel::Utf8("3".to_owned()),
                IndexLabel::Utf8("5".to_owned())
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
            assert_eq!(out.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
        }
    }

    // === bd-2gi.16: Generic GroupBy Aggregation Tests ===

    use super::{
        AggFunc, groupby_agg, groupby_count, groupby_first, groupby_last, groupby_max,
        groupby_mean, groupby_median, groupby_min, groupby_std, groupby_var,
    };

    fn make_grouped_data() -> (Series, Series) {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ],
        )
        .unwrap();
        (keys, values)
    }

    #[test]
    fn groupby_mean_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_mean(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // a: (10+30)/2 = 20.0, b: (20+40)/2 = 30.0
        assert_eq!(
            out.values(),
            &[Scalar::Float64(20.0), Scalar::Float64(30.0)]
        );
        assert_eq!(out.name(), "mean");
    }

    #[test]
    fn groupby_count_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_count(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(out.values(), &[Scalar::Int64(2), Scalar::Int64(2)]);
        assert_eq!(out.name(), "count");
    }

    #[test]
    fn groupby_min_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_min(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // pandas groupby.min() preserves source Int64 dtype (no Float64
        // promotion), matching the br-764ys first/last surgery.
        assert_eq!(out.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
        assert_eq!(out.name(), "min");
    }

    #[test]
    fn groupby_max_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_max(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // pandas groupby.max() preserves source Int64 dtype.
        assert_eq!(out.values(), &[Scalar::Int64(30), Scalar::Int64(40)]);
        assert_eq!(out.name(), "max");
    }

    #[test]
    fn groupby_min_max_utf8_keys_skip_missing_and_keep_order() {
        let keys = Series::from_values(
            "key",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Int64(3),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(5),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let min_sorted = groupby_min(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let max_sorted = groupby_max(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            min_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            max_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            &min_sorted.values()[..2],
            &[Scalar::Int64(5), Scalar::Int64(1)]
        );
        assert_eq!(
            &max_sorted.values()[..2],
            &[Scalar::Int64(5), Scalar::Int64(3)]
        );
        assert!(min_sorted.values()[2].is_missing());
        assert!(max_sorted.values()[2].is_missing());

        let first_seen = GroupByOptions {
            sort: false,
            ..GroupByOptions::default()
        };
        let min_unsorted = groupby_min(
            &keys,
            &values,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let max_unsorted = groupby_max(
            &keys,
            &values,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            min_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            max_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            &min_unsorted.values()[..2],
            &[Scalar::Int64(1), Scalar::Int64(5)]
        );
        assert_eq!(
            &max_unsorted.values()[..2],
            &[Scalar::Int64(3), Scalar::Int64(5)]
        );
        assert!(min_unsorted.values()[2].is_missing());
        assert!(max_unsorted.values()[2].is_missing());
    }

    #[test]
    fn groupby_min_max_object_key_incomparable_values_return_nan() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into())],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("x".into()), Scalar::Int64(1)],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out_min = groupby_min(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let out_max = groupby_max(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out_min.index().labels(), &["a".into()]);
        assert_eq!(out_max.index().labels(), &["a".into()]);
        assert!(out_min.values()[0].is_missing());
        assert!(out_max.values()[0].is_missing());
    }

    #[test]
    fn groupby_first_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_first(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // Per br-frankenpandas-764ys: pandas groupby.first() preserves
        // source Int64 dtype (no Float64 promotion).
        assert_eq!(out.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
        assert_eq!(out.name(), "first");
    }

    #[test]
    fn groupby_last_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_last(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // Per br-frankenpandas-764ys: pandas groupby.last() preserves
        // source Int64 dtype (no Float64 promotion).
        assert_eq!(out.values(), &[Scalar::Int64(30), Scalar::Int64(40)]);
        assert_eq!(out.name(), "last");
    }

    #[test]
    fn groupby_first_last_utf8_keys_skip_missing_and_keep_order() {
        let keys = Series::from_values(
            "key",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Null(NullKind::Null),
                Scalar::Int64(10),
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(5),
                Scalar::Int64(2),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let first_sorted = groupby_first(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let last_sorted = groupby_last(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            first_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            last_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            &first_sorted.values()[..2],
            &[Scalar::Int64(10), Scalar::Int64(1)]
        );
        assert_eq!(
            &last_sorted.values()[..2],
            &[Scalar::Int64(5), Scalar::Int64(2)]
        );
        assert!(first_sorted.values()[2].is_missing());
        assert!(last_sorted.values()[2].is_missing());

        let first_seen = GroupByOptions {
            sort: false,
            ..GroupByOptions::default()
        };
        let first_unsorted = groupby_first(
            &keys,
            &values,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let last_unsorted = groupby_last(
            &keys,
            &values,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            first_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            last_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            &first_unsorted.values()[..2],
            &[Scalar::Int64(1), Scalar::Int64(10)]
        );
        assert_eq!(
            &last_unsorted.values()[..2],
            &[Scalar::Int64(2), Scalar::Int64(5)]
        );
        assert!(first_unsorted.values()[2].is_missing());
        assert!(last_unsorted.values()[2].is_missing());
    }

    #[test]
    fn groupby_first_last_object_values_preserve_selected_scalar() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into())],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("x".into()), Scalar::Int64(7)],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out_first = groupby_first(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let out_last = groupby_last(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out_first.values(), &[Scalar::Utf8("x".into())]);
        assert_eq!(out_last.values(), &[Scalar::Int64(7)]);
    }

    #[test]
    fn groupby_agg_sum_matches_dedicated_sum() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let agg = groupby_agg(
            &keys,
            &values,
            AggFunc::Sum,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let dedicated = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(agg.index().labels(), dedicated.index().labels());
        assert_eq!(agg.values(), dedicated.values());
    }

    #[test]
    fn groupby_agg_sum_prod_utf8_keys_stream_integer_slots() {
        let keys = Series::from_values(
            "key",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Null(NullKind::Null),
                Scalar::Int64(10),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(5),
                Scalar::Int64(3),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let sum_sorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Sum,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let prod_sorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Prod,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            sum_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            prod_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            sum_sorted.values(),
            &[Scalar::Int64(15), Scalar::Int64(5), Scalar::Int64(0)]
        );
        assert_eq!(
            prod_sorted.values(),
            &[Scalar::Int64(50), Scalar::Int64(6), Scalar::Int64(1)]
        );

        let first_seen = GroupByOptions {
            sort: false,
            ..GroupByOptions::default()
        };
        let sum_unsorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Sum,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let prod_unsorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Prod,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            sum_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            prod_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            sum_unsorted.values(),
            &[Scalar::Int64(5), Scalar::Int64(15), Scalar::Int64(0)]
        );
        assert_eq!(
            prod_unsorted.values(),
            &[Scalar::Int64(6), Scalar::Int64(50), Scalar::Int64(1)]
        );
    }

    #[test]
    fn groupby_agg_sum_prod_bool_and_prod_overflow_preserve_fallbacks() {
        let bool_keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let bool_values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let bool_sum = groupby_agg(
            &bool_keys,
            &bool_values,
            AggFunc::Sum,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let bool_prod = groupby_agg(
            &bool_keys,
            &bool_values,
            AggFunc::Prod,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(bool_sum.values(), &[Scalar::Int64(1), Scalar::Int64(1)]);
        assert_eq!(bool_prod.values(), &[Scalar::Int64(0), Scalar::Int64(1)]);

        let overflow_keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into())],
        )
        .unwrap();
        let overflow_values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Int64(i64::MAX), Scalar::Int64(2)],
        )
        .unwrap();
        let overflow_prod = groupby_agg(
            &overflow_keys,
            &overflow_values,
            AggFunc::Prod,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            overflow_prod.values(),
            &[Scalar::Float64((i64::MAX as f64) * 2.0)]
        );
    }

    #[test]
    fn groupby_agg_sum_prod_float64_utf8_keys_stream_counters_2qb1i() {
        let keys = Series::from_values(
            "key",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
                5_i64.into(),
            ],
            vec![
                Scalar::Float64(1.5),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(4.0),
                Scalar::Float64(-2.0),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let sum_sorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Sum,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let prod_sorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Prod,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            sum_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            prod_sorted.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            sum_sorted.values(),
            &[
                Scalar::Float64(6.0),
                Scalar::Float64(-0.5),
                Scalar::Float64(0.0),
            ]
        );
        assert_eq!(
            prod_sorted.values(),
            &[
                Scalar::Float64(8.0),
                Scalar::Float64(-3.0),
                Scalar::Float64(1.0),
            ]
        );

        let first_seen = GroupByOptions {
            sort: false,
            ..GroupByOptions::default()
        };
        let sum_unsorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Sum,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let prod_unsorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Prod,
            first_seen,
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            sum_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            prod_unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            sum_unsorted.values(),
            &[
                Scalar::Float64(-0.5),
                Scalar::Float64(6.0),
                Scalar::Float64(0.0),
            ]
        );
        assert_eq!(
            prod_unsorted.values(),
            &[
                Scalar::Float64(-3.0),
                Scalar::Float64(8.0),
                Scalar::Float64(1.0),
            ]
        );
    }

    #[test]
    fn groupby_agg_sum_prod_timedelta_fallback_preserved_2qb1i() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into())],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Timedelta64(2), Scalar::Timedelta64(3)],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let sum = groupby_agg(
            &keys,
            &values,
            AggFunc::Sum,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let prod = groupby_agg(
            &keys,
            &values,
            AggFunc::Prod,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(sum.values(), &[Scalar::Timedelta64(5)]);
        assert!(prod.values()[0].is_missing());
    }

    #[test]
    fn groupby_mean_with_nulls_skips_missing() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_mean(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        // Mean of [10, 30] (null skipped) = 20.0
        assert_eq!(out.values(), &[Scalar::Float64(20.0)]);
    }

    #[test]
    fn groupby_count_excludes_nulls() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_count(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.values(), &[Scalar::Int64(2)]); // 2 non-null
    }

    #[test]
    fn groupby_min_all_nulls_returns_null() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into())],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Null(NullKind::Null), Scalar::Null(NullKind::Null)],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_min(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert!(out.values()[0].is_missing());
    }

    #[test]
    fn groupby_agg_empty_input() {
        let keys =
            Series::from_values("key", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();
        let values =
            Series::from_values("val", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();

        let mut ledger = EvidenceLedger::new();
        for func in [
            AggFunc::Sum,
            AggFunc::Mean,
            AggFunc::Count,
            AggFunc::Min,
            AggFunc::Max,
            AggFunc::First,
            AggFunc::Last,
        ] {
            let out = groupby_agg(
                &keys,
                &values,
                func,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut ledger,
            )
            .unwrap();
            assert!(
                out.is_empty(),
                "empty input should give empty output for {func:?}"
            );
        }
    }

    #[test]
    fn groupby_agg_sorts_keys_by_default() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_mean(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn groupby_agg_sort_false_preserves_first_seen_order() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_mean(
            &keys,
            &values,
            GroupByOptions {
                sort: false,
                ..GroupByOptions::default()
            },
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["c".into(), "a".into(), "b".into()]);
    }

    // === Std, Var, Median GroupBy Aggregation Tests ===

    #[test]
    fn groupby_std_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_std(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // a: std([10, 30]) = sqrt(((10-20)^2 + (30-20)^2) / 1) = sqrt(200) ≈ 14.142
        // b: std([20, 40]) = sqrt(((20-30)^2 + (40-30)^2) / 1) = sqrt(200) ≈ 14.142
        assert!(
            matches!(out.values()[0], Scalar::Float64(_)),
            "expected Float64"
        );
        let a_std = match &out.values()[0] {
            Scalar::Float64(v) => *v,
            _ => 0.0,
        };
        assert!((a_std - 200.0_f64.sqrt()).abs() < 1e-10, "a std={a_std}");
        assert_eq!(out.name(), "std");
    }

    #[test]
    fn groupby_var_basic() {
        let (keys, values) = make_grouped_data();
        let mut ledger = EvidenceLedger::new();
        let out = groupby_var(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // a: var([10, 30]) = ((10-20)^2 + (30-20)^2) / 1 = 200.0
        // b: var([20, 40]) = ((20-30)^2 + (40-30)^2) / 1 = 200.0
        assert_eq!(
            out.values(),
            &[Scalar::Float64(200.0), Scalar::Float64(200.0)]
        );
        assert_eq!(out.name(), "var");
    }

    #[test]
    fn groupby_var_std_match_pandas() {
        // groups a=[1,2,4,8], b=[1,1,1,5,9]; verified vs pandas 2.2.3
        // var (ddof=1): a=9.583333..., b=12.8; std: a=3.0956959..., b=3.5777088...
        let mut ledger = EvidenceLedger::new();
        let idx: Vec<_> = (0..9).map(|i| (i as i64).into()).collect();
        let mut kvals = vec![Scalar::Utf8("a".into()); 4];
        kvals.extend(vec![Scalar::Utf8("b".into()); 5]);
        let keys = Series::from_values("g", idx.clone(), kvals).unwrap();
        let values = Series::from_values(
            "v",
            idx,
            [1.0, 2.0, 4.0, 8.0, 1.0, 1.0, 1.0, 5.0, 9.0]
                .iter()
                .map(|x| Scalar::Float64(*x))
                .collect(),
        )
        .unwrap();
        let nums = |s: &Series| -> Vec<f64> {
            s.values()
                .iter()
                .filter_map(|v| {
                    if let Scalar::Float64(f) = v {
                        Some(*f)
                    } else {
                        None
                    }
                })
                .collect()
        };
        let approx = |a: f64, b: f64, ctx: &str| {
            assert!((a - b).abs() < 1e-9, "{ctx}: got {a}, expected {b}");
        };

        let var = groupby_var(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let vv = nums(&var);
        approx(vv[0], 9.583333333333334, "var a");
        approx(vv[1], 12.8, "var b");

        let std = groupby_std(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();
        let sv = nums(&std);
        approx(sv[0], 3.095_695_936_882_199, "std a");
        approx(sv[1], 3.5777087639996634, "std b");
    }

    #[test]
    fn groupby_median_basic() {
        let keys = Series::from_values(
            "key",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
            ],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("b".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
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
                Scalar::Int64(5),
                Scalar::Int64(15),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_median(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        // a: median([10, 20, 30]) = 20.0
        // b: median([5, 15]) = (5+15)/2 = 10.0
        assert_eq!(
            out.values(),
            &[Scalar::Float64(20.0), Scalar::Float64(10.0)]
        );
        assert_eq!(out.name(), "median");
    }

    #[test]
    fn groupby_std_single_value_returns_null() {
        let keys =
            Series::from_values("key", vec![0_i64.into()], vec![Scalar::Utf8("a".into())]).unwrap();
        let values =
            Series::from_values("val", vec![0_i64.into()], vec![Scalar::Int64(42)]).unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_std(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        // std of a single value is NaN/null (ddof=1, n-1=0)
        assert!(out.values()[0].is_missing());
    }

    #[test]
    fn groupby_var_with_nulls_skips_missing() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_var(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        // var([10, 30], ddof=1) = ((10-20)^2 + (30-20)^2) / 1 = 200.0
        assert_eq!(out.values(), &[Scalar::Float64(200.0)]);
    }

    #[test]
    fn groupby_var_std_utf8_keys_stream_numeric_counters_uza04202() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..7).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("c".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("d".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            (0..7).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(3.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(5.0),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(10.0),
            ],
        )
        .unwrap();

        let sorted_var = groupby_var(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        let sorted_std = groupby_std(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();

        assert_eq!(
            sorted_var.index().labels(),
            &["a".into(), "b".into(), "c".into(), "d".into()]
        );
        assert_eq!(sorted_std.index().labels(), sorted_var.index().labels());
        assert!(sorted_var.values()[0].is_missing());
        assert_eq!(sorted_var.values()[1], Scalar::Float64(2.0));
        assert!(sorted_var.values()[2].is_missing());
        assert!(sorted_var.values()[3].is_missing());
        let b_std = match sorted_std.values()[1] {
            Scalar::Float64(v) => v,
            _ => f64::NAN,
        };
        assert!((b_std - 2.0_f64.sqrt()).abs() < 1e-12);

        let first_seen = GroupByOptions {
            dropna: true,
            sort: false,
        };
        let unsorted_var = groupby_var(&keys, &values, first_seen, &policy, &mut ledger).unwrap();
        assert_eq!(
            unsorted_var.index().labels(),
            &["b".into(), "a".into(), "c".into(), "d".into()]
        );
        assert_eq!(unsorted_var.values()[0], Scalar::Float64(2.0));
        assert!(unsorted_var.values()[1].is_missing());
    }

    #[test]
    fn groupby_var_std_timedelta_fallback_preserves_dtype_uza04202() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..4).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "td",
            (0..4).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Timedelta64(10),
                Scalar::Timedelta64(20),
                Scalar::Timedelta64(30),
                Scalar::Null(NullKind::NaT),
            ],
        )
        .unwrap();

        let var = groupby_var(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        let std = groupby_std(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();

        assert_eq!(var.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(std.index().labels(), var.index().labels());
        assert_eq!(
            var.values(),
            &[
                Scalar::Timedelta64(50),
                Scalar::Timedelta64(fp_types::Timedelta::NAT)
            ]
        );
        assert_eq!(
            std.values(),
            &[
                Scalar::Timedelta64(7),
                Scalar::Timedelta64(fp_types::Timedelta::NAT)
            ]
        );
    }

    #[test]
    fn groupby_median_utf8_keys_numeric_vectors_uza04203() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..8).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("c".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("d".to_owned()),
                Scalar::Utf8("a".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            (0..8).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(10.0),
                Scalar::Float64(5.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(8.0),
                Scalar::Float64(6.0),
            ],
        )
        .unwrap();

        let sorted = groupby_median(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(
            sorted.index().labels(),
            &["a".into(), "b".into(), "c".into(), "d".into()]
        );
        assert_eq!(
            sorted.values(),
            &[
                Scalar::Float64(6.0),
                Scalar::Float64(3.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(8.0)
            ]
        );

        let first_seen = GroupByOptions {
            dropna: true,
            sort: false,
        };
        let unsorted = groupby_median(&keys, &values, first_seen, &policy, &mut ledger).unwrap();
        assert_eq!(
            unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into(), "d".into()]
        );
        assert_eq!(
            unsorted.values(),
            &[
                Scalar::Float64(3.0),
                Scalar::Float64(6.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(8.0)
            ]
        );
    }

    #[test]
    fn groupby_median_timedelta_fallback_preserves_dtype_uza04203() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..5).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "td",
            (0..5).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Timedelta64(10),
                Scalar::Timedelta64(20),
                Scalar::Timedelta64(30),
                Scalar::Null(NullKind::NaT),
                Scalar::Timedelta64(50),
            ],
        )
        .unwrap();

        let out = groupby_median(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();

        assert_eq!(out.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(
            out.values(),
            &[Scalar::Timedelta64(15), Scalar::Timedelta64(40)]
        );
    }

    #[test]
    fn groupby_nunique_utf8_keys_borrowed_sets_uza04204() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..12).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("c".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("d".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("d".to_owned()),
                Scalar::Utf8("e".to_owned()),
                Scalar::Utf8("e".to_owned()),
                Scalar::Utf8("e".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            (0..12).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Float64(-0.0),
                Scalar::Utf8("x".to_owned()),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(0.0),
                Scalar::Null(NullKind::Null),
                Scalar::Timedelta64(5),
                Scalar::Float64(1.0),
                Scalar::Timedelta64(5),
                Scalar::Bool(true),
                Scalar::Int64(1),
                Scalar::Float64(1.0),
            ],
        )
        .unwrap();

        let sorted = groupby_agg(
            &keys,
            &values,
            AggFunc::Nunique,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(
            sorted.index().labels(),
            &["a".into(), "b".into(), "c".into(), "d".into(), "e".into()]
        );
        assert_eq!(
            sorted.values(),
            &[
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(3),
            ]
        );

        let first_seen = GroupByOptions {
            dropna: true,
            sort: false,
        };
        let unsorted = groupby_nunique(&keys, &values, first_seen, &policy, &mut ledger).unwrap();
        assert_eq!(
            unsorted.index().labels(),
            &["b".into(), "a".into(), "c".into(), "d".into(), "e".into()]
        );
        assert_eq!(
            unsorted.values(),
            &[
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(3),
            ]
        );
    }

    #[test]
    fn groupby_median_even_count() {
        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("a".into()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .unwrap();

        let mut ledger = EvidenceLedger::new();
        let out = groupby_median(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut ledger,
        )
        .unwrap();

        // median([1, 2, 3, 4]) = (2+3)/2 = 2.5
        assert_eq!(out.values(), &[Scalar::Float64(2.5)]);
    }

    #[test]
    fn groupby_agg_empty_input_std_var_median() {
        let keys =
            Series::from_values("key", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();
        let values =
            Series::from_values("val", Vec::<IndexLabel>::new(), Vec::<Scalar>::new()).unwrap();

        let mut ledger = EvidenceLedger::new();
        for func in [AggFunc::Std, AggFunc::Var, AggFunc::Median] {
            let out = groupby_agg(
                &keys,
                &values,
                func,
                GroupByOptions::default(),
                &RuntimePolicy::strict(),
                &mut ledger,
            )
            .unwrap();
            assert!(
                out.is_empty(),
                "empty input should give empty output for {func:?}"
            );
        }
    }

    // === AG-12: Sketching / Streaming Aggregation Tests ===

    mod sketch_tests {
        use fp_types::{NullKind, Scalar};

        use super::super::*;

        // --- HyperLogLog Tests ---

        #[test]
        fn hll_empty_estimate_is_zero() {
            let hll = HyperLogLog::default_precision();
            assert!(hll.estimate() < 1.0, "empty HLL should estimate ~0");
        }

        #[test]
        fn hll_single_element() {
            let mut hll = HyperLogLog::default_precision();
            hll.insert(&Scalar::Int64(42));
            let est = hll.estimate();
            assert!((0.5..=2.0).contains(&est), "single element estimate={est}");
        }

        #[test]
        fn hll_distinct_count_within_error_bound() {
            let mut hll = HyperLogLog::default_precision();
            let n = 10_000;
            for i in 0..n {
                hll.insert(&Scalar::Int64(i));
            }
            let est = hll.estimate();
            let error = (est - n as f64).abs() / n as f64;
            assert!(
                error < 0.05,
                "HLL estimate={est}, expected={n}, relative_error={error}"
            );
        }

        #[test]
        fn hll_duplicates_do_not_inflate_count() {
            let mut hll = HyperLogLog::default_precision();
            // Insert same 100 values 100 times each
            for _ in 0..100 {
                for i in 0..100 {
                    hll.insert(&Scalar::Int64(i));
                }
            }
            let est = hll.estimate();
            // Should estimate ~100, not ~10000
            assert!(
                est < 200.0,
                "duplicates inflated HLL: estimate={est}, expected ~100"
            );
        }

        #[test]
        fn hll_utf8_values() {
            let mut hll = HyperLogLog::default_precision();
            for i in 0..1000 {
                hll.insert(&Scalar::Utf8(format!("key_{i}")));
            }
            let est = hll.estimate();
            let error = (est - 1000.0).abs() / 1000.0;
            assert!(
                error < 0.1,
                "HLL Utf8 estimate={est}, expected=1000, error={error}"
            );
        }

        #[test]
        fn hll_memory_usage() {
            let hll = HyperLogLog::default_precision();
            assert_eq!(hll.memory_bytes(), 16384, "p=14 -> 2^14 = 16384 bytes");
        }

        #[test]
        fn hll_high_rho_does_not_overflow() {
            let mut hll = HyperLogLog::new(6);
            hll.registers[0] = 40;
            hll.registers[1] = 50;
            hll.registers[2] = 60;
            let est = hll.estimate();
            assert!(est.is_finite(), "HLL estimate overflowed: {est}");
            assert!(est > 0.0, "HLL estimate should be positive: {est}");
        }

        // --- KLL Sketch Tests ---

        #[test]
        fn kll_empty_returns_none() {
            let kll = KllSketch::default_accuracy();
            assert!(kll.quantile(0.5).is_none());
            assert!(kll.is_empty());
        }

        #[test]
        fn kll_single_element_returns_it() {
            let mut kll = KllSketch::new(64);
            kll.insert(42.0);
            assert_eq!(kll.quantile(0.0), Some(42.0));
            assert_eq!(kll.quantile(0.5), Some(42.0));
            assert_eq!(kll.quantile(1.0), Some(42.0));
        }

        #[test]
        fn kll_median_within_error() {
            let mut kll = KllSketch::default_accuracy();
            let n = 10_000;
            for i in 0..n {
                kll.insert(i as f64);
            }
            assert_eq!(kll.len(), n);

            let median = kll.quantile(0.5).expect("non-empty");
            // True median of 0..9999 is 4999.5
            let error = (median - 4999.5).abs() / 10_000.0;
            assert!(
                error < 0.02,
                "KLL median={median}, expected ~4999.5, rank_error={error}"
            );
        }

        #[test]
        fn kll_min_max_endpoints() {
            let mut kll = KllSketch::default_accuracy();
            for i in 0..1000 {
                kll.insert(i as f64);
            }

            let min = kll.quantile(0.0).expect("min");
            let max = kll.quantile(1.0).expect("max");
            assert!(min <= 10.0, "KLL min={min}, expected near 0");
            assert!(max >= 990.0, "KLL max={max}, expected near 999");
        }

        #[test]
        fn kll_monotonic_quantiles() {
            let mut kll = KllSketch::default_accuracy();
            for i in 0..5000 {
                kll.insert(i as f64);
            }

            let q25 = kll.quantile(0.25).expect("q25");
            let q50 = kll.quantile(0.50).expect("q50");
            let q75 = kll.quantile(0.75).expect("q75");
            assert!(
                q25 <= q50 && q50 <= q75,
                "quantiles not monotonic: q25={q25}, q50={q50}, q75={q75}"
            );
        }

        #[test]
        #[ignore = "foreground attribution probe"]
        fn kll_compaction_promotion_allocation_profile_xagv2() {
            const K: usize = 32;
            const VALUES: usize = 131_072;
            const SAMPLES: usize = 9;

            #[derive(Debug, PartialEq)]
            struct ProbeKll {
                compactors: Vec<Vec<f64>>,
                size: usize,
                compact_count: usize,
            }

            impl ProbeKll {
                fn new() -> Self {
                    Self {
                        compactors: vec![Vec::with_capacity(K * 2)],
                        size: 0,
                        compact_count: 0,
                    }
                }

                fn insert(&mut self, value: f64, stream: bool) {
                    self.compactors[0].push(value);
                    self.size += 1;
                    if self.compactors[0].len() >= K * 2 {
                        self.compact(0, stream);
                    }
                }

                fn compact(&mut self, level: usize, stream: bool) {
                    self.compactors[level]
                        .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                    let offset = (self.compact_count + level) % 2;
                    self.compact_count = self.compact_count.wrapping_add(1);

                    if level + 1 >= self.compactors.len() {
                        self.compactors.push(Vec::with_capacity(K * 2));
                    }
                    if stream {
                        let (through_current, after_current) =
                            self.compactors.split_at_mut(level + 1);
                        let current = &mut through_current[level];
                        let next = &mut after_current[0];
                        next.extend(
                            current
                                .iter()
                                .copied()
                                .enumerate()
                                .filter_map(|(idx, value)| (idx % 2 == offset).then_some(value)),
                        );
                        current.clear();
                    } else {
                        let promoted = self.compactors[level]
                            .iter()
                            .copied()
                            .enumerate()
                            .filter_map(|(idx, value)| (idx % 2 == offset).then_some(value))
                            .collect::<Vec<_>>();
                        self.compactors[level].clear();
                        self.compactors[level + 1].extend(promoted);
                    }

                    if self.compactors[level + 1].len() >= K * 2 {
                        self.compact(level + 1, stream);
                    }
                }
            }

            fn build(values: &[f64], stream: bool) -> ProbeKll {
                let mut sketch = ProbeKll::new();
                for &value in values {
                    sketch.insert(value, stream);
                }
                sketch
            }

            fn elapsed(values: &[f64], stream: bool) -> (u128, ProbeKll) {
                let started = std::time::Instant::now();
                let sketch = build(std::hint::black_box(values), stream);
                (started.elapsed().as_nanos(), sketch)
            }

            fn median(mut samples: Vec<u128>) -> u128 {
                samples.sort_unstable();
                samples[samples.len() / 2]
            }

            let mut state = 0xD1B5_4A32_D192_ED03_u64;
            let values = (0..VALUES)
                .map(|_| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    ((state >> 24) & 0x00ff_ffff) as f64
                })
                .collect::<Vec<_>>();

            let former = build(&values, false);
            let candidate = build(&values, true);
            assert_eq!(candidate, former);

            for _ in 0..2 {
                std::hint::black_box(elapsed(&values, false));
                std::hint::black_box(elapsed(&values, true));
            }
            let mut former_samples = Vec::with_capacity(SAMPLES);
            let mut candidate_samples = Vec::with_capacity(SAMPLES);
            for sample in 0..SAMPLES {
                let order = if sample.is_multiple_of(2) {
                    [false, true]
                } else {
                    [true, false]
                };
                for stream in order {
                    let (elapsed, sketch) = elapsed(&values, stream);
                    assert_eq!(sketch, former);
                    if stream {
                        candidate_samples.push(elapsed);
                    } else {
                        former_samples.push(elapsed);
                    }
                }
            }
            let former_ns = median(former_samples);
            let candidate_ns = median(candidate_samples);
            println!(
                "KLL_PROMOTION_PROFILE values={VALUES} k={K} compactions={} former_ns={former_ns} candidate_ns={candidate_ns} directional_ratio={:.6}",
                former.compact_count,
                former_ns as f64 / candidate_ns as f64
            );
        }

        fn former_kll_compact(sketch: &mut KllSketch, level: usize) {
            sketch.compactors[level]
                .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let offset = (sketch.compact_count + level) % 2;
            sketch.compact_count = sketch.compact_count.wrapping_add(1);
            let promoted = sketch.compactors[level]
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(idx, value)| (idx % 2 == offset).then_some(value))
                .collect::<Vec<_>>();
            sketch.compactors[level].clear();
            if level + 1 >= sketch.compactors.len() {
                sketch.compactors.push(Vec::with_capacity(sketch.k * 2));
            }
            sketch.compactors[level + 1].extend(promoted);
            if sketch.compactors[level + 1].len() >= sketch.capacity_at_level(level + 1) {
                former_kll_compact(sketch, level + 1);
            }
        }

        fn former_kll_insert(sketch: &mut KllSketch, value: f64) {
            sketch.compactors[0].push(value);
            sketch.size += 1;
            if sketch.compactors[0].len() >= sketch.capacity_at_level(0) {
                former_kll_compact(sketch, 0);
            }
        }

        fn build_kll(values: &[f64], k: usize, candidate: bool) -> KllSketch {
            let mut sketch = KllSketch::new(k);
            for &value in values {
                if candidate {
                    sketch.insert(value);
                } else {
                    former_kll_insert(&mut sketch, value);
                }
            }
            sketch
        }

        fn assert_kll_bits_equal(candidate: &KllSketch, former: &KllSketch) {
            assert_eq!(candidate.k, former.k);
            assert_eq!(candidate.size, former.size);
            assert_eq!(candidate.compact_count, former.compact_count);
            assert_eq!(candidate.compactors.len(), former.compactors.len());
            for (candidate_level, former_level) in
                candidate.compactors.iter().zip(&former.compactors)
            {
                assert_eq!(candidate_level.len(), former_level.len());
                assert!(
                    candidate_level
                        .iter()
                        .zip(former_level)
                        .all(|(candidate, former)| candidate.to_bits() == former.to_bits())
                );
            }
            for quantile in [0.0, 0.25, 0.5, 0.75, 1.0] {
                assert_eq!(
                    candidate.quantile(quantile).map(f64::to_bits),
                    former.quantile(quantile).map(f64::to_bits)
                );
            }
        }

        #[test]
        fn kll_streamed_promotion_matches_former_bits_xagv2() {
            for k in [8, 32, 256] {
                let mut former = KllSketch::new(k);
                let mut candidate = KllSketch::new(k);
                for idx in 0..4_097_u64 {
                    let value = match idx % 257 {
                        0 => f64::NEG_INFINITY,
                        1 => f64::INFINITY,
                        2 => -0.0,
                        3 => 0.0,
                        _ => idx.wrapping_mul(6_364_136_223_846_793_005) as i64 as f64,
                    };
                    former_kll_insert(&mut former, value);
                    candidate.insert(value);
                    if idx < 20 || idx.is_power_of_two() || idx == 4_096 {
                        assert_kll_bits_equal(&candidate, &former);
                    }
                }
            }
        }

        #[test]
        #[ignore = "foreground release A/B harness"]
        fn kll_streamed_promotion_public_ab_xagv2() {
            const K: usize = 32;
            const VALUES: usize = 131_072;
            const BATCH: usize = 4;
            const BLOCKS: usize = 9;

            fn elapsed(values: &[f64], candidate: bool) -> u128 {
                std::hint::black_box(build_kll(values, K, candidate));
                let started = std::time::Instant::now();
                for _ in 0..BATCH {
                    let sketch = build_kll(std::hint::black_box(values), K, candidate);
                    std::hint::black_box(sketch.quantile(0.5));
                    std::hint::black_box(sketch);
                }
                started.elapsed().as_nanos()
            }

            fn median(mut samples: Vec<u128>) -> u128 {
                samples.sort_unstable();
                samples[samples.len() / 2]
            }

            fn spread(left: u128, right: u128) -> f64 {
                left.abs_diff(right) as f64 / ((left + right) as f64 / 2.0)
            }

            let mut state = 0xD1B5_4A32_D192_ED03_u64;
            let values = (0..VALUES)
                .map(|_| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    ((state >> 24) & 0x00ff_ffff) as f64
                })
                .collect::<Vec<_>>();
            let former = build_kll(&values, K, false);
            let candidate = build_kll(&values, K, true);
            assert_kll_bits_equal(&candidate, &former);

            let body_started = std::time::Instant::now();
            let mut former_a = Vec::with_capacity(BLOCKS);
            let mut former_b = Vec::with_capacity(BLOCKS);
            let mut candidate_a = Vec::with_capacity(BLOCKS);
            let mut candidate_b = Vec::with_capacity(BLOCKS);
            for block in 0..BLOCKS {
                let order = if block.is_multiple_of(2) {
                    [(false, 0), (true, 0), (true, 1), (false, 1)]
                } else {
                    [(true, 1), (false, 1), (false, 0), (true, 0)]
                };
                for (candidate, duplicate) in order {
                    let sample = elapsed(&values, candidate);
                    match (candidate, duplicate) {
                        (false, 0) => former_a.push(sample),
                        (false, _) => former_b.push(sample),
                        (true, 0) => candidate_a.push(sample),
                        (true, _) => candidate_b.push(sample),
                    }
                }
            }

            let former_a = median(former_a) / BATCH as u128;
            let former_b = median(former_b) / BATCH as u128;
            let candidate_a = median(candidate_a) / BATCH as u128;
            let candidate_b = median(candidate_b) / BATCH as u128;
            let former_mean = (former_a + former_b) as f64 / 2.0;
            let candidate_mean = (candidate_a + candidate_b) as f64 / 2.0;
            println!(
                "KLL_STREAM_AB values={VALUES} k={K} compactions={} former_a_ns={former_a} former_b_ns={former_b} candidate_a_ns={candidate_a} candidate_b_ns={candidate_b} former_spread={:.6} candidate_spread={:.6} speedup={:.6} body_seconds={:.6}",
                former.compact_count,
                spread(former_a, former_b),
                spread(candidate_a, candidate_b),
                former_mean / candidate_mean,
                body_started.elapsed().as_secs_f64()
            );
        }

        // --- Count-Min Sketch Tests ---

        #[test]
        fn cm_empty_estimate_is_zero() {
            let cm = CountMinSketch::default_accuracy();
            assert_eq!(cm.estimate(&Scalar::Int64(42)), 0);
        }

        #[test]
        fn cm_single_element_exact() {
            let mut cm = CountMinSketch::default_accuracy();
            cm.insert(&Scalar::Int64(42));
            assert_eq!(cm.estimate(&Scalar::Int64(42)), 1);
        }

        #[test]
        fn cm_never_underestimates() {
            let mut cm = CountMinSketch::default_accuracy();
            for _ in 0..100 {
                cm.insert(&Scalar::Utf8("a".into()));
            }
            for _ in 0..50 {
                cm.insert(&Scalar::Utf8("b".into()));
            }
            assert!(
                cm.estimate(&Scalar::Utf8("a".into())) >= 100,
                "CM underestimated 'a'"
            );
            assert!(
                cm.estimate(&Scalar::Utf8("b".into())) >= 50,
                "CM underestimated 'b'"
            );
        }

        #[test]
        fn cm_overestimate_bounded() {
            let mut cm = CountMinSketch::default_accuracy();
            let n = 10_000;
            for i in 0..n {
                cm.insert(&Scalar::Int64(i));
            }
            // Error <= N/width ≈ 9.77; allow 2x margin for hash variance.
            let max_overestimate = 2 * n as u64 / 1024 + 1;
            for i in 0..100 {
                let est = cm.estimate(&Scalar::Int64(i));
                assert!(
                    est <= 1 + max_overestimate,
                    "CM overestimate too high for key={i}: est={est}, max={max_overestimate}"
                );
            }
        }

        // --- Integration Tests: Public API ---

        #[test]
        fn approx_nunique_basic() {
            let values: Vec<Scalar> = (0..1000).map(Scalar::Int64).collect();
            let result = approx_nunique(&values);
            let error = (result.value - 1000.0).abs() / 1000.0;
            assert!(
                error < 0.1,
                "approx_nunique={}, expected=1000, error={error}",
                result.value
            );
        }

        #[test]
        fn approx_nunique_skips_nulls() {
            let values = vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
            ];
            let result = approx_nunique(&values);
            assert!(
                result.value >= 1.0 && result.value <= 4.0,
                "approx_nunique={}, expected ~2",
                result.value
            );
        }

        #[test]
        fn approx_quantile_basic() {
            let values: Vec<Scalar> = (0..1000).map(|i| Scalar::Float64(i as f64)).collect();
            let result = approx_quantile(&values, 0.5).expect("non-empty");
            assert!(
                (result.value - 499.5).abs() < 50.0,
                "approx_quantile median={}, expected ~499.5",
                result.value
            );
        }

        #[test]
        fn approx_quantile_empty_returns_none() {
            let values: Vec<Scalar> = vec![Scalar::Null(NullKind::Null)];
            assert!(approx_quantile(&values, 0.5).is_none());
        }

        #[test]
        fn approx_value_counts_basic() {
            let values = vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(1),
            ];
            let counts = approx_value_counts(&values);
            assert_eq!(counts.len(), 3, "3 distinct values");
            let count_1 = counts
                .iter()
                .find(|(k, _)| k == &Scalar::Int64(1))
                .map(|(_, c)| *c)
                .expect("key 1 present");
            assert!(count_1 >= 3, "count for 1 should be >= 3, got {count_1}");
        }

        #[test]
        fn approx_value_counts_skips_nulls() {
            let values = vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(1),
            ];
            let counts = approx_value_counts(&values);
            assert_eq!(counts.len(), 1, "only non-null values counted");
        }
    }

    // --- groupby_nunique / groupby_prod / groupby_size tests ---

    #[test]
    fn groupby_nunique_basic() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let options = GroupByOptions::default();

        let keys = Series::from_values(
            "key",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
            ],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("y".to_owned()),
            ],
        )
        .unwrap();

        let values = Series::from_values(
            "val",
            vec![
                0_i64.into(),
                1_i64.into(),
                2_i64.into(),
                3_i64.into(),
                4_i64.into(),
            ],
            vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .unwrap();

        let result = groupby_nunique(&keys, &values, options, &policy, &mut ledger).unwrap();
        // Group "x": values [1, 2] -> 2 unique
        // Group "y": values [3, 3, 4] -> 2 unique
        assert_eq!(result.len(), 2);
        assert_eq!(result.values()[0], Scalar::Int64(2)); // x: 2 unique
        assert_eq!(result.values()[1], Scalar::Int64(2)); // y: 2 unique
    }

    #[test]
    fn groupby_nunique_with_nulls() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let options = GroupByOptions::default();

        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
            ],
        )
        .unwrap();

        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(1),
            ],
        )
        .unwrap();

        let result = groupby_nunique(&keys, &values, options, &policy, &mut ledger).unwrap();
        assert_eq!(result.values()[0], Scalar::Int64(1)); // only 1 unique non-null
    }

    #[test]
    fn groupby_prod_basic() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let options = GroupByOptions::default();

        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
                Scalar::Utf8("y".to_owned()),
            ],
        )
        .unwrap();

        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
                Scalar::Int64(5),
            ],
        )
        .unwrap();

        let result = groupby_prod(&keys, &values, options, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2);
        // pandas groupby.prod() preserves Int64 for integer input (br-frankenpandas-rl25i).
        assert_eq!(result.values()[0], Scalar::Int64(6)); // x: 2*3
        assert_eq!(result.values()[1], Scalar::Int64(20)); // y: 4*5
    }

    #[test]
    fn groupby_size_counts_including_nulls() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let options = GroupByOptions::default();

        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("x".to_owned()),
                Scalar::Utf8("y".to_owned()),
            ],
        )
        .unwrap();

        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ],
        )
        .unwrap();

        let result = groupby_size(&keys, &values, options, &policy, &mut ledger).unwrap();
        assert_eq!(result.len(), 2);
        // size counts ALL elements including nulls (unlike count)
        assert_eq!(result.values()[0], Scalar::Int64(3)); // x: 3 total
        assert_eq!(result.values()[1], Scalar::Int64(1)); // y: 1 total
    }

    #[test]
    fn groupby_size_vs_count_difference() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let options = GroupByOptions::default();

        let keys = Series::from_values(
            "key",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Utf8("x".to_owned()), Scalar::Utf8("x".to_owned())],
        )
        .unwrap();

        let values = Series::from_values(
            "val",
            vec![0_i64.into(), 1_i64.into()],
            vec![Scalar::Null(NullKind::Null), Scalar::Int64(5)],
        )
        .unwrap();

        let count_result = groupby_count(&keys, &values, options, &policy, &mut ledger).unwrap();
        let size_result = groupby_size(&keys, &values, options, &policy, &mut ledger).unwrap();

        // count excludes nulls, size includes nulls
        assert_eq!(count_result.values()[0], Scalar::Int64(1)); // 1 non-null
        assert_eq!(size_result.values()[0], Scalar::Int64(2)); // 2 total
    }

    #[test]
    fn groupby_count_size_utf8_counter_path_preserves_null_and_order_semantics() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..6).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("c".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            (0..6).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(5),
                Scalar::Null(NullKind::Null),
            ],
        )
        .unwrap();

        let count = groupby_count(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        let size = groupby_size(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(
            count.index().labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(
            count.values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(0)]
        );
        assert_eq!(
            size.values(),
            &[Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(1)]
        );

        let first_seen = GroupByOptions {
            dropna: true,
            sort: false,
        };
        let first_seen_size =
            groupby_size(&keys, &values, first_seen, &policy, &mut ledger).unwrap();
        assert_eq!(
            first_seen_size.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            first_seen_size.values(),
            &[Scalar::Int64(3), Scalar::Int64(2), Scalar::Int64(1)]
        );
    }

    #[test]
    fn groupby_mean_utf8_counter_path_preserves_null_and_order_semantics() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..6).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("c".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "val",
            (0..6).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(5),
                Scalar::Null(NullKind::Null),
            ],
        )
        .unwrap();

        let mean = groupby_mean(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(mean.index().labels(), &["a".into(), "b".into(), "c".into()]);
        assert_eq!(
            mean.values(),
            &[
                Scalar::Float64(5.0),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN)
            ]
        );

        let first_seen = GroupByOptions {
            dropna: true,
            sort: false,
        };
        let first_seen_mean =
            groupby_mean(&keys, &values, first_seen, &policy, &mut ledger).unwrap();
        assert_eq!(
            first_seen_mean.index().labels(),
            &["b".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            first_seen_mean.values(),
            &[
                Scalar::Float64(2.0),
                Scalar::Float64(5.0),
                Scalar::Null(NullKind::NaN)
            ]
        );
    }

    #[test]
    fn groupby_mean_utf8_counter_path_preserves_timedelta_fallback() {
        let policy = RuntimePolicy::default();
        let mut ledger = EvidenceLedger::new();
        let keys = Series::from_values(
            "key",
            (0..4).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ],
        )
        .unwrap();
        let values = Series::from_values(
            "td",
            (0..4).map(|i| (i as i64).into()).collect(),
            vec![
                Scalar::Timedelta64(10),
                Scalar::Timedelta64(20),
                Scalar::Timedelta64(30),
                Scalar::Null(NullKind::NaT),
            ],
        )
        .unwrap();

        let mean = groupby_mean(
            &keys,
            &values,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(mean.index().labels(), &["a".into(), "b".into()]);
        assert_eq!(
            mean.values(),
            &[Scalar::Timedelta64(15), Scalar::Timedelta64(30)]
        );
    }
}
