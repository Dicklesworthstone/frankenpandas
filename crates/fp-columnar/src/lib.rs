#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! Columnar storage layer for **frankenpandas** — provides the
//! [`Column`] container that backs every `DataFrame` column and
//! `Series` value buffer in fp-frame.
//!
//! A column is a typed value buffer ([`DType`]) plus a separate
//! [`ValidityMask`] tracking which cells are missing. This split
//! mirrors Apache Arrow's storage layout and lets the type system
//! enforce correctness on the dense-value side while keeping
//! pandas-style missing-value semantics ([`NullKind::Null`],
//! [`NullKind::NaN`], [`NullKind::NaT`]) on the validity side.
//!
//! ## Public surface
//!
//! - [`Column`]: the public columnar container. Built from a
//!   [`DType`] + a `Vec<Scalar>`. Exposes value access
//!   ([`Column::value`], [`Column::values`]), reductions
//!   ([`Column::sum`], [`Column::mean`], [`Column::count`], the
//!   nan-aware aggregations from fp-types), and typed binary
//!   operations dispatched through [`ArithmeticOp`] /
//!   [`ComparisonOp`].
//! - [`ColumnData`]: the inner enum holding the dense buffer. Most
//!   callers go through `Column` rather than touching this directly.
//! - [`SparseColumn`]: opt-in sparse encoding (paired value buffer +
//!   index-of-non-fill positions). Stored alongside the dense
//!   `Column` for backwards compat when consumers only need
//!   [`Column`].
//! - [`ValidityMask`]: per-cell missing-value bitmap. Stored on
//!   [`Column`]; exposed for users that want to compose masks
//!   directly (logical masking, conditional updates, etc.).
//! - [`ArithmeticOp`] / [`ComparisonOp`]: enum tags for typed
//!   binary-op dispatch (used by fp-frame's expression engine and
//!   Series arithmetic).
//! - [`CrackIndex`]: an internal positional index used by the
//!   "cracking" optimisation for repeated boolean-mask filters.
//!
//! ## Error reporting
//!
//! [`ColumnError`] enumerates the failure modes (length mismatch,
//! dtype mismatch, missing-value-in-required-slot, etc.). All
//! Column-mutating fns return `Result<_, ColumnError>` so callers
//! get explicit error categories.
//!
//! ## Relationship to other crates
//!
//! - **fp-types** supplies the [`DType`] / [`Scalar`] /
//!   [`NullKind`] / `nan*` reduction primitives this crate composes
//!   on top of.
//! - **fp-frame** stores a `Vec<Column>` per `DataFrame` (one column
//!   per data column) plus a separate `Index` from fp-index for the
//!   row labels.
//! - **fp-index** uses [`Column`] internally for some MultiIndex
//!   level storage.

use std::sync::{Arc, OnceLock};

use fp_types::{
    DType, Interval, IntervalClosed, NullKind, Scalar, SparseDType, Timedelta, Timestamp,
    TypeError, cast_scalar, cast_scalar_owned, common_dtype, infer_dtype, nanall, nanany,
    nanargmax, nanargmin, nancummax, nancummin, nancumprod, nancumsum, nankurt, nanmax, nanmean,
    nanmedian, nanmin, nannunique, nanprod, nanptp, nanquantile, nansem, nanskew, nanstd, nansum,
    nanvar,
};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Eq)]
pub struct ValidityMask {
    words: Vec<u64>,
    len: usize,
}

impl ValidityMask {
    fn is_all_valid_sentinel(&self) -> bool {
        self.len > 0 && self.words.is_empty()
    }

    fn materialized_all_valid_words(len: usize) -> Vec<u64> {
        let word_count = len.div_ceil(64);
        let mut words = vec![u64::MAX; word_count];
        let remainder = len % 64;
        if remainder > 0
            && let Some(last) = words.last_mut()
        {
            *last = (1_u64 << remainder) - 1;
        }
        words
    }

    fn words_are_all_valid(words: &[u64], len: usize) -> bool {
        if len == 0 {
            return words.is_empty();
        }
        let word_count = len.div_ceil(64);
        if words.len() != word_count {
            return false;
        }
        let full_words = len / 64;
        if words.iter().take(full_words).any(|&word| word != u64::MAX) {
            return false;
        }
        let remainder = len % 64;
        if remainder == 0 {
            return true;
        }
        words.get(full_words).copied() == Some((1_u64 << remainder) - 1)
    }

    fn materialize_if_all_valid_sentinel(&mut self) {
        if self.is_all_valid_sentinel() {
            self.words = Self::materialized_all_valid_words(self.len);
        }
    }

    #[must_use]
    pub fn from_values(values: &[Scalar]) -> Self {
        let len = values.len();
        let word_count = len.div_ceil(64);
        let mut words = vec![0_u64; word_count];
        let mut all_valid = true;
        for (idx, value) in values.iter().enumerate() {
            if !value.is_missing() {
                words[idx / 64] |= 1_u64 << (idx % 64);
            } else {
                all_valid = false;
            }
        }
        if all_valid {
            return Self::all_valid(len);
        }
        Self { words, len }
    }

    /// Build a validity mask from a contiguous `f64` buffer, marking NaN
    /// positions invalid. pandas treats float NaN as missing, so this mirrors
    /// what `from_values` would produce for the equivalent `Scalar::Float64`
    /// values (`Scalar::is_missing` is true for NaN). See
    /// [`Column::from_f64_values`].
    #[must_use]
    pub fn from_f64(data: &[f64]) -> Self {
        let len = data.len();
        let word_count = len.div_ceil(64);
        let mut words = vec![0_u64; word_count];
        let mut all_valid = true;
        for (idx, &v) in data.iter().enumerate() {
            if !v.is_nan() {
                words[idx / 64] |= 1_u64 << (idx % 64);
            } else {
                all_valid = false;
            }
        }
        if all_valid {
            return Self::all_valid(len);
        }
        Self { words, len }
    }

    #[must_use]
    pub fn all_valid(len: usize) -> Self {
        Self {
            words: Vec::new(),
            len,
        }
    }

    /// Build a mask from pre-packed validity words (LSB-first within each
    /// word, bit `i` of word `i / 64` = row `i` valid). Bits at positions
    /// `>= len` must be zero. Public (hidden) for typed builders that compute
    /// validity in bulk (br-frankenpandas-7wxoc).
    #[must_use]
    #[doc(hidden)]
    pub fn from_words(words: Vec<u64>, len: usize) -> Self {
        debug_assert_eq!(words.len(), len.div_ceil(64));
        debug_assert!(
            len.is_multiple_of(64) || words.last().is_none_or(|w| w >> (len % 64) == 0),
            "validity bits beyond len must be zero"
        );
        if Self::words_are_all_valid(&words, len) {
            return Self::all_valid(len);
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
        if self.is_all_valid_sentinel() {
            return true;
        }
        (self.words[idx / 64] >> (idx % 64)) & 1 == 1
    }

    pub fn set(&mut self, idx: usize, value: bool) {
        if idx >= self.len {
            return;
        }
        if self.is_all_valid_sentinel() {
            if value {
                return;
            }
            self.materialize_if_all_valid_sentinel();
        }
        if value {
            self.words[idx / 64] |= 1_u64 << (idx % 64);
        } else {
            self.words[idx / 64] &= !(1_u64 << (idx % 64));
        }
    }

    #[must_use]
    pub fn count_valid(&self) -> usize {
        if self.is_all_valid_sentinel() {
            return self.len;
        }
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
        if len == 0 {
            return Self::all_invalid(0);
        }
        if self.is_all_valid_sentinel() && other.is_all_valid_sentinel() {
            return Self::all_valid(len);
        }
        if self.is_all_valid_sentinel() {
            return other.slice(0, len);
        }
        if other.is_all_valid_sentinel() {
            return self.slice(0, len);
        }
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
        if len == 0 {
            return Self::all_invalid(0);
        }
        if self.is_all_valid_sentinel() || other.is_all_valid_sentinel() {
            return Self::all_valid(len);
        }
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
        if self.is_all_valid_sentinel() {
            return Self::all_invalid(self.len);
        }
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

    /// Number of invalid (cleared) positions.
    #[must_use]
    pub fn count_invalid(&self) -> usize {
        self.len.saturating_sub(self.count_valid())
    }

    /// Whether any bit is set.
    #[must_use]
    pub fn any(&self) -> bool {
        if self.is_all_valid_sentinel() {
            return true;
        }
        self.count_valid() > 0
    }

    /// Whether all bits are set.
    #[must_use]
    pub fn all(&self) -> bool {
        if self.is_all_valid_sentinel() {
            return true;
        }
        self.count_valid() == self.len
    }

    /// Bitwise XOR (symmetric difference) with another mask.
    ///
    /// Produced length is `min(self.len, other.len)`.
    #[must_use]
    pub fn xor_mask(&self, other: &Self) -> Self {
        let len = self.len.min(other.len);
        if len == 0 {
            return Self::all_invalid(0);
        }
        if self.is_all_valid_sentinel() && other.is_all_valid_sentinel() {
            return Self::all_invalid(len);
        }
        if self.is_all_valid_sentinel() {
            return other.slice(0, len).not_mask();
        }
        if other.is_all_valid_sentinel() {
            return self.slice(0, len).not_mask();
        }
        let word_count = len.div_ceil(64);
        let mut words: Vec<u64> = self.words[..word_count]
            .iter()
            .zip(&other.words[..word_count])
            .map(|(a, b)| a ^ b)
            .collect();
        let remainder = len % 64;
        if remainder > 0 && !words.is_empty() {
            let last = words.len() - 1;
            words[last] &= (1_u64 << remainder) - 1;
        }
        Self { words, len }
    }

    /// Extract a contiguous sub-range as a new mask.
    ///
    /// `start..start+len` is clamped to the available tail — callers
    /// don't need to pre-validate against `self.len`.
    #[must_use]
    pub fn slice(&self, start: usize, len: usize) -> Self {
        if start >= self.len {
            return Self::all_invalid(0);
        }
        let effective_len = len.min(self.len - start);
        if self.is_all_valid_sentinel() {
            return Self::all_valid(effective_len);
        }
        let mut out = Self::all_invalid(effective_len);
        for i in 0..effective_len {
            if self.get(start + i) {
                out.set(i, true);
            }
        }
        out
    }

    /// Append another mask's bits to the end of this one.
    #[must_use]
    pub fn concat(&self, other: &Self) -> Self {
        let total = self.len + other.len;
        if self.all() && other.all() {
            return Self::all_valid(total);
        }
        let mut out = Self::all_invalid(total);
        for i in 0..self.len {
            if self.get(i) {
                out.set(i, true);
            }
        }
        for i in 0..other.len {
            if other.get(i) {
                out.set(self.len + i, true);
            }
        }
        out
    }

    /// Position of the first valid bit.
    #[must_use]
    pub fn first_valid(&self) -> Option<usize> {
        if self.is_all_valid_sentinel() {
            return Some(0);
        }
        (0..self.len).find(|&i| self.get(i))
    }

    /// Position of the last valid bit.
    #[must_use]
    pub fn last_valid(&self) -> Option<usize> {
        if self.is_all_valid_sentinel() {
            return Some(self.len - 1);
        }
        (0..self.len).rev().find(|&i| self.get(i))
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
        Ok(Self::from_words(words, len))
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
    Datetime64(Vec<i64>),
    Period(Vec<i64>),
    Interval(Vec<Interval>),
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
            DType::Int64 | DType::Int64Nullable => {
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
            DType::Bool | DType::BoolNullable => {
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
            DType::Sparse => Self::Utf8(vec![String::new(); values.len()]),
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
            DType::Datetime64 => {
                let data: Vec<i64> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Datetime64(n) => *n,
                        Scalar::Int64(i) => *i,
                        _ => Timestamp::NAT,
                    })
                    .collect();
                Self::Datetime64(data)
            }
            DType::Period => {
                let data: Vec<i64> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Period(n) => *n,
                        Scalar::Int64(i) => *i,
                        _ => i64::MIN, // NaT sentinel for Period
                    })
                    .collect();
                Self::Period(data)
            }
            DType::Interval => {
                let data: Vec<Interval> = values
                    .iter()
                    .map(|v| match v {
                        Scalar::Interval(interval) => *interval,
                        _ => Interval::new(0.0, 0.0, IntervalClosed::Right),
                    })
                    .collect();
                Self::Interval(data)
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
            Self::Datetime64(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) || *v == Timestamp::NAT {
                        Scalar::Datetime64(Timestamp::NAT)
                    } else {
                        Scalar::Datetime64(*v)
                    }
                })
                .collect(),
            Self::Period(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) || *v == i64::MIN {
                        Scalar::Period(i64::MIN)
                    } else {
                        Scalar::Period(*v)
                    }
                })
                .collect(),
            Self::Interval(data) => data
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if !validity.get(i) {
                        Scalar::missing_for_dtype(dtype)
                    } else {
                        Scalar::Interval(*v)
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
            Self::Datetime64(d) => d.len(),
            Self::Period(d) => d.len(),
            Self::Interval(d) => d.len(),
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
    let apply = binary_f64_apply(op);

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

fn binary_f64_apply(op: ArithmeticOp) -> fn(f64, f64) -> f64 {
    match op {
        ArithmeticOp::Add => |a, b| a + b,
        ArithmeticOp::Sub => |a, b| a - b,
        ArithmeticOp::Mul => |a, b| a * b,
        ArithmeticOp::Div => |a, b| a / b,
        ArithmeticOp::Mod => python_mod_f64,
        ArithmeticOp::Pow => |a, b| a.powf(b),
        ArithmeticOp::FloorDiv => python_floor_div_f64,
    }
}

/// Apply a binary f64 op over two equal-length slices with the operation
/// monomorphized into each arm (br-frankenpandas-f64simd). Unlike a
/// `fn(f64,f64)->f64` pointer applied per element, the closed-form arms let LLVM
/// autovectorize Add/Sub/Mul/Div to packed SIMD; Mod/Pow/FloorDiv keep their
/// scalar helpers but still avoid the indirect call. Element-for-element
/// identical to `binary_f64_apply(op)` applied in order.
#[inline]
fn apply_f64_slices(op: ArithmeticOp, a: &[f64], b: &[f64]) -> Vec<f64> {
    match op {
        ArithmeticOp::Add => a.iter().zip(b).map(|(x, y)| x + y).collect(),
        ArithmeticOp::Sub => a.iter().zip(b).map(|(x, y)| x - y).collect(),
        ArithmeticOp::Mul => a.iter().zip(b).map(|(x, y)| x * y).collect(),
        ArithmeticOp::Div => a.iter().zip(b).map(|(x, y)| x / y).collect(),
        ArithmeticOp::Mod => a
            .iter()
            .zip(b)
            .map(|(x, y)| python_mod_f64(*x, *y))
            .collect(),
        ArithmeticOp::Pow => a.iter().zip(b).map(|(x, y)| x.powf(*y)).collect(),
        ArithmeticOp::FloorDiv => a
            .iter()
            .zip(b)
            .map(|(x, y)| python_floor_div_f64(*x, *y))
            .collect(),
    }
}

fn unit_range_len(start: i64, end: i64) -> Option<usize> {
    usize::try_from(end.checked_sub(start)?.checked_add(1)?).ok()
}

fn python_mod_f64(lhs: f64, rhs: f64) -> f64 {
    if lhs.is_nan() || rhs.is_nan() {
        return f64::NAN;
    }

    if rhs.is_infinite() {
        if lhs.is_infinite() {
            return f64::NAN;
        }
        if lhs == 0.0 {
            return 0.0_f64.copysign(rhs);
        }
        if lhs.is_sign_positive() == rhs.is_sign_positive() {
            lhs
        } else {
            rhs
        }
    } else {
        lhs - python_floor_div_f64(lhs, rhs) * rhs
    }
}

fn python_floor_div_f64(lhs: f64, rhs: f64) -> f64 {
    if lhs.is_nan() || rhs.is_nan() {
        return f64::NAN;
    }

    if rhs.is_infinite() {
        if lhs.is_infinite() {
            return f64::NAN;
        }
        if lhs == 0.0 {
            return (lhs / rhs).floor();
        }
        if lhs.is_sign_positive() == rhs.is_sign_positive() {
            0.0
        } else {
            -1.0
        }
    } else if lhs.is_infinite() && rhs != 0.0 {
        f64::NAN
    } else {
        (lhs / rhs).floor()
    }
}

fn python_floor_div_i64(lhs: i64, rhs: i64) -> i64 {
    debug_assert_ne!(rhs, 0);
    if lhs == i64::MIN && rhs == -1 {
        return i64::MIN;
    }

    let quotient = lhs / rhs;
    let remainder = lhs % rhs;
    if remainder != 0 && ((remainder > 0) != (rhs > 0)) {
        quotient - 1
    } else {
        quotient
    }
}

fn python_mod_i64(lhs: i64, rhs: i64) -> i64 {
    debug_assert_ne!(rhs, 0);
    if lhs == i64::MIN && rhs == -1 {
        return 0;
    }

    let quotient = i128::from(python_floor_div_i64(lhs, rhs));
    let remainder = i128::from(lhs) - quotient * i128::from(rhs);
    let Ok(value) = i64::try_from(remainder) else {
        return 0;
    };
    value
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

    // For Mod/FloorDiv: if any non-missing right operand is zero, fall back
    // to float. We must NOT gate on `combined` (left AND right validity) —
    // pandas promotes the entire result dtype to Float64 whenever a zero
    // divisor appears in the right operand, regardless of whether the
    // matching left position is missing. Gating on combined caused
    // promotion to be skipped when the zero divisor's left counterpart
    // was Null, drifting the column dtype against the conformance oracle
    // (fuzz_column_arith corpus surfaced this on the seed
    // [97, 4, 11, 0, 0, 0, 0, 0, 0, 0, 10]).
    if matches!(op, ArithmeticOp::Mod | ArithmeticOp::FloorDiv) {
        let has_zero_divisor = right
            .iter()
            .enumerate()
            .any(|(i, &r)| right_validity.get(i) && r == 0);
        if has_zero_divisor {
            return None;
        }
    }

    let apply: fn(i64, i64) -> i64 = match op {
        ArithmeticOp::Add => |a, b| a.wrapping_add(b),
        ArithmeticOp::Sub => |a, b| a.wrapping_sub(b),
        ArithmeticOp::Mul => |a, b| a.wrapping_mul(b),
        ArithmeticOp::Mod => python_mod_i64,
        ArithmeticOp::FloorDiv => python_floor_div_i64,
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

enum ScalarValues {
    Eager(Vec<Scalar>),
    LazyAllValidInt64 {
        data: Arc<[i64]>,
        values: OnceLock<Vec<Scalar>>,
    },
    LazyAllValidFloat64 {
        data: Arc<[f64]>,
        values: OnceLock<Vec<Scalar>>,
    },
    LazyNullableFloat64 {
        data: Vec<f64>,
        validity: ValidityMask,
        values: OnceLock<Vec<Scalar>>,
    },
    /// Nullable Int64 mirror of `LazyNullableFloat64`
    /// (br-frankenpandas-lt5qx): contiguous `i64` data + validity, where an
    /// invalid slot materializes `Scalar::Null(NullKind::Null)` — exactly
    /// `Scalar::missing_for_dtype(DType::Int64)`. Unlike Float64 there is no
    /// NaN-as-missing ambiguity: missingness is the validity bit alone.
    LazyNullableInt64 {
        data: Vec<i64>,
        validity: ValidityMask,
        values: OnceLock<Vec<Scalar>>,
    },
    LazyAllValidBool {
        data: Arc<[bool]>,
        values: OnceLock<Vec<Scalar>>,
    },
    /// Contiguous backing for all-valid Utf8 columns
    /// (br-frankenpandas-2krr0): one rolling byte buffer + n+1 offsets (row
    /// `i` = `bytes[offsets[i]..offsets[i+1]]`, always valid UTF-8 by
    /// construction — only built from `&str` data). String-output ops write
    /// here without a per-row heap `String`; the `Vec<Scalar::Utf8>` view
    /// materializes once on demand.
    ///
    /// The byte buffer and offsets are `Arc`-shared (br-frankenpandas-oifvy):
    /// the backing is immutable after construction (string ops always build a
    /// fresh buffer, never mutate in place), so `Column::clone` shares the
    /// `Arc` instead of deep-copying the (often large) byte buffer — O(1)
    /// instead of O(n), and observationally a deep copy because the data can
    /// never change underneath a shared reader.
    LazyContiguousUtf8 {
        bytes: Arc<[u8]>,
        offsets: Arc<[usize]>,
        strictly_increasing: OnceLock<bool>,
        fixed_width: OnceLock<Option<usize>>,
        values: OnceLock<Vec<Scalar>>,
    },
    /// Run-length backing for all-valid Int64 columns whose values arrive as
    /// repeat runs (br-frankenpandas-3ad4n) — e.g. the left lanes of a dense
    /// inner join, where every matched left value repeats `bucket_len` times.
    /// Carries O(runs) memory until a consumer asks for the contiguous i64
    /// buffer (`as_i64_slice`) or the Scalar view, each expanded once on
    /// demand.
    LazyRepeatRunsInt64 {
        runs: Vec<(i64, usize)>,
        total_len: usize,
        data: OnceLock<Vec<i64>>,
        values: OnceLock<Vec<Scalar>>,
    },
    /// Repeat-run backing where the per-run values and run lengths are stored
    /// separately so multiple dense-join output lanes can share one immutable
    /// run-length descriptor.
    LazyRepeatValuesInt64 {
        run_values: Vec<i64>,
        run_lens: Arc<[usize]>,
        total_len: usize,
        data: OnceLock<Vec<i64>>,
        values: OnceLock<Vec<Scalar>>,
    },
    /// Repeated-slice backing for all-valid Int64 columns whose values arrive
    /// as slices of one shared tape (e.g. dense join right lanes). Each
    /// segment is `(start, len)` into `data`, and segment order is the
    /// observable row order.
    LazyRepeatedSlicesInt64 {
        data: Vec<i64>,
        segments: Arc<[(usize, usize)]>,
        total_len: usize,
        expanded: OnceLock<Vec<i64>>,
        values: OnceLock<Vec<Scalar>>,
    },
    /// Zero-copy contiguous row-range VIEW over an `Arc`-shared contiguous-Utf8
    /// backing (br-frankenpandas-jbyuc.1.1.1). Row `i` (`0..len`) is
    /// `bytes[offsets[start + i] .. offsets[start + i + 1]]` — the same shared
    /// `bytes`/`offsets` as the source `LazyContiguousUtf8`, with `start`/`len`
    /// selecting a contiguous window. `take_positions` returns this in O(1)
    /// (two `Arc::clone`s) when the requested positions are a contiguous
    /// ascending range, deferring the per-row byte gather until a consumer
    /// actually materializes the Scalar view. The byte content is identical to
    /// the eager gather: same bytes, same order, all-valid.
    LazyUtf8Slice {
        bytes: Arc<[u8]>,
        offsets: Arc<[usize]>,
        start: usize,
        len: usize,
        values: OnceLock<Vec<Scalar>>,
    },
}

impl ScalarValues {
    fn from_vec(values: Vec<Scalar>) -> Self {
        Self::Eager(values)
    }

    fn lazy_all_valid_int64(data: Vec<i64>) -> Self {
        Self::lazy_all_valid_int64_arc(Arc::from(data))
    }

    /// Share an existing `Arc` i64 buffer in O(1) (used by `Clone`).
    /// (br-frankenpandas-tin7r: immutable typed buffers clone by Arc, the
    /// numeric mirror of the utf8 oifvy lever.)
    fn lazy_all_valid_int64_arc(data: Arc<[i64]>) -> Self {
        Self::LazyAllValidInt64 {
            data,
            values: OnceLock::new(),
        }
    }

    fn lazy_all_valid_float64(data: Vec<f64>) -> Self {
        Self::lazy_all_valid_float64_arc(Arc::from(data))
    }

    /// Share an existing `Arc` f64 buffer in O(1) (used by `Clone`).
    fn lazy_all_valid_float64_arc(data: Arc<[f64]>) -> Self {
        Self::LazyAllValidFloat64 {
            data,
            values: OnceLock::new(),
        }
    }

    fn lazy_nullable_float64(data: Vec<f64>, validity: ValidityMask) -> Self {
        Self::LazyNullableFloat64 {
            data,
            validity,
            values: OnceLock::new(),
        }
    }

    fn lazy_nullable_int64(data: Vec<i64>, validity: ValidityMask) -> Self {
        Self::LazyNullableInt64 {
            data,
            validity,
            values: OnceLock::new(),
        }
    }

    fn lazy_all_valid_bool(data: Vec<bool>) -> Self {
        Self::lazy_all_valid_bool_arc(Arc::from(data))
    }

    /// Share an existing `Arc` bool buffer in O(1) (used by `Clone`).
    fn lazy_all_valid_bool_arc(data: Arc<[bool]>) -> Self {
        Self::LazyAllValidBool {
            data,
            values: OnceLock::new(),
        }
    }

    fn lazy_contiguous_utf8(bytes: Vec<u8>, offsets: Vec<usize>) -> Self {
        debug_assert!(!offsets.is_empty(), "offsets must hold n+1 entries");
        debug_assert_eq!(*offsets.last().expect("non-empty"), bytes.len());
        Self::lazy_contiguous_utf8_arc(Arc::from(bytes), Arc::from(offsets))
    }

    /// Construct a `LazyContiguousUtf8` from already-`Arc`-shared buffers,
    /// sharing them in O(1) instead of re-allocating. Used by `Clone` so two
    /// contiguous-Utf8 columns can share one immutable byte buffer
    /// (br-frankenpandas-oifvy). The witness caches start fresh — they are pure
    /// functions of the (identical) shared buffers, so a clone recomputes the
    /// same value lazily if asked.
    fn lazy_contiguous_utf8_arc(bytes: Arc<[u8]>, offsets: Arc<[usize]>) -> Self {
        debug_assert!(!offsets.is_empty(), "offsets must hold n+1 entries");
        debug_assert_eq!(*offsets.last().expect("non-empty"), bytes.len());
        Self::LazyContiguousUtf8 {
            bytes,
            offsets,
            strictly_increasing: OnceLock::new(),
            fixed_width: OnceLock::new(),
            values: OnceLock::new(),
        }
    }

    /// Build a zero-copy contiguous row-range view over a shared contiguous-Utf8
    /// backing (br-frankenpandas-jbyuc.1.1.1). Rows `start..start+len` of the
    /// source become rows `0..len` of the view. Shares `bytes`/`offsets` in
    /// O(1); the Scalar view materializes on demand.
    fn lazy_utf8_slice(bytes: Arc<[u8]>, offsets: Arc<[usize]>, start: usize, len: usize) -> Self {
        debug_assert!(
            start + len + 1 <= offsets.len(),
            "view window must lie within the source offsets"
        );
        Self::LazyUtf8Slice {
            bytes,
            offsets,
            start,
            len,
            values: OnceLock::new(),
        }
    }

    fn lazy_repeat_runs_int64(runs: Vec<(i64, usize)>, total_len: usize) -> Self {
        debug_assert_eq!(
            runs.iter().map(|&(_, run_len)| run_len).sum::<usize>(),
            total_len
        );
        Self::LazyRepeatRunsInt64 {
            runs,
            total_len,
            data: OnceLock::new(),
            values: OnceLock::new(),
        }
    }

    fn lazy_repeat_values_int64(
        run_values: Vec<i64>,
        run_lens: Arc<[usize]>,
        total_len: usize,
    ) -> Self {
        debug_assert_eq!(run_values.len(), run_lens.len());
        debug_assert_eq!(run_lens.iter().sum::<usize>(), total_len);
        Self::LazyRepeatValuesInt64 {
            run_values,
            run_lens,
            total_len,
            data: OnceLock::new(),
            values: OnceLock::new(),
        }
    }

    fn lazy_repeated_slices_int64(
        data: Vec<i64>,
        segments: Vec<(usize, usize)>,
        total_len: usize,
    ) -> Self {
        Self::lazy_repeated_slices_int64_shared(data, Arc::from(segments), total_len)
    }

    fn lazy_repeated_slices_int64_shared(
        data: Vec<i64>,
        segments: Arc<[(usize, usize)]>,
        total_len: usize,
    ) -> Self {
        debug_assert_eq!(
            segments.iter().map(|&(_, len)| len).sum::<usize>(),
            total_len
        );
        debug_assert!(
            segments
                .iter()
                .all(|&(start, len)| start.checked_add(len).is_some_and(|end| end <= data.len()))
        );
        Self::LazyRepeatedSlicesInt64 {
            data,
            segments,
            total_len,
            expanded: OnceLock::new(),
            values: OnceLock::new(),
        }
    }

    fn expand_repeat_values_i64(
        run_values: &[i64],
        run_lens: &[usize],
        total_len: usize,
    ) -> Vec<i64> {
        const PARALLEL_MIN_VALUES: usize = 1 << 18;
        const PARALLEL_MAX_CHUNKS: usize = 16;

        debug_assert_eq!(run_values.len(), run_lens.len());
        let thread_count = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(PARALLEL_MAX_CHUNKS);
        if total_len < PARALLEL_MIN_VALUES || thread_count < 2 || run_values.is_empty() {
            let mut out = Vec::with_capacity(total_len);
            for (&value, &run_len) in run_values.iter().zip(run_lens.iter()) {
                out.resize(out.len() + run_len, value);
            }
            return out;
        }

        let target = total_len.div_ceil(thread_count).max(1);
        let mut boundaries = vec![(0usize, 0usize)];
        let mut cumulative = 0usize;
        let mut next_target = target;
        for (run_idx, &run_len) in run_lens.iter().enumerate() {
            cumulative += run_len;
            if cumulative >= next_target && run_idx + 1 < run_lens.len() {
                boundaries.push((run_idx + 1, cumulative));
                next_target = cumulative.saturating_add(target);
            }
        }
        debug_assert_eq!(cumulative, total_len);
        boundaries.push((run_lens.len(), total_len));

        let mut out = vec![0i64; total_len];
        let mut chunk_slices = Vec::with_capacity(boundaries.len() - 1);
        let mut rest: &mut [i64] = out.as_mut_slice();
        let mut prev = 0usize;
        for window in boundaries.windows(2) {
            let (chunk_slice, tail) = rest.split_at_mut(window[1].1 - prev);
            prev = window[1].1;
            rest = tail;
            chunk_slices.push(chunk_slice);
        }

        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(chunk_slices.len());
            for (chunk_idx, chunk_slice) in chunk_slices.into_iter().enumerate() {
                let (run_start, _) = boundaries[chunk_idx];
                let (run_end, _) = boundaries[chunk_idx + 1];
                let run_values = &run_values[run_start..run_end];
                let run_lens = &run_lens[run_start..run_end];
                handles.push(scope.spawn(move || {
                    let mut cursor = 0usize;
                    for (&value, &run_len) in run_values.iter().zip(run_lens.iter()) {
                        chunk_slice[cursor..cursor + run_len].fill(value);
                        cursor += run_len;
                    }
                    debug_assert_eq!(cursor, chunk_slice.len());
                }));
            }
            for handle in handles {
                handle
                    .join()
                    .expect("repeat-value expansion worker must not panic");
            }
        });
        out
    }

    fn expand_repeated_slices_i64(
        data: &[i64],
        segments: &[(usize, usize)],
        total_len: usize,
    ) -> Vec<i64> {
        const PARALLEL_MIN_VALUES: usize = 1 << 18;
        const PARALLEL_MAX_CHUNKS: usize = 16;

        let thread_count = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(PARALLEL_MAX_CHUNKS);
        if total_len < PARALLEL_MIN_VALUES || thread_count < 2 || segments.is_empty() {
            let mut out = Vec::with_capacity(total_len);
            for &(start, len) in segments {
                out.extend_from_slice(&data[start..start + len]);
            }
            return out;
        }

        let target = total_len.div_ceil(thread_count).max(1);
        let mut boundaries = vec![(0usize, 0usize)];
        let mut cumulative = 0usize;
        let mut next_target = target;
        for (segment_idx, &(_, len)) in segments.iter().enumerate() {
            cumulative += len;
            if cumulative >= next_target && segment_idx + 1 < segments.len() {
                boundaries.push((segment_idx + 1, cumulative));
                next_target = cumulative.saturating_add(target);
            }
        }
        debug_assert_eq!(cumulative, total_len);
        boundaries.push((segments.len(), total_len));

        let mut out = vec![0i64; total_len];
        let mut chunk_slices = Vec::with_capacity(boundaries.len() - 1);
        let mut rest: &mut [i64] = out.as_mut_slice();
        let mut prev = 0usize;
        for window in boundaries.windows(2) {
            let (chunk_slice, tail) = rest.split_at_mut(window[1].1 - prev);
            prev = window[1].1;
            rest = tail;
            chunk_slices.push(chunk_slice);
        }

        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(chunk_slices.len());
            for (chunk_idx, chunk_slice) in chunk_slices.into_iter().enumerate() {
                let (segment_start, _) = boundaries[chunk_idx];
                let (segment_end, _) = boundaries[chunk_idx + 1];
                let segments = &segments[segment_start..segment_end];
                handles.push(scope.spawn(move || {
                    let mut cursor = 0usize;
                    for &(start, len) in segments {
                        chunk_slice[cursor..cursor + len]
                            .copy_from_slice(&data[start..start + len]);
                        cursor += len;
                    }
                    debug_assert_eq!(cursor, chunk_slice.len());
                }));
            }
            for handle in handles {
                handle
                    .join()
                    .expect("repeated-slice expansion worker must not panic");
            }
        });
        out
    }

    /// The expanded contiguous `i64` buffer of a repeat-run backing, built
    /// once on first request. `None` for every other representation.
    ///
    /// Large expansions are row-chunked across scoped threads (disjoint
    /// `split_at_mut` slices, same scheme as the dense join fill) so a
    /// consumer that forces materialization pays the same parallel fill the
    /// eager path would have, not a serial one.
    fn repeat_runs_i64_data(&self) -> Option<&[i64]> {
        const PARALLEL_MIN_VALUES: usize = 1 << 18;
        const PARALLEL_MAX_CHUNKS: usize = 16;

        if let Self::LazyRepeatRunsInt64 {
            runs,
            total_len,
            data,
            ..
        } = self
        {
            return Some(
                data.get_or_init(|| {
                    let thread_count = std::thread::available_parallelism()
                        .map_or(1, usize::from)
                        .min(PARALLEL_MAX_CHUNKS);
                    if *total_len < PARALLEL_MIN_VALUES || thread_count < 2 || runs.is_empty() {
                        let mut out = Vec::with_capacity(*total_len);
                        for &(value, run_len) in runs {
                            out.resize(out.len() + run_len, value);
                        }
                        return out;
                    }

                    // Chunk boundaries (run_idx, out_pos) balanced by output
                    // size; each worker fills its disjoint output slice.
                    let target = total_len.div_ceil(thread_count).max(1);
                    let mut boundaries = vec![(0usize, 0usize)];
                    let mut cumulative = 0usize;
                    let mut next_target = target;
                    for (run_idx, &(_, run_len)) in runs.iter().enumerate() {
                        cumulative += run_len;
                        if cumulative >= next_target && run_idx + 1 < runs.len() {
                            boundaries.push((run_idx + 1, cumulative));
                            next_target = cumulative.saturating_add(target);
                        }
                    }
                    debug_assert_eq!(cumulative, *total_len);
                    boundaries.push((runs.len(), *total_len));

                    let mut out = vec![0i64; *total_len];
                    let mut chunk_slices = Vec::with_capacity(boundaries.len() - 1);
                    let mut rest: &mut [i64] = out.as_mut_slice();
                    let mut prev = 0usize;
                    for window in boundaries.windows(2) {
                        let (chunk_slice, tail) = rest.split_at_mut(window[1].1 - prev);
                        prev = window[1].1;
                        rest = tail;
                        chunk_slices.push(chunk_slice);
                    }

                    std::thread::scope(|scope| {
                        let mut handles = Vec::with_capacity(chunk_slices.len());
                        for (chunk_idx, chunk_slice) in chunk_slices.into_iter().enumerate() {
                            let (run_start, _) = boundaries[chunk_idx];
                            let (run_end, _) = boundaries[chunk_idx + 1];
                            let runs = &runs[run_start..run_end];
                            handles.push(scope.spawn(move || {
                                let mut cursor = 0usize;
                                for &(value, run_len) in runs {
                                    chunk_slice[cursor..cursor + run_len].fill(value);
                                    cursor += run_len;
                                }
                                debug_assert_eq!(cursor, chunk_slice.len());
                            }));
                        }
                        for handle in handles {
                            handle
                                .join()
                                .expect("repeat-run expansion worker must not panic");
                        }
                    });
                    out
                })
                .as_slice(),
            );
        }
        if let Self::LazyRepeatValuesInt64 {
            run_values,
            run_lens,
            total_len,
            data,
            ..
        } = self
        {
            return Some(
                data.get_or_init(|| {
                    Self::expand_repeat_values_i64(run_values, run_lens, *total_len)
                })
                .as_slice(),
            );
        }
        None
    }

    fn repeated_slices_i64_data(&self) -> Option<&[i64]> {
        if let Self::LazyRepeatedSlicesInt64 {
            data,
            segments,
            total_len,
            expanded,
            ..
        } = self
        {
            return Some(
                expanded
                    .get_or_init(|| Self::expand_repeated_slices_i64(data, segments, *total_len))
                    .as_slice(),
            );
        }
        None
    }

    fn as_slice(&self) -> &[Scalar] {
        match self {
            Self::Eager(values) => values,
            Self::LazyAllValidInt64 { data, values } => values
                .get_or_init(|| data.iter().copied().map(Scalar::Int64).collect())
                .as_slice(),
            Self::LazyAllValidFloat64 { data, values } => values
                .get_or_init(|| data.iter().copied().map(Scalar::Float64).collect())
                .as_slice(),
            Self::LazyNullableFloat64 {
                data,
                validity,
                values,
            } => values
                .get_or_init(|| {
                    data.iter()
                        .enumerate()
                        .map(|(idx, value)| {
                            if validity.get(idx) || value.is_nan() {
                                Scalar::Float64(*value)
                            } else {
                                Scalar::Null(NullKind::NaN)
                            }
                        })
                        .collect()
                })
                .as_slice(),
            Self::LazyAllValidBool { data, values } => values
                .get_or_init(|| data.iter().copied().map(Scalar::Bool).collect())
                .as_slice(),
            Self::LazyContiguousUtf8 {
                bytes,
                offsets,
                values,
                ..
            } => values
                .get_or_init(|| {
                    offsets
                        .windows(2)
                        .map(|w| {
                            Scalar::Utf8(
                                std::str::from_utf8(&bytes[w[0]..w[1]])
                                    .expect("contiguous utf8 buffer is valid by construction")
                                    .to_owned(),
                            )
                        })
                        .collect()
                })
                .as_slice(),
            Self::LazyNullableInt64 {
                data,
                validity,
                values,
            } => values
                .get_or_init(|| {
                    data.iter()
                        .enumerate()
                        .map(|(idx, value)| {
                            if validity.get(idx) {
                                Scalar::Int64(*value)
                            } else {
                                Scalar::Null(NullKind::Null)
                            }
                        })
                        .collect()
                })
                .as_slice(),
            Self::LazyRepeatRunsInt64 {
                runs,
                total_len,
                values,
                ..
            } => values
                .get_or_init(|| {
                    let mut out = Vec::with_capacity(*total_len);
                    for &(value, run_len) in runs {
                        out.resize(out.len() + run_len, Scalar::Int64(value));
                    }
                    out
                })
                .as_slice(),
            Self::LazyRepeatValuesInt64 {
                run_values,
                run_lens,
                total_len,
                values,
                ..
            } => values
                .get_or_init(|| {
                    let mut out = Vec::with_capacity(*total_len);
                    for (&value, &run_len) in run_values.iter().zip(run_lens.iter()) {
                        out.resize(out.len() + run_len, Scalar::Int64(value));
                    }
                    out
                })
                .as_slice(),
            Self::LazyRepeatedSlicesInt64 {
                data,
                segments,
                total_len,
                values,
                ..
            } => values
                .get_or_init(|| {
                    Self::expand_repeated_slices_i64(data, segments, *total_len)
                        .into_iter()
                        .map(Scalar::Int64)
                        .collect()
                })
                .as_slice(),
            Self::LazyUtf8Slice {
                bytes,
                offsets,
                start,
                len,
                values,
            } => values
                .get_or_init(|| {
                    (0..*len)
                        .map(|i| {
                            let lo = offsets[start + i];
                            let hi = offsets[start + i + 1];
                            Scalar::Utf8(
                                std::str::from_utf8(&bytes[lo..hi])
                                    .expect("contiguous utf8 buffer is valid by construction")
                                    .to_owned(),
                            )
                        })
                        .collect()
                })
                .as_slice(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Eager(values) => values.len(),
            Self::LazyAllValidInt64 { data, .. } => data.len(),
            Self::LazyAllValidFloat64 { data, .. } => data.len(),
            Self::LazyNullableFloat64 { data, .. } => data.len(),
            Self::LazyAllValidBool { data, .. } => data.len(),
            Self::LazyContiguousUtf8 { offsets, .. } => offsets.len() - 1,
            Self::LazyNullableInt64 { data, .. } => data.len(),
            Self::LazyRepeatRunsInt64 { total_len, .. } => *total_len,
            Self::LazyRepeatValuesInt64 { total_len, .. } => *total_len,
            Self::LazyRepeatedSlicesInt64 { total_len, .. } => *total_len,
            Self::LazyUtf8Slice { len, .. } => *len,
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn contiguous_utf8_offsets_are_strictly_increasing(bytes: &[u8], offsets: &[usize]) -> bool {
    let Some(n) = offsets.len().checked_sub(1) else {
        return false;
    };
    if n < 2 {
        return true;
    }

    let mut previous = &bytes[offsets[0]..offsets[1]];
    for pos in 1..n {
        let current = &bytes[offsets[pos]..offsets[pos + 1]];
        if previous >= current {
            return false;
        }
        previous = current;
    }
    true
}

fn contiguous_utf8_fixed_width(offsets: &[usize]) -> Option<usize> {
    let n = offsets.len().checked_sub(1)?;
    if n == 0 {
        return Some(0);
    }
    let width = offsets[1].checked_sub(offsets[0])?;
    for pos in 1..n {
        if offsets[pos + 1].checked_sub(offsets[pos])? != width {
            return None;
        }
    }
    Some(width)
}

/// If `positions` is a non-empty contiguous ascending run
/// (`positions[i] == positions[0] + i`), return its start. Returns `None` for
/// an empty slice or the first out-of-sequence position — so a non-contiguous
/// take pays only until the first gap (typically O(1)).
/// (br-frankenpandas-jbyuc.1.1.1)
fn contiguous_ascending_start(positions: &[usize]) -> Option<usize> {
    let first = *positions.first()?;
    for (i, &pos) in positions.iter().enumerate() {
        if pos != first + i {
            return None;
        }
    }
    Some(first)
}

impl Clone for ScalarValues {
    fn clone(&self) -> Self {
        match self {
            Self::Eager(values) => Self::Eager(values.clone()),
            Self::LazyAllValidInt64 { data, .. } => Self::lazy_all_valid_int64_arc(Arc::clone(data)),
            Self::LazyAllValidFloat64 { data, .. } => {
                Self::lazy_all_valid_float64_arc(Arc::clone(data))
            }
            Self::LazyNullableFloat64 { data, validity, .. } => {
                Self::lazy_nullable_float64(data.clone(), validity.clone())
            }
            Self::LazyAllValidBool { data, .. } => Self::lazy_all_valid_bool_arc(Arc::clone(data)),
            Self::LazyContiguousUtf8 { bytes, offsets, .. } => {
                Self::lazy_contiguous_utf8_arc(Arc::clone(bytes), Arc::clone(offsets))
            }
            Self::LazyNullableInt64 { data, validity, .. } => {
                Self::lazy_nullable_int64(data.clone(), validity.clone())
            }
            Self::LazyRepeatRunsInt64 {
                runs, total_len, ..
            } => Self::lazy_repeat_runs_int64(runs.clone(), *total_len),
            Self::LazyRepeatValuesInt64 {
                run_values,
                run_lens,
                total_len,
                ..
            } => {
                Self::lazy_repeat_values_int64(run_values.clone(), Arc::clone(run_lens), *total_len)
            }
            Self::LazyRepeatedSlicesInt64 {
                data,
                segments,
                total_len,
                ..
            } => Self::lazy_repeated_slices_int64_shared(
                data.clone(),
                Arc::clone(segments),
                *total_len,
            ),
            Self::LazyUtf8Slice {
                bytes,
                offsets,
                start,
                len,
                ..
            } => Self::lazy_utf8_slice(Arc::clone(bytes), Arc::clone(offsets), *start, *len),
        }
    }
}

impl std::ops::Deref for ScalarValues {
    type Target = [Scalar];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a ScalarValues {
    type Item = &'a Scalar;
    type IntoIter = std::slice::Iter<'a, Scalar>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl PartialEq for ScalarValues {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl std::fmt::Debug for ScalarValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl Serialize for ScalarValues {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ScalarValues {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<Scalar>::deserialize(deserializer).map(Self::Eager)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Column {
    dtype: DType,
    values: ScalarValues,
    validity: ValidityMask,
    #[serde(skip)]
    data: Option<ColumnData>,
}

impl Clone for Column {
    fn clone(&self) -> Self {
        Self {
            dtype: self.dtype,
            values: self
                .clone_dense_values_from_cache()
                .unwrap_or_else(|| self.values.clone()),
            validity: self.validity.clone(),
            data: None,
        }
    }
}

impl PartialEq for Column {
    fn eq(&self, other: &Self) -> bool {
        self.dtype == other.dtype && self.values == other.values && self.validity == other.validity
    }
}

impl std::fmt::Debug for Column {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Column")
            .field("dtype", &self.dtype)
            .field("values", &self.values)
            .field("validity", &self.validity)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseColumn {
    dtype: SparseDType,
    len: usize,
    indices: Vec<usize>,
    values: Vec<Scalar>,
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

fn nkeep_impl(col: &Column, n: usize, keep: &str, ascending: bool) -> Result<Column, ColumnError> {
    if !matches!(keep, "first" | "last" | "all") {
        return Err(ColumnError::Type(TypeError::NonNumericValue {
            value: keep.to_string(),
            dtype: col.dtype(),
        }));
    }
    // Annotate each value with its original position, then sort
    // (position is the secondary key; Rust's sort_by is stable, so
    // "first" falls out for free on equal primary keys).
    let mut indexed: Vec<(usize, &Scalar)> = col.values().iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        let primary = compare_scalars_na_last(a.1, b.1, ascending);
        match (primary, keep) {
            // "last" policy: on ties, prefer later positions.
            (std::cmp::Ordering::Equal, "last") => b.0.cmp(&a.0),
            (std::cmp::Ordering::Equal, _) => a.0.cmp(&b.0),
            _ => primary,
        }
    });
    let take = n.min(indexed.len());
    let mut end = take;
    if keep == "all" && take > 0 && take < indexed.len() {
        let boundary = indexed[take - 1].1;
        while end < indexed.len() {
            let same = compare_scalars_na_last(indexed[end].1, boundary, ascending).is_eq();
            if !same {
                break;
            }
            end += 1;
        }
    }
    let values: Vec<Scalar> = indexed[..end].iter().map(|(_, v)| (*v).clone()).collect();
    Column::new(col.dtype(), values)
}

fn is_monotonic_in_direction(values: &[Scalar], increasing: bool) -> bool {
    let mut prev: Option<&Scalar> = None;
    for v in values {
        if v.is_missing() {
            continue;
        }
        if let Some(p) = prev {
            let ord = compare_scalars_na_last(p, v, true);
            // `p` should come before `v` in the requested direction. With
            // ascending compare: Less/Equal → non-decreasing OK; Greater
            // breaks. For decreasing we flip the expectation.
            let ok = matches!(
                (ord, increasing),
                (std::cmp::Ordering::Less, true)
                    | (std::cmp::Ordering::Equal, _)
                    | (std::cmp::Ordering::Greater, false)
            );
            if !ok {
                return false;
            }
        }
        prev = Some(v);
    }
    true
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

/// `keep=` policy for `duplicated`/`drop_duplicates`.
#[derive(Clone, Copy)]
enum DupPolicy {
    First,
    Last,
    None,
}

/// Per-element duplicate flags over a contiguous slice of `Copy + Hash + Eq`
/// keys, using a fast (`FxHashSet`) hasher — the typed counterpart to the
/// `Scalar`-enum + SipHash path. Semantics are identical: `First` flags every
/// occurrence after the first; `Last` flags every occurrence before the last;
/// `None` flags every occurrence of any key that appears more than once.
fn duplicated_flags_typed<T>(keys: &[T], policy: DupPolicy) -> Vec<bool>
where
    T: std::hash::Hash + Eq + Copy,
{
    let n = keys.len();
    let mut flags = vec![false; n];
    match policy {
        DupPolicy::First => {
            let mut seen: FxHashSet<T> = FxHashSet::with_capacity_and_hasher(n, Default::default());
            for (idx, &k) in keys.iter().enumerate() {
                flags[idx] = !seen.insert(k);
            }
        }
        DupPolicy::Last => {
            let mut seen: FxHashSet<T> = FxHashSet::with_capacity_and_hasher(n, Default::default());
            for (idx, &k) in keys.iter().enumerate().rev() {
                flags[idx] = !seen.insert(k);
            }
        }
        DupPolicy::None => {
            let mut seen_once: FxHashSet<T> =
                FxHashSet::with_capacity_and_hasher(n, Default::default());
            let mut seen_multiple: FxHashSet<T> = FxHashSet::default();
            for &k in keys {
                if !seen_once.insert(k) {
                    seen_multiple.insert(k);
                }
            }
            for (idx, &k) in keys.iter().enumerate() {
                flags[idx] = seen_multiple.contains(&k);
            }
        }
    }
    flags
}

/// Largest direct-address table we will allocate for integer dedup (entries).
/// At 16M entries the `seen`/`count` table is ~16MB — L3-resident on a typical
/// server — and dedup becomes a hash-free O(n) scan. Beyond this the table
/// stops being cache-friendly and we fall back to the FxHash set.
const DUP_DIRECT_ADDRESS_CAP: u128 = 1 << 24;

/// Min and table size for a direct-address integer dedup, or `None` when the
/// value span is too wide (sparse) to be worth a dense table. We also require
/// the table to be at most ~16x the row count so a handful of widely-separated
/// values don't trigger a giant allocation.
fn i64_direct_address_range(data: &[i64]) -> Option<(i64, usize)> {
    let mut min = data.first().copied()?;
    let mut max = min;
    for &v in &data[1..] {
        if v < min {
            min = v;
        } else if v > max {
            max = v;
        }
    }
    let range = (max as i128 - min as i128 + 1) as u128;
    if range <= DUP_DIRECT_ADDRESS_CAP && range <= (data.len() as u128).saturating_mul(16) {
        Some((min, range as usize))
    } else {
        None
    }
}

/// Hash-free duplicate flags for a bounded-range `i64` slice via a dense
/// direct-address table (no per-element hashing or `Scalar` enum). Identical
/// semantics to [`duplicated_flags_typed`]. `min`/`range` come from
/// [`i64_direct_address_range`], so `(v - min)` is always in `0..range`.
fn duplicated_flags_i64_direct(
    data: &[i64],
    min: i64,
    range: usize,
    policy: DupPolicy,
) -> Vec<bool> {
    let n = data.len();
    let mut flags = vec![false; n];
    let slot = |v: i64| (v as i128 - min as i128) as usize;
    match policy {
        DupPolicy::First => {
            let mut seen = vec![false; range];
            for (idx, &v) in data.iter().enumerate() {
                let s = slot(v);
                flags[idx] = seen[s];
                seen[s] = true;
            }
        }
        DupPolicy::Last => {
            let mut seen = vec![false; range];
            for (idx, &v) in data.iter().enumerate().rev() {
                let s = slot(v);
                flags[idx] = seen[s];
                seen[s] = true;
            }
        }
        DupPolicy::None => {
            // Saturating occupancy count (we only care about 1 vs >1).
            let mut count = vec![0u8; range];
            for &v in data {
                let s = slot(v);
                if count[s] < 2 {
                    count[s] += 1;
                }
            }
            for (idx, &v) in data.iter().enumerate() {
                flags[idx] = count[slot(v)] > 1;
            }
        }
    }
    flags
}

/// Map an `i64` to an order-preserving `u64` radix key (flip the sign bit so
/// two's-complement negatives sort below non-negatives in unsigned order).
#[inline]
fn i64_radix_key(value: i64) -> u64 {
    (value as u64) ^ (1u64 << 63)
}

/// Map an `f64` to an order-preserving `u64` radix key. For a non-negative
/// value flip only the sign bit; for a negative value flip every bit. This is
/// the standard IEEE-754 "sortable bits" transform and is monotonic across the
/// whole finite range (callers guarantee no NaN — `as_f64_slice` only yields
/// all-valid buffers and FP models NaN as missing). `-0.0` and `+0.0` map to
/// distinct keys but compare equal under the comparator path; we normalize
/// `-0.0`→`+0.0` first so the radix order matches `partial_cmp` exactly.
#[inline]
fn f64_radix_key(value: f64) -> u64 {
    let bits = (if value == 0.0 { 0.0 } else { value }).to_bits();
    if bits & (1u64 << 63) != 0 {
        !bits
    } else {
        bits | (1u64 << 63)
    }
}

/// Stable LSD radix argsort over pre-computed `u64` keys (8 passes of 8-bit
/// counting sort). Returns the permutation `perm` such that
/// `keys[perm[0]] <= keys[perm[1]] <= ...`, with equal keys keeping their
/// original relative order (stability == the comparator path's tie behavior).
/// O(n) per pass, comparison-free — replaces the O(n log n) `Scalar`-enum
/// comparator for all-valid numeric columns.
fn radix_argsort_u64(keys: &[u64]) -> Vec<usize> {
    let n = keys.len();
    let mut idx: Vec<usize> = (0..n).collect();
    if n < 2 {
        return idx;
    }
    let mut scratch: Vec<usize> = vec![0; n];
    for shift in (0..64).step_by(8) {
        let mut count = [0usize; 256];
        for &k in keys {
            count[((k >> shift) & 0xff) as usize] += 1;
        }
        // Skip a pass whose byte is constant across the whole column (common
        // for clustered / small-magnitude data) — keeps `idx` in place.
        if count.contains(&n) {
            continue;
        }
        let mut running = 0usize;
        for slot in &mut count {
            let c = *slot;
            *slot = running;
            running += c;
        }
        for &i in &idx {
            let bucket = ((keys[i] >> shift) & 0xff) as usize;
            scratch[count[bucket]] = i;
            count[bucket] += 1;
        }
        std::mem::swap(&mut idx, &mut scratch);
    }
    idx
}

/// Stable LSD radix argsort of an `i64` slice (br-frankenpandas-y5s15): the
/// permutation that orders `values` ascending (or descending), equal values
/// keeping their original order. Bit-identical to a stable `sort_by(i64::cmp)`:
/// `i64_radix_key` is order-preserving and the counting sort is stable;
/// descending flips the key (`!key`) so equal values still keep original order
/// (matching a reversed comparator whose `Equal` arm doesn't reorder). Reusable
/// for any all-Int64 ordering (index labels, single columns).
#[must_use]
pub fn radix_argsort_i64(values: &[i64], ascending: bool) -> Vec<usize> {
    let keys: Vec<u64> = if ascending {
        values.iter().map(|&v| i64_radix_key(v)).collect()
    } else {
        values.iter().map(|&v| !i64_radix_key(v)).collect()
    };
    radix_argsort_u64(&keys)
}

/// Stable LSD radix lexsort over several `u64` key columns
/// (br-frankenpandas-lnsu6). Returns the permutation that orders rows
/// lexicographically by `keys_by_col[0]`, then `keys_by_col[1]`, …, with equal
/// rows keeping their original order — exactly a stable multi-key `sort_by`.
/// The least-significant digit overall is the last column's low byte, so the
/// columns are processed in reverse (each an 8-pass stable counting sort that
/// threads the running permutation), making the *first* column the most
/// significant. O(n·k) and comparison-free. All key vectors must have the same
/// length; callers bake per-column ascending/descending into the keys.
pub fn radix_argsort_multi_u64(keys_by_col: &[Vec<u64>]) -> Vec<usize> {
    let n = keys_by_col.first().map_or(0, Vec::len);
    let mut idx: Vec<usize> = (0..n).collect();
    if n < 2 || keys_by_col.is_empty() {
        return idx;
    }
    let mut scratch: Vec<usize> = vec![0; n];
    for keys in keys_by_col.iter().rev() {
        for shift in (0..64).step_by(8) {
            let mut count = [0usize; 256];
            for &k in keys {
                count[((k >> shift) & 0xff) as usize] += 1;
            }
            if count.contains(&n) {
                continue;
            }
            let mut running = 0usize;
            for slot in &mut count {
                let c = *slot;
                *slot = running;
                running += c;
            }
            for &i in &idx {
                let bucket = ((keys[i] >> shift) & 0xff) as usize;
                scratch[count[bucket]] = i;
                count[bucket] += 1;
            }
            std::mem::swap(&mut idx, &mut scratch);
        }
    }
    idx
}

/// Stable MSD byte-radix argsort over UTF-8 strings.
///
/// Produces the exact permutation of a stable `sort_by` with `String::cmp`
/// (byte-lexicographic, shorter-prefix-first), comparison-free at scale:
/// each level counting-sorts the bucket by the byte at `depth`, with a
/// virtual end-of-string bucket ordered before every byte (ascending) /
/// after every byte (descending) — exactly `cmp`'s prefix rule. Counting
/// scatters preserve relative order and the small-bucket cutoff uses the
/// stable `sort_by` on the (equal-prefix-stripped) suffix, so ties keep
/// their original order at every level, matching the stable comparison
/// sort bit-for-bit in both directions.
fn utf8_msd_argsort(strs: &[&str], ascending: bool) -> Vec<usize> {
    let n = strs.len();
    let mut idx: Vec<usize> = (0..n).collect();
    if n > 1 {
        let mut aux: Vec<usize> = vec![0; n];
        utf8_msd_sort_range(strs, &mut idx, &mut aux, 0, n, 0, ascending);
    }
    idx
}

fn utf8_msd_sort_range(
    strs: &[&str],
    idx: &mut [usize],
    aux: &mut [usize],
    lo: usize,
    hi: usize,
    depth: usize,
    ascending: bool,
) {
    let n = hi - lo;
    if n <= 1 {
        return;
    }
    // Small buckets (and pathologically deep shared prefixes, which bound
    // recursion depth) finish with the stable comparison sort on the suffix:
    // every string in this bucket shares its first `depth` bytes, so suffix
    // order equals full-string order.
    const CUTOFF: usize = 48;
    const MAX_DEPTH: usize = 1024;
    if n <= CUTOFF || depth >= MAX_DEPTH {
        idx[lo..hi].sort_by(|&a, &b| {
            let ord = strs[a].as_bytes()[depth..].cmp(&strs[b].as_bytes()[depth..]);
            if ascending { ord } else { ord.reverse() }
        });
        return;
    }
    // Bucket keys ordered so iterating 0..=256 visits buckets in output order:
    // ascending — EOS first (0), then bytes 1..=256;
    // descending — bytes reversed (255-b), then EOS last (256).
    let key = |s: &str| -> usize {
        let b = s.as_bytes();
        if depth < b.len() {
            if ascending {
                b[depth] as usize + 1
            } else {
                255 - b[depth] as usize
            }
        } else if ascending {
            0
        } else {
            256
        }
    };
    let mut counts = [0usize; 258];
    for &i in idx[lo..hi].iter() {
        counts[key(strs[i]) + 1] += 1;
    }
    for k in 1..258 {
        counts[k] += counts[k - 1];
    }
    // counts[k] = start offset of bucket k within [lo, hi).
    let mut offsets = counts;
    for &i in idx[lo..hi].iter() {
        let k = key(strs[i]);
        aux[lo + offsets[k]] = i;
        offsets[k] += 1;
    }
    idx[lo..hi].copy_from_slice(&aux[lo..hi]);
    // Recurse into byte buckets; the EOS bucket holds fully-equal strings
    // (same first `depth` bytes and length == depth) already in original
    // relative order — nothing to sort.
    let eos_bucket = if ascending { 0 } else { 256 };
    for k in 0..257 {
        if k == eos_bucket {
            continue;
        }
        let b_lo = lo + counts[k];
        let b_hi = lo + counts[k + 1];
        if b_hi - b_lo > 1 {
            utf8_msd_sort_range(strs, idx, aux, b_lo, b_hi, depth + 1, ascending);
        }
    }
}

fn normalized_float_bits(value: f64) -> u64 {
    let normalized = if value == 0.0 { 0.0 } else { value };
    normalized.to_bits()
}

fn interval_key(interval: &Interval) -> (u64, u64, IntervalClosed) {
    (
        normalized_float_bits(interval.left),
        normalized_float_bits(interval.right),
        interval.closed,
    )
}

/// Hashable membership key for a non-missing scalar — the same equivalence
/// `Column::unique` uses (Float64 ±0.0 normalized to one key). `None` for
/// missing values. Lets the np set-ops (`setdiff1d`/`intersect1d`/`setxor1d`/
/// `in1d`) replace an O(N·M) linear `semantic_eq` scan over the other operand
/// with an O(1) hash-set probe. Because every operand is first passed through
/// `unique()` (which dedups by this exact key) and missing/NaN values are
/// filtered out, key equality matches the `semantic_eq` test on the values that
/// actually flow through.
#[derive(Hash, PartialEq, Eq)]
enum SetMemberKey<'a> {
    Bool(bool),
    Int64(i64),
    FloatBits(u64),
    Utf8(&'a str),
    Timedelta64(i64),
    Datetime64(i64),
    Period(i64),
    Interval(u64, u64, IntervalClosed),
}

fn set_member_key(v: &Scalar) -> Option<SetMemberKey<'_>> {
    Some(match v {
        Scalar::Bool(b) => SetMemberKey::Bool(*b),
        Scalar::Int64(i) => SetMemberKey::Int64(*i),
        Scalar::Float64(f) => {
            let norm = if *f == 0.0 { 0.0 } else { *f };
            SetMemberKey::FloatBits(norm.to_bits())
        }
        Scalar::Utf8(s) => SetMemberKey::Utf8(s.as_str()),
        Scalar::Timedelta64(v) => SetMemberKey::Timedelta64(*v),
        Scalar::Datetime64(v) => SetMemberKey::Datetime64(*v),
        Scalar::Period(v) => SetMemberKey::Period(*v),
        Scalar::Interval(v) => {
            let (left, right, closed) = interval_key(v);
            SetMemberKey::Interval(left, right, closed)
        }
        Scalar::Null(_) => return None,
    })
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ColumnError {
    #[error("column length mismatch: left={left}, right={right}")]
    LengthMismatch { left: usize, right: usize },
    #[error("{operation} requires exactly {expected} element(s), got {actual}")]
    InvalidLength {
        operation: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("invalid sorter permutation for column of length {len}: {reason}")]
    InvalidSorter { len: usize, reason: String },
    #[error("mask must be Bool dtype; found {dtype:?}")]
    InvalidMaskType { dtype: DType },
    #[error("column dtype mismatch: left={left:?}, right={right:?}")]
    DTypeMismatch { left: DType, right: DType },
    #[error("Integers to negative integer powers are not allowed.")]
    NegativeIntegerPower,
    #[error(transparent)]
    Type(#[from] TypeError),
}

impl SparseColumn {
    pub fn from_dense(dtype: SparseDType, values: Vec<Scalar>) -> Result<Self, ColumnError> {
        let len = values.len();
        let value_dtype = dtype.value_dtype;
        let fill_value = dtype.fill_value.clone();
        let mut indices = Vec::new();
        let mut sparse_values = Vec::new();

        for (idx, value) in values.into_iter().enumerate() {
            let value = if value.dtype() == value_dtype || value.dtype() == DType::Null {
                Column::normalize_missing_for_dtype(value, value_dtype)
            } else {
                cast_scalar_owned(value, value_dtype)?
            };

            if !value.semantic_eq(&fill_value) {
                indices.push(idx);
                sparse_values.push(value);
            }
        }

        Ok(Self {
            dtype,
            len,
            indices,
            values: sparse_values,
        })
    }

    pub fn from_dense_column(dtype: SparseDType, column: &Column) -> Result<Self, ColumnError> {
        Self::from_dense(dtype, column.values().to_vec())
    }

    #[must_use]
    pub fn sparse_dtype(&self) -> &SparseDType {
        &self.dtype
    }

    #[must_use]
    pub fn value_dtype(&self) -> DType {
        self.dtype.value_dtype
    }

    #[must_use]
    pub fn fill_value(&self) -> &Scalar {
        &self.dtype.fill_value
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
    pub fn indices(&self) -> &[usize] {
        &self.indices
    }

    #[must_use]
    pub fn stored_values(&self) -> &[Scalar] {
        &self.values
    }

    #[must_use]
    pub fn npoints(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn density(&self) -> f64 {
        if self.len == 0 {
            0.0
        } else {
            self.values.len() as f64 / self.len as f64
        }
    }

    #[must_use]
    pub fn to_dense_values(&self) -> Vec<Scalar> {
        let mut values = vec![self.dtype.fill_value.clone(); self.len];
        for (&idx, value) in self.indices.iter().zip(self.values.iter()) {
            values[idx] = value.clone();
        }
        values
    }

    pub fn to_dense_column(&self) -> Result<Column, ColumnError> {
        Column::new(self.dtype.value_dtype, self.to_dense_values())
    }
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

fn round_i64_negative_decimals(value: i64, decimals: i32) -> i64 {
    debug_assert!(decimals < 0);
    let factor = match 10_i128.checked_pow(decimals.unsigned_abs()) {
        Some(factor) => factor,
        None => return 0,
    };
    let magnitude = i128::from(value).abs();
    let quotient = magnitude / factor;
    let remainder = magnitude % factor;
    let rounded_magnitude = match (remainder * 2).cmp(&factor) {
        std::cmp::Ordering::Less => quotient * factor,
        std::cmp::Ordering::Greater => (quotient + 1) * factor,
        std::cmp::Ordering::Equal if quotient % 2 == 0 => quotient * factor,
        std::cmp::Ordering::Equal => (quotient + 1) * factor,
    };
    let rounded = if value < 0 {
        -rounded_magnitude
    } else {
        rounded_magnitude
    };
    match i64::try_from(rounded) {
        Ok(value) => value,
        Err(_) if rounded < 0 => i64::MIN,
        Err(_) => i64::MAX,
    }
}

impl Column {
    fn clone_dense_values_from_cache(&self) -> Option<ScalarValues> {
        if self.validity.len() != self.values.len()
            || self.validity.count_valid() != self.values.len()
        {
            return None;
        }

        match (&self.data, self.dtype) {
            (Some(ColumnData::Bool(data)), DType::Bool)
                if data.len() == self.values.len() =>
            {
                // Carry the contiguous bool buffer through the clone as a lazy
                // all-valid backing (mirrors the Float64 arm) so `as_bool_slice`
                // stays available on the clone — otherwise every bool dense fast
                // path (filter masks, duplicated, isin) bails after a clone. The
                // Scalar view materializes identically (`map(Scalar::Bool)`) on
                // demand. BoolNullable is excluded: an all-valid clone of a
                // nullable-bool column must stay Eager so its dtype-tagged Scalar
                // view is preserved.
                Some(ScalarValues::lazy_all_valid_bool(data.clone()))
            }
            (Some(ColumnData::Int64(data)), DType::Int64)
                if data.len() == self.values.len() =>
            {
                // Carry the contiguous i64 buffer through the clone as a lazy
                // all-valid backing (mirrors the Float64 arm) so `as_i64_slice`
                // stays available on the clone. Previously the clone eagerly
                // materialized `Vec<Scalar::Int64>`, which both cost a full
                // Scalar build AND dropped slice availability — so cloned Int64
                // columns silently missed every dense/direct-address fast path
                // (groupby, value_counts, dedup, joins). Bit-identical Scalar
                // view (`map(Scalar::Int64)`) materializes on demand.
                // Int64Nullable is excluded to preserve its dtype-tagged view.
                Some(ScalarValues::lazy_all_valid_int64(data.clone()))
            }
            (Some(ColumnData::Float64(data)), DType::Float64)
                if data.len() == self.values.len() =>
            {
                Some(ScalarValues::lazy_all_valid_float64(data.clone()))
            }
            (Some(ColumnData::Timedelta64(data)), DType::Timedelta64)
                if data.len() == self.values.len() =>
            {
                Some(ScalarValues::from_vec(
                    data.iter().copied().map(Scalar::Timedelta64).collect(),
                ))
            }
            (Some(ColumnData::Datetime64(data)), DType::Datetime64)
                if data.len() == self.values.len() =>
            {
                Some(ScalarValues::from_vec(
                    data.iter().copied().map(Scalar::Datetime64).collect(),
                ))
            }
            (Some(ColumnData::Period(data)), DType::Period) if data.len() == self.values.len() => {
                Some(ScalarValues::from_vec(
                    data.iter().copied().map(Scalar::Period).collect(),
                ))
            }
            _ => None,
        }
    }

    fn cached_data_for_values(dtype: DType, values: &[Scalar]) -> Option<ColumnData> {
        match dtype {
            DType::Bool
            | DType::BoolNullable
            | DType::Int64
            | DType::Int64Nullable
            | DType::Float64
            | DType::Timedelta64
            | DType::Datetime64
            | DType::Period => Some(ColumnData::from_scalars(values, dtype)),
            _ => None,
        }
    }

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
                .map(|value| {
                    // Constructing a typed column with an explicit dtype is
                    // STRICT: a non-integer float cannot be coerced to int64
                    // (pandas DataFrame(dtype='int64') raises "Trying to coerce
                    // float values to integers"), UNLIKE astype which truncates
                    // toward zero. astype pre-truncates via cast_scalar, so this
                    // coercion path only ever sees raw floats from the explicit
                    // constructor. (br-frankenpandas-8nupg)
                    if matches!(dtype, DType::Int64 | DType::Int64Nullable)
                        && let Scalar::Float64(v) = &value
                        && v.is_finite()
                        && v.fract() != 0.0
                    {
                        return Err(TypeError::LossyFloatToInt { value: *v });
                    }
                    cast_scalar_owned(value, dtype)
                })
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
            validity,
            data: Self::cached_data_for_values(dtype, &coerced),
            values: ScalarValues::from_vec(coerced),
        })
    }

    pub fn from_values(values: Vec<Scalar>) -> Result<Self, ColumnError> {
        let dtype = infer_dtype(&values)?;
        Self::new(dtype, values)
    }

    /// Build an all-valid Int64 column from already-typed contiguous values.
    ///
    /// This carries parser/vector-kernel dtype proofs directly into the
    /// columnar representation and delays `Scalar` materialization until a
    /// caller explicitly asks for scalar values.
    #[must_use]
    pub fn from_i64_values(data: Vec<i64>) -> Self {
        let len = data.len();
        Self {
            dtype: DType::Int64,
            values: ScalarValues::lazy_all_valid_int64(data),
            validity: ValidityMask::all_valid(len),
            data: None,
        }
    }

    /// Build an all-valid Utf8 column from a contiguous byte buffer + n+1
    /// offsets (br-frankenpandas-2krr0). `bytes[offsets[i]..offsets[i+1]]`
    /// must be valid UTF-8 for every row — string-output ops guarantee this
    /// by writing only `&str` data. Semantically identical to
    /// `Column::new(DType::Utf8, scalars)` over the same strings, but the
    /// per-row `String`/`Scalar` boxing is deferred until a consumer reads
    /// the Scalar view.
    #[must_use]
    #[doc(hidden)]
    pub fn from_utf8_contiguous(bytes: Vec<u8>, offsets: Vec<usize>) -> Self {
        let len = offsets.len().saturating_sub(1);
        Self {
            dtype: DType::Utf8,
            values: ScalarValues::lazy_contiguous_utf8(bytes, offsets),
            validity: ValidityMask::all_valid(len),
            data: None,
        }
    }

    /// Build an all-valid Int64 column from `(value, run_len)` repeat runs
    /// (br-frankenpandas-3ad4n). Semantically identical to
    /// `from_i64_values(expanded)` where `expanded` repeats each `value`
    /// `run_len` times, but carries only O(runs) memory until a consumer
    /// forces the contiguous buffer or Scalar view.
    #[must_use]
    #[doc(hidden)]
    pub fn from_i64_repeat_runs(runs: Vec<(i64, usize)>) -> Self {
        let total_len = runs.iter().map(|&(_, run_len)| run_len).sum();
        Self {
            dtype: DType::Int64,
            values: ScalarValues::lazy_repeat_runs_int64(runs, total_len),
            validity: ValidityMask::all_valid(total_len),
            data: None,
        }
    }

    /// Build an all-valid Int64 column from per-run values plus a shared
    /// run-length descriptor. Semantically identical to
    /// [`Column::from_i64_repeat_runs`] over `run_values.zip(run_lens)`, but
    /// dense join lanes can share `run_lens` across columns.
    #[must_use]
    #[doc(hidden)]
    pub fn from_i64_repeat_values_run_lengths(
        run_values: Vec<i64>,
        run_lens: Arc<[usize]>,
    ) -> Self {
        let total_len = run_lens.iter().sum();
        Self {
            dtype: DType::Int64,
            values: ScalarValues::lazy_repeat_values_int64(run_values, run_lens, total_len),
            validity: ValidityMask::all_valid(total_len),
            data: None,
        }
    }

    /// Build an all-valid Int64 column from repeated slices of one shared
    /// tape. Semantically identical to concatenating
    /// `data[start..start+len]` for each segment and calling
    /// [`Column::from_i64_values`].
    #[must_use]
    #[doc(hidden)]
    pub fn from_i64_repeated_slices(data: Vec<i64>, segments: Vec<(usize, usize)>) -> Self {
        let total_len = segments.iter().map(|&(_, len)| len).sum();
        Self {
            dtype: DType::Int64,
            values: ScalarValues::lazy_repeated_slices_int64(data, segments, total_len),
            validity: ValidityMask::all_valid(total_len),
            data: None,
        }
    }

    /// Shared-descriptor counterpart of [`Column::from_i64_repeated_slices`].
    /// `total_len` must equal the sum of all segment lengths.
    #[must_use]
    #[doc(hidden)]
    pub fn from_i64_repeated_slices_shared(
        data: Vec<i64>,
        segments: Arc<[(usize, usize)]>,
        total_len: usize,
    ) -> Self {
        Self {
            dtype: DType::Int64,
            values: ScalarValues::lazy_repeated_slices_int64_shared(data, segments, total_len),
            validity: ValidityMask::all_valid(total_len),
            data: None,
        }
    }

    /// Build an all-valid Float64 column from already-typed contiguous values.
    ///
    /// This is the typed ingestion counterpart to `Column::new(DType::Float64,
    /// Vec<Scalar>)` for sources that have already proven every value is a
    /// valid f64.
    #[must_use]
    pub fn from_f64_values(data: Vec<f64>) -> Self {
        let len = data.len();
        // pandas treats NaN in a float column as MISSING. The Scalar path
        // (Column::new -> ValidityMask::from_values) already marks NaN invalid
        // via Scalar::is_missing, so this typed-ingestion path must agree —
        // otherwise a caller passing NaN gets a column claiming all-valid and
        // as_f64_slice would hand the NaN out as a real value. Fast all-valid
        // path (cheap u64::MAX fill) when no NaN is present; per-bit mask only
        // when one is. (br-frankenpandas-jyhf7)
        let validity = if data.iter().any(|v| v.is_nan()) {
            ValidityMask::from_f64(&data)
        } else {
            ValidityMask::all_valid(len)
        };
        Self {
            dtype: DType::Float64,
            values: ScalarValues::lazy_all_valid_float64(data),
            validity,
            data: None,
        }
    }

    /// Public (hidden) for fp-join's fused dense outer-merge builder
    /// (br-frankenpandas-343ho); invalid slots carry the 0.0-datum convention
    /// and materialize `Scalar::Null(NullKind::NaN)`.
    #[doc(hidden)]
    pub fn from_f64_values_with_validity(data: Vec<f64>, validity: ValidityMask) -> Self {
        debug_assert_eq!(data.len(), validity.len());
        if validity.all() {
            return Self::from_f64_values(data);
        }
        Self {
            dtype: DType::Float64,
            values: ScalarValues::lazy_nullable_float64(data, validity.clone()),
            validity,
            data: None,
        }
    }

    /// Nullable Int64 counterpart of `from_f64_values_with_validity`
    /// (br-frankenpandas-lt5qx): invalid slots materialize
    /// `Scalar::Null(NullKind::Null)` (= `missing_for_dtype(Int64)`), valid
    /// slots `Scalar::Int64(data[i])`. Public (hidden) for fp-join's fused
    /// dense left-merge builder (br-frankenpandas-7wxoc).
    #[doc(hidden)]
    pub fn from_i64_values_with_validity(data: Vec<i64>, validity: ValidityMask) -> Self {
        debug_assert_eq!(data.len(), validity.len());
        if validity.all() {
            return Self::from_i64_values(data);
        }
        Self {
            dtype: DType::Int64,
            values: ScalarValues::lazy_nullable_int64(data, validity.clone()),
            validity,
            data: None,
        }
    }

    /// Null-introducing positional reindex with Float64 promotion
    /// (br-frankenpandas-1bvcl): gather an all-valid Int64/Float64 column by
    /// `Option<usize>` positions into a nullable Float64 column without the
    /// per-row `Scalar` clone + `cast_scalar_owned` + `Column::new`
    /// revalidation. `None` (or out-of-range) slots take the established
    /// aligned-binary gap convention — 0.0 datum + invalid bit — which
    /// materializes `Scalar::Null(NullKind::NaN)`, exactly what the eager
    /// path's `missing_for_dtype(Float64)` cast produces; matched Int64 slots
    /// use `v as f64`, the same conversion as the `cast_scalar_owned`
    /// Int64->Float64 arm. Returns `None` for any other source (nullable,
    /// non-numeric), where the caller's `Scalar` path is the one that must
    /// reason about missingness.
    #[must_use]
    #[doc(hidden)]
    pub fn reindex_promote_float64_by_optional_positions(
        &self,
        positions: &[Option<usize>],
    ) -> Option<Self> {
        enum TypedSource<'a> {
            Int64(&'a [i64]),
            Float64(&'a [f64]),
        }
        let source = if let Some(slice) = self.as_i64_slice() {
            TypedSource::Int64(slice)
        } else {
            let slice = self.as_f64_slice()?;
            TypedSource::Float64(slice)
        };

        let n = positions.len();
        let len = self.len();
        let mut data = Vec::with_capacity(n);
        let mut words = vec![0_u64; n.div_ceil(64)];
        for (out_idx, slot) in positions.iter().enumerate() {
            match slot {
                Some(idx) if *idx < len => {
                    let value = match source {
                        TypedSource::Int64(slice) => slice[*idx] as f64,
                        TypedSource::Float64(slice) => slice[*idx],
                    };
                    data.push(value);
                    // All-valid sources carry no NaN (from_f64_values marks
                    // NaN invalid), so every matched slot is valid.
                    words[out_idx / 64] |= 1_u64 << (out_idx % 64);
                }
                _ => data.push(0.0),
            }
        }
        Some(Self::from_f64_values_with_validity(
            data,
            ValidityMask { words, len: n },
        ))
    }

    /// Borrow the column's contiguous `f64` buffer when this is an all-valid
    /// `Float64` column, enabling typed/SIMD reductions without the per-element
    /// `Scalar` match. Returns `None` for any other dtype or when the column
    /// has missing values — callers fall back to the `Scalar` path, which is
    /// the only path that must reason about missingness. Per
    /// br-frankenpandas-lei31.
    #[must_use]
    pub fn as_f64_slice(&self) -> Option<&[f64]> {
        if self.dtype == DType::Float64 && self.validity.all() {
            if let Some(ColumnData::Float64(data)) = &self.data {
                return Some(data.as_slice());
            }
            if let ScalarValues::LazyAllValidFloat64 { data, .. } = &self.values {
                return Some(data.as_ref());
            }
        }
        None
    }

    /// Borrow the column's contiguous `i64` buffer when this is an all-valid
    /// `Int64` column. See [`Column::as_f64_slice`].
    #[must_use]
    pub fn as_i64_slice(&self) -> Option<&[i64]> {
        if self.dtype == DType::Int64 && self.validity.all() {
            if let Some(ColumnData::Int64(data)) = &self.data {
                return Some(data.as_slice());
            }
            if let ScalarValues::LazyAllValidInt64 { data, .. } = &self.values {
                return Some(data.as_ref());
            }
            if let Some(data) = self.values.repeat_runs_i64_data() {
                return Some(data);
            }
            if let Some(data) = self.values.repeated_slices_i64_data() {
                return Some(data);
            }
        }
        None
    }

    /// Build an all-valid `Bool` column from already-typed contiguous values.
    ///
    /// The typed-ingestion counterpart for boolean results (comparison masks,
    /// predicates) — see [`Column::from_f64_values`]. Defers `Scalar`
    /// materialization until a caller asks for scalar values.
    #[must_use]
    pub fn from_bool_values(data: Vec<bool>) -> Self {
        let len = data.len();
        Self {
            dtype: DType::Bool,
            values: ScalarValues::lazy_all_valid_bool(data),
            validity: ValidityMask::all_valid(len),
            data: None,
        }
    }

    /// Borrow the column's contiguous Utf8 backing — `(bytes, offsets)` with
    /// row `i` = `bytes[offsets[i]..offsets[i+1]]`, always valid UTF-8 —
    /// when this is an all-valid Utf8 column carrying the
    /// `LazyContiguousUtf8` representation (br-frankenpandas-2krr0 rung 3).
    /// Lets chained string ops read the previous op's output without ever
    /// materializing its `Vec<Scalar>` view. Returns `None` for Scalar-backed
    /// or nullable columns — callers fall back to `values()`.
    #[must_use]
    #[doc(hidden)]
    pub fn as_utf8_contiguous(&self) -> Option<(&[u8], &[usize])> {
        if self.dtype == DType::Utf8
            && self.validity.all()
            && let ScalarValues::LazyContiguousUtf8 { bytes, offsets, .. } = &self.values
        {
            return Some((bytes.as_ref(), offsets.as_ref()));
        }
        None
    }

    /// Share the `Arc` contiguous-Utf8 backing plus the source-row offset of
    /// row 0, for an all-valid `LazyContiguousUtf8` (offset 0) or an existing
    /// `LazyUtf8Slice` view (offset `start`). The two `Arc::clone`s are O(1) and
    /// let `take_positions` return a contiguous-range view without copying
    /// (br-frankenpandas-jbyuc.1.1.1).
    fn utf8_arc_view_source(&self) -> Option<(Arc<[u8]>, Arc<[usize]>, usize)> {
        if self.dtype != DType::Utf8 || !self.validity.all() {
            return None;
        }
        match &self.values {
            ScalarValues::LazyContiguousUtf8 { bytes, offsets, .. } => {
                Some((Arc::clone(bytes), Arc::clone(offsets), 0))
            }
            ScalarValues::LazyUtf8Slice {
                bytes,
                offsets,
                start,
                ..
            } => Some((Arc::clone(bytes), Arc::clone(offsets), *start)),
            _ => None,
        }
    }

    /// Borrow the contiguous Utf8 backing only when its byte spans are already
    /// strictly increasing. The witness is cached on the immutable contiguous
    /// backing so repeated ordered joins do not rescan both key columns.
    #[must_use]
    #[doc(hidden)]
    pub fn as_strictly_increasing_utf8_contiguous(&self) -> Option<(&[u8], &[usize])> {
        if self.dtype == DType::Utf8
            && self.validity.all()
            && let ScalarValues::LazyContiguousUtf8 {
                bytes,
                offsets,
                strictly_increasing,
                ..
            } = &self.values
            && *strictly_increasing
                .get_or_init(|| contiguous_utf8_offsets_are_strictly_increasing(bytes, offsets))
        {
            return Some((bytes.as_ref(), offsets.as_ref()));
        }
        None
    }

    /// Borrow a strict contiguous-Utf8 backing and its fixed row byte width.
    ///
    /// The fixed-width witness is cached next to the strict-increasing witness:
    /// ordered string joins can then detect a long equal byte window once and
    /// emit a whole range of 1:1 matches without per-row byte-span comparisons.
    #[must_use]
    pub fn as_fixed_width_strictly_increasing_utf8_contiguous(
        &self,
    ) -> Option<(&[u8], &[usize], usize)> {
        if self.dtype == DType::Utf8
            && self.validity.all()
            && let ScalarValues::LazyContiguousUtf8 {
                bytes,
                offsets,
                strictly_increasing,
                fixed_width,
                ..
            } = &self.values
            && *strictly_increasing
                .get_or_init(|| contiguous_utf8_offsets_are_strictly_increasing(bytes, offsets))
        {
            let width = fixed_width
                .get_or_init(|| contiguous_utf8_fixed_width(offsets))
                .as_ref()
                .copied()?;
            return Some((bytes.as_ref(), offsets.as_ref(), width));
        }
        None
    }

    /// Borrow the column's contiguous `bool` buffer when this is an all-valid
    /// `Bool` column. See [`Column::as_f64_slice`].
    #[must_use]
    pub fn as_bool_slice(&self) -> Option<&[bool]> {
        if self.dtype == DType::Bool && self.validity.all() {
            if let Some(ColumnData::Bool(data)) = &self.data {
                return Some(data.as_slice());
            }
            if let ScalarValues::LazyAllValidBool { data, .. } = &self.values {
                return Some(data.as_ref());
            }
        }
        None
    }

    /// Gather a new column from the given row positions of `self`.
    ///
    /// This is the fast path for materialization (`take`, `iloc`, boolean
    /// filter, `sort_values`, `drop_duplicates`, `reindex`, `head`/`tail`,
    /// groupby row selection). Because every gathered value originates from
    /// `self` it already matches `self.dtype` (no coercion needed), so this
    /// skips the dtype-coercion and object-bucket detection scans that
    /// `Column::new` performs. All-valid source columns clone values directly
    /// and emit an all-valid mask; missing-bearing columns fold the
    /// missing-normalization and validity rebuild into a single pass.
    ///
    /// The output is bit-for-bit identical to
    /// `Column::new(self.dtype(), positions.iter().map(|&p| self.values[p].clone()).collect())`
    /// (the no-coercion branch `Column::new` takes for same-dtype input): each
    /// gathered value is missing-normalized via `normalize_missing_for_dtype`
    /// (generic `Null` → dtype-specific missing, e.g. `NaT` for datetime), and
    /// the validity mask is recomputed from the normalized values'
    /// `is_missing()` exactly as `ValidityMask::from_values` would.
    ///
    /// # Panics
    /// Panics if any position is out of bounds (callers materialize from
    /// validated index positions; this mirrors the prior `values()[pos]` index).
    #[must_use]
    pub fn take_positions(&self, positions: &[usize]) -> Self {
        let n = positions.len();
        if self.validity.all() {
            if let Some(data) = self.take_cached_all_valid_float64_positions(positions) {
                return Self {
                    dtype: self.dtype,
                    values: ScalarValues::lazy_all_valid_float64(data),
                    validity: ValidityMask::all_valid(n),
                    data: None,
                };
            }

            // Symmetric to the Float64 path: gather the contiguous i64 buffer and
            // keep the output lazily typed instead of materializing a
            // Vec<Scalar::Int64> (32 B/elem). Bit-identical — lazy_all_valid_int64
            // materializes Scalar::Int64(data[i]) exactly as the primitive path
            // would, with the same all-valid mask. (br-frankenpandas-uza04)
            if let Some(data) = self.take_cached_all_valid_int64_positions(positions) {
                return Self {
                    dtype: self.dtype,
                    values: ScalarValues::lazy_all_valid_int64(data),
                    validity: ValidityMask::all_valid(n),
                    data: None,
                };
            }

            // Zero-copy contiguous-range view (br-frankenpandas-jbyuc.1.1.1):
            // when the requested positions are a contiguous ascending range over
            // an Arc-shared contiguous-Utf8 backing, share the source `bytes`/
            // `offsets` and defer the per-row byte gather instead of copying.
            // Bit-identical to the eager gather below: the view materializes
            // `Scalar::Utf8` of `bytes[off[start+i]..off[start+i+1]]` for
            // `i in 0..n` — the exact same spans, same order, all-valid mask.
            //
            // Gated `n >= 64`: a view keeps the *whole* source buffer alive via
            // `Arc`, so a tiny contiguous take (head/tail/single-row iloc) would
            // pin a potentially large buffer to hold a handful of rows. Small
            // takes fall through to the eager gather (a cheap, independent copy);
            // only sizeable contiguous ranges — the join-output shape this lever
            // targets — take the zero-copy view.
            if n >= 64
                && let Some((src_bytes, src_offsets, src_start)) = self.utf8_arc_view_source()
                && let Some(range_start) = contiguous_ascending_start(positions)
            {
                return Self {
                    dtype: self.dtype,
                    values: ScalarValues::lazy_utf8_slice(
                        src_bytes,
                        src_offsets,
                        src_start + range_start,
                        n,
                    ),
                    validity: ValidityMask::all_valid(n),
                    data: None,
                };
            }

            // Contiguous-Utf8 gather (br-frankenpandas-nl1tw): an all-valid
            // `LazyContiguousUtf8` column gathers its selected byte spans into one
            // fresh `bytes` buffer + `offsets`, keeping the output lazily typed —
            // no per-row `String` heap clone and no lazy Scalar re-materialization.
            // Bit-identical to the Scalar-clone path: each output slot materializes
            // `Scalar::Utf8` of the exact same span bytes in the same order, with an
            // all-valid mask (the source is all-valid by the enclosing branch).
            if let Some((bytes, offsets)) = self.as_utf8_contiguous() {
                let total: usize = positions
                    .iter()
                    .map(|&pos| offsets[pos + 1] - offsets[pos])
                    .sum();
                let mut new_bytes = Vec::with_capacity(total);
                let mut new_offsets = Vec::with_capacity(n + 1);
                new_offsets.push(0);
                for &pos in positions {
                    new_bytes.extend_from_slice(&bytes[offsets[pos]..offsets[pos + 1]]);
                    new_offsets.push(new_bytes.len());
                }
                return Self {
                    dtype: self.dtype,
                    values: ScalarValues::lazy_contiguous_utf8(new_bytes, new_offsets),
                    validity: ValidityMask::all_valid(n),
                    data: None,
                };
            }

            let values = self
                .take_all_valid_primitive_positions(positions)
                .unwrap_or_else(|| {
                    positions
                        .iter()
                        .map(|&pos| self.values[pos].clone())
                        .collect()
                });
            return Self {
                dtype: self.dtype,
                values: ScalarValues::from_vec(values),
                validity: ValidityMask::all_valid(n),
                data: None,
            };
        }

        // Typed nullable Float64 gather: when the source carries a contiguous
        // f64 buffer with a validity mask (LazyNullableFloat64), gather the data
        // and the validity bits directly instead of cloning a Scalar per row.
        // Bit-identical: that variant materializes Float64(data[i]) when
        // valid-or-NaN and Null(NaN) otherwise, so the missingness of slot `pos`
        // is `validity.get(pos) && !data[pos].is_nan()`; carrying that exact bit
        // (and the raw datum) into from_f64_values_with_validity reproduces the
        // same Scalar at every slot, while skipping the 32 B/elem Vec<Scalar>.
        if let ScalarValues::LazyNullableFloat64 { data: src, .. } = &self.values {
            let mut data = Vec::with_capacity(n);
            let mut words = vec![0_u64; n.div_ceil(64)];
            for (out_idx, &pos) in positions.iter().enumerate() {
                let x = src[pos];
                data.push(x);
                if self.validity.get(pos) && !x.is_nan() {
                    words[out_idx / 64] |= 1_u64 << (out_idx % 64);
                }
            }
            return Self::from_f64_values_with_validity(data, ValidityMask { words, len: n });
        }

        let mut values = Vec::with_capacity(n);
        let mut words = vec![0_u64; n.div_ceil(64)];
        for (out_idx, &pos) in positions.iter().enumerate() {
            let value = Self::normalize_missing_for_dtype(self.values[pos].clone(), self.dtype);
            if !value.is_missing() {
                words[out_idx / 64] |= 1_u64 << (out_idx % 64);
            }
            values.push(value);
        }
        Self {
            dtype: self.dtype,
            values: ScalarValues::from_vec(values),
            validity: ValidityMask { words, len: n },
            data: None,
        }
    }

    fn take_cached_all_valid_float64_positions(&self, positions: &[usize]) -> Option<Vec<f64>> {
        let data = self.as_f64_slice()?;
        let mut values = Vec::with_capacity(positions.len());
        for &pos in positions {
            values.push(data[pos]);
        }
        Some(values)
    }

    fn take_cached_all_valid_int64_positions(&self, positions: &[usize]) -> Option<Vec<i64>> {
        let data = self.as_i64_slice()?;
        let mut values = Vec::with_capacity(positions.len());
        for &pos in positions {
            values.push(data[pos]);
        }
        Some(values)
    }

    fn take_all_valid_primitive_positions(&self, positions: &[usize]) -> Option<Vec<Scalar>> {
        if let Some(values) = self.take_cached_all_valid_primitive_positions(positions) {
            return Some(values);
        }

        let mut values = Vec::with_capacity(positions.len());
        match self.dtype {
            DType::Bool | DType::BoolNullable => {
                for &pos in positions {
                    match &self.values[pos] {
                        Scalar::Bool(value) => values.push(Scalar::Bool(*value)),
                        _ => return None,
                    }
                }
            }
            DType::Int64 | DType::Int64Nullable => {
                for &pos in positions {
                    match &self.values[pos] {
                        Scalar::Int64(value) => values.push(Scalar::Int64(*value)),
                        _ => return None,
                    }
                }
            }
            DType::Float64 => {
                for &pos in positions {
                    match &self.values[pos] {
                        Scalar::Float64(value) => values.push(Scalar::Float64(*value)),
                        _ => return None,
                    }
                }
            }
            DType::Timedelta64 => {
                for &pos in positions {
                    match &self.values[pos] {
                        Scalar::Timedelta64(value) => values.push(Scalar::Timedelta64(*value)),
                        _ => return None,
                    }
                }
            }
            DType::Datetime64 => {
                for &pos in positions {
                    match &self.values[pos] {
                        Scalar::Datetime64(value) => values.push(Scalar::Datetime64(*value)),
                        _ => return None,
                    }
                }
            }
            DType::Period => {
                for &pos in positions {
                    match &self.values[pos] {
                        Scalar::Period(value) => values.push(Scalar::Period(*value)),
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
        Some(values)
    }

    fn take_cached_all_valid_primitive_positions(
        &self,
        positions: &[usize],
    ) -> Option<Vec<Scalar>> {
        match self.dtype {
            DType::Bool => {
                if let Some(data) = self.as_bool_slice() {
                    let mut values = Vec::with_capacity(positions.len());
                    for &pos in positions {
                        values.push(Scalar::Bool(data[pos]));
                    }
                    return Some(values);
                }
            }
            DType::Int64 => {
                if let Some(data) = self.as_i64_slice() {
                    let mut values = Vec::with_capacity(positions.len());
                    for &pos in positions {
                        values.push(Scalar::Int64(data[pos]));
                    }
                    return Some(values);
                }
            }
            DType::Float64 => {
                if let Some(data) = self.as_f64_slice() {
                    let mut values = Vec::with_capacity(positions.len());
                    for &pos in positions {
                        values.push(Scalar::Float64(data[pos]));
                    }
                    return Some(values);
                }
            }
            _ => {}
        }

        let data = self.data.as_ref()?;
        let mut values = Vec::with_capacity(positions.len());
        match (self.dtype, data) {
            (DType::Bool | DType::BoolNullable, ColumnData::Bool(data)) => {
                for &pos in positions {
                    values.push(Scalar::Bool(data[pos]));
                }
            }
            (DType::Int64 | DType::Int64Nullable, ColumnData::Int64(data)) => {
                for &pos in positions {
                    values.push(Scalar::Int64(data[pos]));
                }
            }
            (DType::Float64, ColumnData::Float64(data)) => {
                for &pos in positions {
                    values.push(Scalar::Float64(data[pos]));
                }
            }
            (DType::Timedelta64, ColumnData::Timedelta64(data)) => {
                for &pos in positions {
                    values.push(Scalar::Timedelta64(data[pos]));
                }
            }
            (DType::Datetime64, ColumnData::Datetime64(data)) => {
                for &pos in positions {
                    values.push(Scalar::Datetime64(data[pos]));
                }
            }
            (DType::Period, ColumnData::Period(data)) => {
                for &pos in positions {
                    values.push(Scalar::Period(data[pos]));
                }
            }
            _ => return None,
        }
        Some(values)
    }

    /// Create a column filled with zeros.
    ///
    /// Matches np.zeros().
    pub fn zeros(n: usize, dtype: DType) -> Result<Self, ColumnError> {
        let zero = match dtype {
            DType::Int64 => Scalar::Int64(0),
            DType::Float64 => Scalar::Float64(0.0),
            DType::Bool => Scalar::Bool(false),
            _ => Scalar::Int64(0),
        };
        Self::new(dtype, vec![zero; n])
    }

    /// Create a column filled with ones.
    ///
    /// Matches np.ones().
    pub fn ones(n: usize, dtype: DType) -> Result<Self, ColumnError> {
        let one = match dtype {
            DType::Int64 => Scalar::Int64(1),
            DType::Float64 => Scalar::Float64(1.0),
            DType::Bool => Scalar::Bool(true),
            _ => Scalar::Int64(1),
        };
        Self::new(dtype, vec![one; n])
    }

    /// Create a column filled with a given value.
    ///
    /// Matches np.full().
    pub fn full(n: usize, fill_value: Scalar) -> Result<Self, ColumnError> {
        let dtype = fill_value.dtype();
        Self::new(dtype, vec![fill_value; n])
    }

    /// Create a zeros column with same shape and dtype as self.
    pub fn zeros_like(&self) -> Result<Self, ColumnError> {
        Self::zeros(self.len(), self.dtype)
    }

    /// Create a ones column with same shape and dtype as self.
    pub fn ones_like(&self) -> Result<Self, ColumnError> {
        Self::ones(self.len(), self.dtype)
    }

    /// Create a column filled with fill_value with same shape as self.
    pub fn full_like(&self, fill_value: Scalar) -> Result<Self, ColumnError> {
        Self::new(self.dtype, vec![fill_value; self.len()])
    }

    /// Create an empty column with same dtype as self.
    pub fn empty_like(&self) -> Result<Self, ColumnError> {
        Self::new(self.dtype, Vec::new())
    }

    /// Create a column with evenly spaced values in [start, stop).
    ///
    /// Matches np.arange().
    pub fn arange(start: f64, stop: f64, step: f64) -> Result<Self, ColumnError> {
        if step == 0.0 {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: "step cannot be zero".to_string(),
                dtype: DType::Float64,
            }));
        }
        let mut values = Vec::new();
        let mut x = start;
        if step > 0.0 {
            while x < stop {
                values.push(Scalar::Float64(x));
                x += step;
            }
        } else {
            while x > stop {
                values.push(Scalar::Float64(x));
                x += step;
            }
        }
        Self::new(DType::Float64, values)
    }

    /// Create a column with evenly spaced values over [start, stop].
    ///
    /// Matches np.linspace().
    pub fn linspace(start: f64, stop: f64, num: usize) -> Result<Self, ColumnError> {
        if num == 0 {
            return Self::new(DType::Float64, Vec::new());
        }
        if num == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(start)]);
        }
        let step = (stop - start) / (num - 1) as f64;
        let values: Vec<Scalar> = (0..num)
            .map(|i| Scalar::Float64(start + step * i as f64))
            .collect();
        Self::new(DType::Float64, values)
    }

    /// Create a column with evenly spaced values on a log scale.
    ///
    /// Matches np.logspace().
    pub fn logspace(start: f64, stop: f64, num: usize) -> Result<Self, ColumnError> {
        let lin = Self::linspace(start, stop, num)?;
        let values: Vec<Scalar> = lin
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Float64(x) => Scalar::Float64(10.0_f64.powf(*x)),
                _ => v.clone(),
            })
            .collect();
        Self::new(DType::Float64, values)
    }

    /// Create values evenly spaced on a log scale (geometric progression).
    ///
    /// Matches np.geomspace(start, stop, num). Unlike logspace, start and stop
    /// are the actual boundary values (not exponents).
    pub fn geomspace(start: f64, stop: f64, num: usize) -> Result<Self, ColumnError> {
        if num == 0 {
            return Self::new(DType::Float64, vec![]);
        }
        if start == 0.0 || stop == 0.0 {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: "geomspace endpoints cannot be zero".to_owned(),
                dtype: DType::Float64,
            }));
        }
        if num == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(start)]);
        }

        let log_start = start.ln();
        let log_stop = stop.ln();
        let step = (log_stop - log_start) / (num - 1) as f64;
        let values: Vec<Scalar> = (0..num)
            .map(|i| Scalar::Float64((log_start + step * i as f64).exp()))
            .collect();
        Self::new(DType::Float64, values)
    }

    /// Generate a Hann (Hanning) window.
    ///
    /// Matches np.hanning(M). Returns a raised cosine window of length M.
    pub fn hanning(m: usize) -> Result<Self, ColumnError> {
        if m == 0 {
            return Self::new(DType::Float64, vec![]);
        }
        if m == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(1.0)]);
        }
        let values: Vec<Scalar> = (0..m)
            .map(|n| {
                let val =
                    0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (m - 1) as f64).cos();
                Scalar::Float64(val)
            })
            .collect();
        Self::new(DType::Float64, values)
    }

    /// Generate a Hamming window.
    ///
    /// Matches np.hamming(M). Returns a Hamming window of length M.
    pub fn hamming(m: usize) -> Result<Self, ColumnError> {
        if m == 0 {
            return Self::new(DType::Float64, vec![]);
        }
        if m == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(1.0)]);
        }
        let values: Vec<Scalar> = (0..m)
            .map(|n| {
                let val =
                    0.54 - 0.46 * (2.0 * std::f64::consts::PI * n as f64 / (m - 1) as f64).cos();
                Scalar::Float64(val)
            })
            .collect();
        Self::new(DType::Float64, values)
    }

    /// Generate a Blackman window.
    ///
    /// Matches np.blackman(M). Returns a Blackman window of length M.
    pub fn blackman(m: usize) -> Result<Self, ColumnError> {
        if m == 0 {
            return Self::new(DType::Float64, vec![]);
        }
        if m == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(1.0)]);
        }
        let values: Vec<Scalar> = (0..m)
            .map(|n| {
                let x = n as f64 / (m - 1) as f64;
                let val = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * x).cos()
                    + 0.08 * (4.0 * std::f64::consts::PI * x).cos();
                Scalar::Float64(val)
            })
            .collect();
        Self::new(DType::Float64, values)
    }

    /// Generate a Bartlett (triangular) window.
    ///
    /// Matches np.bartlett(M). Returns a triangular window of length M.
    pub fn bartlett(m: usize) -> Result<Self, ColumnError> {
        if m == 0 {
            return Self::new(DType::Float64, vec![]);
        }
        if m == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(1.0)]);
        }
        let half = (m - 1) as f64 / 2.0;
        let values: Vec<Scalar> = (0..m)
            .map(|n| {
                let val = 1.0 - ((n as f64 - half) / half).abs();
                Scalar::Float64(val)
            })
            .collect();
        Self::new(DType::Float64, values)
    }

    #[must_use]
    pub fn dtype(&self) -> DType {
        self.dtype
    }

    /// Returns true if this column contains any null/missing values.
    #[must_use]
    pub fn has_nulls(&self) -> bool {
        self.validity.count_invalid() > 0
    }

    /// Promote the dtype to its nullable variant if the column has nulls.
    ///
    /// For Int64 with nulls → Int64Nullable, Bool with nulls → BoolNullable.
    /// For already-nullable or other dtypes, returns a clone unchanged.
    #[must_use]
    pub fn promote_to_nullable(&self) -> Self {
        if !self.has_nulls() {
            return self.clone();
        }
        let new_dtype = self.dtype.to_nullable();
        if new_dtype == self.dtype {
            return self.clone();
        }
        Self {
            dtype: new_dtype,
            values: self.values.clone(),
            validity: self.validity.clone(),
            data: self.data.clone(),
        }
    }

    /// Create a new column with a different dtype, preserving the same values.
    ///
    /// This is a low-level operation that only changes the dtype metadata
    /// without converting values. Use only when the values are already valid
    /// for the target dtype.
    #[must_use]
    pub fn with_dtype(&self, dtype: DType) -> Self {
        Self {
            dtype,
            values: self.values.clone(),
            validity: self.validity.clone(),
            data: None,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Number of elements, matching `pd.Series.size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.len()
    }

    /// One-dimensional shape, matching `pd.Series.shape`.
    #[must_use]
    pub fn shape(&self) -> (usize,) {
        (self.len(),)
    }

    /// Number of array dimensions, matching `pd.Series.ndim`.
    #[must_use]
    pub fn ndim(&self) -> usize {
        1
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Alias for [`is_empty`](Self::is_empty), matching `pd.Series.empty`.
    #[must_use]
    pub fn empty(&self) -> bool {
        self.is_empty()
    }

    /// Return a deep copy of this column.
    ///
    /// Matches `pd.Series.copy(deep=True)` at the column storage layer.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Return an immutable view-shaped clone of this column.
    ///
    /// Matches `pd.Series.view()` at the column storage layer.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// One-dimensional transpose is identity.
    ///
    /// Matches `pd.Series.transpose()` at the column storage layer.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Lowercase alias for [`transpose`](Self::transpose).
    #[must_use]
    pub fn t(&self) -> Self {
        self.transpose()
    }

    /// Uppercase pandas spelling for [`transpose`](Self::transpose).
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    #[must_use]
    pub fn values(&self) -> &[Scalar] {
        &self.values
    }

    #[must_use]
    pub fn value(&self, idx: usize) -> Option<&Scalar> {
        self.values.get(idx)
    }

    /// Extract scalar value from a single-element column.
    ///
    /// Matches `pd.Series.item()` at the column storage layer. Returns an
    /// error unless the column has exactly one element.
    pub fn item(&self) -> Result<Scalar, ColumnError> {
        match self.values.as_slice() {
            [value] => Ok(value.clone()),
            values => Err(ColumnError::InvalidLength {
                operation: "item()",
                expected: 1,
                actual: values.len(),
            }),
        }
    }

    #[must_use]
    pub fn validity(&self) -> &ValidityMask {
        &self.validity
    }

    /// Borrow-returning iterator over the column's scalars.
    ///
    /// Convenience over `self.values().iter()` so call sites don't
    /// have to reach through the slice accessor. Preserves
    /// position order.
    pub fn iter_values(&self) -> std::slice::Iter<'_, Scalar> {
        self.values.iter()
    }

    /// Materialize the column's values into an owned `Vec<Scalar>`.
    ///
    /// Matches `pd.Series.to_list()`. Equivalent to
    /// `self.values().to_vec()`; the shorthand survives refactors
    /// that change the internal storage shape.
    #[must_use]
    pub fn to_vec(&self) -> Vec<Scalar> {
        self.values.to_vec()
    }

    /// Alias for [`to_vec`](Self::to_vec), matching `pd.Series.to_list()`.
    #[must_use]
    pub fn to_list(&self) -> Vec<Scalar> {
        self.to_vec()
    }

    /// Alias for [`to_list`](Self::to_list), matching `pd.Series.tolist()`.
    #[must_use]
    pub fn tolist(&self) -> Vec<Scalar> {
        self.to_list()
    }

    /// Owned scalar materialization, matching `pd.Series.to_numpy()`.
    #[must_use]
    pub fn to_numpy(&self) -> Vec<Scalar> {
        self.to_vec()
    }

    /// Flatten values to a one-dimensional vector, matching `pd.Series.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<Scalar> {
        self.to_numpy()
    }

    /// Flatten values to a copy, matching `np.ndarray.flatten()`.
    ///
    /// For 1D arrays this is equivalent to ravel() but explicitly returns
    /// an owned copy rather than potentially a view.
    #[must_use]
    pub fn flatten(&self) -> Vec<Scalar> {
        self.values.to_vec()
    }

    /// Convert to array, matching `np.asarray()`.
    ///
    /// For Column this returns a clone since we're already array-like.
    #[must_use]
    pub fn asarray(&self) -> Self {
        self.clone()
    }

    /// Owned scalar materialization, matching `pd.Series.array`.
    #[must_use]
    pub fn array(&self) -> Vec<Scalar> {
        self.to_vec()
    }

    /// Whether any value in the column is missing.
    ///
    /// Matches `pd.Series.isna().any()` in one pass. Faster than
    /// calling `isnull()` and scanning — returns on the first
    /// missing value seen.
    #[must_use]
    pub fn has_any_missing(&self) -> bool {
        self.values.iter().any(Scalar::is_missing)
    }

    /// Whether any value is missing, matching `pd.Series.hasnans`.
    #[must_use]
    pub fn hasnans(&self) -> bool {
        self.has_any_missing()
    }

    /// Whether every value in the column is missing.
    ///
    /// Matches `pd.Series.isna().all()`. Empty columns return true
    /// (vacuously), mirroring `ValidityMask::all`'s empty-case
    /// convention.
    #[must_use]
    pub fn all_missing(&self) -> bool {
        self.values.iter().all(Scalar::is_missing)
    }

    /// First value in the column (index 0), or `None` when empty.
    ///
    /// Matches `pd.Series.iloc[0]` shorthand. Returns the raw Scalar
    /// including missing markers; callers who want skipna semantics
    /// can pair with `has_any_missing`.
    #[must_use]
    pub fn first(&self) -> Option<&Scalar> {
        self.values.first()
    }

    /// Last value in the column, or `None` when empty.
    #[must_use]
    pub fn last(&self) -> Option<&Scalar> {
        self.values.last()
    }

    /// Count values for which `predicate` returns true.
    ///
    /// Complement to `apply_bool` that yields only the count rather
    /// than materializing a Bool column. Missing inputs are treated
    /// as a non-match (consistent with `apply_bool`'s
    /// missing→Bool(false) contract).
    pub fn count_matching<F>(&self, mut predicate: F) -> usize
    where
        F: FnMut(&Scalar) -> bool,
    {
        self.values
            .iter()
            .filter(|v| !v.is_missing() && predicate(v))
            .count()
    }

    /// Elementwise combine with another column via a user function.
    ///
    /// Matches `pd.Series.combine(other, func)` at the Column layer
    /// without the pandas `fill_value=None` null-propagation policy
    /// — `zip_with` always invokes `func`, passing through missing
    /// values as-is so the caller decides whether to short-circuit
    /// nulls. Length mismatch returns `LengthMismatch`.
    pub fn zip_with<F>(&self, other: &Self, mut func: F) -> Result<Self, ColumnError>
    where
        F: FnMut(&Scalar, &Scalar) -> Scalar,
    {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| func(a, b))
            .collect();
        let inferred = infer_dtype(&out).unwrap_or(self.dtype);
        Self::new(inferred, out)
    }

    /// `(position, scalar)` iterator.
    ///
    /// Shortcut for `iter_values().enumerate()`. Convenience for
    /// callers that need both positions and values and don't want
    /// to reach through the slice accessor.
    pub fn iter_enumerate(&self) -> std::iter::Enumerate<std::slice::Iter<'_, Scalar>> {
        self.values.iter().enumerate()
    }

    /// Apply a predicate per value and collect the results into a
    /// Bool column.
    ///
    /// Like `Column::map` but specialized for predicate functions
    /// returning `bool`. Missing inputs produce `Scalar::Bool(false)`
    /// by default — callers that need null propagation should use
    /// `map` instead so they can emit `Null(NaN)` explicitly.
    pub fn apply_bool<F>(&self, mut predicate: F) -> Result<Self, ColumnError>
    where
        F: FnMut(&Scalar) -> bool,
    {
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| {
                if v.is_missing() {
                    Scalar::Bool(false)
                } else {
                    Scalar::Bool(predicate(v))
                }
            })
            .collect();
        Self::new(DType::Bool, out)
    }

    pub fn reindex_by_positions(&self, positions: &[Option<usize>]) -> Result<Self, ColumnError> {
        let mut present_positions = Vec::with_capacity(positions.len());
        let mut all_present = true;
        for position in positions {
            match position {
                Some(idx) if *idx < self.len() => present_positions.push(*idx),
                Some(_) | None => {
                    all_present = false;
                    break;
                }
            }
        }
        if all_present {
            return Ok(self.take_positions(&present_positions));
        }

        // Typed null-introducing gather (br-frankenpandas-lt5qx): an
        // all-valid Int64/Float64 source skips the per-row Scalar clone +
        // Column::new revalidation. Missing slots (None or out-of-range)
        // produce exactly missing_for_dtype: Null(NullKind::Null) via the
        // nullable-Int64 backing, Null(NullKind::NaN) via the 0.0-datum
        // nullable-Float64 convention; valid slots clone the raw datum.
        let n = positions.len();
        if let Some(slice) = self.as_i64_slice() {
            let mut data = Vec::with_capacity(n);
            let mut words = vec![0_u64; n.div_ceil(64)];
            for (out_idx, slot) in positions.iter().enumerate() {
                match slot {
                    Some(idx) if *idx < slice.len() => {
                        data.push(slice[*idx]);
                        words[out_idx / 64] |= 1_u64 << (out_idx % 64);
                    }
                    _ => data.push(0),
                }
            }
            return Ok(Self::from_i64_values_with_validity(
                data,
                ValidityMask { words, len: n },
            ));
        }
        if let Some(slice) = self.as_f64_slice() {
            let mut data = Vec::with_capacity(n);
            let mut words = vec![0_u64; n.div_ceil(64)];
            for (out_idx, slot) in positions.iter().enumerate() {
                match slot {
                    Some(idx) if *idx < slice.len() => {
                        data.push(slice[*idx]);
                        words[out_idx / 64] |= 1_u64 << (out_idx % 64);
                    }
                    _ => data.push(0.0),
                }
            }
            return Ok(Self::from_f64_values_with_validity(
                data,
                ValidityMask { words, len: n },
            ));
        }

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
                // Typed-input fast path: both operands are already all-valid
                // contiguous Float64 (as_f64_slice => validity.all() AND no NaN),
                // so read the buffers directly — no Scalar materialization, no
                // from_scalars copy, no nan-aware validity scan. Bit-identical to
                // the general arm's all-valid branch: with both inputs valid and
                // NaN-free, the combined validity is all-valid, so it returns
                // from_f64_values(apply(l,r)) too (and from_f64_values still marks
                // any operation-produced NaN missing, identically).
                if let (Some(l), Some(r)) = (self.as_f64_slice(), right.as_f64_slice()) {
                    let apply = binary_f64_apply(op);
                    let result: Vec<f64> = l.iter().zip(r).map(|(&a, &b)| apply(a, b)).collect();
                    return Some(Ok(Self::from_f64_values(result)));
                }
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

                // All inputs valid: preserve the typed result buffer directly
                // instead of rebuilding Vec<Scalar> and rescanning validity.
                // Operation-produced NaN is still marked missing by
                // from_f64_values, exactly like the Scalar::Float64(NaN) path.
                if result_validity.all() {
                    return Some(Ok(Self::from_f64_values(result_data)));
                }

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
                // Typed-input fast path (see the Float64 arm): both operands are
                // all-valid contiguous i64 buffers, so feed vectorized_binary_i64
                // directly — no from_scalars materialization. All-valid inputs =>
                // all-valid result, so from_i64_values, identical to the general
                // arm's all-valid branch.
                if let (Some(l), Some(r)) = (self.as_i64_slice(), right.as_i64_slice()) {
                    let (result_data, _validity) =
                        vectorized_binary_i64(l, r, &self.validity, &right.validity, op)?;
                    return Some(Ok(Self::from_i64_values(result_data)));
                }
                let left_data = ColumnData::from_scalars(&self.values, DType::Int64);
                let right_data = ColumnData::from_scalars(&right.values, DType::Int64);
                let (ColumnData::Int64(l), ColumnData::Int64(r)) = (&left_data, &right_data) else {
                    return None;
                };

                let (result_data, result_validity) =
                    vectorized_binary_i64(l, r, &self.validity, &right.validity, op)?;

                // All inputs valid: keep the typed i64 result buffer as the
                // column source of truth and skip Scalar materialization.
                if result_validity.all() {
                    return Some(Ok(Self::from_i64_values(result_data)));
                }

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

    /// AG-10 fused outer-alignment arithmetic for two Float64 columns.
    ///
    /// Equivalent to `self.reindex_by_positions(lp)?.binary_numeric(
    /// &right.reindex_by_positions(rp)?, op)` for the Float64-output case, but it
    /// gathers `f64` directly from the *original* columns into the union layout
    /// in one pass instead of materializing two intermediate `Vec<Scalar>` and
    /// re-deriving their `f64` views. Provably isomorphic: `from_scalars` is
    /// element-wise and `reindex` is a gather, so `gather(from_scalars(src)) ==
    /// from_scalars(reindex(src))`; the nan-aware validity gathers identically
    /// (a `None` slot reindexes to `missing_for_dtype(Float64) = Null(NaN)`,
    /// i.e. invalid, exactly as the gathered mask marks it). For Float64 output
    /// every invalid position is `Null(NaN)` — matching both arms of
    /// `try_vectorized_binary`'s invalid branch. Caller guarantees both columns
    /// are `Float64`.
    pub fn aligned_binary_f64(
        &self,
        right: &Self,
        left_positions: &[Option<usize>],
        right_positions: &[Option<usize>],
        op: ArithmeticOp,
    ) -> Result<Self, ColumnError> {
        debug_assert_eq!(left_positions.len(), right_positions.len());
        let out_len = left_positions.len();

        let lsrc = self.float64_binary_data();
        let rsrc = right.float64_binary_data();
        let lvalid = self.nan_aware_validity();
        let rvalid = right.nan_aware_validity();

        let apply = binary_f64_apply(op);

        let mut data = Vec::with_capacity(out_len);
        let mut words = vec![0_u64; out_len.div_ceil(64)];
        let mut all_valid = true;
        for (k, left_slot) in left_positions.iter().enumerate() {
            if let Some(i) = *left_slot
                && let Some(j) = right_positions.get(k).copied().flatten()
                && lvalid.get(i)
                && rvalid.get(j)
            {
                let value = apply(lsrc[i], rsrc[j]);
                data.push(value);
                if value.is_nan() {
                    all_valid = false;
                } else {
                    words[k / 64] |= 1_u64 << (k % 64);
                }
            } else {
                data.push(0.0);
                all_valid = false;
            }
        }
        if all_valid {
            return Ok(Self::from_f64_values(data));
        }
        Ok(Self::from_f64_values_with_validity(
            data,
            ValidityMask {
                words,
                len: out_len,
            },
        ))
    }

    /// Fused Float64 arithmetic for two aligned contiguous `Int64` unit ranges.
    ///
    /// The caller has proven the left and right indexes are `[start, end]`
    /// integer ranges and the output index is their contiguous union. This is
    /// isomorphic to [`Self::aligned_binary_f64`] with arithmetic positions, but
    /// it fills the overlapped span directly and leaves non-overlap slots
    /// invalid, avoiding the two `Vec<Option<usize>>` alignment buffers.
    pub fn aligned_binary_f64_int64_unit_ranges(
        &self,
        right: &Self,
        left_range: (i64, i64),
        right_range: (i64, i64),
        union_range: (i64, i64),
        op: ArithmeticOp,
    ) -> Result<Self, ColumnError> {
        if !matches!(self.dtype, DType::Float64) || !matches!(right.dtype, DType::Float64) {
            return Err(ColumnError::DTypeMismatch {
                left: self.dtype,
                right: right.dtype,
            });
        }

        let (left_start, left_end) = left_range;
        let (right_start, right_end) = right_range;
        let (union_start, union_end) = union_range;

        let Some(left_len) = unit_range_len(left_start, left_end) else {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        };
        let Some(right_len) = unit_range_len(right_start, right_end) else {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        };
        let Some(out_len) = unit_range_len(union_start, union_end) else {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        };
        if left_len != self.len() || right_len != right.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        }

        let lsrc = self.float64_binary_data();
        let rsrc = right.float64_binary_data();
        let lvalid = self.nan_aware_validity();
        let rvalid = right.nan_aware_validity();
        let apply = binary_f64_apply(op);

        let mut data = vec![0.0; out_len];
        let mut words = vec![0_u64; out_len.div_ceil(64)];
        let overlap_start = left_start.max(right_start);
        let overlap_end = left_end.min(right_end);
        let mut all_valid = overlap_start == union_start && overlap_end == union_end;

        if overlap_start <= overlap_end {
            for value in overlap_start..=overlap_end {
                let out_idx = (value - union_start) as usize;
                let left_idx = (value - left_start) as usize;
                let right_idx = (value - right_start) as usize;
                if lvalid.get(left_idx) && rvalid.get(right_idx) {
                    let result = apply(lsrc[left_idx], rsrc[right_idx]);
                    data[out_idx] = result;
                    if result.is_nan() {
                        all_valid = false;
                    } else {
                        words[out_idx / 64] |= 1_u64 << (out_idx % 64);
                    }
                } else {
                    all_valid = false;
                }
            }
        }

        if all_valid {
            return Ok(Self::from_f64_values(data));
        }
        Ok(Self::from_f64_values_with_validity(
            data,
            ValidityMask {
                words,
                len: out_len,
            },
        ))
    }

    /// Same-index Float64 arithmetic fast path.
    ///
    /// Isomorphic to calling [`Self::aligned_binary_f64`] with
    /// `Some(i)`/`Some(i)` positions for every row, but avoids allocating and
    /// walking the identity alignment vectors.
    pub fn aligned_binary_f64_same_positions(
        &self,
        right: &Self,
        op: ArithmeticOp,
    ) -> Result<Self, ColumnError> {
        debug_assert_eq!(self.len(), right.len());
        let out_len = self.len();

        let lsrc = self.float64_binary_data();
        let rsrc = right.float64_binary_data();

        // Fully-valid fast path (br-frankenpandas-f64simd): when neither side has
        // a null bit or a NaN, every output position is valid, so emit a single
        // monomorphized (autovectorizing) slice op and skip both the per-element
        // `nan_aware_validity` mask builds and the per-element fn-pointer/validity
        // gating. Bit-identical to the general path under all-valid inputs: the
        // arithmetic and the typed `from_f64_values` constructor are the same.
        if self.validity.all()
            && right.validity.all()
            && !lsrc.iter().any(|x| x.is_nan())
            && !rsrc.iter().any(|x| x.is_nan())
        {
            return Ok(Self::from_f64_values(apply_f64_slices(op, &lsrc, &rsrc)));
        }

        let lvalid = self.nan_aware_validity();
        let rvalid = right.nan_aware_validity();
        let apply = binary_f64_apply(op);

        let mut data = Vec::with_capacity(out_len);
        let mut all_valid = true;
        for i in 0..out_len {
            if lvalid.get(i) && rvalid.get(i) {
                data.push(apply(lsrc[i], rsrc[i]));
            } else {
                all_valid = false;
                break;
            }
        }
        if all_valid {
            return Ok(Self::from_f64_values(data));
        }

        let mut values = Vec::with_capacity(out_len);
        for i in 0..out_len {
            if lvalid.get(i) && rvalid.get(i) {
                values.push(Scalar::Float64(apply(lsrc[i], rsrc[i])));
            } else {
                values.push(Scalar::Null(NullKind::NaN));
            }
        }
        Self::new(DType::Float64, values)
    }

    fn cached_float64_data(&self) -> Option<&[f64]> {
        match &self.data {
            Some(ColumnData::Float64(data)) if data.len() == self.values.len() => {
                return Some(data.as_slice());
            }
            _ => {}
        }

        match &self.values {
            ScalarValues::LazyAllValidFloat64 { data, .. } if data.len() == self.validity.len() => {
                Some(data.as_ref())
            }
            ScalarValues::LazyNullableFloat64 { data, .. } if data.len() == self.validity.len() => {
                Some(data.as_slice())
            }
            _ => None,
        }
    }

    fn float64_binary_data(&self) -> std::borrow::Cow<'_, [f64]> {
        if let Some(data) = self.cached_float64_data() {
            return std::borrow::Cow::Borrowed(data);
        }

        match ColumnData::from_scalars(&self.values, DType::Float64) {
            ColumnData::Float64(data) => std::borrow::Cow::Owned(data),
            _ => unreachable!("Float64 materialization must produce Float64 data"),
        }
    }

    /// Validity mask that also marks NaN float values as invalid.
    #[must_use]
    fn nan_aware_validity(&self) -> ValidityMask {
        let mut mask = self.validity.clone();

        if let Some(data) = self.cached_float64_data() {
            for (i, value) in data.iter().enumerate() {
                if value.is_nan() {
                    mask.set(i, false);
                }
            }
            return mask;
        }

        for (i, value) in self.values.iter().enumerate() {
            if matches!(value, Scalar::Float64(f) if f.is_nan()) {
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
        // Div always produces Float64. Pow keeps Int64 for int**int (numpy/pandas
        // semantics: 2 ** 3 -> int64 8, not float), but promotes to Float64 for any
        // float operand. Mod and FloorDiv preserve int if there are no zero divisors.
        let int_pow = matches!(op, ArithmeticOp::Pow)
            && self.dtype == DType::Int64
            && right.dtype == DType::Int64;
        if matches!(op, ArithmeticOp::Div | ArithmeticOp::Pow) && !int_pow {
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
                        // int ** int stays int64 (numpy/pandas). A negative integer
                        // exponent raises, matching numpy's "Integers to negative
                        // integer powers are not allowed." Overflow wraps like int64.
                        ArithmeticOp::Pow => {
                            if rhs_i64 < 0 {
                                return Err(ColumnError::NegativeIntegerPower);
                            }
                            lhs_i64.wrapping_pow(u32::try_from(rhs_i64).unwrap_or(u32::MAX))
                        }
                        ArithmeticOp::Div | ArithmeticOp::Mod | ArithmeticOp::FloorDiv => {
                            unreachable!()
                        }
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
                    ArithmeticOp::Mod => python_mod_f64(lhs, rhs),
                    ArithmeticOp::Pow => lhs.powf(rhs),
                    ArithmeticOp::FloorDiv => python_floor_div_f64(lhs, rhs),
                };

                Ok(Scalar::Float64(result))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(out_dtype, values)
    }

    /// Element-wise addition, matching `pd.Series.add()`.
    pub fn add(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::Add)
    }

    /// Reverse element-wise addition, matching `pd.Series.radd()`.
    pub fn radd(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::Add)
    }

    /// Element-wise subtraction, matching `pd.Series.sub()`.
    pub fn sub(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::Sub)
    }

    /// Reverse element-wise subtraction, matching `pd.Series.rsub()`.
    pub fn rsub(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::Sub)
    }

    /// Alias for [`sub`](Self::sub), matching `pd.Series.subtract()`.
    pub fn subtract(&self, right: &Self) -> Result<Self, ColumnError> {
        self.sub(right)
    }

    /// Element-wise multiplication, matching `pd.Series.mul()`.
    pub fn mul(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::Mul)
    }

    /// Reverse element-wise multiplication, matching `pd.Series.rmul()`.
    pub fn rmul(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::Mul)
    }

    /// Alias for [`mul`](Self::mul), matching `pd.Series.multiply()`.
    pub fn multiply(&self, right: &Self) -> Result<Self, ColumnError> {
        self.mul(right)
    }

    /// Element-wise true division, matching `pd.Series.div()`.
    pub fn div(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::Div)
    }

    /// Reverse element-wise true division, matching `pd.Series.rdiv()`.
    pub fn rdiv(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::Div)
    }

    /// Alias for [`div`](Self::div), matching `pd.Series.divide()`.
    pub fn divide(&self, right: &Self) -> Result<Self, ColumnError> {
        self.div(right)
    }

    /// Alias for [`div`](Self::div), matching `pd.Series.truediv()`.
    pub fn truediv(&self, right: &Self) -> Result<Self, ColumnError> {
        self.div(right)
    }

    /// Alias for [`rdiv`](Self::rdiv), matching `pd.Series.rtruediv()`.
    pub fn rtruediv(&self, left: &Self) -> Result<Self, ColumnError> {
        self.rdiv(left)
    }

    /// Element-wise floor division, matching `pd.Series.floordiv()`.
    pub fn floordiv(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::FloorDiv)
    }

    /// Reverse element-wise floor division, matching `pd.Series.rfloordiv()`.
    pub fn rfloordiv(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::FloorDiv)
    }

    /// Element-wise modulo, matching `pd.Series.mod()`.
    pub fn r#mod(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::Mod)
    }

    /// Reverse element-wise modulo, matching `pd.Series.rmod()`.
    pub fn rmod(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::Mod)
    }

    /// Element-wise exponentiation, matching `pd.Series.pow()`.
    pub fn pow(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_numeric(right, ArithmeticOp::Pow)
    }

    /// Reverse element-wise exponentiation, matching `pd.Series.rpow()`.
    pub fn rpow(&self, left: &Self) -> Result<Self, ColumnError> {
        left.binary_numeric(self, ArithmeticOp::Pow)
    }

    /// Alias for pow, matching NumPy naming.
    pub fn power(&self, right: &Self) -> Result<Self, ColumnError> {
        self.pow(right)
    }

    /// Float power, always returning Float64.
    ///
    /// Matches np.float_power(x, y). Unlike power(), this always returns
    /// Float64 and returns NaN for negative bases with non-integer exponents
    /// (where the result would be complex).
    pub fn float_power(&self, right: &Self) -> Result<Self, ColumnError> {
        if self.len() != right.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: right.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(right, |b, e| b.powf(e)) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (base, exp) in self.values.iter().zip(&right.values) {
            if base.is_missing() || exp.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let b = base.to_f64().map_err(ColumnError::Type)?;
            let e = exp.to_f64().map_err(ColumnError::Type)?;
            let result = b.powf(e);
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Alias for mod, matching NumPy naming.
    pub fn remainder(&self, right: &Self) -> Result<Self, ColumnError> {
        self.r#mod(right)
    }

    /// Alias for floordiv, matching NumPy naming.
    pub fn floor_divide(&self, right: &Self) -> Result<Self, ColumnError> {
        self.floordiv(right)
    }

    /// Alias for div, matching NumPy naming.
    pub fn true_divide(&self, right: &Self) -> Result<Self, ColumnError> {
        self.div(right)
    }

    /// Element-wise arctangent of y/x.
    pub fn atan2(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(other, |y, x| y.atan2(x)) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (y, x) in self.values.iter().zip(&other.values) {
            if y.is_missing() || x.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let yf = y.to_f64().map_err(ColumnError::Type)?;
            let xf = x.to_f64().map_err(ColumnError::Type)?;
            out.push(Scalar::Float64(yf.atan2(xf)));
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise Euclidean distance sqrt(x^2 + y^2).
    pub fn hypot(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(other, |a, b| a.hypot(b)) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let af = a.to_f64().map_err(ColumnError::Type)?;
            let bf = b.to_f64().map_err(ColumnError::Type)?;
            out.push(Scalar::Float64(af.hypot(bf)));
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise floating-point remainder (fmod).
    pub fn fmod(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(other, |a, b| a % b) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let af = a.to_f64().map_err(ColumnError::Type)?;
            let bf = b.to_f64().map_err(ColumnError::Type)?;
            out.push(Scalar::Float64(af % bf));
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise copysign: magnitude of self with sign of other.
    pub fn copysign(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(other, |m, s| m.copysign(s)) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (mag, sign) in self.values.iter().zip(&other.values) {
            if mag.is_missing() || sign.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let mf = mag.to_f64().map_err(ColumnError::Type)?;
            let sf = sign.to_f64().map_err(ColumnError::Type)?;
            out.push(Scalar::Float64(mf.copysign(sf)));
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise sign: -1, 0, or 1.
    pub fn sign(&self) -> Result<Self, ColumnError> {
        // Typed, dtype-preserving fast path (all-valid only): Int64 -> Int64
        // (-1/0/1), Float64 -> Float64. all-valid Float64 has no NaN so the
        // is_nan branch never fires; -0.0 -> 0.0 (neither >0.0 nor <0.0), exactly
        // as the scalar loop. Bit-identical.
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_i64_values(
                data.iter()
                    .map(|&x| {
                        if x > 0 {
                            1
                        } else if x < 0 {
                            -1
                        } else {
                            0
                        }
                    })
                    .collect(),
            ));
        }
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(
                data.iter()
                    .map(|&x| {
                        if x > 0.0 {
                            1.0
                        } else if x < 0.0 {
                            -1.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            ));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    let s = if *x > 0 {
                        1
                    } else if *x < 0 {
                        -1
                    } else {
                        0
                    };
                    out.push(Scalar::Int64(s));
                }
                Scalar::Float64(x) => {
                    let s = if x.is_nan() {
                        f64::NAN
                    } else if *x > 0.0 {
                        1.0
                    } else if *x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    };
                    out.push(Scalar::Float64(s));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        let dtype = match self.dtype {
            DType::Int64 => DType::Int64,
            _ => DType::Float64,
        };
        Self::new(dtype, out)
    }

    /// Test element-wise for negative sign bit.
    ///
    /// Matches np.signbit(x). Returns True for negative values including -0.0.
    pub fn signbit(&self) -> Result<Self, ColumnError> {
        // Typed fast path (all-valid only, output Bool): Int64 sign via x < 0,
        // Float64 via is_sign_negative (so -0.0 -> true). Bit-identical; all-valid
        // ⇒ the missing -> Bool(false) branch never applies.
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_bool_values(
                data.iter().map(|&x| x < 0).collect(),
            ));
        }
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_bool_values(
                data.iter().map(|&x| x.is_sign_negative()).collect(),
            ));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Bool(false));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Bool(*x < 0)),
                Scalar::Float64(x) => out.push(Scalar::Bool(x.is_sign_negative())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Bool, out)
    }

    /// Compute the Heaviside step function.
    ///
    /// Matches np.heaviside(x, h0). Returns:
    /// - 0 where x < 0
    /// - h0 where x == 0
    /// - 1 where x > 0
    pub fn heaviside(&self, h0: f64) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    let val = if *x < 0 {
                        0.0
                    } else if *x > 0 {
                        1.0
                    } else {
                        h0
                    };
                    out.push(Scalar::Float64(val));
                }
                Scalar::Float64(x) => {
                    let val = if x.is_nan() {
                        f64::NAN
                    } else if *x < 0.0 {
                        0.0
                    } else if *x > 0.0 {
                        1.0
                    } else {
                        h0
                    };
                    out.push(Scalar::Float64(val));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise greatest common divisor.
    ///
    /// Matches np.gcd(x, y). Works on integer values.
    pub fn gcd(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        fn compute_gcd(mut a: i64, mut b: i64) -> i64 {
            a = a.abs();
            b = b.abs();
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            a
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => {
                    out.push(Scalar::Int64(compute_gcd(*x, *y)));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Int64, out)
    }

    /// Element-wise least common multiple.
    ///
    /// Matches np.lcm(x, y). Works on integer values.
    pub fn lcm(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        fn compute_gcd(mut a: i64, mut b: i64) -> i64 {
            a = a.abs();
            b = b.abs();
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            a
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => {
                    let g = compute_gcd(*x, *y);
                    let result = if g == 0 { 0 } else { (x.abs() / g) * y.abs() };
                    out.push(Scalar::Int64(result));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Int64, out)
    }

    /// Element-wise bitwise AND.
    pub fn bitwise_and(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => out.push(Scalar::Int64(x & y)),
                (Scalar::Bool(x), Scalar::Bool(y)) => out.push(Scalar::Bool(*x && *y)),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(self.dtype, out)
    }

    /// Element-wise bitwise OR.
    pub fn bitwise_or(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => out.push(Scalar::Int64(x | y)),
                (Scalar::Bool(x), Scalar::Bool(y)) => out.push(Scalar::Bool(*x || *y)),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(self.dtype, out)
    }

    /// Element-wise bitwise XOR.
    pub fn bitwise_xor(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => out.push(Scalar::Int64(x ^ y)),
                (Scalar::Bool(x), Scalar::Bool(y)) => out.push(Scalar::Bool(*x ^ *y)),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(self.dtype, out)
    }

    /// Element-wise left bit shift.
    ///
    /// Matches np.left_shift(x, y). Shifts bits of x left by y positions.
    pub fn left_shift(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => {
                    let shift = (*y).clamp(0, 63) as u32;
                    out.push(Scalar::Int64(x.wrapping_shl(shift)));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Int64, out)
    }

    /// Element-wise right bit shift.
    ///
    /// Matches np.right_shift(x, y). Shifts bits of x right by y positions.
    pub fn right_shift(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => {
                    let shift = (*y).clamp(0, 63) as u32;
                    out.push(Scalar::Int64(x.wrapping_shr(shift)));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{a:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Int64, out)
    }

    /// Element-wise bitwise NOT (invert).
    pub fn bitwise_not(&self) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Int64(!x)),
                Scalar::Bool(x) => out.push(Scalar::Bool(!x)),
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

    /// Alias for bitwise_not.
    pub fn invert(&self) -> Result<Self, ColumnError> {
        self.bitwise_not()
    }

    /// Element-wise maximum, NaN propagates.
    pub fn maximum(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        // all-valid ⇒ no NaN (NaN floats mark the column invalid), so the scalar
        // loop's is_nan branch never fires and the result is af.max(bf).
        if let Some(out) = self.typed_float_binary(other, f64::max) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let af = a.to_f64().map_err(ColumnError::Type)?;
            let bf = b.to_f64().map_err(ColumnError::Type)?;
            if af.is_nan() || bf.is_nan() {
                out.push(Scalar::Float64(f64::NAN));
            } else {
                out.push(Scalar::Float64(af.max(bf)));
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise minimum, NaN propagates.
    pub fn minimum(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(other, f64::min) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let af = a.to_f64().map_err(ColumnError::Type)?;
            let bf = b.to_f64().map_err(ColumnError::Type)?;
            if af.is_nan() || bf.is_nan() {
                out.push(Scalar::Float64(f64::NAN));
            } else {
                out.push(Scalar::Float64(af.min(bf)));
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise maximum, ignoring NaN.
    pub fn fmax(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        // all-valid numeric ⇒ both to_f64().ok() are Some(non-NaN), so the result
        // is x.max(y) — same as `maximum` on this domain.
        if let Some(out) = self.typed_float_binary(other, f64::max) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            let af = a.to_f64().ok();
            let bf = b.to_f64().ok();
            let result = match (af, bf) {
                (Some(x), Some(y)) if x.is_nan() => y,
                (Some(x), Some(y)) if y.is_nan() => x,
                (Some(x), Some(y)) => x.max(y),
                (Some(x), None) => x,
                (None, Some(y)) => y,
                (None, None) => f64::NAN,
            };
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Element-wise minimum, ignoring NaN.
    pub fn fmin(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        if let Some(out) = self.typed_float_binary(other, f64::min) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            let af = a.to_f64().ok();
            let bf = b.to_f64().ok();
            let result = match (af, bf) {
                (Some(x), Some(y)) if x.is_nan() => y,
                (Some(x), Some(y)) if y.is_nan() => x,
                (Some(x), Some(y)) => x.min(y),
                (Some(x), None) => x,
                (None, Some(y)) => y,
                (None, None) => f64::NAN,
            };
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Logical AND between two boolean columns.
    pub fn logical_and(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            let av = match a {
                Scalar::Bool(x) => *x,
                _ => a.to_f64().map(|v| v != 0.0).unwrap_or(false),
            };
            let bv = match b {
                Scalar::Bool(x) => *x,
                _ => b.to_f64().map(|v| v != 0.0).unwrap_or(false),
            };
            out.push(Scalar::Bool(av && bv));
        }
        Self::new(DType::Bool, out)
    }

    /// Logical OR between two boolean columns.
    pub fn logical_or(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            let av = match a {
                Scalar::Bool(x) => *x,
                _ => a.to_f64().map(|v| v != 0.0).unwrap_or(false),
            };
            let bv = match b {
                Scalar::Bool(x) => *x,
                _ => b.to_f64().map(|v| v != 0.0).unwrap_or(false),
            };
            out.push(Scalar::Bool(av || bv));
        }
        Self::new(DType::Bool, out)
    }

    /// Logical XOR between two boolean columns.
    pub fn logical_xor(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            let av = match a {
                Scalar::Bool(x) => *x,
                _ => a.to_f64().map(|v| v != 0.0).unwrap_or(false),
            };
            let bv = match b {
                Scalar::Bool(x) => *x,
                _ => b.to_f64().map(|v| v != 0.0).unwrap_or(false),
            };
            out.push(Scalar::Bool(av ^ bv));
        }
        Self::new(DType::Bool, out)
    }

    /// Logical NOT (element-wise negation to boolean).
    pub fn logical_not(&self) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Null(NullKind::Null));
                continue;
            }
            let bv = match v {
                Scalar::Bool(x) => *x,
                _ => v.to_f64().map(|x| x != 0.0).unwrap_or(false),
            };
            out.push(Scalar::Bool(!bv));
        }
        Self::new(DType::Bool, out)
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

        // Typed fast path: both operands are all-valid contiguous Float64 (resp.
        // Int64), so compare over the buffers and build the Bool result via
        // from_bool_values — no Scalar materialization or per-element dispatch.
        // Bit-identical to scalar_compare's same-dtype arm (the identical `a <op>
        // b`); all-valid inputs mean no Null branch, and the comparisons never
        // see a NaN (as_f64_slice excludes it).
        if let (Some(l), Some(r)) = (self.as_f64_slice(), right.as_f64_slice()) {
            let bools: Vec<bool> = l
                .iter()
                .zip(r)
                .map(|(&a, &b)| match op {
                    ComparisonOp::Gt => a > b,
                    ComparisonOp::Lt => a < b,
                    ComparisonOp::Eq => a == b,
                    ComparisonOp::Ne => a != b,
                    ComparisonOp::Ge => a >= b,
                    ComparisonOp::Le => a <= b,
                })
                .collect();
            return Ok(Self::from_bool_values(bools));
        }
        if let (Some(l), Some(r)) = (self.as_i64_slice(), right.as_i64_slice()) {
            let bools: Vec<bool> = l
                .iter()
                .zip(r)
                .map(|(&a, &b)| match op {
                    ComparisonOp::Gt => a > b,
                    ComparisonOp::Lt => a < b,
                    ComparisonOp::Eq => a == b,
                    ComparisonOp::Ne => a != b,
                    ComparisonOp::Ge => a >= b,
                    ComparisonOp::Le => a <= b,
                })
                .collect();
            return Ok(Self::from_bool_values(bools));
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

    /// Element-wise equality, matching `pd.Series.eq()`.
    pub fn eq(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_comparison(right, ComparisonOp::Eq)
    }

    /// Element-wise inequality, matching `pd.Series.ne()`.
    pub fn ne(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_comparison(right, ComparisonOp::Ne)
    }

    /// Element-wise less-than comparison, matching `pd.Series.lt()`.
    pub fn lt(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_comparison(right, ComparisonOp::Lt)
    }

    /// Element-wise less-than-or-equal comparison, matching `pd.Series.le()`.
    pub fn le(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_comparison(right, ComparisonOp::Le)
    }

    /// Element-wise greater-than comparison, matching `pd.Series.gt()`.
    pub fn gt(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_comparison(right, ComparisonOp::Gt)
    }

    /// Element-wise greater-than-or-equal comparison, matching `pd.Series.ge()`.
    pub fn ge(&self, right: &Self) -> Result<Self, ColumnError> {
        self.binary_comparison(right, ComparisonOp::Ge)
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

        // Typed fast path (br-frankenpandas-2kpwa): when self is an all-valid
        // contiguous numeric buffer, compare against the scalar directly over the
        // typed slice and build the Bool result via from_bool_values, skipping the
        // per-element Scalar dispatch in scalar_compare and the 32-byte Scalar alloc
        // for every output cell. Bit-identical to the scalar path:
        //   * Float64 self vs any numeric scalar reduces in scalar_compare to the
        //     final "convert both to f64" branch, i.e. `v <op> scalar.to_f64()`.
        //   * Int64 self vs Int64 scalar takes scalar_compare's both-Int64 branch,
        //     i.e. the i64 comparison. (Int64 vs float scalar still uses the AoS
        //     path so f64-promotion semantics stay identical.)
        if let Some(data) = self.as_f64_slice()
            && let Ok(s) = scalar.to_f64()
        {
            let bools: Vec<bool> = data
                .iter()
                .map(|&v| match op {
                    ComparisonOp::Gt => v > s,
                    ComparisonOp::Lt => v < s,
                    ComparisonOp::Eq => v == s,
                    ComparisonOp::Ne => v != s,
                    ComparisonOp::Ge => v >= s,
                    ComparisonOp::Le => v <= s,
                })
                .collect();
            return Ok(Self::from_bool_values(bools));
        }
        if let Some(data) = self.as_i64_slice()
            && let Scalar::Int64(s) = scalar
        {
            let s = *s;
            let bools: Vec<bool> = data
                .iter()
                .map(|&v| match op {
                    ComparisonOp::Gt => v > s,
                    ComparisonOp::Lt => v < s,
                    ComparisonOp::Eq => v == s,
                    ComparisonOp::Ne => v != s,
                    ComparisonOp::Ge => v >= s,
                    ComparisonOp::Le => v <= s,
                })
                .collect();
            return Ok(Self::from_bool_values(bools));
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

        // Typed fast path (br-frankenpandas-lei31 family): gather the contiguous
        // f64/i64 buffer by the mask directly, skipping the 32-byte Scalar clone
        // and the dtype-coercion scan in Column::new. Bit-identical — selects the
        // same positions in the same order, and an all-valid source yields an
        // all-valid result, exactly as the Scalar path + Column::new would.
        //
        // When the mask is an all-valid Bool column (the usual shape — it just
        // came out of a comparison), read its contiguous `bool` buffer instead of
        // `mask.values` (which would force the lazy-bool mask to materialize a
        // full Vec<Scalar::Bool>). `as_bool_slice` returns the raw bits, and for
        // an all-valid bool column `bits[i] == matches!(values[i],
        // Scalar::Bool(true))`, so selection is identical.
        if let Some(mask_bits) = mask.as_bool_slice() {
            if let Some(data) = self.as_f64_slice() {
                let gathered: Vec<f64> = data
                    .iter()
                    .zip(mask_bits)
                    .filter_map(|(&v, &m)| m.then_some(v))
                    .collect();
                return Ok(Self::from_f64_values(gathered));
            }
            if let Some(data) = self.as_i64_slice() {
                let gathered: Vec<i64> = data
                    .iter()
                    .zip(mask_bits)
                    .filter_map(|(&v, &m)| m.then_some(v))
                    .collect();
                return Ok(Self::from_i64_values(gathered));
            }
            let values = self
                .values
                .iter()
                .zip(mask_bits)
                .filter_map(|(val, &m)| m.then_some(val.clone()))
                .collect::<Vec<_>>();
            return Self::new(self.dtype, values);
        }

        if let Some(data) = self.as_f64_slice() {
            let gathered: Vec<f64> = data
                .iter()
                .zip(mask.values.iter())
                .filter_map(|(&v, m)| matches!(m, Scalar::Bool(true)).then_some(v))
                .collect();
            return Ok(Self::from_f64_values(gathered));
        }
        if let Some(data) = self.as_i64_slice() {
            let gathered: Vec<i64> = data
                .iter()
                .zip(mask.values.iter())
                .filter_map(|(&v, m)| matches!(m, Scalar::Bool(true)).then_some(v))
                .collect();
            return Ok(Self::from_i64_values(gathered));
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

    /// Replace elements at specified indices with given values.
    ///
    /// Matches np.put(). Returns a new column with values replaced at indices.
    pub fn put(&self, indices: &[usize], values: &[Scalar]) -> Result<Self, ColumnError> {
        if indices.len() != values.len() {
            return Err(ColumnError::LengthMismatch {
                left: indices.len(),
                right: values.len(),
            });
        }
        let mut out = self.values.to_vec();
        for (&i, v) in indices.iter().zip(values) {
            if i >= out.len() {
                return Err(ColumnError::LengthMismatch {
                    left: out.len(),
                    right: i,
                });
            }
            out[i] = v.clone();
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
        let end = start.saturating_add(len).min(self.values.len());
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

    /// Split column into n equal-ish parts.
    ///
    /// Matches np.array_split(). Returns Vec of Columns.
    pub fn array_split(&self, n: usize) -> Result<Vec<Self>, ColumnError> {
        if n == 0 {
            return Ok(Vec::new());
        }
        let len = self.values.len();
        let base_size = len / n;
        let remainder = len % n;
        let mut result = Vec::with_capacity(n);
        let mut start = 0;
        for i in 0..n {
            let size = base_size + if i < remainder { 1 } else { 0 };
            let part = self.slice(start, size)?;
            result.push(part);
            start += size;
        }
        Ok(result)
    }

    /// Alias for array_split.
    pub fn split(&self, n: usize) -> Result<Vec<Self>, ColumnError> {
        self.array_split(n)
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

    /// Alias for concat, matching np.append.
    pub fn append(&self, other: &Self) -> Result<Self, ColumnError> {
        self.concat(other)
    }

    /// Insert values at given index.
    ///
    /// Matches np.insert(). Returns new column with values inserted.
    pub fn insert(&self, index: usize, values: &[Scalar]) -> Result<Self, ColumnError> {
        let idx = index.min(self.values.len());
        let mut out = Vec::with_capacity(self.values.len() + values.len());
        out.extend_from_slice(&self.values[..idx]);
        out.extend_from_slice(values);
        out.extend_from_slice(&self.values[idx..]);
        Self::new(self.dtype, out)
    }

    /// Delete values at given indices.
    ///
    /// Matches np.delete(). Returns new column with values removed.
    pub fn delete(&self, indices: &[usize]) -> Result<Self, ColumnError> {
        let mut to_delete: FxHashSet<usize> = FxHashSet::default();
        for &i in indices {
            to_delete.insert(i);
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .enumerate()
            .filter(|(i, _)| !to_delete.contains(i))
            .map(|(_, v)| v.clone())
            .collect();
        Self::new(self.dtype, out)
    }

    /// Resize column to new size, padding or truncating as needed.
    ///
    /// Matches np.resize(). If new size is larger, values cycle from beginning.
    pub fn resize(&self, new_size: usize) -> Result<Self, ColumnError> {
        if new_size == 0 || self.values.is_empty() {
            return Self::new(self.dtype, Vec::new());
        }
        let mut out = Vec::with_capacity(new_size);
        let mut i = 0;
        while out.len() < new_size {
            out.push(self.values[i % self.values.len()].clone());
            i += 1;
        }
        Self::new(self.dtype, out)
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

    /// Tile (repeat) the entire column n times.
    ///
    /// Matches `np.tile()`. Unlike repeat which duplicates each element,
    /// tile duplicates the entire array.
    pub fn tile(&self, reps: usize) -> Result<Self, ColumnError> {
        if reps == 0 {
            return Self::new(self.dtype, Vec::new());
        }
        if reps == 1 {
            return Ok(self.clone());
        }
        let mut out = Vec::with_capacity(self.values.len() * reps);
        for _ in 0..reps {
            out.extend_from_slice(&self.values);
        }
        Self::new(self.dtype, out)
    }

    /// Reverse the row order of the column.
    ///
    /// Matches `pd.Series[::-1]` / `iloc[::-1]`. Dtype is preserved.
    pub fn reverse(&self) -> Result<Self, ColumnError> {
        let mut values = self.values.to_vec();
        values.reverse();
        Self::new(self.dtype, values)
    }

    /// Alias for reverse, matching np.flip.
    pub fn flip(&self) -> Result<Self, ColumnError> {
        self.reverse()
    }

    /// Roll array elements along the axis.
    ///
    /// Matches np.roll(a, shift). Elements that roll beyond the last position
    /// are re-introduced at the first, and vice versa. Positive shift rolls
    /// elements to higher indices (right), negative to lower (left).
    pub fn roll(&self, shift: i64) -> Result<Self, ColumnError> {
        let len = self.len();
        if len == 0 {
            return Ok(self.clone());
        }
        let shift = ((shift % len as i64) + len as i64) as usize % len;
        if shift == 0 {
            return Ok(self.clone());
        }
        let mut out = Vec::with_capacity(len);
        let split = len - shift;
        out.extend_from_slice(&self.values[split..]);
        out.extend_from_slice(&self.values[..split]);
        Self::new(self.dtype, out)
    }

    /// Filter values based on a boolean condition column.
    ///
    /// Matches `np.compress()`. Returns only values where condition is True.
    pub fn compress(&self, condition: &Self) -> Result<Self, ColumnError> {
        if self.len() != condition.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: condition.len(),
            });
        }
        let mut out = Vec::new();
        for (v, c) in self.values.iter().zip(&condition.values) {
            match c {
                Scalar::Bool(true) => out.push(v.clone()),
                Scalar::Bool(false) => {}
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{c:?}"),
                        dtype: condition.dtype,
                    }));
                }
            }
        }
        Self::new(self.dtype, out)
    }

    /// Cumulative sum, null-propagating per fp-types::nancumsum.
    ///
    /// Matches `pd.Series.cumsum()`. The resulting column is always
    /// Float64 (matching the numeric accumulator type used in
    /// nancumsum).
    pub fn cumsum(&self) -> Result<Self, ColumnError> {
        // Typed prefix scan: an all-valid Float64 column scans its contiguous
        // buffer, no Scalar materialization in or out. Bit-identical to
        // nancumsum's Float64 arm — the same `running += x` left-fold seeded at
        // 0.0 in the same order; an operation-produced NaN (inf - inf) is flagged
        // missing by from_f64_values, exactly as Self::new(Float64, ...) does.
        if let Some(data) = self.as_f64_slice() {
            let mut running = 0.0_f64;
            let out: Vec<f64> = data
                .iter()
                .map(|&x| {
                    running += x;
                    running
                })
                .collect();
            return Ok(Self::from_f64_values(out));
        }
        let out = nancumsum(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Cumulative product, null-propagating per fp-types::nancumprod.
    pub fn cumprod(&self) -> Result<Self, ColumnError> {
        // Typed prefix scan (see cumsum): nancumprod seeds `running` at 1.0.
        if let Some(data) = self.as_f64_slice() {
            let mut running = 1.0_f64;
            let out: Vec<f64> = data
                .iter()
                .map(|&x| {
                    running *= x;
                    running
                })
                .collect();
            return Ok(Self::from_f64_values(out));
        }
        let out = nancumprod(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Cumulative maximum, null-propagating per fp-types::nancummax.
    pub fn cummax(&self) -> Result<Self, ColumnError> {
        // Typed running maximum: nancummax takes the first non-missing value
        // then `prev.max(x)` (std f64::max semantics) — reproduced over the
        // contiguous buffer for an all-valid Float64 column.
        if let Some(data) = self.as_f64_slice() {
            if let Some((&first, rest)) = data.split_first() {
                let mut running = first;
                let mut out = Vec::with_capacity(data.len());
                out.push(running);
                for &x in rest {
                    running = running.max(x);
                    out.push(running);
                }
                return Ok(Self::from_f64_values(out));
            }
            return Ok(Self::from_f64_values(Vec::new()));
        }
        let out = nancummax(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Cumulative minimum, null-propagating per fp-types::nancummin.
    pub fn cummin(&self) -> Result<Self, ColumnError> {
        // Typed running minimum (see cummax).
        if let Some(data) = self.as_f64_slice() {
            if let Some((&first, rest)) = data.split_first() {
                let mut running = first;
                let mut out = Vec::with_capacity(data.len());
                out.push(running);
                for &x in rest {
                    running = running.min(x);
                    out.push(running);
                }
                return Ok(Self::from_f64_values(out));
            }
            return Ok(Self::from_f64_values(Vec::new()));
        }
        let out = nancummin(&self.values);
        Self::new(DType::Float64, out)
    }

    /// Sum of non-missing values.
    ///
    /// Matches `pd.Series.sum()` in skipna=True mode via fp-types::nansum.
    /// Empty column returns 0.0 (matching pandas).
    #[must_use]
    pub fn sum(&self) -> Scalar {
        // Typed reduction: an all-valid Float64 column sums straight over its
        // contiguous buffer instead of materializing/iterating a Vec<Scalar>.
        // Bit-identical to nansum's Float64 arm: a sequential left-fold seeded
        // at 0.0 over the same values in the same order (no Timedelta/missing
        // branch applies to an all-valid Float64 column).
        if let Some(data) = self.as_f64_slice() {
            let mut s = 0.0_f64;
            for &x in data {
                s += x;
            }
            return Scalar::Float64(s);
        }
        nansum(&self.values)
    }

    /// Arithmetic mean of non-missing values.
    ///
    /// Matches `pd.Series.mean()` via fp-types::nanmean. Empty column
    /// returns Null(NaN).
    #[must_use]
    pub fn mean(&self) -> Scalar {
        // Typed reduction (see `sum`): for an all-valid Float64 column nanmean
        // is `Σ / count` with count == len; an empty column stays Null(NaN).
        if let Some(data) = self.as_f64_slice() {
            if data.is_empty() {
                return Scalar::Null(NullKind::NaN);
            }
            let mut s = 0.0_f64;
            for &x in data {
                s += x;
            }
            return Scalar::Float64(s / data.len() as f64);
        }
        nanmean(&self.values)
    }

    /// Weighted average of non-missing values.
    ///
    /// Matches np.average(a, weights=w). Returns NaN if weights sum to zero.
    #[must_use]
    pub fn weighted_mean(&self, weights: &Self) -> Scalar {
        if self.len() != weights.len() {
            return Scalar::Null(NullKind::NaN);
        }
        let mut sum = 0.0;
        let mut weight_sum = 0.0;
        for (v, w) in self.values.iter().zip(weights.values()) {
            if v.is_missing() || w.is_missing() {
                continue;
            }
            let vf = match v.to_f64() {
                Ok(x) => x,
                Err(_) => continue,
            };
            let wf = match w.to_f64() {
                Ok(x) => x,
                Err(_) => continue,
            };
            sum += vf * wf;
            weight_sum += wf;
        }
        if weight_sum == 0.0 {
            return Scalar::Null(NullKind::NaN);
        }
        Scalar::Float64(sum / weight_sum)
    }

    /// Alias for weighted_mean, matching np.average naming.
    #[must_use]
    pub fn average(&self, weights: &Self) -> Scalar {
        self.weighted_mean(weights)
    }

    /// Minimum non-missing value.
    ///
    /// Matches `pd.Series.min()` via fp-types::nanmin. Preserves dtype
    /// for homogeneous inputs.
    #[must_use]
    pub fn min(&self) -> Scalar {
        // Typed reduction: an all-valid numeric column folds the minimum straight
        // over its contiguous buffer (an associative reduction the compiler can
        // vectorize), skipping the Vec<Scalar> materialization. Bit-identical to
        // nanmin: it keeps the first element on a tie via strict `<` (so -0.0 vs
        // 0.0 ordering is preserved), and returns it dtype-preserved.
        if let Some(data) = self.as_f64_slice()
            && let Some((&first, rest)) = data.split_first()
        {
            let mut m = first;
            for &x in rest {
                if x < m {
                    m = x;
                }
            }
            return Scalar::Float64(m);
        }
        if let Some(data) = self.as_i64_slice()
            && let Some((&first, rest)) = data.split_first()
        {
            let mut m = first;
            for &x in rest {
                if x < m {
                    m = x;
                }
            }
            return Scalar::Int64(m);
        }
        nanmin(&self.values)
    }

    /// Maximum non-missing value.
    ///
    /// Matches `pd.Series.max()` via fp-types::nanmax.
    #[must_use]
    pub fn max(&self) -> Scalar {
        // Typed reduction (see `min`); nanmax keeps the first element on a tie
        // via strict `>`, dtype-preserved.
        if let Some(data) = self.as_f64_slice()
            && let Some((&first, rest)) = data.split_first()
        {
            let mut m = first;
            for &x in rest {
                if x > m {
                    m = x;
                }
            }
            return Scalar::Float64(m);
        }
        if let Some(data) = self.as_i64_slice()
            && let Some((&first, rest)) = data.split_first()
        {
            let mut m = first;
            for &x in rest {
                if x > m {
                    m = x;
                }
            }
            return Scalar::Int64(m);
        }
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
        // Typed reduction (mirror of `sum`): an all-valid Float64 column
        // multiplies straight over its contiguous buffer instead of iterating a
        // Vec<Scalar>. Bit-identical to nanprod's Float64 arm: a sequential
        // left-fold seeded at 1.0 over the same values in the same order. all-
        // valid ⇒ nothing is filtered and the all-missing→Null branch can't fire,
        // so empty folds to Float64(1.0) exactly as nanprod returns for empty.
        if let Some(data) = self.as_f64_slice() {
            let mut p = 1.0_f64;
            for &x in data {
                p *= x;
            }
            return Scalar::Float64(p);
        }
        nanprod(&self.values)
    }

    /// Alias for [`prod`](Self::prod), matching `pd.Series.product()`.
    #[must_use]
    pub fn product(&self) -> Scalar {
        self.prod()
    }

    /// Alias for sum, matching np.nansum.
    #[must_use]
    pub fn nansum(&self) -> Scalar {
        self.sum()
    }

    /// Alias for mean, matching np.nanmean.
    #[must_use]
    pub fn nanmean(&self) -> Scalar {
        self.mean()
    }

    /// Alias for min, matching np.nanmin.
    #[must_use]
    pub fn nanmin(&self) -> Scalar {
        self.min()
    }

    /// Alias for max, matching np.nanmax.
    #[must_use]
    pub fn nanmax(&self) -> Scalar {
        self.max()
    }

    /// Alias for prod, matching np.nanprod.
    #[must_use]
    pub fn nanprod(&self) -> Scalar {
        self.prod()
    }

    /// Alias for std, matching np.nanstd.
    #[must_use]
    pub fn nanstd(&self, ddof: usize) -> Scalar {
        self.std(ddof)
    }

    /// Alias for var, matching np.nanvar.
    #[must_use]
    pub fn nanvar(&self, ddof: usize) -> Scalar {
        self.var(ddof)
    }

    /// Alias for median, matching np.nanmedian.
    #[must_use]
    pub fn nanmedian(&self) -> Scalar {
        self.median()
    }

    fn skipna_false_missing_result(&self, skipna: bool) -> Option<Scalar> {
        if skipna || !self.values.iter().any(Scalar::is_missing) {
            return None;
        }

        Some(if matches!(self.dtype, DType::Timedelta64) {
            Scalar::Timedelta64(Timedelta::NAT)
        } else {
            Scalar::Float64(f64::NAN)
        })
    }

    /// Sum with explicit pandas `skipna=` control.
    ///
    /// Matches `pd.Series.sum(skipna=...)`.
    #[must_use]
    pub fn sum_skipna(&self, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.sum())
    }

    /// Mean with explicit pandas `skipna=` control.
    #[must_use]
    pub fn mean_skipna(&self, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.mean())
    }

    /// Minimum with explicit pandas `skipna=` control.
    #[must_use]
    pub fn min_skipna(&self, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.min())
    }

    /// Maximum with explicit pandas `skipna=` control.
    #[must_use]
    pub fn max_skipna(&self, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.max())
    }

    /// Median with explicit pandas `skipna=` control.
    #[must_use]
    pub fn median_skipna(&self, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.median())
    }

    /// Product with explicit pandas `skipna=` control.
    #[must_use]
    pub fn prod_skipna(&self, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.prod())
    }

    /// Variance with explicit pandas `skipna=` control.
    #[must_use]
    pub fn var_skipna(&self, ddof: usize, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.var(ddof))
    }

    /// Standard deviation with explicit pandas `skipna=` control.
    #[must_use]
    pub fn std_skipna(&self, ddof: usize, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.std(ddof))
    }

    /// Standard error of the mean with explicit pandas `skipna=` control.
    #[must_use]
    pub fn sem_skipna(&self, ddof: usize, skipna: bool) -> Scalar {
        self.skipna_false_missing_result(skipna)
            .unwrap_or_else(|| self.sem(ddof))
    }

    /// Count of non-missing values.
    ///
    /// Matches `pd.Series.count()`.
    #[must_use]
    pub fn count(&self) -> usize {
        self.values.iter().filter(|v| !v.is_missing()).count()
    }

    /// Forward-fill missing values with the most recent non-missing
    /// value, optionally capped by `limit` consecutive fills.
    ///
    /// Matches `pd.Series.ffill(limit=None)`. Leading nulls stay null
    /// until the first non-missing value is seen. `limit=None` means
    /// unbounded; `limit=Some(k)` caps each missing run to `k` fills.
    pub fn ffill(&self, limit: Option<usize>) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        let mut last: Option<Scalar> = None;
        let mut run = 0usize;
        for v in &self.values {
            if !v.is_missing() {
                out.push(v.clone());
                last = Some(v.clone());
                run = 0;
                continue;
            }
            match (&last, limit) {
                (Some(prev), None) => out.push(prev.clone()),
                (Some(prev), Some(cap)) if run < cap => {
                    out.push(prev.clone());
                    run += 1;
                }
                _ => out.push(v.clone()),
            }
        }
        Self::new(self.dtype, out)
    }

    /// Alias for [`ffill`](Self::ffill), matching deprecated `pd.Series.pad()`.
    pub fn pad(&self, limit: Option<usize>) -> Result<Self, ColumnError> {
        self.ffill(limit)
    }

    /// Backward-fill missing values with the next non-missing value,
    /// optionally capped by `limit` consecutive fills.
    ///
    /// Matches `pd.Series.bfill(limit=None)`. Trailing nulls stay null
    /// if no subsequent non-missing value is observed.
    pub fn bfill(&self, limit: Option<usize>) -> Result<Self, ColumnError> {
        let mut out = vec![Scalar::Null(NullKind::NaN); self.values.len()];
        let mut next: Option<Scalar> = None;
        let mut run = 0usize;
        for (i, v) in self.values.iter().enumerate().rev() {
            if !v.is_missing() {
                out[i] = v.clone();
                next = Some(v.clone());
                run = 0;
                continue;
            }
            match (&next, limit) {
                (Some(nxt), None) => out[i] = nxt.clone(),
                (Some(nxt), Some(cap)) if run < cap => {
                    out[i] = nxt.clone();
                    run += 1;
                }
                _ => out[i] = v.clone(),
            }
        }
        Self::new(self.dtype, out)
    }

    /// Alias for [`bfill`](Self::bfill), matching deprecated `pd.Series.backfill()`.
    pub fn backfill(&self, limit: Option<usize>) -> Result<Self, ColumnError> {
        self.bfill(limit)
    }

    /// Count of distinct non-missing values.
    ///
    /// Matches `pd.Series.nunique(dropna=True)`.
    #[must_use]
    pub fn nunique(&self) -> Scalar {
        self.nunique_with_dropna(true)
    }

    /// Count of distinct values with explicit missing-value handling.
    ///
    /// Matches `pd.Series.nunique(dropna=...)`. When `dropna=false`,
    /// all missing values contribute a single extra distinct bucket.
    #[must_use]
    pub fn nunique_with_dropna(&self, dropna: bool) -> Scalar {
        // Dense direct-address fast path: an all-valid, bounded-range Int64
        // column counts distinct values via a seen-bitset indexed by (v-min) —
        // hash-free, no Scalar enum. All-valid ⇒ no missing, so dropna does not
        // add a bucket; bit-identical to nannunique's distinct count. Same gate
        // as unique/isin/duplicated.
        if let Some(data) = self.as_i64_slice()
            && let Some((min, range)) = i64_direct_address_range(data)
        {
            let mut seen = vec![false; range];
            let mut distinct = 0i64;
            for &v in data {
                let slot = (v as i128 - min as i128) as usize;
                if !seen[slot] {
                    seen[slot] = true;
                    distinct += 1;
                }
            }
            return Scalar::Int64(distinct);
        }

        let mut distinct = match nannunique(&self.values) {
            Scalar::Int64(count) => count,
            _ => 0,
        };

        if !dropna && self.values.iter().any(Scalar::is_missing) {
            distinct += 1;
        }

        Scalar::Int64(distinct)
    }

    /// Truthiness reduction: whether any non-missing value is truthy.
    ///
    /// Matches `pd.Series.any()` in skipna=True mode. Empty column
    /// returns false (pandas convention).
    #[must_use]
    pub fn any(&self) -> Scalar {
        nanany(&self.values)
    }

    /// Truthiness reduction: whether all non-missing values are truthy.
    ///
    /// Matches `pd.Series.all()` in skipna=True mode. Empty column
    /// returns true.
    #[must_use]
    pub fn all(&self) -> Scalar {
        nanall(&self.values)
    }

    /// Element-wise difference between consecutive non-missing values.
    ///
    /// Unlike `diff(1)` — which inserts Null(NaN) for every missing
    /// input — this walker-style helper skips nulls when picking the
    /// "previous" value. Positions whose nearest preceding non-missing
    /// neighbor lies before the start of the column (i.e. the first
    /// non-missing value itself, or a missing input) emit Null(NaN).
    /// Matches the common pandas idiom `s.dropna().diff().reindex(s.index)`.
    pub fn diff_valid(&self) -> Result<Self, ColumnError> {
        let mut prev: Option<f64> = None;
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            match v.to_f64() {
                Ok(x) if !x.is_nan() => match prev {
                    Some(p) => {
                        out.push(Scalar::Float64(x - p));
                        prev = Some(x);
                    }
                    None => {
                        out.push(Scalar::Null(NullKind::NaN));
                        prev = Some(x);
                    }
                },
                Ok(_) => out.push(Scalar::Null(NullKind::NaN)),
                Err(err) => return Err(ColumnError::Type(err)),
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Deterministic uniform sampling of `n` rows with a caller-supplied
    /// seed.
    ///
    /// Matches the no-replacement subset of `pd.Series.sample(n,
    /// random_state=seed)`. `n >= len()` returns a clone. Uses an
    /// in-place partial Fisher-Yates shuffle driven by a stateless
    /// LCG so callers can reproduce samples without dragging in
    /// `rand`. Result dtype matches `self`.
    pub fn sample(&self, n: usize, seed: u64) -> Result<Self, ColumnError> {
        let len = self.values.len();
        if n >= len {
            return Ok(self.clone());
        }
        let mut indices: Vec<usize> = (0..len).collect();
        let mut state = seed.wrapping_add(0x9E3779B97F4A7C15);
        for i in 0..n {
            // Standard LCG constants from Knuth (MMIX).
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let bound = (len - i) as u64;
            let pick = i + (state.wrapping_shr(33) % bound) as usize;
            indices.swap(i, pick);
        }
        let values: Vec<Scalar> = indices[..n]
            .iter()
            .map(|&idx| self.values[idx].clone())
            .collect();
        Self::new(self.dtype, values)
    }

    /// Position of the first non-missing value, or None when every
    /// value is missing.
    ///
    /// Matches `pd.Series.first_valid_index()` for positional
    /// indices — callers can map the returned position through their
    /// own Index to recover a label.
    #[must_use]
    pub fn first_valid(&self) -> Option<usize> {
        self.values.iter().position(|v| !v.is_missing())
    }

    /// Alias for [`first_valid`](Self::first_valid), matching
    /// `pd.Series.first_valid_index()` for positional indices.
    #[must_use]
    pub fn first_valid_index(&self) -> Option<usize> {
        self.first_valid()
    }

    /// Position of the last non-missing value, or None when every
    /// value is missing.
    ///
    /// Matches `pd.Series.last_valid_index()` for positional indices.
    #[must_use]
    pub fn last_valid(&self) -> Option<usize> {
        self.values.iter().rposition(|v| !v.is_missing())
    }

    /// Alias for [`last_valid`](Self::last_valid), matching
    /// `pd.Series.last_valid_index()` for positional indices.
    #[must_use]
    pub fn last_valid_index(&self) -> Option<usize> {
        self.last_valid()
    }

    /// Sliding-window sum over `window` consecutive positions.
    ///
    /// Matches `pd.Series.rolling(window).sum()`. Positions with fewer
    /// than `min_periods` non-missing values in the window emit
    /// `Null(NaN)`. `min_periods=0` preserves pandas' convention that
    /// an empty window sums to 0.0. Result dtype is always Float64.
    /// `window=0` returns an all-null Float64 column the same length
    /// as self.
    pub fn rolling_window_sum(
        &self,
        window: usize,
        min_periods: usize,
    ) -> Result<Self, ColumnError> {
        let len = self.values.len();
        if window == 0 {
            return Self::new(DType::Float64, vec![Scalar::Null(NullKind::NaN); len]);
        }
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let start = (i + 1).saturating_sub(window);
            let end = i + 1;
            let mut sum = 0.0_f64;
            let mut observed = 0usize;
            for v in &self.values[start..end] {
                if v.is_missing() {
                    continue;
                }
                match v.to_f64() {
                    Ok(x) if !x.is_nan() => {
                        sum += x;
                        observed += 1;
                    }
                    Ok(_) => {}
                    Err(err) => return Err(ColumnError::Type(err)),
                }
            }
            if observed >= min_periods.max(1) || (min_periods == 0 && end - start > 0) {
                out.push(Scalar::Float64(sum));
            } else {
                out.push(Scalar::Null(NullKind::NaN));
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Per-row missing-value flag (Bool column).
    ///
    /// Matches `pd.Series.isna()` / `isnull()`.
    pub fn isnull(&self) -> Result<Self, ColumnError> {
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| Scalar::Bool(v.is_missing()))
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Alias for [`isnull`](Self::isnull), matching `pd.Series.isna()`.
    pub fn isna(&self) -> Result<Self, ColumnError> {
        self.isnull()
    }

    /// Per-row non-missing flag (Bool column).
    ///
    /// Matches `pd.Series.notna()` / `notnull()`.
    pub fn notnull(&self) -> Result<Self, ColumnError> {
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| Scalar::Bool(!v.is_missing()))
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Alias for [`notnull`](Self::notnull), matching `pd.Series.notna()`.
    pub fn notna(&self) -> Result<Self, ColumnError> {
        self.notnull()
    }

    /// Per-row check for finite values (not NaN or infinity).
    pub fn isfinite(&self) -> Result<Self, ColumnError> {
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| match v {
                Scalar::Float64(f) => Scalar::Bool(f.is_finite()),
                Scalar::Int64(_) => Scalar::Bool(true),
                _ if v.is_missing() => Scalar::Bool(false),
                _ => Scalar::Bool(true),
            })
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Per-row check for infinite values.
    pub fn isinf(&self) -> Result<Self, ColumnError> {
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| match v {
                Scalar::Float64(f) => Scalar::Bool(f.is_infinite()),
                _ => Scalar::Bool(false),
            })
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Per-row check for NaN values.
    pub fn isnan(&self) -> Result<Self, ColumnError> {
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| match v {
                Scalar::Float64(f) => Scalar::Bool(f.is_nan()),
                Scalar::Null(NullKind::NaN) => Scalar::Bool(true),
                _ => Scalar::Bool(false),
            })
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Sample variance (ddof-parameterized).
    ///
    /// Matches `pd.Series.var(ddof=1)`.
    #[must_use]
    pub fn var(&self, ddof: usize) -> Scalar {
        // Typed two-pass reduction: an all-valid Float64 column computes the
        // mean then the sum of squared deviations straight over its contiguous
        // buffer, skipping the Vec<Scalar> materialization. Bit-identical to
        // nanvar's numeric arm — the exact same `Iterator::sum::<f64>()`
        // constructs over the same values in the same order (so seed/ordering
        // match), `Null(NaN)` when count <= ddof.
        if let Some(data) = self.as_f64_slice() {
            let n = data.len();
            if n <= ddof {
                return Scalar::Null(NullKind::NaN);
            }
            let mean: f64 = data.iter().sum::<f64>() / n as f64;
            let sum_sq: f64 = data.iter().map(|&x| (x - mean).powi(2)).sum::<f64>();
            return Scalar::Float64(sum_sq / (n - ddof) as f64);
        }
        nanvar(&self.values, ddof)
    }

    /// Sample standard deviation (ddof-parameterized).
    ///
    /// Matches `pd.Series.std(ddof=1)`.
    #[must_use]
    pub fn std(&self, ddof: usize) -> Scalar {
        // For an all-valid Float64 column nanstd is sqrt(nanvar) (Float64 arm);
        // reuse the typed var. Non-Float64 (e.g. Timedelta) keep nanstd, which
        // has its own dtype-preserving path.
        if self.as_f64_slice().is_some() {
            return match self.var(ddof) {
                Scalar::Float64(v) => Scalar::Float64(v.sqrt()),
                other => other,
            };
        }
        nanstd(&self.values, ddof)
    }

    /// Standard error of the mean (ddof-parameterized).
    ///
    /// Matches `pd.Series.sem(ddof=1)`.
    #[must_use]
    pub fn sem(&self, ddof: usize) -> Scalar {
        nansem(&self.values, ddof)
    }

    /// Sample covariance between this column and another.
    ///
    /// Matches `pd.Series.cov(other)`. Uses ddof=1 by default.
    /// Returns NaN if fewer than 2 valid pairs.
    #[must_use]
    pub fn cov(&self, other: &Self) -> Scalar {
        self.cov_ddof(other, 1)
    }

    /// Sample covariance with custom ddof.
    #[must_use]
    pub fn cov_ddof(&self, other: &Self, ddof: usize) -> Scalar {
        let n = self.values.len().min(other.values.len());
        if n == 0 {
            return Scalar::Null(NullKind::NaN);
        }
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut count = 0usize;
        for i in 0..n {
            let x = match self.values[i].to_f64() {
                Ok(v) if v.is_finite() => v,
                _ => continue,
            };
            let y = match other.values[i].to_f64() {
                Ok(v) if v.is_finite() => v,
                _ => continue,
            };
            sum_x += x;
            sum_y += y;
            count += 1;
        }
        if count <= ddof {
            return Scalar::Null(NullKind::NaN);
        }
        let mean_x = sum_x / count as f64;
        let mean_y = sum_y / count as f64;
        let mut cov_sum = 0.0;
        for i in 0..n {
            let x = match self.values[i].to_f64() {
                Ok(v) if v.is_finite() => v,
                _ => continue,
            };
            let y = match other.values[i].to_f64() {
                Ok(v) if v.is_finite() => v,
                _ => continue,
            };
            cov_sum += (x - mean_x) * (y - mean_y);
        }
        Scalar::Float64(cov_sum / (count - ddof) as f64)
    }

    /// Pearson correlation coefficient between this column and another.
    ///
    /// Matches `pd.Series.corr(other)`. Returns NaN if fewer than 2 valid pairs.
    #[must_use]
    pub fn corr(&self, other: &Self) -> Scalar {
        let n = self.values.len().min(other.values.len());
        if n == 0 {
            return Scalar::Null(NullKind::NaN);
        }
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_xx = 0.0;
        let mut sum_yy = 0.0;
        let mut sum_xy = 0.0;
        let mut count = 0usize;
        for i in 0..n {
            let x = match self.values[i].to_f64() {
                Ok(v) if v.is_finite() => v,
                _ => continue,
            };
            let y = match other.values[i].to_f64() {
                Ok(v) if v.is_finite() => v,
                _ => continue,
            };
            sum_x += x;
            sum_y += y;
            sum_xx += x * x;
            sum_yy += y * y;
            sum_xy += x * y;
            count += 1;
        }
        if count < 2 {
            return Scalar::Null(NullKind::NaN);
        }
        let n_f = count as f64;
        let numerator = n_f * sum_xy - sum_x * sum_y;
        let denom_x = (n_f * sum_xx - sum_x * sum_x).sqrt();
        let denom_y = (n_f * sum_yy - sum_y * sum_y).sqrt();
        if denom_x == 0.0 || denom_y == 0.0 {
            return Scalar::Null(NullKind::NaN);
        }
        Scalar::Float64(numerator / (denom_x * denom_y))
    }

    /// Autocorrelation at a given lag.
    ///
    /// Matches `pd.Series.autocorr(lag)`. Returns NaN if fewer than 2 valid pairs.
    #[must_use]
    pub fn autocorr(&self, lag: usize) -> Scalar {
        if lag >= self.values.len() {
            return Scalar::Null(NullKind::NaN);
        }
        let shifted = match self.shift(lag as i64, Scalar::Null(NullKind::NaN)) {
            Ok(s) => s,
            Err(_) => return Scalar::Null(NullKind::NaN),
        };
        self.corr(&shifted)
    }

    /// Sample skewness (bias-corrected, Fisher-Pearson).
    ///
    /// Matches `pd.Series.skew()`. Requires at least 3 non-missing
    /// values; returns `Null(NaN)` otherwise.
    #[must_use]
    pub fn skew(&self) -> Scalar {
        nanskew(&self.values)
    }

    /// Excess sample kurtosis (Fisher's definition, bias-corrected).
    ///
    /// Matches `pd.Series.kurt()`. Requires at least 4 non-missing
    /// values; returns `Null(NaN)` otherwise.
    #[must_use]
    pub fn kurt(&self) -> Scalar {
        nankurt(&self.values)
    }

    /// Alias for [`kurt`](Self::kurt), matching `pd.Series.kurtosis()`.
    #[must_use]
    pub fn kurtosis(&self) -> Scalar {
        self.kurt()
    }

    /// Peak-to-peak range (max − min) over non-missing values.
    ///
    /// Matches `np.ptp`. Returns `Null(NaN)` for empty or all-missing
    /// columns.
    #[must_use]
    pub fn ptp(&self) -> Scalar {
        nanptp(&self.values)
    }

    /// Whether every non-missing value is distinct.
    ///
    /// Matches `pd.Series.is_unique`.
    #[must_use]
    pub fn is_unique(&self) -> bool {
        !self.has_duplicates()
    }

    /// Whether any non-missing value repeats.
    ///
    /// Matches `pd.Series.has_duplicates`.
    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
            Datetime64(i64),
            Period(i64),
            Interval(u64, u64, IntervalClosed),
        }
        let mut seen: FxHashSet<Key<'_>> = FxHashSet::default();
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
                Scalar::Datetime64(v) => Key::Datetime64(*v),
                Scalar::Period(v) => Key::Period(*v),
                Scalar::Interval(v) => {
                    let (left, right, closed) = interval_key(v);
                    Key::Interval(left, right, closed)
                }
                Scalar::Null(_) => continue,
            };
            if !seen.insert(key) {
                return true;
            }
        }
        false
    }

    /// Percent change between consecutive non-missing values.
    ///
    /// Matches `pd.Series.pct_change(periods=1)` (fill_method defaults
    /// to None on pandas 2.2+, so nulls propagate without forward fill).
    /// Result dtype Float64. Non-numeric inputs return TypeError. The
    /// leading `|periods|` positions are Null(NaN).
    pub fn pct_change(&self, periods: i64) -> Result<Self, ColumnError> {
        let len = self.values.len();
        if len == 0 || periods == 0 {
            return Self::new(DType::Float64, vec![Scalar::Null(NullKind::NaN); len]);
        }
        let abs = periods.unsigned_abs() as usize;
        let mut out: Vec<Scalar> = Vec::with_capacity(len);
        for i in 0..len {
            let prev_idx = if periods > 0 {
                i.checked_sub(abs)
            } else if i + abs < len {
                Some(i + abs)
            } else {
                None
            };
            let Some(pi) = prev_idx else {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            };
            let cur = &self.values[i];
            let prev = &self.values[pi];
            if cur.is_missing() || prev.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            // Per br-frankenpandas-mcu90: Timedelta64 pct_change matches
            // pandas — ns deltas divide as dimensionless f64. Was silently
            // NaN before via the catch-all (Timedelta64.to_f64() errors).
            if let (Scalar::Timedelta64(cur_ns), Scalar::Timedelta64(prev_ns)) = (cur, prev) {
                if *cur_ns == Timedelta::NAT || *prev_ns == Timedelta::NAT {
                    out.push(Scalar::Null(NullKind::NaN));
                    continue;
                }
                let prev_f = *prev_ns as f64;
                if prev_f.abs() < f64::EPSILON {
                    out.push(Scalar::Null(NullKind::NaN));
                } else {
                    out.push(Scalar::Float64((*cur_ns as f64 - prev_f) / prev_f));
                }
                continue;
            }
            match (cur.to_f64(), prev.to_f64()) {
                (Ok(c), Ok(p)) => {
                    if p == 0.0 || p.is_nan() || c.is_nan() {
                        out.push(Scalar::Null(NullKind::NaN));
                    } else {
                        out.push(Scalar::Float64((c - p) / p));
                    }
                }
                _ => out.push(Scalar::Null(NullKind::NaN)),
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Percentage change with optional null fill before computation.
    ///
    /// Matches `pd.Series.pct_change(periods, fill_method=..., limit=...)`.
    /// `fill_method=None` preserves pandas 2.2 default behavior (no fill).
    /// `"ffill"` / `"pad"` forward-fill missing values first, while
    /// `"bfill"` / `"backfill"` backward-fill first. `limit` caps
    /// consecutive fills and is ignored when `fill_method` is `None`.
    pub fn pct_change_with_fill(
        &self,
        periods: i64,
        fill_method: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Self, ColumnError> {
        let filled = match fill_method {
            None => self.clone(),
            Some(method) => match method {
                "ffill" | "pad" => self.ffill(limit)?,
                "bfill" | "backfill" => self.bfill(limit)?,
                other => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: other.to_string(),
                        dtype: self.dtype,
                    }));
                }
            },
        };
        filled.pct_change(periods)
    }

    /// Summary descriptive statistics.
    ///
    /// Matches `pd.Series.describe()` for numeric columns: returns the
    /// seven-value tuple (count, mean, std, min, q25, q50, q75, max)
    /// as a `Vec<(&'static str, Scalar)>` in pandas order. Non-numeric
    /// columns return TypeError. Empty or fully-missing columns
    /// produce Null(NaN) for the moment-based stats and Int64(0) for
    /// count.
    pub fn describe(&self) -> Result<Vec<(&'static str, Scalar)>, ColumnError> {
        if !matches!(
            self.dtype,
            DType::Int64 | DType::Float64 | DType::Timedelta64
        ) {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: format!("{:?}", self.dtype),
                dtype: self.dtype,
            }));
        }
        let count = Scalar::Int64(self.count() as i64);
        let mean = self.mean();
        let std = {
            let nums: Vec<f64> = self
                .values
                .iter()
                .filter(|v| !v.is_missing())
                .filter_map(|v| v.to_f64().ok())
                .collect();
            if nums.len() < 2 {
                Scalar::Null(NullKind::NaN)
            } else {
                let mu = nums.iter().sum::<f64>() / nums.len() as f64;
                let ss: f64 = nums.iter().map(|x| (x - mu).powi(2)).sum();
                Scalar::Float64((ss / (nums.len() as f64 - 1.0)).sqrt())
            }
        };
        let q25 = self.quantile(0.25);
        let q50 = self.quantile(0.5);
        let q75 = self.quantile(0.75);
        let min = self.min();
        let max = self.max();
        Ok(vec![
            ("count", count),
            ("mean", mean),
            ("std", std),
            ("min", min),
            ("25%", q25),
            ("50%", q50),
            ("75%", q75),
            ("max", max),
        ])
    }

    /// Combine two columns element-wise via `func`, using `fill` where
    /// either input is missing.
    ///
    /// Matches `pd.Series.combine(other, func, fill_value=...)`. Result
    /// length is the min of the two inputs (pandas aligns by position
    /// when inputs are the same length; longer inputs are truncated).
    /// Length mismatch returns `LengthMismatch`.
    pub fn combine<F>(
        &self,
        other: &Self,
        mut func: F,
        fill: Option<Scalar>,
    ) -> Result<Self, ColumnError>
    where
        F: FnMut(&Scalar, &Scalar) -> Scalar,
    {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| {
                let a_miss = a.is_missing();
                let b_miss = b.is_missing();
                match (a_miss || b_miss, fill.as_ref()) {
                    // pandas fill_value=None: propagate null, do not invoke func.
                    (true, None) => Scalar::Null(NullKind::NaN),
                    (_, fill_opt) => {
                        let default = fill_opt.unwrap_or(a);
                        let left = if a_miss { default } else { a };
                        let right = if b_miss { fill_opt.unwrap_or(b) } else { b };
                        func(left, right)
                    }
                }
            })
            .collect();
        let inferred = infer_dtype(&out).unwrap_or(self.dtype);
        Self::new(inferred, out)
    }

    /// Numeric-only `map` that converts each non-missing value to f64,
    /// applies `func`, and collects the result.
    ///
    /// Matches the common pattern `pd.Series.apply(lambda x: f(x))`
    /// for numeric-only transforms. Missing values pass through as
    /// Null(NaN) without invoking `func`. Non-numeric inputs return a
    /// type error on the first failing element. Result dtype is
    /// Float64.
    pub fn apply_float<F>(&self, mut func: F) -> Result<Self, ColumnError>
    where
        F: FnMut(f64) -> f64,
    {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            match v.to_f64() {
                Ok(x) => {
                    let y = func(x);
                    if y.is_nan() {
                        out.push(Scalar::Null(NullKind::NaN));
                    } else {
                        out.push(Scalar::Float64(y));
                    }
                }
                Err(err) => return Err(ColumnError::Type(err)),
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Bin non-missing values into `bins` equal-width buckets covering
    /// `[min, max]` and return the count per bin.
    ///
    /// Matches the `bins=n` histogram path behind `pd.Series.hist` (or
    /// `numpy.histogram(bins=n)[0]`). Bucket boundaries are inclusive on
    /// the low side except for the final bucket, which is inclusive on
    /// both sides. Empty columns / bins=0 yield an empty Vec.
    #[must_use]
    pub fn hist_counts(&self, bins: usize) -> Vec<usize> {
        if bins == 0 {
            return Vec::new();
        }
        let nums: Vec<f64> = self
            .values
            .iter()
            .filter(|v| !v.is_missing())
            .filter_map(|v| v.to_f64().ok())
            .filter(|f| !f.is_nan())
            .collect();
        if nums.is_empty() {
            return vec![0; bins];
        }
        let (min, max) = nums
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &x| {
                (lo.min(x), hi.max(x))
            });
        if (max - min).abs() < f64::EPSILON {
            // All values collapse into the first bin.
            let mut counts = vec![0; bins];
            counts[0] = nums.len();
            return counts;
        }
        let width = (max - min) / bins as f64;
        let mut counts = vec![0usize; bins];
        for x in &nums {
            let mut idx = ((x - min) / width) as usize;
            if idx >= bins {
                idx = bins - 1;
            }
            counts[idx] += 1;
        }
        counts
    }

    /// Position of the smallest non-missing value, or None when every
    /// value is missing.
    ///
    /// Matches `pd.Series.argmin()` (skipna=True). Ties resolve to the
    /// first position seen.
    #[must_use]
    pub fn argmin(&self) -> Option<usize> {
        nanargmin(&self.values)
    }

    /// Alias for [`argmin`](Self::argmin), matching `pd.Series.idxmin()`
    /// for positional indices.
    #[must_use]
    pub fn idxmin(&self) -> Option<usize> {
        self.argmin()
    }

    /// Position of the largest non-missing value, or None when every
    /// value is missing.
    ///
    /// Matches `pd.Series.argmax()`.
    #[must_use]
    pub fn argmax(&self) -> Option<usize> {
        nanargmax(&self.values)
    }

    /// Alias for [`argmax`](Self::argmax), matching `pd.Series.idxmax()`
    /// for positional indices.
    #[must_use]
    pub fn idxmax(&self) -> Option<usize> {
        self.argmax()
    }

    /// Alias for argmin, matching np.nanargmin.
    #[must_use]
    pub fn nanargmin(&self) -> Option<usize> {
        self.argmin()
    }

    /// Alias for argmax, matching np.nanargmax.
    #[must_use]
    pub fn nanargmax(&self) -> Option<usize> {
        self.argmax()
    }

    /// Whether non-missing values are non-decreasing.
    ///
    /// Matches `pd.Series.is_monotonic_increasing`. An empty column or
    /// a column with a single non-missing value returns true. Missing
    /// values are skipped when comparing neighbors.
    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        is_monotonic_in_direction(&self.values, true)
    }

    /// Whether non-missing values are non-increasing.
    ///
    /// Matches `pd.Series.is_monotonic_decreasing`.
    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        is_monotonic_in_direction(&self.values, false)
    }

    /// Combine two columns, taking `self` where present and `other`
    /// otherwise.
    ///
    /// Matches `pd.Series.combine_first(other)`. For each aligned
    /// position, the result is `self` when `self` is non-missing, else
    /// `other`. Length mismatch returns `LengthMismatch`. Result
    /// dtype follows `self`.
    pub fn combine_first(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| if a.is_missing() { b.clone() } else { a.clone() })
            .collect();
        Self::new(self.dtype, out)
    }

    /// Clip values below `lower`, leaving the upper bound free.
    ///
    /// Matches `pd.Series.clip(lower=...)`. Thin wrapper over
    /// `clip(Some(lower), None)` that preserves the shortcut reading
    /// convention of pandas.
    pub fn clip_lower(&self, lower: f64) -> Result<Self, ColumnError> {
        self.clip(Some(lower), None)
    }

    /// Clip values above `upper`, leaving the lower bound free.
    ///
    /// Matches `pd.Series.clip(upper=...)`.
    pub fn clip_upper(&self, upper: f64) -> Result<Self, ColumnError> {
        self.clip(None, Some(upper))
    }

    /// Remove duplicated values, keeping the first occurrence.
    ///
    /// Matches `pd.Series.drop_duplicates(keep='first')`.
    pub fn drop_duplicates(&self) -> Result<Self, ColumnError> {
        self.drop_duplicates_keep("first")
    }

    /// Remove duplicated values with explicit pandas `keep=` semantics.
    ///
    /// Supported policies are `"first"`, `"last"`, and `"false"` /
    /// `"none"` for pandas `keep=False`.
    pub fn drop_duplicates_keep(&self, keep: &str) -> Result<Self, ColumnError> {
        let dup = self.duplicated_keep(keep)?;
        let mut out = Vec::with_capacity(self.values.len());
        for (v, keep_flag) in self.values.iter().zip(dup.values.iter()) {
            if matches!(keep_flag, Scalar::Bool(false)) {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Element-wise comparison against `other`, emitting a 2-column
    /// report of differences.
    ///
    /// Matches `pd.Series.compare(other)` — returns two Columns
    /// `(self_values, other_values)` containing only the positions
    /// where the two differ. Missing entries compare equal to each
    /// other. Length-mismatched inputs return `LengthMismatch`.
    pub fn compare(&self, other: &Self) -> Result<(Self, Self), ColumnError> {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (a, b) in self.values.iter().zip(other.values.iter()) {
            let equal = match (a.is_missing(), b.is_missing()) {
                (true, true) => true,
                (true, false) | (false, true) => false,
                (false, false) => a.semantic_eq(b),
            };
            if !equal {
                left.push(a.clone());
                right.push(b.clone());
            }
        }
        Ok((Self::new(self.dtype, left)?, Self::new(other.dtype, right)?))
    }

    /// Apply a unary function over each value.
    ///
    /// Matches `pd.Series.map(func)`. Missing values are passed to the
    /// user function (callers can decide whether to propagate NaN);
    /// result dtype is inferred via `infer_dtype` over the outputs,
    /// falling back to `self.dtype` when inference fails (e.g. empty
    /// or all-null output).
    pub fn map<F>(&self, mut func: F) -> Result<Self, ColumnError>
    where
        F: FnMut(&Scalar) -> Scalar,
    {
        let out: Vec<Scalar> = self.values.iter().map(&mut func).collect();
        let target = infer_dtype(&out).unwrap_or(self.dtype);
        Self::new(target, out)
    }

    /// Linearly interpolate missing numeric values.
    ///
    /// Matches `pd.Series.interpolate(method='linear')` with the
    /// default `limit_direction='forward'`: interior missing runs are
    /// linearly interpolated, LEADING nulls stay null (forward fill
    /// cannot reach them), and TRAILING nulls are forward-filled with
    /// the last valid value (pandas does not extrapolate). Non-numeric
    /// columns return a type error. Result dtype is always Float64.
    pub fn interpolate_linear(&self) -> Result<Self, ColumnError> {
        let len = self.values.len();
        // Convert to f64 once; missing → None.
        let mut floats: Vec<Option<f64>> = Vec::with_capacity(len);
        for v in &self.values {
            if v.is_missing() {
                floats.push(None);
                continue;
            }
            match v.to_f64() {
                Ok(x) if !x.is_nan() => floats.push(Some(x)),
                Ok(_) => floats.push(None),
                Err(err) => return Err(ColumnError::Type(err)),
            }
        }

        // Walk interior gaps between the first and last non-null.
        let first = floats.iter().position(Option::is_some);
        let last = floats.iter().rposition(Option::is_some);
        if let (Some(start), Some(end)) = (first, last) {
            let mut i = start;
            while i < end {
                if floats[i].is_some() {
                    i += 1;
                    continue;
                }
                let gap_start = i;
                while i < end && floats[i].is_none() {
                    i += 1;
                }
                let before = floats[gap_start - 1].expect("anchor");
                let after = floats[i].expect("anchor");
                let span = (i - gap_start + 1) as f64;
                for (k, j) in (gap_start..i).enumerate() {
                    let step = (k + 1) as f64;
                    floats[j] = Some(before + (after - before) * (step / span));
                }
            }
            // Trailing nulls (after the last valid value) are forward-filled
            // with that value — pandas' default limit_direction='forward' carries
            // it forward rather than extrapolating. Leading nulls (before `start`)
            // are intentionally left null. (br-frankenpandas-8ic7c)
            let last_valid = floats[end].expect("last valid anchor");
            for slot in floats.iter_mut().skip(end + 1) {
                *slot = Some(last_valid);
            }
        }

        let out: Vec<Scalar> = floats
            .into_iter()
            .map(|opt| match opt {
                Some(x) => Scalar::Float64(x),
                None => Scalar::Null(NullKind::NaN),
            })
            .collect();
        Self::new(DType::Float64, out)
    }

    /// Alias for [`interpolate_linear`](Self::interpolate_linear), matching
    /// the default `pd.Series.interpolate()` behavior.
    pub fn interpolate(&self) -> Result<Self, ColumnError> {
        self.interpolate_linear()
    }

    /// Linear-interpolation quantile at `q ∈ [0.0, 1.0]`.
    ///
    /// Matches `pd.Series.quantile(q, interpolation='linear')`.
    /// Missing values are skipped (skipna=True). Returns
    /// `Null(NaN)` for empty columns or `q` outside `[0.0, 1.0]`.
    #[must_use]
    pub fn quantile(&self, q: f64) -> Scalar {
        nanquantile(&self.values, q)
    }

    /// Percentile of non-missing values.
    ///
    /// Matches np.percentile(). Takes percentile p in [0, 100].
    #[must_use]
    pub fn percentile(&self, p: f64) -> Scalar {
        self.quantile(p / 100.0)
    }

    /// Alias for quantile, matching np.nanquantile.
    #[must_use]
    pub fn nanquantile(&self, q: f64) -> Scalar {
        self.quantile(q)
    }

    /// Alias for percentile, matching np.nanpercentile.
    #[must_use]
    pub fn nanpercentile(&self, p: f64) -> Scalar {
        self.percentile(p)
    }

    /// Most frequent non-missing values, ascending-sorted.
    ///
    /// Matches `pd.Series.mode()`. Ties are all returned; missing
    /// values are ignored. For empty or all-missing columns the
    /// result is an empty same-dtype column.
    pub fn mode(&self) -> Result<Self, ColumnError> {
        // Counting-sort fast path: an all-valid, bounded-range Int64 column
        // tallies in O(n) via a dense direct-address histogram instead of the
        // SipHash `HashMap` below, and emits the winners with NO sort. Walking
        // the slots in ascending value order (slot s ↔ value `min + s`) yields
        // the most-frequent values already ascending — identical to the
        // `HashMap` path's `winners.sort_by(compare_scalars_na_last(.., true))`,
        // which orders Int64 by exact `i64::cmp`. `as_i64_slice` is `Some` only
        // for a fully-valid Int64 column, so there are no missing values to skip
        // (matching `key_of`'s `None`-on-missing), and an empty column makes
        // `i64_direct_address_range` return `None` → the `HashMap` path returns
        // the empty same-dtype column exactly as before.
        if let Some(data) = self.as_i64_slice()
            && let Some((min, range)) = i64_direct_address_range(data)
        {
            let mut count = vec![0i64; range];
            for &v in data {
                count[(v as i128 - min as i128) as usize] += 1;
            }
            let max_count = count.iter().copied().max().unwrap_or(0);
            let mut winners = Vec::new();
            for (s, &c) in count.iter().enumerate() {
                if c == max_count {
                    winners.push(Scalar::Int64(min + s as i64));
                }
            }
            return Self::new(self.dtype, winners);
        }

        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
            Datetime64(i64),
            Period(i64),
            Interval(u64, u64, IntervalClosed),
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
                Scalar::Datetime64(v) => Key::Datetime64(*v),
                Scalar::Period(v) => Key::Period(*v),
                Scalar::Interval(v) => {
                    let (left, right, closed) = interval_key(v);
                    Key::Interval(left, right, closed)
                }
                Scalar::Null(_) => return None,
            })
        }

        let mut counts: FxHashMap<Key<'_>, (usize, &Scalar)> = FxHashMap::default();
        for v in &self.values {
            if let Some(k) = key_of(v) {
                counts
                    .entry(k)
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, v));
            }
        }
        if counts.is_empty() {
            return Self::new(self.dtype, Vec::new());
        }
        let max_count = counts.values().map(|(c, _)| *c).max().unwrap_or(0);
        let mut winners: Vec<Scalar> = counts
            .values()
            .filter_map(|(c, v)| {
                if *c == max_count {
                    Some((*v).clone())
                } else {
                    None
                }
            })
            .collect();
        winners.sort_by(|a, b| compare_scalars_na_last(a, b, true));
        Self::new(self.dtype, winners)
    }

    /// Approximate memory footprint in bytes.
    ///
    /// Matches `pd.Series.memory_usage(deep=...)`. When `deep` is true
    /// and the column contains Utf8 values, each string's byte length
    /// is counted; otherwise a fixed per-element width is used
    /// (8 bytes for numeric/timedelta, 1 for Bool, pointer-sized for
    /// Utf8, 0 for Null). The ValidityMask is counted separately.
    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        let element_bytes = match self.dtype {
            DType::Bool => 1,
            DType::Int64 | DType::Float64 | DType::Timedelta64 => 8,
            DType::Utf8 => std::mem::size_of::<usize>(),
            _ => 0,
        };
        let base = element_bytes * self.values.len();
        let deep_extra = if deep && self.dtype == DType::Utf8 {
            self.values
                .iter()
                .map(|v| match v {
                    Scalar::Utf8(s) => s.len(),
                    _ => 0,
                })
                .sum::<usize>()
        } else {
            0
        };
        // One bit per element, rounded up to whole bytes.
        let validity_bytes = self.values.len().div_ceil(8);
        base + deep_extra + validity_bytes
    }

    /// Approximate value-buffer footprint, matching `pd.Series.nbytes`.
    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.memory_usage(false)
    }

    /// Return the size in bytes of a single element.
    ///
    /// Matches `pd.Series.dtype.itemsize`. Returns 8 for Int64/Float64/Datetime64/Timedelta64,
    /// 1 for Bool, and an estimate for variable-length types.
    #[must_use]
    pub fn itemsize(&self) -> usize {
        match self.dtype() {
            DType::Bool | DType::BoolNullable => 1,
            DType::Int64
            | DType::Int64Nullable
            | DType::Float64
            | DType::Datetime64
            | DType::Timedelta64
            | DType::Period => 8,
            DType::Utf8 => {
                if self.values.is_empty() {
                    0
                } else {
                    self.memory_usage(true) / self.values.len()
                }
            }
            DType::Null | DType::Categorical | DType::Interval | DType::Sparse => 8,
        }
    }

    /// Element-wise equality into a Bool column.
    ///
    /// Matches `pd.Series.eq(other)`. Both inputs must have the same
    /// length. Missing-on-either-side positions produce `false`
    /// (pandas semantics: NaN != anything, including NaN).
    pub fn equals(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| {
                if a.is_missing() || b.is_missing() {
                    Scalar::Bool(false)
                } else {
                    Scalar::Bool(a.semantic_eq(b))
                }
            })
            .collect();
        Self::new(DType::Bool, out)
    }

    /// Scalar dot product against another column.
    ///
    /// Matches `pd.Series.dot(other)` for numeric columns. Missing
    /// entries on either side contribute zero (consistent with
    /// fp-types nan-aware sums). Length mismatch returns
    /// `LengthMismatch`; non-numeric inputs return a type error on
    /// the first offending value.
    pub fn dot(&self, other: &Self) -> Result<f64, ColumnError> {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let mut sum = 0.0_f64;
        for (a, b) in self.values.iter().zip(other.values.iter()) {
            if a.is_missing() || b.is_missing() {
                continue;
            }
            let av = a.to_f64().map_err(ColumnError::Type)?;
            let bv = b.to_f64().map_err(ColumnError::Type)?;
            if av.is_nan() || bv.is_nan() {
                continue;
            }
            sum += av * bv;
        }
        Ok(sum)
    }

    /// Discrete linear convolution of two 1D sequences.
    ///
    /// Matches np.convolve(a, v, mode). Modes:
    /// - "full": output length = len(a) + len(v) - 1
    /// - "same": output length = max(len(a), len(v))
    /// - "valid": output length = max(len(a), len(v)) - min(len(a), len(v)) + 1
    pub fn convolve(&self, kernel: &Self, mode: &str) -> Result<Self, ColumnError> {
        let a: Vec<f64> = self
            .values
            .iter()
            .map(|v| v.to_f64().unwrap_or(0.0))
            .collect();
        let v: Vec<f64> = kernel
            .values
            .iter()
            .map(|v| v.to_f64().unwrap_or(0.0))
            .collect();

        if a.is_empty() || v.is_empty() {
            return Self::new(DType::Float64, vec![]);
        }

        let full_len = a.len() + v.len() - 1;
        let mut full: Vec<f64> = vec![0.0; full_len];

        for (i, &ai) in a.iter().enumerate() {
            for (j, &vj) in v.iter().enumerate() {
                full[i + j] += ai * vj;
            }
        }

        let out: Vec<f64> = match mode {
            "full" => full,
            "same" => {
                let target_len = a.len().max(v.len());
                let start = (full_len - target_len) / 2;
                full[start..start + target_len].to_vec()
            }
            "valid" => {
                let min_len = a.len().min(v.len());
                let valid_len = a.len().max(v.len()) - min_len + 1;
                let start = min_len - 1;
                full[start..start + valid_len].to_vec()
            }
            _ => {
                return Err(ColumnError::Type(TypeError::NonNumericValue {
                    value: format!("invalid mode '{mode}', expected 'full', 'same', or 'valid'"),
                    dtype: self.dtype,
                }));
            }
        };

        let scalars: Vec<Scalar> = out.into_iter().map(Scalar::Float64).collect();
        Self::new(DType::Float64, scalars)
    }

    /// Cross-correlation of two 1D sequences.
    ///
    /// Matches np.correlate(a, v, mode). This is convolve(a, reverse(v), mode).
    pub fn correlate(&self, other: &Self, mode: &str) -> Result<Self, ColumnError> {
        let reversed = other.reverse()?;
        self.convolve(&reversed, mode)
    }

    /// Fill missing values in `self` with aligned values from `other`.
    ///
    /// Matches `pd.Series.fillna(other)` when `other` is a Series. Only
    /// positions missing in `self` are replaced. Length mismatch
    /// returns `LengthMismatch`. Values from `other` are cast into
    /// `self.dtype`.
    pub fn fillna_with_column(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: other.values.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(v, o)| {
                if v.is_missing() {
                    cast_scalar(o, self.dtype)
                } else {
                    Ok(v.clone())
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(ColumnError::Type)?;
        Self::new(self.dtype, out)
    }

    /// Element-wise quotient and remainder against `divisor`.
    ///
    /// Matches `pd.Series.divmod(other)`: returns
    /// `(self // other, self % other)`. Division by zero, missing
    /// inputs, or non-numeric values yield `Null(NaN)` in both
    /// outputs at that position. Length mismatch returns
    /// `LengthMismatch`. Both result columns are Float64.
    pub fn divmod(&self, divisor: &Self) -> Result<(Self, Self), ColumnError> {
        if self.values.len() != divisor.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: divisor.values.len(),
            });
        }
        let mut quotient = Vec::with_capacity(self.values.len());
        let mut remainder = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(divisor.values.iter()) {
            if a.is_missing() || b.is_missing() {
                quotient.push(Scalar::Null(NullKind::NaN));
                remainder.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            let num = match a.to_f64() {
                Ok(x) if !x.is_nan() => x,
                _ => {
                    quotient.push(Scalar::Null(NullKind::NaN));
                    remainder.push(Scalar::Null(NullKind::NaN));
                    continue;
                }
            };
            let den = match b.to_f64() {
                Ok(x) if !x.is_nan() => x,
                _ => {
                    quotient.push(Scalar::Null(NullKind::NaN));
                    remainder.push(Scalar::Null(NullKind::NaN));
                    continue;
                }
            };
            if den == 0.0 {
                quotient.push(Scalar::Null(NullKind::NaN));
                remainder.push(Scalar::Null(NullKind::NaN));
                continue;
            }
            // Floor-division and Python-style modulo, including pandas' signed
            // zero and infinity behavior.
            let q = python_floor_div_f64(num, den);
            let r = python_mod_f64(num, den);
            quotient.push(Scalar::Float64(q));
            remainder.push(Scalar::Float64(r));
        }
        Ok((
            Self::new(DType::Float64, quotient)?,
            Self::new(DType::Float64, remainder)?,
        ))
    }

    /// Keep values where `cond` is true; replace false positions with
    /// values from an `other` Column (element-wise).
    ///
    /// Matches `pd.Series.where(cond, other)` when `other` is a Series
    /// aligned by position. All three inputs must have the same
    /// length. Cond must be Bool. Missing cond entries propagate as
    /// Null(NaN). The result dtype is `self.dtype`; if `other`'s dtype
    /// differs, values coming from `other` are cast via `cast_scalar`.
    pub fn where_cond_series(&self, cond: &Self, other: &Self) -> Result<Self, ColumnError> {
        if cond.dtype != DType::Bool {
            return Err(ColumnError::InvalidMaskType { dtype: cond.dtype });
        }
        if self.values.len() != cond.values.len() || self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: cond.values.len().max(other.values.len()),
            });
        }
        // Typed branchless select: all-valid Bool cond and same-typed all-valid
        // numeric self/other compute the result straight over the contiguous
        // buffers, with no per-element Scalar dispatch/clone or output Vec<Scalar>.
        // Bit-identical — with an all-valid cond there is no missing branch, and
        // for matching dtypes cast_scalar(o, self.dtype) is the identity, so each
        // slot is cond[i] ? self[i] : other[i]. Mixed/nullable inputs fall back.
        if let Some(cb) = cond.as_bool_slice() {
            if let (Some(s), Some(o)) = (self.as_f64_slice(), other.as_f64_slice()) {
                let out: Vec<f64> = (0..s.len())
                    .map(|i| if cb[i] { s[i] } else { o[i] })
                    .collect();
                return Ok(Self::from_f64_values(out));
            }
            if let (Some(s), Some(o)) = (self.as_i64_slice(), other.as_i64_slice()) {
                let out: Vec<i64> = (0..s.len())
                    .map(|i| if cb[i] { s[i] } else { o[i] })
                    .collect();
                return Ok(Self::from_i64_values(out));
            }
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(cond.values.iter().zip(other.values.iter()))
            .map(|(v, (c, o))| match c {
                Scalar::Bool(true) => Ok(v.clone()),
                Scalar::Bool(false) => cast_scalar(o, self.dtype),
                _ => Ok(Scalar::Null(NullKind::NaN)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(ColumnError::Type)?;
        Self::new(self.dtype, out)
    }

    /// Replace values where `cond` is true with values from `other`
    /// (element-wise); otherwise keep.
    ///
    /// Matches `pd.Series.mask(cond, other)` when `other` is a Series.
    pub fn mask_series(&self, cond: &Self, other: &Self) -> Result<Self, ColumnError> {
        if cond.dtype != DType::Bool {
            return Err(ColumnError::InvalidMaskType { dtype: cond.dtype });
        }
        if self.values.len() != cond.values.len() || self.values.len() != other.values.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.values.len(),
                right: cond.values.len().max(other.values.len()),
            });
        }
        // Typed branchless select (inverse of where_cond_series): cond true picks
        // other, false picks self. Same isomorphism argument.
        if let Some(cb) = cond.as_bool_slice() {
            if let (Some(s), Some(o)) = (self.as_f64_slice(), other.as_f64_slice()) {
                let out: Vec<f64> = (0..s.len())
                    .map(|i| if cb[i] { o[i] } else { s[i] })
                    .collect();
                return Ok(Self::from_f64_values(out));
            }
            if let (Some(s), Some(o)) = (self.as_i64_slice(), other.as_i64_slice()) {
                let out: Vec<i64> = (0..s.len())
                    .map(|i| if cb[i] { o[i] } else { s[i] })
                    .collect();
                return Ok(Self::from_i64_values(out));
            }
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .zip(cond.values.iter().zip(other.values.iter()))
            .map(|(v, (c, o))| match c {
                Scalar::Bool(true) => cast_scalar(o, self.dtype),
                Scalar::Bool(false) => Ok(v.clone()),
                _ => Ok(Scalar::Null(NullKind::NaN)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(ColumnError::Type)?;
        Self::new(self.dtype, out)
    }

    /// Pairwise value substitution.
    ///
    /// Matches `pd.Series.replace(to_replace, value)` when both
    /// arguments are scalar lists of equal length. For each value in
    /// the column, the first (to_replace, replacement) pair that
    /// matches via `Scalar::semantic_eq` is applied. Missing inputs
    /// can be replaced by listing `Scalar::Null(...)` in `to_replace`.
    /// Length mismatch between `to_replace` and `values` returns
    /// `ColumnError::LengthMismatch`.
    pub fn replace_values(
        &self,
        to_replace: &[Scalar],
        replacement: &[Scalar],
    ) -> Result<Self, ColumnError> {
        if to_replace.len() != replacement.len() {
            return Err(ColumnError::LengthMismatch {
                left: to_replace.len(),
                right: replacement.len(),
            });
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| {
                for (target, replacement_val) in to_replace.iter().zip(replacement.iter()) {
                    // Treat all missing variants as matching to_replace = Null.
                    let matches = if target.is_missing() && v.is_missing() {
                        true
                    } else if target.is_missing() || v.is_missing() {
                        false
                    } else {
                        v.semantic_eq(target)
                    };
                    if matches {
                        return replacement_val.clone();
                    }
                }
                v.clone()
            })
            .collect();
        let inferred = infer_dtype(&out).unwrap_or(self.dtype);
        Self::new(inferred, out)
    }

    /// Alias for [`replace_values`](Self::replace_values), matching
    /// `pd.Series.replace(to_replace, value)` for equal-length scalar
    /// list replacements.
    pub fn replace(
        &self,
        to_replace: &[Scalar],
        replacement: &[Scalar],
    ) -> Result<Self, ColumnError> {
        self.replace_values(to_replace, replacement)
    }

    /// Positions where the value is truthy and non-missing.
    ///
    /// Matches `np.nonzero` / `pd.Series.to_numpy().nonzero()` style
    /// behavior. Useful for turning a Bool mask column into explicit
    /// index positions. Non-missing zero-like values (Int64 0,
    /// Float64 0.0, Bool false, empty Utf8) are excluded.
    #[must_use]
    pub fn nonzero(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, v) in self.values.iter().enumerate() {
            if v.is_missing() {
                continue;
            }
            let truthy = match v {
                Scalar::Bool(b) => *b,
                Scalar::Int64(x) => *x != 0,
                Scalar::Float64(x) => *x != 0.0 && !x.is_nan(),
                Scalar::Utf8(s) => !s.is_empty(),
                Scalar::Timedelta64(x) => *x != 0,
                Scalar::Datetime64(x) => *x != Timestamp::NAT,
                Scalar::Period(x) => *x != i64::MIN,
                Scalar::Interval(_) => true,
                Scalar::Null(_) => false,
            };
            if truthy {
                out.push(i);
            }
        }
        out
    }

    /// Count number of non-zero elements.
    ///
    /// Matches np.count_nonzero().
    #[must_use]
    pub fn count_nonzero(&self) -> usize {
        self.nonzero().len()
    }

    /// Indices of non-zero elements as a column.
    ///
    /// Matches np.flatnonzero(). Returns Int64 column of indices.
    pub fn flatnonzero(&self) -> Result<Self, ColumnError> {
        let indices: Vec<Scalar> = self
            .nonzero()
            .into_iter()
            .map(|i| Scalar::Int64(i as i64))
            .collect();
        Self::new(DType::Int64, indices)
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
        // Typed branchless select against a scalar `other`. For Float64 self,
        // Column::new coerces other to Float64, so other.to_f64() (non-missing)
        // is the exact false-branch value; for Int64 self only an Int64 other
        // stays lossless, so that path requires Scalar::Int64. All-valid cond =>
        // no missing branch. Bit-identical; other cases fall back.
        if !other.is_missing()
            && let Some(cb) = cond.as_bool_slice()
        {
            if let Some(s) = self.as_f64_slice()
                && let Ok(o) = other.to_f64()
            {
                let out: Vec<f64> = (0..s.len()).map(|i| if cb[i] { s[i] } else { o }).collect();
                return Ok(Self::from_f64_values(out));
            }
            if let Some(s) = self.as_i64_slice()
                && let Scalar::Int64(o) = other
            {
                let o = *o;
                let out: Vec<i64> = (0..s.len()).map(|i| if cb[i] { s[i] } else { o }).collect();
                return Ok(Self::from_i64_values(out));
            }
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

    /// Alias for [`where_cond`](Self::where_cond), matching
    /// `pd.Series.where(cond, other)` for scalar `other` values.
    pub fn r#where(&self, cond: &Self, other: &Scalar) -> Result<Self, ColumnError> {
        self.where_cond(cond, other)
    }

    /// Rank the values of the column.
    ///
    /// Matches `pd.Series.rank(method=..., ascending=..., na_option='keep')`.
    /// Supported `method` values are `"average"` (pandas default,
    /// ties → average of tied ranks), `"min"` (ties → smallest tied
    /// rank), `"max"` (ties → largest tied rank), `"first"` (ties →
    /// appearance order), and `"dense"` (ties → consecutive integers
    /// with no gaps between distinct groups).
    ///
    /// Missing input positions stay missing in the output (matching
    /// pandas `na_option='keep'`). The result dtype is always Float64
    /// so `"average"` can produce non-integer ranks.
    pub fn rank(&self, method: &str, ascending: bool) -> Result<Self, ColumnError> {
        let valid_method = matches!(method, "average" | "min" | "max" | "first" | "dense");
        if !valid_method {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: method.to_string(),
                dtype: self.dtype,
            }));
        }

        let len = self.values.len();

        // Counting-sort fast path: an all-valid, bounded-range Int64 column ranks
        // in O(n) via a value histogram + prefix sums instead of the O(n log n)
        // sort below. Bit-identical: compare_scalars_na_last compares Int64 with
        // exact `i64::cmp` (no f64 coercion), so grouping by exact value matches
        // the sort's tie groups; the stable sort's within-tie order ("first")
        // is the original order, reproduced by a per-value occurrence counter
        // walked in original order. The rank f64 expressions mirror the sort
        // path's exactly (start_rank/end_rank), so every method/direction agrees.
        if let Some(data) = self.as_i64_slice()
            && let Some((min, range)) = i64_direct_address_range(data)
        {
            let total = data.len() as i64;
            let mut count = vec![0i64; range];
            for &v in data {
                count[(v as i128 - min as i128) as usize] += 1;
            }
            // c_less[s] = # values < value-at-slot-s; dense_asc[s] = 1-based
            // ascending ordinal among present distinct values.
            let mut c_less = vec![0i64; range];
            let mut dense_asc = vec![0i64; range];
            let mut acc = 0i64;
            let mut ord = 0i64;
            for s in 0..range {
                c_less[s] = acc;
                if count[s] > 0 {
                    ord += 1;
                    dense_asc[s] = ord;
                }
                acc += count[s];
            }
            let n_distinct = ord;
            let mut occ = vec![0i64; range];
            let mut ranks = vec![Scalar::Null(NullKind::NaN); len];
            for (i, &v) in data.iter().enumerate() {
                let s = (v as i128 - min as i128) as usize;
                let c = count[s];
                // `before` = sorted-position offset of this value's tie group
                // (values that sort before it): `c_less` ascending, the
                // complement `total - c_less - c` descending.
                let before = if ascending {
                    c_less[s]
                } else {
                    total - c_less[s] - c
                };
                let start_rank = before as f64 + 1.0;
                let end_rank = (before + c) as f64;
                let value = match method {
                    "average" => (start_rank + end_rank) / 2.0,
                    "min" => start_rank,
                    "max" => end_rank,
                    "first" => {
                        let k = occ[s];
                        occ[s] += 1;
                        (before + k) as f64 + 1.0
                    }
                    "dense" => {
                        let d = if ascending {
                            dense_asc[s]
                        } else {
                            n_distinct - dense_asc[s] + 1
                        };
                        d as f64
                    }
                    _ => unreachable!(),
                };
                ranks[i] = Scalar::Float64(value);
            }
            return Self::new(DType::Float64, ranks);
        }

        // Radix fast path: an all-valid, NaN-free Float64 column ranks in O(n)
        // via the stable LSD radix permutation (the same one `sort_values`/
        // `argsort` use) instead of the O(n log n) `Scalar` comparison sort
        // below. Bit-identical: `f64_radix_key` normalizes `-0.0` to `0.0`
        // (exactly as `compare_scalars_na_last`'s `partial_cmp` treats `-0.0 ==
        // 0.0`), `radix_argsort_u64` is stable (ties keep original order, like
        // the stable `sort_by`), and tie groups are detected with f64 `==`
        // (which is `Equal` under `partial_cmp` for the same finite values). A
        // NaN would diverge — `partial_cmp(NaN, _) -> Equal` collapses ties in
        // the comparator path while the radix key sorts NaN to one end, and a
        // NaN value is also `is_missing()` so the comparator path drops it — so
        // any NaN routes to the unchanged comparator fallback. All-valid +
        // NaN-free means every row is ranked (no nulls), so the output is built
        // typed via `from_f64_values`.
        if let Some(data) = self.as_f64_slice()
            && !data.iter().any(|x| x.is_nan())
        {
            let perm = self
                .typed_radix_perm(ascending)
                .expect("f64 slice yields radix perm");
            let n = perm.len();
            let mut ranks = vec![0.0_f64; len];
            let mut cursor = 0usize;
            let mut dense_rank = 0f64;
            while cursor < n {
                let mut end = cursor + 1;
                while end < n && data[perm[end]] == data[perm[cursor]] {
                    end += 1;
                }
                let start_rank = cursor as f64 + 1.0;
                let end_rank = end as f64;
                dense_rank += 1.0;
                #[allow(clippy::needless_range_loop)] // group_idx is also the "first" rank value
                for group_idx in cursor..end {
                    let original = perm[group_idx];
                    ranks[original] = match method {
                        "average" => (start_rank + end_rank) / 2.0,
                        "min" => start_rank,
                        "max" => end_rank,
                        "first" => group_idx as f64 + 1.0,
                        "dense" => dense_rank,
                        _ => unreachable!(),
                    };
                }
                cursor = end;
            }
            return Ok(Self::from_f64_values(ranks));
        }

        let mut non_missing: Vec<(usize, &Scalar)> = Vec::with_capacity(len);
        for (i, v) in self.values.iter().enumerate() {
            if !v.is_missing() {
                non_missing.push((i, v));
            }
        }
        non_missing.sort_by(|a, b| compare_scalars_na_last(a.1, b.1, ascending));

        let mut ranks = vec![Scalar::Null(NullKind::NaN); len];
        let n = non_missing.len();
        let mut cursor = 0usize;
        let mut dense_rank = 0f64;
        while cursor < n {
            let mut end = cursor + 1;
            while end < n {
                let same =
                    compare_scalars_na_last(non_missing[cursor].1, non_missing[end].1, ascending)
                        .is_eq();
                if !same {
                    break;
                }
                end += 1;
            }
            let start_rank = cursor as f64 + 1.0;
            let end_rank = end as f64;
            dense_rank += 1.0;
            for (group_idx, entry) in non_missing.iter().enumerate().take(end).skip(cursor) {
                let original = entry.0;
                let value = match method {
                    "average" => (start_rank + end_rank) / 2.0,
                    "min" => start_rank,
                    "max" => end_rank,
                    "first" => group_idx as f64 + 1.0,
                    "dense" => dense_rank,
                    _ => unreachable!(),
                };
                ranks[original] = Scalar::Float64(value);
            }
            cursor = end;
        }
        Self::new(DType::Float64, ranks)
    }

    /// Position where `needle` would be inserted to preserve sort order.
    ///
    /// Matches `pd.Series.searchsorted(value, side)`. `side` is
    /// `"left"` (first valid insertion position) or `"right"` (last).
    /// The column is assumed sorted ascending with missing values at
    /// the end (consistent with `sort_values(true)`). Missing
    /// `needle` is rejected with a type error.
    pub fn searchsorted(&self, needle: &Scalar, side: &str) -> Result<usize, ColumnError> {
        self.searchsorted_position(needle, side, None)
    }

    /// Position where `needle` would be inserted using an explicit sorter.
    ///
    /// Matches `pd.Series.searchsorted(value, side, sorter=...)` where
    /// `sorter` is a permutation that sorts the column ascending.
    pub fn searchsorted_with_sorter(
        &self,
        needle: &Scalar,
        side: &str,
        sorter: &[usize],
    ) -> Result<usize, ColumnError> {
        self.searchsorted_position(needle, side, Some(sorter))
    }

    /// Positions where `needles` would be inserted to preserve sort order.
    ///
    /// Matches `pd.Series.searchsorted(values, side)` for array-like
    /// inputs. Returns an `Int64` column of insertion positions.
    /// Missing needles are rejected with the same error as the scalar path.
    pub fn searchsorted_values(&self, needles: &[Scalar], side: &str) -> Result<Self, ColumnError> {
        let positions: Vec<Scalar> = needles
            .iter()
            .map(|needle| self.searchsorted_position(needle, side, None))
            .map(|result| result.map(|position| Scalar::Int64(position as i64)))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(DType::Int64, positions)
    }

    /// Positions where `needles` would be inserted using an explicit sorter.
    ///
    /// Matches `pd.Series.searchsorted(values, side, sorter=...)` for
    /// array-like inputs. Returns an `Int64` column of insertion positions.
    pub fn searchsorted_values_with_sorter(
        &self,
        needles: &[Scalar],
        side: &str,
        sorter: &[usize],
    ) -> Result<Self, ColumnError> {
        let positions: Vec<Scalar> = needles
            .iter()
            .map(|needle| self.searchsorted_position(needle, side, Some(sorter)))
            .map(|result| result.map(|position| Scalar::Int64(position as i64)))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(DType::Int64, positions)
    }

    fn searchsorted_position(
        &self,
        needle: &Scalar,
        side: &str,
        sorter: Option<&[usize]>,
    ) -> Result<usize, ColumnError> {
        if side != "left" && side != "right" {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: side.to_string(),
                dtype: self.dtype,
            }));
        }
        if needle.is_missing() {
            return Err(ColumnError::Type(TypeError::ValueIsMissing {
                kind: NullKind::NaN,
            }));
        }

        let sorter = self.validate_searchsorted_sorter(sorter)?;
        let len = sorter.map_or(self.values.len(), <[usize]>::len);
        let mut lo = 0usize;
        let mut hi = len;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_idx = sorter.map_or(mid, |indices| indices[mid]);
            let mid_val = &self.values[mid_idx];
            // Values that are "missing" sort to the end; treat needle as
            // less than any missing slot.
            let ord = if mid_val.is_missing() {
                std::cmp::Ordering::Greater
            } else {
                compare_scalars_na_last(mid_val, needle, true)
            };
            use std::cmp::Ordering;
            let go_right = match (ord, side) {
                (Ordering::Less, _) => true,
                (Ordering::Equal, "left") => false,
                (Ordering::Equal, "right") => true,
                (Ordering::Greater, _) => false,
                _ => unreachable!(),
            };
            if go_right {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    fn validate_searchsorted_sorter<'a>(
        &self,
        sorter: Option<&'a [usize]>,
    ) -> Result<Option<&'a [usize]>, ColumnError> {
        let Some(sorter) = sorter else {
            return Ok(None);
        };
        let len = self.values.len();
        if sorter.len() != len {
            return Err(ColumnError::LengthMismatch {
                left: len,
                right: sorter.len(),
            });
        }
        let mut seen = vec![false; len];
        for &idx in sorter {
            if idx >= len {
                return Err(ColumnError::InvalidSorter {
                    len,
                    reason: format!("index {idx} out of bounds"),
                });
            }
            if std::mem::replace(&mut seen[idx], true) {
                return Err(ColumnError::InvalidSorter {
                    len,
                    reason: format!("index {idx} appears more than once"),
                });
            }
        }
        Ok(Some(sorter))
    }

    /// Return bin indices for values given sorted bin edges.
    ///
    /// Matches np.digitize(). Returns indices such that bins[i-1] <= x < bins[i].
    pub fn digitize(&self, bins: &Self, right: bool) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Int64(0));
                continue;
            }
            let vf = v.to_f64().map_err(ColumnError::Type)?;
            let side = if right { "right" } else { "left" };
            let pos = bins.searchsorted(&Scalar::Float64(vf), side)?;
            out.push(Scalar::Int64(pos as i64));
        }
        Self::new(DType::Int64, out)
    }

    /// Count occurrences of each non-negative integer value.
    ///
    /// Matches np.bincount(). Returns array where output[i] = count of i in input.
    /// Requires non-negative Int64 values.
    pub fn bincount(&self, minlength: usize) -> Result<Self, ColumnError> {
        let mut max_val = 0i64;
        for v in &self.values {
            if v.is_missing() {
                continue;
            }
            match v {
                Scalar::Int64(x) if *x >= 0 => {
                    if *x > max_val {
                        max_val = *x;
                    }
                }
                Scalar::Int64(x) => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("negative value {x}"),
                        dtype: self.dtype,
                    }));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        let len = (max_val as usize + 1).max(minlength);
        let mut counts = vec![0i64; len];
        for v in &self.values {
            if v.is_missing() {
                continue;
            }
            if let Scalar::Int64(x) = v {
                counts[*x as usize] += 1;
            }
        }
        let out: Vec<Scalar> = counts.into_iter().map(Scalar::Int64).collect();
        Self::new(DType::Int64, out)
    }

    /// Compute histogram using provided bin edges.
    ///
    /// Matches np.histogram(a, bins=edges). Returns counts for each bin.
    /// Bins are [edges[i], edges[i+1]) except the last which is [edges[n-1], edges[n]].
    pub fn histogram(&self, bin_edges: &[f64]) -> Result<Self, ColumnError> {
        if bin_edges.len() < 2 {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: "histogram requires at least 2 bin edges".to_owned(),
                dtype: self.dtype,
            }));
        }
        let n_bins = bin_edges.len() - 1;
        let mut counts = vec![0i64; n_bins];

        // Fast path: strictly-increasing edges admit an O(log n_bins) binary
        // search for each value's bin instead of the O(n_bins) linear scan
        // below — O(N·log B) vs O(N·B). Bins are right-open [e_i, e_{i+1}) with
        // an inclusive final right edge, so for x in [e_0, e_last] the bin is
        // `partition_point(|e| e <= x) - 1` clamped to the last bin (so a value
        // exactly at e_last lands in bin n_bins-1); values outside [e_0, e_last]
        // are dropped — bit-identical to the linear scan. Non-strict (duplicate)
        // edges create zero-width bins where the two scans can disagree, so they
        // take the original linear path.
        let strict = bin_edges.windows(2).all(|w| w[0] < w[1]);

        for v in &self.values {
            if v.is_missing() {
                continue;
            }
            let x = match v.to_f64() {
                Ok(f) if f.is_finite() => f,
                _ => continue,
            };
            if strict {
                if x < bin_edges[0] || x > bin_edges[n_bins] {
                    continue;
                }
                let bin = (bin_edges.partition_point(|&e| e <= x) - 1).min(n_bins - 1);
                counts[bin] += 1;
                continue;
            }
            // Linear scan (non-strict edges fallback).
            for i in 0..n_bins {
                let in_bin = if i == n_bins - 1 {
                    // Last bin is inclusive on right
                    x >= bin_edges[i] && x <= bin_edges[i + 1]
                } else {
                    x >= bin_edges[i] && x < bin_edges[i + 1]
                };
                if in_bin {
                    counts[i] += 1;
                    break;
                }
            }
            // Values outside all bins are not counted (matches numpy)
        }

        let out: Vec<Scalar> = counts.into_iter().map(Scalar::Int64).collect();
        Self::new(DType::Int64, out)
    }

    /// Compute histogram with auto-generated bins.
    ///
    /// Matches np.histogram(a, bins=n_bins). Returns (counts, bin_edges).
    /// Bins are evenly spaced between min and max of the data.
    pub fn histogram_auto(&self, n_bins: usize) -> Result<(Self, Vec<f64>), ColumnError> {
        if n_bins == 0 {
            return Err(ColumnError::Type(TypeError::NonNumericValue {
                value: "histogram requires at least 1 bin".to_owned(),
                dtype: self.dtype,
            }));
        }

        // Find min and max
        let mut min_val = f64::INFINITY;
        let mut max_val = f64::NEG_INFINITY;
        for v in &self.values {
            if v.is_missing() {
                continue;
            }
            if let Ok(x) = v.to_f64()
                && x.is_finite()
            {
                min_val = min_val.min(x);
                max_val = max_val.max(x);
            }
        }

        if !min_val.is_finite() || !max_val.is_finite() || min_val > max_val {
            // No valid data
            let counts: Vec<Scalar> = vec![Scalar::Int64(0); n_bins];
            let edges = vec![0.0; n_bins + 1];
            return Ok((Self::new(DType::Int64, counts)?, edges));
        }

        // Generate bin edges
        let range = max_val - min_val;
        let (adj_min, adj_max) = if range == 0.0 {
            // All values are the same - numpy extends by 0.5 on each side
            (min_val - 0.5, max_val + 0.5)
        } else {
            (min_val, max_val)
        };
        let adj_range = adj_max - adj_min;
        let step = adj_range / n_bins as f64;
        let bin_edges: Vec<f64> = (0..=n_bins).map(|i| adj_min + step * i as f64).collect();

        let counts = self.histogram(&bin_edges)?;
        Ok((counts, bin_edges))
    }

    /// Cast the column to a target dtype.
    ///
    /// Matches `pd.Series.astype(dtype)`. Each value is routed through
    /// `fp_types::cast_scalar`, so coercion rules (Int64↔Float64,
    /// Bool→Int64, Utf8 parsing, etc.) match the existing cast table.
    /// Cast failures on any element return `ColumnError::Type` wrapping
    /// the underlying TypeError so the caller can attribute the
    /// failing conversion. Missing values pass through as the
    /// target dtype's canonical missing representation.
    pub fn astype(&self, target: DType) -> Result<Self, ColumnError> {
        if self.dtype == target {
            return Ok(self.clone());
        }
        // Typed fast paths for the two ubiquitous all-valid numeric casts:
        //   Int64 -> Float64 is exactly `x as f64` (the cast_scalar branch), and
        //   Float64 -> Int64 truncates a finite in-range float toward zero via
        //   `v as i64`. NaN floats mark the column invalid so as_f64_slice
        //   declines; an out-of-range float makes cast_scalar error, so we only
        //   take the typed path when every value is in range (otherwise the
        //   Scalar path below reproduces that exact error). Bit-identical.
        if target == DType::Float64
            && let Some(data) = self.as_i64_slice()
        {
            let out: Vec<f64> = data.iter().map(|&x| x as f64).collect();
            return Ok(Self::from_f64_values(out));
        }
        if target == DType::Int64
            && let Some(data) = self.as_f64_slice()
            && data
                .iter()
                .all(|&v| v >= i64::MIN as f64 && v < 9_223_372_036_854_775_808.0)
        {
            let out: Vec<i64> = data.iter().map(|&v| v as i64).collect();
            return Ok(Self::from_i64_values(out));
        }
        let out: Vec<Scalar> = self
            .values
            .iter()
            .map(|v| cast_scalar(v, target))
            .collect::<Result<Vec<_>, _>>()
            .map_err(ColumnError::Type)?;
        Self::new(target, out)
    }

    /// Return the `n` smallest values with explicit keep policy for
    /// ties.
    ///
    /// Matches `pd.Series.nsmallest(n, keep=...)`:
    /// - `"first"`: take the first `n` rows in ascending order, break
    ///   ties by original position (stable).
    /// - `"last"`: on ties, prefer later-appearing rows.
    /// - `"all"`: include every row tied with the `n`-th smallest, so
    ///   the returned column can exceed `n`.
    pub fn nsmallest_keep(&self, n: usize, keep: &str) -> Result<Self, ColumnError> {
        nkeep_impl(self, n, keep, true)
    }

    /// Return the `n` largest values with explicit keep policy for
    /// ties.
    ///
    /// Matches `pd.Series.nlargest(n, keep=...)` — see `nsmallest_keep`
    /// for the shared semantics.
    pub fn nlargest_keep(&self, n: usize, keep: &str) -> Result<Self, ColumnError> {
        nkeep_impl(self, n, keep, false)
    }

    /// Return the `n` largest values.
    ///
    /// Matches `pd.Series.nlargest(n)` with `keep='first'` — ties are
    /// broken by first-seen order via a stable descending sort.
    /// Missing values are placed at the end of the sorted view and
    /// therefore excluded from the top-n when `n` fits within the
    /// non-missing count. `n > len()` clamps to the full column.
    pub fn nlargest(&self, n: usize) -> Result<Self, ColumnError> {
        let sorted = self.sort_values(false)?;
        let take = n.min(sorted.values.len());
        let values: Vec<Scalar> = sorted.values[..take].to_vec();
        Self::new(self.dtype, values)
    }

    /// Return the `n` smallest values.
    ///
    /// Matches `pd.Series.nsmallest(n)` with `keep='first'`.
    pub fn nsmallest(&self, n: usize) -> Result<Self, ColumnError> {
        let sorted = self.sort_values(true)?;
        let take = n.min(sorted.values.len());
        let values: Vec<Scalar> = sorted.values[..take].to_vec();
        Self::new(self.dtype, values)
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
        // Typed branchless select (inverse of where_cond scalar): cond true picks
        // the scalar other, false picks self. Same isomorphism argument.
        if !other.is_missing()
            && let Some(cb) = cond.as_bool_slice()
        {
            if let Some(s) = self.as_f64_slice()
                && let Ok(o) = other.to_f64()
            {
                let out: Vec<f64> = (0..s.len()).map(|i| if cb[i] { o } else { s[i] }).collect();
                return Ok(Self::from_f64_values(out));
            }
            if let Some(s) = self.as_i64_slice()
                && let Scalar::Int64(o) = other
            {
                let o = *o;
                let out: Vec<i64> = (0..s.len()).map(|i| if cb[i] { o } else { s[i] }).collect();
                return Ok(Self::from_i64_values(out));
            }
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
    /// Stable sorting permutation for an all-valid numeric column via radix
    /// sort over the typed buffer, or `None` when the typed fast path does not
    /// apply (non-numeric dtype, or any missing value — those go through the
    /// `Scalar` comparator which alone reasons about na-last placement). The
    /// permutation is bit-identical to the stable comparator path: monotonic
    /// radix keys preserve `<` order and stable counting-sort preserves ties.
    /// Borrowed `&str` view of an all-valid Utf8 column, `None` when the
    /// column has any missing slot or any non-Utf8 scalar (those need the
    /// na-last / mixed-dtype comparator).
    fn as_all_valid_str_vec(&self) -> Option<Vec<&str>> {
        if self.dtype != DType::Utf8 || !self.validity.all() {
            return None;
        }
        // Contiguous fast path (br-frankenpandas-vecff): when the column
        // carries the LazyContiguousUtf8 backing (output of a string op),
        // slice each row's &str straight from the byte buffer instead of
        // forcing the whole Vec<Scalar> to materialize just to read it.
        // Bit-identical: the same &str values in the same order; argsort /
        // group keys only ever borrow them.
        if let Some((bytes, offsets)) = self.as_utf8_contiguous() {
            let mut strs = Vec::with_capacity(offsets.len() - 1);
            for w in offsets.windows(2) {
                strs.push(
                    std::str::from_utf8(&bytes[w[0]..w[1]])
                        .expect("contiguous utf8 buffer is valid by construction"),
                );
            }
            return Some(strs);
        }
        let mut strs = Vec::with_capacity(self.len());
        for v in self.values.iter() {
            match v {
                Scalar::Utf8(s) => strs.push(s.as_str()),
                _ => return None,
            }
        }
        Some(strs)
    }

    fn typed_radix_perm(&self, ascending: bool) -> Option<Vec<usize>> {
        if let Some(data) = self.as_i64_slice() {
            let keys: Vec<u64> = if ascending {
                data.iter().map(|&v| i64_radix_key(v)).collect()
            } else {
                data.iter().map(|&v| !i64_radix_key(v)).collect()
            };
            return Some(radix_argsort_u64(&keys));
        }
        if let Some(data) = self.as_f64_slice() {
            let keys: Vec<u64> = if ascending {
                data.iter().map(|&v| f64_radix_key(v)).collect()
            } else {
                data.iter().map(|&v| !f64_radix_key(v)).collect()
            };
            return Some(radix_argsort_u64(&keys));
        }
        None
    }

    /// Order-preserving `u64` radix keys for this column (per-column ascending/
    /// descending baked in), for the multi-key lexsort
    /// (`radix_argsort_multi_u64`, br-frankenpandas-lnsu6). `Some` only for an
    /// all-valid Int64 or all-valid **no-NaN** Float64 column — the cases where
    /// the radix order is bit-identical to the stable comparator: Int64 `cmp`,
    /// finite-Float64 `partial_cmp` (`-0.0` normalized to `+0.0`). A Float64
    /// column with any NaN returns `None` so the caller keeps the `Scalar`
    /// comparator (which, in the multi-key path, treats `NaN` as compare-Equal —
    /// a semantics the monotonic radix key cannot reproduce).
    #[must_use]
    pub fn typed_radix_keys(&self, ascending: bool) -> Option<Vec<u64>> {
        if let Some(data) = self.as_i64_slice() {
            return Some(if ascending {
                data.iter().map(|&v| i64_radix_key(v)).collect()
            } else {
                data.iter().map(|&v| !i64_radix_key(v)).collect()
            });
        }
        if let Some(data) = self.as_f64_slice() {
            if data.iter().any(|x| x.is_nan()) {
                return None;
            }
            return Some(if ascending {
                data.iter().map(|&v| f64_radix_key(v)).collect()
            } else {
                data.iter().map(|&v| !f64_radix_key(v)).collect()
            });
        }
        None
    }

    pub fn sort_values(&self, ascending: bool) -> Result<Self, ColumnError> {
        // Typed radix fast path: all-valid Int64/Float64 columns sort their
        // contiguous buffer comparison-free, then re-ingest typed (no 32B
        // Scalar clone or enum-match per comparison).
        if let Some(data) = self.as_i64_slice() {
            let perm = self
                .typed_radix_perm(ascending)
                .expect("i64 slice yields perm");
            let sorted: Vec<i64> = perm.iter().map(|&i| data[i]).collect();
            return Ok(Self::from_i64_values(sorted));
        }
        if let Some(data) = self.as_f64_slice() {
            let perm = self
                .typed_radix_perm(ascending)
                .expect("f64 slice yields perm");
            let sorted: Vec<f64> = perm.iter().map(|&i| data[i]).collect();
            return Ok(Self::from_f64_values(sorted));
        }
        // All-valid Utf8: gather by the stable MSD radix permutation. The
        // fallback below sorts (idx, &Scalar) pairs stably with the same
        // ordering, so cloning in permutation order yields the identical
        // value sequence.
        if let Some(strs) = self.as_all_valid_str_vec() {
            let perm = utf8_msd_argsort(&strs, ascending);
            let sorted: Vec<Scalar> = perm.iter().map(|&i| self.values[i].clone()).collect();
            return Self::new(self.dtype, sorted);
        }
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
        self.argsort_with(true)
    }

    /// Stable sorting permutation in either direction. Uses the typed radix
    /// fast path for all-valid Int64/Float64 columns (comparison-free) and the
    /// `Scalar` na-last comparator otherwise. `take(&argsort_with(asc))` equals
    /// `sort_values(asc)`. Missing values sort to the end regardless of `asc`.
    #[must_use]
    pub fn argsort_with(&self, ascending: bool) -> Vec<usize> {
        if let Some(perm) = self.typed_radix_perm(ascending) {
            return perm;
        }
        // All-valid Utf8: stable MSD byte radix replaces the O(n log n)
        // Scalar-comparator sort. Bit-identical — `String::cmp` is exactly
        // byte order with shorter-prefix-first, no value is missing (so the
        // na-last arms never fire), and both sorts are stable.
        if let Some(strs) = self.as_all_valid_str_vec() {
            return utf8_msd_argsort(&strs, ascending);
        }
        let mut indexed: Vec<(usize, &Scalar)> = self.values.iter().enumerate().collect();
        indexed.sort_by(|a, b| compare_scalars_na_last(a.1, b.1, ascending));
        indexed.into_iter().map(|(i, _)| i).collect()
    }

    /// Return indices that partition the array around kth element.
    ///
    /// Matches np.argpartition(). After partition, element at kth position
    /// is in its sorted position, elements before are <= kth element,
    /// elements after are >= kth element.
    pub fn argpartition(&self, kth: usize) -> Result<Vec<usize>, ColumnError> {
        if kth >= self.len() {
            return Err(ColumnError::InvalidLength {
                operation: "argpartition",
                expected: kth + 1,
                actual: self.len(),
            });
        }
        let mut indexed: Vec<(usize, &Scalar)> = self.values.iter().enumerate().collect();
        indexed.select_nth_unstable_by(kth, |a, b| compare_scalars_na_last(a.1, b.1, true));
        Ok(indexed.into_iter().map(|(i, _)| i).collect())
    }

    /// Partition array around kth smallest element.
    ///
    /// Matches np.partition(). Returns a partially sorted array where
    /// element at kth position is in its final sorted position.
    pub fn partition(&self, kth: usize) -> Result<Self, ColumnError> {
        let indices = self.argpartition(kth)?;
        let out: Vec<Scalar> = indices.iter().map(|&i| self.values[i].clone()).collect();
        Self::new(self.dtype, out)
    }

    /// First-order difference: `values[i] - values[i - periods]`.
    ///
    /// Matches `pd.Series.diff(periods)`. The leading `|periods|`
    /// positions are Null(NaN). Negative periods compute
    /// `values[i] - values[i + |periods|]`. Non-numeric inputs return
    /// a type error. Result dtype is always Float64.
    pub fn diff(&self, periods: i64) -> Result<Self, ColumnError> {
        let len = self.values.len();
        // Per br-frankenpandas-e607u: Timedelta64 diff preserves dtype
        // matching pandas, instead of forcing Float64 output and NaN-ing
        // via the to_f64-else catch-all.
        // Per pandas 2.2.3: Bool.diff() is XOR (cur != prev), yielding a bool
        // result with a missing leading element — NOT numeric subtraction
        // (older pandas gave [-1, 0, 1]). Timedelta64 keeps its dtype; all other
        // numeric types diff as Float64.
        let out_dtype = match self.dtype {
            DType::Timedelta64 => DType::Timedelta64,
            DType::Bool => DType::Bool,
            _ => DType::Float64,
        };
        if len == 0 || periods == 0 {
            let null = if out_dtype == DType::Timedelta64 {
                Scalar::Null(NullKind::NaT)
            } else {
                Scalar::Null(NullKind::NaN)
            };
            return Self::new(out_dtype, vec![null; len]);
        }
        let abs = periods.unsigned_abs() as usize;
        let mut out: Vec<Scalar> = Vec::with_capacity(len);
        let null_scalar = if out_dtype == DType::Timedelta64 {
            Scalar::Null(NullKind::NaT)
        } else {
            Scalar::Null(NullKind::NaN)
        };
        for i in 0..len {
            if (periods > 0 && i < abs) || (periods < 0 && i + abs >= len) {
                out.push(null_scalar.clone());
                continue;
            }
            let (cur, prev) = if periods > 0 {
                (&self.values[i], &self.values[i - abs])
            } else {
                (&self.values[i], &self.values[i + abs])
            };
            if cur.is_missing() || prev.is_missing() {
                out.push(null_scalar.clone());
                continue;
            }
            if let (Scalar::Timedelta64(cur_ns), Scalar::Timedelta64(prev_ns)) = (cur, prev) {
                if *cur_ns == Timedelta::NAT || *prev_ns == Timedelta::NAT {
                    out.push(Scalar::Null(NullKind::NaT));
                } else {
                    out.push(Scalar::Timedelta64(cur_ns.saturating_sub(*prev_ns)));
                }
                continue;
            }
            if let (Scalar::Bool(cur_b), Scalar::Bool(prev_b)) = (cur, prev) {
                // pandas 2.2.3 Bool.diff() == (cur XOR prev).
                out.push(Scalar::Bool(cur_b != prev_b));
                continue;
            }
            match (cur.to_f64(), prev.to_f64()) {
                (Ok(a), Ok(b)) => out.push(Scalar::Float64(a - b)),
                _ => out.push(Scalar::Null(NullKind::NaN)),
            }
        }
        Self::new(out_dtype, out)
    }

    /// Consecutive differences with optional prepend/append values.
    ///
    /// Matches np.ediff1d(). Prepend/append scalars are added at boundaries.
    pub fn ediff1d(
        &self,
        to_begin: Option<Scalar>,
        to_end: Option<Scalar>,
    ) -> Result<Self, ColumnError> {
        let mut out = Vec::new();
        if let Some(v) = to_begin {
            out.push(v);
        }
        for i in 1..self.values.len() {
            let cur = &self.values[i];
            let prev = &self.values[i - 1];
            if cur.is_missing() || prev.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let cf = cur.to_f64().map_err(ColumnError::Type)?;
            let pf = prev.to_f64().map_err(ColumnError::Type)?;
            out.push(Scalar::Float64(cf - pf));
        }
        if let Some(v) = to_end {
            out.push(v);
        }
        Self::new(DType::Float64, out)
    }

    /// Numerical gradient using central differences.
    ///
    /// Matches np.gradient() with uniform spacing.
    pub fn gradient(&self) -> Result<Self, ColumnError> {
        let n = self.values.len();
        if n == 0 {
            return Self::new(DType::Float64, Vec::new());
        }
        if n == 1 {
            return Self::new(DType::Float64, vec![Scalar::Float64(0.0)]);
        }
        let vals: Vec<f64> = self
            .values
            .iter()
            .map(|v| v.to_f64().unwrap_or(f64::NAN))
            .collect();
        let mut out = Vec::with_capacity(n);
        out.push(Scalar::Float64(vals[1] - vals[0]));
        for i in 1..n - 1 {
            out.push(Scalar::Float64((vals[i + 1] - vals[i - 1]) / 2.0));
        }
        out.push(Scalar::Float64(vals[n - 1] - vals[n - 2]));
        Self::new(DType::Float64, out)
    }

    /// Trapezoidal numerical integration.
    ///
    /// Matches np.trapz(). Returns scalar result of integral.
    pub fn trapz(&self, dx: f64) -> Result<Scalar, ColumnError> {
        let n = self.values.len();
        if n < 2 {
            return Ok(Scalar::Float64(0.0));
        }
        let vals: Vec<f64> = self
            .values
            .iter()
            .map(|v| v.to_f64().unwrap_or(0.0))
            .collect();
        let mut sum = 0.0;
        for i in 1..n {
            sum += (vals[i - 1] + vals[i]) / 2.0 * dx;
        }
        Ok(Scalar::Float64(sum))
    }

    /// Per-row boolean flag for duplicated values (keep='first').
    ///
    /// Matches `pd.Series.duplicated()` — all but the first occurrence
    /// of each value is flagged true. Missing values are treated as a
    /// single bucket (pandas equates NaN for this purpose).
    pub fn duplicated(&self) -> Result<Self, ColumnError> {
        self.duplicated_keep("first")
    }

    /// Per-row boolean flag for duplicated values with explicit keep policy.
    ///
    /// Matches `pd.Series.duplicated(keep=...)`. Supported policies
    /// are `"first"`, `"last"`, and `"false"` / `"none"` for pandas
    /// `keep=False`.
    pub fn duplicated_keep(&self, keep: &str) -> Result<Self, ColumnError> {
        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Null,
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
            Datetime64(i64),
            Period(i64),
            Interval(u64, u64, IntervalClosed),
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
                Scalar::Datetime64(v) => Key::Datetime64(*v),
                Scalar::Period(v) => Key::Period(*v),
                Scalar::Interval(v) => {
                    let (left, right, closed) = interval_key(v);
                    Key::Interval(left, right, closed)
                }
                Scalar::Null(_) => Key::Null,
            }
        }

        let policy = match keep {
            "first" => DupPolicy::First,
            "last" => DupPolicy::Last,
            "false" | "False" | "none" => DupPolicy::None,
            other => {
                return Err(ColumnError::Type(TypeError::NonNumericValue {
                    value: other.to_string(),
                    dtype: self.dtype,
                }));
            }
        };

        // Typed fast paths: all-valid Int64/Float64 hash their contiguous
        // buffer directly with FxHash, skipping the per-value `Key` enum and
        // SipHash. `as_*_slice` only yields all-valid buffers, so the `Null`
        // bucket never arises; Float64 normalizes -0.0→+0.0 before `to_bits`
        // exactly as `key_of` does, keeping dedup semantics bit-identical.
        if let Some(data) = self.as_i64_slice() {
            // Bounded value span → hash-free direct-address table (O(n), no
            // probing); otherwise the FxHash typed set.
            if let Some((min, range)) = i64_direct_address_range(data) {
                return Ok(Self::from_bool_values(duplicated_flags_i64_direct(
                    data, min, range, policy,
                )));
            }
            return Ok(Self::from_bool_values(duplicated_flags_typed(data, policy)));
        }
        if let Some(data) = self.as_f64_slice() {
            let keys: Vec<u64> = data
                .iter()
                .map(|&f| (if f == 0.0 { 0.0 } else { f }).to_bits())
                .collect();
            return Ok(Self::from_bool_values(duplicated_flags_typed(
                &keys, policy,
            )));
        }

        let mut flags = vec![false; self.values.len()];
        match policy {
            DupPolicy::First => {
                let mut seen: FxHashSet<Key<'_>> = FxHashSet::default();
                for (idx, value) in self.values.iter().enumerate() {
                    flags[idx] = !seen.insert(key_of(value));
                }
            }
            DupPolicy::Last => {
                let mut seen: FxHashSet<Key<'_>> = FxHashSet::default();
                for (idx, value) in self.values.iter().enumerate().rev() {
                    flags[idx] = !seen.insert(key_of(value));
                }
            }
            DupPolicy::None => {
                let mut seen_once: FxHashSet<Key<'_>> = FxHashSet::default();
                let mut seen_multiple: FxHashSet<Key<'_>> = FxHashSet::default();
                for value in &self.values {
                    let key = key_of(value);
                    if !seen_once.insert(key_of(value)) {
                        seen_multiple.insert(key);
                    }
                }
                for (idx, value) in self.values.iter().enumerate() {
                    flags[idx] = seen_multiple.contains(&key_of(value));
                }
            }
        }

        let out: Vec<Scalar> = flags.into_iter().map(Scalar::Bool).collect();
        Self::new(DType::Bool, out)
    }

    /// Bool column indicating whether each value lies in `[lower, upper]`
    /// (or the open interval when `inclusive=false`).
    ///
    /// Matches `pd.Series.between(left, right, inclusive='both'|'neither')`.
    /// Missing values map to false. Non-numeric inputs return a type
    /// error.
    pub fn between(&self, lower: f64, upper: f64, inclusive: bool) -> Result<Self, ColumnError> {
        let policy = if inclusive { "both" } else { "neither" };
        self.between_inclusive(lower, upper, policy)
    }

    /// Bool column indicating whether each value lies between bounds
    /// with pandas string-valued side-inclusion semantics.
    ///
    /// Matches `pd.Series.between(inclusive=...)` for `"both"`,
    /// `"left"`, `"right"`, and `"neither"`.
    pub fn between_inclusive(
        &self,
        lower: f64,
        upper: f64,
        inclusive: &str,
    ) -> Result<Self, ColumnError> {
        let (include_left, include_right) = match inclusive {
            "both" => (true, true),
            "left" => (true, false),
            "right" => (false, true),
            "neither" => (false, false),
            other => {
                return Err(ColumnError::Type(TypeError::NonNumericValue {
                    value: other.to_string(),
                    dtype: self.dtype,
                }));
            }
        };

        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Bool(false));
                continue;
            }
            match v.to_f64() {
                Ok(x) => {
                    let lower_ok = if include_left { x >= lower } else { x > lower };
                    let upper_ok = if include_right { x <= upper } else { x < upper };
                    out.push(Scalar::Bool(lower_ok && upper_ok));
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
        self.factorize_with_options(false, true)
    }

    /// Encode the column as integer codes plus unique values.
    ///
    /// Matches `pd.Series.factorize(sort=..., use_na_sentinel=...)`.
    /// When `sort=true`, uniques are sorted and codes are remapped to the
    /// sorted positions. When `use_na_sentinel=false`, missing values are
    /// emitted as a regular unique bucket instead of `-1`.
    pub fn factorize_with_options(
        &self,
        sort: bool,
        use_na_sentinel: bool,
    ) -> Result<(Self, Self), ColumnError> {
        // Per br-frankenpandas-9433f: HashMap-based code lookup mirrors
        // fp-frame's Series::factorize fix (br-78d0c). fp-columnar can't
        // import fp-frame's ScalarKey (cycle), so define a local
        // hashable wrapper. missing_position tracker handles the
        // use_na_sentinel=false branch separately so multiple null
        // kinds collapse to the same code (matches the existing
        // is_missing-based check).
        #[derive(Hash, PartialEq, Eq, Clone, Copy)]
        enum LocalKey<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
            Datetime64(i64),
            Period(i64),
            Interval(u64, u64, IntervalClosed),
        }
        fn key_of(s: &Scalar) -> Option<LocalKey<'_>> {
            match s {
                Scalar::Null(_) => None,
                Scalar::Bool(b) => Some(LocalKey::Bool(*b)),
                Scalar::Int64(i) => Some(LocalKey::Int64(*i)),
                Scalar::Float64(f) => {
                    if f.is_nan() {
                        None
                    } else {
                        let normalized = if *f == 0.0 { 0.0 } else { *f };
                        Some(LocalKey::FloatBits(normalized.to_bits()))
                    }
                }
                Scalar::Utf8(s) => Some(LocalKey::Utf8(s.as_str())),
                Scalar::Timedelta64(t) => {
                    if *t == Timedelta::NAT {
                        None
                    } else {
                        Some(LocalKey::Timedelta64(*t))
                    }
                }
                Scalar::Datetime64(t) => {
                    if *t == Timestamp::NAT {
                        None
                    } else {
                        Some(LocalKey::Datetime64(*t))
                    }
                }
                Scalar::Period(p) => {
                    if *p == i64::MIN {
                        None
                    } else {
                        Some(LocalKey::Period(*p))
                    }
                }
                Scalar::Interval(interval) => {
                    let (left, right, closed) = interval_key(interval);
                    Some(LocalKey::Interval(left, right, closed))
                }
            }
        }

        let (mut codes, mut uniques): (Vec<Scalar>, Vec<Scalar>) = if let Some((data, min, range)) =
            self.as_i64_slice()
                .and_then(|d| i64_direct_address_range(d).map(|(m, r)| (d, m, r)))
        {
            // Hash-free direct-address factorize for a bounded-range all-valid
            // Int64 column: a dense code table indexed by (v-min) assigns
            // first-seen codes in O(n) with no hashing. All-valid ⇒ no
            // missing/sentinel handling, so this is bit-identical to the
            // HashMap path's first-seen code assignment.
            let mut code_table = vec![-1i64; range];
            let mut uniques: Vec<Scalar> = Vec::new();
            let mut codes: Vec<Scalar> = Vec::with_capacity(data.len());
            for &v in data {
                let slot = (v as i128 - min as i128) as usize;
                let existing = code_table[slot];
                if existing < 0 {
                    let code = uniques.len() as i64;
                    code_table[slot] = code;
                    uniques.push(Scalar::Int64(v));
                    codes.push(Scalar::Int64(code));
                } else {
                    codes.push(Scalar::Int64(existing));
                }
            }
            (codes, uniques)
        } else {
            let mut uniques: Vec<Scalar> = Vec::new();
            let mut idx_map: FxHashMap<LocalKey<'_>, i64> = FxHashMap::default();
            let mut missing_position: Option<i64> = None;
            let mut codes: Vec<Scalar> = Vec::with_capacity(self.values.len());

            for value in &self.values {
                if value.is_missing() {
                    if use_na_sentinel {
                        codes.push(Scalar::Int64(-1));
                    } else if let Some(p) = missing_position {
                        codes.push(Scalar::Int64(p));
                    } else {
                        let code = uniques.len() as i64;
                        missing_position = Some(code);
                        uniques.push(value.clone());
                        codes.push(Scalar::Int64(code));
                    }
                    continue;
                }
                let Some(key) = key_of(value) else {
                    // Defensive: non-missing value that maps to no key
                    // (shouldn't happen for valid Scalar variants).
                    codes.push(Scalar::Int64(-1));
                    continue;
                };
                match idx_map.get(&key) {
                    Some(&p) => codes.push(Scalar::Int64(p)),
                    None => {
                        let code = uniques.len() as i64;
                        idx_map.insert(key, code);
                        uniques.push(value.clone());
                        codes.push(Scalar::Int64(code));
                    }
                }
            }
            drop(idx_map);
            (codes, uniques)
        };

        if sort && !uniques.is_empty() {
            let mut ordering: Vec<usize> = (0..uniques.len()).collect();
            ordering.sort_by(|left, right| {
                compare_scalars_na_last(&uniques[*left], &uniques[*right], true)
            });

            let mut remap = vec![0usize; uniques.len()];
            let sorted_uniques: Vec<Scalar> = ordering
                .into_iter()
                .enumerate()
                .map(|(sorted_position, original_position)| {
                    remap[original_position] = sorted_position;
                    uniques[original_position].clone()
                })
                .collect();

            for code in &mut codes {
                if let Scalar::Int64(value) = code
                    && *value >= 0
                {
                    *value = remap[*value as usize] as i64;
                }
            }

            uniques = sorted_uniques;
        }

        let codes_col = Self::new(DType::Int64, codes)?;
        let uniques_col = Self::new(self.dtype, uniques)?;
        Ok((codes_col, uniques_col))
    }

    /// Element-wise absolute value.
    ///
    /// Matches `pd.Series.abs()`. Int/Float/Bool/Timedelta paths preserve
    /// dtype; Utf8 inputs return `ColumnError::Type` because pandas raises
    /// TypeError on non-numeric .abs().
    pub fn abs(&self) -> Result<Self, ColumnError> {
        // Typed fast path: all-valid Int64/Float64 take abs over the contiguous
        // buffer and re-ingest typed (same dtype preserved), skipping the lazy
        // Scalar materialization and the 32B-per-cell Vec<Scalar>. Bit-identical
        // to the loop below (Int64 wrapping_abs incl i64::MIN; Float64 .abs()
        // incl -0.0→0.0; all-valid ⇒ no missing branch).
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_i64_values(
                data.iter().map(|&x| x.wrapping_abs()).collect(),
            ));
        }
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(
                data.iter().map(|&x| x.abs()).collect(),
            ));
        }

        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(v.clone());
                continue;
            }
            match v {
                Scalar::Bool(x) => out.push(Scalar::Bool(*x)),
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

    /// Alias for abs, matching np.fabs.
    pub fn fabs(&self) -> Result<Self, ColumnError> {
        self.abs()
    }

    /// Alias for abs, matching np.absolute.
    pub fn absolute(&self) -> Result<Self, ColumnError> {
        self.abs()
    }

    /// Negate numeric values. Matches numpy's negative ufunc.
    pub fn neg(&self) -> Result<Self, ColumnError> {
        // Typed, dtype-preserving fast path (all-valid only): Int64 negates over
        // the i64 buffer (wrapping, incl i64::MIN) and stays Int64; Float64
        // negates over the f64 buffer. Bit-identical to the scalar loop.
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_i64_values(
                data.iter().map(|&x| x.wrapping_neg()).collect(),
            ));
        }
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(data.iter().map(|&x| -x).collect()));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(v.clone());
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Int64(x.wrapping_neg())),
                Scalar::Float64(x) => out.push(Scalar::Float64(-x)),
                Scalar::Timedelta64(x) if *x != Timedelta::NAT => {
                    out.push(Scalar::Timedelta64(x.wrapping_neg()))
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

    /// Unary positive (identity for numeric, error for non-numeric).
    pub fn positive(&self) -> Result<Self, ColumnError> {
        for v in &self.values {
            if v.is_missing() {
                continue;
            }
            match v {
                Scalar::Int64(_) | Scalar::Float64(_) | Scalar::Timedelta64(_) => {}
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Ok(self.clone())
    }

    /// Alias for positive.
    pub fn negative(&self) -> Result<Self, ColumnError> {
        self.neg()
    }

    /// Square root of numeric values. Matches numpy's sqrt ufunc.
    pub fn sqrt(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::sqrt) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).sqrt())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.sqrt())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Exponential (e^x) of numeric values. Matches numpy's exp ufunc.
    pub fn exp(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::exp) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).exp())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.exp())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Natural logarithm of numeric values. Matches numpy's log ufunc.
    pub fn log(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::ln) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).ln())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.ln())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Base-10 logarithm of numeric values. Matches numpy's log10 ufunc.
    pub fn log10(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::log10) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).log10())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.log10())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Base-2 logarithm of numeric values. Matches numpy's log2 ufunc.
    pub fn log2(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::log2) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).log2())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.log2())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise sine.
    pub fn sin(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::sin) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).sin())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.sin())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise cosine.
    pub fn cos(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::cos) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).cos())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.cos())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise tangent.
    pub fn tan(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::tan) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).tan())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.tan())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise arcsine.
    pub fn asin(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::asin) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).asin())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.asin())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise arccosine.
    pub fn acos(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::acos) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).acos())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.acos())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise arctangent.
    pub fn atan(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::atan) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).atan())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.atan())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise hyperbolic sine.
    pub fn sinh(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::sinh) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).sinh())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.sinh())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise hyperbolic cosine.
    pub fn cosh(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::cosh) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).cosh())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.cosh())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise hyperbolic tangent.
    pub fn tanh(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::tanh) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).tanh())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.tanh())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise inverse hyperbolic sine.
    pub fn asinh(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::asinh) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).asinh())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.asinh())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise inverse hyperbolic cosine.
    pub fn acosh(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::acosh) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).acosh())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.acosh())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise inverse hyperbolic tangent.
    pub fn atanh(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::atanh) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).atanh())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.atanh())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Numpy-style alias for asin.
    ///
    /// Matches np.arcsin(x).
    pub fn arcsin(&self) -> Result<Self, ColumnError> {
        self.asin()
    }

    /// Numpy-style alias for acos.
    ///
    /// Matches np.arccos(x).
    pub fn arccos(&self) -> Result<Self, ColumnError> {
        self.acos()
    }

    /// Numpy-style alias for atan.
    ///
    /// Matches np.arctan(x).
    pub fn arctan(&self) -> Result<Self, ColumnError> {
        self.atan()
    }

    /// Numpy-style alias for atan2.
    ///
    /// Matches np.arctan2(y, x).
    pub fn arctan2(&self, other: &Self) -> Result<Self, ColumnError> {
        self.atan2(other)
    }

    /// Numpy-style alias for asinh.
    ///
    /// Matches np.arcsinh(x).
    pub fn arcsinh(&self) -> Result<Self, ColumnError> {
        self.asinh()
    }

    /// Numpy-style alias for acosh.
    ///
    /// Matches np.arccosh(x).
    pub fn arccosh(&self) -> Result<Self, ColumnError> {
        self.acosh()
    }

    /// Numpy-style alias for atanh.
    ///
    /// Matches np.arctanh(x).
    pub fn arctanh(&self) -> Result<Self, ColumnError> {
        self.atanh()
    }

    /// Compute element-wise floor.
    /// Typed fast path shared by floor/ceil/trunc: an all-valid Float64 (or
    /// Int64) column maps `f` over its contiguous buffer and re-ingests via
    /// `from_f64_values`, skipping lazy Scalar materialization + the 32 B/cell
    /// Vec<Scalar> + Column::new revalidation. Returns `None` (fall back to the
    /// scalar loop) for nullable / non-numeric columns. Bit-identical: all-valid
    /// ⇒ the `is_missing → NaN` branch never fires; for Int64 the scalar path
    /// casts `x as f64` and `f(x as f64) == x as f64` since the cast is integral;
    /// floor/ceil/trunc of finite/Inf inputs never synthesize a NaN, and
    /// from_f64_values re-marks any NaN exactly as Self::new would.
    fn typed_float_unary(&self, f: fn(f64) -> f64) -> Option<Self> {
        if let Some(data) = self.as_f64_slice() {
            return Some(Self::from_f64_values(data.iter().map(|&x| f(x)).collect()));
        }
        if let Some(data) = self.as_i64_slice() {
            return Some(Self::from_f64_values(
                data.iter().map(|&x| f(x as f64)).collect(),
            ));
        }
        None
    }

    /// All-valid numeric column → an owned `f64` view (Float64 copied, Int64 cast
    /// `x as f64`), exactly as `Scalar::to_f64` would. `None` for nullable /
    /// non-numeric columns (so binary ufuncs fall back to the scalar loop).
    fn all_valid_as_f64(&self) -> Option<Vec<f64>> {
        if let Some(s) = self.as_f64_slice() {
            return Some(s.to_vec());
        }
        if let Some(s) = self.as_i64_slice() {
            return Some(s.iter().map(|&x| x as f64).collect());
        }
        None
    }

    /// Typed fast path for a Float64-output binary ufunc: when both columns are
    /// all-valid numeric, map `f` over the two contiguous buffers and re-ingest
    /// via `from_f64_values`, skipping per-element Scalar dispatch/clone on both
    /// sides. Caller must have validated equal length. Bit-identical: all-valid
    /// ⇒ no missing→NaN branch; `f(a,b)` is the scalar loop's `f(a.to_f64(),
    /// b.to_f64())`; from_f64_values re-marks any NaN result missing as Self::new
    /// would. Returns `None` to fall back when either side is nullable/non-numeric.
    fn typed_float_binary(&self, other: &Self, f: fn(f64, f64) -> f64) -> Option<Self> {
        let a = self.all_valid_as_f64()?;
        let b = other.all_valid_as_f64()?;
        Some(Self::from_f64_values(
            a.iter().zip(b.iter()).map(|(&x, &y)| f(x, y)).collect(),
        ))
    }

    pub fn floor(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::floor) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(*x as f64)),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.floor())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise ceiling.
    pub fn ceil(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::ceil) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(*x as f64)),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.ceil())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise truncation toward zero.
    pub fn trunc(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::trunc) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(*x as f64)),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.trunc())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Replace NaN with zero and infinity with large finite numbers.
    ///
    /// Matches np.nan_to_num(x). NaN becomes 0, positive infinity becomes
    /// a large positive number, negative infinity becomes a large negative number.
    pub fn nan_to_num(&self) -> Result<Self, ColumnError> {
        self.nan_to_num_with_values(0.0, f64::MAX, f64::MIN)
    }

    /// Replace NaN and infinity with specified values.
    ///
    /// Matches np.nan_to_num(x, nan=nan, posinf=posinf, neginf=neginf).
    pub fn nan_to_num_with_values(
        &self,
        nan: f64,
        posinf: f64,
        neginf: f64,
    ) -> Result<Self, ColumnError> {
        // Typed fast path (all-valid only, output Float64). all-valid Float64 has
        // no NaN (NaN marks a column invalid), so only the ±Inf replacements can
        // fire; the Int64 branch is a plain x as f64. Bit-identical to the scalar
        // loop; from_f64_values re-marks a NaN replacement (e.g. posinf=NaN)
        // missing exactly as Self::new would.
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(
                data.iter()
                    .map(|&x| {
                        if x == f64::INFINITY {
                            posinf
                        } else if x == f64::NEG_INFINITY {
                            neginf
                        } else {
                            x
                        }
                    })
                    .collect(),
            ));
        }
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_f64_values(
                data.iter().map(|&x| x as f64).collect(),
            ));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            let result = match v {
                Scalar::Float64(x) => {
                    if x.is_nan() {
                        nan
                    } else if *x == f64::INFINITY {
                        posinf
                    } else if *x == f64::NEG_INFINITY {
                        neginf
                    } else {
                        *x
                    }
                }
                Scalar::Int64(x) => *x as f64,
                Scalar::Null(_) => nan,
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            };
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Round to nearest even integer (banker's rounding).
    ///
    /// Matches np.rint(x). Values exactly halfway between integers round to
    /// the nearest even integer.
    pub fn rint(&self) -> Result<Self, ColumnError> {
        // round-half-to-even; for Int64 round_ties_even(x as f64) == x as f64
        // (integral), matching the scalar Float64(x as f64) branch. Output Float64.
        if let Some(out) = self.typed_float_unary(f64::round_ties_even) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(*x as f64)),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.round_ties_even())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Round toward zero (same as trunc).
    ///
    /// Matches np.fix(x). Alias for trunc().
    pub fn fix(&self) -> Result<Self, ColumnError> {
        self.trunc()
    }

    /// Trim leading and/or trailing zeros from a 1-D array.
    ///
    /// Matches np.trim_zeros(). The `trim` parameter specifies:
    /// - "f" or "fb": trim from front (leading zeros)
    /// - "b" or "fb": trim from back (trailing zeros)
    /// - "fb" (default): trim both
    pub fn trim_zeros(&self, trim: &str) -> Result<Self, ColumnError> {
        let values = &self.values;
        if values.is_empty() {
            return Self::new(self.dtype, vec![]);
        }

        let is_zero = |s: &Scalar| -> bool {
            match s {
                Scalar::Int64(x) => *x == 0,
                Scalar::Float64(x) => *x == 0.0,
                Scalar::Bool(b) => !*b,
                _ => false,
            }
        };

        let mut start = 0;
        let mut end = values.len();

        if trim.contains('f') {
            while start < end && is_zero(&values[start]) {
                start += 1;
            }
        }

        if trim.contains('b') {
            while end > start && is_zero(&values[end - 1]) {
                end -= 1;
            }
        }

        Self::new(self.dtype, values[start..end].to_vec())
    }

    /// Round to the given number of decimals.
    ///
    /// Matches np.around(a, decimals). For negative decimals, rounds to
    /// the left of the decimal point (e.g., decimals=-1 rounds to tens).
    pub fn around(&self, decimals: i32) -> Result<Self, ColumnError> {
        let factor = 10.0_f64.powi(decimals);
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    if decimals >= 0 {
                        out.push(Scalar::Int64(*x));
                    } else {
                        // np.around uses round-half-to-even (banker's), e.g.
                        // around([25], -1) -> 20, not 30.
                        let rounded = ((*x as f64) * factor).round_ties_even() / factor;
                        out.push(Scalar::Int64(rounded as i64));
                    }
                }
                Scalar::Float64(x) => {
                    let rounded = (*x * factor).round_ties_even() / factor;
                    out.push(Scalar::Float64(rounded));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        if decimals >= 0 && self.dtype == DType::Int64 {
            Self::new(DType::Int64, out)
        } else {
            Self::new(DType::Float64, out)
        }
    }

    /// Unwrap by changing deltas between values to their 2*pi complements.
    ///
    /// Matches np.unwrap(). Unwraps radian phase values by adding multiples
    /// of 2*pi when the absolute difference from the previous value exceeds
    /// the discontinuity threshold (default: pi).
    pub fn unwrap(&self, discont: Option<f64>) -> Result<Self, ColumnError> {
        let threshold = discont.unwrap_or(std::f64::consts::PI);
        let two_pi = 2.0 * std::f64::consts::PI;

        let mut out = Vec::with_capacity(self.values.len());
        let mut offset = 0.0;

        for (i, v) in self.values.iter().enumerate() {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let x = match v {
                Scalar::Int64(x) => *x as f64,
                Scalar::Float64(x) => *x,
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            };

            if i == 0 {
                out.push(Scalar::Float64(x));
            } else {
                let prev = match &out[out.len() - 1] {
                    Scalar::Float64(p) if !p.is_nan() => *p,
                    _ => {
                        out.push(Scalar::Float64(x + offset));
                        continue;
                    }
                };

                let diff = x + offset - prev;
                if diff > threshold {
                    offset -= two_pi * ((diff + std::f64::consts::PI) / two_pi).floor();
                } else if diff < -threshold {
                    offset += two_pi * ((-diff + std::f64::consts::PI) / two_pi).floor();
                }
                out.push(Scalar::Float64(x + offset));
            }
        }

        Self::new(DType::Float64, out)
    }

    /// Compute exp(x) - 1 with improved precision for small x.
    pub fn expm1(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::exp_m1) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).exp_m1())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.exp_m1())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute ln(1 + x) with improved precision for small x.
    pub fn log1p(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::ln_1p) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).ln_1p())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.ln_1p())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise cube root.
    pub fn cbrt(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::cbrt) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).cbrt())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.cbrt())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Multiply by 2 raised to an integer power.
    ///
    /// Matches np.ldexp(x, exp). Computes x * 2^exp for each element.
    pub fn ldexp(&self, exp: i32) -> Result<Self, ColumnError> {
        let multiplier = 2.0_f64.powi(exp);
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(*x as f64 * multiplier)),
                Scalar::Float64(x) => out.push(Scalar::Float64(x * multiplier)),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Split into integer and fractional parts.
    ///
    /// Matches np.modf(x). Returns (fractional_part, integer_part) as two columns.
    /// The fractional part has the same sign as the input.
    pub fn modf(&self) -> Result<(Self, Self), ColumnError> {
        let mut frac = Vec::with_capacity(self.values.len());
        let mut int = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                frac.push(Scalar::Float64(f64::NAN));
                int.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    frac.push(Scalar::Float64(0.0));
                    int.push(Scalar::Float64(*x as f64));
                }
                Scalar::Float64(x) => {
                    let i = x.trunc();
                    let f = x - i;
                    frac.push(Scalar::Float64(f));
                    int.push(Scalar::Float64(i));
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Ok((
            Self::new(DType::Float64, frac)?,
            Self::new(DType::Float64, int)?,
        ))
    }

    /// Decompose float into mantissa and exponent.
    ///
    /// Matches np.frexp(x). Returns (mantissa, exponent) where:
    /// - mantissa is in [0.5, 1.0) or exactly 0.0
    /// - exponent is an integer
    /// - x = mantissa * 2^exponent
    pub fn frexp(&self) -> Result<(Self, Self), ColumnError> {
        let mut mantissa = Vec::with_capacity(self.values.len());
        let mut exponent = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                mantissa.push(Scalar::Float64(f64::NAN));
                exponent.push(Scalar::Int64(0));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    let f = *x as f64;
                    if f == 0.0 {
                        mantissa.push(Scalar::Float64(0.0));
                        exponent.push(Scalar::Int64(0));
                    } else {
                        let bits = f.abs().to_bits();
                        let exp_bits = ((bits >> 52) & 0x7ff) as i64;
                        let exp = exp_bits - 1022; // normalized mantissa is in [0.5, 1.0)
                        let mant_bits = (bits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000;
                        let mant = f64::from_bits(mant_bits);
                        let mant = if f < 0.0 { -mant } else { mant };
                        mantissa.push(Scalar::Float64(mant));
                        exponent.push(Scalar::Int64(exp));
                    }
                }
                Scalar::Float64(x) => {
                    if x.is_nan() {
                        mantissa.push(Scalar::Float64(f64::NAN));
                        exponent.push(Scalar::Int64(0));
                    } else if x.is_infinite() {
                        mantissa.push(Scalar::Float64(*x));
                        exponent.push(Scalar::Int64(0));
                    } else if *x == 0.0 {
                        mantissa.push(Scalar::Float64(*x)); // preserves sign of zero
                        exponent.push(Scalar::Int64(0));
                    } else {
                        let bits = x.abs().to_bits();
                        let exp_bits = ((bits >> 52) & 0x7ff) as i64;
                        if exp_bits == 0 {
                            // denormalized number - scale up and extract
                            let scaled = x.abs() * 2.0_f64.powi(64);
                            let sbits = scaled.to_bits();
                            let sexp_bits = ((sbits >> 52) & 0x7ff) as i64;
                            let exp = sexp_bits - 1022 - 64;
                            let mant_bits = (sbits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000;
                            let mant = f64::from_bits(mant_bits);
                            let mant = if *x < 0.0 { -mant } else { mant };
                            mantissa.push(Scalar::Float64(mant));
                            exponent.push(Scalar::Int64(exp));
                        } else {
                            let exp = exp_bits - 1022;
                            let mant_bits = (bits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000;
                            let mant = f64::from_bits(mant_bits);
                            let mant = if *x < 0.0 { -mant } else { mant };
                            mantissa.push(Scalar::Float64(mant));
                            exponent.push(Scalar::Int64(exp));
                        }
                    }
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Ok((
            Self::new(DType::Float64, mantissa)?,
            Self::new(DType::Int64, exponent)?,
        ))
    }

    /// Return the next representable floating-point value after x toward y.
    ///
    /// Matches np.nextafter(x, y). For each pair of elements, returns the
    /// next representable float after x in the direction of y.
    pub fn nextafter(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (v1, v2) in self.values.iter().zip(other.values.iter()) {
            if v1.is_missing() || v2.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let x = v1.to_f64().map_err(ColumnError::Type)?;
            let y = v2.to_f64().map_err(ColumnError::Type)?;
            let result = if x.is_nan() || y.is_nan() {
                f64::NAN
            } else if x == y {
                x
            } else if x == 0.0 {
                // Smallest positive/negative denormal, not MIN_POSITIVE (normalized)
                if y > 0.0 {
                    f64::from_bits(1) // smallest positive denormal ≈ 5e-324
                } else {
                    -f64::from_bits(1) // smallest negative denormal ≈ -5e-324
                }
            } else {
                let bits = x.to_bits() as i64;
                let next_bits = if (x > 0.0) == (y > x) {
                    bits + 1
                } else {
                    bits - 1
                };
                f64::from_bits(next_bits as u64)
            };
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Check if values are negative infinity.
    ///
    /// Matches np.isneginf(x). Returns a Bool column that is True where
    /// the value is negative infinity.
    pub fn isneginf(&self) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Bool(false));
                continue;
            }
            match v {
                Scalar::Int64(_) => out.push(Scalar::Bool(false)),
                Scalar::Float64(x) => out.push(Scalar::Bool(*x == f64::NEG_INFINITY)),
                _ => out.push(Scalar::Bool(false)),
            }
        }
        Self::new(DType::Bool, out)
    }

    /// Check if values are positive infinity.
    ///
    /// Matches np.isposinf(x). Returns a Bool column that is True where
    /// the value is positive infinity.
    pub fn isposinf(&self) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Bool(false));
                continue;
            }
            match v {
                Scalar::Int64(_) => out.push(Scalar::Bool(false)),
                Scalar::Float64(x) => out.push(Scalar::Bool(*x == f64::INFINITY)),
                _ => out.push(Scalar::Bool(false)),
            }
        }
        Self::new(DType::Bool, out)
    }

    /// Compute 2 raised to the power of each element.
    ///
    /// Matches np.exp2(x). Returns 2^x for each element.
    pub fn exp2(&self) -> Result<Self, ColumnError> {
        // Typed fast path (all-valid only, output Float64). The Int64 branch must
        // keep `2.0.powi(x as i32)` (NOT (x as f64).exp2()) to match the scalar
        // loop's exact rounding; Float64 uses x.exp2(). Bit-identical.
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(
                data.iter().map(|&x| x.exp2()).collect(),
            ));
        }
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_f64_values(
                data.iter().map(|&x| 2.0_f64.powi(x as i32)).collect(),
            ));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(2.0_f64.powi(*x as i32))),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.exp2())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute the sinc function.
    ///
    /// Matches np.sinc(x). Returns sin(pi*x) / (pi*x), with sinc(0) = 1.
    pub fn sinc(&self) -> Result<Self, ColumnError> {
        // all-valid ⇒ no NaN, so the scalar formula reduces to 0->1 else
        // sin(πx)/(πx) for both Float64 and Int64 (x as f64). Bit-identical.
        if let Some(out) = self.typed_float_unary(|x| {
            if x == 0.0 {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                px.sin() / px
            }
        }) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    if *x == 0 {
                        out.push(Scalar::Float64(1.0));
                    } else {
                        let px = std::f64::consts::PI * (*x as f64);
                        out.push(Scalar::Float64(px.sin() / px));
                    }
                }
                Scalar::Float64(x) => {
                    if *x == 0.0 {
                        out.push(Scalar::Float64(1.0));
                    } else if x.is_nan() {
                        out.push(Scalar::Float64(f64::NAN));
                    } else {
                        let px = std::f64::consts::PI * x;
                        out.push(Scalar::Float64(px.sin() / px));
                    }
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute log(exp(x) + exp(y)) in a numerically stable way.
    ///
    /// Matches np.logaddexp(x, y). Useful for log-domain arithmetic
    /// where direct computation would overflow/underflow.
    pub fn logaddexp(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (v1, v2) in self.values.iter().zip(other.values.iter()) {
            if v1.is_missing() || v2.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let x = v1.to_f64().map_err(ColumnError::Type)?;
            let y = v2.to_f64().map_err(ColumnError::Type)?;
            let result = if x.is_nan() || y.is_nan() {
                f64::NAN
            } else if x == f64::NEG_INFINITY {
                y
            } else if y == f64::NEG_INFINITY {
                x
            } else if x == f64::INFINITY || y == f64::INFINITY {
                f64::INFINITY
            } else if x >= y {
                x + (y - x).exp().ln_1p()
            } else {
                y + (x - y).exp().ln_1p()
            };
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Compute log2(2**x + 2**y) in a numerically stable way.
    ///
    /// Matches np.logaddexp2(x, y). Like logaddexp but using base 2.
    pub fn logaddexp2(&self, other: &Self) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let ln2 = std::f64::consts::LN_2;
        let mut out = Vec::with_capacity(self.values.len());
        for (v1, v2) in self.values.iter().zip(other.values.iter()) {
            if v1.is_missing() || v2.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            let x = v1.to_f64().map_err(ColumnError::Type)?;
            let y = v2.to_f64().map_err(ColumnError::Type)?;
            let result = if x.is_nan() || y.is_nan() {
                f64::NAN
            } else if x == f64::NEG_INFINITY {
                y
            } else if y == f64::NEG_INFINITY {
                x
            } else if x == f64::INFINITY || y == f64::INFINITY {
                f64::INFINITY
            } else if x >= y {
                x + ((y - x) * ln2).exp().ln_1p() / ln2
            } else {
                y + ((x - y) * ln2).exp().ln_1p() / ln2
            };
            out.push(Scalar::Float64(result));
        }
        Self::new(DType::Float64, out)
    }

    /// Compute spacing between this value and the next representable float.
    ///
    /// Matches np.spacing(x). Returns the ULP (unit in last place) - the
    /// distance to the next representable float away from zero.
    pub fn spacing(&self) -> Result<Self, ColumnError> {
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => {
                    let f = (*x as f64).abs();
                    if f == 0.0 {
                        out.push(Scalar::Float64(f64::from_bits(1)));
                    } else {
                        let bits = f.to_bits();
                        let next = f64::from_bits(bits + 1);
                        out.push(Scalar::Float64(next - f));
                    }
                }
                Scalar::Float64(x) => {
                    if x.is_nan() || x.is_infinite() {
                        out.push(Scalar::Float64(f64::NAN));
                    } else {
                        let f = x.abs();
                        if f == 0.0 {
                            out.push(Scalar::Float64(f64::from_bits(1)));
                        } else {
                            let bits = f.to_bits();
                            let next = f64::from_bits(bits + 1);
                            out.push(Scalar::Float64(next - f));
                        }
                    }
                }
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Convert angles from degrees to radians.
    pub fn radians(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::to_radians) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).to_radians())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.to_radians())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Alias for radians.
    pub fn deg2rad(&self) -> Result<Self, ColumnError> {
        self.radians()
    }

    /// Convert angles from radians to degrees.
    pub fn degrees(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(f64::to_degrees) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64((*x as f64).to_degrees())),
                Scalar::Float64(x) => out.push(Scalar::Float64(x.to_degrees())),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Alias for degrees.
    pub fn rad2deg(&self) -> Result<Self, ColumnError> {
        self.degrees()
    }

    /// Compute element-wise reciprocal (1/x).
    pub fn reciprocal(&self) -> Result<Self, ColumnError> {
        if let Some(out) = self.typed_float_unary(|x| 1.0 / x) {
            return Ok(out);
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Float64(1.0 / (*x as f64))),
                Scalar::Float64(x) => out.push(Scalar::Float64(1.0 / x)),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        Self::new(DType::Float64, out)
    }

    /// Compute element-wise square (x^2).
    pub fn square(&self) -> Result<Self, ColumnError> {
        // Typed, dtype-preserving fast path (all-valid only): Int64 stays Int64
        // (`x * x`, same overflow behavior as the scalar loop); Float64 squares
        // over the f64 buffer. Bit-identical.
        if let Some(data) = self.as_i64_slice() {
            return Ok(Self::from_i64_values(data.iter().map(|&x| x * x).collect()));
        }
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(data.iter().map(|&x| x * x).collect()));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(Scalar::Float64(f64::NAN));
                continue;
            }
            match v {
                Scalar::Int64(x) => out.push(Scalar::Int64(x * x)),
                Scalar::Float64(x) => out.push(Scalar::Float64(x * x)),
                _ => {
                    return Err(ColumnError::Type(TypeError::NonNumericValue {
                        value: format!("{v:?}"),
                        dtype: self.dtype,
                    }));
                }
            }
        }
        let dtype = match self.dtype {
            DType::Int64 => DType::Int64,
            _ => DType::Float64,
        };
        Self::new(dtype, out)
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
        // Typed fast path: an all-valid numeric column clamps straight over its
        // contiguous buffer (output is always Float64), with no per-element
        // Scalar dispatch/clone or output Vec<Scalar>. Bit-identical — the scalar
        // loop applies the lower bound then the upper bound to v.to_f64(), which
        // for an all-valid Float64/Int64 column is exactly data[i] (as f64). NaN
        // floats mark the column invalid (validity.all() false), so as_*_slice
        // declines and missing values keep the Scalar path.
        let clamp = |mut x: f64| {
            if let Some(lo) = lower
                && x < lo
            {
                x = lo;
            }
            if let Some(hi) = upper
                && x > hi
            {
                x = hi;
            }
            x
        };
        if let Some(data) = self.as_f64_slice() {
            let out: Vec<f64> = data.iter().map(|&x| clamp(x)).collect();
            return Ok(Self::from_f64_values(out));
        }
        if let Some(data) = self.as_i64_slice() {
            let out: Vec<f64> = data.iter().map(|&x| clamp(x as f64)).collect();
            return Ok(Self::from_f64_values(out));
        }

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
    /// unchanged for decimals >= 0 and retain Int64 dtype for negative
    /// decimals. Bool columns pass through unchanged. Missing values are
    /// preserved.
    pub fn round(&self, decimals: i32) -> Result<Self, ColumnError> {
        if matches!(self.dtype, DType::Bool) || (self.dtype == DType::Int64 && decimals >= 0) {
            return Ok(self.clone());
        }
        if self.dtype == DType::Int64 {
            let out = self
                .values
                .iter()
                .map(|v| match v {
                    Scalar::Int64(value) => {
                        Scalar::Int64(round_i64_negative_decimals(*value, decimals))
                    }
                    Scalar::Null(kind) => Scalar::Null(*kind),
                    other => other.clone(),
                })
                .collect();
            return Self::new(DType::Int64, out);
        }
        let factor = 10f64.powi(decimals);
        // Typed fast path (mirror of `abs`): an all-valid Float64 column rounds
        // over its contiguous buffer and re-ingests typed, skipping the lazy
        // Scalar materialization, the 32 B/cell Vec<Scalar>, and Column::new's
        // revalidation passes. Bit-identical to the scalar loop below: the
        // formula is the same `(x*factor).round_ties_even()/factor`, and
        // from_f64_values re-marks any NaN result as missing exactly as
        // `Self::new(Float64, ..)` would (all-valid ⇒ no is_missing branch).
        if let Some(data) = self.as_f64_slice() {
            return Ok(Self::from_f64_values(
                data.iter()
                    .map(|&x| (x * factor).round_ties_even() / factor)
                    .collect(),
            ));
        }
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            if v.is_missing() {
                out.push(v.clone());
                continue;
            }
            match v.to_f64() {
                Ok(x) => out.push(Scalar::Float64((x * factor).round_ties_even() / factor)),
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
        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
            Datetime64(i64),
            Period(i64),
            Interval(u64, u64, IntervalClosed),
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
                Scalar::Datetime64(v) => Key::Datetime64(*v),
                Scalar::Period(v) => Key::Period(*v),
                Scalar::Interval(v) => {
                    let (left, right, closed) = interval_key(v);
                    Key::Interval(left, right, closed)
                }
                Scalar::Null(_) => return None,
            })
        }

        // Typed dense-membership fast path: an all-valid Int64 column tested
        // against bounded Int64 needles uses a direct-address presence bitset
        // (indexed by `needle - min`) scanned over the contiguous i64 buffer,
        // instead of a per-element HashSet probe over materialized Scalars.
        // Bit-identical: an Int64 value's key is `Key::Int64`, which only ever
        // matches an Int64 needle (a Float64 5.0 needle is `Key::FloatBits`, a
        // distinct key), so the membership answer is exactly "is this i64 one of
        // the Int64 needles". Falls back for non-Int64 self/needle spans.
        if let Some(data) = self.as_i64_slice() {
            let mut n_min = i64::MAX;
            let mut n_max = i64::MIN;
            let mut saw_int_needle = false;
            for needle in needles {
                if let Scalar::Int64(v) = needle {
                    saw_int_needle = true;
                    n_min = n_min.min(*v);
                    n_max = n_max.max(*v);
                }
            }
            if !saw_int_needle {
                return Ok(Self::from_bool_values(vec![false; data.len()]));
            }
            let span = i128::from(n_max) - i128::from(n_min) + 1;
            if span > 0 && span <= (1i128 << 24) {
                let mut present = vec![false; span as usize];
                for needle in needles {
                    if let Scalar::Int64(v) = needle {
                        present[(v - n_min) as usize] = true;
                    }
                }
                let out: Vec<bool> = data
                    .iter()
                    .map(|&v| v >= n_min && v <= n_max && present[(v - n_min) as usize])
                    .collect();
                return Ok(Self::from_bool_values(out));
            }
        }

        let mut lookup: FxHashSet<Key<'_>> = FxHashSet::default();
        for n in needles {
            if let Some(k) = key_of(n) {
                lookup.insert(k);
            }
        }

        // Typed all-valid Bool output — every slot is a definite true/false
        // (missing input maps to false, never to a missing output), so this is
        // the same column `Self::new(DType::Bool, Vec<Scalar::Bool>)` builds,
        // minus the 32 B/elem Scalar wrap and the validity scan (the Int64
        // dense path above already emits this way).
        let out: Vec<bool> = self
            .values
            .iter()
            .map(|v| match key_of(v) {
                Some(k) => lookup.contains(&k),
                None => false,
            })
            .collect();
        Ok(Self::from_bool_values(out))
    }

    /// Unique values in first-seen order, missing values dropped.
    ///
    /// Matches `pd.Series.unique()` (pandas returns values in order of
    /// appearance and drops NaN/NA). Float NaN is deduplicated on bit
    /// pattern; +0.0 / -0.0 fold to the same key.
    pub fn unique(&self) -> Result<Self, ColumnError> {
        // Dense direct-address fast path: an all-valid, bounded-range Int64
        // column dedups via a seen-bitset indexed by `v - min` — hash-free, no
        // per-element Scalar enum — preserving first-seen order. Bit-identical to
        // the HashSet path below (all-valid ⇒ nothing missing to skip; output is
        // the same first-seen distinct Int64 values). Same gate as isin/dense
        // duplicated (`i64_direct_address_range`).
        if let Some(data) = self.as_i64_slice()
            && let Some((min, range)) = i64_direct_address_range(data)
        {
            let mut seen = vec![false; range];
            let mut out: Vec<i64> = Vec::new();
            for &v in data {
                let slot = (v as i128 - min as i128) as usize;
                if !seen[slot] {
                    seen[slot] = true;
                    out.push(v);
                }
            }
            return Ok(Self::from_i64_values(out));
        }

        #[derive(Hash, PartialEq, Eq)]
        enum Key<'a> {
            Bool(bool),
            Int64(i64),
            FloatBits(u64),
            Utf8(&'a str),
            Timedelta64(i64),
            Datetime64(i64),
            Period(i64),
            Interval(u64, u64, IntervalClosed),
        }

        let mut seen: FxHashSet<Key<'_>> = FxHashSet::default();
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
                Scalar::Datetime64(v) => Key::Datetime64(*v),
                Scalar::Period(v) => Key::Period(*v),
                Scalar::Interval(v) => {
                    let (left, right, closed) = interval_key(v);
                    Key::Interval(left, right, closed)
                }
                Scalar::Null(_) => continue,
            };
            if seen.insert(key) {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Set difference: values in self that are not in other.
    ///
    /// Matches np.setdiff1d().
    pub fn setdiff1d(&self, other: &Self) -> Result<Self, ColumnError> {
        let other_unique = other.unique()?;
        // O(N+M): hash-set membership for `other`, plus a `seen` set replacing
        // the O(N²) `out.any(...)` first-seen dedup.
        let other_set: FxHashSet<SetMemberKey<'_>> = other_unique
            .values()
            .iter()
            .filter_map(set_member_key)
            .collect();
        let mut seen: FxHashSet<SetMemberKey<'_>> = FxHashSet::default();
        let mut out = Vec::new();
        for v in &self.values {
            let Some(key) = set_member_key(v) else {
                continue;
            };
            if !other_set.contains(&key) && seen.insert(key) {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Set intersection: values common to both columns.
    ///
    /// Matches np.intersect1d().
    pub fn intersect1d(&self, other: &Self) -> Result<Self, ColumnError> {
        let self_unique = self.unique()?;
        let other_unique = other.unique()?;
        let other_set: FxHashSet<SetMemberKey<'_>> = other_unique
            .values()
            .iter()
            .filter_map(set_member_key)
            .collect();
        let mut out = Vec::new();
        for v in self_unique.values() {
            let Some(key) = set_member_key(v) else {
                continue;
            };
            if other_set.contains(&key) {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Set union: unique values from both columns.
    ///
    /// Matches np.union1d().
    pub fn union1d(&self, other: &Self) -> Result<Self, ColumnError> {
        let mut combined = self.values.to_vec();
        combined.extend(other.values().iter().cloned());
        let temp = Self::new(self.dtype, combined)?;
        temp.unique()
    }

    /// Set symmetric difference: unique values in either but not both.
    ///
    /// Matches np.setxor1d(). Returns unique values that are in exactly
    /// one of the input arrays.
    pub fn setxor1d(&self, other: &Self) -> Result<Self, ColumnError> {
        let a_unique = self.unique()?;
        let b_unique = other.unique()?;
        let a_set: FxHashSet<SetMemberKey<'_>> = a_unique
            .values()
            .iter()
            .filter_map(set_member_key)
            .collect();
        let b_set: FxHashSet<SetMemberKey<'_>> = b_unique
            .values()
            .iter()
            .filter_map(set_member_key)
            .collect();
        let mut out = Vec::new();
        // Values in a but not in b
        for v in a_unique.values() {
            let Some(key) = set_member_key(v) else {
                continue;
            };
            if !b_set.contains(&key) {
                out.push(v.clone());
            }
        }
        // Values in b but not in a
        for v in b_unique.values() {
            let Some(key) = set_member_key(v) else {
                continue;
            };
            if !a_set.contains(&key) {
                out.push(v.clone());
            }
        }
        Self::new(self.dtype, out)
    }

    /// Test whether each element is contained in other.
    ///
    /// Matches np.in1d(). Returns Bool column.
    pub fn in1d(&self, other: &Self) -> Result<Self, ColumnError> {
        let other_unique = other.unique()?;
        let other_set: FxHashSet<SetMemberKey<'_>> = other_unique
            .values()
            .iter()
            .filter_map(set_member_key)
            .collect();
        // Typed all-valid Bool output — same equivalence as isin above.
        let mut out = Vec::with_capacity(self.values.len());
        for v in &self.values {
            let found = match set_member_key(v) {
                Some(key) => other_set.contains(&key),
                None => false, // missing is never "in" the set (matches the scan)
            };
            out.push(found);
        }
        Ok(Self::from_bool_values(out))
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
        // O(N) tally: a `set_member_key`-keyed hash map gives O(1) lookup
        // instead of the old O(distinct) linear `counts.iter().find(semantic_eq)`
        // per value (O(N·distinct), quadratic for high-cardinality data). The
        // `counts` Vec is still built in first-seen order, so the later
        // stable count-sort breaks ties identically. Bit-identical: is_missing()
        // is tested first (so NaN/NAT sentinels stay in missing_count exactly as
        // before), and for the remaining values set_member_key equality matches
        // semantic_eq (the same key Column::unique uses; ±0.0 normalized).
        let mut counts: Vec<(Scalar, usize)> = Vec::new();
        let mut index: rustc_hash::FxHashMap<SetMemberKey<'_>, usize> =
            rustc_hash::FxHashMap::default();
        let mut missing_count = 0_usize;

        for value in &self.values {
            if value.is_missing() {
                missing_count += 1;
                continue;
            }
            let Some(key) = set_member_key(value) else {
                // Unreachable: every non-missing scalar has a key.
                counts.push((value.clone(), 1));
                continue;
            };
            if let Some(&i) = index.get(&key) {
                counts[i].1 += 1;
            } else {
                index.insert(key, counts.len());
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

    /// Element-wise comparison for approximate equality.
    ///
    /// Matches np.isclose(). Returns True where |a - b| <= atol + rtol * |b|.
    pub fn isclose(&self, other: &Self, rtol: f64, atol: f64) -> Result<Self, ColumnError> {
        if self.len() != other.len() {
            return Err(ColumnError::LengthMismatch {
                left: self.len(),
                right: other.len(),
            });
        }
        let mut out = Vec::with_capacity(self.values.len());
        for (a, b) in self.values.iter().zip(&other.values) {
            if a.is_missing() || b.is_missing() {
                out.push(Scalar::Bool(false));
                continue;
            }
            let af = a.to_f64().map_err(ColumnError::Type)?;
            let bf = b.to_f64().map_err(ColumnError::Type)?;
            let close = (af - bf).abs() <= atol + rtol * bf.abs();
            out.push(Scalar::Bool(close));
        }
        Self::new(DType::Bool, out)
    }

    /// Check if all elements are approximately equal.
    ///
    /// Matches np.allclose(). Returns True if all pairs satisfy isclose.
    pub fn allclose(&self, other: &Self, rtol: f64, atol: f64) -> Result<bool, ColumnError> {
        let close = self.isclose(other, rtol, atol)?;
        for v in close.values() {
            match v {
                Scalar::Bool(true) => continue,
                Scalar::Bool(false) => return Ok(false),
                _ => return Ok(false),
            }
        }
        Ok(true)
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
    use fp_types::{DType, Interval, IntervalClosed, NullKind, Scalar, SparseDType};

    use super::{
        ArithmeticOp, Column, ColumnData, ColumnError, ScalarValues, SparseColumn, ValidityMask,
    };

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
    fn take_positions_matches_validated_materialization() {
        let column = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.5),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.5),
            ],
        )
        .expect("column should build");

        let positions = [2, 1, 0, 2];
        let gathered = column.take_positions(&positions);
        let expected_values = positions
            .iter()
            .map(|&position| column.values()[position].clone())
            .collect::<Vec<_>>();
        let expected =
            Column::new(column.dtype(), expected_values).expect("validated materialization");

        assert_eq!(gathered.dtype(), expected.dtype());
        assert_eq!(gathered.values(), expected.values());
        assert_eq!(gathered.validity(), expected.validity());

        let empty = column.take_positions(&[]);
        assert_eq!(empty.dtype(), column.dtype());
        assert!(empty.values().is_empty());
        assert_eq!(empty.validity(), &ValidityMask::all_invalid(0));
    }

    #[test]
    fn take_positions_all_valid_primitives_match_validated_materialization() {
        let cases = [
            (
                DType::Bool,
                vec![Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(false)],
            ),
            (
                DType::Int64,
                vec![Scalar::Int64(10), Scalar::Int64(-5), Scalar::Int64(42)],
            ),
            (
                DType::Float64,
                vec![
                    Scalar::Float64(1.25),
                    Scalar::Float64(-0.0),
                    Scalar::Float64(9.5),
                ],
            ),
            (
                DType::Timedelta64,
                vec![
                    Scalar::Timedelta64(10),
                    Scalar::Timedelta64(-5),
                    Scalar::Timedelta64(42),
                ],
            ),
            (
                DType::Datetime64,
                vec![
                    Scalar::Datetime64(10),
                    Scalar::Datetime64(-5),
                    Scalar::Datetime64(42),
                ],
            ),
            (
                DType::Period,
                vec![Scalar::Period(10), Scalar::Period(-5), Scalar::Period(42)],
            ),
        ];

        let positions = [2, 0, 2, 1];
        for (dtype, values) in cases {
            let column = Column::new(dtype, values).expect("column should build");
            let gathered = column.take_positions(&positions);
            let expected_values = positions
                .iter()
                .map(|&position| column.values()[position].clone())
                .collect::<Vec<_>>();
            let expected =
                Column::new(column.dtype(), expected_values).expect("validated materialization");

            assert_eq!(gathered.dtype(), expected.dtype());
            assert_eq!(gathered.values(), expected.values());
            assert_eq!(gathered.validity(), expected.validity());
        }
    }

    #[test]
    fn take_positions_preserves_exact_null_kind_contract() {
        // Isomorphism contract for the typed-columnar storage epic
        // (br-frankenpandas-typed-columnar-storage-epic): `take_positions`
        // reproduces the EXACT scalar stored at each source position, including
        // the precise `NullKind` at invalid positions — not merely a
        // valid/invalid bit. `normalize_missing_for_dtype` preserves NaN/NaT
        // null kinds regardless of dtype, so a Float64/Int64 column can legally
        // hold `Null(NaT)`. A future migration to typed `ColumnData` +
        // `ValidityMask` that canonicalizes nulls per dtype would silently
        // break `values()` parity for such columns; this test must keep passing
        // through that migration (the typed store must carry per-position null
        // kind, e.g. a 2-bit NaN/NaT/Null code, not just a validity bit).
        for (dtype, real) in [
            (DType::Float64, Scalar::Float64(2.5)),
            (DType::Int64, Scalar::Int64(7)),
        ] {
            let source = Column::new(
                dtype,
                vec![
                    Scalar::Null(NullKind::NaN),
                    Scalar::Null(NullKind::NaT),
                    Scalar::Null(NullKind::Null),
                    real,
                ],
            )
            .expect("column builds");
            // Compare against what the column actually stored, so the test is
            // robust to constructor canonicalization yet still pins that the
            // gather is byte-for-byte faithful to the stored representation.
            let stored = source.values().to_vec();
            let positions = [3, 2, 1, 0, 1];
            let gathered = source.take_positions(&positions);
            for (out_idx, &pos) in positions.iter().enumerate() {
                assert_eq!(
                    gathered.values()[out_idx],
                    stored[pos],
                    "dtype {dtype:?}: take_positions must reproduce the exact stored scalar \
                     (incl. NullKind) for source position {pos}",
                );
                // Invalid source positions must stay invalid after the gather.
                assert_eq!(
                    gathered.validity().get(out_idx),
                    source.validity().get(pos),
                    "dtype {dtype:?}: validity must follow the gathered position {pos}",
                );
            }
        }
    }

    #[test]
    fn primitive_columns_cache_typed_data_for_take_positions() {
        let column = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.25),
                Scalar::Float64(-0.0),
                Scalar::Float64(9.5),
            ],
        )
        .expect("column should build");

        assert!(matches!(column.data, Some(ColumnData::Float64(_))));
        let positions = [2, 0, 1, 2];
        let gathered = column.take_positions(&positions);
        let expected = Column::new(
            DType::Float64,
            positions
                .iter()
                .map(|&position| column.values()[position].clone())
                .collect(),
        )
        .expect("validated materialization");

        assert_eq!(gathered.values(), expected.values());
        assert_eq!(gathered.validity(), expected.validity());
    }

    #[test]
    fn float64_take_positions_defers_scalar_materialization() {
        let column = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.25),
                Scalar::Float64(-0.0),
                Scalar::Float64(9.5),
            ],
        )
        .expect("column should build");

        let positions = [2, 0, 1, 2];
        let gathered = column.take_positions(&positions);

        assert!(
            matches!(&gathered.values, ScalarValues::LazyAllValidFloat64 { .. }),
            "Float64 gather should defer scalar materialization"
        );
        if let ScalarValues::LazyAllValidFloat64 { data, values } = &gathered.values {
            assert_eq!(
                data.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
                vec![
                    9.5f64.to_bits(),
                    1.25f64.to_bits(),
                    (-0.0f64).to_bits(),
                    9.5f64.to_bits(),
                ]
            );
            assert!(values.get().is_none());
        }
        assert_eq!(gathered.len(), positions.len());
        assert_eq!(
            gathered.validity(),
            &ValidityMask::all_valid(positions.len())
        );

        let expected = Column::new(
            DType::Float64,
            positions
                .iter()
                .map(|&position| column.values()[position].clone())
                .collect(),
        )
        .expect("validated materialization");

        assert_eq!(gathered.values(), expected.values());
        assert!(
            matches!(&gathered.values, ScalarValues::LazyAllValidFloat64 { .. }),
            "Float64 gather should stay lazy after read"
        );
        if let ScalarValues::LazyAllValidFloat64 { values, .. } = &gathered.values {
            assert!(values.get().is_some());
        }
        assert_eq!(gathered.validity(), expected.validity());
    }

    #[test]
    fn reindex_all_present_matches_materialization_and_keeps_float64_lazy() {
        let column = Column::from_f64_values(vec![1.25, -0.0, f64::INFINITY]);

        let positions = [Some(2), Some(0), Some(1), Some(2)];
        let gathered = column
            .reindex_by_positions(&positions)
            .expect("all-present reindex should gather");

        assert!(
            matches!(&gathered.values, ScalarValues::LazyAllValidFloat64 { .. }),
            "all-present Float64 reindex should defer scalar materialization"
        );
        if let ScalarValues::LazyAllValidFloat64 { data, values } = &gathered.values {
            assert_eq!(
                data.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
                vec![
                    f64::INFINITY.to_bits(),
                    1.25f64.to_bits(),
                    (-0.0f64).to_bits(),
                    f64::INFINITY.to_bits(),
                ]
            );
            assert!(values.get().is_none());
        }

        let expected = Column::new(
            DType::Float64,
            positions
                .iter()
                .map(|&position| column.values()[position.expect("present position")].clone())
                .collect(),
        )
        .expect("validated scalar materialization");

        assert_eq!(gathered.dtype(), expected.dtype());
        assert_eq!(gathered.values(), expected.values());
        assert_eq!(gathered.validity(), expected.validity());
    }

    #[test]
    fn column_equality_ignores_skipped_typed_cache() {
        let column = Column::new(
            DType::Int64,
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("column should build");

        let json = serde_json::to_string(&column).expect("serialize");
        let roundtrip: Column = serde_json::from_str(&json).expect("deserialize");

        assert!(column.data.is_some());
        assert!(roundtrip.data.is_none());
        assert_eq!(column, roundtrip);
    }

    #[test]
    fn column_clone_preserves_values_without_copying_private_cache() {
        let column = Column::new(
            DType::Int64,
            vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
        )
        .expect("column should build");

        let cloned = column.clone();

        assert!(column.data.is_some());
        assert!(cloned.data.is_none());
        assert_eq!(column, cloned);
    }

    #[test]
    fn dense_primitive_clone_defers_float64_scalar_materialization_from_typed_cache() {
        let column = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.5),
                Scalar::Float64(-0.0),
                Scalar::Float64(3.25),
            ],
        )
        .expect("column should build");

        let cloned_values = column
            .clone_dense_values_from_cache()
            .expect("all-valid Float64 typed cache should clone");
        assert!(
            matches!(&cloned_values, ScalarValues::LazyAllValidFloat64 { .. }),
            "Float64 clone should defer scalar materialization"
        );
        if let ScalarValues::LazyAllValidFloat64 { data, values } = &cloned_values {
            assert_eq!(
                data.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
                vec![1.5f64.to_bits(), (-0.0f64).to_bits(), 3.25f64.to_bits()]
            );
            assert!(values.get().is_none());
        }

        let cloned = column.clone();
        assert!(
            matches!(&cloned.values, ScalarValues::LazyAllValidFloat64 { .. }),
            "Column::clone should keep all-valid Float64 clone values lazy"
        );
        if let ScalarValues::LazyAllValidFloat64 { values, .. } = &cloned.values {
            assert!(values.get().is_none());
        }
        assert_eq!(cloned.values(), column.values());
        if let ScalarValues::LazyAllValidFloat64 { values, .. } = &cloned.values {
            assert!(values.get().is_some());
        }
        assert_eq!(cloned.validity(), column.validity());
        assert!(cloned.data.is_none());
    }

    #[test]
    fn dense_primitive_clone_falls_back_for_missing_values() {
        let column = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.5),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
            ],
        )
        .expect("column should build");

        assert!(column.clone_dense_values_from_cache().is_none());
        let cloned = column.clone();
        assert_eq!(cloned.values(), column.values());
        assert_eq!(cloned.validity(), column.validity());
        assert!(cloned.data.is_none());
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

    #[test]
    fn sparse_column_omits_fill_values_and_materializes_dense() {
        let dtype = SparseDType::new(DType::Int64, Scalar::Int64(0)).expect("sparse dtype");
        let sparse = SparseColumn::from_dense(
            dtype,
            vec![
                Scalar::Int64(0),
                Scalar::Int64(5),
                Scalar::Int64(0),
                Scalar::Int64(-2),
            ],
        )
        .expect("sparse column");

        assert_eq!(sparse.value_dtype(), DType::Int64);
        assert_eq!(sparse.fill_value(), &Scalar::Int64(0));
        assert_eq!(sparse.len(), 4);
        assert_eq!(sparse.npoints(), 2);
        assert_eq!(sparse.indices(), &[1, 3]);
        assert_eq!(
            sparse.stored_values(),
            &[Scalar::Int64(5), Scalar::Int64(-2)]
        );

        let dense = sparse.to_dense_column().expect("dense column");
        assert_eq!(dense.dtype(), DType::Int64);
        assert_eq!(
            dense.values(),
            &[
                Scalar::Int64(0),
                Scalar::Int64(5),
                Scalar::Int64(0),
                Scalar::Int64(-2),
            ]
        );
    }

    #[test]
    fn sparse_column_preserves_nulls_when_fill_is_not_missing() {
        let dtype = SparseDType::new(DType::Float64, Scalar::Float64(0.0)).expect("sparse dtype");
        let sparse = SparseColumn::from_dense(
            dtype,
            vec![
                Scalar::Float64(0.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.5),
            ],
        )
        .expect("sparse column");

        assert_eq!(sparse.indices(), &[1, 2]);
        assert_eq!(sparse.npoints(), 2);
        assert!((sparse.density() - (2.0 / 3.0)).abs() < f64::EPSILON);
        assert!(sparse.stored_values()[0].is_missing());
        assert_eq!(sparse.stored_values()[1], Scalar::Float64(2.5));

        let dense = sparse.to_dense_column().expect("dense column");
        assert_eq!(
            dense.values(),
            &[
                Scalar::Float64(0.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.5),
            ]
        );
    }

    #[test]
    fn sparse_column_missing_fill_omits_missing_values() {
        let dtype =
            SparseDType::new(DType::Float64, Scalar::Null(NullKind::NaN)).expect("sparse dtype");
        let sparse = SparseColumn::from_dense(
            dtype,
            vec![
                Scalar::Null(NullKind::Null),
                Scalar::Float64(1.5),
                Scalar::Float64(f64::NAN),
            ],
        )
        .expect("sparse column");

        assert_eq!(sparse.fill_value(), &Scalar::Null(NullKind::NaN));
        assert_eq!(sparse.indices(), &[1]);
        assert_eq!(sparse.stored_values(), &[Scalar::Float64(1.5)]);
        assert_eq!(
            sparse.to_dense_values(),
            vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.5),
                Scalar::Null(NullKind::NaN),
            ]
        );
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
        assert!(
            mask.words.is_empty(),
            "all-valid masks store only the logical length"
        );
        for i in 0..100 {
            assert!(mask.get(i), "bit {i} should be valid");
        }
    }

    #[test]
    fn validity_mask_all_valid_sentinel_matches_explicit_words() {
        for len in [1, 2, 63, 64, 65, 127, 128, 129] {
            let sentinel = ValidityMask::all_valid(len);
            let explicit =
                ValidityMask::from_words(ValidityMask::materialized_all_valid_words(len), len);

            assert_eq!(sentinel, explicit, "len {len}");
            assert_eq!(
                sentinel.bits().collect::<Vec<_>>(),
                explicit.bits().collect::<Vec<_>>(),
                "len {len}"
            );
            assert!(sentinel.all(), "len {len}");
            assert_eq!(sentinel.count_invalid(), 0, "len {len}");
        }
    }

    #[test]
    fn validity_mask_all_valid_sentinel_materializes_on_clear() {
        let mut mask = ValidityMask::all_valid(130);
        mask.set(64, true);
        assert!(
            mask.words.is_empty(),
            "setting a valid bit preserves the sentinel"
        );

        mask.set(64, false);
        assert!(!mask.words.is_empty(), "clearing a bit materializes words");
        assert_eq!(mask.len(), 130);
        assert_eq!(mask.count_valid(), 129);
        assert!(!mask.get(64));
        assert!(mask.get(63));
        assert!(mask.get(65));
        assert_eq!(mask.bits().filter(|valid| *valid).count(), 129);
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
    fn validity_mask_sentinel_mask_algebra_matches_explicit_bitmap() {
        let all = ValidityMask::all_valid(5);
        let nullable = ValidityMask::from_values(&[
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(5),
        ]);

        assert_eq!(all.and_mask(&nullable), nullable);
        assert_eq!(nullable.and_mask(&all), nullable);
        assert_eq!(all.or_mask(&nullable), all);
        assert_eq!(nullable.or_mask(&all), all);
        assert_eq!(
            all.xor_mask(&nullable).bits().collect::<Vec<_>>(),
            vec![false, true, false, true, false]
        );
        assert_eq!(
            all.not_mask().bits().collect::<Vec<_>>(),
            vec![false, false, false, false, false]
        );
        assert_eq!(
            all.slice(1, 3).bits().collect::<Vec<_>>(),
            vec![true, true, true]
        );
        assert_eq!(
            all.concat(&ValidityMask::all_valid(2)),
            ValidityMask::all_valid(7)
        );
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
    fn validity_mask_count_invalid_matches_complement() {
        let mask = ValidityMask::from_values(&[
            Scalar::Int64(1),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(2),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
        ]);
        assert_eq!(mask.count_valid(), 3);
        assert_eq!(mask.count_invalid(), 2);
        assert_eq!(mask.count_valid() + mask.count_invalid(), mask.len());
    }

    #[test]
    fn validity_mask_any_and_all() {
        let all_set = ValidityMask::all_valid(4);
        assert!(all_set.any());
        assert!(all_set.all());

        let none_set = ValidityMask::all_invalid(4);
        assert!(!none_set.any());
        assert!(!none_set.all());

        let mixed = ValidityMask::from_values(&[Scalar::Int64(1), Scalar::Null(NullKind::NaN)]);
        assert!(mixed.any());
        assert!(!mixed.all());

        let empty = ValidityMask::all_invalid(0);
        assert!(!empty.any());
        assert!(empty.all()); // vacuously true
    }

    #[test]
    fn validity_mask_xor_finds_differences() {
        let a = ValidityMask::from_values(&[
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(4),
        ]);
        let b = ValidityMask::from_values(&[
            Scalar::Int64(1),
            Scalar::Null(NullKind::NaN),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(4),
        ]);
        let diff = a.xor_mask(&b);
        assert_eq!(diff.len(), 4);
        // position 0: both valid → 0
        // position 1: a valid, b invalid → 1
        // position 2: both invalid → 0
        // position 3: both valid → 0
        assert!(!diff.get(0));
        assert!(diff.get(1));
        assert!(!diff.get(2));
        assert!(!diff.get(3));
    }

    #[test]
    fn validity_mask_slice_extracts_range() {
        let mask = ValidityMask::from_values(&[
            Scalar::Int64(1),            // valid
            Scalar::Null(NullKind::NaN), // invalid
            Scalar::Int64(3),            // valid
            Scalar::Int64(4),            // valid
            Scalar::Null(NullKind::NaN), // invalid
        ]);
        let sub = mask.slice(1, 3);
        assert_eq!(sub.len(), 3);
        assert!(!sub.get(0));
        assert!(sub.get(1));
        assert!(sub.get(2));
    }

    #[test]
    fn validity_mask_slice_past_end_clamps() {
        let mask = ValidityMask::all_valid(3);
        let sub = mask.slice(2, 10);
        assert_eq!(sub.len(), 1);
        assert!(sub.get(0));

        let empty = mask.slice(100, 5);
        assert!(empty.is_empty());
    }

    #[test]
    fn validity_mask_concat_appends() {
        let a = ValidityMask::from_values(&[Scalar::Int64(1), Scalar::Null(NullKind::NaN)]);
        let b = ValidityMask::from_values(&[Scalar::Int64(2), Scalar::Int64(3)]);
        let merged = a.concat(&b);
        assert_eq!(merged.len(), 4);
        assert!(merged.get(0));
        assert!(!merged.get(1));
        assert!(merged.get(2));
        assert!(merged.get(3));
    }

    #[test]
    fn validity_mask_first_last_valid() {
        let mask = ValidityMask::from_values(&[
            Scalar::Null(NullKind::NaN),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Null(NullKind::NaN),
        ]);
        assert_eq!(mask.first_valid(), Some(2));
        assert_eq!(mask.last_valid(), Some(3));

        let none_set = ValidityMask::all_invalid(3);
        assert_eq!(none_set.first_valid(), None);
        assert_eq!(none_set.last_valid(), None);
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
    fn column_data_interval_roundtrip_and_column_uniques_5g5uj() {
        let first = Interval::new(0.0, 1.0, IntervalClosed::Right);
        let second = Interval::new(1.0, 2.0, IntervalClosed::Right);
        let values = vec![
            Scalar::Interval(first),
            Scalar::Null(NullKind::Null),
            Scalar::Interval(second),
            Scalar::Interval(first),
        ];
        let validity = ValidityMask::from_values(&values);
        let data = super::ColumnData::from_scalars(&values, DType::Interval);
        assert_eq!(data.len(), 4);
        let back = data.to_scalars(DType::Interval, &validity);
        assert_eq!(back[0], Scalar::Interval(first));
        assert!(back[1].is_missing());
        assert_eq!(back[2], Scalar::Interval(second));
        assert_eq!(back[3], Scalar::Interval(first));

        let column = Column::new(DType::Interval, values).expect("interval column");
        assert_eq!(column.dtype(), DType::Interval);
        assert!(column.has_duplicates());
        let uniques = column.unique().expect("unique intervals");
        assert_eq!(
            uniques.values(),
            &[Scalar::Interval(first), Scalar::Interval(second)]
        );
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
    fn vectorized_binary_all_valid_keeps_typed_output_lazy() {
        let left = Column::from_f64_values(vec![1.0, 2.0, 3.0]);
        let right = Column::from_f64_values(vec![10.0, 20.0, 30.0]);

        let result = left.binary_numeric(&right, ArithmeticOp::Add).expect("add");

        assert!(result.validity().all());
        assert_eq!(result.as_f64_slice(), Some([11.0, 22.0, 33.0].as_slice()));
        assert!(matches!(
            &result.values,
            ScalarValues::LazyAllValidFloat64 { values, .. } if values.get().is_none()
        ));
    }

    #[test]
    fn vectorized_binary_operation_nan_matches_scalar_validity() {
        let left = Column::from_f64_values(vec![f64::INFINITY]);
        let right = Column::from_f64_values(vec![f64::INFINITY]);

        let result = left.binary_numeric(&right, ArithmeticOp::Sub).expect("sub");

        assert!(!result.validity().get(0));
        assert!(matches!(result.values()[0], Scalar::Float64(v) if v.is_nan()));
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
    fn aligned_binary_f64_matches_reindex_then_binary_numeric() {
        let left = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(f64::NAN),
                Scalar::Float64(3.5),
            ],
        )
        .expect("left");
        let right = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .expect("right");
        let left_positions = [Some(2), None, Some(1), Some(0)];
        let right_positions = [None, Some(0), Some(2), Some(1)];

        let expected_left = left
            .reindex_by_positions(&left_positions)
            .expect("left reindex");
        let expected_right = right
            .reindex_by_positions(&right_positions)
            .expect("right reindex");
        let expected = expected_left
            .binary_numeric(&expected_right, ArithmeticOp::Add)
            .expect("generic add");
        let actual = left
            .aligned_binary_f64(&right, &left_positions, &right_positions, ArithmeticOp::Add)
            .expect("aligned add");

        assert_eq!(actual.dtype(), expected.dtype());
        assert_eq!(actual.values(), expected.values());
        assert_eq!(actual.validity().len(), expected.validity().len());
        for idx in 0..actual.len() {
            assert_eq!(actual.validity().get(idx), expected.validity().get(idx));
        }
    }

    #[test]
    fn aligned_binary_f64_all_valid_keeps_typed_output_lazy() {
        let left = Column::from_f64_values(vec![1.0, 2.0, 3.0]);
        let right = Column::from_f64_values(vec![10.0, 20.0, 30.0]);
        let left_positions = [Some(0), Some(1), Some(2)];
        let right_positions = [Some(0), Some(1), Some(2)];

        let actual = left
            .aligned_binary_f64(&right, &left_positions, &right_positions, ArithmeticOp::Add)
            .expect("aligned add");

        assert!(actual.validity().all());
        assert_eq!(actual.as_f64_slice(), Some([11.0, 22.0, 33.0].as_slice()));
        assert!(matches!(
            &actual.values,
            ScalarValues::LazyAllValidFloat64 { values, .. } if values.get().is_none()
        ));
    }

    #[test]
    fn aligned_binary_f64_nullable_gaps_keep_typed_output_lazy() {
        let left = Column::from_f64_values(vec![1.0, 2.0, 3.0]);
        let right = Column::from_f64_values(vec![10.0, 20.0, 30.0]);
        let left_positions = [Some(0), Some(1), Some(2), None];
        let right_positions = [None, Some(0), Some(1), Some(2)];

        let expected_left = left
            .reindex_by_positions(&left_positions)
            .expect("left reindex");
        let expected_right = right
            .reindex_by_positions(&right_positions)
            .expect("right reindex");
        let expected = expected_left
            .binary_numeric(&expected_right, ArithmeticOp::Add)
            .expect("generic add");
        let actual = left
            .aligned_binary_f64(&right, &left_positions, &right_positions, ArithmeticOp::Add)
            .expect("aligned add");

        assert_eq!(actual.dtype(), expected.dtype());
        assert_eq!(actual.validity(), expected.validity());
        assert!(matches!(
            &actual.values,
            ScalarValues::LazyNullableFloat64 { values, .. } if values.get().is_none()
        ));
        assert_eq!(actual.values(), expected.values());
    }

    #[test]
    fn aligned_binary_f64_int64_unit_ranges_matches_position_alignment() {
        let left = Column::from_f64_values(vec![1.0, 2.0, 3.0]);
        let right = Column::from_f64_values(vec![10.0, 20.0, 30.0]);
        let left_positions = [Some(0), Some(1), Some(2), None];
        let right_positions = [None, Some(0), Some(1), Some(2)];

        let expected = left
            .aligned_binary_f64(&right, &left_positions, &right_positions, ArithmeticOp::Add)
            .expect("position aligned add");
        let actual = left
            .aligned_binary_f64_int64_unit_ranges(&right, (0, 2), (1, 3), (0, 3), ArithmeticOp::Add)
            .expect("unit range aligned add");

        assert_eq!(actual.dtype(), expected.dtype());
        assert_eq!(actual.validity(), expected.validity());
        assert!(matches!(
            &actual.values,
            ScalarValues::LazyNullableFloat64 { values, .. } if values.get().is_none()
        ));
        assert_eq!(actual.values(), expected.values());
    }

    #[test]
    fn aligned_binary_f64_operation_nan_keeps_float_nan_materialization() {
        let left = Column::from_f64_values(vec![f64::INFINITY]);
        let right = Column::from_f64_values(vec![f64::INFINITY]);
        let positions = [Some(0)];

        let actual = left
            .aligned_binary_f64(&right, &positions, &positions, ArithmeticOp::Sub)
            .expect("aligned sub");

        assert!(!actual.validity().get(0));
        assert!(matches!(
            &actual.values,
            ScalarValues::LazyNullableFloat64 { values, .. } if values.get().is_none()
        ));
        assert!(matches!(actual.values()[0], Scalar::Float64(value) if value.is_nan()));
    }

    #[test]
    fn apply_f64_slices_matches_fn_pointer_per_element_f64simd() {
        // The monomorphized slice op must be bit-for-bit identical to the
        // per-element fn pointer across every op and tricky operand
        // (NaN/inf/-0.0/zero divisor/negative base). Compared via raw bits so
        // NaN payloads must also match.
        let vals = [
            0.0_f64,
            -0.0,
            1.0,
            -1.0,
            2.5,
            -3.0,
            4.0,
            0.5,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            1e300,
            -1e-300,
        ];
        let a: Vec<f64> = vals.to_vec();
        for op in [
            ArithmeticOp::Add,
            ArithmeticOp::Sub,
            ArithmeticOp::Mul,
            ArithmeticOp::Div,
            ArithmeticOp::Mod,
            ArithmeticOp::Pow,
            ArithmeticOp::FloorDiv,
        ] {
            for shift in 0..vals.len() {
                let b: Vec<f64> = (0..vals.len())
                    .map(|i| vals[(i + shift) % vals.len()])
                    .collect();
                let got = super::apply_f64_slices(op, &a, &b);
                let apply = super::binary_f64_apply(op);
                let expected: Vec<f64> = a.iter().zip(&b).map(|(x, y)| apply(*x, *y)).collect();
                for i in 0..a.len() {
                    assert_eq!(
                        got[i].to_bits(),
                        expected[i].to_bits(),
                        "op={op:?} a={} b={}",
                        a[i],
                        b[i]
                    );
                }
            }
        }
    }

    #[test]
    fn aligned_binary_f64_same_positions_matches_identity_alignment() {
        let left = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(f64::NAN),
                Scalar::Float64(3.0),
            ],
        )
        .expect("left");
        let right = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Null(NullKind::NaN),
            ],
        )
        .expect("right");
        let positions = [Some(0), Some(1), Some(2)];

        let expected = left
            .aligned_binary_f64(&right, &positions, &positions, ArithmeticOp::Add)
            .expect("identity aligned add");
        let actual = left
            .aligned_binary_f64_same_positions(&right, ArithmeticOp::Add)
            .expect("same-position add");

        assert_eq!(actual.dtype(), expected.dtype());
        assert_eq!(actual.values(), expected.values());
        for idx in 0..actual.len() {
            assert_eq!(actual.validity().get(idx), expected.validity().get(idx));
        }
    }

    #[test]
    fn aligned_binary_f64_borrows_lazy_float64_clone_data() {
        let left = Column::from_f64_values(vec![1.0, f64::NAN, 4.0]).clone();
        let right = Column::from_f64_values(vec![10.0, 20.0, 30.0]).clone();

        assert!(left.data.is_none());
        assert!(right.data.is_none());
        assert!(matches!(
            &left.values,
            ScalarValues::LazyAllValidFloat64 { values, .. } if values.get().is_none()
        ));
        assert!(matches!(
            &right.values,
            ScalarValues::LazyAllValidFloat64 { values, .. } if values.get().is_none()
        ));

        let left_positions = [Some(0), Some(1), Some(2), None];
        let right_positions = [Some(2), Some(1), None, Some(0)];
        let actual = left
            .aligned_binary_f64(&right, &left_positions, &right_positions, ArithmeticOp::Add)
            .expect("aligned add");

        assert_eq!(
            actual.values(),
            &[
                Scalar::Float64(31.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ]
        );
        if let ScalarValues::LazyAllValidFloat64 { values, .. } = &left.values {
            assert!(values.get().is_none());
        }
        if let ScalarValues::LazyAllValidFloat64 { values, .. } = &right.values {
            assert!(values.get().is_none());
        }
    }

    #[test]
    fn from_f64_values_marks_nan_missing_like_scalar_path() {
        // br-frankenpandas-jyhf7: typed ingestion must treat NaN as missing,
        // matching Column::new(Float64, scalars). Otherwise a NaN-bearing column
        // claims all-valid and as_f64_slice leaks the NaN as a real value.
        let typed = Column::from_f64_values(vec![1.0, f64::NAN, 3.0, f64::NAN]);
        let scalar = Column::new(
            DType::Float64,
            vec![
                Scalar::Float64(1.0),
                Scalar::Float64(f64::NAN),
                Scalar::Float64(3.0),
                Scalar::Float64(f64::NAN),
            ],
        )
        .expect("scalar col");

        // Per-position validity agrees with the Scalar path.
        for idx in 0..typed.len() {
            assert_eq!(
                typed.validity().get(idx),
                scalar.validity().get(idx),
                "validity mismatch at {idx}"
            );
        }
        assert!(typed.validity().get(0));
        assert!(!typed.validity().get(1));
        assert!(typed.validity().get(2));
        assert!(!typed.validity().get(3));
        assert_eq!(typed.validity().count_valid(), 2);

        // A NaN-bearing column must NOT expose its raw f64 slice (the typed
        // fast path is only valid when every value is present).
        assert!(typed.as_f64_slice().is_none());

        // No-NaN columns keep the all-valid fast path and expose the slice.
        let clean = Column::from_f64_values(vec![1.0, 2.0, 3.0]);
        assert!(clean.validity().all());
        assert_eq!(clean.as_f64_slice(), Some([1.0, 2.0, 3.0].as_slice()));
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
    fn pandas_arithmetic_aliases_match_binary_numeric() {
        let left = Column::from_values(vec![Scalar::Float64(10.0)]).expect("left");
        let right = Column::from_values(vec![Scalar::Float64(3.0)]).expect("right");

        assert_eq!(
            left.add(&right).expect("add"),
            left.binary_numeric(&right, ArithmeticOp::Add).expect("add")
        );
        assert_eq!(
            left.sub(&right).expect("sub"),
            left.binary_numeric(&right, ArithmeticOp::Sub).expect("sub")
        );
        assert_eq!(
            left.mul(&right).expect("mul"),
            left.binary_numeric(&right, ArithmeticOp::Mul).expect("mul")
        );
        assert_eq!(
            left.div(&right).expect("div"),
            left.binary_numeric(&right, ArithmeticOp::Div).expect("div")
        );
        assert_eq!(
            left.divide(&right).expect("divide"),
            left.div(&right).expect("div")
        );
    }

    #[test]
    fn remaining_pandas_arithmetic_aliases_match_binary_numeric() {
        let left = Column::from_values(vec![Scalar::Float64(10.0)]).expect("left");
        let right = Column::from_values(vec![Scalar::Float64(3.0)]).expect("right");

        assert_eq!(
            left.subtract(&right).expect("subtract"),
            left.sub(&right).expect("sub")
        );
        assert_eq!(
            left.multiply(&right).expect("multiply"),
            left.mul(&right).expect("mul")
        );
        assert_eq!(
            left.truediv(&right).expect("truediv"),
            left.div(&right).expect("div")
        );
        assert_eq!(
            left.floordiv(&right).expect("floordiv"),
            left.binary_numeric(&right, ArithmeticOp::FloorDiv)
                .expect("floordiv")
        );
        assert_eq!(
            left.r#mod(&right).expect("mod"),
            left.binary_numeric(&right, ArithmeticOp::Mod).expect("mod")
        );
        assert_eq!(
            left.pow(&right).expect("pow"),
            left.binary_numeric(&right, ArithmeticOp::Pow).expect("pow")
        );
    }

    #[test]
    fn pandas_reverse_arithmetic_aliases_swap_operands() {
        let series = Column::from_values(vec![Scalar::Float64(10.0)]).expect("series");
        let other = Column::from_values(vec![Scalar::Float64(3.0)]).expect("other");

        assert_eq!(
            series.radd(&other).expect("radd"),
            other
                .binary_numeric(&series, ArithmeticOp::Add)
                .expect("add")
        );
        assert_eq!(
            series.rsub(&other).expect("rsub"),
            other
                .binary_numeric(&series, ArithmeticOp::Sub)
                .expect("sub")
        );
        assert_eq!(
            series.rmul(&other).expect("rmul"),
            other
                .binary_numeric(&series, ArithmeticOp::Mul)
                .expect("mul")
        );
        assert_eq!(
            series.rdiv(&other).expect("rdiv"),
            other
                .binary_numeric(&series, ArithmeticOp::Div)
                .expect("div")
        );
        assert_eq!(
            series.rtruediv(&other).expect("rtruediv"),
            series.rdiv(&other).expect("rdiv")
        );
        assert_eq!(
            series.rfloordiv(&other).expect("rfloordiv"),
            other
                .binary_numeric(&series, ArithmeticOp::FloorDiv)
                .expect("floordiv")
        );
        assert_eq!(
            series.rmod(&other).expect("rmod"),
            other
                .binary_numeric(&series, ArithmeticOp::Mod)
                .expect("mod")
        );
        assert_eq!(
            series.rpow(&other).expect("rpow"),
            other
                .binary_numeric(&series, ArithmeticOp::Pow)
                .expect("pow")
        );
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
        assert!(matches!(modulo.values()[2], Scalar::Float64(v) if (v - 1.0).abs() < 1e-10));

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
    fn int_pow_stays_int64_and_negative_exponent_raises_3w0xn() {
        // br-frankenpandas-3w0xn: int ** int stays int64 (numpy/pandas: 2 ** 3
        // == 8, not 8.0), and a negative integer exponent raises.
        let base = Column::from_values(vec![Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(10)])
            .expect("base");
        let exp = Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(2), Scalar::Int64(2)])
            .expect("exp");
        let pow = base
            .binary_numeric(&exp, ArithmeticOp::Pow)
            .expect("int pow");
        assert_eq!(pow.dtype(), DType::Int64);
        assert_eq!(pow.values()[0], Scalar::Int64(8));
        assert_eq!(pow.values()[1], Scalar::Int64(9));
        assert_eq!(pow.values()[2], Scalar::Int64(100));

        // Negative integer exponent raises (numpy ValueError analogue).
        let neg_exp =
            Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(-1), Scalar::Int64(2)])
                .expect("neg_exp");
        let err = base
            .binary_numeric(&neg_exp, ArithmeticOp::Pow)
            .expect_err("negative integer power must raise");
        assert!(matches!(err, ColumnError::NegativeIntegerPower));

        // A float operand promotes the whole op to Float64.
        let exp_f = Column::from_values(vec![
            Scalar::Float64(3.0),
            Scalar::Float64(2.0),
            Scalar::Float64(2.0),
        ])
        .expect("exp_f");
        let pow_f = base
            .binary_numeric(&exp_f, ArithmeticOp::Pow)
            .expect("mixed int/float pow");
        assert_eq!(pow_f.dtype(), DType::Float64);
        assert!(matches!(pow_f.values()[0], Scalar::Float64(v) if (v - 8.0).abs() < 1e-10));
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
    fn int64_mod_floordiv_match_pandas_negative_operand_signs() {
        let left = Column::from_values(vec![
            Scalar::Int64(7),
            Scalar::Int64(-7),
            Scalar::Int64(-7),
            Scalar::Int64(7),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Int64(-3),
            Scalar::Int64(3),
            Scalar::Int64(-3),
            Scalar::Int64(3),
        ])
        .expect("right");

        let modulo = left.binary_numeric(&right, ArithmeticOp::Mod).expect("mod");
        assert_eq!(modulo.dtype(), DType::Int64);
        assert_eq!(
            modulo.values(),
            &[
                Scalar::Int64(-2),
                Scalar::Int64(2),
                Scalar::Int64(-1),
                Scalar::Int64(1)
            ]
        );

        let floordiv = left
            .binary_numeric(&right, ArithmeticOp::FloorDiv)
            .expect("floordiv");
        assert_eq!(floordiv.dtype(), DType::Int64);
        assert_eq!(
            floordiv.values(),
            &[
                Scalar::Int64(-3),
                Scalar::Int64(-3),
                Scalar::Int64(2),
                Scalar::Int64(2)
            ]
        );
    }

    #[test]
    fn float64_mod_floordiv_match_pandas_negative_operand_signs() {
        let left = Column::from_values(vec![
            Scalar::Float64(7.0),
            Scalar::Float64(-7.0),
            Scalar::Float64(-7.0),
            Scalar::Float64(7.0),
        ])
        .expect("left");
        let right = Column::from_values(vec![
            Scalar::Float64(-3.0),
            Scalar::Float64(3.0),
            Scalar::Float64(-3.0),
            Scalar::Float64(3.0),
        ])
        .expect("right");

        let modulo = left.binary_numeric(&right, ArithmeticOp::Mod).expect("mod");
        assert_eq!(modulo.dtype(), DType::Float64);
        assert!(matches!(modulo.values()[0], Scalar::Float64(v) if (v + 2.0).abs() < 1e-10));
        assert!(matches!(modulo.values()[1], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
        assert!(matches!(modulo.values()[2], Scalar::Float64(v) if (v + 1.0).abs() < 1e-10));
        assert!(matches!(modulo.values()[3], Scalar::Float64(v) if (v - 1.0).abs() < 1e-10));

        let floordiv = left
            .binary_numeric(&right, ArithmeticOp::FloorDiv)
            .expect("floordiv");
        assert_eq!(floordiv.dtype(), DType::Float64);
        assert!(matches!(floordiv.values()[0], Scalar::Float64(v) if (v + 3.0).abs() < 1e-10));
        assert!(matches!(floordiv.values()[1], Scalar::Float64(v) if (v + 3.0).abs() < 1e-10));
        assert!(matches!(floordiv.values()[2], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
        assert!(matches!(floordiv.values()[3], Scalar::Float64(v) if (v - 2.0).abs() < 1e-10));
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
        use fp_types::Scalar;

        use super::super::*;

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
        use fp_types::{NullKind, Scalar};

        use super::super::*;

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
        fn pandas_comparison_aliases_match_binary_comparison() {
            let left = Column::from_values(vec![Scalar::Float64(5.0)]).expect("left");
            let right = Column::from_values(vec![Scalar::Float64(3.0)]).expect("right");

            assert_eq!(
                left.eq(&right).expect("eq"),
                left.binary_comparison(&right, ComparisonOp::Eq)
                    .expect("eq")
            );
            assert_eq!(
                left.ne(&right).expect("ne"),
                left.binary_comparison(&right, ComparisonOp::Ne)
                    .expect("ne")
            );
            assert_eq!(
                left.lt(&right).expect("lt"),
                left.binary_comparison(&right, ComparisonOp::Lt)
                    .expect("lt")
            );
            assert_eq!(
                left.le(&right).expect("le"),
                left.binary_comparison(&right, ComparisonOp::Le)
                    .expect("le")
            );
            assert_eq!(
                left.gt(&right).expect("gt"),
                left.binary_comparison(&right, ComparisonOp::Gt)
                    .expect("gt")
            );
            assert_eq!(
                left.ge(&right).expect("ge"),
                left.binary_comparison(&right, ComparisonOp::Ge)
                    .expect("ge")
            );
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
        fn filter_by_mask_float64_typed_path_matches_scalar() {
            // The Float64 typed gather (as_f64_slice + from_f64_values) must be
            // bit-identical to the Scalar clone path: same selected values, same
            // order, all-valid result.
            let col = Column::from_f64_values(vec![1.5, -0.0, 2.5, f64::INFINITY, 0.0]);
            let mask = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Null(NullKind::Null), // missing -> not selected
            ])
            .expect("mask");
            let result = col.filter_by_mask(&mask).expect("filter");
            assert_eq!(result.dtype(), DType::Float64);
            assert_eq!(
                result.values(),
                &[
                    Scalar::Float64(1.5),
                    Scalar::Float64(-0.0),
                    Scalar::Float64(f64::INFINITY),
                ]
            );
        }

        #[test]
        fn compare_scalar_typed_path_matches_scalar_compare() {
            // Isomorphism proof for br-frankenpandas-2kpwa: the typed f64/i64
            // compare_scalar fast paths must be bit-identical to the per-element
            // scalar_compare reference for every op and operand combination.
            let f64_vals = vec![1.5f64, -0.0, 0.0, 2.5, -3.0, f64::INFINITY, 100.0];
            let i64_vals = vec![1i64, -2, 0, 5, 100, -7];
            let ops = [
                ComparisonOp::Gt,
                ComparisonOp::Lt,
                ComparisonOp::Eq,
                ComparisonOp::Ne,
                ComparisonOp::Ge,
                ComparisonOp::Le,
            ];
            for op in ops {
                // Float64 column vs Float64 scalar.
                for &probe in &[0.0f64, 1.5, 2.5, -3.0, f64::INFINITY] {
                    let got = Column::from_f64_values(f64_vals.clone())
                        .compare_scalar(&Scalar::Float64(probe), op)
                        .expect("f64 cmp");
                    let expected: Vec<Scalar> = f64_vals
                        .iter()
                        .map(|&v| {
                            Scalar::Bool(
                                scalar_compare(&Scalar::Float64(v), &Scalar::Float64(probe), op)
                                    .unwrap(),
                            )
                        })
                        .collect();
                    assert_eq!(
                        got.values(),
                        expected.as_slice(),
                        "f64 op {op:?} probe {probe}"
                    );
                }
                // Float64 column vs Int64 scalar (f64-promotion branch).
                let got = Column::from_f64_values(f64_vals.clone())
                    .compare_scalar(&Scalar::Int64(2), op)
                    .expect("f64-vs-i64 cmp");
                let expected: Vec<Scalar> = f64_vals
                    .iter()
                    .map(|&v| {
                        Scalar::Bool(
                            scalar_compare(&Scalar::Float64(v), &Scalar::Int64(2), op).unwrap(),
                        )
                    })
                    .collect();
                assert_eq!(got.values(), expected.as_slice(), "f64-vs-i64 op {op:?}");
                // Int64 column vs Int64 scalar (both-Int64 branch).
                let got = Column::from_i64_values(i64_vals.clone())
                    .compare_scalar(&Scalar::Int64(0), op)
                    .expect("i64 cmp");
                let expected: Vec<Scalar> = i64_vals
                    .iter()
                    .map(|&v| {
                        Scalar::Bool(
                            scalar_compare(&Scalar::Int64(v), &Scalar::Int64(0), op).unwrap(),
                        )
                    })
                    .collect();
                assert_eq!(got.values(), expected.as_slice(), "i64 op {op:?}");
            }
        }

        #[test]
        #[ignore = "perf timing harness, run with --ignored"]
        fn compare_scalar_typed_vs_aos_timing() {
            use std::time::Instant;
            let n = 5_000_000usize;
            let raw: Vec<f64> = (0..n).map(|i| (i % 1000) as f64 - 500.0).collect();
            let scalars: Vec<Scalar> = raw.iter().map(|&v| Scalar::Float64(v)).collect();
            let probe = Scalar::Float64(0.0);
            let op = ComparisonOp::Gt;

            // AoS reference: per-element scalar_compare + Scalar::Bool alloc.
            let t = Instant::now();
            let aos: Vec<Scalar> = scalars
                .iter()
                .map(|v| Scalar::Bool(scalar_compare(v, &probe, op).unwrap()))
                .collect();
            let aos_ns = t.elapsed().as_nanos();
            std::hint::black_box(&aos);

            // Typed path through compare_scalar (as_f64_slice -> from_bool_values).
            let col = Column::from_f64_values(raw.clone());
            let t = Instant::now();
            let typed = col.compare_scalar(&probe, op).expect("typed cmp");
            let typed_ns = t.elapsed().as_nanos();
            std::hint::black_box(&typed);

            assert_eq!(typed.values(), aos.as_slice(), "typed must match AoS");
            let ratio = aos_ns as f64 / typed_ns as f64;
            println!(
                "compare_scalar Gt n={n}: AoS {aos_ns}ns  typed {typed_ns}ns  Score={ratio:.2}x"
            );
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

    mod iter_and_predicates {
        use fp_types::NullKind;

        use super::*;

        #[test]
        fn iter_values_preserves_order() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let collected: Vec<_> = col.iter_values().cloned().collect();
            assert_eq!(collected, col.values());
        }

        #[test]
        fn to_vec_returns_owned_clone() {
            let col = Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(6)]).expect("col");
            let v = col.to_vec();
            assert_eq!(v, vec![Scalar::Int64(5), Scalar::Int64(6)]);
            // Column still owns its values; to_vec was a clone.
            assert_eq!(col.len(), 2);
        }

        #[test]
        fn copy_returns_independent_clone() {
            let col = Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(6)]).expect("col");
            let copied = col.copy();
            let viewed = col.view();
            let transposed = col.transpose();
            assert_eq!(copied, col);
            assert_eq!(viewed, col);
            assert_eq!(transposed, col);
            assert_eq!(col.t(), transposed);
            assert_eq!(col.T(), transposed);
            assert_ne!(copied.values().as_ptr(), col.values().as_ptr());
            assert_ne!(viewed.values().as_ptr(), col.values().as_ptr());
            assert_ne!(transposed.values().as_ptr(), col.values().as_ptr());
        }

        #[test]
        fn item_extracts_single_value_and_rejects_other_lengths() {
            let single = Column::from_values(vec![Scalar::Int64(5)]).expect("col");
            assert_eq!(single.item(), Ok(Scalar::Int64(5)));

            let empty = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert_eq!(
                empty.item(),
                Err(crate::ColumnError::InvalidLength {
                    operation: "item()",
                    expected: 1,
                    actual: 0,
                })
            );

            let multi = Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(6)]).expect("col");
            assert_eq!(
                multi.item(),
                Err(crate::ColumnError::InvalidLength {
                    operation: "item()",
                    expected: 1,
                    actual: 2,
                })
            );
        }

        #[test]
        fn has_any_missing_detects_null() {
            let populated =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            assert!(!populated.has_any_missing());
            assert_eq!(populated.hasnans(), populated.has_any_missing());
            assert_eq!(populated.nbytes(), populated.memory_usage(false));

            let with_null =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)])
                    .expect("col");
            assert!(with_null.has_any_missing());
            assert_eq!(with_null.hasnans(), with_null.has_any_missing());
            assert_eq!(with_null.nbytes(), with_null.memory_usage(false));
        }

        #[test]
        fn all_missing_empty_is_true() {
            let empty = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert!(empty.all_missing());

            let all_null = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");
            assert!(all_null.all_missing());

            let mixed = Column::from_values(vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)])
                .expect("col");
            assert!(!mixed.all_missing());
        }

        #[test]
        fn apply_bool_positive_predicate() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
            ])
            .expect("col");
            let even = col
                .apply_bool(|v| v.to_f64().map(|f| f as i64 % 2 == 0).unwrap_or(false))
                .expect("apply_bool");
            assert_eq!(even.dtype(), DType::Bool);
            assert_eq!(even.values()[0], Scalar::Bool(false));
            assert_eq!(even.values()[1], Scalar::Bool(true));
            assert_eq!(even.values()[2], Scalar::Bool(false));
            assert_eq!(even.values()[3], Scalar::Bool(true));
        }

        #[test]
        fn first_and_last_return_endpoints() {
            let col = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
            ])
            .expect("col");
            assert_eq!(col.first(), Some(&Scalar::Int64(10)));
            assert_eq!(col.last(), Some(&Scalar::Int64(30)));

            let empty = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert_eq!(empty.first(), None);
            assert_eq!(empty.last(), None);
        }

        #[test]
        fn count_matching_ignores_missing_and_mismatches() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(4),
                Scalar::Int64(6),
            ])
            .expect("col");
            let evens =
                col.count_matching(|v| v.to_f64().map(|f| f as i64 % 2 == 0).unwrap_or(false));
            assert_eq!(evens, 3); // 2, 4, 6 — missing not counted.
        }

        #[test]
        fn zip_with_elementwise_combine() {
            let a = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                .expect("a");
            let b = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
            ])
            .expect("b");
            let sum = a
                .zip_with(&b, |l, r| match (l.to_f64(), r.to_f64()) {
                    (Ok(lf), Ok(rf)) => Scalar::Float64(lf + rf),
                    _ => Scalar::Null(NullKind::NaN),
                })
                .expect("zip_with");
            assert_eq!(sum.values()[0], Scalar::Float64(11.0));
            assert_eq!(sum.values()[1], Scalar::Float64(22.0));
            assert_eq!(sum.values()[2], Scalar::Float64(33.0));
        }

        #[test]
        fn zip_with_length_mismatch_errors() {
            let a = Column::from_values(vec![Scalar::Int64(1)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("b");
            assert!(a.zip_with(&b, |l, _| l.clone()).is_err());
        }

        #[test]
        fn iter_enumerate_yields_positions() {
            let col = Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).expect("col");
            let collected: Vec<_> = col.iter_enumerate().map(|(i, v)| (i, v.clone())).collect();
            assert_eq!(
                collected,
                vec![(0, Scalar::Int64(10)), (1, Scalar::Int64(20))]
            );
        }

        #[test]
        fn apply_bool_missing_maps_to_false() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)])
                .expect("col");
            let result = col.apply_bool(|_| true).expect("apply_bool");
            assert_eq!(result.values()[0], Scalar::Bool(true));
            // Missing input → false (per doc contract).
            assert_eq!(result.values()[1], Scalar::Bool(false));
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
        fn slice_huge_len_clamps_without_overflow() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let tail = col.slice(1, usize::MAX).expect("slice");
            assert_eq!(tail.values(), &[Scalar::Int64(2), Scalar::Int64(3)]);
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
        use fp_types::NullKind;

        use super::*;

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
        use fp_types::NullKind;

        use super::*;

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
        fn abs_bool_preserves_dtype() {
            let bool_col =
                Column::from_values(vec![Scalar::Bool(true), Scalar::Bool(false)]).expect("bool");
            let result = bool_col.abs().expect("abs");
            assert_eq!(result.dtype(), DType::Bool);
            assert_eq!(result.values(), &[Scalar::Bool(true), Scalar::Bool(false)]);
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
        fn round_int_negative_decimals_preserves_dtype() {
            let col = Column::from_values(vec![
                Scalar::Int64(15),
                Scalar::Int64(25),
                Scalar::Int64(35),
                Scalar::Int64(-15),
            ])
            .expect("col");
            let r = col.round(-1).expect("round");
            assert_eq!(r.dtype(), DType::Int64);
            assert_eq!(
                r.values(),
                &[
                    Scalar::Int64(20),
                    Scalar::Int64(20),
                    Scalar::Int64(40),
                    Scalar::Int64(-20)
                ]
            );
        }

        #[test]
        fn round_bool_is_noop() {
            let col =
                Column::from_values(vec![Scalar::Bool(true), Scalar::Bool(false)]).expect("col");
            let r = col.round(-2).expect("round");
            assert_eq!(r.dtype(), DType::Bool);
            assert_eq!(r.values(), col.values());
        }

        #[test]
        fn round_negative_decimals_rounds_left() {
            let col = Column::from_values(vec![Scalar::Float64(1234.0)]).expect("col");
            let r = col.round(-2).expect("round");
            assert_eq!(r.values()[0], Scalar::Float64(1200.0));
        }

        #[test]
        fn round_uses_pandas_half_even_ties() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.5),
                Scalar::Float64(2.5),
                Scalar::Float64(-1.5),
                Scalar::Float64(3.5),
            ])
            .expect("col");
            let r = col.round(0).expect("round");
            assert_eq!(
                r.values(),
                &[
                    Scalar::Float64(2.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(-2.0),
                    Scalar::Float64(4.0)
                ]
            );
        }

        #[test]
        fn round_negative_decimals_uses_half_even_ties() {
            let col = Column::from_values(vec![
                Scalar::Float64(15.0),
                Scalar::Float64(25.0),
                Scalar::Float64(35.0),
                Scalar::Float64(-15.0),
            ])
            .expect("col");
            let r = col.round(-1).expect("round");
            assert_eq!(
                r.values(),
                &[
                    Scalar::Float64(20.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(40.0),
                    Scalar::Float64(-20.0)
                ]
            );
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
        use fp_types::NullKind;

        use super::*;

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

        // Naive comparator reference (the pre-radix Scalar path) for isomorphism
        // proofs: rebuilds the sorted Scalar vec exactly as the old code did.
        fn scalar_sort_reference(values: &[Scalar], ascending: bool) -> Vec<Scalar> {
            let mut indexed: Vec<(usize, &Scalar)> = values.iter().enumerate().collect();
            indexed.sort_by(|a, b| crate::compare_scalars_na_last(a.1, b.1, ascending));
            indexed.into_iter().map(|(_, v)| v.clone()).collect()
        }

        #[test]
        fn radix_sort_matches_scalar_reference_i64_and_f64() {
            // Deterministic LCG covering negatives, zero, duplicates (tie
            // stability), and large magnitudes — the typed radix path must be
            // BIT-IDENTICAL to the stable Scalar comparator path, both orders.
            let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state
            };
            for trial in 0..200 {
                let n = (next() % 400) as usize + 1;
                let i64_vals: Vec<Scalar> = (0..n)
                    .map(|_| {
                        // Narrow range forces frequent ties; occasional wide value.
                        let r = next();
                        let v = if r % 7 == 0 {
                            r as i64 // full-width incl negatives via wraparound
                        } else {
                            (r % 11) as i64 - 5
                        };
                        Scalar::Int64(v)
                    })
                    .collect();
                let f64_vals: Vec<Scalar> = i64_vals
                    .iter()
                    .map(|s| match s {
                        Scalar::Int64(v) => {
                            // Map into floats incl negatives, zero, fractional ties.
                            let f = (*v as f64) / 4.0;
                            Scalar::Float64(if f == 0.0 { 0.0 } else { f })
                        }
                        _ => unreachable!(),
                    })
                    .collect();
                for (vals, label) in [(&i64_vals, "i64"), (&f64_vals, "f64")] {
                    let col = Column::from_values(vals.clone()).expect("col");
                    // Skip if any value became missing (NaN guard not exercised here).
                    assert!(
                        col.validity.all(),
                        "{label} trial {trial}: unexpected missing"
                    );
                    for ascending in [true, false] {
                        let got = col.sort_values(ascending).expect("sort").values().to_vec();
                        let want = scalar_sort_reference(vals, ascending);
                        assert_eq!(
                            got, want,
                            "{label} trial {trial} asc={ascending} sort mismatch"
                        );
                    }
                    // argsort (ascending) must reproduce the stable permutation.
                    let perm = col.argsort();
                    let via_perm: Vec<Scalar> = perm.iter().map(|&i| vals[i].clone()).collect();
                    assert_eq!(
                        via_perm,
                        scalar_sort_reference(vals, true),
                        "{label} trial {trial} argsort mismatch"
                    );
                }
            }
        }

        #[test]
        fn contiguous_utf8_argsort_matches_scalar_reference() {
            let raw = ["bee", "alpha", "bee", "alphabet", "", "zulu"];
            let scalars: Vec<Scalar> = raw
                .iter()
                .map(|value| Scalar::Utf8((*value).to_owned()))
                .collect();
            let scalar_col = Column::from_values(scalars.clone()).expect("scalar col");
            let mut bytes = Vec::new();
            let mut offsets = Vec::with_capacity(raw.len() + 1);
            offsets.push(0);
            for value in raw {
                bytes.extend_from_slice(value.as_bytes());
                offsets.push(bytes.len());
            }
            let contiguous_col = Column::from_utf8_contiguous(bytes, offsets);

            assert_eq!(contiguous_col.argsort_with(true), vec![4, 1, 3, 0, 2, 5]);
            assert_eq!(contiguous_col.argsort_with(false), vec![5, 0, 2, 3, 1, 4]);
            for ascending in [true, false] {
                let got = contiguous_col
                    .sort_values(ascending)
                    .expect("contiguous sort")
                    .values()
                    .to_vec();
                let want = scalar_col
                    .sort_values(ascending)
                    .expect("scalar sort")
                    .values()
                    .to_vec();
                assert_eq!(got, want, "ascending={ascending}");
            }
        }

        #[test]
        fn contiguous_utf8_strict_witness_matches_byte_order_483i5() {
            fn contiguous(values: &[&str]) -> Column {
                let mut bytes = Vec::new();
                let mut offsets = Vec::with_capacity(values.len() + 1);
                offsets.push(0);
                for value in values {
                    bytes.extend_from_slice(value.as_bytes());
                    offsets.push(bytes.len());
                }
                Column::from_utf8_contiguous(bytes, offsets)
            }

            assert!(
                contiguous(&["a", "b", "c"])
                    .as_strictly_increasing_utf8_contiguous()
                    .is_some()
            );
            assert!(
                contiguous(&[])
                    .as_strictly_increasing_utf8_contiguous()
                    .is_some()
            );
            assert!(
                contiguous(&["only"])
                    .as_strictly_increasing_utf8_contiguous()
                    .is_some()
            );
            assert!(
                contiguous(&["a", "a"])
                    .as_strictly_increasing_utf8_contiguous()
                    .is_none()
            );
            assert!(
                contiguous(&["b", "a"])
                    .as_strictly_increasing_utf8_contiguous()
                    .is_none()
            );

            let scalar_backed = Column::from_values(vec![
                Scalar::Utf8("a".to_owned()),
                Scalar::Utf8("b".to_owned()),
            ])
            .expect("scalar-backed utf8");
            assert!(
                scalar_backed
                    .as_strictly_increasing_utf8_contiguous()
                    .is_none()
            );
        }

        #[test]
        fn abs_typed_matches_scalar_reference() {
            // The typed abs fast path must be bit-identical to the Scalar loop
            // for all-valid Int64/Float64, incl i64::MIN (wrapping_abs) and
            // -0.0/large floats.
            let mut state: u64 = 0x2545_F491_4F6C_DD1D;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state
            };
            for trial in 0..150 {
                let n = (next() % 300) as usize + 1;
                let i64_vals: Vec<Scalar> = (0..n)
                    .map(|_| {
                        let r = next();
                        if r % 50 == 0 {
                            Scalar::Int64(i64::MIN)
                        } else {
                            Scalar::Int64((r % 2000) as i64 - 1000)
                        }
                    })
                    .collect();
                let f64_vals: Vec<Scalar> = i64_vals
                    .iter()
                    .map(|s| match s {
                        Scalar::Int64(v) => {
                            let f = if *v == i64::MIN {
                                -0.0
                            } else {
                                *v as f64 / 4.0
                            };
                            Scalar::Float64(f)
                        }
                        _ => unreachable!(),
                    })
                    .collect();
                for vals in [&i64_vals, &f64_vals] {
                    let col = Column::from_values(vals.clone()).expect("col");
                    let got = col.abs().expect("abs").values().to_vec();
                    let want: Vec<Scalar> = vals
                        .iter()
                        .map(|v| match v {
                            Scalar::Int64(x) => Scalar::Int64(x.wrapping_abs()),
                            Scalar::Float64(x) => Scalar::Float64(x.abs()),
                            other => other.clone(),
                        })
                        .collect();
                    // Float abs of -0.0 → 0.0; compare by bits for floats.
                    for (g, w) in got.iter().zip(&want) {
                        match (g, w) {
                            (Scalar::Float64(a), Scalar::Float64(b)) => {
                                assert_eq!(a.to_bits(), b.to_bits(), "trial {trial} float abs")
                            }
                            _ => assert_eq!(g, w, "trial {trial} abs"),
                        }
                    }
                }
            }
        }

        #[test]
        #[ignore = "timing benchmark; run with --ignored --nocapture on the rch VM"]
        fn abs_typed_timing_vs_scalar() {
            use std::time::Instant;
            let n = 5_000_000usize;
            let iters = 10;
            let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state
            };
            let data: Vec<f64> = (0..n)
                .map(|_| (next() % 2_000_000) as f64 - 1_000_000.0)
                .collect();
            let mk = || Column::from_f64_values(data.clone());

            let t0 = Instant::now();
            let mut chk = 0usize;
            for _ in 0..iters {
                chk ^= mk().abs().unwrap().len();
            }
            let typed = t0.elapsed();

            let t1 = Instant::now();
            let mut chk2 = 0usize;
            for _ in 0..iters {
                let col = mk();
                let out: Vec<Scalar> = col
                    .values()
                    .iter()
                    .map(|v| match v {
                        Scalar::Float64(x) => Scalar::Float64(x.abs()),
                        other => other.clone(),
                    })
                    .collect();
                chk2 ^= Column::new(DType::Float64, out).unwrap().len();
            }
            let scalar = t1.elapsed();
            let t2 = Instant::now();
            let mut sink = 0usize;
            for _ in 0..iters {
                sink ^= mk().len();
            }
            let build = t2.elapsed();
            let typed_op = typed.saturating_sub(build).as_secs_f64();
            let scalar_op = scalar.saturating_sub(build).as_secs_f64();
            eprintln!(
                "abs 5M f64 x{iters}: typed={typed:?} scalar={scalar:?} build={build:?} \
                 op-only ratio={:.2}x (full {:.2}x, chk {chk}/{chk2}/{sink})",
                scalar_op / typed_op,
                scalar.as_secs_f64() / typed.as_secs_f64()
            );
        }

        #[test]
        fn factorize_direct_address_matches_reference() {
            // Independent O(n^2) first-seen reference (linear position scan, no
            // hashing/direct-address) for bounded-range all-valid Int64. The
            // direct-address fast path must be bit-identical for codes AND
            // uniques, both sort modes (use_na_sentinel is moot — all valid).
            let mut state: u64 = 0x51A4_3C29_7E10_BB67;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state
            };
            for trial in 0..150 {
                let n = (next() % 300) as usize + 1;
                let data: Vec<i64> = (0..n).map(|_| (next() % 13) as i64 - 6).collect();

                for sort in [false, true] {
                    // Reference: first-seen codes via linear scan, then optional
                    // stable sort of uniques + code remap.
                    let mut uniques: Vec<i64> = Vec::new();
                    let mut codes: Vec<i64> = Vec::with_capacity(n);
                    for &v in &data {
                        match uniques.iter().position(|&u| u == v) {
                            Some(p) => codes.push(p as i64),
                            None => {
                                codes.push(uniques.len() as i64);
                                uniques.push(v);
                            }
                        }
                    }
                    if sort {
                        let mut order: Vec<usize> = (0..uniques.len()).collect();
                        order.sort_by(|&a, &b| uniques[a].cmp(&uniques[b]));
                        let mut remap = vec![0i64; uniques.len()];
                        let sorted: Vec<i64> = order
                            .iter()
                            .enumerate()
                            .map(|(new_pos, &orig)| {
                                remap[orig] = new_pos as i64;
                                uniques[orig]
                            })
                            .collect();
                        for c in &mut codes {
                            *c = remap[*c as usize];
                        }
                        uniques = sorted;
                    }

                    let col = Column::from_values(data.iter().map(|&v| Scalar::Int64(v)).collect())
                        .expect("col");
                    let (code_col, uniq_col) =
                        col.factorize_with_options(sort, true).expect("factorize");
                    let got_codes: Vec<i64> = code_col
                        .values()
                        .iter()
                        .filter_map(|v| match v {
                            Scalar::Int64(c) => Some(*c),
                            _ => None,
                        })
                        .collect();
                    let got_uniques: Vec<i64> = uniq_col
                        .values()
                        .iter()
                        .filter_map(|v| match v {
                            Scalar::Int64(c) => Some(*c),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(got_codes.len(), code_col.len(), "non-int code");
                    assert_eq!(got_uniques.len(), uniq_col.len(), "non-int unique");
                    assert_eq!(got_codes, codes, "trial {trial} sort={sort} codes");
                    assert_eq!(got_uniques, uniques, "trial {trial} sort={sort} uniques");
                }
            }
        }

        #[test]
        #[ignore = "timing benchmark; run with --ignored --nocapture on the rch VM"]
        fn factorize_direct_address_timing_vs_hashmap() {
            use std::{collections::HashMap, time::Instant};
            let n = 5_000_000usize;
            let iters = 10;
            for cardinality in [1_000u64, 2_000_000u64] {
                let mut state: u64 = 0x2468_ACE0_1357_9BDF ^ cardinality;
                let mut next = || {
                    state = state
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    state
                };
                let data: Vec<i64> = (0..n).map(|_| (next() % cardinality) as i64).collect();

                let col = Column::from_i64_values(data.clone());
                let t0 = Instant::now();
                let mut chk = 0i64;
                for _ in 0..iters {
                    let (codes, _u) = col.factorize_with_options(false, true).expect("da");
                    if let Scalar::Int64(c) = &codes.values()[n - 1] {
                        chk ^= *c;
                    }
                }
                let direct = t0.elapsed();

                // OLD: HashMap<i64,i64> first-seen code assignment over &[Scalar].
                let scalar_col =
                    Column::from_values(data.iter().map(|&v| Scalar::Int64(v)).collect())
                        .expect("col");
                let t1 = Instant::now();
                let mut chk2 = 0i64;
                for _ in 0..iters {
                    let mut map: HashMap<i64, i64> = HashMap::new();
                    let mut uniques = 0i64;
                    let mut last = 0i64;
                    for v in scalar_col.values() {
                        if let Scalar::Int64(i) = v {
                            let code = *map.entry(*i).or_insert_with(|| {
                                let c = uniques;
                                uniques += 1;
                                c
                            });
                            last = code;
                        }
                    }
                    chk2 ^= last;
                }
                let scalar = t1.elapsed();
                eprintln!(
                    "factorize 5M i64 card={cardinality} x{iters}: direct={direct:?} hashmap={scalar:?} ratio={:.2}x (chk {chk}/{chk2})",
                    scalar.as_secs_f64() / direct.as_secs_f64()
                );
            }
        }

        #[test]
        fn duplicated_typed_matches_bruteforce_reference() {
            // Independent O(n^2) reference: for keep=first a value is a dup iff
            // an equal value occurs earlier; last iff later; none iff any other
            // position is equal. Float keys compare on normalized to_bits (so
            // -0.0==+0.0) — matching key_of. Proves the typed FxHash fast path
            // is bit-identical for all-valid Int64/Float64 columns.
            let mut state: u64 = 0xD1B5_4A32_D192_ED03;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state
            };
            let fbits = |f: f64| (if f == 0.0 { 0.0 } else { f }).to_bits();
            for trial in 0..150 {
                let n = (next() % 300) as usize + 1;
                // Narrow value range → many duplicates / ties.
                let raw: Vec<i64> = (0..n).map(|_| (next() % 9) as i64 - 4).collect();
                let i64_vals: Vec<Scalar> = raw.iter().map(|&v| Scalar::Int64(v)).collect();
                let f64_vals: Vec<Scalar> = raw
                    .iter()
                    .map(|&v| Scalar::Float64(v as f64 / 2.0))
                    .collect();
                let i64_keys: Vec<i64> = raw.clone();
                let f64_keys: Vec<u64> = raw.iter().map(|&v| fbits(v as f64 / 2.0)).collect();

                for keep in ["first", "last", "false"] {
                    // Brute-force references over both key representations.
                    let bf = |eq_keys: &dyn Fn(usize, usize) -> bool| -> Vec<bool> {
                        (0..n)
                            .map(|i| match keep {
                                "first" => (0..i).any(|j| eq_keys(i, j)),
                                "last" => (i + 1..n).any(|j| eq_keys(i, j)),
                                _ => (0..n).any(|j| j != i && eq_keys(i, j)),
                            })
                            .collect()
                    };
                    let want_i = bf(&|a, b| i64_keys[a] == i64_keys[b]);
                    let want_f = bf(&|a, b| f64_keys[a] == f64_keys[b]);

                    let col_i = Column::from_values(i64_vals.clone()).expect("i64 col");
                    let got_i: Vec<bool> = col_i
                        .duplicated_keep(keep)
                        .expect("dup i64")
                        .values()
                        .iter()
                        .map(|v| matches!(v, Scalar::Bool(true)))
                        .collect();
                    assert_eq!(got_i, want_i, "i64 trial {trial} keep={keep}");

                    let col_f = Column::from_values(f64_vals.clone()).expect("f64 col");
                    let got_f: Vec<bool> = col_f
                        .duplicated_keep(keep)
                        .expect("dup f64")
                        .values()
                        .iter()
                        .map(|v| matches!(v, Scalar::Bool(true)))
                        .collect();
                    assert_eq!(got_f, want_f, "f64 trial {trial} keep={keep}");
                }
            }
        }

        #[test]
        #[ignore = "timing benchmark; run with --ignored --nocapture on the rch VM"]
        fn duplicated_typed_timing_vs_scalar() {
            use std::{collections::HashSet, time::Instant};
            let n = 5_000_000usize;
            let iters = 10;
            // Faithful replica of the OLD path: build the per-value Key enum
            // over &[Scalar] and insert into a std (SipHash) HashSet.
            #[derive(Hash, PartialEq, Eq)]
            enum OldKey {
                Int64(i64),
                Null,
            }
            for cardinality in [1_000u64, 2_000_000u64] {
                let mut state: u64 = 0x0FED_CBA9_8765_4321 ^ cardinality;
                let mut next = || {
                    state = state
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    state
                };
                let data: Vec<i64> = (0..n).map(|_| (next() % cardinality) as i64).collect();

                let col = Column::from_i64_values(data.clone());
                let t0 = Instant::now();
                let mut chk = 0usize;
                for _ in 0..iters {
                    let d = col.duplicated_keep("first").expect("typed");
                    chk ^= d
                        .values()
                        .iter()
                        .filter(|v| matches!(v, Scalar::Bool(true)))
                        .count();
                }
                let typed = t0.elapsed();

                let scalar_col =
                    Column::from_values(data.iter().map(|&v| Scalar::Int64(v)).collect())
                        .expect("col");
                let t1 = Instant::now();
                let mut chk2 = 0usize;
                for _ in 0..iters {
                    let mut seen: HashSet<OldKey> = HashSet::new();
                    let mut count = 0usize;
                    for v in scalar_col.values() {
                        let key = if v.is_missing() {
                            OldKey::Null
                        } else if let Scalar::Int64(i) = v {
                            OldKey::Int64(*i)
                        } else {
                            OldKey::Null
                        };
                        if !seen.insert(key) {
                            count += 1;
                        }
                    }
                    chk2 ^= count;
                }
                let scalar = t1.elapsed();
                eprintln!(
                    "duplicated 5M i64 card={cardinality} x{iters}: typed={typed:?} old_keyenum_siphash={scalar:?} ratio={:.2}x (chk {chk}/{chk2})",
                    scalar.as_secs_f64() / typed.as_secs_f64()
                );
            }
        }

        #[test]
        #[ignore = "timing benchmark; run with --ignored --nocapture on the rch VM"]
        fn radix_sort_timing_vs_scalar() {
            use std::time::Instant;
            let n = 5_000_000usize;
            let mut state: u64 = 0x1234_5678_9ABC_DEF0;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state
            };
            let data: Vec<i64> = (0..n).map(|_| next() as i64).collect();
            let col = Column::from_i64_values(data.clone());

            let iters = 10;
            let t0 = Instant::now();
            let mut checksum = 0i64;
            for _ in 0..iters {
                let sorted = col.sort_values(true).expect("radix");
                checksum ^= match &sorted.values()[0] {
                    Scalar::Int64(v) => *v,
                    _ => 0,
                };
            }
            let radix = t0.elapsed();

            // Old Scalar comparator path, reproduced inline for the A/B.
            let scalar_col = Column::from_values(data.iter().map(|&v| Scalar::Int64(v)).collect())
                .expect("scalar col");
            let t1 = Instant::now();
            let mut checksum2 = 0i64;
            for _ in 0..iters {
                let mut indexed: Vec<(usize, &Scalar)> =
                    scalar_col.values().iter().enumerate().collect();
                indexed.sort_by(|a, b| crate::compare_scalars_na_last(a.1, b.1, true));
                if let Scalar::Int64(v) = indexed[0].1 {
                    checksum2 ^= *v;
                }
            }
            let scalar = t1.elapsed();
            eprintln!(
                "sort_single 5M i64 x{iters}: radix={radix:?} scalar={scalar:?} ratio={:.2}x (chk {checksum}/{checksum2})",
                scalar.as_secs_f64() / radix.as_secs_f64()
            );
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
        fn diff_timedelta64_returns_timedelta_e607u() {
            // Per br-frankenpandas-e607u: Column::diff on Timedelta64 preserves
            // Timedelta dtype (was forced to Float64 NaN before via to_f64 catch-all).
            let one_hour = 3_600 * 1_000_000_000_i64;
            let col = Column::from_values(vec![
                Scalar::Timedelta64(one_hour),
                Scalar::Timedelta64(3 * one_hour),
                Scalar::Timedelta64(2 * one_hour),
            ])
            .expect("col");
            let d = col.diff(1).expect("diff");
            assert_eq!(d.dtype(), DType::Timedelta64);
            assert!(d.values()[0].is_missing()); // first row → NaT
            assert_eq!(d.values()[1], Scalar::Timedelta64(2 * one_hour));
            assert_eq!(d.values()[2], Scalar::Timedelta64(-one_hour));
        }

        #[test]
        fn diff_timedelta64_nat_propagates_e607u() {
            use fp_types::Timedelta;
            let one_hour = 3_600 * 1_000_000_000_i64;
            let col = Column::from_values(vec![
                Scalar::Timedelta64(one_hour),
                Scalar::Timedelta64(Timedelta::NAT),
                Scalar::Timedelta64(2 * one_hour),
            ])
            .expect("col");
            let d = col.diff(1).expect("diff");
            assert_eq!(d.dtype(), DType::Timedelta64);
            assert!(d.values()[0].is_missing());
            assert!(d.values()[1].is_missing()); // NaT current → NaT
            assert!(d.values()[2].is_missing()); // NaT previous → NaT
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
        fn duplicated_keep_variants_match_pandas() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");

            let last = col.duplicated_keep("last").expect("duplicated last");
            assert_eq!(
                last.values(),
                &[
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                    Scalar::Bool(false),
                    Scalar::Bool(false),
                    Scalar::Bool(false),
                    Scalar::Bool(true),
                    Scalar::Bool(false),
                ]
            );

            let none = col.duplicated_keep("false").expect("duplicated none");
            assert_eq!(
                none.values(),
                &[
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                    Scalar::Bool(false),
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                    Scalar::Bool(true),
                ]
            );
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
        fn between_left_and_right_inclusive_edges() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");

            let left = col
                .between_inclusive(1.0, 5.0, "left")
                .expect("between left");
            assert_eq!(
                left.values(),
                &[Scalar::Bool(true), Scalar::Bool(true), Scalar::Bool(false),]
            );

            let right = col
                .between_inclusive(1.0, 5.0, "right")
                .expect("between right");
            assert_eq!(
                right.values(),
                &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(true),]
            );
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
        use fp_types::NullKind;

        use super::*;

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

        #[test]
        fn factorize_with_sort_sorts_uniques_and_relabels_codes() {
            let col = Column::from_values(vec![
                Scalar::Utf8("b".into()),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Utf8("c".into()),
                Scalar::Utf8("a".into()),
            ])
            .expect("col");

            let (codes, uniques) = col.factorize_with_options(true, true).expect("factorize");
            assert_eq!(
                codes.values(),
                &[
                    Scalar::Int64(1),
                    Scalar::Int64(0),
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(0),
                ]
            );
            assert_eq!(
                uniques.values(),
                &[
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("c".into()),
                ]
            );
        }

        #[test]
        fn factorize_with_use_na_sentinel_false_keeps_missing_in_uniques() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.5),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.5),
                Scalar::Null(NullKind::Null),
                Scalar::Float64(1.5),
            ])
            .expect("col");

            let (codes, uniques) = col.factorize_with_options(false, false).expect("factorize");
            assert_eq!(
                codes.values(),
                &[
                    Scalar::Int64(0),
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(1),
                    Scalar::Int64(0),
                ]
            );
            assert_eq!(uniques.dtype(), DType::Float64);
            assert_eq!(
                uniques.values(),
                &[
                    Scalar::Float64(1.5),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(2.5),
                ]
            );
        }

        #[test]
        fn factorize_with_sort_and_use_na_sentinel_false_sorts_missing_last() {
            let col = Column::from_values(vec![
                Scalar::Utf8("b".into()),
                Scalar::Null(NullKind::Null),
                Scalar::Utf8("a".into()),
                Scalar::Utf8("b".into()),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");

            let (codes, uniques) = col.factorize_with_options(true, false).expect("factorize");
            assert_eq!(
                codes.values(),
                &[
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(0),
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                ]
            );
            assert_eq!(
                uniques.values(),
                &[
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Null(NullKind::Null),
                ]
            );
        }
    }

    mod aggregation_helpers {
        use fp_types::{NullKind, Timedelta};

        use super::*;

        fn assert_float_nan(value: Scalar) {
            assert!(
                matches!(value, Scalar::Float64(v) if v.is_nan()),
                "expected Float64(NaN), got {value:?}"
            );
        }

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
        fn product_alias_matches_prod() {
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            assert_eq!(col.product(), col.prod());
        }

        #[test]
        fn skipna_false_aggregate_variants_propagate_nan() {
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ])
            .expect("col");

            assert_eq!(col.sum_skipna(true), col.sum());
            assert_float_nan(col.sum_skipna(false));
            assert_float_nan(col.mean_skipna(false));
            assert_float_nan(col.min_skipna(false));
            assert_float_nan(col.max_skipna(false));
            assert_float_nan(col.median_skipna(false));
            assert_float_nan(col.prod_skipna(false));
            assert_float_nan(col.var_skipna(1, false));
            assert_float_nan(col.std_skipna(1, false));
            assert_float_nan(col.sem_skipna(1, false));
        }

        #[test]
        fn skipna_false_timedelta_aggregate_variants_propagate_nat() {
            let col = Column::from_values(vec![
                Scalar::Timedelta64(Timedelta::NANOS_PER_SEC),
                Scalar::Timedelta64(Timedelta::NAT),
            ])
            .expect("col");

            assert_eq!(col.sum_skipna(false), Scalar::Timedelta64(Timedelta::NAT));
            assert_eq!(col.mean_skipna(false), Scalar::Timedelta64(Timedelta::NAT));
            assert_eq!(col.min_skipna(false), Scalar::Timedelta64(Timedelta::NAT));
            assert_eq!(col.max_skipna(false), Scalar::Timedelta64(Timedelta::NAT));
        }

        #[test]
        fn quantile_median_of_sorted_values() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let quantile = col.quantile(0.5);
            assert!(
                matches!(quantile, Scalar::Float64(v) if (v - 3.0).abs() < 1e-9),
                "expected Float64 median, got {quantile:?}"
            );
        }

        #[test]
        fn quantile_empty_is_null() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert!(col.quantile(0.5).is_missing());
        }

        #[test]
        fn quantile_out_of_range_is_null() {
            let col = Column::from_values(vec![Scalar::Float64(1.0)]).expect("col");
            assert!(col.quantile(1.5).is_missing());
            assert!(col.quantile(-0.1).is_missing());
        }

        #[test]
        fn mode_returns_tied_max_frequency() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(3),
            ])
            .expect("col");
            let m = col.mode().expect("mode");
            assert_eq!(m.values(), &[Scalar::Int64(2), Scalar::Int64(3)]);
        }

        #[test]
        fn mode_ignores_missing_values() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let m = col.mode().expect("mode");
            assert_eq!(m.values(), &[Scalar::Int64(1)]);
        }

        #[test]
        fn mode_empty_is_empty_same_dtype() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            let m = col.mode().expect("mode");
            assert!(m.is_empty());
        }

        #[test]
        fn memory_usage_fixed_width_for_numeric() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let usage = col.memory_usage(false);
            // 3 * 8 + ceil(3/8) = 24 + 1
            assert_eq!(usage, 25);
        }

        #[test]
        fn memory_usage_deep_counts_utf8_bytes() {
            let col = Column::from_values(vec![
                Scalar::Utf8("hi".into()),
                Scalar::Utf8("world".into()),
            ])
            .expect("col");
            let shallow = col.memory_usage(false);
            let deep = col.memory_usage(true);
            assert!(deep > shallow);
            // deep_extra = "hi".len() + "world".len() = 2 + 5 = 7
            assert_eq!(deep - shallow, 7);
        }

        #[test]
        fn interpolate_fills_interior_gaps() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ])
            .expect("col");
            let r = col.interpolate_linear().expect("interpolate");
            assert_eq!(r.values()[0], Scalar::Float64(1.0));
            assert!(
                matches!(&r.values()[1], Scalar::Float64(v) if (*v - 2.0).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[1]
            );
            assert!(
                matches!(&r.values()[2], Scalar::Float64(v) if (*v - 3.0).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[2]
            );
            assert_eq!(r.values()[3], Scalar::Float64(4.0));
        }

        #[test]
        fn interpolate_leading_null_stays_null_trailing_forward_fills() {
            // pandas Series.interpolate(method='linear') default
            // limit_direction='forward': leading NaN stays NaN, interior is
            // interpolated, trailing NaN is forward-filled with the last valid
            // value (NOT extrapolated). [nan,2,nan,4,nan] -> [nan,2,3,4,4].
            // (br-frankenpandas-8ic7c)
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let r = col.interpolate_linear().expect("interpolate");
            assert!(r.values()[0].is_missing());
            assert_eq!(r.values()[1], Scalar::Float64(2.0));
            assert!(
                matches!(&r.values()[2], Scalar::Float64(v) if (*v - 3.0).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[2]
            );
            assert_eq!(r.values()[3], Scalar::Float64(4.0));
            // Trailing NaN forward-filled with the last valid value (4.0).
            assert_eq!(r.values()[4], Scalar::Float64(4.0));
        }

        #[test]
        fn interpolate_trailing_run_forward_fills_without_extrapolating() {
            // [2,4,nan,nan] -> [2,4,4,4] (ffill), NOT [2,4,6,8] (extrapolation).
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let r = col.interpolate_linear().expect("interpolate");
            assert_eq!(
                r.values(),
                &[
                    Scalar::Float64(2.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(4.0),
                ]
            );
        }

        #[test]
        fn interpolate_empty_is_empty_float64() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            let r = col.interpolate_linear().expect("interpolate");
            assert!(r.is_empty());
            assert_eq!(r.dtype(), DType::Float64);
        }

        #[test]
        fn interpolate_alias_matches_default_linear_interpolation() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ])
            .expect("col");

            assert_eq!(
                col.interpolate().expect("interpolate"),
                col.interpolate_linear().expect("interpolate_linear")
            );
        }

        #[test]
        fn drop_duplicates_keeps_first_occurrence() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ])
            .expect("col");
            let d = col.drop_duplicates().expect("drop_duplicates");
            assert_eq!(
                d.values(),
                &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
            );
        }

        #[test]
        fn drop_duplicates_treats_nulls_as_one_bucket() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let d = col.drop_duplicates().expect("drop_duplicates");
            // First null is kept; subsequent null is dropped.
            assert_eq!(d.len(), 2);
            assert!(d.values()[0].is_missing());
            assert_eq!(d.values()[1], Scalar::Int64(1));
        }

        #[test]
        fn drop_duplicates_keep_variants_match_pandas() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");

            let last = col.drop_duplicates_keep("last").expect("drop last");
            assert_eq!(last.len(), 4);
            assert_eq!(last.values()[0], Scalar::Int64(1));
            assert_eq!(last.values()[1], Scalar::Int64(3));
            assert_eq!(last.values()[2], Scalar::Int64(2));
            assert!(last.values()[3].is_missing());

            let none = col.drop_duplicates_keep("false").expect("drop none");
            assert_eq!(none.values(), &[Scalar::Int64(3)]);
        }

        #[test]
        fn compare_returns_only_differences() {
            let a = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                .expect("a");
            let b =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(20), Scalar::Int64(3)])
                    .expect("b");
            let (left, right) = a.compare(&b).expect("compare");
            assert_eq!(left.values(), &[Scalar::Int64(2)]);
            assert_eq!(right.values(), &[Scalar::Int64(20)]);
        }

        #[test]
        fn compare_treats_matching_nulls_as_equal() {
            let a = Column::from_values(vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)])
                .expect("a");
            let b = Column::from_values(vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)])
                .expect("b");
            let (left, right) = a.compare(&b).expect("compare");
            assert!(left.is_empty());
            assert!(right.is_empty());
        }

        #[test]
        fn compare_length_mismatch_errors() {
            let a = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(1)]).expect("b");
            let err = a.compare(&b).unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }

        #[test]
        fn map_applies_unary_function() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let doubled = col
                .map(|v| match v {
                    Scalar::Int64(i) => Scalar::Int64(i * 2),
                    other => other.clone(),
                })
                .expect("map");
            assert_eq!(doubled.values()[0], Scalar::Int64(2));
            assert_eq!(doubled.values()[1], Scalar::Int64(4));
            assert_eq!(doubled.values()[2], Scalar::Int64(6));
        }

        #[test]
        fn map_can_change_dtype() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let as_str = col
                .map(|v| match v {
                    Scalar::Int64(i) => Scalar::Utf8(i.to_string()),
                    other => other.clone(),
                })
                .expect("map");
            assert_eq!(as_str.dtype(), DType::Utf8);
            assert_eq!(as_str.values()[0], Scalar::Utf8("1".into()));
        }

        #[test]
        fn argmin_argmax_skip_missing() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Int64(5),
                Scalar::Int64(2),
            ])
            .expect("col");
            assert_eq!(col.argmin(), Some(2));
            assert_eq!(col.argmax(), Some(3));
            assert_eq!(col.idxmin(), Some(2));
            assert_eq!(col.idxmax(), Some(3));
        }

        #[test]
        fn argmin_argmax_all_missing_returns_none() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");
            assert!(col.argmin().is_none());
            assert!(col.argmax().is_none());
            assert!(col.idxmin().is_none());
            assert!(col.idxmax().is_none());
        }

        #[test]
        fn is_monotonic_increasing_detects_ascending() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(5),
            ])
            .expect("col");
            assert!(col.is_monotonic_increasing());
            assert!(!col.is_monotonic_decreasing());
        }

        #[test]
        fn is_monotonic_decreasing_detects_descending() {
            let col =
                Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(3), Scalar::Int64(1)])
                    .expect("col");
            assert!(col.is_monotonic_decreasing());
            assert!(!col.is_monotonic_increasing());
        }

        #[test]
        fn is_monotonic_skips_missing_values() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
                Scalar::Int64(5),
            ])
            .expect("col");
            assert!(col.is_monotonic_increasing());
        }

        #[test]
        fn is_monotonic_empty_is_true() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert!(col.is_monotonic_increasing());
            assert!(col.is_monotonic_decreasing());
        }

        #[test]
        fn combine_first_fills_missing_from_other() {
            let a = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
            ])
            .expect("b");
            let c = a.combine_first(&b).expect("combine_first");
            assert_eq!(c.values()[0], Scalar::Int64(1));
            assert_eq!(c.values()[1], Scalar::Int64(20));
            assert_eq!(c.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn combine_first_length_mismatch_errors() {
            let a = Column::from_values(vec![Scalar::Int64(1)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("b");
            let err = a.combine_first(&b).unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }

        #[test]
        fn clip_lower_only() {
            let col = Column::from_values(vec![
                Scalar::Float64(-2.0),
                Scalar::Float64(0.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let c = col.clip_lower(0.0).expect("clip_lower");
            assert_eq!(c.values()[0], Scalar::Float64(0.0));
            assert_eq!(c.values()[1], Scalar::Float64(0.0));
            assert_eq!(c.values()[2], Scalar::Float64(5.0));
        }

        #[test]
        fn clip_upper_only() {
            let col = Column::from_values(vec![
                Scalar::Float64(-2.0),
                Scalar::Float64(0.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let c = col.clip_upper(1.0).expect("clip_upper");
            assert_eq!(c.values()[0], Scalar::Float64(-2.0));
            assert_eq!(c.values()[1], Scalar::Float64(0.0));
            assert_eq!(c.values()[2], Scalar::Float64(1.0));
        }

        #[test]
        fn describe_returns_pandas_order() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let stats = col.describe().expect("describe");
            let names: Vec<&str> = stats.iter().map(|(k, _)| *k).collect();
            assert_eq!(
                names,
                vec!["count", "mean", "std", "min", "25%", "50%", "75%", "max"]
            );
            assert_eq!(stats[0].1, Scalar::Int64(5));
            assert!(
                matches!(&stats[1].1, Scalar::Float64(v) if (*v - 3.0).abs() < 1e-9),
                "expected Float64, got {:?}",
                stats[1].1
            );
            assert_eq!(stats[3].1, Scalar::Float64(1.0));
            assert_eq!(stats[7].1, Scalar::Float64(5.0));
        }

        #[test]
        fn describe_rejects_utf8_column() {
            let col = Column::from_values(vec![Scalar::Utf8("a".into())]).expect("col");
            assert!(col.describe().is_err());
        }

        #[test]
        fn combine_uses_fill_for_missing() {
            let a = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("b");
            let out = a
                .combine(
                    &b,
                    |l, r| {
                        if let (Ok(lf), Ok(rf)) = (l.to_f64(), r.to_f64()) {
                            Scalar::Float64(lf + rf)
                        } else {
                            Scalar::Null(NullKind::NaN)
                        }
                    },
                    Some(Scalar::Int64(0)),
                )
                .expect("combine");
            assert_eq!(out.values()[0], Scalar::Float64(11.0));
            assert_eq!(out.values()[1], Scalar::Float64(20.0));
            assert_eq!(out.values()[2], Scalar::Float64(3.0));
        }

        #[test]
        fn combine_length_mismatch_errors() {
            let a = Column::from_values(vec![Scalar::Int64(1)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("b");
            let err = a
                .combine(&b, |l, _| l.clone(), Some(Scalar::Int64(0)))
                .unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }

        #[test]
        fn combine_fill_none_propagates_nulls_without_invoking_func() {
            let a = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("b");
            let mut calls = 0usize;
            let out = a
                .combine(
                    &b,
                    |l, r| {
                        calls += 1;
                        Scalar::Float64(l.to_f64().unwrap() + r.to_f64().unwrap())
                    },
                    None,
                )
                .expect("combine");
            // Only the position with both non-null invokes func.
            assert_eq!(calls, 1);
            assert_eq!(out.values()[0], Scalar::Float64(11.0));
            assert!(out.values()[1].is_missing());
            assert!(out.values()[2].is_missing());
        }

        #[test]
        fn combine_fill_none_all_present_matches_elementwise_apply() {
            let a = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).expect("b");
            let out = a
                .combine(
                    &b,
                    |l, r| Scalar::Int64(l.to_f64().unwrap() as i64 + r.to_f64().unwrap() as i64),
                    None,
                )
                .expect("combine");
            assert_eq!(out.values()[0], Scalar::Int64(11));
            assert_eq!(out.values()[1], Scalar::Int64(22));
        }

        #[test]
        fn apply_float_applies_numeric_fn() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ])
            .expect("col");
            let out = col.apply_float(|x| x.sqrt()).expect("apply_float");
            assert_eq!(out.values()[0], Scalar::Float64(1.0));
            assert!(out.values()[1].is_missing());
            assert_eq!(out.values()[2], Scalar::Float64(2.0));
            assert_eq!(out.dtype(), DType::Float64);
        }

        #[test]
        fn apply_float_rejects_non_numeric() {
            let col = Column::from_values(vec![Scalar::Utf8("x".into())]).expect("col");
            assert!(col.apply_float(|x| x + 1.0).is_err());
        }

        #[test]
        fn hist_counts_equal_width_bins() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(9.0),
            ])
            .expect("col");
            let counts = col.hist_counts(3);
            // Bin width = 3, buckets [0,3), [3,6), [6,9]
            assert_eq!(counts.len(), 3);
            assert_eq!(counts[0], 3); // 0,1,2
            assert_eq!(counts[1], 1); // 3
            assert_eq!(counts[2], 1); // 9 clamps into last bin
        }

        #[test]
        fn hist_counts_zero_bins_is_empty() {
            let col = Column::from_values(vec![Scalar::Float64(1.0)]).expect("col");
            assert!(col.hist_counts(0).is_empty());
        }

        #[test]
        fn hist_counts_constant_column_puts_all_in_first_bin() {
            let col = Column::from_values(vec![
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            let counts = col.hist_counts(3);
            assert_eq!(counts[0], 3);
            assert_eq!(counts[1], 0);
            assert_eq!(counts[2], 0);
        }

        #[test]
        fn nunique_drops_nulls() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            assert_eq!(col.nunique(), Scalar::Int64(2));
        }

        #[test]
        fn nunique_with_dropna_false_counts_missing_once() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");
            assert_eq!(col.nunique_with_dropna(false), Scalar::Int64(3));
        }

        #[test]
        fn nunique_with_dropna_false_all_missing_is_one() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            assert_eq!(col.nunique(), Scalar::Int64(0));
            assert_eq!(col.nunique_with_dropna(false), Scalar::Int64(1));
        }

        #[test]
        fn any_all_reductions() {
            let col = Column::from_values(vec![Scalar::Int64(0), Scalar::Int64(0)]).expect("col");
            assert_eq!(col.any(), Scalar::Bool(false));
            assert_eq!(col.all(), Scalar::Bool(false));

            let mixed = Column::from_values(vec![Scalar::Int64(0), Scalar::Int64(1)]).expect("col");
            assert_eq!(mixed.any(), Scalar::Bool(true));
            assert_eq!(mixed.all(), Scalar::Bool(false));

            let all_true =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            assert_eq!(all_true.all(), Scalar::Bool(true));
        }

        #[test]
        fn is_unique_true_when_no_repeats() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            assert!(col.is_unique());
            assert!(!col.has_duplicates());
        }

        #[test]
        fn has_duplicates_true_when_repeats_present() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(1)]).expect("col");
            assert!(col.has_duplicates());
            assert!(!col.is_unique());
        }

        #[test]
        fn is_unique_ignores_nulls() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            assert!(col.is_unique());
        }

        #[test]
        fn pct_change_periods_one() {
            let col = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Float64(12.0),
                Scalar::Float64(9.0),
            ])
            .expect("col");
            let r = col.pct_change(1).expect("pct_change");
            assert!(r.values()[0].is_missing());
            assert!(
                matches!(&r.values()[1], Scalar::Float64(v) if (*v - 0.2).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[1]
            );
            assert!(
                matches!(&r.values()[2], Scalar::Float64(v) if (*v + 0.25).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[2]
            );
        }

        #[test]
        fn pct_change_zero_prev_yields_null() {
            let col =
                Column::from_values(vec![Scalar::Float64(0.0), Scalar::Float64(5.0)]).expect("col");
            let r = col.pct_change(1).expect("pct_change");
            assert!(r.values()[1].is_missing());
        }

        #[test]
        fn pct_change_timedelta64_matches_pandas_mcu90() {
            // Per br-frankenpandas-mcu90: pct_change on Timedelta64 returns
            // dimensionless f64 ratios; was silently NaN before via the
            // to_f64-else catch-all (Timedelta64.to_f64() errors).
            let one_hour = 3_600 * 1_000_000_000_i64;
            let col = Column::from_values(vec![
                Scalar::Timedelta64(one_hour),
                Scalar::Timedelta64(2 * one_hour),
                Scalar::Timedelta64(4 * one_hour),
            ])
            .expect("col");
            let r = col.pct_change(1).expect("pct_change");
            assert!(r.values()[0].is_missing());
            assert!(
                matches!(&r.values()[1], Scalar::Float64(v) if (*v - 1.0).abs() < 1e-10),
                "expected Float64(1.0), got {:?}",
                r.values()[1]
            );
            assert!(
                matches!(&r.values()[2], Scalar::Float64(v) if (*v - 1.0).abs() < 1e-10),
                "expected Float64(1.0), got {:?}",
                r.values()[2]
            );
        }

        #[test]
        fn pct_change_timedelta64_nat_propagates_mcu90() {
            use fp_types::Timedelta;
            let one_hour = 3_600 * 1_000_000_000_i64;
            let col = Column::from_values(vec![
                Scalar::Timedelta64(one_hour),
                Scalar::Timedelta64(Timedelta::NAT),
                Scalar::Timedelta64(2 * one_hour),
            ])
            .expect("col");
            let r = col.pct_change(1).expect("pct_change");
            assert!(r.values()[0].is_missing());
            assert!(r.values()[1].is_missing()); // NaT current → NaN
            assert!(r.values()[2].is_missing()); // NaT previous → NaN
        }

        #[test]
        fn pct_change_with_fill_ffill_uses_filled_previous_value() {
            let col = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(12.0),
            ])
            .expect("col");
            let r = col
                .pct_change_with_fill(1, Some("ffill"), None)
                .expect("pct_change_with_fill");
            assert!(r.values()[0].is_missing());
            assert!(
                matches!(&r.values()[1], Scalar::Float64(v) if v.abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[1]
            );
            assert!(
                matches!(&r.values()[2], Scalar::Float64(v) if (*v - 0.2).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[2]
            );
        }

        #[test]
        fn pct_change_with_fill_limit_caps_forward_fill_runs() {
            let col = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(20.0),
            ])
            .expect("col");
            let r = col
                .pct_change_with_fill(1, Some("ffill"), Some(1))
                .expect("pct_change_with_fill");
            assert!(r.values()[0].is_missing());
            assert!(
                matches!(&r.values()[1], Scalar::Float64(v) if v.abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[1]
            );
            assert!(r.values()[2].is_missing());
            assert!(r.values()[3].is_missing());
        }

        #[test]
        fn pct_change_with_fill_bfill_aliases_backward_fill() {
            let col = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(20.0),
            ])
            .expect("col");
            let r = col
                .pct_change_with_fill(1, Some("backfill"), None)
                .expect("pct_change_with_fill");
            assert!(r.values()[0].is_missing());
            assert!(
                matches!(&r.values()[1], Scalar::Float64(v) if (*v - 1.0).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[1]
            );
            assert!(
                matches!(&r.values()[2], Scalar::Float64(v) if v.abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[2]
            );
        }

        #[test]
        fn pct_change_with_fill_rejects_invalid_method() {
            let col =
                Column::from_values(vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]).expect("col");
            let err = col
                .pct_change_with_fill(1, Some("nearest"), None)
                .expect_err("invalid fill_method should error");
            assert!(matches!(
                err,
                crate::ColumnError::Type(fp_types::TypeError::NonNumericValue { .. })
            ));
        }

        #[test]
        fn ffill_fills_trailing_missing_runs() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(5),
            ])
            .expect("col");
            let r = col.ffill(None).expect("ffill");
            assert!(r.values()[0].is_missing());
            assert_eq!(r.values()[1], Scalar::Int64(1));
            assert_eq!(r.values()[2], Scalar::Int64(1));
            assert_eq!(r.values()[3], Scalar::Int64(1));
            assert_eq!(r.values()[4], Scalar::Int64(5));
        }

        #[test]
        fn ffill_respects_limit_per_run() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(9),
            ])
            .expect("col");
            let r = col.ffill(Some(2)).expect("ffill");
            assert_eq!(r.values()[0], Scalar::Int64(1));
            assert_eq!(r.values()[1], Scalar::Int64(1));
            assert_eq!(r.values()[2], Scalar::Int64(1));
            assert!(r.values()[3].is_missing());
            assert_eq!(r.values()[4], Scalar::Int64(9));
            assert_eq!(col.pad(Some(2)), col.ffill(Some(2)));
        }

        #[test]
        fn bfill_fills_leading_missing_runs() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            let r = col.bfill(None).expect("bfill");
            assert_eq!(r.values()[0], Scalar::Int64(3));
            assert_eq!(r.values()[1], Scalar::Int64(3));
            assert_eq!(r.values()[2], Scalar::Int64(3));
            // Trailing null stays null (no next value).
            assert!(r.values()[3].is_missing());
        }

        #[test]
        fn bfill_respects_limit_per_run() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(7),
            ])
            .expect("col");
            let r = col.bfill(Some(1)).expect("bfill");
            assert!(r.values()[0].is_missing());
            assert!(r.values()[1].is_missing());
            assert_eq!(r.values()[2], Scalar::Int64(7));
            assert_eq!(r.values()[3], Scalar::Int64(7));
            assert_eq!(col.backfill(Some(1)), col.bfill(Some(1)));
        }

        #[test]
        fn ffill_empty_is_empty_same_dtype() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            let r = col.ffill(None).expect("ffill");
            assert!(r.is_empty());
            assert_eq!(r.dtype(), DType::Null);
        }

        #[test]
        fn pandas_metadata_and_materialization_aliases_match_core_methods()
        -> Result<(), crate::ColumnError> {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(2),
            ])?;

            assert_eq!(col.size(), col.len());
            assert_eq!(col.shape(), (col.len(),));
            assert_eq!(col.ndim(), 1);
            assert_eq!(col.empty(), col.is_empty());
            assert_eq!(col.to_list(), col.to_vec());
            assert_eq!(col.tolist(), col.to_vec());
            assert_eq!(col.to_numpy(), col.to_vec());
            assert_eq!(col.ravel(), col.to_numpy());
            assert_eq!(col.array(), col.to_vec());
            Ok(())
        }

        #[test]
        fn isnull_notnull_flag_missing_positions() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(2),
            ])
            .expect("col");
            let is_null = col.isnull().expect("isnull");
            let not_null = col.notnull().expect("notnull");
            assert_eq!(is_null.dtype(), DType::Bool);
            assert_eq!(
                is_null.values(),
                &[Scalar::Bool(false), Scalar::Bool(true), Scalar::Bool(false),]
            );
            assert_eq!(col.isna(), col.isnull());
            assert_eq!(
                not_null.values(),
                &[Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true),]
            );
            assert_eq!(col.notna(), col.notnull());
        }

        #[test]
        fn var_std_sem_ddof_one() {
            let col = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(4.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
                Scalar::Float64(7.0),
                Scalar::Float64(9.0),
            ])
            .expect("col");
            match col.var(1) {
                Scalar::Float64(v) => assert!((v - 4.571428571428571).abs() < 1e-9),
                other => unreachable!("expected Float64, got {other:?}"),
            }
            match col.std(1) {
                Scalar::Float64(v) => assert!((v - 2.138089935299395).abs() < 1e-9),
                other => unreachable!("expected Float64, got {other:?}"),
            }
            match col.sem(1) {
                Scalar::Float64(v) => assert!((v - 0.7559289460184544).abs() < 1e-9),
                other => unreachable!("expected Float64, got {other:?}"),
            }
        }

        #[test]
        fn skew_symmetric_is_zero() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            match col.skew() {
                Scalar::Float64(v) => assert!(v.abs() < 1e-9),
                other => unreachable!("expected Float64, got {other:?}"),
            }
        }

        #[test]
        fn kurt_uniform_five_values_is_minus_one_point_two() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            match col.kurt() {
                Scalar::Float64(v) => assert!((v + 1.2).abs() < 1e-9),
                other => unreachable!("expected Float64, got {other:?}"),
            }
        }

        #[test]
        fn kurtosis_alias_matches_kurt() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            assert_eq!(col.kurtosis(), col.kurt());
        }

        #[test]
        fn ptp_returns_max_minus_min() {
            let col = Column::from_values(vec![
                Scalar::Float64(3.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(7.0),
                Scalar::Float64(1.0),
            ])
            .expect("col");
            assert_eq!(col.ptp(), Scalar::Float64(6.0));
        }

        #[test]
        fn ptp_empty_is_null() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert!(col.ptp().is_missing());
        }

        #[test]
        fn skew_too_few_values_returns_null() {
            let col =
                Column::from_values(vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]).expect("col");
            assert!(col.skew().is_missing());
        }

        #[test]
        fn rolling_window_sum_full_window() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("col");
            // window=3, min_periods=3 -> [NaN, NaN, 6, 9, 12]
            let r = col.rolling_window_sum(3, 3).expect("rolling");
            assert!(r.values()[0].is_missing());
            assert!(r.values()[1].is_missing());
            assert_eq!(r.values()[2], Scalar::Float64(6.0));
            assert_eq!(r.values()[3], Scalar::Float64(9.0));
            assert_eq!(r.values()[4], Scalar::Float64(12.0));
        }

        #[test]
        fn rolling_window_sum_min_periods_relaxed() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])
            .expect("col");
            // window=3, min_periods=1 -> [1, 3, 6]
            let r = col.rolling_window_sum(3, 1).expect("rolling");
            assert_eq!(r.values()[0], Scalar::Float64(1.0));
            assert_eq!(r.values()[1], Scalar::Float64(3.0));
            assert_eq!(r.values()[2], Scalar::Float64(6.0));
        }

        #[test]
        fn rolling_window_sum_skips_missing() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ])
            .expect("col");
            // window=3, min_periods=2:
            // i=0: {1.0} observed=1 → NaN (below min_periods)
            // i=1: {1.0, NaN} observed=1 → NaN
            // i=2: {1.0, NaN, 3.0} observed=2 → 4.0
            // i=3: {NaN, 3.0, 4.0} observed=2 → 7.0
            let r = col.rolling_window_sum(3, 2).expect("rolling");
            assert!(r.values()[0].is_missing());
            assert!(r.values()[1].is_missing());
            assert_eq!(r.values()[2], Scalar::Float64(4.0));
            assert_eq!(r.values()[3], Scalar::Float64(7.0));
        }

        #[test]
        fn rolling_window_sum_window_zero_is_all_null() {
            let col =
                Column::from_values(vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]).expect("col");
            let r = col.rolling_window_sum(0, 0).expect("rolling");
            assert!(r.values()[0].is_missing());
            assert!(r.values()[1].is_missing());
            assert_eq!(r.dtype(), DType::Float64);
        }

        #[test]
        fn diff_valid_skips_missing_predecessors() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
                Scalar::Float64(7.0),
            ])
            .expect("col");
            let r = col.diff_valid().expect("diff_valid");
            assert!(r.values()[0].is_missing()); // null in, null out
            assert!(r.values()[1].is_missing()); // first non-missing -> no prev
            assert!(r.values()[2].is_missing()); // null in
            assert_eq!(r.values()[3], Scalar::Float64(3.0)); // 4 - 1
            assert_eq!(r.values()[4], Scalar::Float64(3.0)); // 7 - 4
        }

        #[test]
        fn diff_valid_empty_column() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            let r = col.diff_valid().expect("diff_valid");
            assert!(r.is_empty());
            assert_eq!(r.dtype(), DType::Float64);
        }

        #[test]
        fn sample_without_replacement_deterministic_by_seed() {
            let col =
                Column::from_values((0..10).map(Scalar::Int64).collect::<Vec<_>>()).expect("col");
            let a = col.sample(3, 42).expect("sample");
            let b = col.sample(3, 42).expect("sample");
            // Same seed → identical ordering.
            assert_eq!(a.values(), b.values());
            assert_eq!(a.len(), 3);
            // All picks lie within the original range.
            for v in a.values() {
                match v {
                    Scalar::Int64(x) => assert!((0..10).contains(x)),
                    other => unreachable!("unexpected value {other:?}"),
                }
            }
        }

        #[test]
        fn sample_different_seeds_likely_differ() {
            let col =
                Column::from_values((0..100).map(Scalar::Int64).collect::<Vec<_>>()).expect("col");
            let a = col.sample(5, 1).expect("sample");
            let b = col.sample(5, 2).expect("sample");
            // Two independent seeds on a 100-element population: collision
            // probability of the full 5-pick tuple is astronomically low.
            assert_ne!(a.values(), b.values());
        }

        #[test]
        fn sample_n_at_or_above_len_returns_clone() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let r = col.sample(10, 42).expect("sample");
            assert_eq!(r.values(), col.values());
        }

        #[test]
        fn first_valid_last_valid_skip_nulls() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(5),
                Scalar::Int64(7),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            assert_eq!(col.first_valid(), Some(2));
            assert_eq!(col.last_valid(), Some(3));
            assert_eq!(col.first_valid_index(), Some(2));
            assert_eq!(col.last_valid_index(), Some(3));
        }

        #[test]
        fn nsmallest_keep_first_breaks_ties_by_earlier_position() {
            let col = Column::from_values(vec![
                Scalar::Int64(2), // pos 0
                Scalar::Int64(1), // pos 1
                Scalar::Int64(1), // pos 2
                Scalar::Int64(3), // pos 3
                Scalar::Int64(1), // pos 4
            ])
            .expect("col");
            let r = col.nsmallest_keep(2, "first").expect("nsmallest_keep");
            // Two 1s: ties broken by earliest position → positions 1,2 → values [1, 1].
            assert_eq!(r.len(), 2);
            assert_eq!(r.values()[0], Scalar::Int64(1));
            assert_eq!(r.values()[1], Scalar::Int64(1));
        }

        #[test]
        fn nsmallest_keep_last_breaks_ties_by_later_position() {
            let col = Column::from_values(vec![
                Scalar::Int64(1), // pos 0
                Scalar::Int64(2),
                Scalar::Int64(1), // pos 2
                Scalar::Int64(3),
                Scalar::Int64(1), // pos 4
            ])
            .expect("col");
            // Three tied 1s; keep=last picks positions 4, 2 (latest two).
            let r = col.nsmallest_keep(2, "last").expect("nsmallest_keep");
            assert_eq!(r.len(), 2);
            assert_eq!(r.values()[0], Scalar::Int64(1));
            assert_eq!(r.values()[1], Scalar::Int64(1));
        }

        #[test]
        fn nsmallest_keep_all_expands_beyond_n_on_ties() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(1),
                Scalar::Int64(2),
            ])
            .expect("col");
            // n=1 but three 1s tied for smallest; keep='all' returns them all.
            let r = col.nsmallest_keep(1, "all").expect("nsmallest_keep");
            assert_eq!(r.len(), 3);
            assert_eq!(r.values()[0], Scalar::Int64(1));
            assert_eq!(r.values()[1], Scalar::Int64(1));
            assert_eq!(r.values()[2], Scalar::Int64(1));
        }

        #[test]
        fn nlargest_keep_mirror_symmetry() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ])
            .expect("col");
            let r = col.nlargest_keep(1, "all").expect("nlargest_keep");
            assert_eq!(r.len(), 2);
            assert_eq!(r.values()[0], Scalar::Int64(3));
            assert_eq!(r.values()[1], Scalar::Int64(3));
        }

        #[test]
        fn nkeep_invalid_keep_errors() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            assert!(col.nsmallest_keep(1, "middle").is_err());
            assert!(col.nlargest_keep(1, "middle").is_err());
        }

        #[test]
        fn nkeep_zero_is_empty_same_dtype() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let r = col.nsmallest_keep(0, "first").expect("nsmallest_keep");
            assert!(r.is_empty());
            assert_eq!(r.dtype(), DType::Int64);
        }

        #[test]
        fn first_valid_last_valid_all_missing_is_none() {
            let col = Column::from_values(vec![
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::Null),
            ])
            .expect("col");
            assert_eq!(col.first_valid(), None);
            assert_eq!(col.last_valid(), None);
            assert_eq!(col.first_valid_index(), None);
            assert_eq!(col.last_valid_index(), None);
        }

        #[test]
        fn rolling_window_sum_empty_column() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            let r = col.rolling_window_sum(3, 1).expect("rolling");
            assert!(r.is_empty());
            assert_eq!(r.dtype(), DType::Float64);
        }

        #[test]
        fn pct_change_negative_periods() {
            let col = Column::from_values(vec![Scalar::Float64(10.0), Scalar::Float64(15.0)])
                .expect("col");
            let r = col.pct_change(-1).expect("pct_change");
            // (10 - 15) / 15 = -1/3
            assert!(
                matches!(&r.values()[0], Scalar::Float64(v) if (*v + 1.0 / 3.0).abs() < 1e-9),
                "expected Float64, got {:?}",
                r.values()[0]
            );
            assert!(r.values()[1].is_missing());
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
        use fp_types::NullKind;

        use super::*;

        #[test]
        fn where_cond_keeps_true_positions() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let cond = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
            ])
            .expect("cond");
            let fill = Scalar::Int64(-1);
            let out = col.where_cond(&cond, &fill).expect("where");
            assert_eq!(col.r#where(&cond, &fill).expect("where"), out);
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(-1));
            assert_eq!(out.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn mask_inverts_where_cond() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
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
            let cond = Column::from_values(vec![Scalar::Bool(true), Scalar::Null(NullKind::NaN)])
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
        fn equals_elementwise_matches_semantic_eq() {
            let a = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(3),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("b");
            let r = a.equals(&b).expect("equals");
            assert_eq!(r.dtype(), DType::Bool);
            assert_eq!(r.values()[0], Scalar::Bool(true));
            assert_eq!(r.values()[1], Scalar::Bool(false));
            // NaN vs NaN → false (pandas semantics)
            assert_eq!(r.values()[2], Scalar::Bool(false));
        }

        #[test]
        fn equals_length_mismatch_errors() {
            let a = Column::from_values(vec![Scalar::Int64(1)]).expect("a");
            let b = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("b");
            assert!(a.equals(&b).is_err());
        }

        #[test]
        fn dot_ignores_missing() {
            let a = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(3.0),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Float64(4.0),
                Scalar::Float64(5.0),
            ])
            .expect("b");
            let r = a.dot(&b).expect("dot");
            // 1*2 + skip + 3*5 = 17
            assert!((r - 17.0).abs() < 1e-9);
        }

        #[test]
        fn dot_non_numeric_errors() {
            let a = Column::from_values(vec![Scalar::Utf8("x".into())]).expect("a");
            let b = Column::from_values(vec![Scalar::Float64(1.0)]).expect("b");
            assert!(a.dot(&b).is_err());
        }

        #[test]
        fn fillna_with_column_fills_missing_positions() {
            let a = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
            ])
            .expect("b");
            let r = a.fillna_with_column(&b).expect("fillna_with_column");
            assert_eq!(r.values()[0], Scalar::Int64(1));
            assert_eq!(r.values()[1], Scalar::Int64(20));
            assert_eq!(r.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn divmod_returns_quotient_and_remainder() {
            let a = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Float64(7.0),
                Scalar::Float64(-5.0),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Float64(3.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])
            .expect("b");
            let (q, r) = a.divmod(&b).expect("divmod");
            // Python-style: floor(10/3)=3, 10 - 3*3 = 1
            match (&q.values()[0], &r.values()[0]) {
                (Scalar::Float64(qv), Scalar::Float64(rv)) => {
                    assert!((qv - 3.0).abs() < 1e-9);
                    assert!((rv - 1.0).abs() < 1e-9);
                }
                other => unreachable!("unexpected {other:?}"),
            }
            // 7 / 2 → q=3, r=1
            match (&q.values()[1], &r.values()[1]) {
                (Scalar::Float64(qv), Scalar::Float64(rv)) => {
                    assert!((qv - 3.0).abs() < 1e-9);
                    assert!((rv - 1.0).abs() < 1e-9);
                }
                other => unreachable!("unexpected {other:?}"),
            }
            // -5 / 3 → q=-2 (floor), r = -5 - (-2*3) = 1
            match (&q.values()[2], &r.values()[2]) {
                (Scalar::Float64(qv), Scalar::Float64(rv)) => {
                    assert!((qv + 2.0).abs() < 1e-9);
                    assert!((rv - 1.0).abs() < 1e-9);
                }
                other => unreachable!("unexpected {other:?}"),
            }
        }

        #[test]
        fn divmod_zero_divisor_yields_null() {
            let a = Column::from_values(vec![Scalar::Float64(10.0)]).expect("a");
            let b = Column::from_values(vec![Scalar::Float64(0.0)]).expect("b");
            let (q, r) = a.divmod(&b).expect("divmod");
            assert!(q.values()[0].is_missing());
            assert!(r.values()[0].is_missing());
        }

        #[test]
        fn divmod_infinite_operands_match_pandas_float_semantics() {
            let a = Column::from_values(vec![
                Scalar::Float64(f64::INFINITY),
                Scalar::Float64(f64::NEG_INFINITY),
                Scalar::Float64(5.0),
                Scalar::Float64(-5.0),
                Scalar::Float64(f64::INFINITY),
            ])
            .expect("a");
            let b = Column::from_values(vec![
                Scalar::Float64(2.0),
                Scalar::Float64(-2.0),
                Scalar::Float64(f64::INFINITY),
                Scalar::Float64(f64::INFINITY),
                Scalar::Float64(f64::INFINITY),
            ])
            .expect("b");

            let (q, r) = a.divmod(&b).expect("divmod");
            assert!(matches!(q.values()[0], Scalar::Float64(v) if v.is_nan()));
            assert!(matches!(r.values()[0], Scalar::Float64(v) if v.is_nan()));
            assert!(matches!(q.values()[1], Scalar::Float64(v) if v.is_nan()));
            assert!(matches!(r.values()[1], Scalar::Float64(v) if v.is_nan()));
            assert_eq!(q.values()[2], Scalar::Float64(0.0));
            assert_eq!(r.values()[2], Scalar::Float64(5.0));
            assert_eq!(q.values()[3], Scalar::Float64(-1.0));
            assert_eq!(r.values()[3], Scalar::Float64(f64::INFINITY));
            assert!(matches!(q.values()[4], Scalar::Float64(v) if v.is_nan()));
            assert!(matches!(r.values()[4], Scalar::Float64(v) if v.is_nan()));
        }

        #[test]
        fn where_cond_series_fills_from_other_column() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let cond = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
            ])
            .expect("cond");
            let other = Column::from_values(vec![
                Scalar::Int64(10),
                Scalar::Int64(20),
                Scalar::Int64(30),
            ])
            .expect("other");
            let out = col.where_cond_series(&cond, &other).expect("where_series");
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(20));
            assert_eq!(out.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn mask_series_fills_from_other_column() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let cond = Column::from_values(vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
            ])
            .expect("cond");
            let other =
                Column::from_values(vec![Scalar::Int64(0), Scalar::Int64(0), Scalar::Int64(0)])
                    .expect("other");
            let out = col.mask_series(&cond, &other).expect("mask_series");
            assert_eq!(out.values()[0], Scalar::Int64(0));
            assert_eq!(out.values()[1], Scalar::Int64(2));
            assert_eq!(out.values()[2], Scalar::Int64(0));
        }

        #[test]
        fn where_cond_series_rejects_non_bool_cond() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let cond = Column::from_values(vec![Scalar::Int64(1)]).expect("cond");
            let other = Column::from_values(vec![Scalar::Int64(0)]).expect("other");
            let err = col.where_cond_series(&cond, &other).unwrap_err();
            assert!(matches!(err, crate::ColumnError::InvalidMaskType { .. }));
        }

        #[test]
        fn replace_values_applies_first_match() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(2),
            ])
            .expect("col");
            let to_replace = vec![Scalar::Int64(2), Scalar::Int64(3)];
            let replacement = vec![Scalar::Int64(20), Scalar::Int64(30)];
            let out = col
                .replace_values(&to_replace, &replacement)
                .expect("replace");
            let alias = col
                .replace(&to_replace, &replacement)
                .expect("replace alias");
            assert_eq!(alias, out);
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(20));
            assert_eq!(out.values()[2], Scalar::Int64(30));
            assert_eq!(out.values()[3], Scalar::Int64(20));
        }

        #[test]
        fn replace_values_can_replace_nulls() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(2),
            ])
            .expect("col");
            let to_replace = vec![Scalar::Null(NullKind::NaN)];
            let replacement = vec![Scalar::Int64(-1)];
            let out = col
                .replace_values(&to_replace, &replacement)
                .expect("replace");
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(-1));
            assert_eq!(out.values()[2], Scalar::Int64(2));
        }

        #[test]
        fn replace_values_length_mismatch_errors() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col
                .replace_values(&[Scalar::Int64(1)], &[Scalar::Int64(2), Scalar::Int64(3)])
                .unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }

        #[test]
        fn nonzero_returns_truthy_positions() {
            let col = Column::from_values(vec![
                Scalar::Int64(0),
                Scalar::Int64(5),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(-3),
                Scalar::Int64(0),
            ])
            .expect("col");
            assert_eq!(col.nonzero(), vec![1, 3]);
        }

        #[test]
        fn nonzero_empty_column_is_empty() {
            let col = Column::from_values(Vec::<Scalar>::new()).expect("col");
            assert!(col.nonzero().is_empty());
        }

        #[test]
        fn where_rejects_length_mismatch() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let cond = Column::from_values(vec![Scalar::Bool(true)]).expect("cond");
            let err = col.where_cond(&cond, &Scalar::Int64(0)).unwrap_err();
            assert!(matches!(err, crate::ColumnError::LengthMismatch { .. }));
        }
    }

    mod nlargest_nsmallest {
        use fp_types::NullKind;

        use super::*;

        #[test]
        fn nlargest_returns_top_n_descending() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(5),
                Scalar::Int64(2),
                Scalar::Int64(4),
            ])
            .expect("col");
            let top = col.nlargest(3).expect("nlargest");
            assert_eq!(top.len(), 3);
            assert_eq!(top.values()[0], Scalar::Int64(5));
            assert_eq!(top.values()[1], Scalar::Int64(4));
            assert_eq!(top.values()[2], Scalar::Int64(3));
        }

        #[test]
        fn nsmallest_returns_bottom_n_ascending() {
            let col = Column::from_values(vec![
                Scalar::Int64(3),
                Scalar::Int64(1),
                Scalar::Int64(5),
                Scalar::Int64(2),
                Scalar::Int64(4),
            ])
            .expect("col");
            let bot = col.nsmallest(2).expect("nsmallest");
            assert_eq!(bot.len(), 2);
            assert_eq!(bot.values()[0], Scalar::Int64(1));
            assert_eq!(bot.values()[1], Scalar::Int64(2));
        }

        #[test]
        fn nlargest_excludes_missing_when_n_fits() {
            let col = Column::from_values(vec![
                Scalar::Int64(5),
                Scalar::Null(NullKind::NaN),
                Scalar::Int64(3),
                Scalar::Int64(7),
            ])
            .expect("col");
            let top = col.nlargest(2).expect("nlargest");
            assert_eq!(top.len(), 2);
            assert_eq!(top.values()[0], Scalar::Int64(7));
            assert_eq!(top.values()[1], Scalar::Int64(5));
        }

        #[test]
        fn nlargest_n_larger_than_length_clamps() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let top = col.nlargest(100).expect("nlargest");
            assert_eq!(top.len(), 2);
        }

        #[test]
        fn nlargest_zero_is_empty_same_dtype() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let top = col.nlargest(0).expect("nlargest");
            assert!(top.is_empty());
            assert_eq!(top.dtype(), DType::Int64);
        }
    }

    mod astype {
        use fp_types::NullKind;

        use super::*;

        #[test]
        fn astype_int_to_float_preserves_values() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let out = col.astype(DType::Float64).expect("astype");
            assert_eq!(out.dtype(), DType::Float64);
            assert_eq!(out.values()[0], Scalar::Float64(1.0));
            assert_eq!(out.values()[1], Scalar::Float64(2.0));
        }

        #[test]
        fn astype_same_dtype_is_noop_clone() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let out = col.astype(DType::Int64).expect("astype");
            assert_eq!(out.values(), col.values());
        }

        #[test]
        fn astype_bool_to_int() {
            let col =
                Column::from_values(vec![Scalar::Bool(true), Scalar::Bool(false)]).expect("col");
            let out = col.astype(DType::Int64).expect("astype");
            assert_eq!(out.dtype(), DType::Int64);
            assert_eq!(out.values()[0], Scalar::Int64(1));
            assert_eq!(out.values()[1], Scalar::Int64(0));
        }

        #[test]
        fn astype_to_utf8_uses_pandas_string_spellings() {
            let bool_col = Column::new(DType::Bool, vec![Scalar::Bool(true), Scalar::Bool(false)])
                .expect("bool col");
            let int_col = Column::new(DType::Int64, vec![Scalar::Int64(-7)]).expect("int col");
            let float_col = Column::new(
                DType::Float64,
                vec![Scalar::Float64(1.0), Scalar::Null(NullKind::NaN)],
            )
            .expect("float col");

            let bool_out = bool_col.astype(DType::Utf8).expect("astype bool");
            let int_out = int_col.astype(DType::Utf8).expect("astype int");
            let float_out = float_col.astype(DType::Utf8).expect("astype float");

            assert_eq!(bool_out.dtype(), DType::Utf8);
            assert_eq!(
                bool_out.values(),
                &[
                    Scalar::Utf8("True".to_owned()),
                    Scalar::Utf8("False".to_owned()),
                ]
            );
            assert_eq!(int_out.values(), &[Scalar::Utf8("-7".to_owned())]);
            assert_eq!(
                float_out.values(),
                &[
                    Scalar::Utf8("1.0".to_owned()),
                    Scalar::Utf8("nan".to_owned()),
                ]
            );
        }

        #[test]
        fn astype_propagates_missing() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)])
                .expect("col");
            let out = col.astype(DType::Float64).expect("astype");
            assert_eq!(out.values()[0], Scalar::Float64(1.0));
            assert!(out.values()[1].is_missing());
        }

        #[test]
        fn astype_finite_float_to_int_truncates_toward_zero() {
            // pandas astype(int64) truncates finite floats toward zero
            // (br-frankenpandas-qcutc); only non-finite values raise.
            let col = Column::from_values(vec![
                Scalar::Float64(1.5),
                Scalar::Float64(2.9),
                Scalar::Float64(-1.5),
                Scalar::Float64(-2.9),
                Scalar::Float64(0.4),
            ])
            .expect("col");
            let out = col.astype(DType::Int64).expect("truncating cast");
            assert_eq!(
                out.values(),
                &[
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(-1),
                    Scalar::Int64(-2),
                    Scalar::Int64(0),
                ]
            );
            // Non-finite still raises.
            let inf = Column::from_values(vec![Scalar::Float64(f64::INFINITY)]).expect("col");
            assert!(matches!(
                inf.astype(DType::Int64).unwrap_err(),
                crate::ColumnError::Type(_)
            ));
        }

        #[test]
        fn new_int64_from_lossy_float_errors_unlike_astype() {
            // The typed constructor with an explicit dtype is STRICT, matching
            // pandas DataFrame(dtype='int64') which raises on a non-integer
            // float — unlike astype which truncates. (br-frankenpandas-8nupg)
            let err = Column::new(DType::Int64, vec![Scalar::Float64(1.5)]).unwrap_err();
            assert!(matches!(
                err,
                crate::ColumnError::Type(fp_types::TypeError::LossyFloatToInt { .. })
            ));
            // Integer-valued floats still coerce fine (1.0 -> 1).
            let ok = Column::new(DType::Int64, vec![Scalar::Float64(2.0)]).expect("integer float");
            assert_eq!(ok.values(), &[Scalar::Int64(2)]);
        }
    }

    mod rank_searchsorted {
        use fp_types::NullKind;

        use super::*;

        #[test]
        fn rank_average_ties_get_midpoint() {
            let col = Column::from_values(vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
            ])
            .expect("col");
            let r = col.rank("average", true).expect("rank");
            assert_eq!(r.values()[0], Scalar::Float64(1.0));
            // Two tied values occupy positions 2 and 3 → avg = 2.5
            assert_eq!(r.values()[1], Scalar::Float64(2.5));
            assert_eq!(r.values()[2], Scalar::Float64(2.5));
            assert_eq!(r.values()[3], Scalar::Float64(4.0));
        }

        #[test]
        fn rank_min_assigns_lowest_tied_rank() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
            ])
            .expect("col");
            let r = col.rank("min", true).expect("rank");
            assert_eq!(r.values()[1], Scalar::Float64(2.0));
            assert_eq!(r.values()[2], Scalar::Float64(2.0));
            assert_eq!(r.values()[3], Scalar::Float64(4.0));
        }

        #[test]
        fn rank_max_assigns_highest_tied_rank() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
            ])
            .expect("col");
            let r = col.rank("max", true).expect("rank");
            assert_eq!(r.values()[1], Scalar::Float64(3.0));
            assert_eq!(r.values()[2], Scalar::Float64(3.0));
            assert_eq!(r.values()[3], Scalar::Float64(4.0));
        }

        #[test]
        fn rank_first_breaks_ties_by_appearance_order() {
            let col =
                Column::from_values(vec![Scalar::Int64(5), Scalar::Int64(3), Scalar::Int64(3)])
                    .expect("col");
            let r = col.rank("first", true).expect("rank");
            // Sorted positions: (1,3), (2,3), (0,5) → ranks 1,2,3
            assert_eq!(r.values()[0], Scalar::Float64(3.0));
            assert_eq!(r.values()[1], Scalar::Float64(1.0));
            assert_eq!(r.values()[2], Scalar::Float64(2.0));
        }

        #[test]
        fn rank_dense_has_no_gaps() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(3),
            ])
            .expect("col");
            let r = col.rank("dense", true).expect("rank");
            assert_eq!(r.values()[0], Scalar::Float64(1.0));
            assert_eq!(r.values()[1], Scalar::Float64(2.0));
            assert_eq!(r.values()[2], Scalar::Float64(2.0));
            assert_eq!(r.values()[3], Scalar::Float64(3.0));
        }

        #[test]
        fn rank_null_inputs_stay_null() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(2.0),
            ])
            .expect("col");
            let r = col.rank("average", true).expect("rank");
            assert_eq!(r.values()[0], Scalar::Float64(1.0));
            assert!(r.values()[1].is_missing());
            assert_eq!(r.values()[2], Scalar::Float64(2.0));
        }

        #[test]
        fn rank_descending_reverses_assignment() {
            let col =
                Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                    .expect("col");
            let r = col.rank("min", false).expect("rank");
            assert_eq!(r.values()[0], Scalar::Float64(3.0));
            assert_eq!(r.values()[1], Scalar::Float64(2.0));
            assert_eq!(r.values()[2], Scalar::Float64(1.0));
        }

        #[test]
        fn rank_invalid_method_errors() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col.rank("bogus", true).unwrap_err();
            assert!(matches!(err, crate::ColumnError::Type(_)));
        }

        #[test]
        fn searchsorted_left_finds_first_insertion() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(5),
            ])
            .expect("col");
            assert_eq!(col.searchsorted(&Scalar::Int64(2), "left").unwrap(), 1);
            assert_eq!(col.searchsorted(&Scalar::Int64(0), "left").unwrap(), 0);
            assert_eq!(col.searchsorted(&Scalar::Int64(6), "left").unwrap(), 4);
        }

        #[test]
        fn searchsorted_right_finds_last_insertion() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(5),
            ])
            .expect("col");
            assert_eq!(col.searchsorted(&Scalar::Int64(2), "right").unwrap(), 3);
        }

        #[test]
        fn searchsorted_rejects_invalid_side() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col.searchsorted(&Scalar::Int64(0), "middle").unwrap_err();
            assert!(matches!(err, crate::ColumnError::Type(_)));
        }

        #[test]
        fn searchsorted_rejects_missing_needle() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col
                .searchsorted(&Scalar::Null(NullKind::NaN), "left")
                .unwrap_err();
            assert!(matches!(err, crate::ColumnError::Type(_)));
        }

        #[test]
        fn searchsorted_treats_trailing_nulls_as_greater() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Null(NullKind::NaN),
            ])
            .expect("col");
            // needle=3 should land at position 2 (before trailing null).
            assert_eq!(col.searchsorted(&Scalar::Int64(3), "left").unwrap(), 2);
        }

        #[test]
        fn searchsorted_values_left_returns_positions_column() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(5),
            ])
            .expect("col");
            let positions = col
                .searchsorted_values(
                    &[Scalar::Int64(0), Scalar::Int64(2), Scalar::Int64(6)],
                    "left",
                )
                .expect("searchsorted");
            assert_eq!(positions.dtype(), DType::Int64);
            assert_eq!(
                positions.values(),
                &[Scalar::Int64(0), Scalar::Int64(1), Scalar::Int64(4)]
            );
        }

        #[test]
        fn searchsorted_values_right_returns_positions_column() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
                Scalar::Int64(5),
            ])
            .expect("col");
            let positions = col
                .searchsorted_values(
                    &[Scalar::Int64(0), Scalar::Int64(2), Scalar::Int64(6)],
                    "right",
                )
                .expect("searchsorted");
            assert_eq!(
                positions.values(),
                &[Scalar::Int64(0), Scalar::Int64(3), Scalar::Int64(4)]
            );
        }

        #[test]
        fn searchsorted_values_rejects_invalid_side() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col
                .searchsorted_values(&[Scalar::Int64(0)], "middle")
                .unwrap_err();
            assert!(matches!(err, crate::ColumnError::Type(_)));
        }

        #[test]
        fn searchsorted_values_rejects_missing_needles() {
            let col = Column::from_values(vec![Scalar::Int64(1)]).expect("col");
            let err = col
                .searchsorted_values(&[Scalar::Null(NullKind::NaN)], "left")
                .unwrap_err();
            assert!(matches!(err, crate::ColumnError::Type(_)));
        }

        #[test]
        fn searchsorted_with_sorter_uses_argsort_permutation() {
            let col = Column::from_values(vec![
                Scalar::Int64(5),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ])
            .expect("col");
            let sorter = col.argsort();
            assert_eq!(
                col.searchsorted_with_sorter(&Scalar::Int64(2), "left", &sorter)
                    .unwrap(),
                1
            );
            assert_eq!(
                col.searchsorted_with_sorter(&Scalar::Int64(2), "right", &sorter)
                    .unwrap(),
                3
            );
            assert_eq!(
                col.searchsorted_with_sorter(&Scalar::Int64(6), "left", &sorter)
                    .unwrap(),
                4
            );
        }

        #[test]
        fn searchsorted_values_with_sorter_returns_positions_column() {
            let col = Column::from_values(vec![
                Scalar::Int64(5),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(2),
            ])
            .expect("col");
            let sorter = col.argsort();
            let positions = col
                .searchsorted_values_with_sorter(
                    &[Scalar::Int64(0), Scalar::Int64(2), Scalar::Int64(6)],
                    "left",
                    &sorter,
                )
                .expect("searchsorted");
            assert_eq!(
                positions.values(),
                &[Scalar::Int64(0), Scalar::Int64(1), Scalar::Int64(4)]
            );
        }

        #[test]
        fn searchsorted_with_sorter_rejects_length_mismatch() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let err = col
                .searchsorted_with_sorter(&Scalar::Int64(1), "left", &[0])
                .unwrap_err();
            assert!(matches!(
                err,
                crate::ColumnError::LengthMismatch { left: 2, right: 1 }
            ));
        }

        #[test]
        fn searchsorted_with_sorter_rejects_duplicate_or_oob_indices() {
            let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).expect("col");
            let duplicate = col
                .searchsorted_with_sorter(&Scalar::Int64(1), "left", &[0, 0])
                .unwrap_err();
            assert!(matches!(
                duplicate,
                crate::ColumnError::InvalidSorter { .. }
            ));

            let out_of_bounds = col
                .searchsorted_with_sorter(&Scalar::Int64(1), "left", &[0, 2])
                .unwrap_err();
            assert!(matches!(
                out_of_bounds,
                crate::ColumnError::InvalidSorter { .. }
            ));
        }
    }

    mod value_counts {
        use fp_types::NullKind;

        use super::*;

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

        #[test]
        fn python_mod_f64_handles_infinity_divisor() {
            use crate::python_mod_f64;

            assert_eq!(python_mod_f64(5.0, f64::INFINITY), 5.0);
            assert_eq!(python_mod_f64(-5.0, f64::INFINITY), f64::INFINITY);
            assert_eq!(python_mod_f64(5.0, f64::NEG_INFINITY), f64::NEG_INFINITY);
            assert_eq!(python_mod_f64(-5.0, f64::NEG_INFINITY), -5.0);
            assert_eq!(python_mod_f64(0.0, f64::INFINITY), 0.0);
            assert!(python_mod_f64(0.0, f64::NEG_INFINITY).is_sign_negative());
            assert_eq!(python_mod_f64(-0.0, f64::INFINITY), 0.0);
            assert_eq!(python_mod_f64(-0.0, f64::NEG_INFINITY), -0.0);
            assert!(python_mod_f64(f64::NAN, f64::INFINITY).is_nan());
            assert!(python_mod_f64(f64::NAN, f64::NEG_INFINITY).is_nan());
            assert!(python_mod_f64(f64::INFINITY, f64::INFINITY).is_nan());
            assert!(python_mod_f64(f64::NEG_INFINITY, f64::NEG_INFINITY).is_nan());
        }

        #[test]
        fn python_floor_div_f64_handles_infinite_operands() {
            use crate::python_floor_div_f64;

            assert_eq!(python_floor_div_f64(5.0, f64::INFINITY), 0.0);
            assert_eq!(python_floor_div_f64(-5.0, f64::INFINITY), -1.0);
            assert_eq!(python_floor_div_f64(5.0, f64::NEG_INFINITY), -1.0);
            assert_eq!(python_floor_div_f64(-5.0, f64::NEG_INFINITY), 0.0);
            assert_eq!(python_floor_div_f64(0.0, f64::INFINITY), 0.0);
            assert!(python_floor_div_f64(-0.0, f64::INFINITY).is_sign_negative());
            assert!(python_floor_div_f64(0.0, f64::NEG_INFINITY).is_sign_negative());
            assert_eq!(python_floor_div_f64(-0.0, f64::NEG_INFINITY), 0.0);
            assert!(python_floor_div_f64(f64::INFINITY, 2.0).is_nan());
            assert!(python_floor_div_f64(f64::NEG_INFINITY, -2.0).is_nan());
            assert!(python_floor_div_f64(f64::INFINITY, f64::INFINITY).is_nan());
        }

        #[test]
        fn histogram_counts_values_in_bins() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.5),
                Scalar::Float64(1.5),
                Scalar::Float64(2.5),
                Scalar::Float64(1.2),
                Scalar::Float64(2.8),
            ])
            .unwrap();
            let edges = vec![0.0, 1.0, 2.0, 3.0];
            let counts = col.histogram(&edges).unwrap();
            assert_eq!(
                counts.values(),
                &[
                    Scalar::Int64(1), // [0, 1): 0.5
                    Scalar::Int64(2), // [1, 2): 1.5, 1.2
                    Scalar::Int64(2), // [2, 3]: 2.5, 2.8
                ]
            );
        }

        #[test]
        fn histogram_auto_creates_bins() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
                Scalar::Float64(4.0),
            ])
            .unwrap();
            let (counts, edges) = col.histogram_auto(3).unwrap();
            assert_eq!(counts.len(), 3);
            assert_eq!(edges.len(), 4);
            assert!((edges[0] - 1.0).abs() < 1e-10);
            assert!((edges[3] - 4.0).abs() < 1e-10);
        }

        #[test]
        fn histogram_auto_constant_values_extends_range() {
            let col = Column::from_values(vec![
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
            ])
            .unwrap();
            let (counts, edges) = col.histogram_auto(2).unwrap();
            assert_eq!(counts.len(), 2);
            assert!(edges[0] < 5.0);
            assert!(edges[2] > 5.0);
        }

        #[test]
        fn hanning_window_shape() {
            let win = Column::hanning(5).unwrap();
            assert_eq!(win.len(), 5);
            // Endpoints should be 0
            assert!((win.values()[0].to_f64().unwrap()).abs() < 1e-10);
            assert!((win.values()[4].to_f64().unwrap()).abs() < 1e-10);
            // Center should be 1
            assert!((win.values()[2].to_f64().unwrap() - 1.0).abs() < 1e-10);
        }

        #[test]
        fn hamming_window_shape() {
            let win = Column::hamming(5).unwrap();
            assert_eq!(win.len(), 5);
            // Hamming endpoints are ~0.08, not 0
            let v0 = win.values()[0].to_f64().unwrap();
            assert!(v0 > 0.07 && v0 < 0.09);
        }

        #[test]
        fn bartlett_window_triangular() {
            let win = Column::bartlett(5).unwrap();
            assert_eq!(win.len(), 5);
            // Endpoints should be 0
            assert!((win.values()[0].to_f64().unwrap()).abs() < 1e-10);
            assert!((win.values()[4].to_f64().unwrap()).abs() < 1e-10);
            // Center should be 1
            assert!((win.values()[2].to_f64().unwrap() - 1.0).abs() < 1e-10);
        }

        #[test]
        fn convolve_full_mode() {
            let a = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(3.0),
            ])
            .unwrap();
            let v = Column::from_values(vec![Scalar::Float64(1.0), Scalar::Float64(1.0)]).unwrap();
            let result = a.convolve(&v, "full").unwrap();
            // Full convolution: [1*1, 1*1+2*1, 2*1+3*1, 3*1] = [1, 3, 5, 3]
            assert_eq!(result.len(), 4);
            assert!((result.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            assert!((result.values()[1].to_f64().unwrap() - 3.0).abs() < 1e-10);
            assert!((result.values()[2].to_f64().unwrap() - 5.0).abs() < 1e-10);
            assert!((result.values()[3].to_f64().unwrap() - 3.0).abs() < 1e-10);
        }

        #[test]
        fn geomspace_creates_geometric_progression() {
            let col = Column::geomspace(1.0, 1000.0, 4).unwrap();
            assert_eq!(col.len(), 4);
            assert!((col.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            assert!((col.values()[1].to_f64().unwrap() - 10.0).abs() < 1e-10);
            assert!((col.values()[2].to_f64().unwrap() - 100.0).abs() < 1e-10);
            assert!((col.values()[3].to_f64().unwrap() - 1000.0).abs() < 1e-10);
        }

        #[test]
        fn nan_to_num_replaces_special_values() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(f64::NAN),
                Scalar::Float64(f64::INFINITY),
                Scalar::Float64(f64::NEG_INFINITY),
            ])
            .unwrap();
            let result = col.nan_to_num().unwrap();
            assert!((result.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            assert!((result.values()[1].to_f64().unwrap() - 0.0).abs() < 1e-10);
            assert_eq!(result.values()[2].to_f64().unwrap(), f64::MAX);
            assert_eq!(result.values()[3].to_f64().unwrap(), f64::MIN);
        }

        #[test]
        fn rint_rounds_to_nearest_even() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.5),
                Scalar::Float64(1.5),
                Scalar::Float64(2.5),
                Scalar::Float64(3.5),
            ])
            .unwrap();
            let result = col.rint().unwrap();
            // Banker's rounding: 0.5->0, 1.5->2, 2.5->2, 3.5->4
            assert!((result.values()[0].to_f64().unwrap() - 0.0).abs() < 1e-10);
            assert!((result.values()[1].to_f64().unwrap() - 2.0).abs() < 1e-10);
            assert!((result.values()[2].to_f64().unwrap() - 2.0).abs() < 1e-10);
            assert!((result.values()[3].to_f64().unwrap() - 4.0).abs() < 1e-10);
        }

        #[test]
        fn ldexp_multiplies_by_power_of_two() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(0.5),
            ])
            .unwrap();
            let result = col.ldexp(3).unwrap(); // multiply by 2^3 = 8
            assert!((result.values()[0].to_f64().unwrap() - 8.0).abs() < 1e-10);
            assert!((result.values()[1].to_f64().unwrap() - 16.0).abs() < 1e-10);
            assert!((result.values()[2].to_f64().unwrap() - 4.0).abs() < 1e-10);
        }

        #[test]
        fn modf_splits_integer_and_fraction() {
            let col = Column::from_values(vec![
                Scalar::Float64(3.5),
                Scalar::Float64(-2.25),
                Scalar::Float64(1.0),
            ])
            .unwrap();
            let (frac, int) = col.modf().unwrap();
            assert!((frac.values()[0].to_f64().unwrap() - 0.5).abs() < 1e-10);
            assert!((int.values()[0].to_f64().unwrap() - 3.0).abs() < 1e-10);
            assert!((frac.values()[1].to_f64().unwrap() - (-0.25)).abs() < 1e-10);
            assert!((int.values()[1].to_f64().unwrap() - (-2.0)).abs() < 1e-10);
            assert!((frac.values()[2].to_f64().unwrap() - 0.0).abs() < 1e-10);
            assert!((int.values()[2].to_f64().unwrap() - 1.0).abs() < 1e-10);
        }

        #[test]
        fn spacing_returns_ulp() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(-1.0),
                Scalar::Float64(0.0),
            ])
            .unwrap();
            let result = col.spacing().unwrap();
            // Spacing at 1.0 is about 2.2e-16
            let s1 = result.values()[0].to_f64().unwrap();
            assert!(s1 > 0.0 && s1 < 1e-15);
            // Spacing is symmetric for negative numbers
            let s_neg1 = result.values()[1].to_f64().unwrap();
            assert!((s1 - s_neg1).abs() < 1e-20);
            // Spacing at 0 is smallest denormal (not MIN_POSITIVE which is normalized)
            assert_eq!(result.values()[2].to_f64().unwrap(), f64::from_bits(1));
        }

        #[test]
        fn frexp_decomposes_floats() {
            let col = Column::from_values(vec![
                Scalar::Float64(4.0),
                Scalar::Float64(0.5),
                Scalar::Float64(-8.0),
                Scalar::Float64(0.0),
            ])
            .unwrap();
            let (mant, exp) = col.frexp().unwrap();
            // 4.0 = 0.5 * 2^3
            assert!((mant.values()[0].to_f64().unwrap() - 0.5).abs() < 1e-10);
            assert_eq!(exp.values()[0].to_i64().unwrap(), 3);
            // 0.5 = 0.5 * 2^0
            assert!((mant.values()[1].to_f64().unwrap() - 0.5).abs() < 1e-10);
            assert_eq!(exp.values()[1].to_i64().unwrap(), 0);
            // -8.0 = -0.5 * 2^4
            assert!((mant.values()[2].to_f64().unwrap() - (-0.5)).abs() < 1e-10);
            assert_eq!(exp.values()[2].to_i64().unwrap(), 4);
            // 0.0 = 0.0 * 2^0
            assert!((mant.values()[3].to_f64().unwrap() - 0.0).abs() < 1e-10);
            assert_eq!(exp.values()[3].to_i64().unwrap(), 0);
        }

        #[test]
        fn nextafter_returns_adjacent_floats() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(1.0),
                Scalar::Float64(1.0),
            ])
            .unwrap();
            let toward = Column::from_values(vec![
                Scalar::Float64(1.0),
                Scalar::Float64(2.0),
                Scalar::Float64(0.0),
            ])
            .unwrap();
            let result = col.nextafter(&toward).unwrap();
            // nextafter(0, 1) = smallest positive denormal (not MIN_POSITIVE which is normalized)
            assert_eq!(result.values()[0].to_f64().unwrap(), f64::from_bits(1));
            // nextafter(1, 2) > 1
            let r1 = result.values()[1].to_f64().unwrap();
            assert!(r1 > 1.0 && r1 < 1.0 + 1e-15);
            // nextafter(1, 0) < 1
            let r2 = result.values()[2].to_f64().unwrap();
            assert!(r2 < 1.0 && r2 > 1.0 - 1e-15);
        }

        #[test]
        fn isneginf_isposinf_detect_infinities() {
            let col = Column::from_values(vec![
                Scalar::Float64(f64::NEG_INFINITY),
                Scalar::Float64(f64::INFINITY),
                Scalar::Float64(1.0),
                Scalar::Float64(f64::NAN),
            ])
            .unwrap();
            let neginf = col.isneginf().unwrap();
            let posinf = col.isposinf().unwrap();
            assert_eq!(neginf.values()[0], Scalar::Bool(true));
            assert_eq!(neginf.values()[1], Scalar::Bool(false));
            assert_eq!(neginf.values()[2], Scalar::Bool(false));
            assert_eq!(neginf.values()[3], Scalar::Bool(false));
            assert_eq!(posinf.values()[0], Scalar::Bool(false));
            assert_eq!(posinf.values()[1], Scalar::Bool(true));
            assert_eq!(posinf.values()[2], Scalar::Bool(false));
            assert_eq!(posinf.values()[3], Scalar::Bool(false));
        }

        #[test]
        fn exp2_computes_power_of_two() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(1.0),
                Scalar::Float64(3.0),
                Scalar::Float64(-1.0),
            ])
            .unwrap();
            let result = col.exp2().unwrap();
            assert!((result.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            assert!((result.values()[1].to_f64().unwrap() - 2.0).abs() < 1e-10);
            assert!((result.values()[2].to_f64().unwrap() - 8.0).abs() < 1e-10);
            assert!((result.values()[3].to_f64().unwrap() - 0.5).abs() < 1e-10);
        }

        #[test]
        fn sinc_computes_sinc_function() {
            let col = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(1.0),
                Scalar::Float64(0.5),
            ])
            .unwrap();
            let result = col.sinc().unwrap();
            // sinc(0) = 1
            assert!((result.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            // sinc(1) = sin(pi)/pi = 0
            assert!(result.values()[1].to_f64().unwrap().abs() < 1e-10);
            // sinc(0.5) = sin(pi/2)/(pi/2) = 2/pi ≈ 0.6366
            let expected = 2.0 / std::f64::consts::PI;
            assert!((result.values()[2].to_f64().unwrap() - expected).abs() < 1e-10);
        }

        #[test]
        fn logaddexp_computes_stable_log_sum() {
            let x = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(1.0),
                Scalar::Float64(-1000.0),
            ])
            .unwrap();
            let y = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(2.0),
                Scalar::Float64(-1000.0),
            ])
            .unwrap();
            let result = x.logaddexp(&y).unwrap();
            // log(exp(0) + exp(0)) = log(2) ≈ 0.693
            assert!((result.values()[0].to_f64().unwrap() - std::f64::consts::LN_2).abs() < 1e-10);
            // log(exp(1) + exp(2)) ≈ 2.313
            let expected1 = (1.0_f64.exp() + 2.0_f64.exp()).ln();
            assert!((result.values()[1].to_f64().unwrap() - expected1).abs() < 1e-10);
            // log(exp(-1000) + exp(-1000)) = -1000 + log(2)
            let expected2 = -1000.0 + std::f64::consts::LN_2;
            assert!((result.values()[2].to_f64().unwrap() - expected2).abs() < 1e-8);
        }

        #[test]
        fn logaddexp2_computes_stable_log2_sum() {
            let x = Column::from_values(vec![Scalar::Float64(0.0), Scalar::Float64(1.0)]).unwrap();
            let y = Column::from_values(vec![Scalar::Float64(0.0), Scalar::Float64(1.0)]).unwrap();
            let result = x.logaddexp2(&y).unwrap();
            // log2(2^0 + 2^0) = log2(2) = 1
            assert!((result.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            // log2(2^1 + 2^1) = log2(4) = 2
            assert!((result.values()[1].to_f64().unwrap() - 2.0).abs() < 1e-10);
        }

        #[test]
        fn roll_shifts_elements_circularly() {
            let col = Column::from_values(vec![
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(3),
                Scalar::Int64(4),
                Scalar::Int64(5),
            ])
            .unwrap();
            // Roll right by 2: [4, 5, 1, 2, 3]
            let r1 = col.roll(2).unwrap();
            assert_eq!(r1.values()[0].to_i64().unwrap(), 4);
            assert_eq!(r1.values()[1].to_i64().unwrap(), 5);
            assert_eq!(r1.values()[2].to_i64().unwrap(), 1);
            // Roll left by 2: [3, 4, 5, 1, 2]
            let r2 = col.roll(-2).unwrap();
            assert_eq!(r2.values()[0].to_i64().unwrap(), 3);
            assert_eq!(r2.values()[1].to_i64().unwrap(), 4);
            assert_eq!(r2.values()[2].to_i64().unwrap(), 5);
            // Roll by 0 or length is no-op
            let r3 = col.roll(0).unwrap();
            assert_eq!(r3.values()[0].to_i64().unwrap(), 1);
            let r4 = col.roll(5).unwrap();
            assert_eq!(r4.values()[0].to_i64().unwrap(), 1);
        }

        #[test]
        fn trim_zeros_removes_leading_trailing() {
            let col = Column::from_values(vec![
                Scalar::Int64(0),
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(2),
                Scalar::Int64(0),
            ])
            .unwrap();
            // Trim both
            let r1 = col.trim_zeros("fb").unwrap();
            assert_eq!(r1.len(), 2);
            assert_eq!(r1.values()[0].to_i64().unwrap(), 1);
            assert_eq!(r1.values()[1].to_i64().unwrap(), 2);
            // Trim front only
            let r2 = col.trim_zeros("f").unwrap();
            assert_eq!(r2.len(), 3);
            assert_eq!(r2.values()[0].to_i64().unwrap(), 1);
            // Trim back only
            let r3 = col.trim_zeros("b").unwrap();
            assert_eq!(r3.len(), 4);
            assert_eq!(r3.values()[3].to_i64().unwrap(), 2);
        }

        #[test]
        fn around_rounds_to_decimals() {
            let col = Column::from_values(vec![
                Scalar::Float64(1.234),
                Scalar::Float64(5.678),
                Scalar::Float64(3.5),
            ])
            .unwrap();
            // Round to 2 decimals
            let r1 = col.around(2).unwrap();
            assert!((r1.values()[0].to_f64().unwrap() - 1.23).abs() < 1e-10);
            assert!((r1.values()[1].to_f64().unwrap() - 5.68).abs() < 1e-10);
            assert!((r1.values()[2].to_f64().unwrap() - 3.5).abs() < 1e-10);
            // Round to 0 decimals
            let r2 = col.around(0).unwrap();
            assert!((r2.values()[0].to_f64().unwrap() - 1.0).abs() < 1e-10);
            assert!((r2.values()[1].to_f64().unwrap() - 6.0).abs() < 1e-10);
            // Round to -1 (tens) - np.around uses round-half-to-even (banker's)
            let col2 = Column::from_values(vec![
                Scalar::Float64(15.0),
                Scalar::Float64(24.0),
                Scalar::Float64(35.0),
            ])
            .unwrap();
            let r3 = col2.around(-1).unwrap();
            assert!((r3.values()[0].to_f64().unwrap() - 20.0).abs() < 1e-10);
            assert!((r3.values()[1].to_f64().unwrap() - 20.0).abs() < 1e-10);
            assert!((r3.values()[2].to_f64().unwrap() - 40.0).abs() < 1e-10);
        }

        #[test]
        fn around_uses_numpy_half_even_ties() {
            // np.around is round-half-to-EVEN, matching pd.Series.round. The old
            // implementation used f64::round (half away from zero), diverging on
            // exact .5 ties: np.around([0.5,1.5,2.5,3.5]) == [0,2,2,4].
            let col = Column::from_values(vec![
                Scalar::Float64(0.5),
                Scalar::Float64(1.5),
                Scalar::Float64(2.5),
                Scalar::Float64(3.5),
                Scalar::Float64(-2.5),
            ])
            .unwrap();
            let r = col.around(0).unwrap();
            let got: Vec<f64> = r.values().iter().map(|v| v.to_f64().unwrap()).collect();
            assert_eq!(got, vec![0.0, 2.0, 2.0, 4.0, -2.0]);

            // Negative decimals: np.around([15,25,35], -1) == [20,20,40] (25->20).
            let tens = Column::from_values(vec![
                Scalar::Float64(15.0),
                Scalar::Float64(25.0),
                Scalar::Float64(35.0),
            ])
            .unwrap();
            let rt = tens.around(-1).unwrap();
            let gott: Vec<f64> = rt.values().iter().map(|v| v.to_f64().unwrap()).collect();
            assert_eq!(gott, vec![20.0, 20.0, 40.0]);

            // around must agree with round (both banker's).
            assert_eq!(
                col.around(0).unwrap().values(),
                col.round(0).unwrap().values()
            );
        }

        #[test]
        fn unwrap_removes_phase_discontinuities() {
            use std::f64::consts::PI;
            let col = Column::from_values(vec![
                Scalar::Float64(0.0),
                Scalar::Float64(PI * 0.9),
                Scalar::Float64(-PI * 0.9), // jump > PI
                Scalar::Float64(0.0),
            ])
            .unwrap();
            let result = col.unwrap(None).unwrap();
            // After unwrap, the sequence should be continuous
            assert!((result.values()[0].to_f64().unwrap() - 0.0).abs() < 1e-10);
            // Second value unchanged
            assert!((result.values()[1].to_f64().unwrap() - PI * 0.9).abs() < 1e-10);
            // Third value should be unwrapped (added 2*PI)
            let v2 = result.values()[2].to_f64().unwrap();
            let v1 = result.values()[1].to_f64().unwrap();
            assert!((v2 - v1).abs() < PI); // difference should now be < PI
        }
    }

    // ── Nullable Int64/Bool column tests (br-frankenpandas-rg8ys.6.4) ────

    #[test]
    fn column_has_nulls_detects_missing_values() {
        let col_with_null = Column::from_values(vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
        ])
        .unwrap();
        assert!(col_with_null.has_nulls());

        let col_no_null =
            Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
                .unwrap();
        assert!(!col_no_null.has_nulls());
    }

    #[test]
    fn column_promote_to_nullable_upgrades_dtype() {
        let col = Column::new(
            DType::Int64,
            vec![
                Scalar::Int64(1),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(3),
            ],
        )
        .unwrap();
        assert_eq!(col.dtype(), DType::Int64);
        assert!(col.has_nulls());

        let promoted = col.promote_to_nullable();
        assert_eq!(promoted.dtype(), DType::Int64Nullable);
        assert_eq!(promoted.len(), 3);
    }

    #[test]
    fn column_promote_to_nullable_noop_without_nulls() {
        let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])
            .unwrap();
        let promoted = col.promote_to_nullable();
        // No nulls, so dtype stays Int64
        assert_eq!(promoted.dtype(), DType::Int64);
    }

    #[test]
    fn column_with_dtype_changes_metadata() {
        let col = Column::new(DType::Int64, vec![Scalar::Int64(42)]).unwrap();
        let changed = col.with_dtype(DType::Int64Nullable);
        assert_eq!(changed.dtype(), DType::Int64Nullable);
        assert_eq!(changed.values()[0], Scalar::Int64(42));
    }

    #[test]
    fn nullable_int64_from_scalars_preserves_storage() {
        use super::ColumnData;
        let values = vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)];
        let data = ColumnData::from_scalars(&values, DType::Int64Nullable);
        assert!(matches!(&data, ColumnData::Int64(_)));
        if let ColumnData::Int64(arr) = data {
            assert_eq!(arr, vec![1, 2, 3]);
        }
    }

    #[test]
    fn typed_all_valid_constructors_keep_single_typed_backing() {
        let ints = Column::from_i64_values(vec![1, 2, 3]);
        assert_eq!(ints.dtype(), DType::Int64);
        assert!(ints.validity.all());
        assert!(ints.data.is_none());
        assert_eq!(ints.as_i64_slice(), Some([1, 2, 3].as_slice()));
        assert_eq!(
            ints.values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );

        let floats = Column::from_f64_values(vec![1.5, -0.0, f64::INFINITY]);
        assert_eq!(floats.dtype(), DType::Float64);
        assert!(floats.validity.all());
        assert!(floats.data.is_none());
        assert_eq!(
            floats.as_f64_slice().map(|values| {
                values
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>()
            }),
            Some(vec![
                1.5f64.to_bits(),
                (-0.0f64).to_bits(),
                f64::INFINITY.to_bits()
            ])
        );
        assert_eq!(
            floats.values(),
            &[
                Scalar::Float64(1.5),
                Scalar::Float64(-0.0),
                Scalar::Float64(f64::INFINITY)
            ]
        );
    }

    #[test]
    fn repeated_slice_int64_column_matches_eager_materialization() {
        let lazy = Column::from_i64_repeated_slices(
            vec![10, 11, 12, 20, 21],
            vec![(0, 3), (3, 2), (0, 3)],
        );
        let eager = Column::from_i64_values(vec![10, 11, 12, 20, 21, 10, 11, 12]);

        assert_eq!(lazy.dtype(), DType::Int64);
        assert!(lazy.validity.all());
        assert_eq!(lazy.len(), eager.len());
        assert_eq!(lazy.as_i64_slice(), eager.as_i64_slice());
        assert_eq!(lazy.values(), eager.values());
        assert_eq!(lazy, eager);
    }
}
