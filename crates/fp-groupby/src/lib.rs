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
        let remaining = if self.p >= 64 {
            0
        } else {
            (hash << self.p) | (1_u64 << (self.p - 1))
        };
        let rho = remaining.leading_zeros() as u8 + 1;
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
                .map(|&r| f64::from(2_u32.pow(u32::from(r))).recip())
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

        // Promote every other element to the next level; discard the rest.
        // Standard KLL: compactor is cleared after compaction.
        let promoted: Vec<f64> = self.compactors[level]
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(i, v)| if i % 2 == offset { Some(v) } else { None })
            .collect();

        self.compactors[level].clear();

        // Ensure next level exists.
        if level + 1 >= self.compactors.len() {
            self.compactors.push(Vec::with_capacity(self.k * 2));
        }
        self.compactors[level + 1].extend(promoted);

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

    // === AG-12: Sketching / Streaming Aggregation Tests ===

    mod sketch_tests {
        use super::super::*;
        use fp_types::{NullKind, Scalar};

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
}
