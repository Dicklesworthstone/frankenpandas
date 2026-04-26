#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! Foundational value-type abstractions for **frankenpandas** — the
//! enums, structs, and free functions that every other crate
//! (fp-columnar, fp-index, fp-frame, fp-io, ...) consumes when
//! representing scalar data, dtypes, missing values, and time deltas.
//!
//! The types here intentionally stay tiny and dependency-light
//! (`serde`, `thiserror`) so they can sit at the bottom of the
//! workspace dep graph.
//!
//! ## Core value types
//!
//! - [`DType`]: the dtype enum — `Null`, `Bool`, `Int64`, `Float64`,
//!   `Utf8`, `Categorical`, `Timedelta64`, `Sparse`. Drives column /
//!   series storage decisions across the workspace.
//! - [`Scalar`]: the per-cell value enum, parameterized by `DType`.
//!   Each variant holds the actual data (`Int64(i64)`, `Float64(f64)`,
//!   `Utf8(String)`, ...) plus the `Null(NullKind)` variant for
//!   missing values.
//! - [`NullKind`]: distinguishes the three pandas missing-value
//!   "flavors" — `Null` (Python `None` / SQL NULL), `NaN`
//!   (floating-point not-a-number), `NaT` (timedelta / datetime
//!   not-a-time). `Scalar::Null(...)` carries the kind so downstream
//!   code can preserve pandas semantics.
//! - [`SparseDType`]: descriptor for sparse-encoded dtypes (paired
//!   value dtype + fill value).
//!
//! ## Time / duration types
//!
//! - [`Timedelta`]: nanosecond-precision duration with arithmetic
//!   helpers ([`Timedelta::add`], [`Timedelta::sub`],
//!   [`Timedelta::mul_scalar`], [`Timedelta::div_scalar`],
//!   [`Timedelta::div_timedelta`]) that propagate `NaT` per pandas
//!   semantics. [`TimedeltaComponents`] breaks a timedelta into
//!   days/hours/minutes/seconds/nanos for display.
//! - [`Timestamp`]: nanosecond-precision wall-clock timestamp with
//!   optional timezone. Includes floor / ceil / round helpers and
//!   `NaT` propagation.
//!
//! ## Dtype inference + casting
//!
//! - [`infer_dtype`]: derive a [`DType`] from a slice of scalars
//!   (used during DataFrame construction).
//! - [`common_dtype`]: pandas-style dtype promotion for binary ops.
//! - [`cast_scalar`] / [`cast_scalar_owned`]: convert a scalar to a
//!   target dtype with explicit error reporting on impossible casts.
//!
//! ## Missing-value helpers
//!
//! Free fns matching `pd.isna` / `pd.notna` / `pd.fillna` / `pd.dropna`
//! plus the `nan*` aggregations ([`nansum`], [`nanmean`], [`nancount`],
//! [`nanmin`], [`nanmax`], [`nanmedian`], [`nanvar`], [`nanstd`])
//! that mirror pandas' missing-aware reductions.
//!
//! ## Error reporting
//!
//! Errors are explicit enums via `thiserror`: [`TypeError`] for
//! dtype-related failures (incompatible-cast, no-common-dtype) and
//! [`TimedeltaError`] for parse failures.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DType {
    Null,
    Bool,
    Int64,
    Float64,
    #[serde(alias = "string", alias = "str")]
    Utf8,
    Categorical,
    Timedelta64,
    Sparse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseDType {
    pub value_dtype: DType,
    pub fill_value: Scalar,
}

impl SparseDType {
    /// Construct a pandas-style sparse dtype descriptor.
    ///
    /// This records the logical dense value dtype plus the scalar value that is
    /// elided from storage. The concrete sparse column representation lives in
    /// fp-columnar; this descriptor is the shared public contract.
    pub fn new(value_dtype: DType, fill_value: Scalar) -> Result<Self, TypeError> {
        if matches!(value_dtype, DType::Null | DType::Sparse) {
            return Err(TypeError::InvalidSparseValueDType { dtype: value_dtype });
        }

        let fill_value = if fill_value.is_missing() {
            Scalar::missing_for_dtype(value_dtype)
        } else {
            cast_scalar_owned(fill_value, value_dtype)?
        };

        Ok(Self {
            value_dtype,
            fill_value,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    #[serde(alias = "string", alias = "str")]
    Utf8(String),
    Timedelta64(i64),
}

impl std::fmt::Display for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null(NullKind::NaN) => write!(f, "NaN"),
            Self::Null(NullKind::NaT) => write!(f, "NaT"),
            Self::Null(NullKind::Null) => write!(f, "None"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int64(v) => write!(f, "{v}"),
            Self::Float64(v) => write!(f, "{v}"),
            Self::Utf8(s) => write!(f, "{s}"),
            Self::Timedelta64(nanos) => write!(f, "{}", Timedelta::format(*nanos)),
        }
    }
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
            Self::Timedelta64(_) => DType::Timedelta64,
        }
    }

    #[must_use]
    pub fn is_missing(&self) -> bool {
        match self {
            Self::Null(_) => true,
            Self::Float64(v) => v.is_nan(),
            Self::Timedelta64(v) => *v == Timedelta::NAT,
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
            DType::Timedelta64 => Self::Timedelta64(Timedelta::NAT),
            DType::Null => Self::Null(NullKind::Null),
            DType::Bool | DType::Int64 | DType::Utf8 | DType::Categorical | DType::Sparse => {
                Self::Null(NullKind::Null)
            }
        }
    }

    #[must_use]
    pub fn semantic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Float64(a), Self::Float64(b)) => {
                if a.is_nan() && b.is_nan() {
                    return true;
                }
                if *a == *b {
                    return true;
                }
                let diff = (*a - *b).abs();
                let max_abs = a.abs().max(b.abs());
                if max_abs == 0.0 {
                    diff < f64::EPSILON
                } else {
                    diff / max_abs < 1e-14
                }
            }
            (Self::Null(_), Self::Float64(v)) | (Self::Float64(v), Self::Null(_)) => v.is_nan(),
            // All Null kinds (Null / NaN / NaT) mark missingness; they are
            // semantically indistinguishable for oracle-parity checks even
            // though derived PartialEq would reject a cross-kind pair.
            // fp-frame normalizes Float64 column missing cells to
            // Null(NaN) at Column::new time, while fixture oracles encode
            // the canonical missing marker as Null(Null).
            (Self::Null(_), Self::Null(_)) => true,
            _ => self == other,
        }
    }

    #[must_use]
    pub fn semantic_le(&self, other: &Self) -> bool {
        match self.semantic_cmp(other) {
            std::cmp::Ordering::Less | std::cmp::Ordering::Equal => true,
            std::cmp::Ordering::Greater => false,
        }
    }

    #[must_use]
    pub fn semantic_ge(&self, other: &Self) -> bool {
        match self.semantic_cmp(other) {
            std::cmp::Ordering::Greater | std::cmp::Ordering::Equal => true,
            std::cmp::Ordering::Less => false,
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
        if self.is_missing() {
            other.clone()
        } else {
            self.clone()
        }
    }

    #[must_use]
    pub fn semantic_cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Int64(a), Self::Int64(b)) => a.cmp(b),
            (Self::Float64(a), Self::Float64(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Self::Utf8(a), Self::Utf8(b)) => a.cmp(b),
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Null(a), Self::Null(b)) => a.cmp(b),
            (Self::Timedelta64(a), Self::Timedelta64(b)) => {
                if *a == Timedelta::NAT || *b == Timedelta::NAT {
                    std::cmp::Ordering::Equal
                } else {
                    a.cmp(b)
                }
            }
            // Cross-numeric comparison
            (Self::Int64(a), Self::Float64(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Self::Float64(a), Self::Int64(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
            // Fallback to debug representation for inconsistent types
            (a, b) => format!("{a:?}").cmp(&format!("{b:?}")),
        }
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
            Self::Timedelta64(v) if *v == Timedelta::NAT => Err(TypeError::ValueIsMissing {
                kind: NullKind::NaT,
            }),
            Self::Timedelta64(v) => Err(TypeError::NonNumericValue {
                value: Timedelta::format(*v),
                dtype: DType::Timedelta64,
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
    #[error("sparse value dtype cannot be {dtype:?}")]
    InvalidSparseValueDType { dtype: DType },
    #[error("interval_range step must be finite, positive, and not NaN (got {step})")]
    InvalidIntervalStep { step: f64 },
    #[error("interval_range step {step} does not evenly divide range end-start={span}")]
    IntervalStepDoesNotDivide { step: f64, span: f64 },
}

pub fn common_dtype(left: DType, right: DType) -> Result<DType, TypeError> {
    use DType::{Bool, Categorical, Float64, Int64, Null, Sparse, Timedelta64};

    let out = match (left, right) {
        (a, b) if a == b => a,
        (Null, other) | (other, Null) => other,
        (Categorical, Categorical) => Categorical,
        (Bool, Int64) | (Int64, Bool) => Int64,
        (Bool, Float64) | (Float64, Bool) => Float64,
        (Int64, Float64) | (Float64, Int64) => Float64,
        (Timedelta64, Timedelta64) => Timedelta64,
        (Sparse, _) | (_, Sparse) => return Err(TypeError::IncompatibleDtypes { left, right }),
        _ => return Err(TypeError::IncompatibleDtypes { left, right }),
    };

    Ok(out)
}

pub fn infer_dtype(values: &[Scalar]) -> Result<DType, TypeError> {
    let mut current = DType::Null;
    let mut saw_utf8 = false;
    let mut saw_timedelta = false;
    let mut saw_non_utf8_non_null = false;

    for value in values {
        match value.dtype() {
            DType::Null => {}
            DType::Utf8 => saw_utf8 = true,
            DType::Timedelta64 => {
                saw_timedelta = true;
                if current == DType::Null {
                    current = DType::Timedelta64;
                } else if current != DType::Timedelta64 {
                    return Err(TypeError::IncompatibleDtypes {
                        left: current,
                        right: DType::Timedelta64,
                    });
                }
            }
            other => {
                saw_non_utf8_non_null = true;
                current = common_dtype(current, other)?;
            }
        }

        if saw_utf8 && saw_non_utf8_non_null {
            // Constructor inference follows pandas object-dtype behavior for
            // heterogeneous string/scalar payloads while arithmetic coercion
            // remains governed by the stricter common_dtype lattice.
            return Ok(DType::Utf8);
        }
        if saw_timedelta && saw_non_utf8_non_null {
            return Err(TypeError::IncompatibleDtypes {
                left: DType::Timedelta64,
                right: current,
            });
        }
    }

    if saw_utf8 {
        Ok(DType::Utf8)
    } else {
        Ok(current)
    }
}

/// Cast a scalar to a target dtype, taking ownership to avoid redundant clones
/// when the value already has the correct type (AG-03: identity-cast skip).
pub fn cast_scalar_owned(value: Scalar, target: DType) -> Result<Scalar, TypeError> {
    let from = value.dtype();
    if from == target {
        return Ok(value);
    }
    if value.is_missing() {
        return Ok(Scalar::missing_for_dtype(target));
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
                if *v < i64::MIN as f64 || *v >= 9223372036854775808.0 {
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
        DType::Categorical => Err(TypeError::InvalidCast { from, to: target }),
        DType::Timedelta64 => match &value {
            Scalar::Int64(v) => Ok(Scalar::Timedelta64(*v)),
            Scalar::Utf8(s) => Timedelta::parse(s)
                .map(Scalar::Timedelta64)
                .map_err(|_| TypeError::InvalidCast { from, to: target }),
            _ => Err(TypeError::InvalidCast { from, to: target }),
        },
        DType::Sparse => Err(TypeError::InvalidCast { from, to: target }),
    }
}

/// Cast a scalar reference to a target dtype (clones only when conversion is needed).
pub fn cast_scalar(value: &Scalar, target: DType) -> Result<Scalar, TypeError> {
    cast_scalar_owned(value.clone(), target)
}

// ── Timedelta support ──────────────────────────────────────────────────

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TimedeltaError {
    #[error("invalid timedelta string: {0}")]
    InvalidFormat(String),
    #[error("overflow in timedelta computation")]
    Overflow,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TimedeltaComponents {
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub milliseconds: i64,
    pub microseconds: i64,
    pub nanoseconds: i64,
}

pub struct Timedelta;

impl Timedelta {
    pub const NANOS_PER_MICRO: i64 = 1_000;
    pub const NANOS_PER_MILLI: i64 = 1_000_000;
    pub const NANOS_PER_SEC: i64 = 1_000_000_000;
    pub const NANOS_PER_MIN: i64 = 60 * Self::NANOS_PER_SEC;
    pub const NANOS_PER_HOUR: i64 = 60 * Self::NANOS_PER_MIN;
    pub const NANOS_PER_DAY: i64 = 24 * Self::NANOS_PER_HOUR;
    pub const NANOS_PER_WEEK: i64 = 7 * Self::NANOS_PER_DAY;

    pub const NAT: i64 = i64::MIN;

    pub fn parse(s: &str) -> Result<i64, TimedeltaError> {
        let s = s.trim();

        if s.eq_ignore_ascii_case("nat") {
            return Ok(Self::NAT);
        }

        let (negative, s) = if let Some(rest) = s.strip_prefix('-') {
            (true, rest.trim())
        } else {
            (false, s)
        };

        if let Some(nanos) = Self::try_parse_time_format(s) {
            return Ok(if negative { -nanos } else { nanos });
        }

        let nanos = Self::parse_compound(s)?;
        Ok(if negative { -nanos } else { nanos })
    }

    fn try_parse_time_format(s: &str) -> Option<i64> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }

        let hours: i64 = parts[0].parse().ok()?;
        let minutes: i64 = parts[1].parse().ok()?;

        let (seconds, frac_nanos) = if parts.len() == 3 {
            if let Some((sec_str, frac_str)) = parts[2].split_once('.') {
                let sec: i64 = sec_str.parse().ok()?;
                let padded = format!("{:0<9}", frac_str);
                let frac: i64 = padded[..9].parse().ok()?;
                (sec, frac)
            } else {
                let sec: i64 = parts[2].parse().ok()?;
                (sec, 0)
            }
        } else {
            (0, 0)
        };

        Some(
            hours * Self::NANOS_PER_HOUR
                + minutes * Self::NANOS_PER_MIN
                + seconds * Self::NANOS_PER_SEC
                + frac_nanos,
        )
    }

    fn parse_compound(s: &str) -> Result<i64, TimedeltaError> {
        let mut total: i64 = 0;
        let mut remaining = s;

        while !remaining.is_empty() {
            remaining = remaining.trim_start();
            if remaining.is_empty() {
                break;
            }

            let num_end = remaining
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(remaining.len());

            if num_end == 0 {
                return Err(TimedeltaError::InvalidFormat(s.to_string()));
            }

            let num_str = &remaining[..num_end];
            let num: f64 = num_str
                .parse()
                .map_err(|_| TimedeltaError::InvalidFormat(s.to_string()))?;

            remaining = remaining[num_end..].trim_start();

            let unit_end = remaining
                .find(|c: char| c.is_ascii_digit() || c.is_whitespace())
                .unwrap_or(remaining.len());

            let unit = &remaining[..unit_end];
            remaining = &remaining[unit_end..];

            let multiplier = Self::unit_to_nanos(unit)
                .ok_or_else(|| TimedeltaError::InvalidFormat(s.to_string()))?;

            let nanos = (num * multiplier as f64).round() as i64;
            total = total.checked_add(nanos).ok_or(TimedeltaError::Overflow)?;
        }

        if total == 0 && !s.trim().is_empty() && s.trim() != "0" {
            return Err(TimedeltaError::InvalidFormat(s.to_string()));
        }

        Ok(total)
    }

    /// Map a pandas-style frequency-alias string to a nanosecond-count.
    ///
    /// Recognizes pandas's offset alias core set plus common word forms:
    /// W/week(s), D/day(s), H/hr/hour(s), m/T/min/minute(s), s/sec/second(s),
    /// ms/milli/millisecond(s)/L, us/µs/micro/microsecond(s)/U, ns/nano/
    /// nanosecond(s)/N. Empty string maps to days (matches pandas default).
    /// Returns `None` for unrecognized aliases — callers can choose to map
    /// that to NaT (consistent with the rest of fp-types) or surface as a
    /// typed error.
    ///
    /// Per br-frankenpandas-lbsx (9p0u Phase 2.6): public surface so
    /// downstream crates can consume the same alias map fp-types uses for
    /// `Timedelta::from_unit` / `Timestamp::*_to_unit`.
    #[must_use]
    pub fn unit_to_nanos(unit: &str) -> Option<i64> {
        match unit.to_lowercase().as_str() {
            "w" | "week" | "weeks" => Some(Self::NANOS_PER_WEEK),
            "d" | "day" | "days" => Some(Self::NANOS_PER_DAY),
            "h" | "hr" | "hour" | "hours" => Some(Self::NANOS_PER_HOUR),
            "m" | "min" | "minute" | "minutes" | "t" => Some(Self::NANOS_PER_MIN),
            "s" | "sec" | "second" | "seconds" => Some(Self::NANOS_PER_SEC),
            "ms" | "milli" | "millis" | "millisecond" | "milliseconds" | "l" => {
                Some(Self::NANOS_PER_MILLI)
            }
            "us" | "µs" | "micro" | "micros" | "microsecond" | "microseconds" | "u" => {
                Some(Self::NANOS_PER_MICRO)
            }
            "ns" | "nano" | "nanos" | "nanosecond" | "nanoseconds" | "n" => Some(1),
            "" => Some(Self::NANOS_PER_DAY),
            _ => None,
        }
    }

    pub fn components(nanos: i64) -> TimedeltaComponents {
        if nanos == Self::NAT {
            return TimedeltaComponents::default();
        }

        let negative = nanos < 0;
        let abs_nanos = nanos.unsigned_abs() as i64;

        let days = abs_nanos / Self::NANOS_PER_DAY;
        let rem = abs_nanos % Self::NANOS_PER_DAY;

        let hours = rem / Self::NANOS_PER_HOUR;
        let rem = rem % Self::NANOS_PER_HOUR;

        let minutes = rem / Self::NANOS_PER_MIN;
        let rem = rem % Self::NANOS_PER_MIN;

        let seconds = rem / Self::NANOS_PER_SEC;
        let rem = rem % Self::NANOS_PER_SEC;

        let milliseconds = rem / Self::NANOS_PER_MILLI;
        let rem = rem % Self::NANOS_PER_MILLI;

        let microseconds = rem / Self::NANOS_PER_MICRO;
        let nanoseconds = rem % Self::NANOS_PER_MICRO;

        TimedeltaComponents {
            days: if negative { -days } else { days },
            hours,
            minutes,
            seconds,
            milliseconds,
            microseconds,
            nanoseconds,
        }
    }

    pub fn total_seconds(nanos: i64) -> f64 {
        if nanos == Self::NAT {
            f64::NAN
        } else {
            nanos as f64 / Self::NANOS_PER_SEC as f64
        }
    }

    pub fn format(nanos: i64) -> String {
        if nanos == Self::NAT {
            return "NaT".to_string();
        }

        let comp = Self::components(nanos);
        let sign = if nanos < 0 { "-" } else { "" };

        let time_part = format!("{:02}:{:02}:{:02}", comp.hours, comp.minutes, comp.seconds);

        let frac = comp.milliseconds * 1_000_000 + comp.microseconds * 1_000 + comp.nanoseconds;

        if frac > 0 {
            format!("{}{} days {}.{:09}", sign, comp.days.abs(), time_part, frac)
        } else {
            format!("{}{} days {}", sign, comp.days.abs(), time_part)
        }
    }

    pub fn from_unit(value: f64, unit: &str) -> Result<i64, TimedeltaError> {
        let multiplier = Self::unit_to_nanos(unit)
            .ok_or_else(|| TimedeltaError::InvalidFormat(unit.to_string()))?;
        Ok((value * multiplier as f64).round() as i64)
    }

    // ── Arithmetic (br-frankenpandas-4r56 Phase 1) ──────────────────────
    //
    // NaT propagation: any arithmetic with `NAT` returns `NAT`. Matches
    // pandas `pd.NaT + anything == NaT`, `pd.NaT - anything == NaT`, etc.
    // Saturation: i64 overflow clamps to i64::MAX/MIN (never wraps). Matches
    // pandas's OverflowError surface at the type-system boundary.

    /// Add two Timedelta nanosecond values. NaT propagates; saturates on overflow.
    #[must_use]
    pub fn add(a: i64, b: i64) -> i64 {
        if a == Self::NAT || b == Self::NAT {
            return Self::NAT;
        }
        a.saturating_add(b)
    }

    /// Subtract two Timedelta nanosecond values. NaT propagates; saturates on overflow.
    #[must_use]
    pub fn sub(a: i64, b: i64) -> i64 {
        if a == Self::NAT || b == Self::NAT {
            return Self::NAT;
        }
        a.saturating_sub(b)
    }

    /// Negate a Timedelta value. NaT stays NaT. Saturates on overflow
    /// (pandas: `-pd.Timedelta.min` is NaT since min == -max - 1 cannot be negated).
    #[must_use]
    pub fn neg(a: i64) -> i64 {
        if a == Self::NAT {
            return Self::NAT;
        }
        a.saturating_neg()
    }

    /// Absolute value of a Timedelta. NaT stays NaT. Saturates on overflow.
    #[must_use]
    pub fn abs(a: i64) -> i64 {
        if a == Self::NAT {
            return Self::NAT;
        }
        a.saturating_abs()
    }

    /// Multiply a Timedelta value by an integer factor. NaT propagates;
    /// saturates on overflow.
    ///
    /// Matches pandas `pd.Timedelta(...) * int`.
    #[must_use]
    pub fn mul_scalar(a: i64, factor: i64) -> i64 {
        if a == Self::NAT {
            return Self::NAT;
        }
        a.saturating_mul(factor)
    }

    /// Floor-divide a Timedelta value by an integer divisor. NaT propagates.
    /// Returns NaT on divide-by-zero (matches pandas, which raises, but we
    /// surface as NaT to avoid panics at the type-system boundary).
    ///
    /// Matches pandas / Python `pd.Timedelta(...) // int`: floor division,
    /// not truncation toward zero. `-100 // 3 == -34`, and `100 // -3 ==
    /// -34`. Rust's `/` truncates toward zero and `div_euclid` keeps the
    /// remainder non-negative — neither matches pandas when the divisor is
    /// negative. This helper adjusts trunc-toward-zero into floor.
    #[must_use]
    pub fn div_scalar(a: i64, divisor: i64) -> i64 {
        if a == Self::NAT || divisor == 0 {
            return Self::NAT;
        }
        // NAT == i64::MIN so the classic `i64::MIN / -1` overflow path is
        // already handled by the NAT check above. `(i64::MIN + 1) / -1`
        // equals `i64::MAX` with no overflow, so we never need a
        // saturation branch here.
        let q = a / divisor;
        let r = a % divisor;
        // If remainder is non-zero and has opposite sign from divisor,
        // Rust's trunc-toward-zero `/` is one step above the floor. Adjust
        // down by 1 to match Python/pandas floor division.
        if r != 0 && (r < 0) != (divisor < 0) {
            q - 1
        } else {
            q
        }
    }

    /// Divide two Timedelta values, returning the ratio as f64.
    /// Matches pandas `pd.Timedelta(...) / pd.Timedelta(...)` → float.
    /// NaT in either operand → NaN. Zero divisor → ±Inf (per IEEE 754).
    #[must_use]
    pub fn div_timedelta(a: i64, b: i64) -> f64 {
        if a == Self::NAT || b == Self::NAT {
            return f64::NAN;
        }
        (a as f64) / (b as f64)
    }
}

// ── Timestamp types (br-frankenpandas-9p0u — 4r56 Phase 2) ─────────────
//
// Nanosecond-precision i64 since Unix epoch + optional IANA tz name.
// TZ-dependent arithmetic (DST transitions, tz conversion) is deferred
// to Phase 3 which pulls chrono_tz into fp-types; Phase 2 stores the
// tz name as opaque metadata and performs arithmetic on the absolute
// nanos axis only.

/// A nanosecond-precision point in time, Unix-epoch anchored.
///
/// Phase 2 scope: construction, arithmetic, equality, ordering, serde.
/// TZ semantics (IANA tz lookup, DST-aware shift) are deferred to Phase
/// 3 — see br-frankenpandas-4r56.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timestamp {
    /// Nanoseconds since Unix epoch. `i64::MIN` is NaT.
    pub nanos: i64,
    /// Optional IANA time-zone name (e.g. `"US/Eastern"`). `None` means
    /// naive / UTC-anchored. Phase 2 treats this as opaque metadata;
    /// Phase 3 wires chrono_tz interpretation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

impl Timestamp {
    /// NaT sentinel, parallel to `Timedelta::NAT`.
    pub const NAT: i64 = i64::MIN;

    /// Construct a UTC-anchored (tz=None) Timestamp from nanoseconds
    /// since Unix epoch.
    #[must_use]
    pub const fn from_nanos(nanos: i64) -> Self {
        Self { nanos, tz: None }
    }

    /// Construct a Timestamp tagged with an IANA tz name.
    ///
    /// Phase 2 doesn't interpret the tz — it only carries the name
    /// through arithmetic + serde. Phase 3 wires chrono_tz conversion.
    #[must_use]
    pub fn from_nanos_tz(nanos: i64, tz_name: impl Into<String>) -> Self {
        Self {
            nanos,
            tz: Some(tz_name.into()),
        }
    }

    /// The NaT sentinel value for a Timestamp.
    #[must_use]
    pub const fn nat() -> Self {
        Self {
            nanos: Self::NAT,
            tz: None,
        }
    }

    /// True iff this Timestamp is NaT.
    #[must_use]
    pub const fn is_nat(&self) -> bool {
        self.nanos == Self::NAT
    }

    /// Add a Timedelta. NaT in either operand → NaT; saturates on overflow.
    /// TZ is preserved from `self`.
    #[must_use]
    pub fn add_timedelta(&self, td_nanos: i64) -> Self {
        if self.is_nat() || td_nanos == Timedelta::NAT {
            return Self::nat();
        }
        Self {
            nanos: self.nanos.saturating_add(td_nanos),
            tz: self.tz.clone(),
        }
    }

    /// Subtract a Timedelta. NaT propagation + saturation; TZ preserved.
    #[must_use]
    pub fn sub_timedelta(&self, td_nanos: i64) -> Self {
        if self.is_nat() || td_nanos == Timedelta::NAT {
            return Self::nat();
        }
        Self {
            nanos: self.nanos.saturating_sub(td_nanos),
            tz: self.tz.clone(),
        }
    }

    /// Subtract another Timestamp. Returns a Timedelta (i64 nanos).
    /// NaT in either → `Timedelta::NAT`; saturates on overflow.
    #[must_use]
    pub fn sub_timestamp(&self, other: &Self) -> i64 {
        if self.is_nat() || other.is_nat() {
            return Timedelta::NAT;
        }
        self.nanos.saturating_sub(other.nanos)
    }

    /// NaT-aware semantic equality: two NaT Timestamps are equal to each
    /// other (matches pandas `pd.NaT == pd.NaT` under `equals()`, though
    /// pandas's `==` operator returns False for NaT==NaT — we follow the
    /// `semantic_eq` convention used elsewhere in fp-types).
    #[must_use]
    pub fn semantic_eq(&self, other: &Self) -> bool {
        if self.is_nat() && other.is_nat() {
            return true;
        }
        if self.is_nat() || other.is_nat() {
            return false;
        }
        self.nanos == other.nanos && self.tz == other.tz
    }

    // ── Rounding to a Timedelta unit (br-frankenpandas-5h6n) ────────────
    //
    // Pure i64 arithmetic on the nanos axis. tz is preserved. Phase 3
    // chrono_tz integration will add a tz-aware variant that handles DST
    // boundaries correctly; these methods operate on the absolute time
    // axis, matching pandas's tz-naive `.floor` / `.ceil` / `.round`
    // semantics for unit values smaller than a day.

    /// Round down to the nearest multiple of `unit_nanos`.
    ///
    /// Matches `pd.Timestamp(...).floor(unit)`. NaT in `self` or a
    /// non-positive `unit_nanos` returns NaT.
    #[must_use]
    pub fn floor_to(&self, unit_nanos: i64) -> Self {
        if self.is_nat() || unit_nanos <= 0 {
            return Self::nat();
        }
        Self {
            nanos: self.nanos.div_euclid(unit_nanos) * unit_nanos,
            tz: self.tz.clone(),
        }
    }

    /// Round up to the nearest multiple of `unit_nanos`.
    ///
    /// Matches `pd.Timestamp(...).ceil(unit)`. NaT or non-positive
    /// `unit_nanos` returns NaT. Already-multiple inputs return self.
    #[must_use]
    pub fn ceil_to(&self, unit_nanos: i64) -> Self {
        if self.is_nat() || unit_nanos <= 0 {
            return Self::nat();
        }
        let rem = self.nanos.rem_euclid(unit_nanos);
        let nanos = if rem == 0 {
            self.nanos
        } else {
            self.nanos.saturating_add(unit_nanos - rem)
        };
        Self {
            nanos,
            tz: self.tz.clone(),
        }
    }

    /// Round to the nearest multiple of `unit_nanos`, banker's rounding
    /// (half-to-even) on ties.
    ///
    /// Matches `pd.Timestamp(...).round(unit)`. NaT or non-positive
    /// `unit_nanos` returns NaT.
    #[must_use]
    pub fn round_to(&self, unit_nanos: i64) -> Self {
        if self.is_nat() || unit_nanos <= 0 {
            return Self::nat();
        }
        let floor = self.nanos.div_euclid(unit_nanos);
        let rem = self.nanos.rem_euclid(unit_nanos);
        let half = unit_nanos / 2;
        let chosen_floor = if rem < half {
            floor
        } else if rem > half {
            floor + 1
        } else if unit_nanos % 2 != 0 {
            // Odd unit can't have a true half; treat as round-up.
            floor + 1
        } else {
            // Tie: pick the even multiple.
            if floor % 2 == 0 { floor } else { floor + 1 }
        };
        Self {
            nanos: chosen_floor.saturating_mul(unit_nanos),
            tz: self.tz.clone(),
        }
    }

    // ── String-unit rounding (br-frankenpandas-lbsx) ────────────────────
    //
    // Pandas convenience: `.floor('H')` / `.ceil('1D')` / `.round('s')`.
    // These delegate to `Timedelta::unit_to_nanos` for unit lookup, then to
    // the nanos-based `floor_to`/`ceil_to`/`round_to`. Unknown unit strings
    // return NaT, matching the rest of fp-types' "missing-input → missing-
    // output" convention.

    /// Round down to the nearest multiple of the named unit.
    ///
    /// Matches `pd.Timestamp(...).floor(unit)`. Unknown unit → NaT.
    #[must_use]
    pub fn floor_to_unit(&self, unit: &str) -> Self {
        match Timedelta::unit_to_nanos(unit) {
            Some(unit_nanos) => self.floor_to(unit_nanos),
            None => Self::nat(),
        }
    }

    /// Round up to the nearest multiple of the named unit.
    ///
    /// Matches `pd.Timestamp(...).ceil(unit)`. Unknown unit → NaT.
    #[must_use]
    pub fn ceil_to_unit(&self, unit: &str) -> Self {
        match Timedelta::unit_to_nanos(unit) {
            Some(unit_nanos) => self.ceil_to(unit_nanos),
            None => Self::nat(),
        }
    }

    /// Round to the nearest multiple of the named unit, banker's rounding.
    ///
    /// Matches `pd.Timestamp(...).round(unit)`. Unknown unit → NaT.
    #[must_use]
    pub fn round_to_unit(&self, unit: &str) -> Self {
        match Timedelta::unit_to_nanos(unit) {
            Some(unit_nanos) => self.round_to(unit_nanos),
            None => Self::nat(),
        }
    }
}

impl std::fmt::Display for Timestamp {
    /// Phase 2 debug-style format; Phase 3 replaces with pandas ISO-8601
    /// notation once chrono interpretation lands.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_nat() {
            return f.write_str("NaT");
        }
        match &self.tz {
            Some(tz) => write!(f, "Timestamp[{}, {}]", self.nanos, tz),
            None => write!(f, "Timestamp[{}, UTC]", self.nanos),
        }
    }
}

impl PartialOrd for Timestamp {
    /// Orders by nanos axis; NaT is incomparable (`None`). Tz difference
    /// does not affect ordering — two Timestamps at the same absolute
    /// nanos compare equal regardless of tz label (Phase 3 will revisit
    /// whether tz affects ordering semantics).
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.is_nat() || other.is_nat() {
            return None;
        }
        Some(self.nanos.cmp(&other.nanos))
    }
}

// ── Missingness utilities ──────────────────────────────────────────────

pub fn isna(values: &[Scalar]) -> Vec<bool> {
    values.iter().map(Scalar::is_missing).collect()
}

pub fn isnull(values: &[Scalar]) -> Vec<bool> {
    isna(values)
}

pub fn notna(values: &[Scalar]) -> Vec<bool> {
    values.iter().map(|v| !v.is_missing()).collect()
}

pub fn notnull(values: &[Scalar]) -> Vec<bool> {
    notna(values)
}

pub fn count_na(values: &[Scalar]) -> usize {
    values.iter().filter(|v| v.is_missing()).count()
}

pub fn fill_na(values: &[Scalar], fill: &Scalar) -> Vec<Scalar> {
    values
        .iter()
        .map(|v| {
            if v.is_missing() {
                fill.clone()
            } else {
                v.clone()
            }
        })
        .collect()
}

pub fn dropna(values: &[Scalar]) -> Vec<Scalar> {
    values.iter().filter(|v| !v.is_missing()).cloned().collect()
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

pub fn nanany(values: &[Scalar]) -> Scalar {
    for v in values {
        if v.is_missing() {
            continue;
        }
        match v {
            Scalar::Bool(flag) if *flag => return Scalar::Bool(true),
            Scalar::Int64(val) if *val != 0 => return Scalar::Bool(true),
            Scalar::Float64(val) if !val.is_nan() && *val != 0.0 => return Scalar::Bool(true),
            Scalar::Utf8(val) if !val.is_empty() => return Scalar::Bool(true),
            _ => continue,
        }
    }
    Scalar::Bool(false)
}

pub fn nanall(values: &[Scalar]) -> Scalar {
    for v in values {
        if v.is_missing() {
            continue;
        }
        match v {
            Scalar::Bool(flag) if !*flag => return Scalar::Bool(false),
            Scalar::Int64(val) if *val == 0 => return Scalar::Bool(false),
            Scalar::Float64(val) if val.is_nan() || *val == 0.0 => return Scalar::Bool(false),
            Scalar::Utf8(val) if val.is_empty() => return Scalar::Bool(false),
            _ => continue,
        }
    }
    Scalar::Bool(true)
}

pub fn nancount(values: &[Scalar]) -> Scalar {
    let n = values.iter().filter(|v| !v.is_missing()).count();
    Scalar::Int64(n as i64)
}

pub fn nanmin(values: &[Scalar]) -> Scalar {
    let mut min: Option<&Scalar> = None;
    for v in values {
        if v.is_missing() {
            continue;
        }
        match (min, v) {
            (None, _) => min = Some(v),
            (Some(Scalar::Int64(a)), Scalar::Int64(b)) => {
                if b < a {
                    min = Some(v)
                }
            }
            (Some(Scalar::Float64(a)), Scalar::Float64(b)) => {
                if *b < *a {
                    min = Some(v)
                }
            }
            (Some(Scalar::Utf8(a)), Scalar::Utf8(b)) => {
                if b < a {
                    min = Some(v)
                }
            }
            (Some(Scalar::Bool(a)), Scalar::Bool(b)) => {
                if b < a {
                    min = Some(v)
                }
            }
            (Some(a), b) => match (a.to_f64(), b.to_f64()) {
                (Ok(af), Ok(bf)) if bf < af => min = Some(v),
                (Ok(_), Ok(_)) => {}
                _ => return Scalar::Null(NullKind::NaN),
            },
        }
    }
    match min {
        Some(v) => v.clone(),
        None => Scalar::Null(NullKind::NaN),
    }
}

pub fn nanmax(values: &[Scalar]) -> Scalar {
    let mut max: Option<&Scalar> = None;
    for v in values {
        if v.is_missing() {
            continue;
        }
        match (max, v) {
            (None, _) => max = Some(v),
            (Some(Scalar::Int64(a)), Scalar::Int64(b)) => {
                if b > a {
                    max = Some(v)
                }
            }
            (Some(Scalar::Float64(a)), Scalar::Float64(b)) => {
                if *b > *a {
                    max = Some(v)
                }
            }
            (Some(Scalar::Utf8(a)), Scalar::Utf8(b)) => {
                if b > a {
                    max = Some(v)
                }
            }
            (Some(Scalar::Bool(a)), Scalar::Bool(b)) => {
                if b > a {
                    max = Some(v)
                }
            }
            (Some(a), b) => match (a.to_f64(), b.to_f64()) {
                (Ok(af), Ok(bf)) if bf > af => max = Some(v),
                (Ok(_), Ok(_)) => {}
                _ => return Scalar::Null(NullKind::NaN),
            },
        }
    }
    match max {
        Some(v) => v.clone(),
        None => Scalar::Null(NullKind::NaN),
    }
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

/// Standard error of the mean over non-missing values.
///
/// Matches `pd.Series.sem(ddof=1)` / `scipy.stats.sem`. Computed as
/// `std(values, ddof) / sqrt(n)` where `n` is the non-missing count.
/// Returns `Null(NaN)` when `n <= ddof`.
pub fn nansem(values: &[Scalar], ddof: usize) -> Scalar {
    let nums = collect_finite(values);
    if nums.len() <= ddof {
        return Scalar::Null(NullKind::NaN);
    }
    match nanstd(values, ddof) {
        Scalar::Float64(s) => Scalar::Float64(s / (nums.len() as f64).sqrt()),
        other => other,
    }
}

/// Peak-to-peak range of non-missing values (max − min).
///
/// Matches `np.ptp` behavior on nan-safe inputs. Returns `Null(NaN)`
/// for empty or all-missing inputs.
pub fn nanptp(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in &nums {
        if *x < lo {
            lo = *x;
        }
        if *x > hi {
            hi = *x;
        }
    }
    Scalar::Float64(hi - lo)
}

/// Sample skewness (bias-corrected, Fisher-Pearson) over non-missing values.
///
/// Matches `pd.Series.skew()`. Requires at least 3 non-missing values;
/// returns `Null(NaN)` otherwise, and when the sample standard deviation
/// is zero.
pub fn nanskew(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    let n = nums.len() as f64;
    if n < 3.0 {
        return Scalar::Null(NullKind::NaN);
    }
    let mean = nums.iter().sum::<f64>() / n;
    let m2: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    let m3: f64 = nums.iter().map(|x| (x - mean).powi(3)).sum();
    let s2 = m2 / (n - 1.0);
    if s2 == 0.0 {
        return Scalar::Float64(0.0);
    }
    let s3 = s2.powf(1.5);
    Scalar::Float64((n / ((n - 1.0) * (n - 2.0))) * (m3 / s3))
}

/// Excess sample kurtosis (Fisher's definition, bias-corrected) over
/// non-missing values.
///
/// Matches `pd.Series.kurt()`. Requires at least 4 non-missing values;
/// returns `Null(NaN)` otherwise, and when the sample standard deviation
/// is zero.
pub fn nankurt(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    let n = nums.len() as f64;
    if n < 4.0 {
        return Scalar::Null(NullKind::NaN);
    }
    let mean = nums.iter().sum::<f64>() / n;
    let m2: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    let m4: f64 = nums.iter().map(|x| (x - mean).powi(4)).sum();
    let s2 = m2 / (n - 1.0);
    if s2 == 0.0 {
        return Scalar::Float64(0.0);
    }
    let adj = (n * (n + 1.0)) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
    let sub = (3.0 * (n - 1.0).powi(2)) / ((n - 2.0) * (n - 3.0));
    Scalar::Float64(adj * (m4 / (s2 * s2)) - sub)
}

/// Product of non-missing values. Returns 1.0 for empty input (matching pandas).
pub fn nanprod(values: &[Scalar]) -> Scalar {
    let nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Float64(1.0);
    }
    Scalar::Float64(nums.iter().product())
}

/// Cumulative sum respecting null propagation.
///
/// Matches `np.nancumsum` / `pd.Series.cumsum()`. Missing input positions
/// pass through as `Null(NaN)` in the output; the running sum ignores
/// those positions when accumulating.
pub fn nancumsum(values: &[Scalar]) -> Vec<Scalar> {
    let mut out = Vec::with_capacity(values.len());
    let mut running = 0.0_f64;
    for v in values {
        if v.is_missing() {
            out.push(Scalar::Null(NullKind::NaN));
            continue;
        }
        match v.to_f64() {
            Ok(x) if !x.is_nan() => {
                running += x;
                out.push(Scalar::Float64(running));
            }
            _ => out.push(Scalar::Null(NullKind::NaN)),
        }
    }
    out
}

/// Cumulative product respecting null propagation.
///
/// Matches `np.nancumprod` / `pd.Series.cumprod()`. Missing positions
/// pass through as `Null(NaN)` without advancing the running product.
pub fn nancumprod(values: &[Scalar]) -> Vec<Scalar> {
    let mut out = Vec::with_capacity(values.len());
    let mut running = 1.0_f64;
    for v in values {
        if v.is_missing() {
            out.push(Scalar::Null(NullKind::NaN));
            continue;
        }
        match v.to_f64() {
            Ok(x) if !x.is_nan() => {
                running *= x;
                out.push(Scalar::Float64(running));
            }
            _ => out.push(Scalar::Null(NullKind::NaN)),
        }
    }
    out
}

/// Cumulative maximum respecting null propagation.
///
/// Matches `pd.Series.cummax()`. Missing positions pass through as
/// `Null(NaN)` without updating the running maximum. The first
/// non-missing value initializes the running maximum.
pub fn nancummax(values: &[Scalar]) -> Vec<Scalar> {
    let mut out = Vec::with_capacity(values.len());
    let mut running: Option<f64> = None;
    for v in values {
        if v.is_missing() {
            out.push(Scalar::Null(NullKind::NaN));
            continue;
        }
        match v.to_f64() {
            Ok(x) if !x.is_nan() => {
                running = Some(match running {
                    Some(prev) => prev.max(x),
                    None => x,
                });
                out.push(Scalar::Float64(running.unwrap()));
            }
            _ => out.push(Scalar::Null(NullKind::NaN)),
        }
    }
    out
}

/// Cumulative minimum respecting null propagation.
///
/// Matches `pd.Series.cummin()`. Symmetric to `nancummax`.
pub fn nancummin(values: &[Scalar]) -> Vec<Scalar> {
    let mut out = Vec::with_capacity(values.len());
    let mut running: Option<f64> = None;
    for v in values {
        if v.is_missing() {
            out.push(Scalar::Null(NullKind::NaN));
            continue;
        }
        match v.to_f64() {
            Ok(x) if !x.is_nan() => {
                running = Some(match running {
                    Some(prev) => prev.min(x),
                    None => x,
                });
                out.push(Scalar::Float64(running.unwrap()));
            }
            _ => out.push(Scalar::Null(NullKind::NaN)),
        }
    }
    out
}

/// Linear-interpolation quantile over non-missing numeric values.
///
/// Matches `np.nanquantile(values, q)` with `interpolation='linear'`.
/// Returns `Null(NaN)` for empty inputs or when `q` is outside
/// `[0.0, 1.0]`.
pub fn nanquantile(values: &[Scalar], q: f64) -> Scalar {
    if !(0.0..=1.0).contains(&q) {
        return Scalar::Null(NullKind::NaN);
    }
    let mut nums = collect_finite(values);
    if nums.is_empty() {
        return Scalar::Null(NullKind::NaN);
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = nums.len();
    if n == 1 {
        return Scalar::Float64(nums[0]);
    }
    let pos = q * (n - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        return Scalar::Float64(nums[lo]);
    }
    let weight = pos - lo as f64;
    Scalar::Float64(nums[lo] + (nums[hi] - nums[lo]) * weight)
}

/// Position (in the original slice) of the non-missing maximum.
///
/// Matches `np.nanargmax`. Returns `None` if every value is missing.
/// Ties resolve to the first position seen (matching numpy).
pub fn nanargmax(values: &[Scalar]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, v) in values.iter().enumerate() {
        if v.is_missing() {
            continue;
        }
        if let Ok(x) = v.to_f64() {
            if x.is_nan() {
                continue;
            }
            match best {
                None => best = Some((i, x)),
                Some((_, cur)) if x > cur => best = Some((i, x)),
                _ => {}
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Position (in the original slice) of the non-missing minimum.
///
/// Matches `np.nanargmin`. Returns `None` if every value is missing.
pub fn nanargmin(values: &[Scalar]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, v) in values.iter().enumerate() {
        if v.is_missing() {
            continue;
        }
        if let Ok(x) = v.to_f64() {
            if x.is_nan() {
                continue;
            }
            match best {
                None => best = Some((i, x)),
                Some((_, cur)) if x < cur => best = Some((i, x)),
                _ => {}
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Count of unique non-missing values.
pub fn nannunique(values: &[Scalar]) -> Scalar {
    use std::collections::HashSet;
    #[derive(Hash, PartialEq, Eq)]
    enum ScalarKey<'a> {
        Bool(bool),
        Int64(i64),
        FloatBits(u64),
        Utf8(&'a str),
        Timedelta64(i64),
    }

    let mut seen = HashSet::new();
    for val in values {
        if val.is_missing() {
            continue;
        }
        let key = match val {
            Scalar::Bool(v) => ScalarKey::Bool(*v),
            Scalar::Int64(v) => ScalarKey::Int64(*v),
            Scalar::Float64(v) => {
                let normalized = if *v == 0.0 { 0.0 } else { *v };
                ScalarKey::FloatBits(normalized.to_bits())
            }
            Scalar::Utf8(v) => ScalarKey::Utf8(v.as_str()),
            Scalar::Timedelta64(v) => ScalarKey::Timedelta64(*v),
            Scalar::Null(_) => continue,
        };
        seen.insert(key);
    }
    Scalar::Int64(seen.len() as i64)
}

// ── Interval types (br-frankenpandas-j8k4 Phase 1) ──────────────────────
//
// Scaffolding for pandas `pd.Interval` / `pd.IntervalIndex` / `pd.IntervalDtype`.
//
// Phase 1 ships float-valued intervals only (matches `cut`/`qcut` output on
// numeric bins — the dominant pandas use case). Generic-subtype intervals
// over Int64 / Timestamp are deferred to Phase 2 alongside the DType::Interval
// enum-variant wiring. See br-j8k4 for the phased roadmap.
//
// Semantics mirror pandas: closed tells which endpoints are included.
//   Left    → [left, right)
//   Right   → (left, right]       ← pandas default
//   Both    → [left, right]
//   Neither → (left, right)

/// Endpoint-inclusion policy for an `Interval`.
///
/// Matches pandas `pd.Interval.closed` / `pd.IntervalDtype.closed` string
/// values ("left" / "right" / "both" / "neither").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IntervalClosed {
    /// `[left, right)` — left-inclusive, right-exclusive.
    Left,
    /// `(left, right]` — left-exclusive, right-inclusive. Pandas default.
    #[default]
    Right,
    /// `[left, right]` — both endpoints included.
    Both,
    /// `(left, right)` — neither endpoint included.
    Neither,
}

impl IntervalClosed {
    /// Left endpoint included?
    #[must_use]
    pub fn left_closed(self) -> bool {
        matches!(self, Self::Left | Self::Both)
    }

    /// Right endpoint included?
    #[must_use]
    pub fn right_closed(self) -> bool {
        matches!(self, Self::Right | Self::Both)
    }
}

impl std::fmt::Display for IntervalClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Left => write!(f, "left"),
            Self::Right => write!(f, "right"),
            Self::Both => write!(f, "both"),
            Self::Neither => write!(f, "neither"),
        }
    }
}

/// A bounded numeric interval between two `f64` endpoints.
///
/// Matches `pd.Interval(left, right, closed)` on the numeric-subtype path.
/// Accessors match pandas: `.left`, `.right`, `.closed`, `.length`, `.mid`,
/// `.contains`, `.is_empty`, `.overlaps`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub left: f64,
    pub right: f64,
    #[serde(default)]
    pub closed: IntervalClosed,
}

impl Interval {
    /// Construct an interval. No validation on `left <= right` — pandas also
    /// accepts reversed intervals (they're non-empty only if empty-by-design).
    #[must_use]
    pub const fn new(left: f64, right: f64, closed: IntervalClosed) -> Self {
        Self {
            left,
            right,
            closed,
        }
    }

    /// `right - left` (pandas `.length`). Negative for reversed intervals.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.right - self.left
    }

    /// Midpoint `(left + right) / 2` (pandas `.mid`).
    #[must_use]
    pub fn mid(&self) -> f64 {
        (self.left + self.right) / 2.0
    }

    /// Empty iff endpoints coincide AND at least one side is open.
    /// Pandas semantics: `pd.Interval(3, 3, 'right').is_empty → True`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.left == self.right && !matches!(self.closed, IntervalClosed::Both)
    }

    /// Does `value` fall inside this interval?
    ///
    /// NaN always returns false, matching pandas `pd.Interval.__contains__`
    /// behavior (NaN doesn't compare equal to anything).
    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        if value.is_nan() {
            return false;
        }
        let left_ok = if self.closed.left_closed() {
            value >= self.left
        } else {
            value > self.left
        };
        let right_ok = if self.closed.right_closed() {
            value <= self.right
        } else {
            value < self.right
        };
        left_ok && right_ok
    }

    /// Do `self` and `other` share any point?
    ///
    /// Matches `pd.Interval.overlaps(other)`. Two intervals overlap iff the
    /// max of their lefts is less than the min of their rights, with
    /// endpoint-inclusion determining the strictness of the comparison when
    /// they touch exactly.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.left > other.right || other.left > self.right {
            return false;
        }
        // Touching-at-a-point: overlap iff both sides at that touchpoint are closed.
        if self.right == other.left {
            return self.closed.right_closed() && other.closed.left_closed();
        }
        if other.right == self.left {
            return other.closed.right_closed() && self.closed.left_closed();
        }
        true
    }
}

impl std::fmt::Display for Interval {
    /// Matches `str(pd.Interval(0, 5, 'right'))` → `"(0, 5]"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let left_bracket = if self.closed.left_closed() { '[' } else { '(' };
        let right_bracket = if self.closed.right_closed() { ']' } else { ')' };
        write!(
            f,
            "{left_bracket}{}, {}{right_bracket}",
            self.left, self.right
        )
    }
}

// ── interval_range builders (br-frankenpandas-xaom — Phase 2 of j8k4) ────

/// Build `periods` equal-width intervals spanning `[start, end]`.
///
/// Matches `pd.interval_range(start, end, periods=N, closed=...)` for the
/// numeric-subtype case. Returns exactly `periods` intervals; when
/// `periods == 0` or `start >= end`, returns an empty vector (matches
/// pandas's empty IntervalIndex).
///
/// ```
/// use fp_types::{interval_range_by_periods, IntervalClosed};
/// let bins = interval_range_by_periods(0.0, 10.0, 5, IntervalClosed::Right);
/// assert_eq!(bins.len(), 5);
/// assert_eq!(bins[0].left, 0.0);
/// assert_eq!(bins[0].right, 2.0);
/// assert_eq!(bins[4].right, 10.0);
/// ```
#[must_use]
pub fn interval_range_by_periods(
    start: f64,
    end: f64,
    periods: usize,
    closed: IntervalClosed,
) -> Vec<Interval> {
    if periods == 0 || !start.is_finite() || !end.is_finite() || start >= end {
        return Vec::new();
    }
    let step = (end - start) / (periods as f64);
    let mut out = Vec::with_capacity(periods);
    for i in 0..periods {
        let left = start + step * (i as f64);
        // Use end exactly for the final right edge to avoid float drift.
        let right = if i + 1 == periods {
            end
        } else {
            start + step * ((i + 1) as f64)
        };
        out.push(Interval::new(left, right, closed));
    }
    out
}

/// Build equal-`step`-width intervals spanning `[start, end]`.
///
/// Matches `pd.interval_range(start, end, freq=step, closed=...)` for the
/// numeric-subtype case. `step` must be finite and positive; `(end - start)`
/// must be an integer multiple of `step` (within float tolerance) — pandas
/// raises `ValueError` otherwise; this fn returns `Err(TypeError::IntervalStepDoesNotDivide)`.
///
/// Returns an empty vector when `start == end` (matches pandas' zero-bin
/// IntervalIndex); returns an empty vector when `start > end` (pandas also
/// returns empty rather than erroring in this case).
pub fn interval_range_by_step(
    start: f64,
    end: f64,
    step: f64,
    closed: IntervalClosed,
) -> Result<Vec<Interval>, TypeError> {
    if !step.is_finite() || !step.is_sign_positive() || step == 0.0 {
        return Err(TypeError::InvalidIntervalStep { step });
    }
    if !start.is_finite() || !end.is_finite() || start >= end {
        return Ok(Vec::new());
    }
    let span = end - start;
    let periods_f = span / step;
    let periods = periods_f.round() as i64;
    if periods <= 0 {
        return Ok(Vec::new());
    }
    let reconstructed = step * (periods as f64);
    // Relative tolerance: allow float-rounding noise proportional to span.
    if (span - reconstructed).abs() > span.abs() * 1e-9 + 1e-12 {
        return Err(TypeError::IntervalStepDoesNotDivide { step, span });
    }
    let periods = periods as usize;
    let mut out = Vec::with_capacity(periods);
    for i in 0..periods {
        let left = start + step * (i as f64);
        let right = if i + 1 == periods {
            end
        } else {
            start + step * ((i + 1) as f64)
        };
        out.push(Interval::new(left, right, closed));
    }
    Ok(out)
}

// ── Period types (br-frankenpandas-epoj Phase 1) ────────────────────────
//
// Scaffolding for pandas `pd.Period` / `pd.PeriodIndex` / `pd.PeriodDtype`.
//
// A Period is a calendar *span* (Q1 2024, Jan 2024, 2024-03-15), distinct
// from a Timestamp (an instant). Phase 1 ships the PeriodFreq enum +
// Period struct with ordinal-based arithmetic (Period + n, Period - Period),
// Display in pandas notation, and parse from standard strings. Calendar-
// conversion (ordinal ↔ ymd) and DType::Period wiring land in Phase 2.

/// Period frequency code. Matches pandas offset alias core set.
///
/// The ordinal axis is frequency-specific: for Monthly, ordinal 0 is a
/// fixed anchor (pandas uses months since 1970-01). Phase 1 doesn't
/// commit to a specific epoch yet — the ordinal scheme is opaque until
/// Phase 2 wires calendar arithmetic. What Phase 1 DOES nail down is:
/// same-freq Periods compare + subtract; Period + i64 shifts by `n`
/// periods of the declared frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING-KEBAB-CASE")]
#[non_exhaustive]
pub enum PeriodFreq {
    /// `A` / `Y` — annual periods.
    Annual,
    /// `Q` — quarterly periods.
    Quarterly,
    /// `M` — monthly periods.
    Monthly,
    /// `W` — weekly periods.
    Weekly,
    /// `D` — daily periods.
    Daily,
    /// `B` — business-day periods.
    Business,
    /// `H` — hourly periods.
    Hourly,
    /// `T` / `min` — minutely periods.
    Minutely,
    /// `S` — secondly periods.
    Secondly,
}

impl PeriodFreq {
    /// Parse a pandas-style frequency alias. Recognizes the common subset
    /// (A/Y, Q, M, W, D, B, H, T/min, S). Case-insensitive.
    pub fn parse(alias: &str) -> Option<Self> {
        match alias.to_ascii_uppercase().as_str() {
            "A" | "Y" | "ANNUAL" | "YEARLY" => Some(Self::Annual),
            "Q" | "QUARTERLY" => Some(Self::Quarterly),
            "M" | "MONTHLY" => Some(Self::Monthly),
            "W" | "WEEKLY" => Some(Self::Weekly),
            "D" | "DAILY" => Some(Self::Daily),
            "B" | "BUSINESS" => Some(Self::Business),
            "H" | "HOURLY" => Some(Self::Hourly),
            "T" | "MIN" | "MINUTELY" => Some(Self::Minutely),
            "S" | "SECONDLY" => Some(Self::Secondly),
            _ => None,
        }
    }

    /// Canonical pandas alias string.
    #[must_use]
    pub const fn alias(self) -> &'static str {
        match self {
            Self::Annual => "A",
            Self::Quarterly => "Q",
            Self::Monthly => "M",
            Self::Weekly => "W",
            Self::Daily => "D",
            Self::Business => "B",
            Self::Hourly => "H",
            Self::Minutely => "T",
            Self::Secondly => "S",
        }
    }
}

impl std::fmt::Display for PeriodFreq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.alias())
    }
}

/// A single pandas-style Period value.
///
/// Stored as an integer ordinal on a frequency-specific axis plus the
/// frequency code. Two Periods with different `freq` are incompatible —
/// arithmetic and comparison require same-freq operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Period {
    pub ordinal: i64,
    pub freq: PeriodFreq,
}

impl Period {
    #[must_use]
    pub const fn new(ordinal: i64, freq: PeriodFreq) -> Self {
        Self { ordinal, freq }
    }

    /// Same-freq ordinal comparison. Returns `None` if `freq` differs —
    /// caller decides whether that's an error or a panic site.
    #[must_use]
    pub fn cmp_same_freq(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.freq != other.freq {
            return None;
        }
        Some(self.ordinal.cmp(&other.ordinal))
    }

    /// Shift by `n` periods of the current frequency.
    /// Matches `pd.Period + n` and `pd.Period - n`.
    #[must_use]
    pub fn shift(&self, n: i64) -> Self {
        Self {
            ordinal: self.ordinal.saturating_add(n),
            freq: self.freq,
        }
    }

    /// Period-difference in units of the shared frequency.
    /// Returns `None` if `freq` differs (pandas raises IncompatibleFrequency).
    #[must_use]
    pub fn diff(&self, other: &Self) -> Option<i64> {
        if self.freq != other.freq {
            return None;
        }
        Some(self.ordinal.saturating_sub(other.ordinal))
    }
}

impl std::fmt::Display for Period {
    /// Phase 1: ordinal+freq form, e.g. `Period[Q, 216]`. Calendar-
    /// formatted display (`2024Q1`, `2024-03`) lands in Phase 2 once the
    /// ordinal-to-ymd arithmetic is wired.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Period[{}, {}]", self.freq, self.ordinal)
    }
}

/// Build `periods` consecutive Periods starting at `start`.
///
/// Matches `pd.period_range(start, periods=N, freq=start.freq)` for the
/// count-based form. The frequency is taken from `start` — pandas requires
/// `freq` to match when both are passed; mismatches are an error in
/// pandas, but here we sidestep ambiguity by deriving from `start.freq`.
///
/// Per br-frankenpandas-2jef (epoj Phase 2). Pure ordinal arithmetic — no
/// calendar conversion (Phase 3 wires chrono). `periods=0` returns empty.
///
/// ```
/// use fp_types::{period_range, Period, PeriodFreq};
/// let q1 = Period::new(216, PeriodFreq::Quarterly);
/// let year = period_range(q1, 4);
/// assert_eq!(year.len(), 4);
/// assert_eq!(year[0].ordinal, 216);
/// assert_eq!(year[3].ordinal, 219);
/// ```
#[must_use]
pub fn period_range(start: Period, periods: usize) -> Vec<Period> {
    (0..periods).map(|i| start.shift(i as i64)).collect()
}

#[cfg(test)]
mod tests {
    use super::{DType, NullKind, Scalar, SparseDType, cast_scalar, common_dtype, infer_dtype};

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
    fn semantic_eq_treats_nan_as_missing_null() {
        let left = Scalar::Float64(f64::NAN);
        let right = Scalar::Null(NullKind::Null);
        assert!(left.semantic_eq(&right));
    }

    #[test]
    fn common_dtype_rejects_string_numeric_mix() {
        let err = common_dtype(DType::Utf8, DType::Int64).expect_err("must fail");
        assert_eq!(
            err.to_string(),
            "dtype coercion from Utf8 to Int64 has no compatible common type"
        );
        let err = common_dtype(DType::Float64, DType::Utf8).expect_err("must fail");
        assert_eq!(
            err.to_string(),
            "dtype coercion from Float64 to Utf8 has no compatible common type"
        );
    }

    #[test]
    fn sparse_dtype_normalizes_fill_value_to_value_dtype() {
        let dtype =
            SparseDType::new(DType::Float64, Scalar::Int64(0)).expect("fill should cast");

        assert_eq!(dtype.value_dtype, DType::Float64);
        assert_eq!(dtype.fill_value, Scalar::Float64(0.0));
    }

    #[test]
    fn sparse_dtype_rejects_sparse_value_dtype() {
        let err = SparseDType::new(DType::Sparse, Scalar::Int64(0)).expect_err("must reject");

        assert_eq!(err.to_string(), "sparse value dtype cannot be Sparse");
    }

    #[test]
    fn common_dtype_rejects_sparse_dense_mix() {
        let err = common_dtype(DType::Sparse, DType::Int64).expect_err("must fail");

        assert_eq!(
            err.to_string(),
            "dtype coercion from Sparse to Int64 has no compatible common type"
        );
    }

    #[test]
    fn infer_dtype_preserves_string_numeric_mix_as_utf8_bucket() {
        let values = vec![Scalar::Utf8("x".into()), Scalar::Int64(7)];
        assert_eq!(
            infer_dtype(&values).expect("dtype should infer"),
            DType::Utf8
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
    fn nannunique_merges_negative_zero_and_zero() {
        let vals = vec![
            Scalar::Float64(-0.0),
            Scalar::Float64(0.0),
            Scalar::Float64(1.0),
        ];
        assert_eq!(super::nannunique(&vals), Scalar::Int64(2));
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
    fn nanmin_nanmax_mixed_incompatible_types_returns_nan() {
        let vals = vec![Scalar::Int64(5), Scalar::Utf8("hello".into())];
        assert!(super::nanmin(&vals).is_missing());
        assert!(super::nanmax(&vals).is_missing());

        let vals2 = vec![Scalar::Utf8("a".into()), Scalar::Float64(3.0)];
        assert!(super::nanmin(&vals2).is_missing());
        assert!(super::nanmax(&vals2).is_missing());
    }

    #[test]
    fn nanmin_nanmax_compatible_numeric_types_ok() {
        let vals = vec![Scalar::Int64(5), Scalar::Float64(3.0), Scalar::Bool(true)];
        assert_eq!(super::nanmin(&vals), Scalar::Bool(true));
        assert_eq!(super::nanmax(&vals), Scalar::Int64(5));
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
        assert!(matches!(var, Scalar::Float64(_)), "expected Float64");
        if let Scalar::Float64(v) = var {
            assert!((v - 4.0).abs() < 1e-10);
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
        assert!(matches!(var, Scalar::Float64(_)), "expected Float64");
        if let Scalar::Float64(v) = var {
            assert!((v - 32.0 / 7.0).abs() < 1e-10);
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
        assert!(matches!(std, Scalar::Float64(_)), "expected Float64");
        if let Scalar::Float64(v) = std {
            assert!((v - 2.0).abs() < 1e-10);
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

    // ── Timedelta tests ────────────────────────────────────────────────

    #[test]
    fn timedelta_parse_simple_units() {
        use super::Timedelta;
        assert_eq!(Timedelta::parse("1d").unwrap(), Timedelta::NANOS_PER_DAY);
        assert_eq!(
            Timedelta::parse("2h").unwrap(),
            2 * Timedelta::NANOS_PER_HOUR
        );
        assert_eq!(
            Timedelta::parse("30m").unwrap(),
            30 * Timedelta::NANOS_PER_MIN
        );
        assert_eq!(
            Timedelta::parse("45s").unwrap(),
            45 * Timedelta::NANOS_PER_SEC
        );
        assert_eq!(
            Timedelta::parse("100ms").unwrap(),
            100 * Timedelta::NANOS_PER_MILLI
        );
        assert_eq!(
            Timedelta::parse("500us").unwrap(),
            500 * Timedelta::NANOS_PER_MICRO
        );
        assert_eq!(Timedelta::parse("1000ns").unwrap(), 1000);
    }

    #[test]
    fn timedelta_parse_compound() {
        use super::Timedelta;
        let expected = Timedelta::NANOS_PER_DAY
            + 2 * Timedelta::NANOS_PER_HOUR
            + 30 * Timedelta::NANOS_PER_MIN;
        assert_eq!(Timedelta::parse("1d 2h 30m").unwrap(), expected);
        assert_eq!(Timedelta::parse("1d2h30m").unwrap(), expected);
    }

    #[test]
    fn timedelta_parse_time_format() {
        use super::Timedelta;
        let expected = Timedelta::NANOS_PER_HOUR
            + 30 * Timedelta::NANOS_PER_MIN
            + 45 * Timedelta::NANOS_PER_SEC;
        assert_eq!(Timedelta::parse("01:30:45").unwrap(), expected);
    }

    #[test]
    fn timedelta_parse_nat() {
        use super::Timedelta;
        assert_eq!(Timedelta::parse("NaT").unwrap(), Timedelta::NAT);
        assert_eq!(Timedelta::parse("nat").unwrap(), Timedelta::NAT);
    }

    #[test]
    fn timedelta_parse_negative() {
        use super::Timedelta;
        assert_eq!(Timedelta::parse("-1d").unwrap(), -Timedelta::NANOS_PER_DAY);
    }

    #[test]
    fn timedelta_components() {
        use super::Timedelta;
        let nanos = Timedelta::NANOS_PER_DAY
            + Timedelta::NANOS_PER_HOUR
            + Timedelta::NANOS_PER_MIN
            + Timedelta::NANOS_PER_SEC
            + Timedelta::NANOS_PER_MILLI
            + 2 * Timedelta::NANOS_PER_MICRO
            + 3;
        let comp = Timedelta::components(nanos);
        assert_eq!(comp.days, 1);
        assert_eq!(comp.hours, 1);
        assert_eq!(comp.minutes, 1);
        assert_eq!(comp.seconds, 1);
        assert_eq!(comp.milliseconds, 1);
        assert_eq!(comp.microseconds, 2);
        assert_eq!(comp.nanoseconds, 3);
    }

    #[test]
    fn timedelta_total_seconds() {
        use super::Timedelta;
        let nanos = 90_000_000_000i64; // 90 seconds
        assert!((Timedelta::total_seconds(nanos) - 90.0).abs() < 1e-9);
        assert!(Timedelta::total_seconds(Timedelta::NAT).is_nan());
    }

    #[test]
    fn timedelta_format_basic() {
        use super::Timedelta;
        assert_eq!(Timedelta::format(Timedelta::NAT), "NaT");
        assert_eq!(
            Timedelta::format(Timedelta::NANOS_PER_DAY),
            "1 days 00:00:00"
        );
        assert_eq!(
            Timedelta::format(Timedelta::NANOS_PER_DAY + 2 * Timedelta::NANOS_PER_HOUR),
            "1 days 02:00:00"
        );
    }

    #[test]
    fn timedelta_scalar_dtype() {
        let td = Scalar::Timedelta64(86_400_000_000_000);
        assert_eq!(td.dtype(), DType::Timedelta64);
    }

    #[test]
    fn timedelta_scalar_is_missing() {
        use super::Timedelta;
        let valid = Scalar::Timedelta64(1000);
        let nat = Scalar::Timedelta64(Timedelta::NAT);
        assert!(!valid.is_missing());
        assert!(nat.is_missing());
    }

    #[test]
    fn dtype_utf8_deserializes_legacy_aliases() {
        let dtype: DType = serde_json::from_str("\"str\"").unwrap();
        assert_eq!(dtype, DType::Utf8);

        let dtype: DType = serde_json::from_str("\"string\"").unwrap();
        assert_eq!(dtype, DType::Utf8);
    }

    #[test]
    fn scalar_utf8_deserializes_legacy_aliases() {
        let scalar: Scalar = serde_json::from_str(r#"{"kind":"str","value":"x"}"#).unwrap();
        assert_eq!(scalar, Scalar::Utf8("x".to_owned()));

        let scalar: Scalar = serde_json::from_str(r#"{"kind":"string","value":"y"}"#).unwrap();
        assert_eq!(scalar, Scalar::Utf8("y".to_owned()));
    }

    #[test]
    fn nancumsum_skips_nulls_and_accumulates() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
        ];
        let out = super::nancumsum(&values);
        assert!(matches!(out[0], Scalar::Float64(v) if (v - 1.0).abs() < 1e-9));
        assert!(out[1].is_missing());
        assert!(matches!(out[2], Scalar::Float64(v) if (v - 3.0).abs() < 1e-9));
        assert!(matches!(out[3], Scalar::Float64(v) if (v - 6.0).abs() < 1e-9));
    }

    #[test]
    fn nancumprod_skips_nulls_and_multiplies() {
        let values = vec![
            Scalar::Float64(2.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
        ];
        let out = super::nancumprod(&values);
        assert!(matches!(out[0], Scalar::Float64(v) if (v - 2.0).abs() < 1e-9));
        assert!(out[1].is_missing());
        assert!(matches!(out[2], Scalar::Float64(v) if (v - 6.0).abs() < 1e-9));
        assert!(matches!(out[3], Scalar::Float64(v) if (v - 24.0).abs() < 1e-9));
    }

    #[test]
    fn nancummax_tracks_running_max() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(2.0),
            Scalar::Float64(5.0),
        ];
        let out = super::nancummax(&values);
        assert_eq!(out[0], Scalar::Float64(1.0));
        assert_eq!(out[1], Scalar::Float64(3.0));
        assert!(out[2].is_missing());
        assert_eq!(out[3], Scalar::Float64(3.0));
        assert_eq!(out[4], Scalar::Float64(5.0));
    }

    #[test]
    fn nancummin_tracks_running_min() {
        let values = vec![
            Scalar::Float64(5.0),
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(4.0),
            Scalar::Float64(1.0),
        ];
        let out = super::nancummin(&values);
        assert_eq!(out[0], Scalar::Float64(5.0));
        assert_eq!(out[1], Scalar::Float64(3.0));
        assert!(out[2].is_missing());
        assert_eq!(out[3], Scalar::Float64(3.0));
        assert_eq!(out[4], Scalar::Float64(1.0));
    }

    #[test]
    fn nanquantile_linear_interpolation_matches_numpy() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
        ];
        // median
        let q = super::nanquantile(&values, 0.5);
        assert!(matches!(q, Scalar::Float64(v) if (v - 3.0).abs() < 1e-9));
        // 25th percentile: interpolate between 2.0 and 3.0 at pos 1.0 → 2.0
        let q25 = super::nanquantile(&values, 0.25);
        assert!(matches!(q25, Scalar::Float64(v) if (v - 2.0).abs() < 1e-9));
    }

    #[test]
    fn nanquantile_ignores_nulls() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
        ];
        let q = super::nanquantile(&values, 0.5);
        assert!(matches!(q, Scalar::Float64(v) if (v - 2.0).abs() < 1e-9));
    }

    #[test]
    fn nanquantile_empty_and_out_of_range_yield_null() {
        assert!(super::nanquantile(&[], 0.5).is_missing());
        assert!(super::nanquantile(&[Scalar::Float64(1.0)], 1.5).is_missing());
        assert!(super::nanquantile(&[Scalar::Float64(1.0)], -0.1).is_missing());
    }

    #[test]
    fn nanargmax_returns_first_position() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(2.0),
        ];
        assert_eq!(super::nanargmax(&values), Some(2));
    }

    #[test]
    fn nanargmin_returns_first_position() {
        let values = vec![
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(1.0),
            Scalar::Float64(1.0),
        ];
        assert_eq!(super::nanargmin(&values), Some(2));
    }

    #[test]
    fn nanargmax_all_missing_returns_none() {
        let values = vec![Scalar::Null(NullKind::NaN), Scalar::Null(NullKind::Null)];
        assert_eq!(super::nanargmax(&values), None);
        assert_eq!(super::nanargmin(&values), None);
    }

    #[test]
    fn nansem_matches_std_over_sqrt_n() {
        let values = vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(5.0),
            Scalar::Float64(7.0),
            Scalar::Float64(9.0),
        ];
        // numpy/scipy: std(ddof=1) = 2.138089935299395; sem = std/sqrt(8) = 0.7559
        let sem = super::nansem(&values, 1);
        assert!(matches!(sem, Scalar::Float64(_)));
        let Scalar::Float64(v) = sem else {
            return;
        };
        assert!((v - 0.7559289460184544).abs() < 1e-9);
    }

    #[test]
    fn nansem_empty_returns_null() {
        assert!(super::nansem(&[], 1).is_missing());
        assert!(super::nansem(&[Scalar::Float64(1.0)], 1).is_missing());
    }

    #[test]
    fn nanptp_returns_max_minus_min() {
        let values = vec![
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(7.0),
            Scalar::Float64(1.0),
        ];
        assert_eq!(super::nanptp(&values), Scalar::Float64(6.0));
    }

    #[test]
    fn nanptp_empty_returns_null() {
        assert!(super::nanptp(&[]).is_missing());
        assert!(super::nanptp(&[Scalar::Null(NullKind::NaN)]).is_missing());
    }

    #[test]
    fn nanskew_symmetric_distribution_near_zero() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
        ];
        // Perfectly symmetric -> skew = 0
        let skew = super::nanskew(&values);
        assert!(matches!(skew, Scalar::Float64(_)));
        let Scalar::Float64(v) = skew else {
            return;
        };
        assert!(v.abs() < 1e-9);
    }

    #[test]
    fn nanskew_too_few_values_returns_null() {
        assert!(super::nanskew(&[]).is_missing());
        assert!(super::nanskew(&[Scalar::Float64(1.0), Scalar::Float64(2.0)]).is_missing());
    }

    #[test]
    fn nankurt_symmetric_uniform_distribution() {
        let values = vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
        ];
        // pandas kurt([1,2,3,4,5]) = -1.2
        let kurt = super::nankurt(&values);
        assert!(matches!(kurt, Scalar::Float64(_)));
        let Scalar::Float64(v) = kurt else {
            return;
        };
        assert!((v + 1.2).abs() < 1e-9);
    }

    #[test]
    fn nankurt_too_few_values_returns_null() {
        let vals: Vec<Scalar> = (0..3).map(|i| Scalar::Float64(i as f64)).collect();
        assert!(super::nankurt(&vals).is_missing());
    }

    #[test]
    fn nanskew_constant_series_returns_zero() {
        let values = vec![
            Scalar::Float64(5.0),
            Scalar::Float64(5.0),
            Scalar::Float64(5.0),
        ];
        assert_eq!(super::nanskew(&values), Scalar::Float64(0.0));
        assert_eq!(
            super::nankurt(&[
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
                Scalar::Float64(5.0),
            ]),
            Scalar::Float64(0.0)
        );
    }

    // ── Interval tests (br-frankenpandas-j8k4) ──────────────────────────

    use super::{Interval, IntervalClosed};

    #[test]
    fn interval_default_closed_is_right() {
        assert_eq!(IntervalClosed::default(), IntervalClosed::Right);
    }

    #[test]
    fn interval_left_and_right_closed_helpers() {
        assert!(IntervalClosed::Left.left_closed());
        assert!(!IntervalClosed::Left.right_closed());
        assert!(!IntervalClosed::Right.left_closed());
        assert!(IntervalClosed::Right.right_closed());
        assert!(IntervalClosed::Both.left_closed());
        assert!(IntervalClosed::Both.right_closed());
        assert!(!IntervalClosed::Neither.left_closed());
        assert!(!IntervalClosed::Neither.right_closed());
    }

    #[test]
    fn interval_display_matches_pandas_notation() {
        assert_eq!(
            Interval::new(0.0, 5.0, IntervalClosed::Right).to_string(),
            "(0, 5]"
        );
        assert_eq!(
            Interval::new(0.0, 5.0, IntervalClosed::Left).to_string(),
            "[0, 5)"
        );
        assert_eq!(
            Interval::new(0.0, 5.0, IntervalClosed::Both).to_string(),
            "[0, 5]"
        );
        assert_eq!(
            Interval::new(0.0, 5.0, IntervalClosed::Neither).to_string(),
            "(0, 5)"
        );
    }

    #[test]
    fn interval_length_and_mid() {
        let i = Interval::new(2.0, 10.0, IntervalClosed::Right);
        assert_eq!(i.length(), 8.0);
        assert_eq!(i.mid(), 6.0);
    }

    #[test]
    fn interval_contains_matches_closed_policy() {
        let right = Interval::new(0.0, 5.0, IntervalClosed::Right);
        assert!(!right.contains(0.0));
        assert!(right.contains(2.5));
        assert!(right.contains(5.0));

        let left = Interval::new(0.0, 5.0, IntervalClosed::Left);
        assert!(left.contains(0.0));
        assert!(left.contains(2.5));
        assert!(!left.contains(5.0));

        let both = Interval::new(0.0, 5.0, IntervalClosed::Both);
        assert!(both.contains(0.0));
        assert!(both.contains(5.0));

        let neither = Interval::new(0.0, 5.0, IntervalClosed::Neither);
        assert!(!neither.contains(0.0));
        assert!(!neither.contains(5.0));
        assert!(neither.contains(2.5));
    }

    #[test]
    fn interval_contains_nan_returns_false() {
        let i = Interval::new(0.0, 10.0, IntervalClosed::Both);
        assert!(!i.contains(f64::NAN));
    }

    #[test]
    fn interval_is_empty_matches_pandas() {
        // pd.Interval(3, 3, 'right').is_empty → True
        assert!(Interval::new(3.0, 3.0, IntervalClosed::Right).is_empty());
        assert!(Interval::new(3.0, 3.0, IntervalClosed::Left).is_empty());
        assert!(Interval::new(3.0, 3.0, IntervalClosed::Neither).is_empty());
        // pd.Interval(3, 3, 'both').is_empty → False (single point)
        assert!(!Interval::new(3.0, 3.0, IntervalClosed::Both).is_empty());
        // Non-degenerate intervals are never empty.
        assert!(!Interval::new(0.0, 5.0, IntervalClosed::Right).is_empty());
    }

    #[test]
    fn interval_overlaps_disjoint_returns_false() {
        let a = Interval::new(0.0, 1.0, IntervalClosed::Right);
        let b = Interval::new(2.0, 3.0, IntervalClosed::Right);
        assert!(!a.overlaps(&b));
        assert!(!b.overlaps(&a));
    }

    #[test]
    fn interval_overlaps_nested_returns_true() {
        let outer = Interval::new(0.0, 10.0, IntervalClosed::Right);
        let inner = Interval::new(3.0, 7.0, IntervalClosed::Right);
        assert!(outer.overlaps(&inner));
        assert!(inner.overlaps(&outer));
    }

    #[test]
    fn interval_overlaps_touching_respects_closed_policy() {
        // (0, 1] touching (1, 2] at point 1.
        let right_right = (
            Interval::new(0.0, 1.0, IntervalClosed::Right),
            Interval::new(1.0, 2.0, IntervalClosed::Right),
        );
        // right_right.0 is closed at 1; right_right.1 is open at 1 → no overlap.
        assert!(!right_right.0.overlaps(&right_right.1));

        // [0, 1] touching [1, 2] — both closed at 1 → overlap.
        let both_both = (
            Interval::new(0.0, 1.0, IntervalClosed::Both),
            Interval::new(1.0, 2.0, IntervalClosed::Both),
        );
        assert!(both_both.0.overlaps(&both_both.1));
    }

    #[test]
    fn interval_roundtrips_through_serde_json() {
        let i = Interval::new(1.5, 3.25, IntervalClosed::Both);
        let json = serde_json::to_string(&i).expect("serialize");
        let back: Interval = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(i, back);
    }

    #[test]
    fn interval_serde_default_closed_is_right_when_missing() {
        // JSON payloads that omit `closed` deserialize with the pandas default.
        let back: Interval =
            serde_json::from_str(r#"{"left":0.0,"right":5.0}"#).expect("deserialize");
        assert_eq!(back.closed, IntervalClosed::Right);
    }

    // ── Period tests (br-frankenpandas-epoj) ────────────────────────────

    use super::{Period, PeriodFreq};

    #[test]
    fn period_freq_parses_canonical_aliases() {
        assert_eq!(PeriodFreq::parse("A"), Some(PeriodFreq::Annual));
        assert_eq!(PeriodFreq::parse("Y"), Some(PeriodFreq::Annual));
        assert_eq!(PeriodFreq::parse("Q"), Some(PeriodFreq::Quarterly));
        assert_eq!(PeriodFreq::parse("M"), Some(PeriodFreq::Monthly));
        assert_eq!(PeriodFreq::parse("W"), Some(PeriodFreq::Weekly));
        assert_eq!(PeriodFreq::parse("D"), Some(PeriodFreq::Daily));
        assert_eq!(PeriodFreq::parse("B"), Some(PeriodFreq::Business));
        assert_eq!(PeriodFreq::parse("H"), Some(PeriodFreq::Hourly));
        assert_eq!(PeriodFreq::parse("T"), Some(PeriodFreq::Minutely));
        assert_eq!(PeriodFreq::parse("min"), Some(PeriodFreq::Minutely));
        assert_eq!(PeriodFreq::parse("S"), Some(PeriodFreq::Secondly));
    }

    #[test]
    fn period_freq_parse_is_case_insensitive() {
        assert_eq!(PeriodFreq::parse("quarterly"), Some(PeriodFreq::Quarterly));
        assert_eq!(PeriodFreq::parse("MONTHLY"), Some(PeriodFreq::Monthly));
    }

    #[test]
    fn period_freq_rejects_unknown_aliases() {
        assert_eq!(PeriodFreq::parse("nanosec"), None);
        assert_eq!(PeriodFreq::parse(""), None);
        assert_eq!(PeriodFreq::parse("xyz"), None);
    }

    #[test]
    fn period_freq_alias_roundtrip() {
        for freq in [
            PeriodFreq::Annual,
            PeriodFreq::Quarterly,
            PeriodFreq::Monthly,
            PeriodFreq::Weekly,
            PeriodFreq::Daily,
            PeriodFreq::Business,
            PeriodFreq::Hourly,
            PeriodFreq::Minutely,
            PeriodFreq::Secondly,
        ] {
            assert_eq!(PeriodFreq::parse(freq.alias()), Some(freq));
        }
    }

    #[test]
    fn period_shift_advances_ordinal() {
        let q1 = Period::new(216, PeriodFreq::Quarterly);
        let q2 = q1.shift(1);
        assert_eq!(q2.ordinal, 217);
        assert_eq!(q2.freq, PeriodFreq::Quarterly);
        let q0 = q1.shift(-1);
        assert_eq!(q0.ordinal, 215);
    }

    #[test]
    fn period_shift_saturates_on_overflow() {
        let p = Period::new(i64::MAX - 2, PeriodFreq::Daily);
        assert_eq!(p.shift(100).ordinal, i64::MAX);
        let p = Period::new(i64::MIN + 2, PeriodFreq::Daily);
        assert_eq!(p.shift(-100).ordinal, i64::MIN);
    }

    #[test]
    fn period_diff_returns_period_count() {
        let a = Period::new(216, PeriodFreq::Quarterly);
        let b = Period::new(220, PeriodFreq::Quarterly);
        assert_eq!(b.diff(&a), Some(4));
        assert_eq!(a.diff(&b), Some(-4));
    }

    #[test]
    fn period_diff_rejects_mismatched_freq() {
        let monthly = Period::new(100, PeriodFreq::Monthly);
        let quarterly = Period::new(100, PeriodFreq::Quarterly);
        assert_eq!(monthly.diff(&quarterly), None);
        assert_eq!(quarterly.diff(&monthly), None);
    }

    #[test]
    fn period_cmp_same_freq_respects_ordinal_order() {
        use std::cmp::Ordering;
        let a = Period::new(10, PeriodFreq::Monthly);
        let b = Period::new(20, PeriodFreq::Monthly);
        assert_eq!(a.cmp_same_freq(&b), Some(Ordering::Less));
        assert_eq!(b.cmp_same_freq(&a), Some(Ordering::Greater));
        assert_eq!(a.cmp_same_freq(&a), Some(Ordering::Equal));
    }

    #[test]
    fn period_cmp_cross_freq_returns_none() {
        let m = Period::new(1, PeriodFreq::Monthly);
        let q = Period::new(1, PeriodFreq::Quarterly);
        assert_eq!(m.cmp_same_freq(&q), None);
    }

    #[test]
    fn period_display_carries_freq_and_ordinal() {
        let p = Period::new(216, PeriodFreq::Quarterly);
        assert_eq!(p.to_string(), "Period[Q, 216]");
    }

    #[test]
    fn period_roundtrips_through_serde_json() {
        let p = Period::new(42, PeriodFreq::Weekly);
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Period = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, back);
    }

    // ── period_range tests (br-frankenpandas-2jef — epoj Phase 2) ───────

    use super::period_range;

    #[test]
    fn period_range_zero_periods_is_empty() {
        let start = Period::new(216, PeriodFreq::Quarterly);
        assert!(period_range(start, 0).is_empty());
    }

    #[test]
    fn period_range_single_period_returns_start_only() {
        let start = Period::new(216, PeriodFreq::Quarterly);
        let r = period_range(start, 1);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], start);
    }

    #[test]
    fn period_range_increments_ordinal_by_one_per_step() {
        let start = Period::new(216, PeriodFreq::Quarterly);
        let r = period_range(start, 4);
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].ordinal, 216);
        assert_eq!(r[1].ordinal, 217);
        assert_eq!(r[2].ordinal, 218);
        assert_eq!(r[3].ordinal, 219);
    }

    #[test]
    fn period_range_preserves_frequency() {
        let start = Period::new(0, PeriodFreq::Monthly);
        let r = period_range(start, 12);
        assert!(r.iter().all(|p| p.freq == PeriodFreq::Monthly));
    }

    #[test]
    fn period_range_negative_starting_ordinal_works() {
        // Ordinal axis is signed — pre-epoch periods are valid.
        let start = Period::new(-3, PeriodFreq::Annual);
        let r = period_range(start, 5);
        assert_eq!(r.iter().map(|p| p.ordinal).collect::<Vec<_>>(), vec![-3, -2, -1, 0, 1]);
    }

    #[test]
    fn period_range_large_n_does_not_panic() {
        // 1024 monthly periods — large enough to catch any allocation bug.
        let start = Period::new(0, PeriodFreq::Monthly);
        let r = period_range(start, 1024);
        assert_eq!(r.len(), 1024);
        assert_eq!(r[1023].ordinal, 1023);
    }

    // ── interval_range tests (br-frankenpandas-xaom) ────────────────────

    use super::{TypeError, interval_range_by_periods, interval_range_by_step};

    #[test]
    fn interval_range_by_periods_matches_pandas_default_case() {
        // pd.interval_range(0, 10, periods=5) → [(0,2],(2,4],(4,6],(6,8],(8,10]]
        let bins = interval_range_by_periods(0.0, 10.0, 5, IntervalClosed::Right);
        assert_eq!(bins.len(), 5);
        for (i, bin) in bins.iter().enumerate() {
            assert_eq!(bin.left, (i as f64) * 2.0);
            assert_eq!(bin.right, ((i + 1) as f64) * 2.0);
            assert_eq!(bin.closed, IntervalClosed::Right);
        }
    }

    #[test]
    fn interval_range_by_periods_final_edge_is_exact_end() {
        // Guards against accumulated float drift on the last right edge.
        let bins = interval_range_by_periods(0.0, 1.0, 7, IntervalClosed::Right);
        assert_eq!(bins.last().unwrap().right, 1.0);
    }

    #[test]
    fn interval_range_by_periods_zero_periods_is_empty() {
        assert!(interval_range_by_periods(0.0, 10.0, 0, IntervalClosed::Right).is_empty());
    }

    #[test]
    fn interval_range_by_periods_reversed_range_is_empty() {
        // pandas: pd.interval_range(10, 0, periods=5) → IntervalIndex([]).
        assert!(interval_range_by_periods(10.0, 0.0, 5, IntervalClosed::Right).is_empty());
    }

    #[test]
    fn interval_range_by_periods_preserves_closed_policy() {
        for closed in [
            IntervalClosed::Left,
            IntervalClosed::Right,
            IntervalClosed::Both,
            IntervalClosed::Neither,
        ] {
            let bins = interval_range_by_periods(0.0, 4.0, 2, closed);
            assert!(bins.iter().all(|b| b.closed == closed));
        }
    }

    #[test]
    fn interval_range_by_step_matches_pandas_default_case() {
        // pd.interval_range(0, 10, freq=2) → [(0,2],(2,4],(4,6],(6,8],(8,10]]
        let bins = interval_range_by_step(0.0, 10.0, 2.0, IntervalClosed::Right).expect("ok");
        assert_eq!(bins.len(), 5);
        assert_eq!(bins[0].left, 0.0);
        assert_eq!(bins[4].right, 10.0);
    }

    #[test]
    fn interval_range_by_step_rejects_non_positive_step() {
        assert!(matches!(
            interval_range_by_step(0.0, 10.0, 0.0, IntervalClosed::Right),
            Err(TypeError::InvalidIntervalStep { .. })
        ));
        assert!(matches!(
            interval_range_by_step(0.0, 10.0, -2.0, IntervalClosed::Right),
            Err(TypeError::InvalidIntervalStep { .. })
        ));
        assert!(matches!(
            interval_range_by_step(0.0, 10.0, f64::NAN, IntervalClosed::Right),
            Err(TypeError::InvalidIntervalStep { .. })
        ));
        assert!(matches!(
            interval_range_by_step(0.0, 10.0, f64::INFINITY, IntervalClosed::Right),
            Err(TypeError::InvalidIntervalStep { .. })
        ));
    }

    #[test]
    fn interval_range_by_step_rejects_non_dividing_step() {
        // pandas: pd.interval_range(0, 10, freq=3) → ValueError
        // (span=10 not divisible by step=3). Reject with IntervalStepDoesNotDivide.
        assert!(matches!(
            interval_range_by_step(0.0, 10.0, 3.0, IntervalClosed::Right),
            Err(TypeError::IntervalStepDoesNotDivide { .. })
        ));
    }

    #[test]
    fn interval_range_by_step_reversed_range_is_empty() {
        let bins = interval_range_by_step(10.0, 0.0, 2.0, IntervalClosed::Right).expect("ok");
        assert!(bins.is_empty());
    }

    #[test]
    fn interval_range_by_step_degenerate_zero_span_is_empty() {
        let bins = interval_range_by_step(5.0, 5.0, 1.0, IntervalClosed::Right).expect("ok");
        assert!(bins.is_empty());
    }

    #[test]
    fn interval_range_by_step_accepts_float_step_within_tolerance() {
        // step=0.1 ten times == 1.0 but float arithmetic produces 0.9999...
        let bins = interval_range_by_step(0.0, 1.0, 0.1, IntervalClosed::Right).expect("ok");
        assert_eq!(bins.len(), 10);
        assert_eq!(bins.last().unwrap().right, 1.0);
    }

    // ── Timedelta arithmetic tests (br-frankenpandas-4r56 Phase 1) ──────

    use super::Timedelta;

    #[test]
    fn timedelta_add_sums_non_nat() {
        let one_hour = Timedelta::NANOS_PER_HOUR;
        let one_day = Timedelta::NANOS_PER_DAY;
        assert_eq!(Timedelta::add(one_hour, one_day), one_hour + one_day);
    }

    #[test]
    fn timedelta_add_propagates_nat() {
        assert_eq!(Timedelta::add(Timedelta::NAT, 100), Timedelta::NAT);
        assert_eq!(Timedelta::add(100, Timedelta::NAT), Timedelta::NAT);
        assert_eq!(
            Timedelta::add(Timedelta::NAT, Timedelta::NAT),
            Timedelta::NAT
        );
    }

    #[test]
    fn timedelta_add_saturates_on_overflow() {
        assert_eq!(Timedelta::add(i64::MAX - 10, 100), i64::MAX);
        // Note: i64::MIN is NaT; use MIN+1 to test saturation on the negative side.
        assert_eq!(Timedelta::add(i64::MIN + 10, -100), i64::MIN);
    }

    #[test]
    fn timedelta_sub_subtracts_non_nat() {
        let one_hour = Timedelta::NANOS_PER_HOUR;
        assert_eq!(Timedelta::sub(one_hour, Timedelta::NANOS_PER_MIN), one_hour - Timedelta::NANOS_PER_MIN);
    }

    #[test]
    fn timedelta_sub_propagates_nat() {
        assert_eq!(Timedelta::sub(Timedelta::NAT, 100), Timedelta::NAT);
        assert_eq!(Timedelta::sub(100, Timedelta::NAT), Timedelta::NAT);
    }

    #[test]
    fn timedelta_neg_flips_sign_non_nat() {
        assert_eq!(Timedelta::neg(5), -5);
        assert_eq!(Timedelta::neg(-5), 5);
        assert_eq!(Timedelta::neg(0), 0);
    }

    #[test]
    fn timedelta_neg_preserves_nat() {
        assert_eq!(Timedelta::neg(Timedelta::NAT), Timedelta::NAT);
    }

    #[test]
    fn timedelta_abs_returns_magnitude() {
        assert_eq!(Timedelta::abs(-5), 5);
        assert_eq!(Timedelta::abs(5), 5);
        assert_eq!(Timedelta::abs(0), 0);
        assert_eq!(Timedelta::abs(Timedelta::NAT), Timedelta::NAT);
    }

    #[test]
    fn timedelta_mul_scalar_scales() {
        let three_hours = Timedelta::NANOS_PER_HOUR * 3;
        assert_eq!(Timedelta::mul_scalar(Timedelta::NANOS_PER_HOUR, 3), three_hours);
        assert_eq!(Timedelta::mul_scalar(100, 0), 0);
        assert_eq!(Timedelta::mul_scalar(100, -2), -200);
    }

    #[test]
    fn timedelta_mul_scalar_saturates() {
        assert_eq!(Timedelta::mul_scalar(i64::MAX, 2), i64::MAX);
        // (i64::MIN + 1) * 2 saturates to i64::MIN (magnitude too large).
        assert_eq!(Timedelta::mul_scalar(i64::MIN + 1, 2), i64::MIN);
    }

    #[test]
    fn timedelta_mul_scalar_propagates_nat() {
        assert_eq!(Timedelta::mul_scalar(Timedelta::NAT, 5), Timedelta::NAT);
    }

    #[test]
    fn timedelta_div_scalar_floor_divides() {
        // Floor division (matches Python / pandas): -100 // 3 == -34, not -33.
        assert_eq!(Timedelta::div_scalar(100, 3), 33);
        assert_eq!(Timedelta::div_scalar(-100, 3), -34);
        assert_eq!(Timedelta::div_scalar(100, -3), -34);
        assert_eq!(Timedelta::div_scalar(-100, -3), 33);
    }

    #[test]
    fn timedelta_div_scalar_zero_divisor_returns_nat() {
        assert_eq!(Timedelta::div_scalar(100, 0), Timedelta::NAT);
    }

    #[test]
    fn timedelta_div_scalar_min_neg_one_propagates_nat() {
        // i64::MIN aliases NaT, so `div_scalar(i64::MIN, _)` propagates NaT
        // — the `i64::MIN / -1` arithmetic-overflow case is subsumed.
        assert_eq!(Timedelta::div_scalar(i64::MIN, -1), Timedelta::NAT);
        // (i64::MIN + 1) is a real timedelta; `/ -1` does not overflow.
        assert_eq!(Timedelta::div_scalar(i64::MIN + 1, -1), i64::MAX);
    }

    #[test]
    fn timedelta_div_scalar_propagates_nat() {
        assert_eq!(Timedelta::div_scalar(Timedelta::NAT, 10), Timedelta::NAT);
    }

    #[test]
    fn timedelta_div_timedelta_returns_float_ratio() {
        let two_hours = Timedelta::NANOS_PER_HOUR * 2;
        let one_hour = Timedelta::NANOS_PER_HOUR;
        assert!((Timedelta::div_timedelta(two_hours, one_hour) - 2.0).abs() < 1e-12);
        assert!((Timedelta::div_timedelta(one_hour, two_hours) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn timedelta_div_timedelta_nat_returns_nan() {
        assert!(Timedelta::div_timedelta(Timedelta::NAT, 100).is_nan());
        assert!(Timedelta::div_timedelta(100, Timedelta::NAT).is_nan());
    }

    // ── Timestamp tests (br-frankenpandas-9p0u — 4r56 Phase 2) ──────────

    use super::Timestamp;

    #[test]
    fn timestamp_from_nanos_is_naive_utc() {
        let ts = Timestamp::from_nanos(1_700_000_000_000_000_000);
        assert_eq!(ts.nanos, 1_700_000_000_000_000_000);
        assert_eq!(ts.tz, None);
        assert!(!ts.is_nat());
    }

    #[test]
    fn timestamp_from_nanos_tz_carries_tz_name() {
        let ts = Timestamp::from_nanos_tz(1_700_000_000_000_000_000, "US/Eastern");
        assert_eq!(ts.tz.as_deref(), Some("US/Eastern"));
    }

    #[test]
    fn timestamp_add_timedelta_shifts_nanos_and_preserves_tz() {
        let ts = Timestamp::from_nanos_tz(0, "US/Eastern");
        let one_day = Timedelta::NANOS_PER_DAY;
        let shifted = ts.add_timedelta(one_day);
        assert_eq!(shifted.nanos, one_day);
        assert_eq!(shifted.tz.as_deref(), Some("US/Eastern"));
    }

    #[test]
    fn timestamp_add_timedelta_saturates_on_overflow() {
        let ts = Timestamp::from_nanos(i64::MAX - 10);
        let shifted = ts.add_timedelta(100);
        assert_eq!(shifted.nanos, i64::MAX);
    }

    #[test]
    fn timestamp_add_timedelta_propagates_nat() {
        // NaT Timestamp + anything = NaT.
        assert!(Timestamp::nat().add_timedelta(100).is_nat());
        // Timestamp + NaT Timedelta = NaT.
        assert!(Timestamp::from_nanos(0).add_timedelta(Timedelta::NAT).is_nat());
    }

    #[test]
    fn timestamp_sub_timedelta_shifts_backward() {
        let ts = Timestamp::from_nanos(1_000);
        let shifted = ts.sub_timedelta(Timedelta::NANOS_PER_MICRO);
        assert_eq!(shifted.nanos, 0);
    }

    #[test]
    fn timestamp_sub_timestamp_returns_timedelta_nanos() {
        let t0 = Timestamp::from_nanos(0);
        let t1 = Timestamp::from_nanos(Timedelta::NANOS_PER_HOUR);
        assert_eq!(t1.sub_timestamp(&t0), Timedelta::NANOS_PER_HOUR);
        assert_eq!(t0.sub_timestamp(&t1), -Timedelta::NANOS_PER_HOUR);
    }

    #[test]
    fn timestamp_sub_timestamp_nat_propagates() {
        let ts = Timestamp::from_nanos(1_000);
        assert_eq!(Timestamp::nat().sub_timestamp(&ts), Timedelta::NAT);
        assert_eq!(ts.sub_timestamp(&Timestamp::nat()), Timedelta::NAT);
    }

    #[test]
    fn timestamp_semantic_eq_treats_two_nat_as_equal() {
        assert!(Timestamp::nat().semantic_eq(&Timestamp::nat()));
        assert!(!Timestamp::nat().semantic_eq(&Timestamp::from_nanos(0)));
        assert!(!Timestamp::from_nanos(0).semantic_eq(&Timestamp::nat()));
    }

    #[test]
    fn timestamp_partial_cmp_orders_by_nanos_nat_is_incomparable() {
        use std::cmp::Ordering;
        let a = Timestamp::from_nanos(0);
        let b = Timestamp::from_nanos(100);
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Less));
        assert_eq!(b.partial_cmp(&a), Some(Ordering::Greater));
        assert_eq!(a.partial_cmp(&a), Some(Ordering::Equal));
        assert_eq!(a.partial_cmp(&Timestamp::nat()), None);
        assert_eq!(Timestamp::nat().partial_cmp(&Timestamp::nat()), None);
    }

    #[test]
    fn timestamp_display_matches_phase2_debug_format() {
        assert_eq!(Timestamp::from_nanos(42).to_string(), "Timestamp[42, UTC]");
        assert_eq!(
            Timestamp::from_nanos_tz(42, "US/Eastern").to_string(),
            "Timestamp[42, US/Eastern]"
        );
        assert_eq!(Timestamp::nat().to_string(), "NaT");
    }

    #[test]
    fn timestamp_roundtrips_through_serde_json() {
        let naive = Timestamp::from_nanos(1_700_000_000_000_000_000);
        let json = serde_json::to_string(&naive).expect("serialize");
        let back: Timestamp = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(naive, back);

        let tz_aware = Timestamp::from_nanos_tz(1_700_000_000_000_000_000, "US/Eastern");
        let json = serde_json::to_string(&tz_aware).expect("serialize");
        let back: Timestamp = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(tz_aware, back);
    }

    #[test]
    fn timestamp_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Timestamp>();
    }

    // ── Timestamp rounding tests (br-frankenpandas-5h6n) ────────────────

    #[test]
    fn timestamp_floor_to_rounds_down() {
        // 12:34:56 → floor by 1H → 12:00:00
        let h = Timedelta::NANOS_PER_HOUR;
        let twelve_h = h * 12;
        let twelve_thirty_four = twelve_h + Timedelta::NANOS_PER_MIN * 34 + Timedelta::NANOS_PER_SEC * 56;
        let ts = Timestamp::from_nanos(twelve_thirty_four);
        let floored = ts.floor_to(h);
        assert_eq!(floored.nanos, twelve_h);
    }

    #[test]
    fn timestamp_floor_to_handles_already_aligned() {
        // 12:00:00 floored by 1H → 12:00:00 (no change).
        let h = Timedelta::NANOS_PER_HOUR;
        let twelve_h = h * 12;
        let ts = Timestamp::from_nanos(twelve_h);
        assert_eq!(ts.floor_to(h).nanos, twelve_h);
    }

    #[test]
    fn timestamp_floor_to_handles_negative_nanos() {
        // -100 ns floored by 60 ns:
        //   div_euclid(-100, 60) = -2 (since -2*60=-120, rem=20 ≥ 0).
        //   result = -2 * 60 = -120.
        let ts = Timestamp::from_nanos(-100);
        assert_eq!(ts.floor_to(60).nanos, -120);
    }

    #[test]
    fn timestamp_ceil_to_rounds_up() {
        // 12:34:56 → ceil by 1H → 13:00:00.
        let h = Timedelta::NANOS_PER_HOUR;
        let twelve_h = h * 12;
        let thirteen_h = h * 13;
        let twelve_thirty_four = twelve_h + Timedelta::NANOS_PER_MIN * 34 + Timedelta::NANOS_PER_SEC * 56;
        let ts = Timestamp::from_nanos(twelve_thirty_four);
        assert_eq!(ts.ceil_to(h).nanos, thirteen_h);
    }

    #[test]
    fn timestamp_ceil_to_no_op_on_aligned() {
        let h = Timedelta::NANOS_PER_HOUR;
        let twelve_h = h * 12;
        let ts = Timestamp::from_nanos(twelve_h);
        assert_eq!(ts.ceil_to(h).nanos, twelve_h);
    }

    #[test]
    fn timestamp_round_to_rounds_to_nearest() {
        // 12:30:01 (one second past the half-hour): round to 1H → 13:00:00.
        let h = Timedelta::NANOS_PER_HOUR;
        let twelve_h = h * 12;
        let twelve_thirty_one_sec = twelve_h + Timedelta::NANOS_PER_MIN * 30 + Timedelta::NANOS_PER_SEC;
        let ts = Timestamp::from_nanos(twelve_thirty_one_sec);
        assert_eq!(ts.round_to(h).nanos, h * 13);

        // 12:29:59 (one second before half): round to 1H → 12:00:00.
        let twelve_twenty_nine_sec = twelve_h + Timedelta::NANOS_PER_MIN * 29 + Timedelta::NANOS_PER_SEC * 59;
        let ts = Timestamp::from_nanos(twelve_twenty_nine_sec);
        assert_eq!(ts.round_to(h).nanos, twelve_h);
    }

    #[test]
    fn timestamp_round_to_bankers_tie_to_even() {
        // Tie cases: rem == unit/2 exactly. Pick even-multiple floor.
        // unit=10, so half=5. nanos=5: floor=0 (even), so → 0.
        // nanos=15: floor=1 (odd), so → 20.
        // nanos=25: floor=2 (even), so → 20.
        // nanos=35: floor=3 (odd), so → 40.
        assert_eq!(Timestamp::from_nanos(5).round_to(10).nanos, 0);
        assert_eq!(Timestamp::from_nanos(15).round_to(10).nanos, 20);
        assert_eq!(Timestamp::from_nanos(25).round_to(10).nanos, 20);
        assert_eq!(Timestamp::from_nanos(35).round_to(10).nanos, 40);
    }

    #[test]
    fn timestamp_round_to_zero_unit_returns_nat() {
        let ts = Timestamp::from_nanos(100);
        assert!(ts.round_to(0).is_nat());
        assert!(ts.floor_to(0).is_nat());
        assert!(ts.ceil_to(0).is_nat());
    }

    #[test]
    fn timestamp_round_to_negative_unit_returns_nat() {
        let ts = Timestamp::from_nanos(100);
        assert!(ts.round_to(-10).is_nat());
        assert!(ts.floor_to(-10).is_nat());
        assert!(ts.ceil_to(-10).is_nat());
    }

    #[test]
    fn timestamp_rounding_propagates_nat() {
        let nat = Timestamp::nat();
        assert!(nat.floor_to(60).is_nat());
        assert!(nat.ceil_to(60).is_nat());
        assert!(nat.round_to(60).is_nat());
    }

    #[test]
    fn timestamp_rounding_preserves_tz() {
        let ts = Timestamp::from_nanos_tz(100, "US/Eastern");
        assert_eq!(ts.floor_to(60).tz.as_deref(), Some("US/Eastern"));
        assert_eq!(ts.ceil_to(60).tz.as_deref(), Some("US/Eastern"));
        assert_eq!(ts.round_to(60).tz.as_deref(), Some("US/Eastern"));
    }

    // ── Timestamp string-unit rounding tests (br-frankenpandas-lbsx) ────

    #[test]
    fn timestamp_floor_to_unit_h_rounds_to_hour() {
        let h = Timedelta::NANOS_PER_HOUR;
        let twelve_h = h * 12;
        let twelve_thirty_four = twelve_h
            + Timedelta::NANOS_PER_MIN * 34
            + Timedelta::NANOS_PER_SEC * 56;
        let ts = Timestamp::from_nanos(twelve_thirty_four);
        assert_eq!(ts.floor_to_unit("H").nanos, twelve_h);
        assert_eq!(ts.floor_to_unit("h").nanos, twelve_h);
        assert_eq!(ts.floor_to_unit("hour").nanos, twelve_h);
        assert_eq!(ts.floor_to_unit("hours").nanos, twelve_h);
        assert_eq!(ts.floor_to_unit("hr").nanos, twelve_h);
    }

    #[test]
    fn timestamp_ceil_to_unit_d_rounds_to_day() {
        // 12:34:56 → ceil to 1 day → 24:00:00 (next day).
        let h = Timedelta::NANOS_PER_HOUR;
        let d = Timedelta::NANOS_PER_DAY;
        let twelve_thirty_four = h * 12 + Timedelta::NANOS_PER_MIN * 34;
        let ts = Timestamp::from_nanos(twelve_thirty_four);
        assert_eq!(ts.ceil_to_unit("D").nanos, d);
        assert_eq!(ts.ceil_to_unit("day").nanos, d);
        assert_eq!(ts.ceil_to_unit("days").nanos, d);
    }

    #[test]
    fn timestamp_round_to_unit_min_rounds_to_minute() {
        // 12:34:31 → round to 1 minute → 12:35:00.
        let m = Timedelta::NANOS_PER_MIN;
        let twelve_thirty_four_thirty_one = Timedelta::NANOS_PER_HOUR * 12
            + m * 34
            + Timedelta::NANOS_PER_SEC * 31;
        let ts = Timestamp::from_nanos(twelve_thirty_four_thirty_one);
        let expected = Timedelta::NANOS_PER_HOUR * 12 + m * 35;
        assert_eq!(ts.round_to_unit("min").nanos, expected);
        assert_eq!(ts.round_to_unit("T").nanos, expected); // pandas pre-2.2 alias
        assert_eq!(ts.round_to_unit("minute").nanos, expected);
    }

    #[test]
    fn timestamp_unit_rounding_unknown_unit_returns_nat() {
        let ts = Timestamp::from_nanos(100);
        assert!(ts.floor_to_unit("fortnight").is_nat());
        assert!(ts.ceil_to_unit("century").is_nat());
        assert!(ts.round_to_unit("xyz").is_nat());
    }

    #[test]
    fn timestamp_unit_rounding_propagates_nat() {
        let nat = Timestamp::nat();
        assert!(nat.floor_to_unit("H").is_nat());
        assert!(nat.ceil_to_unit("H").is_nat());
        assert!(nat.round_to_unit("H").is_nat());
    }

    #[test]
    fn timestamp_unit_rounding_preserves_tz() {
        let ts = Timestamp::from_nanos_tz(Timedelta::NANOS_PER_HOUR * 12 + 100, "US/Eastern");
        assert_eq!(ts.floor_to_unit("H").tz.as_deref(), Some("US/Eastern"));
        assert_eq!(ts.ceil_to_unit("H").tz.as_deref(), Some("US/Eastern"));
        assert_eq!(ts.round_to_unit("H").tz.as_deref(), Some("US/Eastern"));
    }

    #[test]
    fn timedelta_unit_to_nanos_is_now_public_and_matches_pandas_aliases() {
        // Public surface check: pandas alias core set.
        assert_eq!(Timedelta::unit_to_nanos("W"), Some(Timedelta::NANOS_PER_WEEK));
        assert_eq!(Timedelta::unit_to_nanos("D"), Some(Timedelta::NANOS_PER_DAY));
        assert_eq!(Timedelta::unit_to_nanos("H"), Some(Timedelta::NANOS_PER_HOUR));
        assert_eq!(Timedelta::unit_to_nanos("min"), Some(Timedelta::NANOS_PER_MIN));
        assert_eq!(Timedelta::unit_to_nanos("s"), Some(Timedelta::NANOS_PER_SEC));
        assert_eq!(Timedelta::unit_to_nanos("ms"), Some(Timedelta::NANOS_PER_MILLI));
        assert_eq!(Timedelta::unit_to_nanos("us"), Some(Timedelta::NANOS_PER_MICRO));
        assert_eq!(Timedelta::unit_to_nanos("ns"), Some(1));
        // Empty string → days (pandas default).
        assert_eq!(Timedelta::unit_to_nanos(""), Some(Timedelta::NANOS_PER_DAY));
        // Unknown alias → None.
        assert_eq!(Timedelta::unit_to_nanos("century"), None);
    }
}
