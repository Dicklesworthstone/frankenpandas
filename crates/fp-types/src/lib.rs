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
    #[serde(alias = "string", alias = "str")]
    Utf8,
    Categorical,
    Timedelta64,
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
            DType::Bool | DType::Int64 | DType::Utf8 | DType::Categorical => {
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
            (Self::Null(NullKind::NaN), Self::Float64(v))
            | (Self::Float64(v), Self::Null(NullKind::NaN)) => v.is_nan(),
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
}

pub fn common_dtype(left: DType, right: DType) -> Result<DType, TypeError> {
    use DType::{Bool, Categorical, Float64, Int64, Null, Timedelta64};

    let out = match (left, right) {
        (a, b) if a == b => a,
        (Null, other) | (other, Null) => other,
        (Categorical, Categorical) => Categorical,
        (Bool, Int64) | (Int64, Bool) => Int64,
        (Bool, Float64) | (Float64, Bool) => Float64,
        (Int64, Float64) | (Float64, Int64) => Float64,
        (Timedelta64, Timedelta64) => Timedelta64,
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

    fn unit_to_nanos(unit: &str) -> Option<i64> {
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
        let err = common_dtype(DType::Float64, DType::Utf8).expect_err("must fail");
        assert_eq!(
            err.to_string(),
            "dtype coercion from Float64 to Utf8 has no compatible common type"
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
}
