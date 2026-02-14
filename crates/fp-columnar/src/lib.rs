#![forbid(unsafe_code)]

use fp_types::{DType, NullKind, Scalar, TypeError, cast_scalar_owned, common_dtype, infer_dtype};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Eq)]
pub struct ValidityMask {
    words: Vec<u64>,
    len: usize,
}

impl ValidityMask {
    #[must_use]
    pub fn from_values(values: &[Scalar]) -> Self {
        let len = values.len();
        let word_count = len.div_ceil(64);
        let mut words = vec![0_u64; word_count];
        for (idx, value) in values.iter().enumerate() {
            if !value.is_missing() {
                words[idx / 64] |= 1_u64 << (idx % 64);
            }
        }
        Self { words, len }
    }

    #[must_use]
    pub fn all_valid(len: usize) -> Self {
        let word_count = len.div_ceil(64);
        let mut words = vec![u64::MAX; word_count];
        let remainder = len % 64;
        if remainder > 0 && !words.is_empty() {
            let last = words.len() - 1;
            words[last] = (1_u64 << remainder) - 1;
        }
        Self { words, len }
    }

    #[must_use]
    pub fn all_invalid(len: usize) -> Self {
        let word_count = len.div_ceil(64);
        Self {
            words: vec![0_u64; word_count],
            len,
        }
    }

    #[must_use]
    pub fn get(&self, idx: usize) -> bool {
        if idx >= self.len {
            return false;
        }
        (self.words[idx / 64] >> (idx % 64)) & 1 == 1
    }

    pub fn set(&mut self, idx: usize, value: bool) {
        if idx >= self.len {
            return;
        }
        if value {
            self.words[idx / 64] |= 1_u64 << (idx % 64);
        } else {
            self.words[idx / 64] &= !(1_u64 << (idx % 64));
        }
    }

    #[must_use]
    pub fn count_valid(&self) -> usize {
        let full_words = self.len / 64;
        let mut count: u32 = self.words[..full_words]
            .iter()
            .map(|w| w.count_ones())
            .sum();
        let remainder = self.len % 64;
        if remainder > 0 && full_words < self.words.len() {
            let mask = (1_u64 << remainder) - 1;
            count += (self.words[full_words] & mask).count_ones();
        }
        count as usize
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn and_mask(&self, other: &Self) -> Self {
        let len = self.len.min(other.len);
        let word_count = len.div_ceil(64);
        let words = self.words[..word_count]
            .iter()
            .zip(&other.words[..word_count])
            .map(|(a, b)| a & b)
            .collect();
        Self { words, len }
    }

    #[must_use]
    pub fn or_mask(&self, other: &Self) -> Self {
        let len = self.len.min(other.len);
        let word_count = len.div_ceil(64);
        let words = self.words[..word_count]
            .iter()
            .zip(&other.words[..word_count])
            .map(|(a, b)| a | b)
            .collect();
        Self { words, len }
    }

    #[must_use]
    pub fn not_mask(&self) -> Self {
        let mut words: Vec<u64> = self.words.iter().map(|w| !w).collect();
        let remainder = self.len % 64;
        if remainder > 0 && !words.is_empty() {
            let last = words.len() - 1;
            words[last] &= (1_u64 << remainder) - 1;
        }
        Self {
            words,
            len: self.len,
        }
    }

    /// Returns an iterator yielding bool values, compatible with the previous
    /// `&[bool]` API. Materializes from the packed representation.
    pub fn bits(&self) -> impl Iterator<Item = bool> + '_ {
        (0..self.len).map(|idx| self.get(idx))
    }
}

impl PartialEq for ValidityMask {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.bits().eq(other.bits())
    }
}

impl Serialize for ValidityMask {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let bits: Vec<bool> = self.bits().collect();
        let mut state = serializer.serialize_struct("ValidityMask", 1)?;
        state.serialize_field("bits", &bits)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ValidityMask {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            bits: Vec<bool>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let len = raw.bits.len();
        let word_count = len.div_ceil(64);
        let mut words = vec![0_u64; word_count];
        for (idx, &valid) in raw.bits.iter().enumerate() {
            if valid {
                words[idx / 64] |= 1_u64 << (idx % 64);
            }
        }
        Ok(Self { words, len })
    }
}

/// AG-10: Typed array representation for vectorized batch execution.
///
/// Stores column data as contiguous typed arrays rather than `Vec<Scalar>`.
/// Validity is tracked by `ValidityMask`; invalid positions hold unspecified
/// values in the typed array (callers must check validity before reading).
///
/// This eliminates per-element enum dispatch for arithmetic operations,
/// enabling SIMD auto-vectorization on `&[f64]` / `&[i64]` slices.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    Float64(Vec<f64>),
    Int64(Vec<i64>),
    Bool(Vec<bool>),
    Utf8(Vec<String>),
}

impl ColumnData {
    /// Materialize typed array from a `Vec<Scalar>` and its `ValidityMask`.
    ///
    /// Invalid positions get a default sentinel (0 / 0.0 / false / "").
    /// The caller must pair this with the corresponding `ValidityMask` to
    /// interpret which positions are actually valid.
    #[must_use]
    pub fn from_scalars(values: &[Scalar], dtype: DType) -> Self {
        match dtype {
            DType::Float64 => {
                let data: Vec<f64> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Float64(f) => *f,
                        Scalar::Int64(i) => *i as f64,
                        Scalar::Bool(b) => {
                            if *b {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        _ => 0.0, // sentinel for invalid positions
                    })
                    .collect();
                Self::Float64(data)
            }
            DType::Int64 => {
                let data: Vec<i64> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Int64(i) => *i,
                        Scalar::Bool(b) => i64::from(*b),
                        _ => 0, // sentinel for invalid positions
                    })
                    .collect();
                Self::Int64(data)
            }
            DType::Bool => {
                let data: Vec<bool> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Bool(b) => *b,
                        _ => false,
                    })
                    .collect();
                Self::Bool(data)
            }
            DType::Utf8 => {
                let data: Vec<String> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Utf8(s) => s.clone(),
                        _ => String::new(),
                    })
                    .collect();
                Self::Utf8(data)
            }
            DType::Null => Self::Float64(vec![0.0; values.len()]),
        }
    }

    /// Convert typed array back to `Vec<Scalar>`, respecting `ValidityMask`.
    #[must_use]
    pub fn to_scalars(&self, dtype: DType, validity: &ValidityMask) -> Vec<Scalar> {
        match self {
            Self::Float64(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) {
                        Scalar::missing_for_dtype(dtype)
                    } else {
                        Scalar::Float64(*v)
                    }
                })
                .collect(),
            Self::Int64(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) {
                        Scalar::missing_for_dtype(dtype)
                    } else {
                        Scalar::Int64(*v)
                    }
                })
                .collect(),
            Self::Bool(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) {
                        Scalar::missing_for_dtype(dtype)
                    } else {
                        Scalar::Bool(*v)
                    }
                })
                .collect(),
            Self::Utf8(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) {
                        Scalar::missing_for_dtype(dtype)
                    } else {
                        Scalar::Utf8(v.clone())
                    }
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Float64(d) => d.len(),
            Self::Int64(d) => d.len(),
            Self::Bool(d) => d.len(),
            Self::Utf8(d) => d.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// AG-10: Vectorized binary arithmetic on `&[f64]` slices.
///
/// Both inputs must have the same length. The combined validity mask
/// determines which positions produce a valid output; invalid positions
/// get 0.0 as a sentinel. Returns `(result_data, result_validity)`.
fn vectorized_binary_f64(
    left: &[f64],
    right: &[f64],
    left_validity: &ValidityMask,
    right_validity: &ValidityMask,
    op: ArithmeticOp,
) -> (Vec<f64>, ValidityMask) {
    let combined = left_validity.and_mask(right_validity);

    // Zip iterators over contiguous slices — auto-vectorizable by LLVM.
    let apply: fn(f64, f64) -> f64 = match op {
        ArithmeticOp::Add => |a, b| a + b,
        ArithmeticOp::Sub => |a, b| a - b,
        ArithmeticOp::Mul => |a, b| a * b,
        ArithmeticOp::Div => |a, b| a / b,
    };

    let out: Vec<f64> = left
        .iter()
        .zip(right.iter())
        .enumerate()
        .map(|(i, (&l, &r))| {
            if combined.get(i) {
                apply(l, r)
            } else {
                0.0 // sentinel for invalid positions
            }
        })
        .collect();

    (out, combined)
}

/// AG-10: Vectorized binary arithmetic on `&[i64]` slices.
///
/// Produces `i64` results for Add/Sub/Mul. For Div, returns `None`
/// to signal the caller should use the `f64` path instead.
fn vectorized_binary_i64(
    left: &[i64],
    right: &[i64],
    left_validity: &ValidityMask,
    right_validity: &ValidityMask,
    op: ArithmeticOp,
) -> Option<(Vec<i64>, ValidityMask)> {
    // Division always promotes to Float64 in pandas semantics.
    if matches!(op, ArithmeticOp::Div) {
        return None;
    }

    let combined = left_validity.and_mask(right_validity);

    let apply: fn(i64, i64) -> i64 = match op {
        ArithmeticOp::Add => |a, b| a.wrapping_add(b),
        ArithmeticOp::Sub => |a, b| a.wrapping_sub(b),
        ArithmeticOp::Mul => |a, b| a.wrapping_mul(b),
        ArithmeticOp::Div => unreachable!(),
    };

    let out: Vec<i64> = left
        .iter()
        .zip(right.iter())
        .enumerate()
        .map(|(i, (&l, &r))| {
            if combined.get(i) {
                apply(l, r)
            } else {
                0 // sentinel for invalid positions
            }
        })
        .collect();

    Some((out, combined))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    dtype: DType,
    values: Vec<Scalar>,
    validity: ValidityMask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArithmeticOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ColumnError {
    #[error("column length mismatch: left={left}, right={right}")]
    LengthMismatch { left: usize, right: usize },
    #[error(transparent)]
    Type(#[from] TypeError),
}

impl Column {
    /// Construct a column, coercing values to the target dtype.
    /// AG-03: takes ownership of the values vec and uses `cast_scalar_owned`
    /// to skip cloning when values already have the correct dtype.
    pub fn new(dtype: DType, values: Vec<Scalar>) -> Result<Self, ColumnError> {
        let needs_coercion = values.iter().any(|v| {
            let d = v.dtype();
            d != dtype && d != DType::Null
        });

        let coerced = if needs_coercion {
            values
                .into_iter()
                .map(|value| cast_scalar_owned(value, dtype))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            // No coercion needed: values already match dtype.
            // Only remap Null variants to the dtype-specific missing marker.
            values
                .into_iter()
                .map(|value| match value {
                    Scalar::Null(_) => Scalar::missing_for_dtype(dtype),
                    other => other,
                })
                .collect()
        };

        let validity = ValidityMask::from_values(&coerced);

        Ok(Self {
            dtype,
            values: coerced,
            validity,
        })
    }

    pub fn from_values(values: Vec<Scalar>) -> Result<Self, ColumnError> {
        let dtype = infer_dtype(&values)?;
        Self::new(dtype, values)
    }

    #[must_use]
    pub fn dtype(&self) -> DType {
        self.dtype
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn values(&self) -> &[Scalar] {
        &self.values
    }

    #[must_use]
    pub fn value(&self, idx: usize) -> Option<&Scalar> {
        self.values.get(idx)
    }

    #[must_use]
    pub fn validity(&self) -> &ValidityMask {
        &self.validity
    }

    pub fn reindex_by_positions(&self, positions: &[Option<usize>]) -> Result<Self, ColumnError> {
        let values = positions
            .iter()
            .map(|slot| match slot {
                Some(idx) => self
                    .values
                    .get(*idx)
                    .cloned()
                    .unwrap_or_else(|| Scalar::missing_for_dtype(self.dtype)),
                None => Scalar::missing_for_dtype(self.dtype),
            })
            .collect::<Vec<_>>();

        Self::new(self.dtype, values)
    }

    /// AG-10: Attempt vectorized typed-array path for binary arithmetic.
    ///
    /// Preconditions: both columns same length, out_dtype already computed.
    /// Returns `Some(Column)` if vectorized path succeeded, `None` to
    /// signal fallback to the scalar path.
    fn try_vectorized_binary(
        &self,
        right: &Self,
        op: ArithmeticOp,
        out_dtype: DType,
    ) -> Option<Result<Self, ColumnError>> {
        // Vectorized path: both sides same numeric dtype, no NaN-vs-Null
        // distinction needed (i.e. both Int64, or both Float64 / promoted to Float64).
        match out_dtype {
            DType::Float64 => {
                let left_data = ColumnData::from_scalars(&self.values, DType::Float64);
                let right_data = ColumnData::from_scalars(&right.values, DType::Float64);
                let (ColumnData::Float64(l), ColumnData::Float64(r)) = (&left_data, &right_data)
                else {
                    return None;
                };

                // We need NaN-aware validity: original validity + NaN propagation.
                // Build validity masks that treat NaN source scalars as invalid.
                let left_nan_aware = self.nan_aware_validity();
                let right_nan_aware = right.nan_aware_validity();

                let (result_data, result_validity) =
                    vectorized_binary_f64(l, r, &left_nan_aware, &right_nan_aware, op);

                // Build output scalars respecting NaN propagation: if either
                // input was NaN (not just Null), mark output as Null(NaN).
                let values: Vec<Scalar> = result_data
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        if !result_validity.get(i) {
                            // Preserve NaN vs Null distinction from inputs.
                            if self.is_nan_at(i) || right.is_nan_at(i) {
                                Scalar::Null(NullKind::NaN)
                            } else {
                                Scalar::missing_for_dtype(out_dtype)
                            }
                        } else {
                            Scalar::Float64(*v)
                        }
                    })
                    .collect();

                Some(Self::new(out_dtype, values))
            }
            DType::Int64 if !matches!(op, ArithmeticOp::Div) => {
                // Both must actually be Int64 for the i64 fast path.
                if self.dtype != DType::Int64 || right.dtype != DType::Int64 {
                    return None;
                }
                let left_data = ColumnData::from_scalars(&self.values, DType::Int64);
                let right_data = ColumnData::from_scalars(&right.values, DType::Int64);
                let (ColumnData::Int64(l), ColumnData::Int64(r)) = (&left_data, &right_data) else {
                    return None;
                };

                let (result_data, result_validity) =
                    vectorized_binary_i64(l, r, &self.validity, &right.validity, op)?;

                let values: Vec<Scalar> = result_data
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        if !result_validity.get(i) {
                            Scalar::missing_for_dtype(out_dtype)
                        } else {
                            Scalar::Int64(*v)
                        }
                    })
                    .collect();

                Some(Self::new(out_dtype, values))
            }
            _ => None, // Bool, Utf8, etc. — use scalar fallback
        }
    }

    /// Validity mask that also marks NaN float values as invalid.
    #[must_use]
    fn nan_aware_validity(&self) -> ValidityMask {
        let mut mask = self.validity.clone();
        for (i, v) in self.values.iter().enumerate() {
            if matches!(v, Scalar::Float64(f) if f.is_nan()) {
                mask.set(i, false);
            }
        }
        mask
    }

    /// Check if position `i` holds a NaN-class missing value.
    fn is_nan_at(&self, i: usize) -> bool {
        self.values.get(i).is_some_and(|v| v.is_nan())
    }

    pub fn binary_numeric(&self, right: &Self, op: ArithmeticOp) -> Result<Self, ColumnError> {
        if self.len() != right.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        }

        let mut out_dtype = common_dtype(self.dtype, right.dtype)?;
        if matches!(out_dtype, DType::Bool) {
            out_dtype = DType::Int64;
        }
        if matches!(op, ArithmeticOp::Div) {
            out_dtype = DType::Float64;
        }

        // AG-10: Try vectorized path first; fallback to scalar path.
        if let Some(result) = self.try_vectorized_binary(right, op, out_dtype) {
            return result;
        }

        // Scalar fallback path (original implementation).
        let values = self
            .values
            .iter()
            .zip(&right.values)
            .map(|(left, right)| {
                if left.is_missing() || right.is_missing() {
                    return Ok::<_, ColumnError>(if left.is_nan() || right.is_nan() {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::missing_for_dtype(out_dtype)
                    });
                }

                let lhs = left.to_f64()?;
                let rhs = right.to_f64()?;
                let result = match op {
                    ArithmeticOp::Add => lhs + rhs,
                    ArithmeticOp::Sub => lhs - rhs,
                    ArithmeticOp::Mul => lhs * rhs,
                    ArithmeticOp::Div => lhs / rhs,
                };

                if matches!(out_dtype, DType::Int64)
                    && result.is_finite()
                    && result == result.trunc()
                    && result >= i64::MIN as f64
                    && result <= i64::MAX as f64
                {
                    Ok(Scalar::Int64(result as i64))
                } else {
                    Ok(Scalar::Float64(result))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(out_dtype, values)
    }

    #[must_use]
    pub fn semantic_eq(&self, other: &Self) -> bool {
        self.dtype == other.dtype
            && self.values.len() == other.values.len()
            && self
                .values
                .iter()
                .zip(&other.values)
                .all(|(left, right)| left.semantic_eq(right))
    }
}

// ---------------------------------------------------------------------------
// AG-14: Database Cracking — Adaptive Column Sorting
// ---------------------------------------------------------------------------

/// Adaptive crack index for progressive column partitioning.
///
/// Maintains a permutation of row indices and a sorted set of crack points.
/// Each filter operation partitions the relevant region around the predicate
/// pivot, progressively sorting the column across repeated queries.
///
/// Only works with numeric columns (values convertible to f64).
///
/// # Example
/// ```ignore
/// let mut crack = CrackIndex::new(column.len());
/// let gt5 = crack.filter_gt(&column, 5.0);  // partitions around 5.0
/// let gt3 = crack.filter_gt(&column, 3.0);  // refines: only re-scans [0, 5.0] region
/// ```
pub struct CrackIndex {
    /// Permuted row indices. Between consecutive crack points,
    /// elements are unsorted but bounded by the crack values.
    perm: Vec<usize>,
    /// Sorted crack points: (pivot_value, split_position_in_perm).
    /// All perm[..split] map to values <= pivot, perm[split..] map to values > pivot
    /// (within the containing region).
    cracks: Vec<(f64, usize)>,
}

impl CrackIndex {
    /// Create a new crack index for a column of `len` rows.
    #[must_use]
    pub fn new(len: usize) -> Self {
        Self {
            perm: (0..len).collect(),
            cracks: Vec::new(),
        }
    }

    /// Number of crack points recorded so far.
    #[must_use]
    pub fn num_cracks(&self) -> usize {
        self.cracks.len()
    }

    /// Return row indices where `column[row] > value`.
    pub fn filter_gt(&mut self, column: &Column, value: f64) -> Vec<usize> {
        let split = self.crack_at(column, value);
        self.perm[split..].to_vec()
    }

    /// Return row indices where `column[row] <= value`.
    pub fn filter_lte(&mut self, column: &Column, value: f64) -> Vec<usize> {
        let split = self.crack_at(column, value);
        self.perm[..split].to_vec()
    }

    /// Return row indices where `column[row] >= value`.
    pub fn filter_gte(&mut self, column: &Column, value: f64) -> Vec<usize> {
        // Crack just below value: use value - epsilon conceptually.
        // We crack at value, then scan the <= region for exact matches.
        let split = self.crack_at(column, value);
        // Everything in perm[split..] is > value.
        // Also include exact matches from perm[..split].
        let mut result: Vec<usize> = self.perm[split..].to_vec();
        for &idx in &self.perm[..split] {
            if let Some(v) = column.value(idx)
                && let Ok(f) = v.to_f64()
                && f == value
            {
                result.push(idx);
            }
        }
        result
    }

    /// Return row indices where `column[row] < value`.
    pub fn filter_lt(&mut self, column: &Column, value: f64) -> Vec<usize> {
        let split = self.crack_at(column, value);
        // perm[..split] has values <= value. Filter out exact matches.
        self.perm[..split]
            .iter()
            .copied()
            .filter(|&idx| {
                column
                    .value(idx)
                    .and_then(|v| v.to_f64().ok())
                    .is_some_and(|f| f < value)
            })
            .collect()
    }

    /// Return row indices where `column[row] == value`.
    pub fn filter_eq(&mut self, column: &Column, value: f64) -> Vec<usize> {
        let split = self.crack_at(column, value);
        // Exact matches are all in perm[..split] (the <= region).
        self.perm[..split]
            .iter()
            .copied()
            .filter(|&idx| {
                column
                    .value(idx)
                    .and_then(|v| v.to_f64().ok())
                    .is_some_and(|f| f == value)
            })
            .collect()
    }

    /// Ensure a crack point exists at `value`. Returns the split position
    /// such that perm[..split] are all <= value and perm[split..] are all > value.
    fn crack_at(&mut self, column: &Column, value: f64) -> usize {
        // Check if we already have this exact crack point.
        if let Ok(pos) = self.cracks.binary_search_by(|probe| {
            probe
                .0
                .partial_cmp(&value)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            return self.cracks[pos].1;
        }

        // Find the region to partition: between the nearest crack points.
        let (region_start, region_end) = self.find_region(value);

        // Partition perm[region_start..region_end] around `value`.
        // Move indices with column[idx] <= value to the left, > value to the right.
        let split = self.partition_region(column, region_start, region_end, value);

        // Insert the new crack point, maintaining sorted order.
        let insert_pos = self
            .cracks
            .binary_search_by(|probe| {
                probe
                    .0
                    .partial_cmp(&value)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|pos| pos);
        self.cracks.insert(insert_pos, (value, split));

        split
    }

    /// Find the region [start, end) in `perm` that contains `value`.
    fn find_region(&self, value: f64) -> (usize, usize) {
        let mut start = 0;
        let mut end = self.perm.len();

        for &(crack_val, crack_pos) in &self.cracks {
            if crack_val < value {
                start = start.max(crack_pos);
            } else {
                end = end.min(crack_pos);
                break;
            }
        }

        (start, end)
    }

    /// Partition perm[start..end] so that indices with column values <= pivot
    /// come first. Returns the split position (absolute index in perm).
    fn partition_region(&mut self, column: &Column, start: usize, end: usize, pivot: f64) -> usize {
        // Simple two-pointer partition (like quicksort partition).
        let region = &mut self.perm[start..end];
        let mut write = 0;

        for read in 0..region.len() {
            let idx = region[read];
            let val = column
                .value(idx)
                .and_then(|v| v.to_f64().ok())
                .unwrap_or(f64::NEG_INFINITY); // missing values sort to left

            if val <= pivot {
                region.swap(write, read);
                write += 1;
            }
        }

        start + write
    }
}

#[cfg(test)]
mod tests {
    use fp_types::{NullKind, Scalar};

    use super::{ArithmeticOp, Column, ValidityMask};

    #[test]
    fn reindex_injects_missing_values() {
        let column = Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)])
            .expect("column should build");

        let out = column
            .reindex_by_positions(&[Some(1), None, Some(0)])
            .expect("reindex should work");

        assert_eq!(
            out.values(),
            &[
                Scalar::Int64(20),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(10)
            ]
        );
    }

    #[test]
    fn numeric_addition_propagates_missing() {
        let left = Column::from_values(vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(f64::NAN),
        ])
        .expect("left");
        let right = Column::from_values(vec![Scalar::Int64(2), Scalar::Int64(5), Scalar::Int64(3)])
            .expect("right");

        let out = left
            .binary_numeric(&right, ArithmeticOp::Add)
            .expect("add should pass");

        assert_eq!(out.values()[0], Scalar::Float64(3.0));
        assert_eq!(out.values()[1], Scalar::Null(NullKind::NaN));
        assert_eq!(out.values()[2], Scalar::Null(NullKind::NaN));
    }

    // === Packed Bitvec ValidityMask Tests ===

    #[test]
    fn validity_mask_from_values_packs_correctly() {
        let values = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
        ];
        let mask = ValidityMask::from_values(&values);
        assert_eq!(mask.len(), 3);
        assert!(mask.get(0));
        assert!(!mask.get(1));
        assert!(mask.get(2));
        assert_eq!(mask.count_valid(), 2);
    }

    #[test]
    fn validity_mask_all_valid() {
        let mask = ValidityMask::all_valid(100);
        assert_eq!(mask.len(), 100);
        assert_eq!(mask.count_valid(), 100);
        for i in 0..100 {
            assert!(mask.get(i), "bit {i} should be valid");
        }
    }

    #[test]
    fn validity_mask_all_invalid() {
        let mask = ValidityMask::all_invalid(100);
        assert_eq!(mask.len(), 100);
        assert_eq!(mask.count_valid(), 0);
        for i in 0..100 {
            assert!(!mask.get(i), "bit {i} should be invalid");
        }
    }

    #[test]
    fn validity_mask_set_and_get() {
        let mut mask = ValidityMask::all_invalid(128);
        mask.set(0, true);
        mask.set(63, true);
        mask.set(64, true);
        mask.set(127, true);
        assert!(mask.get(0));
        assert!(mask.get(63));
        assert!(mask.get(64));
        assert!(mask.get(127));
        assert!(!mask.get(1));
        assert_eq!(mask.count_valid(), 4);

        mask.set(63, false);
        assert!(!mask.get(63));
        assert_eq!(mask.count_valid(), 3);
    }

    #[test]
    fn validity_mask_and_or_not() {
        let mut a = ValidityMask::all_invalid(4);
        a.set(0, true);
        a.set(1, true);

        let mut b = ValidityMask::all_invalid(4);
        b.set(1, true);
        b.set(2, true);

        let and = a.and_mask(&b);
        assert!(and.get(1));
        assert!(!and.get(0));
        assert!(!and.get(2));
        assert_eq!(and.count_valid(), 1);

        let or = a.or_mask(&b);
        assert!(or.get(0));
        assert!(or.get(1));
        assert!(or.get(2));
        assert!(!or.get(3));
        assert_eq!(or.count_valid(), 3);

        let not_a = a.not_mask();
        assert!(!not_a.get(0));
        assert!(!not_a.get(1));
        assert!(not_a.get(2));
        assert!(not_a.get(3));
        assert_eq!(not_a.count_valid(), 2);
    }

    #[test]
    fn validity_mask_bits_iterator() {
        let values = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
            Scalar::Float64(f64::NAN),
        ];
        let mask = ValidityMask::from_values(&values);
        let bits: Vec<bool> = mask.bits().collect();
        assert_eq!(bits, vec![true, false, true, false]);
    }

    #[test]
    fn validity_mask_serde_round_trip() {
        let values = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
        ];
        let mask = ValidityMask::from_values(&values);
        let json = serde_json::to_string(&mask).expect("serialize");
        let back: ValidityMask = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(mask, back);
        // Verify backward-compatible format
        assert!(json.contains("\"bits\""), "should serialize as bits field");
    }

    #[test]
    fn validity_mask_empty() {
        let mask = ValidityMask::from_values(&[]);
        assert!(mask.is_empty());
        assert_eq!(mask.len(), 0);
        assert_eq!(mask.count_valid(), 0);
        assert_eq!(mask.bits().count(), 0);
    }

    #[test]
    fn validity_mask_boundary_65_elements() {
        let mut values = vec![Scalar::Int64(1); 65];
        values[64] = Scalar::Null(NullKind::Null);
        let mask = ValidityMask::from_values(&values);
        assert_eq!(mask.len(), 65);
        assert_eq!(mask.count_valid(), 64);
        assert!(mask.get(63));
        assert!(!mask.get(64));
    }

    #[test]
    fn validity_mask_equality() {
        let a = ValidityMask::from_values(&[Scalar::Int64(1), Scalar::Null(NullKind::Null)]);
        let b = ValidityMask::from_values(&[Scalar::Int64(1), Scalar::Null(NullKind::Null)]);
        let c = ValidityMask::from_values(&[Scalar::Null(NullKind::Null), Scalar::Int64(1)]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn validity_mask_nan_is_invalid() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Float64(f64::NAN),
            Scalar::Null(NullKind::NaN),
        ];
        let mask = ValidityMask::from_values(&values);
        assert!(mask.get(0));
        assert!(!mask.get(1), "Float64(NaN) should be invalid");
        assert!(!mask.get(2), "Null(NaN) should be invalid");
        assert_eq!(mask.count_valid(), 1);
    }

    #[test]
    fn validity_mask_dense_null_half() {
        let values: Vec<Scalar> = (0..1000)
            .map(|i| {
                if i % 2 == 0 {
                    Scalar::Int64(i)
                } else {
                    Scalar::Null(NullKind::Null)
                }
            })
            .collect();
        let mask = ValidityMask::from_values(&values);
        assert_eq!(mask.len(), 1000);
        assert_eq!(mask.count_valid(), 500);
    }

    // === AG-10: ColumnData and Vectorized Path Tests ===

    #[test]
    fn column_data_float64_roundtrip() {
        let values = vec![
            Scalar::Float64(1.5),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
        ];
        let validity = ValidityMask::from_values(&values);
        let data = super::ColumnData::from_scalars(&values, fp_types::DType::Float64);
        let back = data.to_scalars(fp_types::DType::Float64, &validity);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0], Scalar::Float64(1.5));
        assert!(back[1].is_nan(), "position 1 should be NaN-missing");
        assert_eq!(back[2], Scalar::Float64(3.0));
    }

    #[test]
    fn column_data_int64_roundtrip() {
        let values = vec![
            Scalar::Int64(10),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(30),
        ];
        let validity = ValidityMask::from_values(&values);
        let data = super::ColumnData::from_scalars(&values, fp_types::DType::Int64);
        assert_eq!(data.len(), 3);
        let back = data.to_scalars(fp_types::DType::Int64, &validity);
        assert_eq!(back[0], Scalar::Int64(10));
        assert!(back[1].is_missing());
        assert_eq!(back[2], Scalar::Int64(30));
    }

    #[test]
    fn vectorized_f64_addition_matches_scalar() {
        let left = Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(30.0),
        ])
        .expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        assert_eq!(result.values()[0], Scalar::Float64(11.0));
        assert_eq!(result.values()[1], Scalar::Float64(22.0));
        assert_eq!(result.values()[2], Scalar::Float64(33.0));
    }

    #[test]
    fn vectorized_i64_addition_matches_scalar() {
        let left = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
            .expect("left");
        let right = Column::from_values(vec![
            Scalar::Int64(10),
            Scalar::Int64(20),
            Scalar::Int64(30),
        ])
        .expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        assert_eq!(result.values()[0], Scalar::Int64(11));
        assert_eq!(result.values()[1], Scalar::Int64(22));
        assert_eq!(result.values()[2], Scalar::Int64(33));
    }

    #[test]
    fn vectorized_f64_with_nulls_propagates_missing() {
        let left = Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Null(NullKind::NaN),
        ])
        .expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        assert_eq!(result.values()[0], Scalar::Float64(11.0));
        assert!(result.values()[1].is_nan(), "null+valid should be NaN");
        assert!(result.values()[2].is_nan(), "valid+null should be NaN");
    }

    #[test]
    fn vectorized_i64_with_nulls_propagates_missing() {
        let left = Column::from_values(vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Int64(10),
            Scalar::Int64(20),
            Scalar::Null(NullKind::Null),
        ])
        .expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        assert_eq!(result.values()[0], Scalar::Int64(11));
        assert!(result.values()[1].is_missing());
        assert!(result.values()[2].is_missing());
    }

    #[test]
    fn vectorized_division_promotes_to_float64() {
        let left = Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(21)]).expect("left");
        let right = Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(7)]).expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Div).expect("div");
        // Division always promotes to Float64.
        assert_eq!(result.dtype(), fp_types::DType::Float64);
        assert!(matches!(result.values()[0], Scalar::Float64(v) if (v - 10.0/3.0).abs() < 1e-10));
        assert_eq!(result.values()[1], Scalar::Float64(3.0));
    }

    #[test]
    fn vectorized_all_four_ops_f64() {
        let left = Column::from_values(vec![Scalar::Float64(10.0)]).expect("left");
        let right = Column::from_values(vec![Scalar::Float64(3.0)]).expect("right");

        let add = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        let sub = left.binary_numeric(&right, ArithmeticOp::Sub).expect("sub");
        let mul = left.binary_numeric(&right, ArithmeticOp::Mul).expect("mul");
        let div = left.binary_numeric(&right, ArithmeticOp::Div).expect("div");

        assert_eq!(add.values()[0], Scalar::Float64(13.0));
        assert_eq!(sub.values()[0], Scalar::Float64(7.0));
        assert_eq!(mul.values()[0], Scalar::Float64(30.0));
        assert!(matches!(div.values()[0], Scalar::Float64(v) if (v - 10.0/3.0).abs() < 1e-10));
    }

    #[test]
    fn vectorized_empty_columns() {
        let left = Column::from_values(vec![]).expect("left");
        let right = Column::from_values(vec![]).expect("right");
        let result = left
            .binary_numeric(&right, ArithmeticOp::Add)
            .expect("add empty");
        assert!(result.is_empty());
    }

    #[test]
    fn vectorized_large_column_matches_scalar_semantics() {
        // Build large columns to exercise batch processing.
        let n = 4096;
        let left_values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
        let right_values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64((n - i) as f64)).collect();

        let left = Column::from_values(left_values).expect("left");
        let right = Column::from_values(right_values).expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");

        // Every position should sum to n.
        for (i, v) in result.values().iter().enumerate() {
            assert_eq!(*v, Scalar::Float64(n as f64), "position {i} should be {n}");
        }
    }

    #[test]
    fn vectorized_nan_vs_null_distinction_preserved() {
        // Float64 column: NaN is a specific kind of missing.
        let left =
            Column::from_values(vec![Scalar::Float64(f64::NAN), Scalar::Null(NullKind::NaN)])
                .expect("left");
        let right =
            Column::from_values(vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]).expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        // Both positions should be NaN-missing (not generic Null).
        assert!(result.values()[0].is_nan(), "NaN + valid = NaN");
        assert!(result.values()[1].is_nan(), "NaN-null + valid = NaN");
    }

    #[test]
    fn vectorized_mixed_type_falls_back_to_scalar() {
        // Int64 + Float64 promotes to Float64 — vectorized path handles this.
        let left = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("left");
        let right =
            Column::from_values(vec![Scalar::Float64(0.5), Scalar::Float64(1.5)]).expect("right");

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");
        assert_eq!(result.dtype(), fp_types::DType::Float64);
        assert_eq!(result.values()[0], Scalar::Float64(1.5));
        assert_eq!(result.values()[1], Scalar::Float64(3.5));
    }

    #[test]
    fn vectorized_i64_sub_and_mul() {
        let left = Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).expect("left");
        let right = Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(5)]).expect("right");

        let sub = left.binary_numeric(&right, ArithmeticOp::Sub).expect("sub");
        assert_eq!(sub.values()[0], Scalar::Int64(7));
        assert_eq!(sub.values()[1], Scalar::Int64(15));

        let mul = left.binary_numeric(&right, ArithmeticOp::Mul).expect("mul");
        assert_eq!(mul.values()[0], Scalar::Int64(30));
        assert_eq!(mul.values()[1], Scalar::Int64(100));
    }

    // === AG-14: Database Cracking Tests ===

    mod crack_tests {
        use super::super::*;
        use fp_types::Scalar;

        fn make_column(values: &[f64]) -> Column {
            Column::from_values(values.iter().map(|&v| Scalar::Float64(v)).collect()).expect("col")
        }

        #[test]
        fn crack_filter_gt_basic() {
            let col = make_column(&[1.0, 5.0, 3.0, 7.0, 2.0]);
            let mut crack = CrackIndex::new(col.len());

            let gt3 = crack.filter_gt(&col, 3.0);
            let mut gt3_vals: Vec<f64> = gt3
                .iter()
                .map(|&i| col.values()[i].to_f64().unwrap())
                .collect();
            gt3_vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(gt3_vals, vec![5.0, 7.0]);
            assert_eq!(crack.num_cracks(), 1);
        }

        #[test]
        fn crack_filter_lte_basic() {
            let col = make_column(&[1.0, 5.0, 3.0, 7.0, 2.0]);
            let mut crack = CrackIndex::new(col.len());

            let lte3 = crack.filter_lte(&col, 3.0);
            let mut lte3_vals: Vec<f64> = lte3
                .iter()
                .map(|&i| col.values()[i].to_f64().unwrap())
                .collect();
            lte3_vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(lte3_vals, vec![1.0, 2.0, 3.0]);
        }

        #[test]
        fn crack_filter_eq() {
            let col = make_column(&[1.0, 3.0, 3.0, 7.0, 3.0]);
            let mut crack = CrackIndex::new(col.len());

            let eq3 = crack.filter_eq(&col, 3.0);
            assert_eq!(eq3.len(), 3, "three values equal to 3.0");
            for &idx in &eq3 {
                assert_eq!(col.values()[idx].to_f64().unwrap(), 3.0);
            }
        }

        #[test]
        fn crack_filter_lt() {
            let col = make_column(&[1.0, 5.0, 3.0, 7.0, 2.0]);
            let mut crack = CrackIndex::new(col.len());

            let lt3 = crack.filter_lt(&col, 3.0);
            let mut lt3_vals: Vec<f64> = lt3
                .iter()
                .map(|&i| col.values()[i].to_f64().unwrap())
                .collect();
            lt3_vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(lt3_vals, vec![1.0, 2.0]);
        }

        #[test]
        fn crack_filter_gte() {
            let col = make_column(&[1.0, 5.0, 3.0, 7.0, 2.0]);
            let mut crack = CrackIndex::new(col.len());

            let gte3 = crack.filter_gte(&col, 3.0);
            let mut gte3_vals: Vec<f64> = gte3
                .iter()
                .map(|&i| col.values()[i].to_f64().unwrap())
                .collect();
            gte3_vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(gte3_vals, vec![3.0, 5.0, 7.0]);
        }

        #[test]
        fn crack_progressive_refinement() {
            let col = make_column(&[10.0, 2.0, 8.0, 4.0, 6.0, 1.0, 9.0, 3.0, 7.0, 5.0]);
            let mut crack = CrackIndex::new(col.len());

            // First crack at 5.0
            let gt5 = crack.filter_gt(&col, 5.0);
            assert_eq!(gt5.len(), 5);
            assert_eq!(crack.num_cracks(), 1);

            // Second crack at 3.0 — only re-partitions the [<=5.0] region
            let gt3 = crack.filter_gt(&col, 3.0);
            assert_eq!(gt3.len(), 7); // 4,5,6,7,8,9,10
            assert_eq!(crack.num_cracks(), 2);

            // Third crack at 7.0 — only re-partitions the [>5.0] region
            let gt7 = crack.filter_gt(&col, 7.0);
            assert_eq!(gt7.len(), 3); // 8,9,10
            assert_eq!(crack.num_cracks(), 3);
        }

        #[test]
        fn crack_duplicate_crack_point_is_idempotent() {
            let col = make_column(&[1.0, 5.0, 3.0, 7.0, 2.0]);
            let mut crack = CrackIndex::new(col.len());

            let gt3_first = crack.filter_gt(&col, 3.0);
            let gt3_second = crack.filter_gt(&col, 3.0);

            // Same results both times
            let mut a: Vec<usize> = gt3_first;
            let mut b: Vec<usize> = gt3_second;
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b);
            assert_eq!(crack.num_cracks(), 1, "no duplicate crack point");
        }

        #[test]
        fn crack_empty_column() {
            let col = make_column(&[]);
            let mut crack = CrackIndex::new(col.len());

            assert!(crack.filter_gt(&col, 5.0).is_empty());
            assert!(crack.filter_lte(&col, 5.0).is_empty());
        }

        #[test]
        fn crack_single_element() {
            let col = make_column(&[42.0]);
            let mut crack = CrackIndex::new(col.len());

            assert!(crack.filter_gt(&col, 42.0).is_empty());
            assert_eq!(crack.filter_lte(&col, 42.0).len(), 1);
            assert_eq!(crack.filter_eq(&col, 42.0).len(), 1);
        }

        #[test]
        fn crack_all_same_values() {
            let col = make_column(&[5.0, 5.0, 5.0, 5.0]);
            let mut crack = CrackIndex::new(col.len());

            assert!(crack.filter_gt(&col, 5.0).is_empty());
            assert_eq!(crack.filter_lte(&col, 5.0).len(), 4);
            assert_eq!(crack.filter_eq(&col, 5.0).len(), 4);
        }

        #[test]
        fn crack_isomorphism_with_full_scan() {
            // Cracked filter must return identical results to naive full scan.
            let col = make_column(&[10.0, 2.0, 8.0, 4.0, 6.0, 1.0, 9.0, 3.0, 7.0, 5.0]);
            let mut crack = CrackIndex::new(col.len());

            for pivot in [1.0, 3.0, 5.0, 7.0, 9.0, 0.0, 11.0] {
                let mut cracked: Vec<usize> = crack.filter_gt(&col, pivot);
                cracked.sort_unstable();

                let mut naive: Vec<usize> = (0..col.len())
                    .filter(|&i| col.values()[i].to_f64().unwrap() > pivot)
                    .collect();
                naive.sort_unstable();

                assert_eq!(
                    cracked, naive,
                    "cracked vs naive mismatch for pivot={pivot}"
                );
            }
        }

        #[test]
        fn crack_int64_column() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(5),
                Scalar::Int64(3),
                Scalar::Int64(8),
                Scalar::Int64(1),
            ])
            .expect("col");
            let mut crack = CrackIndex::new(col.len());

            let gt5 = crack.filter_gt(&col, 5.0);
            let mut gt5_vals: Vec<i64> = gt5
                .iter()
                .map(|&i| match &col.values()[i] {
                    Scalar::Int64(v) => *v,
                    _ => panic!("expected Int64"),
                })
                .collect();
            gt5_vals.sort_unstable();
            assert_eq!(gt5_vals, vec![8, 10]);
        }

        #[test]
        fn crack_large_column_correctness() {
            let n = 1000;
            let values: Vec<f64> = (0..n).map(|i| ((i * 7 + 13) % n) as f64).collect();
            let col = make_column(&values);
            let mut crack = CrackIndex::new(col.len());

            // Multiple cracks at different points
            for pivot in [100.0, 500.0, 250.0, 750.0, 50.0, 900.0] {
                let mut cracked: Vec<usize> = crack.filter_gt(&col, pivot);
                cracked.sort_unstable();

                let mut naive: Vec<usize> =
                    (0..n as usize).filter(|&i| values[i] > pivot).collect();
                naive.sort_unstable();

                assert_eq!(cracked, naive, "large column mismatch for pivot={pivot}");
            }
        }
    }
}
