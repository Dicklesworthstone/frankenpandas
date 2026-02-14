#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DType {
    Null,
    Bool,
    Int64,
    Float64,
    Utf8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullKind {
    Null,
    NaN,
    NaT,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Scalar {
    Null(NullKind),
    Bool(bool),
    Int64(i64),
    Float64(f64),
    Utf8(String),
}

impl Scalar {
    #[must_use]
    pub fn dtype(&self) -> DType {
        match self {
            Self::Null(_) => DType::Null,
            Self::Bool(_) => DType::Bool,
            Self::Int64(_) => DType::Int64,
            Self::Float64(_) => DType::Float64,
            Self::Utf8(_) => DType::Utf8,
        }
    }

    #[must_use]
    pub fn is_missing(&self) -> bool {
        match self {
            Self::Null(_) => true,
            Self::Float64(v) => v.is_nan(),
            _ => false,
        }
    }

    #[must_use]
    pub fn is_nan(&self) -> bool {
        matches!(self, Self::Null(NullKind::NaN)) || matches!(self, Self::Float64(v) if v.is_nan())
    }

    #[must_use]
    pub fn missing_for_dtype(dtype: DType) -> Self {
        match dtype {
            DType::Float64 => Self::Null(NullKind::NaN),
            DType::Null => Self::Null(NullKind::Null),
            DType::Bool | DType::Int64 | DType::Utf8 => Self::Null(NullKind::Null),
        }
    }

    #[must_use]
    pub fn semantic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Float64(a), Self::Float64(b)) => (a.is_nan() && b.is_nan()) || (a == b),
            (Self::Null(NullKind::NaN), Self::Float64(v))
            | (Self::Float64(v), Self::Null(NullKind::NaN)) => v.is_nan(),
            _ => self == other,
        }
    }

    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null(_))
    }

    #[must_use]
    pub fn is_na(&self) -> bool {
        self.is_missing()
    }

    #[must_use]
    pub fn coalesce(&self, other: &Self) -> Self {
        if self.is_missing() { other.clone() } else { self.clone() }
    }

    pub fn to_f64(&self) -> Result<f64, TypeError> {
        match self {
            Self::Bool(v) => Ok(if *v { 1.0 } else { 0.0 }),
            Self::Int64(v) => Ok(*v as f64),
            Self::Float64(v) => Ok(*v),
            Self::Null(kind) => Err(TypeError::ValueIsMissing { kind: *kind }),
            Self::Utf8(v) => Err(TypeError::NonNumericValue {
                value: v.clone(),
                dtype: DType::Utf8,
            }),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TypeError {
    #[error("dtype coercion from {left:?} to {right:?} has no compatible common type")]
    IncompatibleDtypes { left: DType, right: DType },
    #[error("cannot cast scalar of dtype {from:?} to {to:?}")]
    InvalidCast { from: DType, to: DType },
    #[error("cannot cast float {value} to int64 without loss")]
    LossyFloatToInt { value: f64 },
    #[error("expected 0/1 for bool cast from int64 but found {value}")]
    InvalidBoolInt { value: i64 },
    #[error("expected 0.0/1.0 for bool cast from float64 but found {value}")]
    InvalidBoolFloat { value: f64 },
    #[error("value {value:?} has non-numeric dtype {dtype:?}")]
    NonNumericValue { value: String, dtype: DType },
    #[error("value is missing ({kind:?})")]
    ValueIsMissing { kind: NullKind },
}

pub fn common_dtype(left: DType, right: DType) -> Result<DType, TypeError> {
    use DType::{Bool, Float64, Int64, Null, Utf8};

    let out = match (left, right) {
        (a, b) if a == b => a,
        (Null, other) | (other, Null) => other,
        (Bool, Int64) | (Int64, Bool) => Int64,
        (Bool, Float64) | (Float64, Bool) => Float64,
        (Int64, Float64) | (Float64, Int64) => Float64,
        (Utf8, Utf8) => Utf8,
        _ => return Err(TypeError::IncompatibleDtypes { left, right }),
    };

    Ok(out)
}

pub fn infer_dtype(values: &[Scalar]) -> Result<DType, TypeError> {
    let mut current = DType::Null;
    for value in values {
        current = common_dtype(current, value.dtype())?;
    }
    Ok(current)
}

/// Cast a scalar to a target dtype, taking ownership to avoid redundant clones
/// when the value already has the correct type (AG-03: identity-cast skip).
pub fn cast_scalar_owned(value: Scalar, target: DType) -> Result<Scalar, TypeError> {
    let from = value.dtype();
    if matches!(value, Scalar::Null(_)) {
        return Ok(Scalar::missing_for_dtype(target));
    }
    if from == target {
        return Ok(value);
    }

    // Note: identity casts (from == target) are handled above, so same-type
    // arms are omitted from the match below.
    match target {
        DType::Null => Ok(Scalar::Null(NullKind::Null)),
        DType::Bool => match &value {
            Scalar::Int64(v) => match *v {
                0 => Ok(Scalar::Bool(false)),
                1 => Ok(Scalar::Bool(true)),
                _ => Err(TypeError::InvalidBoolInt { value: *v }),
            },
            Scalar::Float64(v) => {
                if *v == 0.0 {
                    Ok(Scalar::Bool(false))
                } else if *v == 1.0 {
                    Ok(Scalar::Bool(true))
                } else {
                    Err(TypeError::InvalidBoolFloat { value: *v })
                }
            }
            _ => Err(TypeError::InvalidCast { from, to: target }),
        },
        DType::Int64 => match &value {
            Scalar::Bool(v) => Ok(Scalar::Int64(i64::from(*v))),
            Scalar::Float64(v) => {
                if !v.is_finite() || *v != v.trunc() {
                    return Err(TypeError::LossyFloatToInt { value: *v });
                }
                if *v < i64::MIN as f64 || *v > i64::MAX as f64 {
                    return Err(TypeError::LossyFloatToInt { value: *v });
                }
                Ok(Scalar::Int64(*v as i64))
            }
            _ => Err(TypeError::InvalidCast { from, to: target }),
        },
        DType::Float64 => match &value {
            Scalar::Bool(v) => Ok(Scalar::Float64(if *v { 1.0 } else { 0.0 })),
            Scalar::Int64(v) => Ok(Scalar::Float64(*v as f64)),
            _ => Err(TypeError::InvalidCast { from, to: target }),
        },
        DType::Utf8 => Err(TypeError::InvalidCast { from, to: target }),
    }
}

/// Cast a scalar reference to a target dtype (clones only when conversion is needed).
pub fn cast_scalar(value: &Scalar, target: DType) -> Result<Scalar, TypeError> {
    cast_scalar_owned(value.clone(), target)
}

// ── Missingness utilities ──────────────────────────────────────────────

pub fn isna(values: &[Scalar]) -> Vec<bool> {
    values.iter().map(Scalar::is_missing).collect()
}

pub fn notna(values: &[Scalar]) -> Vec<bool> {
    values.iter().map(|v| !v.is_missing()).collect()
}

pub fn count_na(values: &[Scalar]) -> usize {
    values.iter().filter(|v| v.is_missing()).count()
}

pub fn fill_na(values: &[Scalar], fill: &Scalar) -> Vec<Scalar> {
    values
        .iter()
        .map(|v| if v.is_missing() { fill.clone() } else { v.clone() })
        .collect()
}

pub fn dropna(values: &[Scalar]) -> Vec<Scalar> {
    values
        .iter()
        .filter(|v| !v.is_missing())
        .cloned()
        .collect()
}

// ── Nanops: null-skipping numeric reductions ───────────────────────────

fn collect_finite(values: &[Scalar]) -> Vec<f64> {
    values
        .iter()
        .filter(|v| !v.is_missing())
        .filter_map(|v| v.to_f64().ok())
        .collect()
}

pub fn nansum(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Float64(0.0);
    }
    Scalar::Float64(nums.iter().sum())
}

pub fn nanmean(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }
    let sum: f64 = nums.iter().sum();
    Scalar::Float64(sum / nums.len() as f64)
}

pub fn nancount(values: &[Scalar]) -> Scalar {
    let n = values.iter().filter(|v| !v.is_missing()).count();
    Scalar::Int64(n as i64)
}

pub fn nanmin(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }
    Scalar::Float64(nums.iter().copied().fold(f64::INFINITY, f64::min))
}

pub fn nanmax(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }
    Scalar::Float64(nums.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

pub fn nanmedian(values: &[Scalar]) -> Scalar {
    let mut nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = nums.len() / 2;
    if nums.len().is_multiple_of(2) {
        Scalar::Float64((nums[mid - 1] + nums[mid]) / 2.0)
    } else {
        Scalar::Float64(nums[mid])
    }
}

pub fn nanvar(values: &[Scalar], ddof: usize) -> Scalar {
    let nums = collect_finite(values);
    if nums.len() <= ddof {
        return Scalar::Null(NullKind::NaN);
    }
    let mean: f64 = nums.iter().sum::<f64>() / nums.len() as f64;
    let sum_sq: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    Scalar::Float64(sum_sq / (nums.len() - ddof) as f64)
}

pub fn nanstd(values: &[Scalar], ddof: usize) -> Scalar {
    match nanvar(values, ddof) {
        Scalar::Float64(v) => Scalar::Float64(v.sqrt()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{DType, NullKind, Scalar, cast_scalar, common_dtype, infer_dtype};

    #[test]
    fn dtype_inference_coerces_numeric_values() {
        let values = vec![Scalar::Bool(true), Scalar::Int64(7), Scalar::Float64(3.5)];
        assert_eq!(
            infer_dtype(&values).expect("dtype should infer"),
            DType::Float64
        );
    }

    #[test]
    fn missing_values_get_target_missing_marker() {
        let missing = Scalar::Null(NullKind::Null);
        let cast = cast_scalar(&missing, DType::Float64).expect("missing casts");
        assert_eq!(cast, Scalar::Null(NullKind::NaN));
    }

    #[test]
    fn semantic_eq_treats_nan_as_equal() {
        let left = Scalar::Float64(f64::NAN);
        let right = Scalar::Null(NullKind::NaN);
        assert!(left.semantic_eq(&right));
    }

    #[test]
    fn common_dtype_rejects_string_numeric_mix() {
        let err = common_dtype(DType::Utf8, DType::Int64).expect_err("must fail");
        assert_eq!(
            err.to_string(),
            "dtype coercion from Utf8 to Int64 has no compatible common type"
        );
    }

    // ── Scalar missingness methods ─────────────────────────────────────

    #[test]
    fn is_null_detects_explicit_nulls() {
        assert!(Scalar::Null(NullKind::Null).is_null());
        assert!(Scalar::Null(NullKind::NaN).is_null());
        assert!(!Scalar::Int64(42).is_null());
        assert!(!Scalar::Float64(f64::NAN).is_null());
    }

    #[test]
    fn is_na_matches_is_missing() {
        let vals = vec![
            Scalar::Null(NullKind::Null),
            Scalar::Float64(f64::NAN),
            Scalar::Int64(0),
            Scalar::Bool(false),
        ];
        for v in &vals {
            assert_eq!(v.is_na(), v.is_missing());
        }
    }

    #[test]
    fn coalesce_picks_first_non_missing() {
        let null = Scalar::Null(NullKind::Null);
        let fill = Scalar::Int64(99);
        assert_eq!(null.coalesce(&fill), fill);
        assert_eq!(fill.coalesce(&null), fill);
    }

    // ── Missingness utilities ──────────────────────────────────────────

    #[test]
    fn isna_notna_complement() {
        let vals = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(f64::NAN),
            Scalar::Float64(3.0),
        ];
        let na = super::isna(&vals);
        let not = super::notna(&vals);
        assert_eq!(na, vec![false, true, true, false]);
        for (a, b) in na.iter().zip(not.iter()) {
            assert_ne!(a, b);
        }
    }

    #[test]
    fn count_na_counts_missing() {
        let vals = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(f64::NAN),
        ];
        assert_eq!(super::count_na(&vals), 2);
    }

    #[test]
    fn fill_na_replaces_missing() {
        let vals = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(f64::NAN),
            Scalar::Int64(4),
        ];
        let filled = super::fill_na(&vals, &Scalar::Int64(0));
        assert_eq!(filled[0], Scalar::Int64(1));
        assert_eq!(filled[1], Scalar::Int64(0));
        assert_eq!(filled[2], Scalar::Int64(0));
        assert_eq!(filled[3], Scalar::Int64(4));
    }

    #[test]
    fn dropna_removes_missing() {
        let vals = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(3),
            Scalar::Float64(f64::NAN),
        ];
        let kept = super::dropna(&vals);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], Scalar::Int64(1));
        assert_eq!(kept[1], Scalar::Int64(3));
    }

    // ── Nanops ─────────────────────────────────────────────────────────

    #[test]
    fn nansum_skips_nulls() {
        let vals = vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(2.0),
            Scalar::Float64(f64::NAN),
            Scalar::Int64(7),
        ];
        assert_eq!(super::nansum(&vals), Scalar::Float64(10.0));
    }

    #[test]
    fn nansum_empty_returns_zero() {
        assert_eq!(super::nansum(&[]), Scalar::Float64(0.0));
    }

    #[test]
    fn nanmean_basic() {
        let vals = vec![
            Scalar::Float64(2.0),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(4.0),
        ];
        assert_eq!(super::nanmean(&vals), Scalar::Float64(3.0));
    }

    #[test]
    fn nanmean_all_null_returns_nan() {
        let vals = vec![Scalar::Null(NullKind::Null), Scalar::Float64(f64::NAN)];
        assert!(super::nanmean(&vals).is_missing());
    }

    #[test]
    fn nancount_counts_non_missing() {
        let vals = vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(3.0),
        ];
        assert_eq!(super::nancount(&vals), Scalar::Int64(2));
    }

    #[test]
    fn nanmin_basic() {
        let vals = vec![
            Scalar::Float64(5.0),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(2.0),
            Scalar::Float64(8.0),
        ];
        assert_eq!(super::nanmin(&vals), Scalar::Float64(2.0));
    }

    #[test]
    fn nanmax_basic() {
        let vals = vec![
            Scalar::Float64(5.0),
            Scalar::Float64(f64::NAN),
            Scalar::Float64(8.0),
        ];
        assert_eq!(super::nanmax(&vals), Scalar::Float64(8.0));
    }

    #[test]
    fn nanmin_nanmax_empty_returns_nan() {
        assert!(super::nanmin(&[]).is_missing());
        assert!(super::nanmax(&[]).is_missing());
    }

    #[test]
    fn nanmedian_odd_count() {
        let vals = vec![
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
        ];
        assert_eq!(super::nanmedian(&vals), Scalar::Float64(2.0));
    }

    #[test]
    fn nanmedian_even_count() {
        let vals = vec![
            Scalar::Float64(1.0),
            Scalar::Float64(3.0),
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
        ];
        assert_eq!(super::nanmedian(&vals), Scalar::Float64(2.5));
    }

    #[test]
    fn nanvar_population() {
        let vals = vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(5.0),
            Scalar::Float64(7.0),
            Scalar::Float64(9.0),
        ];
        let var = super::nanvar(&vals, 0);
        if let Scalar::Float64(v) = var {
            assert!((v - 4.0).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn nanvar_sample_ddof1() {
        let vals = vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(5.0),
            Scalar::Float64(7.0),
            Scalar::Float64(9.0),
        ];
        let var = super::nanvar(&vals, 1);
        if let Scalar::Float64(v) = var {
            assert!((v - 32.0 / 7.0).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn nanvar_insufficient_values_returns_nan() {
        let vals = vec![Scalar::Float64(5.0)];
        assert!(super::nanvar(&vals, 1).is_missing());
    }

    #[test]
    fn nanstd_is_sqrt_of_var() {
        let vals = vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(5.0),
            Scalar::Float64(7.0),
            Scalar::Float64(9.0),
        ];
        let std = super::nanstd(&vals, 0);
        if let Scalar::Float64(v) = std {
            assert!((v - 2.0).abs() < 1e-10);
        } else {
            panic!("expected Float64");
        }
    }

    #[test]
    fn nanops_with_mixed_types() {
        let vals = vec![
            Scalar::Bool(true),
            Scalar::Int64(3),
            Scalar::Float64(6.0),
            Scalar::Null(NullKind::Null),
        ];
        assert_eq!(super::nansum(&vals), Scalar::Float64(10.0));
        assert_eq!(super::nancount(&vals), Scalar::Int64(3));
    }

    #[test]
    fn nanops_all_missing_returns_identity() {
        let vals = vec![Scalar::Null(NullKind::Null), Scalar::Float64(f64::NAN)];
        assert_eq!(super::nansum(&vals), Scalar::Float64(0.0));
        assert!(super::nanmean(&vals).is_missing());
        assert!(super::nanmedian(&vals).is_missing());
        assert!(super::nanvar(&vals, 0).is_missing());
        assert!(super::nanstd(&vals, 0).is_missing());
    }
}
