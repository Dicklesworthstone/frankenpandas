#![forbid(unsafe_code)]

use fp_types::{
    DType, NullKind, Scalar, Timedelta, TypeError, cast_scalar, cast_scalar_owned, common_dtype,
    infer_dtype, nancummax, nancummin, nancumprod, nancumsum, nanmax, nanmean, nanmedian, nanmin,
    nanprod, nansum,
};
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
    Timedelta64(Vec<i64>),
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
                        Scalar::Bool(true) => 1.0,
                        Scalar::Bool(false) => 0.0,
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
            DType::Categorical => {
                let data: Vec<i64> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Int64(i) => *i,
                        _ => -1,
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
            DType::Timedelta64 => {
                let data: Vec<i64> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Timedelta64(n) => *n,
                        Scalar::Int64(i) => *i,
                        _ => Timedelta::NAT,
                    })
                    .collect();
                Self::Timedelta64(data)
            }
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
            Self::Timedelta64(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) || *v == Timedelta::NAT {
                        Scalar::Timedelta64(Timedelta::NAT)
                    } else {
                        Scalar::Timedelta64(*v)
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
            Self::Timedelta64(d) => d.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Compare two non-missing scalars using the given comparison operator.
///
/// Both scalars are converted to `f64` for comparison. For `Utf8` values,
/// lexicographic ordering is used. Returns `Err` for incompatible types.
fn scalar_compare(left: &Scalar, right: &Scalar, op: ComparisonOp) -> Result<bool, ColumnError> {
    // Coerce differing numeric types to avoid precision loss (e.g. Bool vs Int64).
    let left_dtype = left.dtype();
    let right_dtype = right.dtype();
    if left_dtype != right_dtype
        && let Ok(common) = fp_types::common_dtype(left_dtype, right_dtype)
        && common == DType::Int64
    {
        let l_cast = fp_types::cast_scalar(left, common)?;
        let r_cast = fp_types::cast_scalar(right, common)?;
        // Handle Int64 comparisons to avoid precision loss in f64 cast.
        if let (Scalar::Int64(a), Scalar::Int64(b)) = (&l_cast, &r_cast) {
            return Ok(match op {
                ComparisonOp::Gt => a > b,
                ComparisonOp::Lt => a < b,
                ComparisonOp::Eq => a == b,
                ComparisonOp::Ne => a != b,
                ComparisonOp::Ge => a >= b,
                ComparisonOp::Le => a <= b,
            });
        }
    }

    // Handle Utf8 comparisons separately (lexicographic).
    if let (Scalar::Utf8(a), Scalar::Utf8(b)) = (left, right) {
        return Ok(match op {
            ComparisonOp::Gt => a > b,
            ComparisonOp::Lt => a < b,
            ComparisonOp::Eq => a == b,
            ComparisonOp::Ne => a != b,
            ComparisonOp::Ge => a >= b,
            ComparisonOp::Le => a <= b,
        });
    }

    // Handle Bool comparisons (false < true).
    if let (Scalar::Bool(a), Scalar::Bool(b)) = (left, right) {
        return Ok(match op {
            ComparisonOp::Gt => *a && !*b,
            ComparisonOp::Lt => !*a && *b,
            ComparisonOp::Eq => a == b,
            ComparisonOp::Ne => a != b,
            ComparisonOp::Ge => *a >= *b,
            ComparisonOp::Le => *a <= *b,
        });
    }

    // Handle Int64 comparisons to avoid precision loss in f64 cast.
    if let (Scalar::Int64(a), Scalar::Int64(b)) = (left, right) {
        return Ok(match op {
            ComparisonOp::Gt => a > b,
            ComparisonOp::Lt => a < b,
            ComparisonOp::Eq => a == b,
            ComparisonOp::Ne => a != b,
            ComparisonOp::Ge => a >= b,
            ComparisonOp::Le => a <= b,
        });
    }

    // Numeric: convert both to f64.
    let lhs = left.to_f64()?;
    let rhs = right.to_f64()?;

    Ok(match op {
        ComparisonOp::Gt => lhs > rhs,
        ComparisonOp::Lt => lhs < rhs,
        ComparisonOp::Eq => lhs == rhs,
        ComparisonOp::Ne => lhs != rhs,
        ComparisonOp::Ge => lhs >= rhs,
        ComparisonOp::Le => lhs <= rhs,
    })
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
        ArithmeticOp::Mod => |a, b| a % b,
        ArithmeticOp::Pow => |a, b| a.powf(b),
        ArithmeticOp::FloorDiv => |a, b| (a / b).floor(),
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
    let combined = left_validity.and_mask(right_validity);

    // Div and Pow always produce floats
    if matches!(op, ArithmeticOp::Div | ArithmeticOp::Pow) {
        return None;
    }

    // For Mod/FloorDiv: if any valid position has zero divisor, fall back to float
    // (pandas promotes to float64 to represent NaN/inf for division by zero)
    if matches!(op, ArithmeticOp::Mod | ArithmeticOp::FloorDiv) {
        let has_zero_divisor = right
            .iter()
            .enumerate()
            .any(|(i, &r)| combined.get(i) && r == 0);
        if has_zero_divisor {
            return None;
        }
    }

    let apply: fn(i64, i64) -> i64 = match op {
        ArithmeticOp::Add => |a, b| a.wrapping_add(b),
        ArithmeticOp::Sub => |a, b| a.wrapping_sub(b),
        ArithmeticOp::Mul => |a, b| a.wrapping_mul(b),
        ArithmeticOp::Mod => |a, b| {
            if a == i64::MIN && b == -1 {
                0
            } else {
                a.rem_euclid(b)
            }
        },
        ArithmeticOp::FloorDiv => |a, b| {
            if a == i64::MIN && b == -1 {
                i64::MIN
            } else {
                a.div_euclid(b)
            }
        },
        ArithmeticOp::Div | ArithmeticOp::Pow => unreachable!("handled by early return above"),
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
    Mod,
    Pow,
    FloorDiv,
}

/// Element-wise comparison operations that produce `Bool`-typed columns.
///
/// Null propagation: any missing/NaN input produces a missing output.
/// This matches pandas nullable-integer semantics (`pd.NA` propagation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOp {
    Gt,
    Lt,
    Eq,
    Ne,
    Ge,
    Le,
}

fn compare_scalars_na_last(left: &Scalar, right: &Scalar, ascending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left.is_missing(), right.is_missing()) {
        (true, true) => Ordering::Equal,
        // Missing always sorts to the end, regardless of direction.
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let ord = match (left, right) {
                (Scalar::Int64(a), Scalar::Int64(b)) => a.cmp(b),
                (Scalar::Float64(a), Scalar::Float64(b)) => {
                    a.partial_cmp(b).unwrap_or(Ordering::Equal)
                }
                (Scalar::Bool(a), Scalar::Bool(b)) => a.cmp(b),
                (Scalar::Utf8(a), Scalar::Utf8(b)) => a.cmp(b),
                (Scalar::Timedelta64(a), Scalar::Timedelta64(b)) => a.cmp(b),
                (a, b) => match (a.to_f64(), b.to_f64()) {
                    (Ok(af), Ok(bf)) => af.partial_cmp(&bf).unwrap_or(Ordering::Equal),
                    _ => Ordering::Equal,
                },
            };
            if ascending { ord } else { ord.reverse() }
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ColumnError {
    #[error("column length mismatch: left={left}, right={right}")]
    LengthMismatch { left: usize, right: usize },
    #[error("mask must be Bool dtype; found {dtype:?}")]
    InvalidMaskType { dtype: DType },
    #[error("column dtype mismatch: left={left:?}, right={right:?}")]
    DTypeMismatch { left: DType, right: DType },
    #[error(transparent)]
    Type(#[from] TypeError),
}

fn saturating_i64_to_usize(value: i64) -> usize {
    if value <= 0 {
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

impl Column {
    fn normalize_missing_for_dtype(value: Scalar, dtype: DType) -> Scalar {
        match value {
            Scalar::Null(NullKind::NaN) => Scalar::Null(NullKind::NaN),
            Scalar::Null(NullKind::NaT) => Scalar::Null(NullKind::NaT),
            Scalar::Null(_) => Scalar::missing_for_dtype(dtype),
            other => other,
        }
    }

    /// Construct a column, coercing values to the target dtype.
    /// AG-03: takes ownership of the values vec and uses `cast_scalar_owned`
    /// to skip cloning when values already have the correct dtype.
    pub fn new(dtype: DType, values: Vec<Scalar>) -> Result<Self, ColumnError> {
        let preserve_utf8_object_bucket = matches!(dtype, DType::Utf8)
            && values.iter().any(|value| matches!(value, Scalar::Utf8(_)))
            && values
                .iter()
                .any(|value| !matches!(value, Scalar::Utf8(_) | Scalar::Null(_)));
        let needs_coercion = values.iter().any(|v| {
            let d = v.dtype();
            d != dtype && d != DType::Null
        }) && !preserve_utf8_object_bucket;

        let coerced = if preserve_utf8_object_bucket {
            values
                .into_iter()
                .map(|value| Self::normalize_missing_for_dtype(value, dtype))
                .collect()
        } else if needs_coercion {
            values
                .into_iter()
                .map(|value| cast_scalar_owned(value, dtype))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            // No coercion needed: values already match dtype.
            // Preserve explicit NaN/NaT markers; remap generic Null to dtype-specific missing.
            values
                .into_iter()
                .map(|value| Self::normalize_missing_for_dtype(value, dtype))
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
        // Div and Pow always produce Float64; Mod and FloorDiv preserve int if no zero divisors
        if matches!(op, ArithmeticOp::Div | ArithmeticOp::Pow) {
            out_dtype = DType::Float64;
        }

        // AG-10: Try vectorized path first; fallback to scalar path.
        if let Some(result) = self.try_vectorized_binary(right, op, out_dtype) {
            return result;
        }

        // For Mod/FloorDiv: if vectorized failed (likely due to zero divisors), use Float64
        if matches!(op, ArithmeticOp::Mod | ArithmeticOp::FloorDiv)
            && matches!(out_dtype, DType::Int64)
        {
            out_dtype = DType::Float64;
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

                if matches!(out_dtype, DType::Int64) {
                    let lhs_i64 = match cast_scalar(left, DType::Int64)? {
                        Scalar::Int64(v) => v,
                        _ => unreachable!(),
                    };
                    let rhs_i64 = match cast_scalar(right, DType::Int64)? {
                        Scalar::Int64(v) => v,
                        _ => unreachable!(),
                    };
                    let result = match op {
                        ArithmeticOp::Add => lhs_i64.wrapping_add(rhs_i64),
                        ArithmeticOp::Sub => lhs_i64.wrapping_sub(rhs_i64),
                        ArithmeticOp::Mul => lhs_i64.wrapping_mul(rhs_i64),
                        ArithmeticOp::Div
                        | ArithmeticOp::Mod
                        | ArithmeticOp::Pow
                        | ArithmeticOp::FloorDiv => unreachable!(),
                    };
                    return Ok(Scalar::Int64(result));
                }

                let lhs = left.to_f64()?;
                let rhs = right.to_f64()?;
                let result = match op {
                    ArithmeticOp::Add => lhs + rhs,
                    ArithmeticOp::Sub => lhs - rhs,
                    ArithmeticOp::Mul => lhs * rhs,
                    ArithmeticOp::Div => lhs / rhs,
                    ArithmeticOp::Mod => lhs % rhs,
                    ArithmeticOp::Pow => lhs.powf(rhs),
                    ArithmeticOp::FloorDiv => (lhs / rhs).floor(),
                };

                Ok(Scalar::Float64(result))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(out_dtype, values)
    }

    /// Element-wise comparison producing a `Bool`-typed column.
    ///
    /// Both columns must have the same length. Missing values (Null or NaN)
    /// propagate: if either operand is missing, the result is missing.
    pub fn binary_comparison(&self, right: &Self, op: ComparisonOp) -> Result<Self, ColumnError> {
        if self.len() != right.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        }

        let values = self
            .values
            .iter()
            .zip(&right.values)
            .map(|(l, r)| -> Result<Scalar, ColumnError> {
                if l.is_missing() || r.is_missing() {
                    return Ok(Scalar::Null(NullKind::Null));
                }
                let result = scalar_compare(l, r, op)?;
                Ok(Scalar::Bool(result))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(DType::Bool, values)
    }

    /// Compare every element against a scalar value, producing a `Bool`-typed column.
    ///
    /// Missing values in the column propagate as missing in the result.
    pub fn compare_scalar(&self, scalar: &Scalar, op: ComparisonOp) -> Result<Self, ColumnError> {
        if scalar.is_missing() {
            // Comparing against missing always produces all-missing.
            let values = vec![Scalar::Null(NullKind::Null); self.len()];
            return Self::new(DType::Bool, values);
        }

        let values = self
            .values
            .iter()
            .map(|v| -> Result<Scalar, ColumnError> {
                if v.is_missing() {
                    return Ok(Scalar::Null(NullKind::Null));
                }
                let result = scalar_compare(v, scalar, op)?;
                Ok(Scalar::Bool(result))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(DType::Bool, values)
    }

    /// Select elements where `mask` is `true`, producing a new column.
    ///
    /// The mask must be a `Bool`-typed column of the same length.
    /// Missing values in the mask are treated as `false` (not selected).
    pub fn filter_by_mask(&self, mask: &Self) -> Result<Self, ColumnError> {
        if mask.dtype != DType::Bool {
            return Err(ColumnError::InvalidMaskType { dtype: mask.dtype });
        }
        if self.len() != mask.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: mask.len(),
            });
        }

        let values = self
            .values
            .iter()
            .zip(mask.values.iter())
            .filter_map(|(val, mask_val)| match mask_val {
                Scalar::Bool(true) => Some(val.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        Self::new(self.dtype, values)
    }

    /// Fill missing values with a replacement scalar.
    ///
    /// Returns a new column where every missing position is replaced
    /// by `fill_value`. The fill value is cast to the column's dtype.
    pub fn fillna(&self, fill_value: &Scalar) -> Result<Self, ColumnError> {
        if self.dtype == DType::Null {
            let replacement_dtype = if fill_value.is_missing() {
                DType::Null
            } else {
                fill_value.dtype()
            };
            let values = self
                .values
                .iter()
                .map(|value| {
                    if value.is_missing() {
                        fill_value.clone()
                    } else {
                        value.clone()
                    }
                })
                .collect();
            return Self::new(replacement_dtype, values);
        }

        let cast_fill = cast_scalar(fill_value, self.dtype)?;
        let values = self
            .values
            .iter()
            .map(|v| {
                if v.is_missing() {
                    cast_fill.clone()
                } else {
                    v.clone()
                }
            })
            .collect();

        Self::new(self.dtype, values)
    }

    /// Remove missing values, returning a shorter column.
    pub fn dropna(&self) -> Result<Self, ColumnError> {
        let values = self
            .values
            .iter()
            .filter(|v| !v.is_missing())
            .cloned()
            .collect();

        Self::new(self.dtype, values)
    }

    /// Gather rows by integer position.
    ///
    /// Matches `pd.Series.take(indices)`. Each index must fall within
    /// `0..len()`; out-of-range positions return
    /// `ColumnError::LengthMismatch` (left=length, right=offending
    /// index).
    pub fn take(&self, indices: &[usize]) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(indices.len());
        for &i in indices {
            match self.values.get(i) {
                Some(v) => out.push(v.clone()),
                None => {
                    return Err(ColumnError::LengthMismatch {
                        left: self.values.len(),
                        right: i,
                    });
                }
            }
        }
        Self::new(self.dtype, out)
    }

    /// Contiguous slice by positional range `start..start+len`.
    ///
    /// Out-of-range requests are clamped to the available tail so a
    /// start past `len()` yields an empty column with the same dtype,
    /// matching pandas' permissive slice semantics.
    pub fn slice(&self, start: usize, len: usize) -> Result<Self, ColumnError> {
        if start >= self.values.len() {
            return Self::new(self.dtype, Vec::new());
        }
        let end = (start + len).min(self.values.len());
        let values = self.values[start..end].to_vec();
        Self::new(self.dtype, values)
    }

    /// Return the first `n` values.
    ///
    /// Matches pandas `head(n)` semantics on a 1-D array-like surface.
    /// Negative `n` returns all values except the last `-n`.
    pub fn head(&self, n: i64) -> Result<Self, ColumnError> {
        let take = normalize_head_take(n, self.len());
        self.slice(0, take)
    }

    /// Return the last `n` values.
    ///
    /// Matches pandas `tail(n)` semantics on a 1-D array-like surface.
    /// Negative `n` returns all values except the first `-n`.
    pub fn tail(&self, n: i64) -> Result<Self, ColumnError> {
        let (start, len) = normalize_tail_window(n, self.len());
        self.slice(start, len)
    }

    /// Concatenate `other` onto `self`, preserving dtype.
    ///
    /// Returns `ColumnError::DTypeMismatch` when `other.dtype()` differs
    /// from `self.dtype()`.
    pub fn concat(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.dtype != other.dtype {
            return Err(ColumnError::DTypeMismatch {
                left: self.dtype,
                right: other.dtype,
            });
        }
        let mut values = Vec::with_capacity(self.values.len() + other.values.len());
        values.extend_from_slice(&self.values);
        values.extend_from_slice(&other.values);
        Self::new(self.dtype, values)
    }

    /// Repeat each value `repeats` times contiguously.
    ///
    /// Matches `pd.Series.repeat(n)`. `repeats=0` yields an empty
    /// column; `repeats=1` is a clone.
    pub fn repeat(&self, repeats: usize) -> Result<Self, ColumnError> {
        if repeats == 0 {
            return Self::new(self.dtype, Vec::new());
        }
        if repeats == 1 {
            return Ok(self.clone());
        }
        let mut out = Vec::with_capacity(self.values.len() * repeats);
        for v in &self.values {
            for _ in 0..repeats {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Reverse the row order of the column.
    ///
    /// Matches `pd.Series[::-1]` / `iloc[::-1]`. Dtype is preserved.
    pub fn reverse(&self) -> Result<Self, ColumnError> {
        let mut values = self.values.clone();
        values.reverse();
        Self::new(self.dtype, values)
    }

    /// Cumulative sum, null-propagating per fp-types::nancumsum.
    ///
    /// Matches `pd.Series.cumsum()`. The resulting column is always
    /// Float64 (matching the numeric accumulator type used in
    /// nancumsum).
    pub fn cumsum(&self) -> Result<Self, ColumnError> {
        let out = nancumsum(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Cumulative product, null-propagating per fp-types::nancumprod.
    pub fn cumprod(&self) -> Result<Self, ColumnError> {
        let out = nancumprod(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Cumulative maximum, null-propagating per fp-types::nancummax.
    pub fn cummax(&self) -> Result<Self, ColumnError> {
        let out = nancummax(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Cumulative minimum, null-propagating per fp-types::nancummin.
    pub fn cummin(&self) -> Result<Self, ColumnError> {
        let out = nancummin(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Sum of non-missing values.
    ///
    /// Matches `pd.Series.sum()` in skipna=True mode via fp-types::nansum.
    /// Empty column returns 0.0 (matching pandas).
    #[must_use]
    pub fn sum(&self) -> Scalar {
        nansum(&self.values)
    }

    /// Arithmetic mean of non-missing values.
    ///
    /// Matches `pd.Series.mean()` via fp-types::nanmean. Empty column
    /// returns Null(NaN).
    #[must_use]
    pub fn mean(&self) -> Scalar {
        nanmean(&self.values)
    }

    /// Minimum non-missing value.
    ///
    /// Matches `pd.Series.min()` via fp-types::nanmin. Preserves dtype
    /// for homogeneous inputs.
    #[must_use]
    pub fn min(&self) -> Scalar {
        nanmin(&self.values)
    }

    /// Maximum non-missing value.
    ///
    /// Matches `pd.Series.max()` via fp-types::nanmax.
    #[must_use]
    pub fn max(&self) -> Scalar {
        nanmax(&self.values)
    }

    /// Median of non-missing values.
    ///
    /// Matches `pd.Series.median()` via fp-types::nanmedian.
    #[must_use]
    pub fn median(&self) -> Scalar {
        nanmedian(&self.values)
    }

    /// Product of non-missing values.
    ///
    /// Matches `pd.Series.prod()` via fp-types::nanprod. Empty column
    /// returns 1.0 (matching pandas).
    #[must_use]
    pub fn prod(&self) -> Scalar {
        nanprod(&self.values)
    }

    /// Count of non-missing values.
    ///
    /// Matches `pd.Series.count()`.
    #[must_use]
    pub fn count(&self) -> usize {
        self.values.iter().filter(|v| !v.is_missing()).count()
    }

    /// Keep values where `cond` is true; replace false positions with
    /// `other`.
    ///
    /// Matches `pd.Series.where(cond, other)`. `cond` must be a Bool
    /// column of the same length (otherwise `LengthMismatch`). Missing
    /// positions in `cond` propagate as Null(NaN) in the result.
    pub fn where_cond(&self, cond: &Self, other: &Scalar) -> Result<Self, ColumnError> {
        if cond.dtype != DType::Bool {
            return Err(ColumnError::InvalidMaskType { dtype: cond.dtype });
        }
        if self.values.len() != cond.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: cond.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(cond.values.iter())
            .map(|(v, c)| match c {
                Scalar::Bool(true) => v.clone(),
                Scalar::Bool(false) => other.clone(),
                _ => Scalar::Null(NullKind::NaN),
            })
            .collect();
        Self::new(self.dtype, out)
    }

    /// Replace values where `cond` is true with `other`; otherwise keep.
    ///
    /// Matches `pd.Series.mask(cond, other)` — the logical inverse of
    /// `where_cond`. Same validation rules apply.
    pub fn mask(&self, cond: &Self, other: &Scalar) -> Result<Self, ColumnError> {
        if cond.dtype != DType::Bool {
            return Err(ColumnError::InvalidMaskType { dtype: cond.dtype });
        }
        if self.values.len() != cond.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: cond.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(cond.values.iter())
            .map(|(v, c)| match c {
                Scalar::Bool(true) => other.clone(),
                Scalar::Bool(false) => v.clone(),
                _ => Scalar::Null(NullKind::NaN),
            })
            .collect();
        Self::new(self.dtype, out)
    }

    /// Sort values in ascending or descending order.
    ///
    /// Matches `pd.Series.sort_values(ascending=...)`. Missing values
    /// are placed at the end (pandas `na_position='last'` default).
    /// Stable sort.
    pub fn sort_values(&self, ascending: bool) -> Result<Self, ColumnError> {
        let mut indexed: Vec<(usize, &Scalar)> = self.values.iter().enumerate().collect();
        indexed.sort_by(|a, b| compare_scalars_na_last(a.1, b.1, ascending));
        let sorted: Vec<Scalar> = indexed.into_iter().map(|(_, v)| v.clone()).collect();
        Self::new(self.dtype, sorted)
    }

    /// Positions that would sort the column ascending.
    ///
    /// Matches `pd.Series.argsort()`. Returns a `Vec<usize>` such that
    /// `take(&argsort)` equals `sort_values(true)`. Missing values
    /// sort to the end; stable.
    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        let mut indexed: Vec<(usize, &Scalar)> = self.values.iter().enumerate().collect();
        indexed.sort_by(|a, b| compare_scalars_na_last(a.1, b.1, true));
        indexed.into_iter().map(|(i, _)| i).collect()
    }

    /// First-order difference: `values[i] - values[i - periods]`.
    ///
    /// Matches `pd.Series.diff(periods)`. The leading `|periods|`
    /// positions are Null(NaN). Negative periods compute
    /// `values[i] - values[i + |periods|]`. Non-numeric inputs return
    /// a type error. Result dtype is always Float64.
    pub fn diff(&self, periods: i64) -> Result<Self, ColumnError> {
        let len = self.values.len();
        if len == 0 || periods == 0 {
            return Self::new(DType::Float64, vec![Scalar::Null(NullKind::NaN); len]);
        }
        let abs = periods.unsigned_abs() as usize;
        let mut out: Vec<Scalar> = Vec::with_capacity(len);
        for i in 0..len {
            if (periods > 0 && i < abs) || (periods < 0 && i + abs >= len) {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            let (cur, prev) = if periods > 0 {
                (&self.values[i], &self.values[i - abs])
            } else {
                (&self.values[i], &self.values[i + abs])
            };
            if cur.is_missing() || prev.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            match (cur.to_f64(), prev.to_f64()) {
                (Ok(a), Ok(b)) => out.push(Scalar::Float64(a - b)),
                _ => out.push(Scalar::Null(NullKind::NaN)),
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Per-row boolean flag for duplicated values (keep='first').
    ///
    /// Matches `pd.Series.duplicated()` — all but the first occurrence
    /// of each value is flagged true. Missing values are treated as a
    /// single bucket (pandas equates NaN for this purpose).
    pub fn duplicated(&self) -> Result<Self, ColumnError> {
        use std::collections::HashSet;
        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Null,
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
        }
        fn key_of(v: &Scalar) -> Key<'_> {
            if v.is_missing() {
                return Key::Null;
            }
            match v {
                Scalar::Bool(b) => Key::Bool(*b),
                Scalar::Int64(i) => Key::Int64(*i),
                Scalar::Float64(f) => {
                    let norm = if *f == 0.0 { 0.0 } else { *f };
                    Key::FloatBits(norm.to_bits())
                }
                Scalar::Utf8(s) => Key::Utf8(s.as_str()),
                Scalar::Timedelta64(v) => Key::Timedelta64(*v),
                Scalar::Null(_) => Key::Null,
            }
        }

        let mut seen: HashSet<Key<'_>> = HashSet::new();
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| Scalar::Bool(!seen.insert(key_of(v))))
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Bool column indicating whether each value lies in `[lower, upper]`
    /// (or the open interval when `inclusive=false`).
    ///
    /// Matches `pd.Series.between(left, right, inclusive='both'|'neither')`.
    /// Missing values map to false. Non-numeric inputs return a type
    /// error.
    pub fn between(&self, lower: f64, upper: f64, inclusive: bool) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Bool(false));
                continue;
            }
            match v.to_f64() {
                Ok(x) => {
                    let in_range = if inclusive {
                        x >= lower && x <= upper
                    } else {
                        x > lower && x < upper
                    };
                    out.push(Scalar::Bool(in_range));
                }
                Err(err) => return Err(ColumnError::Type(err)),
            }
        }
        Self::new(DType::Bool, out)
    }

    /// Encode the column as integer codes plus unique values.
    ///
    /// Matches `pd.Series.factorize()` default behavior: missing values
    /// map to `-1`, and uniques preserve first-seen order.
    pub fn factorize(&self) -> Result<(Self, Self), ColumnError> {
        let mut uniques: Vec<Scalar> = Vec::new();
        let mut codes: Vec<Scalar> = Vec::with_capacity(self.values.len());

        for value in &self.values {
            if value.is_missing() {
                codes.push(Scalar::Int64(-1));
                continue;
            }

            if let Some(position) = uniques.iter().position(|existing| existing == value) {
                codes.push(Scalar::Int64(position as i64));
            } else {
                codes.push(Scalar::Int64(uniques.len() as i64));
                uniques.push(value.clone());
            }
        }

        let codes_col = Self::new(DType::Int64, codes)?;
        let uniques_col = Self::new(self.dtype, uniques)?;
        Ok((codes_col, uniques_col))
    }

    /// Element-wise absolute value.
    ///
    /// Matches `pd.Series.abs()`. Int/Float/Timedelta paths preserve
    /// dtype; Bool/Utf8 inputs return `ColumnError::Type` because
    /// pandas raises TypeError on non-numeric .abs().
    pub fn abs(&self) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(v.clone());
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Int64(x.wrapping_abs())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.abs())),
                Scalar::Timedelta64(x) if *x != Timedelta::NAT => {
                    out.push(Scalar::Timedelta64(x.wrapping_abs()))
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(self.dtype, out)
    }

    /// Shift column values by `periods` positions, filling vacated slots
    /// with `fill`.
    ///
    /// Matches `pd.Series.shift(periods, fill_value)` for the positional
    /// form. Positive periods shift right (vacates the head); negative
    /// periods shift left (vacates the tail).
    pub fn shift(&self, periods: i64, fill: Scalar) -> Result<Self, ColumnError> {
        let len = self.values.len();
        if len == 0 || periods == 0 {
            return Ok(self.clone());
        }
        let abs = periods.unsigned_abs() as usize;
        let mut out: Vec<Scalar> = Vec::with_capacity(len);
        if abs >= len {
            for _ in 0..len {
                out.push(fill.clone());
            }
        } else if periods > 0 {
            for _ in 0..abs {
                out.push(fill.clone());
            }
            out.extend_from_slice(&self.values[..len - abs]);
        } else {
            out.extend_from_slice(&self.values[abs..]);
            for _ in 0..abs {
                out.push(fill.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Clip numeric values to `[lower, upper]`.
    ///
    /// Matches `pd.Series.clip(lower, upper)`. `None` on either bound
    /// disables that side. Non-numeric inputs return a type error.
    /// Missing values pass through unchanged. Result dtype is Float64
    /// (via `infer_dtype`) to accommodate fractional clipping.
    pub fn clip(&self, lower: Option<f64>, upper: Option<f64>) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(v.clone());
                continue;
            }
            let numeric = match v.to_f64() {
                Ok(x) => x,
                Err(err) => return Err(ColumnError::Type(err)),
            };
            let mut clipped = numeric;
            if let Some(lo) = lower
                && clipped < lo
            {
                clipped = lo;
            }
            if let Some(hi) = upper
                && clipped > hi
            {
                clipped = hi;
            }
            out.push(Scalar::Float64(clipped));
        }
        Self::new(DType::Float64, out)
    }

    /// Round numeric values to `decimals` decimal places.
    ///
    /// Matches `pd.Series.round(decimals)`. Negative `decimals` rounds
    /// to the left of the decimal point. Int columns pass through
    /// unchanged for decimals >= 0. Missing values are preserved.
    pub fn round(&self, decimals: i32) -> Result<Self, ColumnError> {
        if self.dtype == DType::Int64 && decimals >= 0 {
            return Ok(self.clone());
        }
        let factor = 10f64.powi(decimals);
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(v.clone());
                continue;
            }
            match v.to_f64() {
                Ok(x) => out.push(Scalar::Float64((x * factor).round() / factor)),
                Err(err) => return Err(ColumnError::Type(err)),
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Per-row boolean membership test against `needles`.
    ///
    /// Matches `pd.Series.isin(values)`. The result is always a Bool
    /// column the same length as `self`. Missing input positions map
    /// to `false` (pandas convention — NaN is never "in" a set).
    pub fn isin(&self, needles: &[Scalar]) -> Result<Self, ColumnError> {
        use std::collections::HashSet;
        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
        }
        fn key_of(v: &Scalar) -> Option<Key<'_>> {
            if v.is_missing() {
                return None;
            }
            Some(match v {
                Scalar::Bool(b) => Key::Bool(*b),
                Scalar::Int64(i) => Key::Int64(*i),
                Scalar::Float64(f) => {
                    let norm = if *f == 0.0 { 0.0 } else { *f };
                    Key::FloatBits(norm.to_bits())
                }
                Scalar::Utf8(s) => Key::Utf8(s.as_str()),
                Scalar::Timedelta64(v) => Key::Timedelta64(*v),
                Scalar::Null(_) => return None,
            })
        }

        let mut lookup: HashSet<Key<'_>> = HashSet::new();
        for n in needles {
            if let Some(k) = key_of(n) {
                lookup.insert(k);
            }
        }

        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| match key_of(v) {
                Some(k) => Scalar::Bool(lookup.contains(&k)),
                None => Scalar::Bool(false),
            })
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Unique values in first-seen order, missing values dropped.
    ///
    /// Matches `pd.Series.unique()` (pandas returns values in order of
    /// appearance and drops NaN/NA). Float NaN is deduplicated on bit
    /// pattern; +0.0 / -0.0 fold to the same key.
    pub fn unique(&self) -> Result<Self, ColumnError> {
        use std::collections::HashSet;
        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
        }

        let mut seen: HashSet<Key<'_>> = HashSet::new();
        let mut out = Vec::new();
        for v in &self.values {
            if v.is_missing() {
                continue;
            }
            let key = match v {
                Scalar::Bool(b) => Key::Bool(*b),
                Scalar::Int64(i) => Key::Int64(*i),
                Scalar::Float64(f) => {
                    let norm = if *f == 0.0 { 0.0 } else { *f };
                    Key::FloatBits(norm.to_bits())
                }
                Scalar::Utf8(s) => Key::Utf8(s.as_str()),
                Scalar::Timedelta64(v) => Key::Timedelta64(*v),
                Scalar::Null(_) => continue,
            };
            if seen.insert(key) {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Count occurrences of each distinct value.
    ///
    /// Matches `pd.Series.value_counts()` default behavior at the
    /// columnar level: missing values are dropped, counts are sorted
    /// descending, and first-seen order breaks ties.
    pub fn value_counts(&self) -> Result<(Self, Self), ColumnError> {
        self.value_counts_with_options(false, true, false, true)
    }

    /// Count occurrences of each distinct value with pandas-style options.
    ///
    /// Returns a pair of columns `(values, counts)`. The `values`
    /// column preserves the source dtype; the `counts` column is Int64
    /// unless `normalize=true`, in which case it is Float64.
    pub fn value_counts_with_options(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> Result<(Self, Self), ColumnError> {
        let mut counts: Vec<(Scalar, usize)> = Vec::new();
        let mut missing_count = 0_usize;

        for value in &self.values {
            if value.is_missing() {
                missing_count += 1;
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

        if !dropna && missing_count > 0 {
            counts.push((Scalar::Null(NullKind::NaN), missing_count));
        }

        if sort {
            if ascending {
                counts.sort_by_key(|(_, count)| *count);
            } else {
                counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            }
        }

        let total = if normalize {
            counts.iter().map(|(_, count)| *count).sum::<usize>() as f64
        } else {
            1.0
        };

        let mut values_out = Vec::with_capacity(counts.len());
        let mut counts_out = Vec::with_capacity(counts.len());
        for (value, count) in counts {
            values_out.push(value);
            if normalize {
                let normalized = if total == 0.0 {
                    0.0
                } else {
                    count as f64 / total
                };
                counts_out.push(Scalar::Float64(normalized));
            } else {
                counts_out.push(Scalar::Int64(i64::try_from(count).unwrap_or(i64::MAX)));
            }
        }

        let values = Self::new(self.dtype, values_out)?;
        let counts = Self::new(
            if normalize {
                DType::Float64
            } else {
                DType::Int64
            },
            counts_out,
        )?;
        Ok((values, counts))
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
        self.perm[..split]
            .iter()
            .copied()
            .filter(|&idx| {
                column
                    .value(idx)
                    .and_then(|v| v.to_f64().ok())
                    .is_some_and(|f| f <= value)
            })
            .collect()
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
    use fp_types::{DType, NullKind, Scalar};

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
    fn column_from_values_preserves_mixed_utf8_numeric_scalars() {
        let column = Column::from_values(vec![Scalar::Utf8("x".into()), Scalar::Int64(1)])
            .expect("mixed object-like constructor should succeed");

        assert_eq!(column.dtype(), DType::Utf8);
        assert_eq!(
            column.values(),
            &[Scalar::Utf8("x".into()), Scalar::Int64(1)]
        );
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
    fn vectorized_f64_mod_pow_floordiv() {
        let left = Column::from_values(vec![
            Scalar::Float64(10.0),
            Scalar::Float64(2.0),
            Scalar::Float64(-3.0),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Float64(3.0),
            Scalar::Float64(3.0),
            Scalar::Float64(2.0),
        ])
        .expect("right");

        let modulo = left.binary_numeric(&right, ArithmeticOp::Mod).expect("mod");
        assert_eq!(modulo.dtype(), DType::Float64);
        assert!(matches!(modulo.values()[0], Scalar::Float64(v) if (v - 1.0).abs() < 1e-10));
        assert!(matches!(modulo.values()[1], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
        assert!(
            matches!(modulo.values()[2], Scalar::Float64(v) if (v - (-3.0_f64 % 2.0)).abs() < 1e-10)
        );

        let pow = left.binary_numeric(&right, ArithmeticOp::Pow).expect("pow");
        assert_eq!(pow.dtype(), DType::Float64);
        assert!(matches!(pow.values()[0], Scalar::Float64(v) if (v - 1000.0).abs() < 1e-10));
        assert!(matches!(pow.values()[1], Scalar::Float64(v) if (v - 8.0).abs() < 1e-10));
        assert!(matches!(pow.values()[2], Scalar::Float64(v) if (v - 9.0).abs() < 1e-10));

        let floordiv = left
            .binary_numeric(&right, ArithmeticOp::FloorDiv)
            .expect("floordiv");
        assert_eq!(floordiv.dtype(), DType::Float64);
        assert!(matches!(floordiv.values()[0], Scalar::Float64(v) if (v - 3.0).abs() < 1e-10));
        assert!(matches!(floordiv.values()[1], Scalar::Float64(v) if (v - 0.0).abs() < 1e-10));
        assert!(matches!(floordiv.values()[2], Scalar::Float64(v) if (v - -2.0).abs() < 1e-10));
    }

    #[test]
    fn int64_mod_floordiv_preserves_dtype() {
        // Test that int % int and int // int stay Int64 (pandas parity)
        let left = Column::from_values(vec![
            Scalar::Int64(10),
            Scalar::Int64(20),
            Scalar::Int64(30),
        ])
        .expect("left");
        let right = Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(7), Scalar::Int64(4)])
            .expect("right");

        let modulo = left.binary_numeric(&right, ArithmeticOp::Mod).expect("mod");
        assert_eq!(modulo.dtype(), DType::Int64, "mod should preserve Int64");
        assert_eq!(modulo.values()[0], Scalar::Int64(1));
        assert_eq!(modulo.values()[1], Scalar::Int64(6));
        assert_eq!(modulo.values()[2], Scalar::Int64(2));

        let floordiv = left
            .binary_numeric(&right, ArithmeticOp::FloorDiv)
            .expect("floordiv");
        assert_eq!(
            floordiv.dtype(),
            DType::Int64,
            "floordiv should preserve Int64"
        );
        assert_eq!(floordiv.values()[0], Scalar::Int64(3));
        assert_eq!(floordiv.values()[1], Scalar::Int64(2));
        assert_eq!(floordiv.values()[2], Scalar::Int64(7));
    }

    #[test]
    fn int64_mod_floordiv_with_zero_promotes_to_float() {
        // Test that int % 0 and int // 0 promote to Float64 (pandas parity)
        let left = Column::from_values(vec![
            Scalar::Int64(10),
            Scalar::Int64(20),
            Scalar::Int64(30),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Int64(3),
            Scalar::Int64(0), // Zero divisor
            Scalar::Int64(4),
        ])
        .expect("right");

        let modulo = left.binary_numeric(&right, ArithmeticOp::Mod).expect("mod");
        assert_eq!(
            modulo.dtype(),
            DType::Float64,
            "mod with zero should promote to Float64"
        );
        assert!(matches!(modulo.values()[0], Scalar::Float64(v) if (v - 1.0).abs() < 1e-10));
        assert!(matches!(modulo.values()[1], Scalar::Float64(v) if v.is_nan()));
        assert!(matches!(modulo.values()[2], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));

        let floordiv = left
            .binary_numeric(&right, ArithmeticOp::FloorDiv)
            .expect("floordiv");
        assert_eq!(
            floordiv.dtype(),
            DType::Float64,
            "floordiv with zero should promote to Float64"
        );
        assert!(matches!(floordiv.values()[0], Scalar::Float64(v) if (v - 3.0).abs() < 1e-10));
        assert!(matches!(floordiv.values()[1], Scalar::Float64(v) if v.is_infinite()));
        assert!(matches!(floordiv.values()[2], Scalar::Float64(v) if (v - 7.0).abs() < 1e-10));
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
                .filter_map(|&i| match &col.values()[i] {
                    Scalar::Int64(v) => Some(*v),
                    _ => None,
                })
                .collect();
            assert_eq!(gt5_vals.len(), gt5.len(), "expected Int64 values");
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

    // === Comparison, Filter, and Missing-Data Operation Tests ===

    mod comparison_tests {
        use super::super::*;
        use fp_types::{NullKind, Scalar};

        #[test]
        fn comparison_gt_int64() {
            let left =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(5), Scalar::Int64(3)])
                    .expect("left");
            let right =
                Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(3), Scalar::Int64(3)])
                    .expect("right");

            let result = left
                .binary_comparison(&right, ComparisonOp::Gt)
                .expect("gt");
            assert_eq!(result.dtype(), fp_types::DType::Bool);
            assert_eq!(result.values()[0], Scalar::Bool(false));
            assert_eq!(result.values()[1], Scalar::Bool(true));
            assert_eq!(result.values()[2], Scalar::Bool(false));
        }

        #[test]
        fn comparison_all_ops_numeric() {
            let left = Column::from_values(vec![Scalar::Float64(5.0)]).expect("left");
            let right = Column::from_values(vec![Scalar::Float64(3.0)]).expect("right");

            let gt = left
                .binary_comparison(&right, ComparisonOp::Gt)
                .expect("gt");
            let lt = left
                .binary_comparison(&right, ComparisonOp::Lt)
                .expect("lt");
            let eq = left
                .binary_comparison(&right, ComparisonOp::Eq)
                .expect("eq");
            let ne = left
                .binary_comparison(&right, ComparisonOp::Ne)
                .expect("ne");
            let ge = left
                .binary_comparison(&right, ComparisonOp::Ge)
                .expect("ge");
            let le = left
                .binary_comparison(&right, ComparisonOp::Le)
                .expect("le");

            assert_eq!(gt.values()[0], Scalar::Bool(true));
            assert_eq!(lt.values()[0], Scalar::Bool(false));
            assert_eq!(eq.values()[0], Scalar::Bool(false));
            assert_eq!(ne.values()[0], Scalar::Bool(true));
            assert_eq!(ge.values()[0], Scalar::Bool(true));
            assert_eq!(le.values()[0], Scalar::Bool(false));
        }

        #[test]
        fn comparison_equality_equal_values() {
            let col = Column::from_values(vec![Scalar::Int64(42)]).expect("col");
            let result = col.binary_comparison(&col, ComparisonOp::Eq).expect("eq");
            assert_eq!(result.values()[0], Scalar::Bool(true));

            let ne = col.binary_comparison(&col, ComparisonOp::Ne).expect("ne");
            assert_eq!(ne.values()[0], Scalar::Bool(false));
        }

        #[test]
        fn comparison_null_propagation() {
            let left = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
            ])
            .expect("left");
            let right = Column::from_values(vec![
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null),
            ])
            .expect("right");

            let result = left
                .binary_comparison(&right, ComparisonOp::Gt)
                .expect("gt");
            assert_eq!(result.values()[0], Scalar::Bool(false));
            assert!(result.values()[1].is_missing(), "null op valid = null");
            assert!(result.values()[2].is_missing(), "valid op null = null");
        }

        #[test]
        fn comparison_utf8_lexicographic() {
            let left = Column::from_values(vec![
                Scalar::Utf8("banana".to_string()),
                Scalar::Utf8("apple".to_string()),
            ])
            .expect("left");
            let right = Column::from_values(vec![
                Scalar::Utf8("apple".to_string()),
                Scalar::Utf8("cherry".to_string()),
            ])
            .expect("right");

            let gt = left
                .binary_comparison(&right, ComparisonOp::Gt)
                .expect("gt");
            assert_eq!(gt.values()[0], Scalar::Bool(true));
            assert_eq!(gt.values()[1], Scalar::Bool(false));
        }

        #[test]
        fn compare_scalar_gt() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(5),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
            ])
            .expect("col");

            let result = col
                .compare_scalar(&Scalar::Int64(3), ComparisonOp::Gt)
                .expect("gt");
            assert_eq!(result.values()[0], Scalar::Bool(false));
            assert_eq!(result.values()[1], Scalar::Bool(true));
            assert!(result.values()[2].is_missing());
            assert_eq!(result.values()[3], Scalar::Bool(false));
        }

        #[test]
        fn compare_scalar_with_missing_scalar() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");

            let result = col
                .compare_scalar(&Scalar::Null(NullKind::Null), ComparisonOp::Eq)
                .expect("eq");
            assert!(result.values()[0].is_missing());
            assert!(result.values()[1].is_missing());
        }

        #[test]
        fn filter_by_mask_basic() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ])
            .expect("col");
            let mask = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(false),
            ])
            .expect("mask");

            let result = col.filter_by_mask(&mask).expect("filter");
            assert_eq!(result.len(), 2);
            assert_eq!(result.values()[0], Scalar::Int64(10));
            assert_eq!(result.values()[1], Scalar::Int64(30));
        }

        #[test]
        fn filter_by_mask_null_treated_as_false() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let mask = Column::from_values(vec![Scalar::Bool(true), Scalar::Null(NullKind::Null)])
                .expect("mask");

            let result = col.filter_by_mask(&mask).expect("filter");
            assert_eq!(result.len(), 1);
            assert_eq!(result.values()[0], Scalar::Int64(1));
        }

        #[test]
        fn filter_by_mask_rejects_non_boolean_mask() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let mask = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(0)]).expect("mask");

            let err = col.filter_by_mask(&mask).expect_err("non-bool mask");
            assert!(matches!(err, ColumnError::InvalidMaskType { .. }));
        }

        #[test]
        fn filter_by_mask_empty_result() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let mask =
                Column::from_values(vec![Scalar::Bool(false), Scalar::Bool(false)]).expect("mask");

            let result = col.filter_by_mask(&mask).expect("filter");
            assert!(result.is_empty());
        }

        #[test]
        fn fillna_replaces_missing() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");

            let result = col.fillna(&Scalar::Int64(0)).expect("fillna");
            assert_eq!(result.values()[0], Scalar::Int64(1));
            assert_eq!(result.values()[1], Scalar::Int64(0));
            assert_eq!(result.values()[2], Scalar::Int64(3));
            assert_eq!(result.values()[3], Scalar::Int64(0));
            assert_eq!(result.validity().count_valid(), 4);
        }

        #[test]
        fn dropna_removes_missing() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");

            let result = col.dropna().expect("dropna");
            assert_eq!(result.len(), 2);
            assert_eq!(result.values()[0], Scalar::Int64(1));
            assert_eq!(result.values()[1], Scalar::Int64(3));
        }

        #[test]
        fn comparison_empty_columns() {
            let left = Column::from_values(vec![]).expect("left");
            let right = Column::from_values(vec![]).expect("right");
            let result = left
                .binary_comparison(&right, ComparisonOp::Eq)
                .expect("eq");
            assert!(result.is_empty());
        }

        #[test]
        fn comparison_length_mismatch_error() {
            let left = Column::from_values(vec![Scalar::Int64(1)]).expect("left");
            let right =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("right");
            assert!(left.binary_comparison(&right, ComparisonOp::Eq).is_err());
        }

        #[test]
        fn comparison_bool_ordering() {
            let left =
                Column::from_values(vec![Scalar::Bool(true), Scalar::Bool(false)]).expect("left");
            let right =
                Column::from_values(vec![Scalar::Bool(false), Scalar::Bool(true)]).expect("right");

            let gt = left
                .binary_comparison(&right, ComparisonOp::Gt)
                .expect("gt");
            assert_eq!(gt.values()[0], Scalar::Bool(true));
            assert_eq!(gt.values()[1], Scalar::Bool(false));
        }
    }

    mod take_slice_concat_repeat {
        use super::*;

        #[test]
        fn take_reorders_rows() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
            ])
            .expect("col");
            let picked = col.take(&[2, 0, 1]).expect("take");
            assert_eq!(picked.values()[0], Scalar::Int64(30));
            assert_eq!(picked.values()[1], Scalar::Int64(10));
            assert_eq!(picked.values()[2], Scalar::Int64(20));
        }

        #[test]
        fn take_out_of_bounds_errors() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col.take(&[5]).unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }

        #[test]
        fn slice_returns_contiguous_range() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ])
            .expect("col");
            let middle = col.slice(1, 2).expect("slice");
            assert_eq!(middle.len(), 2);
            assert_eq!(middle.values()[0], Scalar::Int64(2));
            assert_eq!(middle.values()[1], Scalar::Int64(3));
        }

        #[test]
        fn slice_past_end_yields_empty() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let empty = col.slice(10, 5).expect("slice");
            assert!(empty.is_empty());
            assert_eq!(empty.dtype(), DType::Int64);
        }

        #[test]
        fn slice_len_clamps_to_tail() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            let tail = col.slice(2, 100).expect("slice");
            assert_eq!(tail.len(), 1);
            assert_eq!(tail.values()[0], Scalar::Float64(3.0));
        }

        #[test]
        fn head_returns_first_n_values() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ])
            .expect("col");
            let out = col.head(2).expect("head");
            assert_eq!(out.values(), &[Scalar::Int64(10), Scalar::Int64(20)]);
        }

        #[test]
        fn tail_returns_last_n_values() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
            ])
            .expect("col");
            let out = col.tail(2).expect("tail");
            assert_eq!(out.values(), &[Scalar::Int64(30), Scalar::Int64(40)]);
        }

        #[test]
        fn head_tail_negative_n_match_pandas_style() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
                Scalar::Int64(40),
                Scalar::Int64(50),
            ])
            .expect("col");
            let head = col.head(-2).expect("head");
            let tail = col.tail(-2).expect("tail");
            assert_eq!(
                head.values(),
                &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]
            );
            assert_eq!(
                tail.values(),
                &[Scalar::Int64(30), Scalar::Int64(40), Scalar::Int64(50)]
            );
        }

        #[test]
        fn head_tail_large_negative_n_saturate_to_empty() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            let head = col.head(-10).expect("head");
            let tail = col.tail(-10).expect("tail");
            assert!(head.is_empty());
            assert!(tail.is_empty());
            assert_eq!(head.dtype(), DType::Float64);
            assert_eq!(tail.dtype(), DType::Float64);
        }

        #[test]
        fn concat_appends_same_dtype() {
            let a = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(3)]).expect("b");
            let combined = a.concat(&b).expect("concat");
            assert_eq!(combined.len(), 3);
            assert_eq!(combined.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn concat_different_dtypes_errors() {
            let a = Column::from_values(vec![Scalar::Int64(1)]).expect("a");
            let b = Column::from_values(vec![Scalar::Utf8("x".into())]).expect("b");
            let err = a.concat(&b).unwrap_err();
            assert!(matches!(err, crate::ColumnError::DTypeMismatch { .. }));
        }

        #[test]
        fn repeat_duplicates_contiguously() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let out = col.repeat(3).expect("repeat");
            assert_eq!(out.len(), 6);
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(1));
            assert_eq!(out.values()[2], Scalar::Int64(1));
            assert_eq!(out.values()[3], Scalar::Int64(2));
        }

        #[test]
        fn repeat_zero_is_empty_same_dtype() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let out = col.repeat(0).expect("repeat");
            assert!(out.is_empty());
            assert_eq!(out.dtype(), DType::Int64);
        }

        #[test]
        fn repeat_one_is_clone() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let out = col.repeat(1).expect("repeat");
            assert_eq!(out.values(), col.values());
        }
    }

    mod reverse_head_tail_cumulatives_unique {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn reverse_swaps_order_and_preserves_dtype() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let r = col.reverse().expect("reverse");
            assert_eq!(r.values()[0], Scalar::Int64(3));
            assert_eq!(r.values()[2], Scalar::Int64(1));
            assert_eq!(r.dtype(), DType::Int64);
        }

        #[test]
        fn head_positive_takes_first_n() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ])
            .expect("col");
            let h = col.head(2).expect("head");
            assert_eq!(h.len(), 2);
            assert_eq!(h.values()[0], Scalar::Int64(1));
            assert_eq!(h.values()[1], Scalar::Int64(2));
        }

        #[test]
        fn head_negative_drops_last_n() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let h = col.head(-1).expect("head");
            assert_eq!(h.len(), 2);
            assert_eq!(h.values()[1], Scalar::Int64(2));
        }

        #[test]
        fn tail_positive_takes_last_n() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ])
            .expect("col");
            let t = col.tail(2).expect("tail");
            assert_eq!(t.len(), 2);
            assert_eq!(t.values()[0], Scalar::Int64(3));
            assert_eq!(t.values()[1], Scalar::Int64(4));
        }

        #[test]
        fn tail_negative_drops_first_n() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let t = col.tail(-1).expect("tail");
            assert_eq!(t.len(), 2);
            assert_eq!(t.values()[0], Scalar::Int64(2));
        }

        #[test]
        fn head_tail_out_of_range_clamps() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            assert_eq!(col.head(10).expect("head").len(), 1);
            assert_eq!(col.tail(10).expect("tail").len(), 1);
            assert_eq!(col.head(-10).expect("head").len(), 0);
            assert_eq!(col.tail(-10).expect("tail").len(), 0);
        }

        #[test]
        fn cumsum_produces_float64_running_sum() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            let c = col.cumsum().expect("cumsum");
            assert_eq!(c.dtype(), DType::Float64);
            assert_eq!(c.values()[0], Scalar::Float64(1.0));
            assert!(c.values()[1].is_missing());
            assert_eq!(c.values()[2], Scalar::Float64(4.0));
        }

        #[test]
        fn cumprod_running_product() {
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ])
            .expect("col");
            let c = col.cumprod().expect("cumprod");
            assert_eq!(c.values()[2], Scalar::Float64(24.0));
        }

        #[test]
        fn cummax_cummin_running_extrema() {
            let col = Column::from_values(vec![
                Scalar::Float64(3.0),
                Scalar::Float64(1.0),
                Scalar::Float64(4.0),
                Scalar::Float64(1.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let mx = col.cummax().expect("cummax");
            assert_eq!(mx.values()[4], Scalar::Float64(5.0));
            let mn = col.cummin().expect("cummin");
            assert_eq!(mn.values()[4], Scalar::Float64(1.0));
        }

        #[test]
        fn unique_preserves_first_seen_order() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(2),
                Scalar::Int64(1),
            ])
            .expect("col");
            let u = col.unique().expect("unique");
            assert_eq!(u.len(), 3);
            assert_eq!(u.values()[0], Scalar::Int64(3));
            assert_eq!(u.values()[1], Scalar::Int64(1));
            assert_eq!(u.values()[2], Scalar::Int64(2));
        }

        #[test]
        fn unique_drops_nulls() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let u = col.unique().expect("unique");
            assert_eq!(u.len(), 1);
            assert_eq!(u.values()[0], Scalar::Int64(1));
        }
    }

    mod abs_shift_clip_round_isin {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn abs_int_and_float() {
            let int_col =
                Column::from_values(vec![Scalar::Int64(-3), Scalar::Int64(0), Scalar::Int64(5)])
                    .expect("int");
            let a = int_col.abs().expect("abs");
            assert_eq!(a.values()[0], Scalar::Int64(3));
            assert_eq!(a.values()[1], Scalar::Int64(0));

            let float_col =
                Column::from_values(vec![Scalar::Float64(-1.5), Scalar::Null(NullKind::NaN)])
                    .expect("float");
            let b = float_col.abs().expect("abs");
            assert_eq!(b.values()[0], Scalar::Float64(1.5));
            assert!(b.values()[1].is_missing());
        }

        #[test]
        fn abs_utf8_errors() {
            let col = Column::from_values(vec![Scalar::Utf8("x".into())]).expect("col");
            assert!(col.abs().is_err());
        }

        #[test]
        fn shift_positive_pads_left_with_fill() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let s = col.shift(1, Scalar::Null(NullKind::NaN)).expect("shift");
            assert!(s.values()[0].is_missing());
            assert_eq!(s.values()[1], Scalar::Int64(1));
            assert_eq!(s.values()[2], Scalar::Int64(2));
        }

        #[test]
        fn shift_negative_pads_right() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let s = col.shift(-1, Scalar::Int64(0)).expect("shift");
            assert_eq!(s.values()[0], Scalar::Int64(2));
            assert_eq!(s.values()[1], Scalar::Int64(3));
            assert_eq!(s.values()[2], Scalar::Int64(0));
        }

        #[test]
        fn shift_zero_is_clone() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let s = col.shift(0, Scalar::Int64(-1)).expect("shift");
            assert_eq!(s.values(), col.values());
        }

        #[test]
        fn clip_both_bounds() {
            let col = Column::from_values(vec![
                Scalar::Float64(-5.0),
                Scalar::Float64(3.0),
                Scalar::Float64(10.0),
            ])
            .expect("col");
            let c = col.clip(Some(0.0), Some(5.0)).expect("clip");
            assert_eq!(c.values()[0], Scalar::Float64(0.0));
            assert_eq!(c.values()[1], Scalar::Float64(3.0));
            assert_eq!(c.values()[2], Scalar::Float64(5.0));
        }

        #[test]
        fn clip_none_bounds_are_noop() {
            let col = Column::from_values(vec![Scalar::Float64(-5.0), Scalar::Float64(10.0)])
                .expect("col");
            let c = col.clip(None, None).expect("clip");
            assert_eq!(c.values()[0], Scalar::Float64(-5.0));
            assert_eq!(c.values()[1], Scalar::Float64(10.0));
        }

        #[test]
        fn round_rounds_floats() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.234),
                Scalar::Float64(5.678),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let r = col.round(1).expect("round");
            assert_eq!(r.values()[0], Scalar::Float64(1.2));
            assert_eq!(r.values()[1], Scalar::Float64(5.7));
            assert!(r.values()[2].is_missing());
        }

        #[test]
        fn round_int_nonnegative_decimals_is_noop() {
            let col = Column::from_values(vec![Scalar::Int64(12), Scalar::Int64(34)]).expect("col");
            let r = col.round(2).expect("round");
            assert_eq!(r.values(), col.values());
            assert_eq!(r.dtype(), DType::Int64);
        }

        #[test]
        fn round_negative_decimals_rounds_left() {
            let col = Column::from_values(vec![Scalar::Float64(1234.0)]).expect("col");
            let r = col.round(-2).expect("round");
            assert_eq!(r.values()[0], Scalar::Float64(1200.0));
        }

        #[test]
        fn isin_returns_bool_column() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let needles = vec![Scalar::Int64(1), Scalar::Int64(3)];
            let r = col.isin(&needles).expect("isin");
            assert_eq!(r.dtype(), DType::Bool);
            assert_eq!(r.values()[0], Scalar::Bool(true));
            assert_eq!(r.values()[1], Scalar::Bool(false));
            assert_eq!(r.values()[2], Scalar::Bool(true));
            assert_eq!(r.values()[3], Scalar::Bool(false));
        }

        #[test]
        fn isin_empty_needles_yields_all_false() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let r = col.isin(&[]).expect("isin");
            assert_eq!(r.values()[0], Scalar::Bool(false));
            assert_eq!(r.values()[1], Scalar::Bool(false));
        }
    }

    mod sort_diff_duplicated_between {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn sort_values_ascending_puts_nulls_last() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Int64(2),
            ])
            .expect("col");
            let s = col.sort_values(true).expect("sort");
            assert_eq!(s.values()[0], Scalar::Int64(1));
            assert_eq!(s.values()[1], Scalar::Int64(2));
            assert_eq!(s.values()[2], Scalar::Int64(3));
            assert!(s.values()[3].is_missing());
        }

        #[test]
        fn sort_values_descending_keeps_nulls_last() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ])
            .expect("col");
            let s = col.sort_values(false).expect("sort");
            assert_eq!(s.values()[0], Scalar::Int64(3));
            assert_eq!(s.values()[1], Scalar::Int64(2));
            assert_eq!(s.values()[2], Scalar::Int64(1));
            assert!(s.values()[3].is_missing());
        }

        #[test]
        fn argsort_matches_take_sort_values() {
            let col =
                Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)])
                    .expect("col");
            let positions = col.argsort();
            assert_eq!(positions, vec![1, 2, 0]);
            let via_take = col.take(&positions).expect("take");
            let via_sort = col.sort_values(true).expect("sort");
            assert_eq!(via_take.values(), via_sort.values());
        }

        #[test]
        fn diff_periods_one_subtracts_prev() {
            let col =
                Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(8), Scalar::Int64(10)])
                    .expect("col");
            let d = col.diff(1).expect("diff");
            assert!(d.values()[0].is_missing());
            assert_eq!(d.values()[1], Scalar::Float64(3.0));
            assert_eq!(d.values()[2], Scalar::Float64(2.0));
            assert_eq!(d.dtype(), DType::Float64);
        }

        #[test]
        fn diff_negative_period_looks_ahead() {
            let col =
                Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(8), Scalar::Int64(10)])
                    .expect("col");
            let d = col.diff(-1).expect("diff");
            assert_eq!(d.values()[0], Scalar::Float64(-3.0));
            assert_eq!(d.values()[1], Scalar::Float64(-2.0));
            assert!(d.values()[2].is_missing());
        }

        #[test]
        fn duplicated_keep_first() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ])
            .expect("col");
            let d = col.duplicated().expect("duplicated");
            assert_eq!(d.values()[0], Scalar::Bool(false));
            assert_eq!(d.values()[1], Scalar::Bool(false));
            assert_eq!(d.values()[2], Scalar::Bool(true));
            assert_eq!(d.values()[3], Scalar::Bool(false));
            assert_eq!(d.values()[4], Scalar::Bool(true));
        }

        #[test]
        fn duplicated_treats_nulls_as_one_bucket() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
            ])
            .expect("col");
            let d = col.duplicated().expect("duplicated");
            assert_eq!(d.values()[0], Scalar::Bool(false));
            assert_eq!(d.values()[1], Scalar::Bool(true));
            assert_eq!(d.values()[2], Scalar::Bool(false));
        }

        #[test]
        fn between_inclusive_both() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.5),
                Scalar::Float64(1.0),
                Scalar::Float64(5.0),
                Scalar::Float64(6.0),
            ])
            .expect("col");
            let b = col.between(1.0, 5.0, true).expect("between");
            assert_eq!(b.values()[0], Scalar::Bool(false));
            assert_eq!(b.values()[1], Scalar::Bool(true));
            assert_eq!(b.values()[2], Scalar::Bool(true));
            assert_eq!(b.values()[3], Scalar::Bool(false));
        }

        #[test]
        fn between_exclusive() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let b = col.between(1.0, 5.0, false).expect("between");
            assert_eq!(b.values()[0], Scalar::Bool(false));
            assert_eq!(b.values()[1], Scalar::Bool(true));
            assert_eq!(b.values()[2], Scalar::Bool(false));
        }

        #[test]
        fn between_missing_maps_to_false() {
            let col = Column::from_values(vec![Scalar::Null(NullKind::NaN), Scalar::Float64(3.0)])
                .expect("col");
            let b = col.between(1.0, 5.0, true).expect("between");
            assert_eq!(b.values()[0], Scalar::Bool(false));
            assert_eq!(b.values()[1], Scalar::Bool(true));
        }
    }

    mod factorize {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn factorize_preserves_first_seen_order() {
            let col = Column::from_values(vec![
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
            ])
            .expect("col");

            let (codes, uniques) = col.factorize().expect("factorize");
            assert_eq!(codes.dtype(), DType::Int64);
            assert_eq!(
                codes.values(),
                &[
                    Scalar::Int64(0),
                    Scalar::Int64(1),
                    Scalar::Int64(0),
                    Scalar::Int64(2),
                    Scalar::Int64(1),
                ]
            );
            assert_eq!(
                uniques.values(),
                &[
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("c".into()),
                ]
            );
        }

        #[test]
        fn factorize_missing_values_map_to_negative_one() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.5),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.5),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(1.5),
            ])
            .expect("col");

            let (codes, uniques) = col.factorize().expect("factorize");
            assert_eq!(
                codes.values(),
                &[
                    Scalar::Int64(0),
                    Scalar::Int64(-1),
                    Scalar::Int64(1),
                    Scalar::Int64(-1),
                    Scalar::Int64(0),
                ]
            );
            assert_eq!(uniques.dtype(), DType::Float64);
            assert_eq!(
                uniques.values(),
                &[Scalar::Float64(1.5), Scalar::Float64(2.5)]
            );
        }

        #[test]
        fn factorize_empty_column_returns_empty_outputs() {
            let col = Column::new(DType::Int64, Vec::new()).expect("col");
            let (codes, uniques) = col.factorize().expect("factorize");
            assert!(codes.is_empty());
            assert!(uniques.is_empty());
            assert_eq!(codes.dtype(), DType::Int64);
            assert_eq!(uniques.dtype(), DType::Int64);
        }
    }

    mod aggregation_helpers {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn sum_skips_nulls() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            let sum = col.sum();
            assert!(matches!(sum, Scalar::Float64(_)), "expected Float64 result");
            if let Scalar::Float64(v) = sum {
                assert!((v - 6.0).abs() < 1e-9);
            }
        }

        #[test]
        fn sum_empty_is_zero() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert_eq!(col.sum(), Scalar::Float64(0.0));
        }

        #[test]
        fn mean_matches_sum_over_count() {
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(6.0),
            ])
            .expect("col");
            let mean = col.mean();
            assert!(
                matches!(mean, Scalar::Float64(_)),
                "expected Float64 result"
            );
            if let Scalar::Float64(v) = mean {
                assert!((v - 4.0).abs() < 1e-9);
            }
        }

        #[test]
        fn mean_empty_is_null() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert!(col.mean().is_missing());
        }

        #[test]
        fn min_max_extrema_skip_nulls() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Int64(5),
                Scalar::Int64(2),
            ])
            .expect("col");
            assert_eq!(col.min(), Scalar::Int64(1));
            assert_eq!(col.max(), Scalar::Int64(5));
        }

        #[test]
        fn median_of_odd_count() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(5.0),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            let median = col.median();
            assert!(
                matches!(median, Scalar::Float64(_)),
                "expected Float64 result"
            );
            if let Scalar::Float64(v) = median {
                assert!((v - 3.0).abs() < 1e-9);
            }
        }

        #[test]
        fn prod_multiplies_non_nulls() {
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ])
            .expect("col");
            let prod = col.prod();
            assert!(
                matches!(prod, Scalar::Float64(_)),
                "expected Float64 result"
            );
            if let Scalar::Float64(v) = prod {
                assert!((v - 24.0).abs() < 1e-9);
            }
        }

        #[test]
        fn prod_empty_is_one() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert_eq!(col.prod(), Scalar::Float64(1.0));
        }

        #[test]
        fn count_excludes_nulls() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(2),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");
            assert_eq!(col.count(), 2);
        }
    }

    mod where_mask {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn where_cond_keeps_true_positions() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
            ])
            .expect("col");
            let cond = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
            ])
            .expect("cond");
            let fill = Scalar::Int64(-1);
            let out = col.where_cond(&cond, &fill).expect("where");
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(-1));
            assert_eq!(out.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn mask_inverts_where_cond() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
            ])
            .expect("col");
            let cond = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
            ])
            .expect("cond");
            let fill = Scalar::Int64(0);
            let out = col.mask(&cond, &fill).expect("mask");
            assert_eq!(out.values()[0], Scalar::Int64(0));
            assert_eq!(out.values()[1], Scalar::Int64(2));
            assert_eq!(out.values()[2], Scalar::Int64(0));
        }

        #[test]
        fn where_missing_cond_propagates_null() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let cond = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("cond");
            let fill = Scalar::Int64(-1);
            let out = col.where_cond(&cond, &fill).expect("where");
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert!(out.values()[1].is_missing());
        }

        #[test]
        fn where_rejects_non_bool_cond() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let cond = Column::from_values(vec![Scalar::Int64(1)]).expect("cond");
            let err = col.where_cond(&cond, &Scalar::Int64(0)).unwrap_err();
            assert!(matches!(err, crate::ColumnError::InvalidMaskType { .. }));
        }

        #[test]
        fn where_rejects_length_mismatch() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let cond = Column::from_values(vec![Scalar::Bool(true)]).expect("cond");
            let err = col.where_cond(&cond, &Scalar::Int64(0)).unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }
    }

    mod value_counts {
        use super::*;
        use fp_types::NullKind;

        #[test]
        fn value_counts_default_drops_missing_and_sorts_descending() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
            ])
            .expect("col");

            let (values, counts) = col.value_counts().expect("value_counts");
            assert_eq!(
                values.values(),
                &[Scalar::Int64(3), Scalar::Int64(1), Scalar::Int64(2)]
            );
            assert_eq!(
                counts.values(),
                &[Scalar::Int64(3), Scalar::Int64(2), Scalar::Int64(1)]
            );
        }

        #[test]
        fn value_counts_sort_false_preserves_first_seen_order() {
            let col = Column::from_values(vec![
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(1),
            ])
            .expect("col");

            let (values, counts) = col
                .value_counts_with_options(false, false, false, true)
                .expect("value_counts");
            assert_eq!(
                values.values(),
                &[Scalar::Int64(2), Scalar::Int64(1), Scalar::Int64(3)]
            );
            assert_eq!(
                counts.values(),
                &[Scalar::Int64(2), Scalar::Int64(2), Scalar::Int64(1)]
            );
        }

        #[test]
        fn value_counts_dropna_false_includes_missing_bucket() {
            let col = Column::from_values(vec![
                Scalar::Utf8("a".into()),
                Scalar::Null(NullKind::NaN),
                Scalar::Utf8("a".into()),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");

            let (values, counts) = col
                .value_counts_with_options(false, true, false, false)
                .expect("value_counts");
            assert_eq!(values.values()[0], Scalar::Utf8("a".into()));
            assert!(values.values()[1].is_missing());
            assert_eq!(counts.values(), &[Scalar::Int64(2), Scalar::Int64(2)]);
        }

        #[test]
        fn value_counts_normalize_uses_returned_total() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");

            let (values, counts) = col
                .value_counts_with_options(true, true, false, true)
                .expect("value_counts");
            assert_eq!(
                values.values(),
                &[Scalar::Float64(1.0), Scalar::Float64(2.0)]
            );
            assert_eq!(counts.dtype(), DType::Float64);
            assert_eq!(
                counts.values(),
                &[Scalar::Float64(2.0 / 3.0), Scalar::Float64(1.0 / 3.0)]
            );
        }
    }
}
